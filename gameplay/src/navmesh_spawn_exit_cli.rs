//! Bounded private-geometry discovery; all map coordinates supplied externally.
use mag_gameplay::{
    bot_physics::{Body, WorldTerrain},
    collision::{CollisionLimits, CollisionWorld},
    navmesh::{self, Artifact, NavMesh, Status},
};
use serde::Deserialize;
use serde_json::json;
use std::{collections::BTreeMap, fs, io::Write};
type Point = [f64; 3];
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Parameters {
    groups: Vec<Group>,
    #[serde(default)]
    cadences: Vec<Vec<f64>>,
    #[serde(default)]
    initial_slices: Vec<f64>,
    #[serde(default = "default_radius")]
    max_radius_steps: i32,
    #[serde(default = "default_headings")]
    heading_offsets_radians: Vec<f64>,
    #[serde(default = "default_distances")]
    landing_distances: Vec<f64>,
}
fn default_radius() -> i32 {
    15
}
fn default_headings() -> Vec<f64> {
    vec![0., -0.4, 0.4, -0.8, 0.8, -1.2, 1.2]
}
fn default_distances() -> Vec<f64> {
    vec![3., 5., 7., 9., 12.]
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Group {
    label: String,
    original_link_id: u32,
    spawn: Point,
    goal: Point,
    new_id_base: u32,
}
fn supported(t: &WorldTerrain<'_>, probe: Point, b: Body) -> Result<Point, String> {
    t.require_horizontal_coverage(probe, probe, b)
        .map_err(|e| e.code().to_owned())?;
    let mut p = t
        .sample_ground(probe, 2., 4.)
        .map_err(|e| e.code().to_owned())?
        .ok_or("ground_absent")?
        .position;
    p[1] += 0.4;
    let m = t.start_motion(p, b).map_err(|e| e.code().to_owned())?;
    let m = t
        .advance(m, [0.; 3], 1., b)
        .map_err(|e| e.code().to_owned())?;
    if !m.grounded || m.blocked {
        return Err("unsupported_or_blocked".into());
    }
    Ok(m.position)
}
fn xz(a: Point, b: Point) -> f64 {
    (a[0] - b[0]).hypot(a[2] - b[2])
}
fn protected_bounds(l: &navmesh::DropLink) -> (Point, Point) {
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for m in &l.trajectory {
        for k in 0..3 {
            lo[k] = lo[k].min(m.position[k] - [0.35, 0.0, 0.35][k]);
            hi[k] = hi[k].max(m.position[k] + [0.35, 1.8, 0.35][k]);
        }
    }
    for k in 0..3 {
        lo[k] = lo[k].min(l.launch[k] - [1.5, 0.0, 1.5][k]);
        hi[k] = hi[k].max(l.launch[k] + [1.5, 1.8, 1.5][k]);
    }
    (lo, hi)
}
fn swept_overlaps(a: Point, b: Point, l: &navmesh::DropLink) -> bool {
    let (lo, hi) = protected_bounds(l);
    (0..3).all(|k| {
        a[k].min(b[k]) - [0.35, 0.0, 0.35][k] < hi[k]
            && a[k].max(b[k]) + [0.35, 1.8, 0.35][k] > lo[k]
    })
}
fn replay(
    t: &WorldTerrain,
    mesh: &NavMesh,
    from: Point,
    to: Point,
    body: Body,
    avoid: Option<&navmesh::DropLink>,
) -> Result<(Point, usize), String> {
    let route = mesh.route(from, to, 0.3).map_err(|e| e.0.to_owned())?;
    if route.status != Status::Complete {
        return Err(format!("route_{:?}", route.status));
    }
    let mut m = t
        .start_motion(from, body)
        .map_err(|e| e.code().to_owned())?;
    m = t
        .advance(m, [0.; 3], 0.25, body)
        .map_err(|e| e.code().to_owned())?;
    let mut cursor = 0;
    let mut stalls = 0;
    for tick in 0..2400 {
        while cursor < route.waypoints.len() && xz(m.position, route.waypoints[cursor]) < 0.08 {
            cursor += 1;
        }
        if cursor == route.waypoints.len() {
            if xz(m.position, to) <= 0.4 && m.grounded && !m.blocked {
                return Ok((m.position, tick));
            }
            return Err("arrival_not_supported".into());
        }
        let target = route.waypoints[cursor];
        let dx = target[0] - m.position[0];
        let dz = target[2] - m.position[2];
        let d = dx.hypot(dz);
        let speed = 4.5_f64.min(d / 0.05);
        let old = m.position;
        m = t
            .advance(m, [dx / d * speed, 0., dz / d * speed], 0.05, body)
            .map_err(|e| e.code().to_owned())?;
        if avoid.is_some_and(|l| swept_overlaps(old, m.position, l)) {
            return Err("incoming_crosses_protected_tube".into());
        }
        if m.blocked {
            return Err("physics_blocked".into());
        }
        if xz(old, m.position) < 0.0001 {
            stalls += 1;
        } else {
            stalls = 0;
        }
        if stalls > 20 {
            return Err("physics_stalled".into());
        }
    }
    Err("replay_deadline".into())
}
fn robust_descent(
    t: &WorldTerrain,
    mesh: &NavMesh,
    spawn: Point,
    l: &navmesh::DropLink,
    recorded: &[Vec<f64>],
    initial_slices: &[f64],
) -> Result<Vec<serde_json::Value>, String> {
    let mut evidence = vec![];
    for cadence in recorded {
        let body = Body::default();
        let r = mesh
            .route(spawn, l.launch, 0.1)
            .map_err(|e| e.0.to_owned())?;
        let mut m = t
            .start_motion(spawn, body)
            .map_err(|e| e.code().to_owned())?;
        let mut cursor = 0;
        let mut tick = 0;
        for _ in 0..2400 {
            while cursor < r.waypoints.len() && xz(m.position, r.waypoints[cursor]) <= 1. / 32. {
                cursor += 1;
            }
            if cursor == r.waypoints.len() && m.grounded {
                break;
            }
            let target = r.waypoints.get(cursor).copied().unwrap_or(m.position);
            let d = xz(m.position, target);
            let dt = cadence[tick % cadence.len()].min(if d > 0.0000001 { d / 4.5 } else { 0.25 });
            let v = if d > 0.0000001 {
                [
                    (target[0] - m.position[0]) / d * 4.5,
                    0.,
                    (target[2] - m.position[2]) / d * 4.5,
                ]
            } else {
                [0.; 3]
            };
            m = t.advance(m, v, dt, body).map_err(|e| e.code().to_owned())?;
            if m.blocked {
                return Err("robust_prefix_blocked".into());
            }
            tick += 1;
        }
        if !m.grounded || xz(m.position, l.launch) > 1. / 32. {
            return Err("robust_prefix_deadline".into());
        }
        let initial = m;
        for &initial_dt in initial_slices {
            m = initial;
            let mut first = true;
            let mut elapsed = 0.;
            let mut done = false;
            for _ in 0..1000 {
                let nearest = l
                    .trajectory
                    .iter()
                    .map(|q| {
                        q.position
                            .iter()
                            .zip(m.position)
                            .map(|(a, b)| (a - b).powi(2))
                            .sum::<f64>()
                    })
                    .fold(f64::INFINITY, f64::min);
                if nearest > 0.36 {
                    return Err("robust_outside_cert".into());
                }
                let d = xz(m.position, l.landing);
                if d <= 1. / 32. && m.grounded {
                    done = true;
                    break;
                }
                let dt = (if first {
                    first = false;
                    initial_dt
                } else {
                    cadence[tick % cadence.len()]
                })
                .min(if d > 0.0000001 { d / 4.5 } else { 0.25 });
                let v = if d > 0.0000001 {
                    [
                        (l.landing[0] - m.position[0]) / d * 4.5,
                        0.,
                        (l.landing[2] - m.position[2]) / d * 4.5,
                    ]
                } else {
                    [0.; 3]
                };
                m = t.advance(m, v, dt, body).map_err(|e| e.code().to_owned())?;
                if m.blocked {
                    return Err("robust_descent_blocked".into());
                }
                elapsed += dt;
                if elapsed > 15. {
                    break;
                }
                tick += 1;
            }
            if !done {
                return Err("robust_descent_deadline".into());
            }
            evidence.push(json!({"cadence":cadence,"initial_dt":initial_dt,"landing":m.position,"seconds":elapsed,"grounded":m.grounded}));
        }
    }
    Ok(evidence)
}

pub fn run(
    world_path: &str,
    mesh_path: &str,
    parameters: &str,
    output: &str,
    fall_rules: &str,
    health: u16,
) -> Result<(), Box<dyn std::error::Error>> {
    let rules = mag_gameplay::fall_damage::Rules::load(fall_rules)?;
    if health == 0 {
        return Err("positive health required".into());
    }
    let config: Parameters = serde_json::from_slice(&fs::read(parameters)?)?;
    if config.groups.is_empty() || config.groups.len() > 16 {
        return Err("expected1..16groups".into());
    }
    if config.groups.iter().any(|g| {
        g.label.is_empty()
            || g.new_id_base == 0
            || g.original_link_id == 0
            || g.spawn.iter().chain(g.goal.iter()).any(|v| !v.is_finite())
    }) {
        return Err("invalid source group".into());
    }
    if config.cadences.is_empty() != config.initial_slices.is_empty()
        || config.cadences.len() > 32
        || config.initial_slices.len() > 64
        || config.cadences.iter().any(|c| {
            c.is_empty()
                || c.len() > 2000
                || c.iter().any(|v| !v.is_finite() || *v <= 0. || *v > 0.25)
        })
        || config
            .initial_slices
            .iter()
            .any(|v| !v.is_finite() || *v <= 0. || *v > 0.25)
    {
        return Err("invalid robustness slices".into());
    }
    if !(3..=40).contains(&config.max_radius_steps)
        || config.heading_offsets_radians.is_empty()
        || config.heading_offsets_radians.len() > 64
        || config
            .heading_offsets_radians
            .iter()
            .any(|v| !v.is_finite() || v.abs() > std::f64::consts::TAU)
        || config.landing_distances.is_empty()
        || config.landing_distances.len() > 16
        || config
            .landing_distances
            .iter()
            .any(|v| !v.is_finite() || *v <= 0. || *v > 16.)
    {
        return Err("invalid search bounds".into());
    }
    let out = std::path::Path::new(output);
    fs::create_dir(out)?;
    let world = CollisionWorld::load(world_path, CollisionLimits::default())?;
    let hash = navmesh::sha256(&fs::read(world_path)?);
    let mut art: Artifact = serde_json::from_slice(&fs::read(mesh_path)?)?;
    let old = art.drop_links.clone();
    art.drop_links.clear();
    let mesh = NavMesh::load(art.clone(), &hash, &world.metadata().map_name)?;
    mesh.validate_coverage(&world)?;
    let t = WorldTerrain::new(Some(&world), &world.metadata().map_name).map_err(|e| e.code())?;
    let body = Body::default();
    let mut all = old.clone();
    let mut rows = vec![];
    for parameter in &config.groups {
        let original = old
            .iter()
            .find(|l| l.id == parameter.original_link_id)
            .ok_or("source link absent")?;
        let spawn = parameter.spawn;
        let goal = parameter.goal;
        let squad = &parameter.label;
        let mut group = vec![original.clone()];
        let mut failures = BTreeMap::<String, usize>::new();
        let mut attempts = 0;
        let mut evidence = vec![];
        'scan: for radius in 3_i32..=config.max_radius_steps {
            for ix in -radius..=radius {
                for iz in -radius..=radius {
                    if ix.abs() != radius && iz.abs() != radius {
                        continue;
                    }
                    let probe = [
                        original.launch[0] + ix as f64 * 1.5,
                        original.launch[1],
                        original.launch[2] + iz as f64 * 1.5,
                    ];
                    if group.iter().any(|l| xz(probe, l.launch) < 5.0) {
                        continue;
                    }
                    attempts += 1;
                    let Ok(launch) = supported(&t, probe, body) else {
                        continue;
                    };
                    if mesh
                        .route(spawn, launch, 0.1)
                        .map(|r| r.status != Status::Complete)
                        .unwrap_or(true)
                    {
                        continue;
                    }
                    let base = (original.landing[2] - original.launch[2])
                        .atan2(original.landing[0] - original.launch[0]);
                    for &distance in &config.landing_distances {
                        for &angle in &config.heading_offsets_radians {
                            let theta = base + angle;
                            let probe = [
                                launch[0] + distance * theta.cos(),
                                launch[1],
                                launch[2] + distance * theta.sin(),
                            ];
                            let Some(g) = t.sample_ground(probe, 3., 15.).ok().flatten() else {
                                continue;
                            };
                            let Ok(landing) = supported(&t, g.position, body) else {
                                continue;
                            };
                            if landing[1] >= launch[1] - 0.1 {
                                continue;
                            }
                            let Ok(suffix) = mesh.route(landing, goal, 5.) else {
                                continue;
                            };
                            if suffix.status != Status::Complete {
                                continue;
                            }
                            let id = parameter
                                .new_id_base
                                .checked_add(group.len() as u32)
                                .ok_or("link ID overflow")?;
                            if all.iter().any(|l| l.id == id) {
                                return Err("duplicate candidate ID".into());
                            }
                            let link = match navmesh::DropLink::certify(
                                &world, &hash, id, launch, landing, 4.5,
                            ) {
                                Ok(v) => v,
                                Err(e) => {
                                    *failures.entry(e.0.into()).or_default() += 1;
                                    continue;
                                }
                            };
                            if rules
                                .predict_drop(link.max_drop_height as f32, health)?
                                .remaining_health_quarter_points
                                != health
                            {
                                *failures
                                    .entry("drop_predicts_health_loss".into())
                                    .or_default() += 1;
                                continue;
                            }
                            if group.iter().any(|other| {
                                let (a, b) = protected_bounds(other);
                                let (c, d) = protected_bounds(&link);
                                (0..3).all(|k| a[k] < d[k] && b[k] > c[k])
                            }) {
                                *failures
                                    .entry("overlapping_protected_corridor".into())
                                    .or_default() += 1;
                                continue;
                            }
                            if mesh
                                .route(link.landing, goal, 5.)
                                .map(|r| r.status != Status::Complete)
                                .unwrap_or(true)
                            {
                                continue;
                            }
                            let robustness = if config.cadences.is_empty() {
                                vec![]
                            } else {
                                match robust_descent(
                                    &t,
                                    &mesh,
                                    spawn,
                                    &link,
                                    &config.cadences,
                                    &config.initial_slices,
                                ) {
                                    Ok(v) => v,
                                    Err(e) => {
                                        *failures.entry(e).or_default() += 1;
                                        continue;
                                    }
                                }
                            };
                            let prefix = match replay(&t, &mesh, spawn, launch, body, None) {
                                Ok(v) => v,
                                Err(e) => {
                                    *failures.entry(e).or_default() += 1;
                                    continue;
                                }
                            };
                            println!(
                                "group={} new={} launch={:?} landing={:?} prefix_ticks={}",
                                original.id, id, launch, link.landing, prefix.1
                            );
                            std::io::stdout().flush()?;
                            evidence.push(json!({"id":id,"launch":launch,"landing":link.landing,"prefix_replay_ticks":prefix.1,"suffix_route_length":suffix.length_m,"max_drop":link.max_drop_height,"robustness":robustness}));
                            if all.len() >= 64 {
                                return Err("candidate link limit".into());
                            }
                            all.push(link.clone());
                            group.push(link);
                            if group.len() >= 4 {
                                break 'scan;
                            }
                            continue 'scan;
                        }
                    }
                }
            }
        }
        rows.push(json!({"original_drop":original.id,"squad":squad,"attempts":attempts,"candidates":evidence,"failures":failures}));
        fs::write(out.join("evidence.json"), serde_json::to_vec_pretty(&rows)?)?;
        art.drop_links = all.clone();
        fs::write(
            out.join("world-alternatives.mag-nav"),
            serde_json::to_vec(&art)?,
        )?;
        println!(
            "done group={} alternatives={} attempts={}",
            original.id,
            group.len(),
            attempts
        );
    }
    fs::write(
        out.join("provenance.json"),
        serde_json::to_vec_pretty(
            &json!({"format":"mag_spawn_exit_search_v1","world_sha256":hash,"input_mesh_sha256":navmesh::sha256(&fs::read(mesh_path)?),"parameters_sha256":navmesh::sha256(&fs::read(parameters)?),"output_mesh_sha256":navmesh::sha256(&fs::read(out.join("world-alternatives.mag-nav"))?),"speed_m_s":4.5,"fall_rules_sha256":navmesh::sha256(&fs::read(fall_rules)?),"health_quarter_points":health,"accepted_health_policy":"no predicted loss under supplied actual fall rules","maximum_launch_ring_m":config.max_radius_steps as f64*1.5,"production_follower_acceptance":"not established by discovery"}),
        )?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn private_parameters_are_explicit_and_reject_unknown_fields() {
        let p:Parameters=serde_json::from_str(r#"{"groups":[{"label":"synthetic","original_link_id":1,"spawn":[0,0,0],"goal":[1,0,1],"new_id_base":100}]}"#).unwrap();
        assert_eq!(p.groups[0].new_id_base, 100);
        let wide:Parameters=serde_json::from_str(r#"{"groups":[],"max_radius_steps":40,"heading_offsets_radians":[-3.141592653589793,0,3.141592653589793],"landing_distances":[2,15],"cadences":[[0.25,0.007]],"initial_slices":[0.001,0.25]}"#).unwrap();
        assert_eq!(wide.max_radius_steps, 40);
        assert_eq!(wide.cadences[0], vec![0.25, 0.007]);
        assert_eq!(wide.initial_slices, vec![0.001, 0.25]);

        assert!(
            serde_json::from_str::<Parameters>(r#"{"groups":[],"disable_collision":true}"#)
                .is_err()
        );
        assert!(
            serde_json::from_str::<Parameters>(r#"{"groups":[{"label":"missing-source"}]}"#)
                .is_err()
        );
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplayCase {
    drop_id: u32,
    motion: mag_gameplay::bot_physics::Motion,
    slices: Vec<f64>,
}
/// Replay exact retained physics state without resampling or assigning any pose.
pub fn replay_case(
    world_path: &str,
    mesh_path: &str,
    fixture_path: &str,
    output: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let f: ReplayCase = serde_json::from_slice(&fs::read(fixture_path)?)?;
    if f.slices.is_empty()
        || f.slices.len() > 2000
        || f.slices
            .iter()
            .any(|v| !v.is_finite() || *v <= 0. || *v > 0.25)
    {
        return Err("invalid replay slices".into());
    }
    let world = CollisionWorld::load(world_path, CollisionLimits::default())?;
    let hash = navmesh::sha256(&fs::read(world_path)?);
    let art: Artifact = serde_json::from_slice(&fs::read(mesh_path)?)?;
    let mesh = NavMesh::load(art, &hash, &world.metadata().map_name)?;
    let l = mesh
        .artifact()
        .drop_links
        .iter()
        .find(|l| l.id == f.drop_id)
        .ok_or("replay link absent")?;
    let t = WorldTerrain::new(Some(&world), &world.metadata().map_name).map_err(|e| e.code())?;
    let mut m = f.motion;
    let mut rows = vec![];
    let mut error = None;
    for &dt in &f.slices {
        let nearest = l
            .trajectory
            .iter()
            .map(|q| {
                q.position
                    .iter()
                    .zip(m.position)
                    .map(|(a, b)| (a - b).powi(2))
                    .sum::<f64>()
            })
            .fold(f64::INFINITY, f64::min);
        if nearest > 0.36 {
            error = Some("outside_cert");
            break;
        }
        let d = xz(m.position, l.landing);
        let dt = dt.min(if d > 1e-7 { d / 4.5 } else { 0.25 });
        let v = if d > 1e-7 {
            [
                (l.landing[0] - m.position[0]) / d * 4.5,
                0.,
                (l.landing[2] - m.position[2]) / d * 4.5,
            ]
        } else {
            [0.; 3]
        };
        let before = m;
        m = t.advance(m, v, dt, Body::default()).map_err(|e| e.code())?;
        rows.push(json!({"before":before,"after":m,"slice":dt}));
        if m.blocked {
            error = Some("physics_blocked");
            break;
        }
        if xz(m.position, l.landing) <= 1. / 32. && m.grounded {
            break;
        }
    }
    let result = json!({"format":"mag_retained_drop_replay_v1","drop_id":f.drop_id,"world_sha256":hash,"fixture_sha256":navmesh::sha256(&fs::read(fixture_path)?),"rows":rows,"error":error,"final_motion":m,"reached":m.grounded&&xz(m.position,l.landing)<=1./32.});
    let bytes = serde_json::to_vec_pretty(&result)?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)?;
    file.write_all(&bytes)?;
    Ok(())
}
