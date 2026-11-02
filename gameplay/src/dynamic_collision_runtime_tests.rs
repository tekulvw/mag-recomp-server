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
        source["creation_scope"] = json!("native_vehicle_source_points");
    }
    let mut input = json!({"format":"mag_ground_vehicles","schema_version":1,"source":source,
      "vehicles":[{"index":if hidden {4}else{0},"spawn_point_index":if hidden {16}else{0},"spawn_point_token":if hidden {1016}else{1000},"vehicle_type_token":12345,"faction":1,"team":0,"modifier":0,"initial_state":if hidden {0}else{2},"position":{"x":100.,"y":3.,"z":100.},"rotation":[0.,0.,0.,1.],"health_quarter_points":600}]});
    if hidden {
        input["vehicle_sources"] = json!([{"vehicle_index":4,"spawn_point_index":16,"spawn_point_token":1016,"vehicle_type_token":12345,"spawn_type":9,"user_class":1,"source_family":35,"authored_active":false,"team":0}]);
    }
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

fn respawn_fixture() -> (
    Engine<Vec<u8>>,
    crate::acquisition_spawn_effects::RespawnRequest,
) {
    let mut e = with_vehicle(true);
    reserve(&mut e, 0);
    let actor = e.active.as_mut().unwrap();
    ready(actor);
    actor.life = CharacterLife::Alive {
        generation: 1,
        health_quarter_points: 400,
    };
    actor.tracking.server_position(
        CharacterPosition {
            x: 101.45,
            y: 3.,
            z: 100.,
        },
        player_tracking::PositionOrigin::ServerSpawn,
        0,
    );
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "synthetic-acq-respawn-{}-{}.json",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::write(&path,json!({"format":"mag_collision_world","schema_version":1,"coordinate_space":"world","metadata":{"map_name":e.spawn_catalog.as_ref().unwrap().map_name(),"build":"synthetic","coverage":"complete","provenance":["explicit invented empty static world"],"unknowns":[]},"instances":[]}).to_string()).unwrap();
    e.collision_world =
        Some(crate::collision::CollisionWorld::load(&path, Default::default()).unwrap());
    std::fs::remove_file(path).unwrap();
    let current = e.current_ground_vehicle(4).unwrap();
    let request = crate::acquisition_spawn_effects::RespawnRequest {
        source: crate::acquisition_ground_runtime::Key {
            index: 4,
            generation: 1,
        },
        point: crate::acquisition_ground_runtime::QualifiedRespawn {
            spawn_point_index: current.spawn_point_index,
            spawn_point_token: current.spawn_point_token,
            active: true,
            pose: crate::acquisition_ground_runtime::Pose {
                position: current.position,
                rotation: current.rotation,
            },
        },
        initial: false,
        physics: Default::default(),
        filter_rows: [u32::MAX; 32],
        now: Duration::from_millis(10),
    };
    (e, request)
}
#[test]
fn actual_existing_respawn_stages_push_and_records_before_ground_and_character_install() {
    let (mut e, request) = respawn_fixture();
    let mut native = Default::default();
    let before = e.sequence;
    let prepared = e
        .prepare_existing_acquisition_respawn(request, &native)
        .unwrap();
    assert_eq!(e.sequence, before);
    assert_eq!(e.ground_runtime.as_ref().unwrap().get(4).unwrap().state, 0);
    assert_eq!(
        e.active
            .as_ref()
            .unwrap()
            .tracking
            .sample
            .unwrap()
            .position
            .x,
        101.45
    );
    assert!(!e.active.as_ref().unwrap().tracking.correction_pending());
    e.record_existing_acquisition_respawn(prepared, &mut native, 1)
        .unwrap();
    assert_eq!(e.sequence, before + 1);
    let vehicle = e.ground_runtime.as_ref().unwrap().get(4).unwrap();
    assert_eq!(vehicle.state, 2);
    assert_eq!(vehicle.spawn_counter, 1);
    assert_eq!(
        vehicle.body,
        crate::acquisition_ground_runtime::Body::Inactive
    );
    let actor = e.active.as_ref().unwrap();
    assert!(actor.tracking.sample.unwrap().position.x > 101.5);
    assert!(actor.tracking.correction_pending());
    assert!(matches!(
        actor.life,
        CharacterLife::Alive {
            health_quarter_points: 400,
            ..
        }
    ));
    assert!(!actor.pending_shared.is_empty());
    e.tick_acquisition_ground_registration(1).unwrap();
    assert_eq!(
        e.ground_runtime.as_ref().unwrap().get(4).unwrap().body,
        crate::acquisition_ground_runtime::Body::Operational
    );
    assert_eq!(
        e.dynamic_collision_snapshot(u32::MAX).unwrap().root_count(),
        3
    );
}
#[test]
fn respawn_record_failure_and_missing_world_preserve_all_owned_authority() {
    let (mut e, request) = respawn_fixture();
    let mut native = Default::default();
    let prepared = e
        .prepare_existing_acquisition_respawn(request, &native)
        .unwrap();
    e.max_log = e.written;
    assert!(
        e.record_existing_acquisition_respawn(prepared, &mut native, 1)
            .is_err()
    );
    assert_eq!(e.ground_runtime.as_ref().unwrap().get(4).unwrap().state, 0);
    assert_eq!(
        e.active
            .as_ref()
            .unwrap()
            .tracking
            .sample
            .unwrap()
            .position
            .x,
        101.45
    );
    assert!(!e.active.as_ref().unwrap().tracking.correction_pending());
    let (mut e, request) = respawn_fixture();
    e.collision_world = None;
    assert!(
        e.prepare_existing_acquisition_respawn(request, &native)
            .is_err()
    );
    assert_eq!(
        e.ground_runtime
            .as_ref()
            .unwrap()
            .get(4)
            .unwrap()
            .spawn_counter,
        0
    );
}

