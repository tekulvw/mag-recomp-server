//! Bot cover in firefights: bots that are shot at or that meet an enemy move
//! to nearby geometry that blocks the enemy's line of fire, fight from it by
//! stepping out to a peek spot and back, and leave it when flanked, when it
//! stops covering them, when their objective needs them, or after a bounded
//! dwell. Specified in docs/bot-cover.md; this is an original design (retail
//! had no bot AI).
use super::*;
use crate::bot_perception::{Cause, clear_view, torso};
use crate::bot_registry::WorldBot;
use crate::development_target::BotPhase;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex, OnceLock};

/// Population-file policy (`cover`). Omitted fields use these defaults.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Policy {
    pub enabled: bool,
    /// Cover is searched within this horizontal distance of the bot.
    pub search_radius_m: f64,
    /// A walking route into cover may be at most this long.
    pub max_route_m: f64,
    /// Cover nearer the threat than this is not taken.
    pub min_threat_distance_m: f64,
    /// The cover's protected direction must be within this angle of the
    /// direction to the threat.
    pub facing_degrees: f64,
    /// An attacker only takes cover that is at most this much farther from
    /// its destination than the bot is now.
    pub attacker_goal_slack_m: f64,
    /// An attacker this close to its destination keeps pushing instead.
    pub attacker_push_m: f64,
    /// Bounding forward from cover needs cover at least this much closer to
    /// the attacker's destination.
    pub bound_gain_m: f64,
    /// Longest stay in one cover for attackers and for defenders.
    pub attacker_dwell_ms: u64,
    pub defender_dwell_ms: u64,
    /// A defender only takes cover this close to its post.
    pub defender_leash_m: f64,
    /// Hidden and exposed phases of the peek cycle, each varied by up to
    /// `jitter_ms` per bot and cycle.
    pub hide_ms: u64,
    pub expose_ms: u64,
    pub jitter_ms: u64,
    /// How often a hidden bot checks that its cover still blocks the threat.
    pub recheck_ms: u64,
    /// After a failed search, the bot looks again after this long.
    pub retry_ms: u64,
    /// After leaving cover, the bot does not take cover again for this long
    /// unless it is hit.
    pub reengage_cooldown_ms: u64,
    /// Route pause cooldown given to perception when a bot leaves cover.
    pub perception_calm_ms: u64,
    /// A move into cover (or to its peek spot) that takes longer fails.
    pub move_timeout_ms: u64,
    /// Two bots never take cover spots closer than this.
    pub spacing_m: f64,
    pub max_candidates: usize,
    pub max_rays_per_tick: usize,
    pub geometry_work_per_tick: usize,
    pub max_routes_per_tick: usize,
    /// Use low cover: crouch behind it (published as Character stance 1,
    /// the client's crouched stance) and stand to fire over it. Off keeps
    /// every bot standing and uses high cover only.
    pub crouch: bool,
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            enabled: false,
            search_radius_m: 16.,
            max_route_m: 24.,
            min_threat_distance_m: 5.,
            facing_degrees: 60.,
            attacker_goal_slack_m: 3.,
            attacker_push_m: 15.,
            bound_gain_m: 4.,
            attacker_dwell_ms: 6000,
            defender_dwell_ms: 20_000,
            defender_leash_m: 18.,
            hide_ms: 1000,
            expose_ms: 2200,
            jitter_ms: 400,
            recheck_ms: 500,
            retry_ms: 1500,
            reengage_cooldown_ms: 5000,
            perception_calm_ms: 6000,
            move_timeout_ms: 8000,
            spacing_m: 1.5,
            max_candidates: 4,
            max_rays_per_tick: 24,
            geometry_work_per_tick: 32_768,
            max_routes_per_tick: 4,
            crouch: false,
        }
    }
}
impl Policy {
    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        let m = |v: f64, lo: f64, hi: f64| v.is_finite() && (lo..=hi).contains(&v);
        if !m(self.search_radius_m, 1., 64.)
            || !m(self.max_route_m, 1., 128.)
            || !m(self.min_threat_distance_m, 0., 64.)
            || !m(self.facing_degrees, 1., 90.)
            || !m(self.attacker_goal_slack_m, 0., 64.)
            || !m(self.attacker_push_m, 0., 256.)
            || !m(self.bound_gain_m, 0., 64.)
            || !(500..=120_000).contains(&self.attacker_dwell_ms)
            || !(500..=300_000).contains(&self.defender_dwell_ms)
            || !m(self.defender_leash_m, 1., 256.)
            || !(100..=10_000).contains(&self.hide_ms)
            || !(100..=10_000).contains(&self.expose_ms)
            || self.jitter_ms > self.hide_ms.min(self.expose_ms)
            || !(50..=5000).contains(&self.recheck_ms)
            || !(50..=60_000).contains(&self.retry_ms)
            || self.reengage_cooldown_ms > 120_000
            || self.perception_calm_ms > 120_000
            || !(1000..=60_000).contains(&self.move_timeout_ms)
            || !m(self.spacing_m, 0., 8.)
            || !(1..=16).contains(&self.max_candidates)
            || !(2..=256).contains(&self.max_rays_per_tick)
            || !(1024..=1_000_000).contains(&self.geometry_work_per_tick)
            || !(1..=64).contains(&self.max_routes_per_tick)
        {
            return Err("invalid_bot_cover_policy");
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Cover spots.

/// Probe heights above the feet: crouched torso and top, standing torso,
/// shoulder and head.
const HEIGHTS: [f64; 5] = [0.55, 0.75, 1.05, 1.45, 1.75];
/// Spacing of samples along walkable boundary edges.
const SAMPLE_SPACING_M: f64 = 1.25;
/// How far beyond a sample the covering surface may stand.
const STATIC_REACH_M: f64 = 1.0;
const STRUCTURE_REACH_M: f64 = 1.5;
/// Sideways step from a high spot to where the bot fires past its edge.
const PEEK_OFFSET_M: f64 = 0.8;
const GRID_M: f64 = 8.;
/// A surface steeper than this (normal's vertical part below it) is a wall.
const WALL_NORMAL_Y: f64 = 0.7;
/// Body height of a crouched bot; its torso (aim point and muzzle) is at
/// 60% of it, as a standing body's is.
pub(crate) const CROUCH_HEIGHT_M: f64 = 1.2;
/// The client's Character stance codes (standing, crouched).
const STANCE_STANDING: u8 = 0;
const STANCE_CROUCHED: u8 = 1;

/// One place a bot can stand next to something that blocks shots from one
/// side.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub(crate) struct Spot {
    pub feet: [f64; 3],
    /// Horizontal unit direction the covering surface protects against.
    pub dir: [f64; 2],
    /// High cover hides a standing body; low cover only a crouched one.
    pub high: bool,
    /// Where a high spot's bot steps to fire past the cover's left or
    /// right edge (relative to `dir`).
    pub peeks: [Option<[f64; 3]>; 2],
    /// Next to a placed structure rather than the static map.
    pub structure: bool,
}

/// Cover spots with a horizontal grid index.
#[derive(Default)]
pub(crate) struct CoverMap {
    spots: Vec<Spot>,
    grid: HashMap<(i32, i32), Vec<u32>>,
    pub(crate) edges: usize,
    pub(crate) samples: usize,
    pub(crate) build_ms: u64,
}
impl CoverMap {
    fn from_spots(mut spots: Vec<Spot>, edges: usize, samples: usize) -> Self {
        // One spot per 1 m cell and facing octant.
        let mut kept = std::collections::HashSet::new();
        spots.retain(|s| {
            let octant = ((s.dir[1].atan2(s.dir[0]) + std::f64::consts::PI)
                / (std::f64::consts::FRAC_PI_4))
                .floor() as i32
                % 8;
            kept.insert((
                s.feet[0].floor() as i32,
                s.feet[2].floor() as i32,
                (s.feet[1] / 2.).floor() as i32,
                octant,
            ))
        });
        let mut grid: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        for (i, s) in spots.iter().enumerate() {
            grid.entry(cell(s.feet)).or_default().push(i as u32);
        }
        Self {
            spots,
            grid,
            edges,
            samples,
            build_ms: 0,
        }
    }
    pub(crate) fn len(&self) -> usize {
        self.spots.len()
    }
    pub(crate) fn high(&self) -> usize {
        self.spots.iter().filter(|s| s.high).count()
    }
    /// Spots within `radius` (horizontally) of `p`.
    pub(crate) fn near(&self, p: [f64; 3], radius: f64) -> impl Iterator<Item = &Spot> {
        let r = (radius / GRID_M).ceil() as i32;
        let (cx, cz) = cell(p);
        (cx - r..=cx + r)
            .flat_map(move |x| (cz - r..=cz + r).map(move |z| (x, z)))
            .filter_map(|k| self.grid.get(&k))
            .flatten()
            .map(|&i| &self.spots[i as usize])
            .filter(move |s| (s.feet[0] - p[0]).hypot(s.feet[2] - p[2]) <= radius)
    }
}
fn cell(p: [f64; 3]) -> (i32, i32) {
    (
        (p[0] / GRID_M).floor() as i32,
        (p[2] / GRID_M).floor() as i32,
    )
}
fn add(a: [f64; 3], d: [f64; 2], k: f64, up: f64) -> [f64; 3] {
    [a[0] + d[0] * k, a[1] + up, a[2] + d[1] * k]
}

/// Classify a spot at `feet` protected along `dir`: `Some(true)` high,
/// `Some(false)` low, None when no wall stands there or it covers only part
/// of a body (a railing, a window). `blocked` returns the normal of the
/// first surface a segment meets.
fn classify(
    feet: [f64; 3],
    dir: [f64; 2],
    reach: f64,
    blocked: &mut dyn FnMut([f64; 3], [f64; 3]) -> Option<[f64; 3]>,
) -> Option<bool> {
    let mut hits = [false; 5];
    for (i, h) in HEIGHTS.iter().enumerate() {
        let from = add(feet, dir, 0., *h);
        hits[i] =
            blocked(from, add(feet, dir, reach, *h)).is_some_and(|n| n[1].abs() < WALL_NORMAL_Y);
        // Low hits decide early whether anything is there at all.
        if i == 1 && !(hits[0] && hits[1]) {
            return None;
        }
    }
    match hits {
        [true, true, true, true, true] => Some(true),
        [true, true, false, false, false] => Some(false),
        _ => None,
    }
}

/// The peek spots of a high spot: a step sideways that is on the walkable
/// surface (`project`), reachable without crossing the cover, and clear of
/// it at standing height.
fn peeks(
    feet: [f64; 3],
    dir: [f64; 2],
    reach: f64,
    blocked: &mut dyn FnMut([f64; 3], [f64; 3]) -> Option<[f64; 3]>,
    project: &mut dyn FnMut([f64; 3]) -> Option<[f64; 3]>,
) -> [Option<[f64; 3]>; 2] {
    // Left of `dir` is (-dir.z, dir.x) in the XZ plane.
    let left = [-dir[1], dir[0]];
    [1., -1.].map(|side| {
        let lateral = [left[0] * side, left[1] * side];
        let p = project(add(feet, lateral, PEEK_OFFSET_M, 0.))?;
        if (p[1] - feet[1]).abs() > 0.5
            || blocked(add(feet, lateral, 0., 1.05), add(p, lateral, 0.35, 1.05)).is_some()
        {
            return None;
        }
        let open = [1.05, 1.45]
            .iter()
            .all(|h| blocked(add(p, dir, 0., *h), add(p, dir, reach + 0.5, *h)).is_none());
        open.then_some(p)
    })
}

fn dist_to_segment_xz(p: [f64; 3], a: [f64; 3], b: [f64; 3]) -> f64 {
    let (dx, dz) = (b[0] - a[0], b[2] - a[2]);
    let len2 = dx * dx + dz * dz;
    let t = if len2 <= 1e-12 {
        0.
    } else {
        (((p[0] - a[0]) * dx + (p[2] - a[2]) * dz) / len2).clamp(0., 1.)
    };
    (p[0] - a[0] - dx * t).hypot(p[2] - a[2] - dz * t)
}

/// Every walkable boundary edge of the navmesh (an edge no link covers),
/// sampled and probed against the static world on the projectile row.
pub(crate) fn build_static(
    world: &collision::CollisionWorld,
    mesh: &navmesh::NavMesh,
) -> Result<CoverMap, &'static str> {
    let started = std::time::Instant::now();
    let bounds = mesh.artifact().bounds.clone();
    let row = world.query_row(crate::collision::PROJECTILE_QUERY_LAYER);
    let regions: Vec<_> = world
        .verified_physics_regions()
        .filter(|r| r.obstacles_verified)
        .map(|r| (r.min, r.max))
        .collect();
    let verified = |p: [f64; 3]| {
        regions.iter().any(|(min, max)| {
            (0..3).all(|i| p[i] >= min[i] && p[i] <= max[i]) && p[1] + 2. <= max[1]
        })
    };
    let mut blocked = |from: [f64; 3], to: [f64; 3]| {
        let mut work = usize::MAX;
        world
            .raycast_row_bounded(from, to, row, &mut work)
            .ok()
            .and_then(|q| q.nearest.map(|h| h.normal))
    };
    // Collect polygons chunk by chunk; a chunk with too many is split.
    let mut polys = std::collections::BTreeSet::new();
    let mut chunks = Vec::new();
    let step = 64.;
    let mut x = bounds.min[0];
    while x < bounds.max[0] {
        let mut z = bounds.min[2];
        while z < bounds.max[2] {
            chunks.push(([x, z], step));
            z += step;
        }
        x += step;
    }
    while let Some(([x, z], size)) = chunks.pop() {
        let min = [x, bounds.min[1] - 1., z];
        let max = [x + size, bounds.max[1] + 1., z + size];
        match mesh.polys_in_box(min, max) {
            Ok(found) => polys.extend(found),
            Err(_) if size > 2. => {
                let h = size * 0.5;
                for (dx, dz) in [(0., 0.), (h, 0.), (0., h), (h, h)] {
                    chunks.push(([x + dx, z + dz], h));
                }
            }
            Err(e) => return Err(e.0),
        }
    }
    let mut spots = Vec::new();
    let (mut edges, mut samples) = (0, 0);
    for poly in polys {
        let Ok(info) = mesh.poly_info(poly) else {
            continue;
        };
        let n = info.vertices.len();
        if n < 3 {
            continue;
        }
        let centroid: [f64; 3] =
            std::array::from_fn(|i| info.vertices.iter().map(|v| v[i]).sum::<f64>() / n as f64);
        for i in 0..n {
            let (a, b) = (info.vertices[i], info.vertices[(i + 1) % n]);
            let length = (b[0] - a[0]).hypot(b[2] - a[2]);
            if length < 0.4
                || info.links.iter().any(|(_, p, q)| {
                    dist_to_segment_xz(*p, a, b) < 0.05 && dist_to_segment_xz(*q, a, b) < 0.05
                })
            {
                continue;
            }
            edges += 1;
            let t = [(b[0] - a[0]) / length, (b[2] - a[2]) / length];
            let mut inward = [-t[1], t[0]];
            if inward[0] * (centroid[0] - a[0]) + inward[1] * (centroid[2] - a[2]) < 0. {
                inward = [-inward[0], -inward[1]];
            }
            let dir = [-inward[0], -inward[1]];
            let count = (length / SAMPLE_SPACING_M).floor().max(1.) as usize;
            for k in 0..count {
                let f = (k as f64 + 0.5) / count as f64;
                let edge: [f64; 3] = std::array::from_fn(|j| a[j] + (b[j] - a[j]) * f);
                let xz = [edge[0] + inward[0] * 0.05, edge[2] + inward[1] * 0.05];
                let y = mesh
                    .poly_heights(poly, &[xz])
                    .ok()
                    .and_then(|v| v[0])
                    .unwrap_or(edge[1]);
                let feet = [xz[0], y, xz[1]];
                samples += 1;
                if !verified(feet) {
                    continue;
                }
                let Some(high) = classify(feet, dir, STATIC_REACH_M, &mut blocked) else {
                    continue;
                };
                let component = mesh.component_id(feet).ok();
                let mut project = |p: [f64; 3]| {
                    let c = component?;
                    mesh.goal_candidates(p, 0.6)
                        .ok()?
                        .into_iter()
                        .find(|(id, q)| *id == c && (q[0] - p[0]).hypot(q[2] - p[2]) < 0.2)
                        .map(|(_, q)| q)
                };
                let peeks = if high {
                    peeks(feet, dir, STATIC_REACH_M, &mut blocked, &mut project)
                } else {
                    [None; 2]
                };
                if high && peeks.iter().all(Option::is_none) {
                    // A wall with no edge to fire past hides but cannot fight.
                    continue;
                }
                spots.push(Spot {
                    feet,
                    dir,
                    high,
                    peeks,
                    structure: false,
                });
            }
        }
    }
    let mut map = CoverMap::from_spots(spots, edges, samples);
    map.build_ms = started.elapsed().as_millis() as u64;
    Ok(map)
}

