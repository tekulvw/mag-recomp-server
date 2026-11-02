use mag_gameplay::{
    acquisition_atomic::WorldKey,
    acquisition_spawn_blockers::BlockerQueries,
    acquisition_volume_damage::{Target, TargetKey, TargetKind},
    acquisition_world::{Filters, RootKey, Settings, World},
    collision::{CollisionLimits, CollisionWorld},
    dynamic_collision::{Catalog, Pose, Snapshot},
};
#[path = "support/dynamic_collision_fixture.rs"]
mod support;
use std::collections::BTreeMap;
fn target(id: u64, kind: TargetKind) -> Target {
    Target {
        key: TargetKey { id, generation: 1 },
        kind,
        team: 0,
    }
}
fn static_world(collision: bool) -> CollisionWorld {
    static_world_with_region(collision, false)
}
fn static_world_with_region(collision: bool, regional: bool) -> CollisionWorld {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "synthetic-world-{}-{}.json",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let mut row = serde_json::json!({"id":"invented-wall","vertices_xyz":[[3.,-4.,-4.],[3.,4.,-4.],[3.,0.,4.]],"triangles":[[0,1,2]]});
    if collision {
        row["collision"] = serde_json::json!({"filter_info":0,"convex_radius":0.,"child_filter_policy":"uniform","provenance":"invented static filter and margin"});
    }
    let mut value = serde_json::json!({"format":"mag_collision_world","schema_version":1,"coordinate_space":"world","metadata":{"map_name":"invented","build":"synthetic","coverage":"complete","provenance":["invented"],"unknowns":[]},"instances":[row]});
    if regional {
        value["metadata"]["coverage"] = serde_json::json!("provisional");
        value["metadata"]["unknowns"] =
            serde_json::json!(["Outside the invented verified region is unknown"]);
        value["metadata"]["physics_regions"] = serde_json::json!([{"min":[-2.,-2.,-2.],"max":[5.,3.,3.],"ground_verified":true,"obstacles_verified":true,"provenance":["invented regional coverage"]}]);
    }
    std::fs::write(&path, value.to_string()).unwrap();
    let w = CollisionWorld::load(&path, CollisionLimits::default()).unwrap();
    std::fs::remove_file(path).unwrap();
    w
}

