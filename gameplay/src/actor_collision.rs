//! Bounded dynamic body broad phase. Authoritative humans are obstacles, never corrected.
//! Entries are boxes, or capsules when inserted as a capsule body; a capsule
//! mover meets a capsule entry exactly (`sweep_vertical_capsules`) and any
//! other pairing through the two bodies' boxes.
//!
//! A capsule bot's entry is a trail: the capsule at every pose of the path it
//! reserved, its current pose last, so a capsule mover meets a moving capsule
//! bot round (contact normals run axis to axis) rather than through the box
//! around that path. A capsule mover that already overlaps a bot's capsule
//! (crowded spawns) may make any move that does not deepen the overlap, and
//! `separation` says how far it must move to leave it.
//!
//! A box bot's entry also keeps the box of its current pose. A box mover
//! that already stands inside that pose (two bots stepping into each other)
//! may likewise make any move that does not deepen the overlap, including
//! falling onto the floor, and `separation` parts it along its nearest
//! horizontal exit. Without this, two such boxes block every move of each
//! other, gravity included, and stand stuck in the air until one dies.
use crate::bot_physics::{Body, Point};
use crate::capsule_sweep::{VerticalCapsule, sweep_vertical_capsules};
use std::collections::{BTreeMap, BTreeSet};
const CELL: f64 = 2.;
/// Every Character in a full client pool (`world_capacity::MAX_WORLD_CHARACTERS`)
/// plus vehicle and bunker bodies, with room to spare.
const MAX_BODIES: usize = 2 * crate::world_capacity::MAX_WORLD_CHARACTERS;
const MAX_CELLS: usize = 256;
#[derive(Clone, Copy, Debug)]
pub(crate) struct Bounds {
    pub min: Point,
    pub max: Point,
}
impl Bounds {
    pub fn body(p: Point, b: Body) -> Self {
        // A capsule's rounded bottom may reach slightly below its feet.
        let low = match b.shape.capsule() {
            None => p[1],
            Some(_) => p[1] + b.bottom().min(0.0),
        };
        Self {
            min: [p[0] - b.radius, low, p[2] - b.radius],
            max: [p[0] + b.radius, p[1] + b.height, p[2] + b.radius],
        }
    }
    pub fn union(self, other: Self) -> Self {
        Self {
            min: std::array::from_fn(|i| self.min[i].min(other.min[i])),
            max: std::array::from_fn(|i| self.max[i].max(other.max[i])),
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct Hit {
    pub id: u16,
    pub fraction: f64,
    pub normal: Point,
}
/// Space a bot reserved by moving: the box its sweep covered and the feet it
/// moved from and to. A capsule body's entry follows the path between them;
/// a box body's uses only the box.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Reservation {
    pub bounds: Bounds,
    pub path: [Point; 2],
}
/// Most capsule poses one trail entry keeps; a longer path is sampled more
/// sparsely (its box still covers all of it for the broad phase).
const MAX_TRAIL: usize = 24;
#[derive(Clone)]
struct Entry {
    bounds: Bounds,
    allow_overlap_escape: bool,
    /// A capsule body entry: its dimensions and the centres of every pose it
    /// occupies or reserved, the current pose last.
    capsule: Option<Trail>,
    /// A reserved box body's current pose (its actual body, not the space
    /// it swept): the box a mover already inside may part from.
    pose: Option<Bounds>,
}
#[derive(Clone)]
struct Trail {
    capsule: VerticalCapsule,
    centres: Vec<Point>,
}
impl Trail {
    fn at(pose: Option<(Point, VerticalCapsule)>) -> Option<Self> {
        pose.map(|(centre, capsule)| Self {
            capsule,
            centres: vec![centre],
        })
    }
    fn current(&self) -> Point {
        self.centres[self.centres.len() - 1]
    }
}
#[derive(Default)]
pub(crate) struct ActorGrid {
    bodies: BTreeMap<u16, Entry>,
    cells: BTreeMap<(i32, i32), BTreeSet<u16>>,
    /// The bots' body shape: the shape `probe` asks with.
    probe_shape: crate::bot_physics::BodyShape,
}
#[derive(Clone, Copy)]
pub(crate) struct Query<'a> {
    pub grid: &'a ActorGrid,
    pub actor: u16,
}
impl ActorGrid {
    /// A grid whose occupancy probes take the bots' `shape` (see `probe`).
    pub(crate) fn with_probe_shape(shape: crate::bot_physics::BodyShape) -> Self {
        Self {
            probe_shape: shape,
            ..Self::default()
        }
    }
    /// The body a bot-placement query (is this spot or path clear for a
    /// bot?) asks with: the default box, or the bots' capsule, which then
    /// meets other capsule bodies exactly rather than through their boxes.
    pub(crate) fn probe(&self) -> Body {
        Body::default().with_shape(self.probe_shape)
    }
    #[cfg(test)]
    pub(crate) fn diagnostic_entries(&self) -> Vec<(u16, Bounds, bool)> {
        self.bodies
            .iter()
            .map(|(&id, entry)| (id, entry.bounds, entry.allow_overlap_escape))
            .collect()
    }

