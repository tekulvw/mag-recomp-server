// SYNTHETIC: every world here is authored in code; no game catalogs or captures.
use super::*;
use crate::bot_physics::{Body, WorldTerrain};
use serde_json::json;

const STATIC_WORLD: u32 = 9;
const VEHICLE_ONLY: u32 = 11;
/// What the character controller's retail filter row collides with among the
/// static layers: the static world, not vehicle-only blockers.
const CHARACTER_ROW: u32 = 1 << STATIC_WORLD;

fn collision(layer: u32) -> serde_json::Value {
    json!({"filter_info":layer,"ray_child_filter_info":0,"convex_radius":0.,
        "provenance":"synthetic","child_filter_policy":"uniform"})
}

/// A floor, then a wall across it at x = 10 on `wall_layer` (None: no filter
/// metadata at all).
fn corridor(wall_layer: Option<u32>) -> CollisionWorld {
    let mut wall = json!({"id":"wall","vertices_xyz":[[10.,0.,0.],[10.,0.,10.],[10.,4.,10.],[10.,4.,0.]],
        "triangles":[[0,1,2],[0,2,3]]});
    if let Some(layer) = wall_layer {
        wall["collision"] = collision(layer);
    }
    CollisionWorld::from_bytes(
        json!({
            "format":"mag_collision_world","schema_version":1,"coordinate_space":"world",
            "metadata":{"map_name":"synthetic","build":"synthetic","coverage":"provisional",
                "provenance":["synthetic"],"unknowns":["Not a game map"],"physics_regions":[
                    {"min":[-1.,-2.,-1.],"max":[21.,6.,11.],"ground_verified":true,
                     "obstacles_verified":true,"provenance":["synthetic corridor"]}]},
            "instances":[
                {"id":"floor","vertices_xyz":[[0.,0.,0.],[20.,0.,0.],[20.,0.,10.],[0.,0.,10.]],
                 "triangles":[[0,2,1],[0,3,2]],"collision":collision(STATIC_WORLD)},
                wall]
        })
        .to_string()
        .as_bytes(),
        CollisionLimits::default(),
    )
    .unwrap()
}

/// Walk a bot body from x = 7 toward x = 13 for two seconds; where it ends.
fn walk(world: &CollisionWorld) -> f64 {
    let terrain = WorldTerrain::new(Some(world), "synthetic").unwrap();
    let body = Body::default();
    let mut motion = terrain.start_motion([7., 0.02, 5.], body).unwrap();
    for _ in 0..40 {
        motion = terrain.advance(motion, [3., 0., 0.], 0.05, body).unwrap();
    }
    motion.position[0]
}

#[test]
fn bots_walk_through_layers_their_filter_row_passes() {
    let vehicle_only = corridor(Some(VEHICLE_ONLY));
    // Without movement layers every triangle is solid, as before.
    assert_eq!(vehicle_only.character_movement_layers(), None);
    assert!(walk(&vehicle_only) < 10.);
    let filtered = vehicle_only.with_character_movement_layers(Some(CHARACTER_ROW));
    assert!(walk(&filtered) > 12.);
    // Other queries still see the vehicle-only wall.
    let (start, end) = ([5., 1., 5.], [15., 1., 5.]);
    let mut budget = usize::MAX;
    assert!(
        filtered
            .raycast_segment(start, end)
            .unwrap()
            .nearest
            .is_some()
    );
    assert!(
        filtered
            .sweep_aabb(start, end, [0.35, 0.9, 0.35], &mut budget)
            .unwrap()
            .is_some()
    );
    assert!(
        filtered
            .movement_raycast_bounded(start, end, &mut budget)
            .unwrap()
            .nearest
            .is_none()
    );
}

#[test]
fn walls_on_movement_layers_or_without_metadata_stay_solid() {
    let solid = corridor(Some(STATIC_WORLD)).with_character_movement_layers(Some(CHARACTER_ROW));
    assert!(walk(&solid) < 10.);
    let unknown = corridor(None).with_character_movement_layers(Some(CHARACTER_ROW));
    assert!(walk(&unknown) < 10.);
}
