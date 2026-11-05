//! Local passage access geometry, independent of maps and actor roles.
use crate::{actor_collision::ActorGrid, bot_physics::Body, navmesh, replication};
use std::time::Duration;

#[derive(Clone)]
pub(super) struct Progress {
    pub generation: u32,
    pub objective: Option<u16>,
    pub role_slot: u8,
    pub position: replication::CharacterPosition,
    pub at: Duration,
}
#[derive(Clone)]
pub(super) struct HoldingAccess {
    pub requester: u16,
    pub generation: u32,
    pub source_since: Duration,
    pub lane: Vec<[[f64; 3]; 2]>,
    pub until: Duration,
}
#[derive(Clone)]
pub(super) struct Yield {
    pub requester: u16,
    pub generation: u32,
    pub objective: Option<u16>,
    pub role_slot: u8,
    pub lane: Vec<[[f64; 3]; 2]>,
    pub started: Duration,
    pub goal: Option<replication::CharacterPosition>,
    pub attempt: usize,
    pub retry_at: Duration,
    pub resume: bool,
}

/// The current physical approach ends before certified departure. No flight is
/// predicted or admitted by this helper; the existing certificate owns that.
pub(super) fn approach_prefix(
    current: [f64; 3],
    route: &navmesh::Route,
    cursor: usize,
    drop: u32,
) -> Option<Vec<[[f64; 3]; 2]>> {
    let end = route
        .waypoint_kinds
        .iter()
        .enumerate()
        .find_map(|(i, kind)| {
            (*kind == navmesh::WaypointKind::DropStart && route.waypoint_drop_ids[i] == Some(drop))
                .then_some(i)
        })?;
    if cursor > end {
        return None;
    }
    let mut from = current;
    Some(
        route.waypoints[cursor..=end]
            .iter()
            .map(|&to| {
                let segment = [from, to];
                from = to;
                segment
            })
            .collect(),
    )
}

/// Bounded nearby ground access, stopping before an off-mesh flight. Used only
/// after an actual stalled body contact, not as a map-wide corridor reservation.
pub(super) fn local_prefix(
    current: [f64; 3],
    route: &navmesh::Route,
    cursor: usize,
    mut length: f64,
) -> Vec<[[f64; 3]; 2]> {
    let mut from = current;
    let mut lane = Vec::new();
    for i in cursor..route.waypoints.len() {
        if route.waypoint_kinds[i] == navmesh::WaypointKind::DropEnd {
            break;
        }
        let mut to = route.waypoints[i];
        let distance = (to[0] - from[0]).hypot(to[2] - from[2]);
        if distance > length {
            let fraction = length / distance;
            to = std::array::from_fn(|axis| from[axis] + fraction * (to[axis] - from[axis]));
        }
        lane.push([from, to]);
        from = to;
        length -= distance;
        if length <= 0. || route.waypoint_kinds[i] == navmesh::WaypointKind::DropStart {
            break;
        }
    }
    lane
}

/// Use the same body sweeps as physical movement, including humans, vertical
/// separation and published swept reservations. Exhausted work fails closed.
pub(super) fn approach_blocker(
    actors: &ActorGrid,
    actor: u16,
    approach: &[[[f64; 3]; 2]],
    budget: &mut usize,
) -> Result<Option<u16>, &'static str> {
    for &[from, to] in approach {
        // A long diagonal envelope can exceed ActorGrid's broad-phase cell cap.
        // Small contiguous sweeps preserve exactly the same collision semantics.
        let extent = (0..3).map(|i| (to[i] - from[i]).abs()).fold(0., f64::max);
        let pieces = (extent / 8.).ceil().max(1.) as usize;
        if pieces > 64 {
            return Err("bot_traffic_approach_extent");
        }
        let mut start = from;
        for i in 1..=pieces {
            if *budget == 0 {
                return Err("bot_traffic_query_limit");
            }
            *budget -= 1;
            let end = std::array::from_fn(|axis| {
                from[axis] + (to[axis] - from[axis]) * i as f64 / pieces as f64
            });
            if let Some(hit) = actors.sweep(actor, start, end, Body::default(), budget)? {
                return Ok(Some(hit.id));
            }
            start = end;
        }
    }
    Ok(None)
}