    /// Insert a current authoritative human body, permitting an overlapping
    /// bot to move out if the human has entered its existing position.
    #[cfg(test)]
    pub fn insert(&mut self, id: u16, bounds: Bounds) -> Result<(), &'static str> {
        self.insert_entry(id, bounds, true, None, None)
    }
    /// Bot swept envelopes contain empty space as well as actual body poses.
    /// Starting inside one cannot authorize the human-overlap escape exception.
    pub fn insert_reserved(&mut self, id: u16, bounds: Bounds) -> Result<(), &'static str> {
        self.insert_entry(id, bounds, false, None, None)
    }
    /// As `insert`, for a human standing at `feet` with `body`. A box body
    /// is exactly `insert(id, Bounds::body(feet, body))`.
    pub fn insert_body(&mut self, id: u16, feet: Point, body: Body) -> Result<(), &'static str> {
        let trail = Trail::at(capsule_at(feet, body));
        self.insert_entry(id, Bounds::body(feet, body), true, trail, None)
    }
    /// As `insert_reserved`, for a bot's actual pose rather than an envelope.
    pub fn insert_reserved_body(
        &mut self,
        id: u16,
        feet: Point,
        body: Body,
    ) -> Result<(), &'static str> {
        let trail = Trail::at(capsule_at(feet, body));
        let bounds = Bounds::body(feet, body);
        let pose = trail.is_none().then_some(bounds);
        self.insert_entry(id, bounds, false, trail, pose)
    }
    /// As `insert_reserved`, for a bot whose `body` moved through the feet
    /// poses `path` (straight between consecutive ones), its current pose
    /// last, and whose sweep covered `swept`. A capsule body becomes a trail
    /// of capsules along the path, spaced at most half its radius apart while
    /// the path is short; a box body is `insert_reserved` of the box covering
    /// `swept` and every pose, which keeps the last pose as its current one.
    pub fn insert_reserved_path(
        &mut self,
        id: u16,
        path: &[Point],
        body: Body,
        swept: Bounds,
    ) -> Result<(), &'static str> {
        let (&first, rest) = path.split_first().ok_or("actor_collision_empty_path")?;
        let mut feet = vec![first];
        let spacing = 0.5 * body.radius;
        let mut from = first;
        for &to in rest {
            let length = (0..3)
                .map(|i| (to[i] - from[i]).powi(2))
                .sum::<f64>()
                .sqrt();
            if !length.is_finite() {
                return Err("actor_collision_invalid_bounds");
            }
            let pieces = ((length / spacing).ceil() as usize).clamp(1, MAX_TRAIL);
            for k in 1..=pieces {
                let f = k as f64 / pieces as f64;
                feet.push(std::array::from_fn(|i| from[i] + (to[i] - from[i]) * f));
            }
            from = to;
        }
        if feet.len() > MAX_TRAIL {
            // Keep the start and the current pose, and evenly spaced ones between.
            let last = feet.len() - 1;
            feet = (0..MAX_TRAIL)
                .map(|k| feet[k * last / (MAX_TRAIL - 1)])
                .collect();
        }
        let bounds = feet
            .iter()
            .fold(swept, |b, &p| b.union(Bounds::body(p, body)));
        let trail = body.shape.capsule().map(|(capsule, centre)| Trail {
            capsule,
            centres: feet.iter().map(|p| [p[0], p[1] + centre, p[2]]).collect(),
        });
        let pose = trail
            .is_none()
            .then(|| Bounds::body(feet[feet.len() - 1], body));
        self.insert_entry(id, bounds, false, trail, pose)
    }
    fn insert_entry(
        &mut self,
        id: u16,
        bounds: Bounds,
        allow_overlap_escape: bool,
        capsule: Option<Trail>,
        pose: Option<Bounds>,
    ) -> Result<(), &'static str> {
        if !self.bodies.contains_key(&id) && self.bodies.len() >= MAX_BODIES {
            return Err("actor_collision_body_limit");
        }
        let cells = cells(bounds)?;
        self.remove(id);
        self.bodies.insert(
            id,
            Entry {
                bounds,
                allow_overlap_escape,
                capsule,
                pose,
            },
        );
        for cell in cells {
            self.cells.entry(cell).or_default().insert(id);
        }
        Ok(())
    }
    pub fn remove(&mut self, id: u16) {
        if let Some(old) = self.bodies.remove(&id)
            && let Ok(cells) = cells(old.bounds)
        {
            for cell in cells {
                if let Some(ids) = self.cells.get_mut(&cell) {
                    ids.remove(&id);
                    if ids.is_empty() {
                        self.cells.remove(&cell);
                    }
                }
            }
        }
    }
    pub fn sweep(
        &self,
        actor: u16,
        from: Point,
        to: Point,
        b: Body,
        budget: &mut usize,
    ) -> Result<Option<Hit>, &'static str> {
        let bounds = Bounds::body(from, b).union(Bounds::body(to, b));
        let mut candidates = BTreeSet::new();
        for cell in cells(bounds)? {
            if let Some(ids) = self.cells.get(&cell) {
                candidates.extend(ids.iter().copied());
            }
        }
        let mut nearest: Option<Hit> = None;
        for id in candidates {
            if id == actor {
                continue;
            }
            if *budget == 0 {
                return Err("actor_collision_query_limit");
            }
            *budget -= 1;
            let entry = &self.bodies[&id];
            let contact = match (capsule_at(from, b), &entry.capsule) {
                (Some((centre, mover)), Some(trail)) => {
                    let end = capsule_at(to, b).map_or(centre, |(c, _)| c);
                    let mut first: Option<(f64, Point)> = None;
                    for &other_centre in &trail.centres {
                        if let Some(hit) = sweep_capsules(
                            centre,
                            end,
                            mover,
                            other_centre,
                            trail.capsule,
                            entry.allow_overlap_escape,
                        ) && first.is_none_or(|f| hit.0 < f.0)
                        {
                            first = Some(hit);
                        }
                    }
                    first
                }
                _ => match entry.pose.filter(|_| !entry.allow_overlap_escape) {
                    Some(pose) if parts_from_box(from, to, b, pose) => None,
                    _ => sweep_box(from, to, b, entry.bounds, entry.allow_overlap_escape),
                },
            };
            if let Some((fraction, normal)) = contact
                && nearest.is_none_or(|h| fraction < h.fraction)
            {
                nearest = Some(Hit {
                    id,
                    fraction,
                    normal,
                });
            }
        }
        Ok(nearest)
    }
    /// Whether `id` is a reserved (bot or vehicle) entry rather than an
    /// authoritative human.
    pub(crate) fn reserved(&self, id: u16) -> bool {
        self.bodies
            .get(&id)
            .is_some_and(|entry| !entry.allow_overlap_escape)
    }
    pub fn occupied(&self, actor: u16, p: Point, b: Body) -> Result<bool, &'static str> {
        Ok(self.sweep(actor, p, p, b, &mut 256)?.is_some())
    }
    /// For a capsule `body` with its feet at `feet`: the horizontal
    /// displacement that would take it out of every capsule body it overlaps
    /// (each one's current pose), summed over them; None when it overlaps
    /// none. Each overlap contributes its depth along the horizontal
    /// direction from the other body's axis to this one's; bodies whose axes
    /// coincide part along a direction fixed by their ids. A box `body` is
    /// parted from the box bots' current poses it overlaps instead
    /// (`box_separation`).
    pub fn separation(
        &self,
        actor: u16,
        feet: Point,
        body: Body,
        budget: &mut usize,
    ) -> Result<Option<Point>, &'static str> {
        let Some((centre, mover)) = capsule_at(feet, body) else {
            return self.box_separation(actor, feet, body, budget);
        };
        let mut candidates = BTreeSet::new();
        for cell in cells(Bounds::body(feet, body))? {
            if let Some(ids) = self.cells.get(&cell) {
                candidates.extend(ids.iter().copied());
            }
        }
        let mut push = [0.; 3];
        let mut any = false;
        for id in candidates {
            let Some(trail) = self.bodies[&id].capsule.as_ref().filter(|_| id != actor) else {
                continue;
            };
            if *budget == 0 {
                return Err("actor_collision_query_limit");
            }
            *budget -= 1;
            let other = trail.current();
            let reach = mover.radius + trail.capsule.radius;
            let gap =
                ((centre[1] - other[1]).abs() - mover.half_height - trail.capsule.half_height)
                    .max(0.);
            let (dx, dz) = (centre[0] - other[0], centre[2] - other[2]);
            let across = dx.hypot(dz);
            let depth = reach - across.hypot(gap);
            if depth <= 1e-9 {
                continue;
            }
            let away = if across > 1e-6 {
                [dx / across, 0., dz / across]
            } else if actor < id {
                [1., 0., 0.]
            } else {
                [-1., 0., 0.]
            };
            for i in [0, 2] {
                push[i] += away[i] * depth;
            }
            any = true;
        }
        Ok(any.then_some(push))
    }
    /// For a box `body` with its feet at `feet`: the horizontal displacement
    /// that would take it out of every box bot's current pose it overlaps,
    /// summed over them; None when it overlaps none. Each overlap contributes
    /// its depth along the horizontal axis it is shallower on, away from the
    /// other pose's centre (by their ids when the centres coincide on it). A
    /// body that overlaps another less vertically than horizontally (one
    /// standing on the other's head) is not parted sideways.
    fn box_separation(
        &self,
        actor: u16,
        feet: Point,
        body: Body,
        budget: &mut usize,
    ) -> Result<Option<Point>, &'static str> {
        let mine = Bounds::body(feet, body);
        let mut candidates = BTreeSet::new();
        for cell in cells(mine)? {
            if let Some(ids) = self.cells.get(&cell) {
                candidates.extend(ids.iter().copied());
            }
        }
        let mut push = [0.; 3];
        let mut any = false;
        for id in candidates {
            let entry = &self.bodies[&id];
            let Some(other) = entry
                .pose
                .filter(|_| id != actor && !entry.allow_overlap_escape)
            else {
                continue;
            };
            if *budget == 0 {
                return Err("actor_collision_query_limit");
            }
            *budget -= 1;
            let depth: [f64; 3] = std::array::from_fn(|i| {
                mine.max[i].min(other.max[i]) - mine.min[i].max(other.min[i])
            });
            if depth.iter().any(|d| *d <= 1e-9) {
                continue;
            }
            let axis = if depth[0] <= depth[2] { 0 } else { 2 };
            if depth[1] < depth[axis] {
                continue;
            }
            let apart = (mine.min[axis] + mine.max[axis]) - (other.min[axis] + other.max[axis]);
            let sign = if apart.abs() > 1e-9 {
                apart.signum()
            } else if actor < id {
                1.
            } else {
                -1.
            };
            push[axis] += sign * depth[axis];
            any = true;
        }
        Ok(any.then_some(push))
    }
}
/// Whether a box mover of `b` moving from `from` to `to` starts inside the
/// body box `pose` and never goes deeper into it: the straight move does not
/// lengthen the way out through the nearest face. That way out is the least
/// of the distances to the six faces of the space `b`'s feet cannot enter,
/// which is concave along a straight move, so a move that does not lengthen
/// it at the start never does. A stationary query still finds the overlap.
fn parts_from_box(from: Point, to: Point, b: Body, pose: Bounds) -> bool {
    let top = match b.shape.capsule() {
        None => pose.max[1],
        Some(_) => pose.max[1] - b.bottom().min(0.),
    };
    let min = [
        pose.min[0] - b.radius,
        pose.min[1] - b.height,
        pose.min[2] - b.radius,
    ];
    let max = [pose.max[0] + b.radius, top, pose.max[2] + b.radius];
    if !(0..3).all(|i| from[i] > min[i] + 1e-8 && from[i] < max[i] - 1e-8) {
        return false;
    }
    let delta: Point = std::array::from_fn(|i| to[i] - from[i]);
    let length = delta.iter().map(|d| d * d).sum::<f64>().sqrt();
    if length <= 1e-9 {
        return false;
    }
    // (distance to a face, its rate of change along the move)
    let faces = (0..3).flat_map(|i| [(from[i] - min[i], delta[i]), (max[i] - from[i], -delta[i])]);
    let nearest = faces.clone().map(|f| f.0).fold(f64::INFINITY, f64::min);
    let rate = faces
        .filter(|f| f.0 <= nearest + 1e-9)
        .map(|f| f.1)
        .fold(f64::INFINITY, f64::min);
    rate <= 1e-9 * length
}
fn cells(bounds: Bounds) -> Result<Vec<(i32, i32)>, &'static str> {
    if (0..3).any(|i| {
        !bounds.min[i].is_finite()
            || !bounds.max[i].is_finite()
            || bounds.min[i] > bounds.max[i]
            || bounds.min[i].abs() > 100000.
            || bounds.max[i].abs() > 100000.
    }) {
        return Err("actor_collision_invalid_bounds");
    }
    let min = [
        (bounds.min[0] / CELL).floor() as i32,
        (bounds.min[2] / CELL).floor() as i32,
    ];
    let max = [
        (bounds.max[0] / CELL).floor() as i32,
        (bounds.max[2] / CELL).floor() as i32,
    ];
    if i64::from(max[0] - min[0] + 1) * i64::from(max[1] - min[1] + 1) > MAX_CELLS as i64 {
        return Err("actor_collision_cell_limit");
    }
    Ok((min[0]..=max[0])
        .flat_map(|x| (min[1]..=max[1]).map(move |z| (x, z)))
        .collect())
}
fn capsule_at(feet: Point, body: Body) -> Option<(Point, VerticalCapsule)> {
    body.shape
        .capsule()
        .map(|(capsule, centre)| ([feet[0], feet[1] + centre, feet[2]], capsule))
}
/// A capsule mover against one capsule of an entry. A genuine human overlap
/// may only escape: a start inside it is clear when it moves away along the
/// contact normal, as `sweep_box` lets it move out along its nearest exit. A
/// start inside a bot's capsule is clear for any move that does not approach
/// its axis (the contact normal is the gradient of the distance between the
/// axes, and that distance is convex along a straight move, so the overlap
/// never deepens); a stationary query still reports the overlap.
fn sweep_capsules(
    from: Point,
    to: Point,
    mover: VerticalCapsule,
    other_centre: Point,
    other: VerticalCapsule,
    allow_overlap_escape: bool,
) -> Option<(f64, Point)> {
    let (fraction, normal) = sweep_vertical_capsules(from, to, mover, other_centre, other)?;
    let delta: Point = std::array::from_fn(|i| to[i] - from[i]);
    let away: f64 = (0..3).map(|i| delta[i] * normal[i]).sum();
    if fraction == 0. && allow_overlap_escape && away > 1e-9 {
        return None;
    }
    let length = delta.iter().map(|d| d * d).sum::<f64>().sqrt();
    if fraction == 0.
        && !allow_overlap_escape
        && length > 1e-9
        && away >= -1e-9 * length
        && overlaps(from, mover, other_centre, other)
    {
        return None;
    }
    Some((fraction, normal))
}
/// Whether capsules centred at `a` and `b` overlap (deeper than tangency).
fn overlaps(a: Point, first: VerticalCapsule, b: Point, second: VerticalCapsule) -> bool {
    let gap = ((a[1] - b[1]).abs() - first.half_height - second.half_height).max(0.);
    let reach = first.radius + second.radius - crate::capsule_sweep::CONTACT_TOLERANCE;
    (a[0] - b[0]).hypot(a[2] - b[2]).hypot(gap) < reach
}
fn sweep_box(
    from: Point,
    to: Point,
    b: Body,
    other: Bounds,
    allow_overlap_escape: bool,
) -> Option<(f64, Point)> {
    // A capsule mover meets a box through its own box, whose bottom may sit
    // slightly below its feet.
    let top = match b.shape.capsule() {
        None => other.max[1],
        Some(_) => other.max[1] - b.bottom().min(0.),
    };
    let min = [
        other.min[0] - b.radius,
        other.min[1] - b.height,
        other.min[2] - b.radius,
    ];
    let max = [other.max[0] + b.radius, top, other.max[2] + b.radius];
    let delta: Point = std::array::from_fn(|i| to[i] - from[i]);
    if (0..3).all(|i| from[i] > min[i] + 1e-8 && from[i] < max[i] - 1e-8) {
        // A human may authoritatively enter an existing bot. Permit only an
        // improving escape along the nearest exit, never push the human.
        let mut exit = (f64::INFINITY, 0, 0.);
        for i in 0..3 {
            for (depth, sign) in [(from[i] - min[i], -1.), (max[i] - from[i], 1.)] {
                if depth < exit.0 {
                    exit = (depth, i, sign);
                }
            }
        }
        if allow_overlap_escape && delta[exit.1] * exit.2 > 1e-9 {
            return None;
        }
        let mut normal = [0.; 3];
        normal[exit.1] = exit.2;
        return Some((0., normal));
    }
    // Retain the entering face even when rounding puts the start just inside
    // it (within the boundary tolerance above). Clamping the time first loses
    // the normal and incorrectly lets an inward move pass through the body.
    let mut enter = f64::NEG_INFINITY;
    let mut leave = 1f64;
    let mut normal = [0.; 3];
    for i in 0..3 {
        if delta[i].abs() < 1e-12 {
            if from[i] <= min[i] + 1e-8 || from[i] >= max[i] - 1e-8 {
                return None;
            }
            continue;
        }
        let a = (min[i] - from[i]) / delta[i];
        let c = (max[i] - from[i]) / delta[i];
        let near = a.min(c);
        let far = a.max(c);
        if near >= enter {
            enter = near;
            normal = [0.; 3];
            normal[i] = if delta[i] > 0. { -1. } else { 1. };
        }
        leave = leave.min(far);
        if enter > leave {
            return None;
        }
    }
    let entering_boundary = (0..3).any(|i| {
        (normal[i] < 0. && from[i] - min[i] <= 1e-8) || (normal[i] > 0. && max[i] - from[i] <= 1e-8)
    });
    if leave < 0. || enter > 1. || normal == [0.; 3] || (enter < 0. && !entering_boundary) {
        None
    } else {
        Some((enter.max(0.), normal))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn grid(p: Point) -> ActorGrid {
        let mut g = ActorGrid::default();
        g.insert(2, Bounds::body(p, Body::default())).unwrap();
        g
    }
    #[test]
    fn a_full_character_pool_and_its_vehicles_fit_the_broad_phase() {
        let mut g = ActorGrid::default();
        let at = |id: u16| [f64::from(id % 32) * 3., 0., f64::from(id / 32) * 3.];
        // Every Character index, then a vehicle body per Character.
        for id in 0..2 * crate::world_capacity::MAX_WORLD_CHARACTERS as u16 {
            g.insert(id, Bounds::body(at(id), Body::default())).unwrap();
        }
        // Moving a body never needs a new entry; one more body is refused.
        g.insert(0, Bounds::body([1., 0., 1.], Body::default()))
            .unwrap();
        assert_eq!(
            g.insert(u16::MAX, Bounds::body([1., 0., 1.], Body::default())),
            Err("actor_collision_body_limit")
        );
    }
    #[test]
    fn swept_human_blocks_tunneling_and_reports_contact() {
        let g = grid([3., 0., 0.]);
        let h = g
            .sweep(1, [0., 0., 0.], [6., 0., 0.], Body::default(), &mut 100)
            .unwrap()
            .expect("must not pass living body");
        assert!((h.fraction - 2.3 / 6.).abs() < 1e-9);
        assert_eq!(h.normal, [-1., 0., 0.]);
    }
    #[test]
    fn vertical_separation_and_self_are_clear() {
        let g = grid([3., 3., 0.]);
        assert!(
            g.sweep(1, [0., 0., 0.], [6., 0., 0.], Body::default(), &mut 100)
                .unwrap()
                .is_none()
        );
        let g = grid([0.; 3]);
        assert!(!g.occupied(2, [0.; 3], Body::default()).unwrap());
        assert!(g.occupied(1, [0.; 3], Body::default()).unwrap());
    }
    #[test]
    fn descending_hits_top_and_keeps_gravity_contact_normal() {
        let g = grid([0.; 3]);
        let h = g
            .sweep(1, [0., 5., 0.], [0., 0., 0.], Body::default(), &mut 100)
            .unwrap()
            .unwrap();
        assert_eq!(h.normal, [0., 1., 0.]);
        assert!((h.fraction - 0.64).abs() < 1e-9);
    }
    #[test]
    fn query_limits_fail_closed() {
        let g = grid([3., 0., 0.]);
        assert!(
            g.sweep(1, [0.; 3], [6., 0., 0.], Body::default(), &mut 0)
                .is_err()
        );
        assert!(
            g.sweep(1, [0.; 3], [1000., 0., 1000.], Body::default(), &mut 100)
                .is_err()
        );
    }
    #[test]
    fn reserved_side_contact_preserves_tangent_gravity_but_blocks_inward_motion() {
        let body = Body::default();
        let neighbor = Bounds::body([0.7, -0.2, 0.0], body);
        let mut grid = ActorGrid::default();
        grid.insert_reserved(2, neighbor).unwrap();
        // Side-by-side airborne bodies must not acquire vertical support from
        // a horizontal face, including floating point noise at that face.
        for x in [0.0, 1e-10, -1e-10] {
            let from = [x, 0.0, 0.0];
            assert!(
                grid.sweep(1, from, [x, -0.1, 0.0], body, &mut 8)
                    .unwrap()
                    .is_none()
            );
            assert!(
                grid.sweep(1, from, [0.1, -0.1, 0.0], body, &mut 8)
                    .unwrap()
                    .is_some()
            );
            assert!(
                grid.sweep(1, from, [-0.1, -0.1, 0.0], body, &mut 8)
                    .unwrap()
                    .is_none()
            );
        }
    }
    #[test]
    fn reserved_phantom_corner_cannot_authorize_crossing_actual_bot() {
        let body = Body::default();
        let current = Bounds::body([2., 0., 2.], body);
        let envelope = Bounds::body([0., 0., 0.], body).union(current);
        let from = [0.5, 0., 2.];
        let to = [2., 0., 2.5];
        let mut grid = ActorGrid::default();
        grid.insert_reserved(2, envelope).unwrap();
        // The start is only inside an empty corner of the swept union, while
        // the requested move would enter the actual current neighbor's body.
        assert!(sweep_box(from, from, body, current, true).is_none());
        assert!(sweep_box(to, to, body, current, true).is_some());
        let hit = grid.sweep(1, from, to, body, &mut 8).unwrap().unwrap();
        assert_eq!(hit.id, 2);
        assert_eq!(hit.fraction, 0.);
    }
    #[test]
    fn genuine_human_overlap_can_escape_without_moving_human() {
        let body = Body::default();
        let human = Bounds::body([2., 0., 2.], body);
        let mut grid = ActorGrid::default();
        grid.insert(2, human).unwrap();
        let from = [2.3, 0., 2.];
        assert!(
            grid.sweep(1, from, [3., 0., 2.], body, &mut 8)
                .unwrap()
                .is_none()
        );
        assert!(
            grid.sweep(1, from, [2., 0., 2.], body, &mut 8)
                .unwrap()
                .is_some()
        );
        assert_eq!(grid.bodies[&2].bounds.min, human.min);
        assert_eq!(grid.bodies[&2].bounds.max, human.max);
        // Replacing an entry also replaces its collision policy.
        grid.insert_reserved(2, human).unwrap();
        assert!(
            grid.sweep(1, from, [3., 0., 2.], body, &mut 8)
                .unwrap()
                .is_some()
        );
    }
    fn capsule_body() -> Body {
        Body::default().with_shape(crate::bot_physics::BodyShape::Capsule {
            radius: 0.4,
            half_height: 0.5,
            centre_height: 0.9,
        })
    }
    #[test]
    fn capsule_pairs_meet_at_their_summed_radius_and_slide_off_round() {
        let body = capsule_body();
        let mut grid = ActorGrid::default();
        grid.insert_reserved_body(2, [3., 0., 0.], body).unwrap();
        // Head on: stops with the axes 0.8 m apart, normal straight back.
        let h = grid
            .sweep(1, [0., 0., 0.], [6., 0., 0.], body, &mut 100)
            .unwrap()
            .unwrap();
        assert!((h.fraction - 2.2 / 6.).abs() < 1e-9);
        assert!((h.normal[0] + 1.).abs() < 1e-9 && h.normal[1].abs() < 1e-9);
        // Offset sideways the contact normal turns, so the slide goes round.
        let h = grid
            .sweep(1, [0., 0., 0.5], [6., 0., 0.5], body, &mut 100)
            .unwrap()
            .unwrap();
        assert!(h.normal[0] < 0. && h.normal[2] > 0.3, "{h:?}");
        // Past the summed radius (where the boxes would still touch at a
        // corner) the capsules are clear.
        assert!(
            grid.sweep(1, [0., 0., 0.81], [6., 0., 0.81], body, &mut 100)
                .unwrap()
                .is_none()
        );
        // Standing on top: the rounded tops meet along the axis.
        let h = grid
            .sweep(1, [3., 5., 0.], [3., 0., 0.], body, &mut 100)
            .unwrap()
            .unwrap();
        assert!((h.normal[1] - 1.).abs() < 1e-9);
        assert!((h.fraction - (5. - 1.8) / 5.).abs() < 1e-9);
    }
    #[test]
    fn a_capsule_human_overlap_can_only_be_escaped() {
        let body = capsule_body();
        let mut grid = ActorGrid::default();
        grid.insert_body(2, [2., 0., 2.], body).unwrap();
        let from = [2.3, 0., 2.];
        assert!(
            grid.sweep(1, from, [3., 0., 2.], body, &mut 8)
                .unwrap()
                .is_none()
        );
        assert!(
            grid.sweep(1, from, [2., 0., 2.], body, &mut 8)
                .unwrap()
                .is_some()
        );
        // A reserved bot body lets a capsule mover part from it too (any
        // move that does not approach its axis), but never deeper in, and a
        // stationary query still finds the overlap.
        grid.insert_reserved_body(2, [2., 0., 2.], body).unwrap();
        assert!(
            grid.sweep(1, from, [3., 0., 2.], body, &mut 8)
                .unwrap()
                .is_none()
        );
        assert!(
            grid.sweep(1, from, [2., 0., 2.], body, &mut 8)
                .unwrap()
                .is_some()
        );
        assert!(grid.occupied(1, from, body).unwrap());
        // A box mover meets the capsule entry through its box, and a capsule
        // mover meets a reserved box envelope through its own box.
        assert!(
            grid.sweep(1, [0., 0., 2.], [4., 0., 2.], Body::default(), &mut 8)
                .unwrap()
                .is_some()
        );
        let mut grid = ActorGrid::default();
        grid.insert_reserved(2, Bounds::body([3., 0., 0.], Body::default()))
            .unwrap();
        let h = grid
            .sweep(1, [0., 0., 0.], [6., 0., 0.], body, &mut 8)
            .unwrap()
            .unwrap();
        assert!((h.fraction - 2.25 / 6.).abs() < 1e-9);
    }
    #[test]
    fn box_body_entries_are_unchanged() {
        let mut a = ActorGrid::default();
        let mut b = ActorGrid::default();
        a.insert(2, Bounds::body([3., 0., 0.], Body::default()))
            .unwrap();
        b.insert_body(2, [3., 0., 0.], Body::default()).unwrap();
        for (from, to) in [([0., 0., 0.], [6., 0., 0.]), ([3., 5., 0.], [3., 0., 0.])] {
            let x = a
                .sweep(1, from, to, Body::default(), &mut 8)
                .unwrap()
                .unwrap();
            let y = b
                .sweep(1, from, to, Body::default(), &mut 8)
                .unwrap()
                .unwrap();
            assert_eq!((x.fraction, x.normal), (y.fraction, y.normal));
        }
    }
    #[test]
    fn broad_phase_body_capacity_and_crossing_reservations_are_bounded() {
        let mut grid = ActorGrid::default();
        for id in 0..MAX_BODIES as u16 {
            grid.insert(
                id,
                Bounds::body([f64::from(id) * 4., 0., 0.], Body::default()),
            )
            .unwrap();
        }
        let mut budget = 8;
        assert!(
            grid.sweep(
                500,
                [-2., 0., 0.],
                [2., 0., 0.],
                Body::default(),
                &mut budget
            )
            .unwrap()
            .is_some()
        );
        assert!(budget >= 5, "far bodies must stay outside candidate work");
        assert!(
            grid.insert(
                MAX_BODIES as u16,
                Bounds::body([2000., 0., 0.], Body::default())
            )
            .is_err()
        );
        let mut grid = ActorGrid::default();
        grid.insert_reserved(
            1,
            Bounds::body([-2., 0., 0.], Body::default())
                .union(Bounds::body([2., 0., 0.], Body::default())),
        )
        .unwrap();
        assert!(
            grid.sweep(2, [0., 0., -2.], [0., 0., 2.], Body::default(), &mut 8)
                .unwrap()
                .is_some(),
            "crossing published swept paths must reserve space"
        );
    }

    #[test]
    fn a_capsule_bot_overlap_permits_parting_tangential_and_vertical_moves_only() {
        let body = capsule_body();
        let mut grid = ActorGrid::default();
        grid.insert_reserved_body(2, [0., 0., 0.], body).unwrap();
        let from = [0.5, 0., 0.];
        for (to, clear) in [
            ([0.9, 0., 0.], true),    // straight out
            ([0.5, 0., 0.3], true),   // tangential: the axes only part
            ([0.5, -0.1, 0.], true),  // settling onto the ground beneath
            ([0.5, 0.3, 0.], true),   // a step lift
            ([0.3, 0., 0.], false),   // deeper in
            ([0.45, 0., 0.3], false), // round but approaching first
        ] {
            assert_eq!(
                grid.sweep(1, from, to, body, &mut 8).unwrap().is_none(),
                clear,
                "{to:?}"
            );
        }
        // A capsule standing on top may not be pushed down into it.
        assert!(
            grid.sweep(1, [0., 1.7, 0.], [0., 1.6, 0.], body, &mut 8)
                .unwrap()
                .is_some()
        );
        // A box mover keeps the box rule: no way out of a reserved body.
        assert!(
            grid.sweep(1, from, [0.9, 0., 0.], Body::default(), &mut 8)
                .unwrap()
                .is_some()
        );
    }
    #[test]
    fn a_capsule_trail_reserves_its_path_round_and_meets_movers_axis_to_axis() {
        let body = capsule_body();
        let mut grid = ActorGrid::default();
        let swept = Bounds::body([0., 0., 0.], Body::default())
            .union(Bounds::body([4., 0., 0.], Body::default()));
        grid.insert_reserved_path(2, &[[0., 0., 0.], [4., 0., 0.]], body, swept)
            .unwrap();
        // Crossing the reserved path is refused, as for the box envelope.
        let h = grid
            .sweep(1, [2., 0., -3.], [2., 0., 3.], body, &mut 8)
            .unwrap()
            .expect("the path is reserved");
        assert!((h.normal[2] + 1.).abs() < 1e-6, "{h:?}");
        // Off the round end of the trail (where its box would still reach)
        // a body is clear, and a mover glancing the end is turned round it.
        assert!(!grid.occupied(1, [4.6, 0., -0.6], body).unwrap());
        assert!(
            grid.occupied(1, [4.6, 0., -0.6], Body::default()).unwrap(),
            "a box meets the trail's box"
        );
        let h = grid
            .sweep(1, [4.5, 0., -3.], [4.5, 0., 3.], body, &mut 8)
            .unwrap()
            .unwrap();
        assert!(h.normal[0] > 0.3 && h.normal[2] < 0., "{h:?}");
        // The current pose is the last one.
        let mut budget = 8;
        let push = grid
            .separation(1, [4.5, 0., 0.], body, &mut budget)
            .unwrap()
            .unwrap();
        assert!((push[0] - 0.3).abs() < 1e-9 && push[2] == 0., "{push:?}");
        assert!(
            grid.separation(1, [0.5, 0., 0.], body, &mut budget)
                .unwrap()
                .is_none(),
            "only the current pose is parted from"
        );
    }
    #[test]
    fn separation_is_the_overlap_depth_away_from_each_axis() {
        let body = capsule_body();
        let mut grid = ActorGrid::default();
        grid.insert_reserved_body(2, [0., 0., 0.], body).unwrap();
        grid.insert_reserved_body(3, [1., 0., 0.], body).unwrap();
        let mut budget = 8;
        // Between the two: pushed from both, mostly out of the nearer one.
        let push = grid
            .separation(1, [0.6, 0., 0.3], body, &mut budget)
            .unwrap()
            .unwrap();
        let d2 = 0.6f64.hypot(0.3);
        let d3 = 0.4f64.hypot(0.3);
        let expected = [
            0.6 / d2 * (0.8 - d2) - 0.4 / d3 * (0.8 - d3),
            0.3 / d2 * (0.8 - d2) + 0.3 / d3 * (0.8 - d3),
        ];
        assert!((push[0] - expected[0]).abs() < 1e-9, "{push:?}");
        assert!((push[2] - expected[1]).abs() < 1e-9, "{push:?}");
        // Touching is not overlapping; a box body is never parted.
        assert!(
            grid.separation(1, [-0.8, 0., 0.], body, &mut budget)
                .unwrap()
                .is_none()
        );
        assert!(
            grid.separation(1, [0.3, 0., 0.], Body::default(), &mut budget)
                .unwrap()
                .is_none()
        );
        // Coincident axes part in opposite directions fixed by the ids.
        let mut grid = ActorGrid::default();
        grid.insert_reserved_body(1, [0., 0., 0.], body).unwrap();
        grid.insert_reserved_body(2, [0., 0., 0.], body).unwrap();
        let a = grid.separation(1, [0.; 3], body, &mut 8).unwrap().unwrap();
        let b = grid.separation(2, [0.; 3], body, &mut 8).unwrap().unwrap();
        assert!(a[0] > 0. && (a[0] + b[0]).abs() < 1e-12, "{a:?} {b:?}");
    }
    #[test]
    fn a_box_body_path_is_exactly_its_swept_box() {
        let body = Body::default();
        let swept = Bounds::body([0., 0., 0.], body).union(Bounds::body([2., 0., 1.], body));
        let mut a = ActorGrid::default();
        let mut b = ActorGrid::default();
        a.insert_reserved(2, swept).unwrap();
        b.insert_reserved_path(2, &[[0., 0., 0.], [2., 0., 1.]], body, swept)
            .unwrap();
        assert_eq!(a.bodies[&2].bounds.min, b.bodies[&2].bounds.min);
        assert_eq!(a.bodies[&2].bounds.max, b.bodies[&2].bounds.max);
        assert!(b.bodies[&2].capsule.is_none() && !b.bodies[&2].allow_overlap_escape);
    }

    #[test]
    fn placement_probes_take_the_bots_shape() {
        // A box world probes with the default box, exactly as before.
        let grid = ActorGrid::default();
        let probe = grid.probe();
        assert_eq!(probe.shape, crate::bot_physics::BodyShape::Box);
        assert_eq!(
            (probe.radius, probe.height),
            (Body::default().radius, Body::default().height)
        );
        // A capsule world probes with the capsule, which meets a standing
        // capsule bot round: a spot diagonally off it (clear of the round
        // body, inside its box) is free.
        let body = capsule_body();
        let mut grid = ActorGrid::with_probe_shape(body.shape);
        grid.insert_reserved_body(2, [0., 0., 0.], body).unwrap();
        assert_eq!(grid.probe().radius, 0.4);
        let corner = [0.6, 0., 0.6];
        assert!(!grid.occupied(1, corner, grid.probe()).unwrap());
        assert!(grid.occupied(1, corner, Body::default()).unwrap());
        // Straight beside it the round body still occupies the spot.
        assert!(grid.occupied(1, [0.75, 0., 0.], grid.probe()).unwrap());
    }
    #[test]
    fn a_box_bot_overlap_permits_non_deepening_moves_and_falling_only() {
        let body = Body::default();
        let mut grid = ActorGrid::default();
        grid.insert_reserved_body(2, [0., 0., 0.], body).unwrap();
        // 0.26 m apart on x, 0.04 m on z: the nearest way out is +x.
        let from = [0.26, 0., 0.04];
        let clear = |to: Point| grid.sweep(1, from, to, body, &mut 8).unwrap().is_none();
        assert!(clear([0.6, 0., 0.04]), "parting along the nearest exit");
        assert!(clear([0.26, -0.3, 0.04]), "falling onto the floor");
        assert!(clear([0.26, 0., 0.5]), "tangential");
        assert!(!clear([0., 0., 0.04]), "into the other body");
        assert!(!clear([0.1, 0., 0.3]), "deeper on the nearest exit");
        // Standing still it is still inside the other bot.
        assert!(grid.occupied(1, from, body).unwrap());
        // Coincident bodies may part in any direction.
        assert!(
            grid.sweep(1, [0.; 3], [0., 0., -0.5], body, &mut 8)
                .unwrap()
                .is_none()
        );
    }
    #[test]
    fn a_box_bot_inside_only_the_swept_space_or_a_human_keeps_the_old_rules() {
        let body = Body::default();
        // The path's swept space beyond the current pose still blocks.
        let mut grid = ActorGrid::default();
        let swept = Bounds::body([0., 0., 0.], body).union(Bounds::body([3., 0., 0.], body));
        grid.insert_reserved_path(2, &[[3., 0., 0.]], body, swept)
            .unwrap();
        assert!(
            grid.sweep(1, [0.2, 0., 0.], [-0.5, 0., 0.], body, &mut 8)
                .unwrap()
                .is_some()
        );
        // Inside the current pose it may part, but not cross it.
        let from = [3.3, 0., 0.];
        assert!(
            grid.sweep(1, from, [3.8, 0., 0.], body, &mut 8)
                .unwrap()
                .is_none()
        );
        assert!(
            grid.sweep(1, from, [2.0, 0., 0.], body, &mut 8)
                .unwrap()
                .is_some()
        );
        // A human: only its nearest exit, never falling inside it.
        let mut grid = ActorGrid::default();
        grid.insert_body(2, [0., 0., 0.], body).unwrap();
        let from = [0.26, 0., 0.04];
        assert!(
            grid.sweep(1, from, [0.6, 0., 0.04], body, &mut 8)
                .unwrap()
                .is_none()
        );
        assert!(
            grid.sweep(1, from, [0.26, 0., 0.5], body, &mut 8)
                .unwrap()
                .is_some()
        );
        // A plain reserved box (a vehicle or bunker) has no pose to part from.
        let mut grid = ActorGrid::default();
        grid.insert_reserved(2, Bounds::body([0., 0., 0.], body))
            .unwrap();
        assert!(
            grid.sweep(1, from, [0.6, 0., 0.04], body, &mut 8)
                .unwrap()
                .is_some()
        );
        assert_eq!(grid.separation(1, from, body, &mut 8).unwrap(), None);
    }
    #[test]
    fn box_separation_is_the_shallower_horizontal_depth_away_from_the_pose() {
        let body = Body::default();
        let mut grid = ActorGrid::default();
        grid.insert_reserved_body(2, [0., 0., 0.], body).unwrap();
        let mut budget = 8;
        let push = grid
            .separation(1, [0.26, 0., 0.04], body, &mut budget)
            .unwrap()
            .unwrap();
        assert!((push[0] - 0.44).abs() < 1e-9 && push[1] == 0. && push[2] == 0.);
        let push = grid
            .separation(1, [0.04, 0., -0.5], body, &mut budget)
            .unwrap()
            .unwrap();
        assert!(push[0] == 0. && (push[2] + 0.2).abs() < 1e-9);
        // Coincident: parted by id order; touching is not overlapping; one
        // standing on the other's head is not pushed sideways.
        let push = grid
            .separation(1, [0.; 3], body, &mut budget)
            .unwrap()
            .unwrap();
        assert!((push[0] - 0.7).abs() < 1e-9);
        assert_eq!(
            grid.separation(1, [0.7, 0., 0.], body, &mut budget)
                .unwrap(),
            None
        );
        assert_eq!(
            grid.separation(1, [0.3, 1.7, 0.], body, &mut budget)
                .unwrap(),
            None
        );
    }
}