/// Spots around each standing placed structure: samples around its
/// footprint, projected onto the navmesh, probed against the structure and
/// the static world.
pub(crate) fn build_structures(
    world: &collision::CollisionWorld,
    mesh: &navmesh::NavMesh,
    structures: &crate::bot_structure_collision::StructureObstacles,
) -> CoverMap {
    let started = std::time::Instant::now();
    let body = mesh.body();
    let row = world.query_row(crate::collision::PROJECTILE_QUERY_LAYER);
    let mut blocked = |from: [f64; 3], to: [f64; 3]| {
        let mut work = usize::MAX;
        let s = structures
            .raycast(from, to, &mut work)
            .ok()
            .flatten()
            .map(|h| (h.0, h.2));
        let w = world
            .raycast_row_bounded(from, to, row, &mut work)
            .ok()
            .and_then(|q| q.nearest.map(|h| (h.fraction, h.normal)));
        match (s, w) {
            (Some(a), Some(b)) => Some(if a.0 <= b.0 { a.1 } else { b.1 }),
            (a, b) => a.or(b).map(|v| v.1),
        }
    };
    let mut spots = Vec::new();
    let mut samples = 0;
    for o in structures.obstacles() {
        let pad = body.radius + 0.35;
        let (x0, z0, x1, z1) = (
            o.min[0] - pad,
            o.min[2] - pad,
            o.max[0] + pad,
            o.max[2] + pad,
        );
        if (x1 - x0) * (z1 - z0) > 4096. {
            continue;
        }
        // Each side: its start, end and the direction toward the structure.
        let sides = [
            ([x0, z0], [x1, z0], [0., 1.]),
            ([x1, z0], [x1, z1], [-1., 0.]),
            ([x1, z1], [x0, z1], [0., -1.]),
            ([x0, z1], [x0, z0], [1., 0.]),
        ];
        for (a, b, dir) in sides {
            let length = (b[0] - a[0]).hypot(b[1] - a[1]);
            let count = (length / SAMPLE_SPACING_M).floor().max(1.) as usize;
            for k in 0..count {
                let f = (k as f64 + 0.5) / count as f64;
                let p = [
                    a[0] + (b[0] - a[0]) * f,
                    o.min[1] + 0.5,
                    a[1] + (b[1] - a[1]) * f,
                ];
                samples += 1;
                let Some((component, feet)) = mesh.goal_candidates(p, 2.).ok().and_then(|c| {
                    c.into_iter()
                        .filter(|(_, q)| (q[0] - p[0]).hypot(q[2] - p[2]) < 0.3)
                        .min_by(|x, y| (x.1[1] - p[1]).abs().total_cmp(&(y.1[1] - p[1]).abs()))
                }) else {
                    continue;
                };
                if structures.body_penetrates(feet, body).unwrap_or(true) {
                    continue;
                }
                let Some(high) = classify(feet, dir, STRUCTURE_REACH_M, &mut blocked) else {
                    continue;
                };
                let mut project = |q: [f64; 3]| {
                    mesh.goal_candidates(q, 0.6)
                        .ok()?
                        .into_iter()
                        .find(|(id, r)| *id == component && (r[0] - q[0]).hypot(r[2] - q[2]) < 0.2)
                        .map(|(_, r)| r)
                        .filter(|r| !structures.body_penetrates(*r, body).unwrap_or(true))
                };
                let peeks = if high {
                    peeks(feet, dir, STRUCTURE_REACH_M, &mut blocked, &mut project)
                } else {
                    [None; 2]
                };
                if high && peeks.iter().all(Option::is_none) {
                    continue;
                }
                spots.push(Spot {
                    feet,
                    dir,
                    high,
                    peeks,
                    structure: true,
                });
            }
        }
    }
    let mut map = CoverMap::from_spots(spots, structures.obstacles().len(), samples);
    map.build_ms = started.elapsed().as_millis() as u64;
    map
}

