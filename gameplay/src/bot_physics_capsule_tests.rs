//! SYNTHETIC locomotion regressions for the capsule body: worlds are authored
//! in code (shared with the box's steep-ground tests). No game geometry, and
//! the capsule's dimensions are this test's own.
use super::steep_tests::{floor, lintel_world, lip_world, overhang_world, ramp, world};
use super::*;
use crate::actor_collision::{ActorGrid, Query};

/// A capsule 1.8 m tall with its rounded bottom at the feet.
fn shape() -> BodyShape {
    BodyShape::Capsule {
        radius: 0.4,
        half_height: 0.5,
        centre_height: 0.9,
    }
}

fn body() -> Body {
    Body {
        max_slope_degrees: 54.0,
        ..Default::default()
    }
    .with_shape(shape())
}

/// Walk at `speed` toward `goal` in 0.25 s slices (the navigation cadence)
/// for at most `seconds`; true once within 0.3 m of the goal horizontally.
fn walk_in(terrain: &WorldTerrain, start: Point, goal: Point, speed: f64, seconds: f64) -> bool {
    let b = body();
    let mut m = terrain.start_motion(start, b).unwrap();
    m = terrain.advance(m, [0.0; 3], 0.5, b).unwrap();
    assert!(m.grounded, "start must settle: {m:?}");
    let mut t = 0.0;
    while t < seconds {
        let dx = goal[0] - m.position[0];
        let dz = goal[2] - m.position[2];
        let d = dx.hypot(dz);
        if d < 0.3 {
            return true;
        }
        let step = (d / speed).min(0.25);
        m = terrain
            .advance(m, [dx / d * speed, 0.0, dz / d * speed], step, b)
            .unwrap();
        t += step;
    }
    false
}

fn walk(world: &CollisionWorld, start: Point, goal: Point, speed: f64, seconds: f64) -> bool {
    let terrain = WorldTerrain::new(Some(world), "synthetic_steep").unwrap();
    walk_in(&terrain, start, goal, speed, seconds)
}

#[test]
fn a_capsule_body_takes_its_radius_and_top_from_the_shape_and_must_match_them() {
    let b = body();
    assert_eq!(b.radius, 0.4);
    assert!((b.height - 1.8).abs() < 1e-12);
    assert!(b.validate().is_ok());
    assert_eq!(
        Body { radius: 0.35, ..b }.validate(),
        Err(PhysicsError::InvalidQuery)
    );
    assert_eq!(
        Body {
            shape: BodyShape::Capsule {
                radius: 0.4,
                half_height: 0.5,
                centre_height: 3.0,
            },
            ..b
        }
        .validate(),
        Err(PhysicsError::InvalidQuery)
    );
}

#[test]
fn a_capsule_climbs_and_descends_a_plain_ramp_at_the_slope_limit() {
    let (world, low, high) = ramp(53.5);
    for speed in [4.5, 6.0] {
        assert!(walk(&world, low, high, speed, 10.0), "up at {speed}");
        let start = [high[0], high[1] + 0.05, high[2]];
        assert!(walk(&world, start, low, speed, 10.0), "down at {speed}");
    }
}

#[test]
fn a_capsule_does_not_climb_a_ramp_steeper_than_the_limit() {
    let (world, low, high) = ramp(60.0);
    for speed in [4.5, 6.0] {
        assert!(!walk(&world, low, high, speed, 10.0), "at {speed}");
    }
}

/// A floor with a raised platform of `rise` across the way.
fn step_world(rise: f64) -> CollisionWorld {
    world(
        &[
            floor(0.0, 5.0, 0.0, 0.0, 10.0),
            [
                [5.0, 0.0, 0.0],
                [5.0, 0.0, 10.0],
                [5.0, rise, 10.0],
                [5.0, rise, 0.0],
            ],
            floor(5.0, 12.0, rise, 0.0, 10.0),
        ],
        [-1.0, -3.0, -1.0],
        [13.0, 5.0, 11.0],
    )
}

#[test]
fn a_capsule_steps_over_a_lip_below_step_height_but_not_a_higher_one() {
    for speed in [4.5, 6.0] {
        assert!(
            walk(
                &step_world(0.3),
                [3.0, 0.05, 5.0],
                [9.0, 0.3, 5.0],
                speed,
                6.0
            ),
            "0.3 m at {speed}"
        );
        assert!(
            !walk(
                &step_world(0.6),
                [3.0, 0.05, 5.0],
                [9.0, 0.6, 5.0],
                speed,
                6.0
            ),
            "0.6 m at {speed}"
        );
    }
}

#[test]
fn a_capsule_passes_under_a_lintel_it_fits_under_and_not_one_lower() {
    for speed in [4.5, 6.0] {
        assert!(
            walk(
                &lintel_world(1.9),
                [3.0, 0.05, 5.0],
                [9.0, 0.0, 5.0],
                speed,
                6.0
            ),
            "at {speed}"
        );
    }
    assert!(!walk(
        &lintel_world(1.7),
        [3.0, 0.05, 5.0],
        [9.0, 0.0, 5.0],
        4.5,
        6.0
    ));
}

#[test]
fn a_capsule_walks_past_a_steep_bank_beside_walkable_ground() {
    let world = lip_world();
    for speed in [4.5, 6.0] {
        assert!(
            walk(&world, [3.0, 0.05, 5.0], [10.0, 0.25, 5.0], speed, 6.0),
            "at {speed}"
        );
    }
}

