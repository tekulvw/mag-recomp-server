//! SYNTHETIC regressions for candidate spots a body would start inside: open
//! ground with walls, blocks, a step and a placed structure, all authored in
//! code. No game catalogs, meshes or captures.
use super::*;
use crate::bot_physics::{Body, BodyShape, PhysicsError, WorldTerrain};
use crate::collision::CollisionWorld;

const MAP: &str = "synthetic_obstructed";
const OBSTRUCTED: &str = "bot_physics_start_obstructed";
const INVALID: &str = "bot_physics_invalid_query";

/// Axis-aligned solid boxes (min, max) as closed triangle soups.
fn solids(parts: &[([f64; 3], [f64; 3])]) -> (Vec<[f64; 3]>, Vec<[u32; 3]>) {
    let mut vertices = Vec::new();
    let mut triangles = Vec::new();
    for (lo, hi) in parts {
        let n = vertices.len() as u32;
        for k in 0..8 {
            vertices.push([
                if k & 1 == 0 { lo[0] } else { hi[0] },
                if k & 2 == 0 { lo[1] } else { hi[1] },
                if k & 4 == 0 { lo[2] } else { hi[2] },
            ]);
        }
        for f in [
            [0, 1, 3, 2],
            [4, 6, 7, 5],
            [0, 4, 5, 1],
            [2, 3, 7, 6],
            [0, 2, 6, 4],
            [1, 5, 7, 3],
        ] {
            triangles.push([n + f[0], n + f[1], n + f[2]]);
            triangles.push([n + f[0], n + f[2], n + f[3]]);
        }
    }
    (vertices, triangles)
}

fn mesh(id: &str, parts: &[([f64; 3], [f64; 3])], region: bool) -> CollisionWorld {
    let (vertices, triangles) = solids(parts);
    let regions = if region {
        json!([{"min":[0.,-3.,0.],"max":[20.,8.,20.],"ground_verified":true,
            "obstacles_verified":true,"provenance":["synthetic"]}])
    } else {
        json!([])
    };
    let raw = serde_json::to_vec(&json!({
        "format":"mag_collision_world","schema_version":1,"coordinate_space":"world",
        "metadata":{"map_name":MAP,"build":"synthetic","coverage":"provisional",
            "provenance":["synthetic obstructed spots"],"unknowns":["Not a game map"],
            "physics_regions":regions},
        "instances":[{"id":id,"vertices_xyz":vertices,"triangles":triangles}]
    }))
    .unwrap();
    CollisionWorld::from_bytes(&raw, Default::default()).unwrap()
}

/// A 20 m square floor slab (top at y = 0) and `parts` standing on it.
fn world(parts: &[([f64; 3], [f64; 3])]) -> CollisionWorld {
    let mut all = vec![([0., -0.5, 0.], [20., 0., 20.])];
    all.extend_from_slice(parts);
    mesh("arena", &all, true)
}

fn at(p: [f64; 3]) -> replication::CharacterPosition {
    position(p)
}

/// The search used by role spots and recovery steps: a metre up, eight down.
fn beside(
    world: &CollisionWorld,
    raw: [f64; 3],
    body: Body,
) -> Result<replication::CharacterPosition, &'static str> {
    supported_beside(Some(world), MAP, at(raw), 1., 8., body, |_| true)
}

fn fits(world: &CollisionWorld, feet: replication::CharacterPosition, body: Body) -> bool {
    let terrain = WorldTerrain::new(Some(world), MAP).unwrap();
    terrain
        .start_motion([f64::from(feet.x), f64::from(feet.y) + 0.5, f64::from(feet.z)], body)
        .is_ok()
}

fn flat(a: replication::CharacterPosition, b: [f64; 3]) -> f64 {
    (f64::from(a.x) - b[0]).hypot(f64::from(a.z) - b[2])
}

/// A wall 0.4 m thick and 3 m high along x = 10.
fn wall() -> CollisionWorld {
    world(&[([10., 0., 0.], [10.4, 3., 20.])])
}

#[test]
fn a_spot_against_a_wall_is_obstructed_not_an_invalid_query() {
    let world = wall();
    let raw = [9.9, 0.5, 10.];
    // The exact column: the body's half-width reaches into the wall.
    assert_eq!(
        supported_interval(Some(&world), MAP, at(raw), 1., 8., Body::default()),
        Err(OBSTRUCTED)
    );
    let terrain = WorldTerrain::new(Some(&world), MAP).unwrap();
    assert_eq!(
        terrain.start_motion(raw, Body::default()).unwrap_err(),
        PhysicsError::Obstructed
    );
    assert_eq!(PhysicsError::Obstructed.code(), OBSTRUCTED);
}

