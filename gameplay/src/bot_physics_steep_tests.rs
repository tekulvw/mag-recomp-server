//! SYNTHETIC locomotion regressions for the upright box on steep and
//! overhung ground: worlds are authored in code. No game geometry.
use super::*;

/// One triangle soup instance per entry; each quad is `[a, b, c, d]` in order.
fn world(quads: &[[Point; 4]], min: Point, max: Point) -> CollisionWorld {
    let mut vertices = Vec::new();
    let mut triangles = Vec::new();
    for q in quads {
        let n = vertices.len() as u32;
        vertices.extend(q.iter().copied());
        triangles.extend([[n, n + 1, n + 2], [n, n + 2, n + 3]]);
    }
    let raw = serde_json::to_vec(&serde_json::json!({
        "format":"mag_collision_world","schema_version":1,"coordinate_space":"world",
        "metadata":{"map_name":"synthetic_steep","build":"synthetic","coverage":"provisional",
            "provenance":["synthetic steep locomotion"],"unknowns":["Not a game map"],"physics_regions":[
                {"min":min,"max":max,"ground_verified":true,"obstacles_verified":true,
                 "provenance":["synthetic"]}]},
        "instances":[{"id":"steep","vertices_xyz":vertices,"triangles":triangles}]
    }))
    .unwrap();
    CollisionWorld::from_bytes(&raw, Default::default()).unwrap()
}

/// A horizontal quad facing up (counter-clockwise seen from above).
fn floor(x0: f64, x1: f64, y: f64, z0: f64, z1: f64) -> [Point; 4] {
    [[x0, y, z0], [x0, y, z1], [x1, y, z1], [x1, y, z0]]
}

fn body() -> Body {
    Body {
        max_slope_degrees: 54.0,
        ..Default::default()
    }
}