/// Pad exclusion uses a round horizontal lane and interpolated route height,
/// avoiding large diagonal AABBs and unrelated floors above/below the lane.
pub(super) fn in_lane(p: [f64; 3], lane: &[[[f64; 3]; 2]], spacing: f64) -> bool {
    lane.iter().any(|&[a, b]| {
        let dx = b[0] - a[0];
        let dz = b[2] - a[2];
        let length = dx * dx + dz * dz;
        let t = if length > 0. {
            ((p[0] - a[0]) * dx + (p[2] - a[2]) * dz) / length
        } else {
            0.
        }
        .clamp(0., 1.);
        let height = a[1] + (b[1] - a[1]) * t;
        (p[1] - height).abs() < Body::default().height
            && (p[0] - a[0] - t * dx).hypot(p[2] - a[2] - t * dz) < spacing
    })
}

/// Reserve only access corridors that actually cross at body height. A passage
/// already in certified flight no longer reserves its ground approach.
pub(super) fn lanes_conflict(a: &[[[f64; 3]; 2]], b: &[[[f64; 3]; 2]], budget: &mut usize) -> bool {
    // Upright boxes can meet at diagonal corners. A circumscribed horizontal
    // radius conservatively includes them regardless of route orientation.
    let spacing = 2. * Body::default().radius * std::f64::consts::SQRT_2;
    for &[p, q] in a {
        for &[r, s] in b {
            if *budget == 0 {
                return true;
            }
            *budget -= 1;
            if [p, q].iter().any(|&x| in_lane(x, &[[r, s]], spacing))
                || [r, s].iter().any(|&x| in_lane(x, &[[p, q]], spacing))
            {
                return true;
            }
            let d = [q[0] - p[0], q[2] - p[2]];
            let e = [s[0] - r[0], s[2] - r[2]];
            let cross = |x: [f64; 2], y: [f64; 2]| x[0] * y[1] - x[1] * y[0];
            let det = cross(d, e);
            if det.abs() < 1e-9 {
                continue;
            }
            let delta = [r[0] - p[0], r[2] - p[2]];
            let t = cross(delta, e) / det;
            let u = cross(delta, d) / det;
            if (0. ..=1.).contains(&t)
                && (0. ..=1.).contains(&u)
                && (p[1] + t * (q[1] - p[1]) - r[1] - u * (s[1] - r[1])).abs()
                    < Body::default().height
            {
                return true;
            }
        }
    }
    false
}

/// Two certified passages exclude each other for their whole service only when
/// bodies on their actual launch pads, trajectories or landings can meet.
/// The coarse protected AABB (a square entry buffer plus the whole flight box)
/// is deliberately not used: parallel drops off one ledge whose swept bodies
/// never touch remain independent exits and may be served concurrently.
pub(super) fn passages_exclusive(
    a: &navmesh::DropLink,
    b: &navmesh::DropLink,
    entry_buffer_m: f64,
) -> bool {
    let body = Body::default();
    let path = |link: &navmesh::DropLink| -> Vec<[[f64; 3]; 2]> {
        let points: Vec<_> = std::iter::once(link.launch)
            .chain(link.trajectory.iter().map(|m| m.position))
            .chain(std::iter::once(link.landing))
            .collect();
        points.windows(2).map(|w| [w[0], w[1]]).collect()
    };
    // A launch pad is where a committing body stands before departure. Its
    // approach direction is route-specific, so the full square is retained.
    let on_pad = |link: &navmesh::DropLink, p: [f64; 3]| {
        let reach = entry_buffer_m + body.radius;
        (p[0] - link.launch[0]).abs() <= reach
            && (p[2] - link.launch[2]).abs() <= reach
            && (p[1] - link.launch[1]).abs() < body.height
    };
    let (pa, pb) = (path(a), path(b));
    let mut budget = usize::MAX;
    lanes_conflict(&pa, &pb, &mut budget)
        || pb.iter().flatten().any(|&p| on_pad(a, p))
        || pa.iter().flatten().any(|&p| on_pad(b, p))
}

/// Unordered exclusive passage pairs, computed once per loaded navmesh.
pub(super) fn exclusive_pairs(
    links: &[navmesh::DropLink],
    entry_buffer_m: f64,
) -> std::collections::BTreeSet<(u32, u32)> {
    let mut pairs = std::collections::BTreeSet::new();
    for (i, a) in links.iter().enumerate() {
        for b in &links[i + 1..] {
            if passages_exclusive(a, b, entry_buffer_m) {
                pairs.insert((a.id.min(b.id), a.id.max(b.id)));
            }
        }
    }
    pairs
}