#[test]
fn a_spot_against_a_wall_moves_just_clear_of_it_on_its_floor() {
    let world = wall();
    let raw = [9.9, 0.5, 10.];
    let body = Body::default();
    let feet = beside(&world, raw, body).unwrap();
    assert!(f64::from(feet.y).abs() < 0.05, "{feet:?}");
    assert!(f64::from(feet.x) <= 10. - body.radius + 1e-6, "{feet:?}");
    assert!(flat(feet, raw) <= 0.5 + 1e-6, "{feet:?}");
    assert!(fits(&world, feet, body));
}

#[test]
fn a_spot_that_already_fits_is_unchanged() {
    let world = wall();
    let raw = [5., 0.5, 10.];
    let exact = supported_interval(Some(&world), MAP, at(raw), 1., 8., Body::default()).unwrap();
    assert_eq!(beside(&world, raw, Body::default()), Ok(exact));
    assert!(flat(exact, raw) < 1e-6);
}

/// The floor with one placed structure: a solid box `half` metres either side
/// of (10, 10) in x and z and `height` tall, as the mode places an
/// objective's console, crate or shell.
fn placed(half: f64, height: f64) -> CollisionWorld {
    let shape = mesh("structure", &[([-half, 0., -half], [half, height, half])], false);
    let structures = std::sync::Arc::new(
        crate::bot_structure_collision::StructureObstacles::from_surfaces(
            (1, 1),
            vec![(
                "objective-0-1-0".into(),
                crate::dynamic_collision::Pose::at([10., 0., 10.]),
                shape,
            )],
        ),
    );
    let mut world = world(&[]);
    world.set_bot_structures(Some(structures));
    world
}

/// A 2 m square console 1.2 m tall in the middle of the floor: lower than
/// a body, so a body over its footprint stands through its top.
fn machine() -> CollisionWorld {
    placed(1., 1.2)
}

#[test]
fn a_spot_inside_a_prop_moves_out_beside_it() {
    let world = machine();
    // The trigger-centre case: the column is inside the console. A search
    // from below its top finds the floor under it, where no body fits.
    let raw = [10., 1., 10.];
    let body = Body::default();
    assert_eq!(
        supported_interval(Some(&world), MAP, at(raw), 0., 4., body),
        Err(OBSTRUCTED)
    );
    let feet = supported_beside(Some(&world), MAP, at(raw), 0., 4., body, |_| true).unwrap();
    assert!(f64::from(feet.y).abs() < 0.05, "{feet:?}");
    assert!(flat(feet, raw) <= 1.5 + 1e-6, "{feet:?}");
    assert!(fits(&world, feet, body));
}

#[test]
fn an_operator_approach_moves_beside_its_prop_but_stays_in_range() {
    let world = machine();
    // A trigger hugging the machine: 1.2 m either side of its centre in x,
    // 2.5 m in z, from the floor to 1 m (below the console's top).
    let volumes = [crate::objective_range::TriggerBox {
        centre: [10., 0.5, 10.],
        axes: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
        half_extents: [1.2, 0.5, 2.5],
    }];
    let range = crate::objective_range::ObjectiveRange {
        position: [10., 0.5, 10.],
        maximum_distance_m: 3.,
        volumes: &volumes,
    };
    let raw = at(range.approach(0));
    assert_eq!(point(raw), [10., 1., 10.]);
    let body = Body::default();
    // Unfiltered, the first free spot is east of the machine, outside the
    // trigger; the operator's spot is the first one inside it.
    let east = supported_beside(Some(&world), MAP, raw, 0., 4., body, |_| true).unwrap();
    assert!(!range.contains_planned(point(east)), "{east:?}");
    let feet = supported_operator(Some(&world), MAP, raw, range, body).unwrap();
    assert!(range.contains_planned(point(feet)), "{feet:?}");
    assert!(f64::from(feet.y).abs() < 0.05, "{feet:?}");
    assert!(fits(&world, feet, body));
}

#[test]
fn a_spot_with_no_room_within_two_metres_stays_obstructed() {
    // A 6 m block: every ring column is inside it.
    let world = placed(3., 1.2);
    assert_eq!(
        supported_beside(Some(&world), MAP, at([10., 1., 10.]), 0., 4., Body::default(), |_| true),
        Err(OBSTRUCTED)
    );
}