/// Walk at `speed` toward `goal` in 0.25 s slices (the navigation cadence)
/// for at most `seconds`; true once within 0.3 m of the goal horizontally.
fn walk(world: &CollisionWorld, start: Point, goal: Point, speed: f64, seconds: f64) -> bool {
    let terrain = WorldTerrain::new(Some(world), "synthetic_steep").unwrap();
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

fn ramp(degrees: f64) -> (CollisionWorld, Point, Point) {
    let rise = 4.0;
    let x1 = 6.0;
    let x2 = x1 + rise / degrees.to_radians().tan();
    let world = world(
        &[
            floor(0.0, x1, 0.0, 0.0, 10.0),
            [
                [x1, 0.0, 0.0],
                [x1, 0.0, 10.0],
                [x2, rise, 10.0],
                [x2, rise, 0.0],
            ],
            floor(x2, x2 + 6.0, rise, 0.0, 10.0),
        ],
        [-1.0, -3.0, -1.0],
        [x2 + 7.0, rise + 5.0, 11.0],
    );
    (world, [3.0, 0.05, 5.0], [x2 + 3.0, rise, 5.0])
}

#[test]
fn a_plain_ramp_at_the_slope_limit_is_climbed_and_descended_walking_and_sprinting() {
    let (world, low, high) = ramp(53.5);
    for speed in [4.5, 6.0] {
        assert!(walk(&world, low, high, speed, 10.0), "up at {speed}");
        let start = [high[0], high[1] + 0.05, high[2]];
        assert!(walk(&world, start, low, speed, 10.0), "down at {speed}");
    }
}

#[test]
fn a_ramp_steeper_than_the_limit_is_still_not_climbed() {
    let (world, low, high) = ramp(60.0);
    assert!(!walk(&world, low, high, 4.5, 10.0));
}

/// A 0.25 m step up onto a platform with a steep rock bank running along it
/// beside the way, its faces steeper than the slope limit. A body walking
/// along the platform overlaps the bank's foot with one side of its footprint
/// while the platform under its centre is flat, so a step's downward sweep
/// first meets the steep face.
fn lip_world() -> CollisionWorld {
    let (edge, top, lip_x, crest_h) = (5.0, 0.25, 5.5, 1.0);
    let (z0, crest, z1) = (5.15, 5.6, 6.05);
    world(
        &[
            floor(0.0, edge, 0.0, 0.0, 10.0),
            // The step's riser, facing -x.
            [
                [edge, 0.0, 0.0],
                [edge, 0.0, 10.0],
                [edge, top, 10.0],
                [edge, top, 0.0],
            ],
            floor(edge, 12.0, top, 0.0, 10.0),
            // The lip's near end rises as steeply as its sides.
            [
                [lip_x, top, z0],
                [lip_x, top, z1],
                [lip_x + 0.45, crest_h, crest + 0.01],
                [lip_x + 0.45, crest_h, crest - 0.01],
            ],
            [
                [lip_x, top, z0],
                [lip_x + 0.45, crest_h, crest - 0.01],
                [12.0, crest_h, crest],
                [12.0, top, z0],
            ],
            [
                [lip_x + 0.45, crest_h, crest + 0.01],
                [lip_x, top, z1],
                [12.0, top, z1],
                [12.0, crest_h, crest],
            ],
        ],
        [-1.0, -3.0, -1.0],
        [13.0, 5.0, 11.0],
    )
}

#[test]
fn a_steep_bank_beside_walkable_ground_is_walked_past() {
    let world = lip_world();
    for speed in [4.5, 6.0] {
        assert!(
            walk(&world, [3.0, 0.05, 5.0], [10.0, 0.25, 5.0], speed, 6.0),
            "at {speed}"
        );
    }
}

/// A beam across the way whose underside is `clearance` above the floor.
fn lintel_world(clearance: f64) -> CollisionWorld {
    let (x0, x1) = (5.5, 6.5);
    world(
        &[
            floor(0.0, 12.0, 0.0, 0.0, 10.0),
            // Underside, facing down.
            [
                [x0, clearance, 0.0],
                [x1, clearance, 0.0],
                [x1, clearance, 10.0],
                [x0, clearance, 10.0],
            ],
            floor(x0, x1, clearance + 0.5, 0.0, 10.0),
            [
                [x0, clearance, 0.0],
                [x0, clearance, 10.0],
                [x0, clearance + 0.5, 10.0],
                [x0, clearance + 0.5, 0.0],
            ],
            [
                [x1, clearance, 0.0],
                [x1, clearance + 0.5, 0.0],
                [x1, clearance + 0.5, 10.0],
                [x1, clearance, 10.0],
            ],
        ],
        [-1.0, -3.0, -1.0],
        [13.0, 5.0, 11.0],
    )
}

#[test]
fn a_lintel_the_body_fits_under_does_not_stop_it_mid_step() {
    // The body is 1.8 m tall: it fits under 1.9 m, but not once lifted a step.
    let world = lintel_world(1.9);
    for speed in [4.5, 6.0] {
        assert!(
            walk(&world, [3.0, 0.05, 5.0], [9.0, 0.0, 5.0], speed, 6.0),
            "at {speed}"
        );
    }
}

#[test]
fn a_lintel_lower_than_the_body_still_blocks() {
    let world = lintel_world(1.7);
    assert!(!walk(&world, [3.0, 0.05, 5.0], [9.0, 0.0, 5.0], 4.5, 6.0));
}

/// A 14 degree rise along +z under a leaning overhang whose underside faces
/// down and toward +x and climbs more slowly than the floor.
fn overhang_world() -> CollisionWorld {
    let (floor_grade, roof_grade) = (0.25, 0.2);
    world(
        &[
            [
                [0.0, 0.0, 0.0],
                [0.0, 10.0 * floor_grade, 10.0],
                [10.0, 10.0 * floor_grade, 10.0],
                [10.0, 0.0, 0.0],
            ],
            // Underside: 1.1 m high at x = 3, rising 0.75 m per metre toward +x.
            [
                [3.0, 1.1, 0.0],
                [3.0, 1.1 + 10.0 * roof_grade, 10.0],
                [5.0, 2.6 + 10.0 * roof_grade, 10.0],
                [5.0, 2.6, 0.0],
            ],
        ],
        [-1.0, -3.0, -1.0],
        [11.0, 8.0, 11.0],
    )
}

#[test]
fn a_body_pressed_under_a_leaning_overhang_slides_out_along_it() {
    let world = overhang_world();
    let terrain = WorldTerrain::new(Some(&world), "synthetic_steep").unwrap();
    let b = body();
    // Pressed against the underside at the foot of the rise: the closest
    // start toward -x that does not overlap it.
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
    // Walk straight up the rise, the way a route along the overhang leads.
    for _ in 0..6 {
        m = terrain.advance(m, [0.0, 0.0, 4.5], 0.25, b).unwrap();
    }
    assert!(m.position[2] > start[2] + 3.0, "{m:?} from {start:?}");
    assert!(m.grounded, "{m:?}");
}
