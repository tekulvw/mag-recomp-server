//! Actual Engine roster collection using invented external shape resources.
use super::*;
use crate::{dynamic_collision::*, dynamic_collision_runtime::RuntimeBindings};
#[path = "../tests/support/dynamic_collision_fixture.rs"]
mod support;
fn configured() -> Engine<Vec<u8>> {
    let e = acquisition_entry::acquisition();
    let catalog = Catalog::from_json(&support::fixture().to_string()).unwrap();
    e.with_dynamic_collision_bodies(
        catalog,
        RuntimeBindings {
            character_template: "invented-character".into(),
            vehicles: vec![],
        },
    )
    .unwrap()
}
#[test]
fn actual_roster_retains_initial_dead_and_moved_human_body_and_rejects_stale_context() {
    let mut e = configured();
    reserve(&mut e, 0);
    let at = |p| QueryBox {
        pose: Pose::at(p),
        half_extents: [0.1; 3],
        radius: 0.05,
    };
    let s = e.dynamic_collision_snapshot(u32::MAX).unwrap();
    assert_eq!(s.root_count(), 1);
    assert_eq!(
        s.penetrations(at([0., 0.5, 0.]), Default::default())
            .unwrap()
            .len(),
        1
    );
    assert!(e.dynamic_collision_snapshot_is_current(&s));
    e.active.as_mut().unwrap().life = CharacterLife::AwaitingRespawn { generation: 1 };
    e.active.as_mut().unwrap().tracking.server_position(
        CharacterPosition {
            x: 8.,
            y: 0.,
            z: 0.,
        },
        player_tracking::PositionOrigin::ServerSpawn,
        10,
    );
    // A genuine committed event normally advances this revision; no shared state
    // is changed by creating/querying a snapshot itself.
    e.sequence += 1;
    assert!(!e.dynamic_collision_snapshot_is_current(&s));
    let dead = e.dynamic_collision_snapshot(1 << 6).unwrap();
    assert_eq!(dead.root_count(), 1);
    assert!(
        dead.penetrations(at([0., 0.5, 0.]), Default::default())
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        dead.penetrations(at([8., 0.5, 0.]), Default::default())
            .unwrap()
            .len(),
        1
    );
    assert!(
        e.dynamic_collision_snapshot(1 << 21)
            .unwrap()
            .penetrations(at([8., 0.5, 0.]), Default::default())
            .unwrap()
            .is_empty()
    );
}
#[test]
fn missing_live_pose_or_ground_template_cannot_silently_drop_a_body() {
    let mut e = configured();
    reserve(&mut e, 0);
    e.active.as_mut().unwrap().life = CharacterLife::Alive {
        generation: 1,
        health_quarter_points: 400,
    };
    assert!(e.dynamic_collision_snapshot(u32::MAX).is_err());
    let e = acquisition_entry::acquisition();
    let bindings = RuntimeBindings {
        character_template: "missing".into(),
        vehicles: vec![],
    };
    assert!(
        e.with_dynamic_collision_bodies(
            Catalog::from_json(&support::fixture().to_string()).unwrap(),
            bindings
        )
        .is_err()
    );
}

#[test]
fn snapshots_from_another_engine_are_not_current_even_with_matching_world_revision() {
    let mut first = configured();
    reserve(&mut first, 0);
    let mut second = configured();
    reserve(&mut second, 0);
    let snapshot = first.dynamic_collision_snapshot(u32::MAX).unwrap();
    assert_eq!(first.sequence, second.sequence);
    assert!(!second.dynamic_collision_snapshot_is_current(&snapshot));
}