#[test]
fn a_spot_beside_a_raised_block_never_moves_onto_its_top() {
    // A 1.5 m block east of the spot: the ring's first columns land on its
    // top, a floor no step reaches. The spot moves west on its own floor.
    let world = world(&[([10., 0., 0.], [14., 1.5, 20.])]);
    let raw = [9.9, 0.5, 10.];
    let body = Body::default();
    let feet = supported_beside(Some(&world), MAP, at(raw), 8., 8., body, |_| true).unwrap();
    assert!(f64::from(feet.y).abs() < 0.05, "{feet:?}");
    assert!(f64::from(feet.x) < 10., "{feet:?}");
    // Without the same-floor rule the first ring column would be the top.
    let top = supported_interval(Some(&world), MAP, at([10.15, 0.5, 10.]), 8., 8., body).unwrap();
    assert!((f64::from(top.y) - 1.5).abs() < 0.05, "{top:?}");
}

#[test]
fn a_placed_structure_obstructs_a_spot_and_the_spot_moves_beside_it() {
    let base = world(&[]);
    let world = placed(1., 2.);
    let raw = [10., 0.5, 10.];
    let body = Body::default();
    assert!(supported_interval(Some(&base), MAP, at(raw), 1., 8., body).is_ok());
    assert_eq!(
        supported_interval(Some(&world), MAP, at([8.9, 0.5, 10.]), 1., 8., body),
        Err(OBSTRUCTED)
    );
    let feet = beside(&world, [8.9, 0.5, 10.], body).unwrap();
    assert!(f64::from(feet.y).abs() < 0.05, "{feet:?}");
    assert!(f64::from(feet.x) <= 9. - body.radius + 1e-6, "{feet:?}");
    assert!(fits(&world, feet, body));
}

#[test]
fn a_capsule_body_against_a_wall_moves_clear_of_it_too() {
    let world = wall();
    let body = Body {
        max_slope_degrees: 54.,
        ..Default::default()
    }
    .with_shape(BodyShape::Capsule {
        radius: 0.4,
        half_height: 0.5,
        centre_height: 0.9,
    });
    let raw = [9.8, 0.5, 10.];
    assert_eq!(
        supported_interval(Some(&world), MAP, at(raw), 1., 8., body),
        Err(OBSTRUCTED)
    );
    let feet = beside(&world, raw, body).unwrap();
    assert!(f64::from(feet.x) <= 10. - body.radius + 1e-6, "{feet:?}");
    assert!(fits(&world, feet, body));
}

#[test]
fn malformed_and_unsupported_queries_are_refused_without_a_search() {
    let world = wall();
    let body = Body::default();
    // Non-finite coordinates and an empty search window are malformed.
    for (raw, up, down) in [
        ([f64::NAN, 0.5, 10.], 1., 8.),
        ([9.9, f64::INFINITY, 10.], 1., 8.),
        ([9.9, 0.5, 10.], 0., 0.),
        ([9.9, 0.5, 10.], -1., 8.),
    ] {
        assert_eq!(
            supported_beside(Some(&world), MAP, at(raw), up, down, body, |_| true),
            Err(INVALID),
            "{raw:?} {up} {down}"
        );
    }
    // A body outside the source's own limits is malformed too.
    assert_eq!(
        beside(&world, [9.9, 0.5, 10.], Body { radius: 3., ..body }),
        Err(INVALID)
    );
    // No collision world, a map mismatch, no ground, and ground outside
    // verified coverage are not obstructions and are not searched around.
    assert_eq!(
        supported_beside(None, MAP, at([5., 0.5, 10.]), 1., 8., body, |_| true),
        Err(PhysicsError::Unavailable.code())
    );
    assert_eq!(
        supported_beside(Some(&world), "another_map", at([5., 0.5, 10.]), 1., 8., body, |_| true),
        Err(PhysicsError::MapMismatch.code())
    );
    assert_eq!(beside(&world, [5., 30., 10.], body), Err("bot_objective_unsupported"));
    let uncovered = mesh("arena", &[([0., -0.5, 0.], [20., 0., 20.])], false);
    assert_eq!(
        beside(&uncovered, [5., 0.5, 10.], body),
        Err(PhysicsError::CoverageUnknown.code())
    );
}