#[test]
fn region_must_cover_the_entire_swept_capsule_before_any_hit_is_returned() {
    let (s, a, b, filters) = snapshot();
    let statics = static_world_with_region(true, true);
    let mut w = World::new(&s, &statics, filters, Settings::default()).unwrap();
    let mut actor = w.character(a, b).unwrap();
    assert!(actor.cast_world([4., 0., 0.]).unwrap().is_some());
    assert!(matches!(
        actor.cast_world([6., 0., 0.]),
        Err("world_cast_outside_verified_region")
    ));
}
#[test]
fn excluding_the_spawning_chassis_does_not_exclude_its_other_root() {
    let mut fixture = support::fixture();
    fixture["templates"][0]["roots"][1]["translation"] = serde_json::json!([1.9, 0., 0.]);
    fixture["templates"][0]["roots"][1]["shape"]["half_extents"] =
        serde_json::json!([0.1, 0.1, 0.1]);
    let catalog = Catalog::from_json(&fixture.to_string()).unwrap();
    let actor = target(3, TargetKind::Character);
    let source = target(1, TargetKind::GroundVehicle);
    let snapshot = Snapshot::capture(
        WorldKey {
            world_id: 1,
            incarnation: 1,
        },
        0,
        &[actor, source],
        vec![
            catalog
                .instantiate("invented-character", actor, Pose::at([1.45, 0., 0.]), false)
                .unwrap(),
            catalog
                .instantiate("invented-vehicle", source, Pose::at([0.; 3]), false)
                .unwrap(),
        ],
        u32::MAX,
    )
    .unwrap();
    let statics = static_world(true);
    let filters = Filters::owned_entities(&snapshot, &statics, [u32::MAX; 32]).unwrap();
    let mut w = World::new(&snapshot, &statics, filters, Settings::default()).unwrap();
    let mut actor = w
        .character(
            RootKey {
                entity: 3,
                generation: 1,
                root: "capsule".into(),
            },
            RootKey {
                entity: 1,
                generation: 1,
                root: "chassis".into(),
            },
        )
        .unwrap();
    assert_eq!(
        actor.cast_world([1.5, 0., 0.]).unwrap().unwrap().fraction,
        0.
    );
}
fn snapshot() -> (Snapshot, RootKey, RootKey, Filters) {
    let c = Catalog::from_json(&support::fixture().to_string()).unwrap();
    let actor = target(3, TargetKind::Character);
    let vehicle = target(1, TargetKind::GroundVehicle);
    let rows = vec![
        c.instantiate("invented-character", actor, Pose::at([1.45, 0., 0.]), false)
            .unwrap(),
        c.instantiate("invented-vehicle", vehicle, Pose::at([0.; 3]), false)
            .unwrap(),
    ];
    let s = Snapshot::capture(
        WorldKey {
            world_id: 1,
            incarnation: 1,
        },
        0,
        &[actor, vehicle],
        rows,
        u32::MAX,
    )
    .unwrap();
    let character = RootKey {
        entity: 3,
        generation: 1,
        root: "capsule".into(),
    };
    let spawn = RootKey {
        entity: 1,
        generation: 1,
        root: "chassis".into(),
    };
    let mut roots = BTreeMap::new();
    roots.insert(character.clone(), 21 | (1 << 16));
    roots.insert(spawn.clone(), 22 | (2 << 16));
    roots.insert(
        RootKey {
            root: "wheel".into(),
            ..spawn.clone()
        },
        22 | (2 << 16),
    );
    (
        s,
        character,
        spawn,
        Filters {
            rows: [u32::MAX; 32],
            roots,
        },
    )
}
#[test]
fn actual_static_sweep_hits_wall_and_separating_start_contact_can_escape() {
    let (s, a, b, filters) = snapshot();
    let statics = static_world(true);
    let mut world = World::new(&s, &statics, filters, Settings::default()).unwrap();
    let mut actor = world.character(a, b).unwrap();
    let contact = actor.closest_spawn_contact().unwrap().unwrap();
    assert!((contact.signed_distance + 0.05).abs() < 1e-5);
    assert!(contact.normal[0] > 0.99);
    let hit = actor.cast_world([4., 0., 0.]).unwrap().unwrap();
    assert!((hit.fraction - (3. - 0.4 - 1.45) / (4. - 1.45)).abs() < 0.002);
    assert!(hit.normal[0] < -0.99);
    assert!(actor.cast_world([1.5, 0., 0.]).unwrap().is_none());
    actor.move_character([1.5, 0., 0.]).unwrap();
    assert_eq!(actor.position(), [1.5, 0., 0.]);
}
#[test]
fn missing_static_or_current_dynamic_filter_data_is_an_error() {
    let (s, _, _, filters) = snapshot();
    assert!(
        World::new(
            &s,
            &static_world(false),
            filters.clone(),
            Settings::default()
        )
        .is_err()
    );
    let mut missing = filters;
    missing.roots.clear();
    assert!(World::new(&s, &static_world(true), missing, Settings::default()).is_err());
}
#[test]
fn full_group_filter_uses_subgroup_exclusions_before_layer_table() {
    let (s, a, b, mut filters) = snapshot();
    filters.rows = [0; 32];
    // Same nonzero group bypasses layer mask. Matching exclusion disables.
    assert!(Filters::allows(
        &filters.rows,
        (7 << 16) | (1 << 5),
        (7 << 16) | (2 << 5)
    ));
    assert!(!Filters::allows(
        &filters.rows,
        (7 << 16) | (1 << 5),
        (7 << 16) | (1 << 10)
    ));
    let statics = static_world(true);
    let mut w = World::new(&s, &statics, filters, Settings::default()).unwrap();
    assert!(
        w.character(a, b)
            .unwrap()
            .cast_world([4., 0., 0.])
            .unwrap()
            .is_none()
    );
}
#[test]
fn actual_push_handler_moves_shallow_blocker_and_limits_fail_without_partial_success() {
    let (s, a, b, filters) = snapshot();
    let statics = static_world(true);
    let mut w = World::new(&s, &statics, filters.clone(), Settings::default()).unwrap();
    let result = w.resolve_spawn(&b, &Default::default()).unwrap();
    assert!(result.lethal_characters.is_empty());
    assert_eq!(result.moved.len(), 1);
    assert!(result.moved[0].2[0] > 1.5);
    let mut w = World::new(
        &s,
        &statics,
        filters,
        Settings {
            query_limits: mag_gameplay::dynamic_collision::QueryLimits {
                max_support_points: 1,
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .unwrap();
    assert!(w.character(a, b).unwrap().cast_world([4., 0., 0.]).is_err());
}
#[test]
fn allocated_hidden_spawn_shape_can_resolve_without_registering_roots() {
    let catalog = Catalog::from_json(&support::fixture().to_string()).unwrap();
    let actor = target(3, TargetKind::Character);
    let source = target(1, TargetKind::GroundVehicle);
    let snapshot = Snapshot::capture(
        WorldKey {
            world_id: 1,
            incarnation: 1,
        },
        0,
        &[actor, source],
        vec![
            catalog
                .instantiate("invented-character", actor, Pose::at([1.45, 0., 0.]), false)
                .unwrap(),
            catalog
                .instantiate_inactive_ground("invented-vehicle", source, Pose::at([0.; 3]))
                .unwrap(),
        ],
        u32::MAX,
    )
    .unwrap();
    let statics = static_world(true);
    let filters = Filters::owned_entities(&snapshot, &statics, [u32::MAX; 32]).unwrap();
    let mut world = World::new(&snapshot, &statics, filters, Settings::default()).unwrap();
    let source = world
        .add_allocated_spawn(
            &catalog,
            "invented-vehicle",
            source,
            Pose::at([0.; 3]),
            "chassis",
        )
        .unwrap();
    assert_eq!(snapshot.root_count(), 1);
    assert_eq!(
        world
            .resolve_spawn(&source, &Default::default())
            .unwrap()
            .moved
            .len(),
        1
    );
}

#[test]
fn deep_character_overlap_is_lethal_and_vehicle_root_multiplicity_is_preserved() {
    let catalog = Catalog::from_json(&support::fixture().to_string()).unwrap();
    let source = target(1, TargetKind::GroundVehicle);
    let other = target(2, TargetKind::GroundVehicle);
    let actor = target(3, TargetKind::Character);
    let snapshot = Snapshot::capture(
        WorldKey {
            world_id: 1,
            incarnation: 1,
        },
        0,
        &[source, other, actor],
        vec![
            catalog
                .instantiate("invented-vehicle", source, Pose::at([0.; 3]), false)
                .unwrap(),
            catalog
                .instantiate("invented-vehicle", other, Pose::at([1.95, 0., 0.]), false)
                .unwrap(),
            catalog
                .instantiate("invented-character", actor, Pose::at([0.; 3]), false)
                .unwrap(),
        ],
        u32::MAX,
    )
    .unwrap();
    let statics = static_world(true);
    let filters = Filters::owned_entities(&snapshot, &statics, [u32::MAX; 32]).unwrap();
    let mut world = World::new(&snapshot, &statics, filters, Settings::default()).unwrap();
    let resolution = world
        .resolve_spawn(
            &RootKey {
                entity: 1,
                generation: 1,
                root: "chassis".into(),
            },
            &[(2, 1)].into(),
        )
        .unwrap();
    assert_eq!(resolution.lethal_characters, vec![actor]);
    assert!(resolution.moved.is_empty());
    assert_eq!(resolution.lethal_ground, vec![other, other]);
}