fn with_vehicle(hidden: bool) -> Engine<Vec<u8>> {
    let mut e = acquisition_entry::acquisition();
    let mut spawns = acquisition_entry::acquisition_spawns();
    if hidden {
        spawns["maps"][0]["spawn_count"] = json!(17);
        spawns["maps"][0]["locations"].as_array_mut().unwrap().push(json!({"index":16,"token":1016,"kind":"unsupported","active":false,"position":{"x":100.,"y":3.,"z":100.}}));
    }
    e.spawn_catalog = Some(super::spawns(spawns).unwrap());
    let mut source = json!({"spawn_catalog_sha256":e.spawn_catalog.as_ref().unwrap().source_sha256().unwrap(),"native_elf_sha256":"a".repeat(64),"native_binding_sha256":"b".repeat(64),"vehicle_properties_sha256":"c".repeat(64)});
    if hidden {
        source["creation_scope"] = json!("native_requested_free_slot_hidden_state0");
    }
    let input = json!({"format":"mag_ground_vehicles","schema_version":1,"source":source,
      "vehicles":[{"index":if hidden {4}else{0},"spawn_point_index":if hidden {16}else{0},"spawn_point_token":if hidden {1016}else{1000},"vehicle_type_token":12345,"faction":1,"team":0,"modifier":0,"initial_state":if hidden {0}else{2},"position":{"x":100.,"y":3.,"z":100.},"rotation":[0.,0.,0.,1.],"health_quarter_points":600}]});
    let vehicles = crate::ground_vehicles::Catalog::from_json(
        &input.to_string(),
        e.spawn_catalog.as_ref().unwrap(),
    )
    .unwrap();
    let bodies = Catalog::from_json(&support::fixture().to_string()).unwrap();
    let bindings = RuntimeBindings {
        character_template: "invented-character".into(),
        vehicles: vec![crate::dynamic_collision_runtime::VehicleBinding {
            vehicle_type_token: 12345,
            faction: 1,
            template: "invented-vehicle".into(),
        }],
    };
    e.with_dynamic_collision_bodies(bodies, bindings)
        .unwrap()
        .with_ground_vehicles(vehicles)
        .unwrap()
}
#[test]
fn operational_ground_keeps_both_roots_and_mounted_character_follows_vehicle() {
    let mut e = with_vehicle(false);
    reserve(&mut e, 0);
    let actor = e.active.as_mut().unwrap();
    actor.life = CharacterLife::Alive {
        generation: 1,
        health_quarter_points: 400,
    };
    actor.ground_seat = Some(crate::ground_seats::PassengerSeat {
        vehicle_index: 0,
        seat: 2,
    });
    let snapshot = e.dynamic_collision_snapshot(1 << 21).unwrap();
    assert_eq!(snapshot.root_count(), 3);
    assert_eq!(
        snapshot
            .penetrations(
                QueryBox {
                    pose: Pose::at([100., 3.5, 100.]),
                    half_extents: [0.1; 3],
                    radius: 0.05
                },
                Default::default()
            )
            .unwrap()
            .len(),
        1
    );
}
#[test]
fn allocated_hidden_ground_is_in_roster_but_has_no_registered_roots() {
    let mut e = with_vehicle(true);
    reserve(&mut e, 0);
    let snapshot = e.dynamic_collision_snapshot(u32::MAX).unwrap();
    assert_eq!(snapshot.entity_count(), 2);
    assert_eq!(snapshot.root_count(), 1);
}

#[test]
fn vehicle_source_metadata_can_admit_nonhuman_active_truck_without_enabling_player_spawn() {
    let mut e = acquisition_entry::acquisition();
    let mut value = acquisition_entry::acquisition_spawns();
    value["maps"][0]["spawn_count"] = json!(17);
    value["maps"][0]["locations"].as_array_mut().unwrap().push(json!({"index":16,"token":1016,"kind":"unsupported","active":false,"position":{"x":100.,"y":3.,"z":100.}}));
    e.spawn_catalog = Some(super::spawns(value).unwrap());
    let spawns = e.spawn_catalog.as_ref().unwrap();
    let mut input = json!({"format":"mag_ground_vehicles","schema_version":1,
      "source":{"spawn_catalog_sha256":spawns.source_sha256().unwrap(),"native_elf_sha256":"a".repeat(64),"native_binding_sha256":"b".repeat(64),"vehicle_properties_sha256":"c".repeat(64),"creation_scope":"native_vehicle_source_points"},
      "vehicles":[{"index":4,"spawn_point_index":16,"spawn_point_token":1016,"vehicle_type_token":12345,"faction":1,"team":0,"modifier":0,"initial_state":2,"position":{"x":100.,"y":3.,"z":100.},"rotation":[0.,0.,0.,1.],"health_quarter_points":600}],
      "vehicle_sources":[{"vehicle_index":4,"spawn_point_index":16,"spawn_point_token":1016,"vehicle_type_token":12345,"spawn_type":7,"user_class":1,"source_family":35,"authored_active":true,"team":0}]});
    assert!(crate::ground_vehicles::Catalog::from_json(&input.to_string(), spawns).is_ok());
    assert!(!spawns.point(16).unwrap().active);
    input["vehicle_sources"][0]["spawn_point_token"] = json!(1017);
    assert!(crate::ground_vehicles::Catalog::from_json(&input.to_string(), spawns).is_err());
    input["vehicle_sources"][0]["spawn_point_token"] = json!(1016);
    input["vehicle_sources"][0]["authored_active"] = json!(false);
    assert!(crate::ground_vehicles::Catalog::from_json(&input.to_string(), spawns).is_err());
}
