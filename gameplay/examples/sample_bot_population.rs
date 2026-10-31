//! Offline population sampling from supplied private collision and navmesh artifacts.
//! Outputs no assets. Every accepted slot is grounded and has a complete certified mesh route.
//! Final route locomotion must be checked with the production follower separately.
use mag_gameplay::{
    bot_physics::{Body, WorldTerrain},
    collision::{CollisionLimits, CollisionWorld},
    navmesh::{self, Artifact, NavMesh, Status},
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, io::Write};
type Point = [f64; 3];
type SampleTrial = (Point, Point, (usize, usize, f64));
fn point(s: &str) -> Result<Point, Box<dyn std::error::Error>> {
    Ok(s.split(',')
        .map(str::parse)
        .collect::<Result<Vec<_>, _>>()?
        .try_into()
        .map_err(|_| "point requires x,y,z")?)
}
fn distance(a: Point, b: Point) -> f64 {
    a.iter()
        .zip(b)
        .map(|(a, b)| (a - b).powi(2))
        .sum::<f64>()
        .sqrt()
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
fn obj(p: Point) -> Value {
    json!({"x":p[0],"y":p[1],"z":p[2]})
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let a: Vec<String> = std::env::args().skip(1).collect();
    if a.len() != 8 {
        return Err("usage: sample_bot_population WORLD MESH TEAM0_ANCHOR TEAM0_GOAL TEAM1_ANCHOR OUTPUT EVIDENCE FALL_RULES".into());
    }
    let world = CollisionWorld::load(&a[0], CollisionLimits::default())?;
    let hash = navmesh::sha256(&fs::read(&a[0])?);
    let artifact: Artifact = serde_json::from_slice(&fs::read(&a[1])?)?;
    let mesh = NavMesh::load(artifact, &hash, &world.metadata().map_name)?;
    mesh.validate_coverage(&world)?;
    let terrain =
        WorldTerrain::new(Some(&world), &world.metadata().map_name).map_err(|e| e.code())?;
    let body = Body::default();
    let rules = mag_gameplay::fall_damage::Rules::load(&a[7])?;
    let anchor0 = point(&a[2])?;
    let goal0 = supported(&terrain, point(&a[3])?, body)?;
    let anchor1 = point(&a[4])?;
    let mut slots = Vec::new();
    let mut positions: Vec<Point> = Vec::new();
    let mut failures: BTreeMap<String, usize> = BTreeMap::new();
    let mut evidence = Vec::new();
    let mut attempts = 0;
    for (team, anchor) in [(0, anchor0), (1, anchor1)] {
        let mut accepted = 0;
        'sampling: for radius in 1_i32..=10 {
            for dx in -radius..=radius {
                for dz in -radius..=radius {
                    if dx.abs() != radius && dz.abs() != radius {
                        continue;
                    }
                    attempts += 1;
                    let probe = [
                        anchor[0] + f64::from(dx) * 1.5,
                        anchor[1],
                        anchor[2] + f64::from(dz) * 1.5,
                    ];
                    let trial = (|| -> Result<SampleTrial, String> {
                        let p = supported(&terrain, probe, body)?;
                        if distance(p, anchor0) < 0.71
                            || positions.iter().any(|q| distance(p, *q) < 0.71)
                        {
                            return Err("separation".into());
                        }
                        let goal = if team == 0 {
                            goal0
                        } else {
                            supported(&terrain, [p[0] - 3., p[1], p[2]], body)?
                        };
                        let route = mesh
                            .route_with_drops(p, goal, 5., &rules, 400)
                            .map_err(|e| e.0.to_owned())?;
                        if route.status != Status::Complete {
                            return Err(format!("route_{:?}", route.status));
                        }
                        if team == 1 && distance(route.projected_goal, goal) > 0.5 {
                            return Err("short_goal_projection_distance".into());
                        }
                        let drops = route
                            .waypoint_kinds
                            .iter()
                            .filter(|k| **k != navmesh::WaypointKind::Walk)
                            .count();
                        Ok((p, goal, (drops, route.waypoints.len(), route.length_m)))
                    })();
                    match trial {
                        Ok((p, g, (drops, waypoints, length))) => {
                            positions.push(p);
                            slots.push(
                                json!({"team":team,"position":obj(p),"goal":obj(g),"sprint":false}),
                            );
                            evidence.push(json!({"team":team,"position":p,"goal":g,"directed_drop_waypoints":drops,"waypoints":waypoints,"length_m":length,"grounded":true,"blocked":false}));
                            accepted += 1;
                            if accepted == 4 {
                                break 'sampling;
                            }
                        }
                        Err(reason) => {
                            *failures.entry(reason).or_default() += 1;
                        }
                    }
                }
            }
        }
        if accepted != 4 {
            return Err(format!(
                "insufficient team{team} slots: {accepted}; failures={failures:?}"
            )
            .into());
        }
    }
    let config = json!({"format":"mag_bot_population_v1","map_name":world.metadata().map_name,"slots":slots});
    let summary = json!({"format":"mag_bot_population_sampling_v1","world_sha256":hash,"attempts":attempts,"accepted":slots.len(),"failures":failures,"minimum_separation_m":0.71,"human_anchor_excluded":anchor0,"production_follower_validation":"required_separately","slots":evidence});
    for (path, value) in [(&a[5], config), (&a[6], summary.clone())] {
        if let Some(parent) = std::path::Path::new(path).parent() {
            fs::create_dir_all(parent)?;
        }
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        f.write_all(&serde_json::to_vec_pretty(&value)?)?;
        f.write_all(b"\n")?;
    }
    println!(
        "{}",
        json!({"attempts":attempts,"accepted":slots.len(),"failures":failures})
    );
    Ok(())
}