type MapKey = (String, String);
/// Static cover per (collision world, navmesh geometry), shared by every
/// engine in the process, so a profile selected again reuses its spots.
fn static_cache() -> &'static Mutex<HashMap<MapKey, Arc<CoverMap>>> {
    static CACHE: OnceLock<Mutex<HashMap<MapKey, Arc<CoverMap>>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

// ---------------------------------------------------------------------------
// Runtime.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum State {
    /// Walking into the hide spot (or, `fire`, out to the peek spot).
    Moving {
        fire: bool,
    },
    Hidden,
    Exposed,
}

#[derive(Clone, Debug)]
pub(crate) struct Episode {
    life: (u32, u32),
    spot: Spot,
    hide: [f64; 3],
    fire: [f64; 3],
    threat: [f64; 3],
    enemy: Option<u16>,
    state: State,
    until: Duration,
    started: Duration,
    moved_at: Duration,
    arrived: Option<Duration>,
    next_check: Duration,
    attacker: bool,
    cycle: u32,
    bound: u32,
    last_threat: Duration,
    /// Departure held this long for a squadmate's covering fire.
    waiting_since: Option<Duration>,
}

#[derive(Default, Clone, Copy, Serialize)]
struct Stats {
    started: u64,
    arrived: u64,
    bounds: u64,
    peeks: u64,
    searches: u64,
    no_spot: u64,
    rays: u64,
    routes: u64,
    rejected_routes: u64,
    deferred: u64,
    hidden_ms: u64,
    exposed_ms: u64,
    moving_ms: u64,
    busy_us: u64,
    max_tick_us: u64,
    window_max_us: u64,
}

