//! Local passage access geometry, independent of maps and actor roles.
use crate::{actor_collision::ActorGrid, bot_physics::Body, navmesh, replication};
use std::time::Duration;

pub(super) struct Progress {
    pub generation: u32,
    pub objective: Option<u16>,
    pub role_slot: u8,
    pub position: replication::CharacterPosition,
    pub at: Duration,
}
pub(super) struct HoldingAccess {
    pub requester: u16,
    pub generation: u32,
    pub source_since: Duration,
    pub lane: Vec<[[f64; 3]; 2]>,
    pub until: Duration,
}
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actor_collision::Bounds;
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
