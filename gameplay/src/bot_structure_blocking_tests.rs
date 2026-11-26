//! SYNTHETIC structure blocking: an invented wall across a route.
use super::*;

fn wall_at_x10() -> Blocker {
    Blocker {
        objective: 5,
        bounds: crate::actor_collision::Bounds {
            min: [9.9, 0., -3.],
            max: [10.1, 4., 3.],
        },
    }
}

/// The wall is solid between z -3 and 3, 4 m tall, at x = 10.
fn hit(_: u16, c: [f64; 3], h: [f64; 3]) -> Result<bool, &'static str> {
    Ok((c[0] - h[0]..=c[0] + h[0]).contains(&10.)
        && c[2] + h[2] >= -3.
        && c[2] - h[2] <= 3.
        && c[1] - h[1] <= 4.)
}

#[test]
fn a_route_through_a_standing_wall_is_blocked_and_one_around_it_is_not() {
    let blockers = [wall_at_x10()];
    let through = [[0., 0., 0.], [20., 0., 0.]];
    assert_eq!(route_blocked(&through, &blockers, hit), Ok(Some(5)));
    let around = [[0., 0., 0.], [0., 0., 6.], [20., 0., 6.], [20., 0., 0.]];
    assert_eq!(route_blocked(&around, &blockers, hit), Ok(None));
    // Without the structure (destroyed), the same route is open.
    assert_eq!(route_blocked(&through, &[], hit), Ok(None));
}

#[test]
fn a_route_starting_inside_the_geometry_may_leave_it() {
    let blockers = [wall_at_x10()];
    assert_eq!(
        route_blocked(&[[10., 0., 0.], [0., 0., 0.]], &blockers, hit),
        Ok(None)
    );
    // Leaving and walking back in is blocked.
    assert_eq!(
        route_blocked(
            &[[10., 0., 0.], [0., 0., 0.], [20., 0., 0.]],
            &blockers,
            hit
        ),
        Ok(Some(5))
    );
}

#[test]
fn geometry_below_the_stepping_body_does_not_block() {
    // A bridge deck or rubble below the body's lowest point.
    let low = |_: u16, c: [f64; 3], h: [f64; 3]| Ok(c[1] - h[1] <= 0.2);
    let blockers = [wall_at_x10()];
    assert_eq!(
        route_blocked(&[[0., 0., 0.], [20., 0., 0.]], &blockers, low),
        Ok(None)
    );
}
