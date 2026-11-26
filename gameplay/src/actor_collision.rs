//! Bounded dynamic body broad phase. Authoritative humans are obstacles, never corrected.
//! Entries are boxes, or capsules when inserted as a capsule body; a capsule
//! mover meets a capsule entry exactly (`sweep_vertical_capsules`) and any
//! other pairing through the two bodies' boxes.
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
#[derive(Clone, Copy)]
struct Entry {
    bounds: Bounds,
    allow_overlap_escape: bool,
    /// The capsule centre and dimensions of a capsule body entry.
    capsule: Option<(Point, VerticalCapsule)>,
}
#[derive(Default)]
pub(crate) struct ActorGrid {
    bodies: BTreeMap<u16, Entry>,
    cells: BTreeMap<(i32, i32), BTreeSet<u16>>,
}
#[derive(Clone, Copy)]
pub(crate) struct Query<'a> {
    pub grid: &'a ActorGrid,
    pub actor: u16,
}
impl ActorGrid {
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
        self.insert_entry(id, bounds, true, None)
    }
    /// Bot swept envelopes contain empty space as well as actual body poses.
    /// Starting inside one cannot authorize the human-overlap escape exception.
    pub fn insert_reserved(&mut self, id: u16, bounds: Bounds) -> Result<(), &'static str> {
        self.insert_entry(id, bounds, false, None)
    }
    /// As `insert`, for a human standing at `feet` with `body`. A box body
    /// is exactly `insert(id, Bounds::body(feet, body))`.
    pub fn insert_body(&mut self, id: u16, feet: Point, body: Body) -> Result<(), &'static str> {
        self.insert_entry(id, Bounds::body(feet, body), true, capsule_at(feet, body))
    }
    /// As `insert_reserved`, for a bot's actual pose rather than an envelope.
    pub fn insert_reserved_body(
        &mut self,
        id: u16,
        feet: Point,
        body: Body,
    ) -> Result<(), &'static str> {
        self.insert_entry(id, Bounds::body(feet, body), false, capsule_at(feet, body))
    }
    fn insert_entry(
        &mut self,
        id: u16,
        bounds: Bounds,
        allow_overlap_escape: bool,
        capsule: Option<(Point, VerticalCapsule)>,
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
            let entry = self.bodies[&id];
            let contact = match (capsule_at(from, b), entry.capsule) {
                (Some((centre, mover)), Some((other_centre, other))) => sweep_capsules(
                    centre,
                    capsule_at(to, b).map_or(centre, |(c, _)| c),
                    mover,
                    other_centre,
                    other,
                    entry.allow_overlap_escape,
                ),
                _ => sweep_box(from, to, b, entry.bounds, entry.allow_overlap_escape),
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
    pub fn occupied(&self, actor: u16, p: Point, b: Body) -> Result<bool, &'static str> {
        Ok(self.sweep(actor, p, p, b, &mut 256)?.is_some())
    }
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
/// A capsule mover against a capsule entry. A genuine human overlap may only
/// escape: a start inside it is clear when it moves away along the contact
/// normal, as `sweep_box` lets it move out along its nearest exit.
fn sweep_capsules(
    from: Point,
    to: Point,
    mover: VerticalCapsule,
    other_centre: Point,
    other: VerticalCapsule,
    allow_overlap_escape: bool,
) -> Option<(f64, Point)> {
    let (fraction, normal) = sweep_vertical_capsules(from, to, mover, other_centre, other)?;
    let away: f64 = (0..3).map(|i| (to[i] - from[i]) * normal[i]).sum();
    if fraction == 0. && allow_overlap_escape && away > 1e-9 {
        return None;
    }
    Some((fraction, normal))
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
        // A reserved bot body allows no escape.
        grid.insert_reserved_body(2, [2., 0., 2.], body).unwrap();
        assert!(
            grid.sweep(1, from, [3., 0., 2.], body, &mut 8)
                .unwrap()
                .is_some()
        );
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
}