#[test]
fn later_blocker_failure_and_changed_ground_candidate_cannot_install_respawn() {
    let (mut e, request) = respawn_fixture();
    let mut native = Default::default();
    e.active.as_mut().unwrap().tracking.server_position(
        CharacterPosition {
            x: 100.,
            y: 3.,
            z: 100.,
        },
        player_tracking::PositionOrigin::ServerSpawn,
        0,
    );
    let sequence = e.sequence;
    assert!(matches!(
        e.prepare_existing_acquisition_respawn(request, &native),
        Err("acquisition_character_native_state_missing")
    ));
    assert_eq!(e.sequence, sequence);
    assert_eq!(
        e.ground_runtime
            .as_ref()
            .unwrap()
            .get(4)
            .unwrap()
            .spawn_counter,
        0
    );
    let (mut e, request) = respawn_fixture();
    let prepared = e
        .prepare_existing_acquisition_respawn(request, &native)
        .unwrap();
    let runtime = e.ground_runtime.as_mut().unwrap();
    let key = runtime.get(4).unwrap().key;
    let mut pose = runtime.get(4).unwrap().pose;
    pose.position.x += 1.;
    runtime.force_pose(key, pose).unwrap();
    assert!(matches!(
        e.record_existing_acquisition_respawn(prepared, &mut native, 1),
        Err("stale_acquisition_ground_candidate")
    ));
    assert_eq!(e.ground_runtime.as_ref().unwrap().get(4).unwrap().state, 0);
    assert!(!e.active.as_ref().unwrap().tracking.correction_pending());
}

#[test]
fn bot_body_uses_current_life_and_integrator_pose_after_respawn() {
    let mut e = configured();
    reserve(&mut e, 0);
    let mut bots =
        crate::bot_registry::identity_tests::populated_engine("Synthetic Body Generation");
    let (id, bot) = bots.bot_registry.bots.iter_mut().next().unwrap();
    let id = *id;
    bot.target.generation = 1;
    let runtime = bot.target.runtime.as_mut().unwrap();
    runtime.generation = 7;
    runtime.position = CharacterPosition {
        x: 9.,
        y: 0.,
        z: 9.,
    };
    runtime.physics_motion.as_mut().unwrap().position = [6.002, 0., 6.001];
    e.bot_registry.bots = bots.bot_registry.bots;
    let snapshot = e.dynamic_collision_snapshot(u32::MAX).unwrap();
    let hits = snapshot
        .penetrations(
            QueryBox {
                pose: Pose::at([6.002, 0.5, 6.001]),
                half_extents: [0.01; 3],
                radius: 0.05,
            },
            Default::default(),
        )
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].target.key.id, u64::from(0x03800000 | u32::from(id)));
    assert_eq!(hits[0].target.key.generation, 7);
}