#[test]
fn a_capsule_pressed_under_a_leaning_overhang_slides_out_along_it() {
    let world = overhang_world();
    let terrain = WorldTerrain::new(Some(&world), "synthetic_steep").unwrap();
    let b = body();
    let mut start = [4.6, 0.25, 0.5];
    while terrain
        .start_motion([start[0] - 0.005, start[1], start[2]], b)
        .is_ok()
    {
        start[0] -= 0.005;
    }
    let mut m = terrain.start_motion(start, b).unwrap();
    m = terrain.advance(m, [0.0; 3], 0.5, b).unwrap();
    assert!(m.grounded);
    for _ in 0..6 {
        m = terrain.advance(m, [0.0, 0.0, 4.5], 0.25, b).unwrap();
    }
    assert!(m.position[2] > start[2] + 3.0, "{m:?} from {start:?}");
    assert!(m.grounded, "{m:?}");
}

#[test]
fn a_capsule_stands_on_an_edge_only_while_its_contact_normal_is_walkable() {
    // A 1 m ledge. With its axis just past the edge the body rests on the
    // edge, whose contact normal (edge toward axis) is within the slope
    // limit; further out that normal is steep and the body rolls off.
    let world = world(
        &[
            floor(0.0, 5.0, 1.0, 0.0, 10.0),
            [
                [5.0, 1.0, 0.0],
                [5.0, 1.0, 10.0],
                [5.0, 0.0, 10.0],
                [5.0, 0.0, 0.0],
            ],
            floor(5.0, 12.0, 0.0, 0.0, 10.0),
        ],
        [-1.0, -3.0, -1.0],
        [13.0, 5.0, 11.0],
    );
    let terrain = WorldTerrain::new(Some(&world), "synthetic_steep").unwrap();
    let b = body();
    let settle = |x: f64| {
        let m = terrain.start_motion([x, 1.02, 5.0], b).unwrap();
        terrain.advance(m, [0.0; 3], 1.0, b).unwrap()
    };
    let inside = settle(4.9);
    assert!(inside.grounded && (inside.position[1] - 1.0).abs() < 0.05);
    let edge = settle(5.2);
    assert!(edge.grounded && edge.position[1] > 0.9, "{edge:?}");
    let over = settle(5.35);
    assert!(over.grounded, "{over:?}");
    assert!(over.position[1] < 0.05, "rolled off and landed: {over:?}");
}

#[test]
fn local_traversal_uses_the_capsule() {
    let terrain_world = lintel_world(1.9);
    let terrain = WorldTerrain::new(Some(&terrain_world), "synthetic_steep").unwrap();
    // The capsule fits under 1.9 m without lifting a full step into it.
    assert!(
        terrain
            .can_traverse([3.0, 0.0, 5.0], [8.0, 0.0, 5.0], body())
            .unwrap()
    );
    let low_world = lintel_world(1.7);
    let low = WorldTerrain::new(Some(&low_world), "synthetic_steep").unwrap();
    assert!(
        !low.can_traverse([3.0, 0.0, 5.0], [8.0, 0.0, 5.0], body())
            .unwrap()
    );
    let steps = step_world(0.3);
    let terrain = WorldTerrain::new(Some(&steps), "synthetic_steep").unwrap();
    assert!(
        terrain
            .can_traverse([3.0, 0.0, 5.0], [7.0, 0.3, 5.0], body())
            .unwrap()
    );
    // Local traversal admits rises up to a step plus the ground snap, as it
    // does for the box; a higher one is a wall.
    let wall = step_world(0.8);
    let terrain = WorldTerrain::new(Some(&wall), "synthetic_steep").unwrap();
    assert!(
        !terrain
            .can_traverse([3.0, 0.0, 5.0], [7.0, 0.8, 5.0], body())
            .unwrap()
    );
}

#[test]
fn a_capsule_bot_slides_past_another_capsule_and_is_blocked_head_on() {
    let world = world(
        &[floor(0.0, 12.0, 0.0, 0.0, 10.0)],
        [-1.0, -3.0, -1.0],
        [13.0, 5.0, 11.0],
    );
    let b = body();
    let mut grid = ActorGrid::default();
    // Another body standing in the way, its axis 0.3 m to one side.
    grid.insert_reserved_body(2, [6.0, 0.0, 5.3], b).unwrap();
    let terrain = WorldTerrain::new(Some(&world), "synthetic_steep")
        .unwrap()
        .with_actors(Query {
            grid: &grid,
            actor: 1,
        });
    assert!(walk_in(
        &terrain,
        [3.0, 0.0, 5.0],
        [9.0, 0.0, 5.0],
        4.5,
        6.0
    ));
    // Head on, the bot stops where the two capsules touch.
    let mut grid = ActorGrid::default();
    grid.insert_reserved_body(2, [6.0, 0.0, 5.0], b).unwrap();
    let terrain = WorldTerrain::new(Some(&world), "synthetic_steep")
        .unwrap()
        .with_actors(Query {
            grid: &grid,
            actor: 1,
        });
    let mut m = terrain.start_motion([3.0, 0.0, 5.0], b).unwrap();
    for _ in 0..8 {
        m = terrain.advance(m, [4.5, 0.0, 0.0], 0.25, b).unwrap();
    }
    assert!(m.blocked && terrain.actor_contact() == Some(2), "{m:?}");
    assert!((m.position[0] - 5.2).abs() < 0.02, "{m:?}");
}