/// Engine-owned cover state.
#[derive(Default)]
pub(crate) struct World {
    override_enabled: Option<bool>,
    override_crouch: Option<bool>,
    map: Option<(MapKey, Arc<CoverMap>)>,
    job: Option<(
        MapKey,
        std::thread::JoinHandle<Result<CoverMap, &'static str>>,
    )>,
    map_failed: Option<MapKey>,
    structures: Option<((u64, u32), Arc<CoverMap>)>,
    structure_job: Option<((u64, u32), std::thread::JoinHandle<CoverMap>)>,
    episodes: BTreeMap<u16, Episode>,
    /// Bots crouched behind low cover, by life.
    crouched: BTreeMap<u16, (u32, u32)>,
    /// Per bot life: no new search before then.
    /// Per bot life: no new search before then; `true` when it is the
    /// cooldown after leaving cover (a hit to a walking bot overrides it).
    next_try: BTreeMap<u16, ((u32, u32), Duration, bool)>,
    ends: BTreeMap<&'static str, u64>,
    cursor: u16,
    stats: Stats,
    last_summary: Option<Duration>,
    last_tick: Option<Duration>,
}
impl World {
    /// Whether the cover layer currently drives this bot.
    pub(crate) fn controls(&self, id: u16) -> bool {
        self.episodes.contains_key(&id)
    }
    /// Body height the bot presents to shots and sight lines.
    pub(crate) fn body_height(&self, id: u16) -> f64 {
        if self.crouched.contains_key(&id) {
            CROUCH_HEIGHT_M
        } else {
            bot_physics::Body::default().height
        }
    }
    pub(crate) fn controlled_ids(&self) -> BTreeSet<u16> {
        self.episodes.keys().copied().collect()
    }
    #[cfg(test)]
    pub(crate) fn episode(&self, id: u16) -> Option<&Episode> {
        self.episodes.get(&id)
    }
    fn clear(&mut self) {
        self.episodes.clear();
        self.crouched.clear();
        self.next_try.clear();
    }
}
#[cfg(test)]
impl Episode {
    pub(crate) fn state(&self) -> State {
        self.state
    }
    pub(crate) fn spot(&self) -> Spot {
        self.spot
    }
    pub(crate) fn fire(&self) -> [f64; 3] {
        self.fire
    }
}

/// The torso a bot hides at a spot: crouched behind low cover.
fn hidden_torso(s: &Spot) -> [f64; 3] {
    if s.high {
        torso(s.feet)
    } else {
        crate::bot_perception::torso_at(s.feet, CROUCH_HEIGHT_M)
    }
}
fn hdist(a: [f64; 3], b: [f64; 3]) -> f64 {
    (a[0] - b[0]).hypot(a[2] - b[2])
}
fn jitter(id: u16, cycle: u32, span: u64) -> u64 {
    if span == 0 {
        return 0;
    }
    let mut x = (u64::from(id) << 32) ^ u64::from(cycle).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 31)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    (x ^ (x >> 29)) % (2 * span + 1)
}
fn phase_ms(base: u64, span: u64, id: u16, cycle: u32) -> Duration {
    Duration::from_millis(base + jitter(id, cycle, span) - span)
}

/// What a bot may be interrupted by cover: ordinary standing or walking,
/// never a passage, queue, supply task, recovery, traffic yield or an
/// objective interaction.
fn eligible(bot: &WorldBot) -> bool {
    bot.target.runtime.is_some_and(|r| {
        r.physics_motion
            .is_some_and(|m| m.grounded && m.slide_velocity == [0.; 2])
            && !r.navmesh_state.on_player
    }) && unclaimed(bot)
}
/// No other task owns the bot's body: a cover episode in progress ends when
/// this stops holding.
fn unclaimed(bot: &WorldBot) -> bool {
    let Some(r) = bot.target.runtime else {
        return false;
    };
    bot.health > 0
        && !matches!(r.phase, BotPhase::Dead)
        && !r.navmesh_state.navigation_aborted
        && !bot.has_supply_action()
        && !bot.task.operator
        && !bot.task.recovering
        && !bot.target.registry_ground_recovery()
        && !bot.drop_committed
        && bot.drop_waiting.is_none()
        && bot.drop_permit.is_none()
        && bot.drop_queue_passage.is_none()
        && bot.drop_holding_goal.is_none()
        && !bot.drop_resume
        && !bot.drop_egress
        && !bot.drop_retreat
        && !bot.drop_reroute
        && bot.traffic_yield.is_none()
}

struct Budget {
    rays: usize,
    work: usize,
    routes: usize,
}

#[derive(Clone, Copy)]
struct Threat {
    point: [f64; 3],
    enemy: Option<u16>,
    last: Duration,
}

impl<W: Write> Engine<W> {
    /// Process-wide switch (`--bot-cover off|on`) over every population.
    pub fn set_bot_cover_override(&mut self, enabled: Option<bool>) {
        self.bot_cover.override_enabled = enabled;
    }
    /// `--bot-cover crouch` also turns low cover (published crouching) on.
    pub fn set_bot_cover_crouch_override(&mut self, crouch: Option<bool>) {
        self.bot_cover.override_crouch = crouch;
    }
    pub(crate) fn bot_cover_policy(&self) -> Option<Policy> {
        let config = self.bot_registry.config.as_ref()?;
        let enabled = self
            .bot_cover
            .override_enabled
            .unwrap_or(config.cover.enabled);
        (enabled && self.bot_perception_policy().is_some()).then(|| Policy {
            crouch: self
                .bot_cover
                .override_crouch
                .unwrap_or(config.cover.crouch),
            ..config.cover.clone()
        })
    }