#[cfg(test)]
pub(super) fn synthetic_link(id: u32, launch: [f64; 3], landing: [f64; 3]) -> navmesh::DropLink {
    // SYNTHETIC: straight interpolated flight, not a certified game trace.
    let trajectory = (1..=20)
        .map(|i| {
            let t = f64::from(i) / 20.;
            let mut m = crate::bot_physics::Motion::new(std::array::from_fn(|axis| {
                launch[axis] + (landing[axis] - launch[axis]) * t
            }));
            m.grounded = i == 20;
            m
        })
        .collect();
    navmesh::DropLink {
        id,
        world_sha256: "0".repeat(64),
        launch,
        landing,
        speed: crate::retail_movement::WALK_FORWARD_MPS,
        max_drop_height: launch[1] - landing[1],
        trajectory,
        trajectory_sha256: String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actor_collision::Bounds;
    #[test]
    fn parallel_drops_off_one_ledge_are_independent_but_crossing_drops_exclude() {
        // Two drops whose 1.5 m launch squares make their protected AABBs touch,
        // while their bodies stay about 2.6 m apart for the whole flight.
        let a = synthetic_link(1, [0., 4., 0.], [0., 1.5, 5.]);
        let b = synthetic_link(2, [3., 4., -4.], [2.6, 1.5, 4.5]);
        let buffer = 1.5;
        let protected = |l: &navmesh::DropLink| {
            crate::development_bot_movement::protected_drop_bounds(l, buffer)
        };
        assert!(
            (0..3).all(|i| protected(&a).min[i] <= protected(&b).max[i]
                && protected(&a).max[i] >= protected(&b).min[i]),
            "fixture must reproduce the coarse-AABB overlap"
        );
        assert!(!passages_exclusive(&a, &b, buffer));
        let crossing = synthetic_link(3, [-3., 4., 2.], [3., 1.5, 2.]);
        assert!(passages_exclusive(&a, &crossing, buffer));
        // A flight passing over another passage's launch pad excludes it.
        let over_pad = synthetic_link(4, [1., 4.2, -3.], [1., 1.5, 3.]);
        assert!(passages_exclusive(&a, &over_pad, buffer));
        // Same horizontal path on a separate floor is independent.
        let lower = synthetic_link(5, [0., 0., 0.], [0., -2.5, 5.]);
        assert!(!passages_exclusive(&a, &lower, buffer));
        assert_eq!(
            exclusive_pairs(&[a, b, crossing, over_pad, lower], buffer),
            std::collections::BTreeSet::from([(1, 3), (1, 4), (2, 3), (3, 4)])
        );
    }
    #[test]
    fn traffic_geometry_is_rotation_translation_and_floor_independent() {
        for angle in [0., std::f64::consts::FRAC_PI_3] {
            for shift in [[0.; 3], [123., 37., -250.]] {
                let transform = |p: [f64; 3]| {
                    [
                        shift[0] + p[0] * angle.cos() - p[2] * angle.sin(),
                        shift[1] + p[1],
                        shift[2] + p[0] * angle.sin() + p[2] * angle.cos(),
                    ]
                };
                let a = [[transform([-4., 0., 0.]), transform([4., 0., 0.])]];
                let b = [[transform([0., 0., -4.]), transform([0., 0., 4.])]];
                let upper = [[transform([0., 4., -4.]), transform([0., 4., 4.])]];
                let corner = [[transform([0.5, 0., 0.5]); 2]];
                let point_lane = [[transform([0.; 3]); 2]];
                assert!(
                    lanes_conflict(&point_lane, &corner, &mut 256),
                    "diagonal box corners overlap"
                );
                assert!(lanes_conflict(&a, &b, &mut 256));
                assert!(!lanes_conflict(&a, &upper, &mut 256));
                assert!(in_lane(transform([2., 0., 0.5]), &a, 1.5));
                assert!(!in_lane(transform([2., 4., 0.5]), &a, 1.5));
                let mut actors = ActorGrid::default();
                actors
                    .insert(7, Bounds::body(transform([0.; 3]), Body::default()))
                    .unwrap();
                assert_eq!(approach_blocker(&actors, 9, &a, &mut 256).unwrap(), Some(7));
                assert_eq!(
                    approach_blocker(&actors, 9, &upper, &mut 256).unwrap(),
                    None
                );
                assert_eq!(approach_blocker(&actors, 7, &a, &mut 256).unwrap(), None);
                assert!(approach_blocker(&actors, 9, &a, &mut 0).is_err());
            }
        }
    }
}