    /// Keep the static and structure spot maps current; builds run on a
    /// worker thread (inline in tests).
    fn refresh_cover_maps(&mut self, wall: u64) -> Result<(), &'static str> {
        let (Some(world), Some(mesh)) =
            (self.collision_world.as_ref(), self.navmesh_world.as_ref())
        else {
            return Ok(());
        };
        let key = (
            world.source_sha256().to_owned(),
            mesh.artifact().geometry_sha256.clone(),
        );
        let mut records = Vec::new();
        if self.bot_cover.map.as_ref().is_none_or(|(k, _)| *k != key) {
            if let Some(map) = static_cache().lock().unwrap().get(&key).cloned() {
                self.bot_cover.map = Some((key.clone(), map));
            } else if self.bot_cover.map_failed.as_ref() != Some(&key) {
                match self.bot_cover.job.take() {
                    Some((k, job)) if k == key && !job.is_finished() => {
                        self.bot_cover.job = Some((k, job));
                    }
                    Some((k, job)) if k == key => {
                        match job.join().unwrap_or(Err("bot_cover_build_panicked")) {
                            Ok(map) => {
                                records.push(json!({"op":"bot_cover_map","spots":map.len(),
                                    "high":map.high(),"edges":map.edges,"samples":map.samples,
                                    "build_ms":map.build_ms,"collision_world_sha256":key.0}));
                                let map = Arc::new(map);
                                static_cache()
                                    .lock()
                                    .unwrap()
                                    .insert(key.clone(), map.clone());
                                self.bot_cover.map = Some((key.clone(), map));
                            }
                            Err(error) => {
                                records.push(json!({"op":"bot_cover_map","error":error,
                                    "collision_world_sha256":key.0}));
                                self.bot_cover.map_failed = Some(key.clone());
                            }
                        }
                    }
                    _ => {
                        let (w, m) = (world.clone(), mesh.clone());
                        let job = std::thread::Builder::new()
                            .name("bot-cover-map".into())
                            .spawn(move || build_static(&w, &m))
                            .map_err(|_| "bot_cover_thread_unavailable")?;
                        if cfg!(test) {
                            while !job.is_finished() {
                                std::thread::yield_now();
                            }
                        }
                        self.bot_cover.job = Some((key.clone(), job));
                    }
                }
            }
        }
        let snapshot = world.bot_structures_arc().cloned();
        let skey = snapshot.as_ref().map(|s| s.key());
        if self.bot_cover.structures.as_ref().map(|(k, _)| *k) != skey {
            match (snapshot, self.bot_cover.structure_job.take()) {
                (None, _) => self.bot_cover.structures = None,
                (Some(_), Some((k, job))) if Some(k) == skey && !job.is_finished() => {
                    self.bot_cover.structure_job = Some((k, job));
                }
                (Some(_), Some((k, job))) if Some(k) == skey => {
                    if let Ok(map) = job.join() {
                        records.push(json!({"op":"bot_cover_structure_spots","spots":map.len(),
                            "high":map.high(),"structures":map.edges,"build_ms":map.build_ms}));
                        self.bot_cover.structures = Some((k, Arc::new(map)));
                    }
                }
                (Some(snapshot), _) => {
                    let (w, m) = (world.clone(), mesh.clone());
                    let k = snapshot.key();
                    let job = std::thread::Builder::new()
                        .name("bot-cover-structures".into())
                        .spawn(move || build_structures(&w, &m, &snapshot))
                        .map_err(|_| "bot_cover_thread_unavailable")?;
                    if cfg!(test) {
                        while !job.is_finished() {
                            std::thread::yield_now();
                        }
                    }
                    self.bot_cover.structure_job = Some((k, job));
                }
            }
        }
        for record in records {
            self.record(record, wall)?;
        }
        Ok(())
    }

    /// Torso-height position of a living character by object index.
    fn cover_bodies(&self) -> BTreeMap<u16, (u8, [f64; 3])> {
        let mut bodies = BTreeMap::new();
        for a in self.active.iter().chain(self.peers.values()) {
            if !(a.entered_game && a.ready_for_updates && !a.is_mounted() && a.life.is_alive()) {
                continue;
            }
            if let Some(sample) = a.tracking.sample {
                let p = sample.position;
                bodies.insert(
                    a.reservation.player,
                    (a.team(), [p.x.into(), p.y.into(), p.z.into()]),
                );
            }
        }
        for (&id, bot) in &self.bot_registry.bots {
            if let Some(r) = bot.target.runtime
                && bot.health > 0
                && !matches!(r.phase, BotPhase::Dead)
            {
                bodies.insert(id, (bot.target.squad.team(), r.physical_position()));
            }
        }
        bodies
    }

    /// The point a bot is fighting: its weapon's enemy, else perception's.
    fn cover_threat(
        &self,
        id: u16,
        now: Duration,
        bodies: &BTreeMap<u16, (u8, [f64; 3])>,
    ) -> Option<(Threat, bool)> {
        let bot = &self.bot_registry.bots[&id];
        let team = bot.target.squad.team();
        let view = self.bot_perception.threat_view(id, now);
        let held = bot.firearm.target_index();
        let enemy = held.or(view.and_then(|v| v.enemy));
        let live = enemy
            .and_then(|e| bodies.get(&e))
            .filter(|(t, _)| *t != team);
        let point = match (live, view) {
            (Some((_, feet)), _) => torso(*feet),
            (None, Some(v)) => v.point,
            (None, None) => return None,
        };
        let urgent = held.is_some()
            || view.is_some_and(|v| v.damaged || (v.urgent && v.cause >= Cause::NearMiss));
        Some((
            Threat {
                point,
                enemy: live.and(enemy),
                last: if held.is_some() {
                    now
                } else {
                    view.map_or(now, |v| v.last)
                },
            },
            urgent,
        ))
    }

    /// Whether the bot attacks its objective, and the destination it pushes
    /// toward or holds.
    fn cover_role(&self, bot: &WorldBot) -> (bool, Option<[f64; 3]>) {
        let goal = bot
            .task
            .role_goal
            .or(bot.task.goal)
            .map(|g| [g.x.into(), g.y.into(), g.z.into()]);
        let attacker = bot
            .task
            .objective
            .and_then(|o| self.bot_objective_center(o))
            .is_some_and(|(_, owner)| owner != bot.target.squad.team());
        (attacker, goal)
    }

    /// Find a spot near `me` that hides from `threat` and lets the bot fire
    /// back; `forward` demands a bound toward the destination.
    #[allow(clippy::too_many_arguments)]
    fn choose_cover(
        &mut self,
        id: u16,
        policy: &Policy,
        me: [f64; 3],
        threat: [f64; 3],
        (attacker, goal): (bool, Option<[f64; 3]>),
        forward: bool,
        taken: &[[f64; 3]],
        budget: &mut Budget,
    ) -> Result<Option<(Spot, [f64; 3])>, ()> {
        self.bot_cover.stats.searches += 1;
        let cos = policy.facing_degrees.to_radians().cos();
        let mut candidates: Vec<(f64, Spot)> = Vec::new();
        let maps: Vec<Arc<CoverMap>> = self
            .bot_cover
            .map
            .iter()
            .map(|(_, m)| m.clone())
            .chain(self.bot_cover.structures.iter().map(|(_, m)| m.clone()))
            .collect();
        let goal_now = goal.map(|g| hdist(me, g));
        for map in &maps {
            for s in map.near(me, policy.search_radius_m) {
                // Low cover hides only a crouched body.
                if !s.high && !policy.crouch {
                    continue;
                }
                let to = [threat[0] - s.feet[0], threat[2] - s.feet[2]];
                let d = to[0].hypot(to[1]);
                if d < policy.min_threat_distance_m
                    || (to[0] * s.dir[0] + to[1] * s.dir[1]) / d.max(1e-6) < cos
                    || taken.iter().any(|t| hdist(*t, s.feet) < policy.spacing_m)
                {
                    continue;
                }
                let mut cost = hdist(me, s.feet);
                if let (Some(g), Some(now_d)) = (goal, goal_now) {
                    let there = hdist(s.feet, g);
                    if attacker {
                        if there > now_d + policy.attacker_goal_slack_m
                            || (forward && there > now_d - policy.bound_gain_m)
                        {
                            continue;
                        }
                        cost += 0.5 * (there - now_d);
                    } else if there > policy.defender_leash_m {
                        continue;
                    }
                }
                candidates.push((cost, *s));
            }
        }
        candidates.sort_by(|a, b| a.0.total_cmp(&b.0));
        let map_name = self.map_name().to_owned();
        for (_, s) in candidates.into_iter().take(policy.max_candidates) {
            let before = budget.rays;
            // Hidden: the threat cannot see the bot's torso at the spot.
            let hidden = clear_view(
                self.collision_world.as_ref(),
                self.bot_sight_structures(),
                &map_name,
                threat,
                hidden_torso(&s),
                &mut budget.rays,
                &mut budget.work,
            )?;
            if hidden {
                self.bot_cover.stats.rays += (before - budget.rays) as u64;
                continue;
            }
            // Firing: standing up over low cover, or one peek spot past high
            // cover's edge, sees the threat.
            let mut fire = None;
            let low = [s.feet];
            let spots: &[[f64; 3]] = if s.high { &[] } else { &low };
            for p in spots.iter().chain(s.peeks.iter().flatten()) {
                if clear_view(
                    self.collision_world.as_ref(),
                    self.bot_sight_structures(),
                    &map_name,
                    torso(*p),
                    threat,
                    &mut budget.rays,
                    &mut budget.work,
                )? {
                    fire = Some(*p);
                    break;
                }
            }
            self.bot_cover.stats.rays += (before - budget.rays) as u64;
            if let Some(fire) = fire {
                let _ = id;
                return Ok(Some((s, fire)));
            }
        }
        Ok(None)
    }

    /// Plan and start a short walk; false when it is not a complete walking
    /// route within the policy's length.
    fn cover_walk(
        &mut self,
        id: u16,
        to: [f64; 3],
        sprint: bool,
        max_m: f64,
        now: Duration,
        budget: &mut Budget,
    ) -> Result<bool, &'static str> {
        let Some(mesh) = self.navmesh_world.clone() else {
            return Ok(false);
        };
        if budget.routes == 0 {
            return Ok(false);
        }
        budget.routes -= 1;
        self.bot_cover.stats.routes += 1;
        let bot = self.bot_registry.bots.get_mut(&id).unwrap();
        let start = bot.target.runtime.unwrap().physical_position();
        let Ok(route) = mesh.route(start, to, 0.3) else {
            self.bot_cover.stats.rejected_routes += 1;
            return Ok(false);
        };
        if route.status != navmesh::Status::Complete
            || route.length_m > max_m
            || route.waypoints.len() < 2
            || route
                .waypoint_kinds
                .iter()
                .any(|k| *k != navmesh::WaypointKind::Walk)
            || hdist(route.projected_goal, to) > 0.5
        {
            self.bot_cover.stats.rejected_routes += 1;
            return Ok(false);
        }
        match bot.target.begin_registry_route(mesh, route, sprint, now) {
            Ok(()) => Ok(true),
            Err(_) => {
                self.bot_cover.stats.rejected_routes += 1;
                Ok(false)
            }
        }
    }

    /// End a bot's cover episode and hand it back to its objective.
    fn end_cover(
        &mut self,
        id: u16,
        now: Duration,
        reason: &'static str,
        policy: &Policy,
        stop: bool,
        receipts: &mut Vec<Value>,
    ) {
        let Some(e) = self.bot_cover.episodes.remove(&id) else {
            return;
        };
        *self.bot_cover.ends.entry(reason).or_default() += 1;
        self.set_cover_crouch(id, false, now);
        if let Some(bot) = self.bot_registry.bots.get_mut(&id) {
            if stop {
                bot.target.stop_registry_move();
            }
            if reason != "objective_changed" && bot.health > 0 {
                // Objective bots replan their tactical destination; a fixed
                // goal population walks to its slot goal again.
                bot.task.resume_after_supply(now);
                if !self
                    .bot_registry
                    .config
                    .as_ref()
                    .is_some_and(|c| c.autonomous)
                {
                    bot.move_attempted = false;
                }
            }
            self.bot_cover.next_try.insert(
                id,
                (
                    e.life,
                    now + Duration::from_millis(policy.reengage_cooldown_ms),
                    true,
                ),
            );
        }
        self.bot_perception.calm(id, now, policy.perception_calm_ms);
        receipts.push(
            json!({"op":"bot_cover_end","timestamp_ms":now.as_millis() as u64,
            "object_index":id,"generation":e.life.1,"reason":reason,"state":e.state,
            "attacker":e.attacker,"structure":e.spot.structure,"cycles":e.cycle,"bounds":e.bound,
            "in_cover_ms":e.arrived.map_or(0, |a| now.saturating_sub(a).as_millis() as u64),
            "episode_ms":now.saturating_sub(e.started).as_millis() as u64}),
        );
    }

    /// Crouch or stand a bot and publish its Character stance.
    fn set_cover_crouch(&mut self, id: u16, crouch: bool, now: Duration) {
        let Some(bot) = self.bot_registry.bots.get_mut(&id) else {
            return;
        };
        let Some(r) = bot.target.runtime.filter(|_| bot.health > 0) else {
            return;
        };
        let stance = if crouch {
            STANCE_CROUCHED
        } else {
            STANCE_STANDING
        };
        if crouch {
            self.bot_cover
                .crouched
                .insert(id, (bot.allocation_generation, r.generation));
        } else {
            self.bot_cover.crouched.remove(&id);
        }
        if bot.stance == stance {
            return;
        }
        bot.stance = stance;
        let fields = player_tracking::ClientCharacterFields {
            state: Some(replication::CharacterState {
                state: 1,
                stance,
                user_data: [0; 3],
            }),
            ..Default::default()
        };
        if let Ok(packet) = (replication::multiplayer::CharacterOwnerDelta {
            sequence: 2,
            timestamp_ms: now.as_millis() as u32,
            player: id,
            fields: &fields,
        })
        .encode_with_profile(self.wire_profile)
        {
            self.broadcast_bot_packet(packet);
        }
    }

    /// Objective planning took this bot for something else (a new duty, an
    /// operator selection, a blast evacuation); its route is already reset.
    pub(crate) fn preempt_cover(
        &mut self,
        id: u16,
        now: Duration,
        wall: u64,
    ) -> Result<(), &'static str> {
        let Some(policy) = self.bot_cover_policy() else {
            return Ok(());
        };
        let mut receipts = Vec::new();
        self.end_cover(id, now, "objective_changed", &policy, false, &mut receipts);
        for r in receipts {
            self.record(r, wall)?;
        }
        Ok(())
    }

    pub(crate) fn tick_bot_cover(&mut self, now: Duration, wall: u64) -> Result<(), &'static str> {
        let started = std::time::Instant::now();
        let dt = self
            .bot_cover
            .last_tick
            .map_or(Duration::ZERO, |t| now.saturating_sub(t))
            .min(Duration::from_millis(250));
        self.bot_cover.last_tick = Some(now);
        let Some(policy) = self.bot_cover_policy().filter(|_| {
            self.match_started
                && self.match_terminal.is_none()
                && !self.bot_registry.bots.is_empty()
        }) else {
            let ids: Vec<_> = self.bot_cover.episodes.keys().copied().collect();
            for id in ids {
                if let Some(bot) = self.bot_registry.bots.get_mut(&id) {
                    bot.target.stop_registry_move();
                    bot.task.resume_after_supply(now);
                }
            }
            self.bot_cover.clear();
            return Ok(());
        };
        self.refresh_cover_maps(wall)?;
        if self.bot_cover.map.is_none() && self.bot_cover.structures.is_none() {
            return Ok(());
        }
        let bodies = self.cover_bodies();
        let held = self.bot_perception.held_ids();
        let mut receipts = Vec::new();
        let mut budget = Budget {
            rays: policy.max_rays_per_tick,
            work: policy.geometry_work_per_tick,
            routes: policy.max_routes_per_tick,
        };
        // Episodes first: they are cheap unless they relocate.
        let ids: Vec<_> = self.bot_cover.episodes.keys().copied().collect();
        for id in ids {
            self.step_cover_episode(id, now, dt, &policy, &bodies, &mut budget, &mut receipts)?;
        }
        // Then new searches, round robin.
        let mut ids: Vec<_> = self.bot_registry.bots.keys().copied().collect();
        if !ids.is_empty() {
            let offset = ids.partition_point(|&id| id < self.bot_cover.cursor) % ids.len();
            ids.rotate_left(offset);
        }
        for id in ids {
            if self.bot_cover.episodes.contains_key(&id) {
                continue;
            }
            if budget.rays < 2 || budget.routes == 0 || budget.work == 0 {
                self.bot_cover.stats.deferred += 1;
                self.bot_cover.cursor = id;
                break;
            }
            self.bot_cover.cursor = id.wrapping_add(1);
            self.try_start_cover(id, now, &policy, &bodies, &held, &mut budget, &mut receipts)?;
        }
        let busy = started.elapsed().as_micros() as u64;
        let stats = &mut self.bot_cover.stats;
        stats.busy_us += busy;
        stats.max_tick_us = stats.max_tick_us.max(busy);
        stats.window_max_us = stats.window_max_us.max(busy);
        if self
            .bot_cover
            .last_summary
            .is_none_or(|last| now.saturating_sub(last) >= Duration::from_secs(10))
        {
            self.bot_cover.last_summary = Some(now);
            let by_state = |s: fn(&State) -> bool| {
                self.bot_cover
                    .episodes
                    .values()
                    .filter(|e| s(&e.state))
                    .count()
            };
            receipts.push(json!({"op":"bot_cover_summary","timestamp_ms":now.as_millis() as u64,
                "episodes":self.bot_cover.episodes.len(),
                "hidden":by_state(|s| *s == State::Hidden),"exposed":by_state(|s| *s == State::Exposed),
                "moving":by_state(|s| matches!(s, State::Moving{..})),
                "spots":self.bot_cover.map.as_ref().map_or(0, |(_, m)| m.len()),
                "structure_spots":self.bot_cover.structures.as_ref().map_or(0, |(_, m)| m.len()),
                "bots":self.bot_registry.bots.len(),"ends":self.bot_cover.ends,"totals":self.bot_cover.stats}));
            self.bot_cover.stats.window_max_us = 0;
        }
        for r in receipts {
            self.record(r, wall)?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn try_start_cover(
        &mut self,
        id: u16,
        now: Duration,
        policy: &Policy,
        bodies: &BTreeMap<u16, (u8, [f64; 3])>,
        held: &BTreeSet<u16>,
        budget: &mut Budget,
        receipts: &mut Vec<Value>,
    ) -> Result<(), &'static str> {
        let bot = &self.bot_registry.bots[&id];
        let Some(r) = bot.target.runtime else {
            return Ok(());
        };
        let life = (bot.allocation_generation, r.generation);
        let autonomous = self
            .bot_registry
            .config
            .as_ref()
            .is_some_and(|c| c.autonomous);
        if !eligible(bot) || (autonomous && bot.task.objective.is_none()) {
            return Ok(());
        }
        let walking = held.contains(&id) && matches!(r.phase, BotPhase::Relocating);
        let standing = !matches!(r.phase, BotPhase::Relocating) && crate::bot_firearms::safe(bot);
        if !walking && !standing {
            return Ok(());
        }
        let Some((threat, urgent)) = self.cover_threat(id, now, bodies) else {
            return Ok(());
        };
        if !urgent {
            return Ok(());
        }
        let damaged = self
            .bot_perception
            .threat_view(id, now)
            .is_some_and(|v| v.damaged);
        if let Some((l, until, cooldown)) = self.bot_cover.next_try.get(&id)
            && *l == life
            && now < *until
            && !(damaged && walking && *cooldown)
        {
            return Ok(());
        }
        let me = r.physical_position();
        let role = self.cover_role(bot);
        if role.0
            && role
                .1
                .is_some_and(|g| hdist(me, g) <= policy.attacker_push_m)
        {
            return Ok(());
        }
        let taken = self.cover_taken(id, bodies);
        let chosen =
            match self.choose_cover(id, policy, me, threat.point, role, false, &taken, budget) {
                Ok(c) => c,
                Err(()) => {
                    self.bot_cover.stats.deferred += 1;
                    return Ok(());
                }
            };
        let Some((spot, fire)) = chosen else {
            self.bot_cover.stats.no_spot += 1;
            self.bot_cover.next_try.insert(
                id,
                (life, now + Duration::from_millis(policy.retry_ms), false),
            );
            return Ok(());
        };
        if !self.cover_walk(id, spot.feet, true, policy.max_route_m, now, budget)? {
            self.bot_cover.next_try.insert(
                id,
                (life, now + Duration::from_millis(policy.retry_ms), false),
            );
            return Ok(());
        }
        self.bot_cover.stats.started += 1;
        self.bot_cover.episodes.insert(
            id,
            Episode {
                life,
                spot,
                hide: spot.feet,
                fire,
                threat: threat.point,
                enemy: threat.enemy,
                state: State::Moving { fire: false },
                until: now,
                started: now,
                moved_at: now,
                arrived: None,
                next_check: now,
                attacker: role.0,
                cycle: 0,
                bound: 0,
                last_threat: threat.last,
                waiting_since: None,
            },
        );
        receipts.push(json!({"op":"bot_cover_start","timestamp_ms":now.as_millis() as u64,
            "object_index":id,"generation":life.1,"trigger":if walking {"walking"} else {"standing"},
            "attacker":role.0,"position":me,"spot":spot,"fire":fire,"threat":threat.point,
            "enemy":threat.enemy,"distance_m":hdist(me, spot.feet)}));
        Ok(())
    }

    /// Spots other bots hold or walk to, and where other bots stand.
    fn cover_taken(&self, id: u16, bodies: &BTreeMap<u16, (u8, [f64; 3])>) -> Vec<[f64; 3]> {
        self.bot_cover
            .episodes
            .iter()
            .filter(|(i, _)| **i != id)
            .flat_map(|(_, e)| [e.hide, e.fire])
            .chain(
                bodies
                    .iter()
                    .filter(|(i, _)| **i != id)
                    .map(|(_, (_, feet))| *feet),
            )
            .collect()
    }

    #[allow(clippy::too_many_arguments)]
    fn step_cover_episode(
        &mut self,
        id: u16,
        now: Duration,
        dt: Duration,
        policy: &Policy,
        bodies: &BTreeMap<u16, (u8, [f64; 3])>,
        budget: &mut Budget,
        receipts: &mut Vec<Value>,
    ) -> Result<(), &'static str> {
        let Some(bot) = self.bot_registry.bots.get(&id) else {
            self.bot_cover.episodes.remove(&id);
            return Ok(());
        };
        let mut e = self.bot_cover.episodes[&id].clone();
        let r = bot.target.runtime;
        let life = r.map(|r| (bot.allocation_generation, r.generation));
        if life != Some(e.life) || bot.health == 0 {
            // A new life or a death publishes its own standing state.
            self.bot_cover.episodes.remove(&id);
            self.bot_cover.crouched.remove(&id);
            if let Some(bot) = self.bot_registry.bots.get_mut(&id) {
                bot.stance = STANCE_STANDING;
            }
            return Ok(());
        }
        let r = r.unwrap();
        // Something more important owns the bot now; leave its movement be.
        if !unclaimed(bot) {
            self.end_cover(id, now, "preempted", policy, false, receipts);
            return Ok(());
        }
        let ms = dt.as_millis() as u64;
        match e.state {
            State::Moving { .. } => self.bot_cover.stats.moving_ms += ms,
            State::Hidden => self.bot_cover.stats.hidden_ms += ms,
            State::Exposed => self.bot_cover.stats.exposed_ms += ms,
        }
        // Waiting by choice is neither an objective nor a traffic stall.
        {
            let bot = self.bot_registry.bots.get_mut(&id).unwrap();
            bot.task.hold_progress(now);
            if let Some(progress) = bot.traffic_progress.as_mut() {
                progress.at = now;
            }
        }
        if let Some((threat, _)) = self.cover_threat(id, now, bodies) {
            e.threat = threat.point;
            e.enemy = threat.enemy;
            e.last_threat = e.last_threat.max(threat.last);
        }
        let me = r.physical_position();
        let quiet = now.saturating_sub(e.last_threat);
        let sight_memory = Duration::from_millis(
            self.bot_perception_policy()
                .map_or(4000, |p| p.sight_memory_ms),
        );
        if quiet > sight_memory {
            self.end_cover(id, now, "threat_cleared", policy, true, receipts);
            return Ok(());
        }
        let bot = &self.bot_registry.bots[&id];
        let relocating = matches!(r.phase, BotPhase::Relocating);
        let map_name = self.map_name().to_owned();
        match e.state {
            State::Moving { fire } => {
                let target = if fire { e.fire } else { e.hide };
                if !relocating {
                    if hdist(me, target) > 0.75 {
                        self.end_cover(id, now, "move_failed", policy, false, receipts);
                        return Ok(());
                    }
                    if fire {
                        e.state = State::Exposed;
                        e.until = now + phase_ms(policy.expose_ms, policy.jitter_ms, id, e.cycle);
                        self.bot_cover.stats.peeks += 1;
                    } else {
                        if e.arrived.is_none() {
                            e.arrived = Some(now);
                            self.bot_cover.stats.arrived += 1;
                        }
                        e.state = State::Hidden;
                        e.until = now + phase_ms(policy.hide_ms, policy.jitter_ms, id, e.cycle);
                        e.next_check = now;
                        if !e.spot.high {
                            self.set_cover_crouch(id, true, now);
                        }
                    }
                } else if now.saturating_sub(e.moved_at)
                    > Duration::from_millis(policy.move_timeout_ms)
                {
                    self.end_cover(id, now, "move_failed", policy, true, receipts);
                    return Ok(());
                }
            }
            State::Hidden | State::Exposed => {
                if relocating {
                    // Something else moved the bot.
                    self.end_cover(id, now, "preempted", policy, false, receipts);
                    return Ok(());
                }
                let dwell = Duration::from_millis(if e.attacker {
                    policy.attacker_dwell_ms
                } else {
                    policy.defender_dwell_ms
                });
                let dwelled = e.arrived.is_some_and(|a| now.saturating_sub(a) >= dwell);
                if dwelled && e.state == State::Hidden {
                    // Bound forward under a squadmate's fire when one is
                    // fighting from cover nearby; otherwise go now.
                    let team = bot.target.squad.squad_index();
                    let covering = self.bot_cover.episodes.iter().any(|(i, o)| {
                        *i != id
                            && o.state == State::Exposed
                            && hdist(o.fire, me) <= 25.
                            && self
                                .bot_registry
                                .bots
                                .get(i)
                                .is_some_and(|b| b.target.squad.squad_index() == team)
                    });
                    let waiting = self.bot_cover.episodes.iter().any(|(i, o)| {
                        *i != id
                            && o.arrived.is_some()
                            && hdist(o.hide, me) <= 25.
                            && self
                                .bot_registry
                                .bots
                                .get(i)
                                .is_some_and(|b| b.target.squad.squad_index() == team)
                    });
                    let since = *e.waiting_since.get_or_insert(now);
                    if !covering
                        && waiting
                        && now.saturating_sub(since) < Duration::from_millis(1500)
                    {
                        self.bot_cover.episodes.insert(id, e);
                        return Ok(());
                    }
                    if e.attacker {
                        let role = self.cover_role(bot);
                        let taken = self.cover_taken(id, bodies);
                        if let Ok(Some((spot, fire))) =
                            self.choose_cover(id, policy, me, e.threat, role, true, &taken, budget)
                            && self.cover_walk(
                                id,
                                spot.feet,
                                true,
                                policy.max_route_m,
                                now,
                                budget,
                            )?
                        {
                            self.set_cover_crouch(id, false, now);
                            self.bot_cover.stats.bounds += 1;
                            receipts.push(
                                json!({"op":"bot_cover_bound","timestamp_ms":now.as_millis() as u64,
                                "object_index":id,"generation":e.life.1,"from":e.hide,"spot":spot,
                                "covered":covering}),
                            );
                            e.spot = spot;
                            e.hide = spot.feet;
                            e.fire = fire;
                            e.state = State::Moving { fire: false };
                            e.moved_at = now;
                            e.arrived = None;
                            e.bound += 1;
                            e.waiting_since = None;
                            self.bot_cover.episodes.insert(id, e);
                            return Ok(());
                        }
                    }
                    self.end_cover(id, now, "dwell", policy, true, receipts);
                    return Ok(());
                }
                if e.state == State::Hidden && now >= e.next_check {
                    e.next_check = now + Duration::from_millis(policy.recheck_ms);
                    let before = budget.rays;
                    match clear_view(
                        self.collision_world.as_ref(),
                        self.bot_sight_structures(),
                        &map_name,
                        e.threat,
                        hidden_torso(&e.spot),
                        &mut budget.rays,
                        &mut budget.work,
                    ) {
                        Ok(true) => {
                            self.bot_cover.stats.rays += (before - budget.rays) as u64;
                            // Flanked, or the cover is gone: find another.
                            self.end_cover(
                                id,
                                now,
                                if e.spot.structure {
                                    "exposed_structure"
                                } else {
                                    "flanked"
                                },
                                policy,
                                true,
                                receipts,
                            );
                            self.bot_cover.next_try.remove(&id);
                            return Ok(());
                        }
                        Ok(false) => self.bot_cover.stats.rays += (before - budget.rays) as u64,
                        Err(()) => {
                            self.bot_cover.stats.deferred += 1;
                            e.next_check = now;
                        }
                    }
                }
                let reloading = bot.firearm.reloading();
                // Reload and heal hidden: a hurt bot stays down up to two
                // more hidden phases while its self-healing works.
                let hurt =
                    bot.health < 300 && now < e.until + Duration::from_millis(2 * policy.hide_ms);
                if e.state == State::Hidden && now >= e.until && !reloading && !hurt {
                    if !e.spot.high {
                        // Stand up over low cover.
                        self.set_cover_crouch(id, false, now);
                        e.state = State::Exposed;
                        e.until = now + phase_ms(policy.expose_ms, policy.jitter_ms, id, e.cycle);
                        e.cycle += 1;
                        self.bot_cover.stats.peeks += 1;
                    } else if self.cover_walk(id, e.fire, false, 4., now, budget)? {
                        // Step out to the peek spot.
                        e.state = State::Moving { fire: true };
                        e.moved_at = now;
                        e.cycle += 1;
                    } else if budget.routes > 0 {
                        // The peek spot can no longer be reached.
                        self.end_cover(id, now, "peek_unreachable", policy, false, receipts);
                        return Ok(());
                    }
                } else if e.state == State::Exposed && (now >= e.until || reloading) {
                    if !e.spot.high {
                        self.set_cover_crouch(id, true, now);
                        e.state = State::Hidden;
                        e.until = now + phase_ms(policy.hide_ms, policy.jitter_ms, id, e.cycle);
                        e.next_check = now;
                    } else if self.cover_walk(id, e.hide, false, 4., now, budget)? {
                        e.state = State::Moving { fire: false };
                        e.moved_at = now;
                    } else if budget.routes > 0 {
                        self.end_cover(id, now, "hide_unreachable", policy, false, receipts);
                        return Ok(());
                    }
                }
            }
        }
        self.bot_cover.episodes.insert(id, e);
        Ok(())
    }
}
