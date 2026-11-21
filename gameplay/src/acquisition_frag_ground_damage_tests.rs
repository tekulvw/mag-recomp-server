//! Authenticated ordinary frag against a normally captured synthetic Escort.
use super::*;
use std::sync::Arc;

const ESCORT: u16 = 4;

fn fixture(friendly: bool) -> Engine<Vec<u8>> {
    let mut engine = ground_firearm_fixture(0., friendly);
    let structural = crate::acquisition_structural::connected_tests::fixture()
        .acquisition_gameplay
        .unwrap()
        .setup
        .structural
        .clone()
        .unwrap();
    let runtime = engine.acquisition_gameplay.as_mut().unwrap();
    crate::acquisition_gameplay_engine::test_support::install_structural_fixture(
        runtime, structural,
    );
    let setup = Arc::get_mut(&mut runtime.setup).unwrap();
    // Static wall uses raw root 9, child 0, mirroring the source filter pair.
    setup.filter_rows[24] |= (1 << 22) | 1;
    setup.filter_rows[22] |= 1 << 24;
    setup.frag_ground_areas = Some(Arc::new(
        crate::acquisition_frag_ground::Catalog::synthetic(41, 42, 12345),
    ));
    let mut areas = vec![json!({"armor_area":0,"multiplier":1.}); 16];
    areas[6] = json!({"armor_area":2,"multiplier":0.5});
    setup.ground_damage = Some(
        serde_json::from_value(json!({
            "friendly_fire_multiplier":0.5,
            "profiles":[{"vehicle_type_token":12345,"faction":1,"max_health":150.,
                "critical_loss_percent_per_second":0.,
                "armor":{"rating":10.,"prevention":0.5,"coverage":[0.,0.,0.5,0.]}}],
            "areas":areas,
        }))
        .unwrap(),
    );
    engine.item_catalog = crate::human_kill_reward_tests::vehicle_grenade_items_with_cover(100.);
    let ground = engine.ground_runtime.as_mut().unwrap();
    let escort = ground.get(ESCORT).unwrap();
    assert_eq!(escort.state, 2);
    let position = escort.pose.position;
    ground
        .force_pose(
            escort.key,
            ground::Pose {
                position,
                rotation: [0., 1., 0., 0.],
            },
        )
        .unwrap();
    engine.collision_world = Some(scene(&engine, position.z + 20., Some(255)));

    let batches = engine.ground_vehicles.as_ref().unwrap().batch_count();
    let actor = engine.active.as_mut().unwrap();
    actor.ground_vehicle_batch = batches;
    actor.tracking.server_position(
        CharacterPosition {
            x: position.x + 15.,
            ..position
        },
        crate::player_tracking::PositionOrigin::ServerSpawn,
        3200,
    );
    let mut kit = actor.spawned_loadout.take().unwrap();
    kit.inventories[0].slots[4] = Some(4);
    actor.spawned_loadout = Some(kit);
    actor.grenade_remaining = 1;
    let launch = crate::hospital_rocket_runtime::tests::launch_wire(&engine, 3, 4);
    assert!(!dispatch(&mut engine, &launch, 3300).unwrap().is_empty());
    assert!(engine.active.as_ref().unwrap().projectiles.get(3).is_some());
    engine
}

fn scene(engine: &Engine<Vec<u8>>, z: f32, material: Option<u8>) -> collision::CollisionWorld {
    let position = engine.current_ground_vehicle(ESCORT).unwrap().position;
    let (x, y) = (position.x, position.y);
    collision::CollisionWorld::from_bytes(
        json!({
            "format":"mag_collision_world","schema_version":1,"coordinate_space":"world",
            "metadata":{"map_name":engine.map_name(),"build":"synthetic",
                "coverage":"provisional","provenance":["synthetic"],"unknowns":[]},
            "instances":[{"id":"frag-wall","vertices_xyz":[[x-4.,y-4.,z],[x+4.,y-4.,z],[x,y+4.,z]],
                "triangles":[[0,1,2]],"material_index":material,
                "collision":{"filter_info":9,"ray_child_filter_info":0,"convex_radius":0.,
                    "provenance":"synthetic","child_filter_policy":"uniform"}}]
        })
        .to_string()
        .as_bytes(),
        Default::default(),
    )
    .unwrap()
}

fn terminal(engine: &Engine<Vec<u8>>) -> Vec<u8> {
    let position = engine.current_ground_vehicle(ESCORT).unwrap().position;
    grenade_report_for_slot(
        engine,
        200,
        CharacterPosition {
            z: position.z + 2.,
            ..position
        },
        3,
    )
}

fn last_event(engine: &Engine<Vec<u8>>) -> Value {
    engine
        .writer
        .split(|&byte| byte == b'\n')
        .rev()
        .filter_map(|row| serde_json::from_slice::<Value>(row).ok())
        .find(|row| row["op"] == "event")
        .unwrap()
}

fn vehicle_damage(engine: &Engine<Vec<u8>>) -> replication::VehicleDamage {
    engine
        .ground_runtime
        .as_ref()
        .unwrap()
        .get(ESCORT)
        .unwrap()
        .damage_wire()
}

#[test]
fn acquisition_frag_ground_native_area_changes_escort_health_once() {
    for (friendly, expected_health) in [(false, 502), (true, 558)] {
        let mut engine = fixture(friendly);
        let wire = terminal(&engine);
        dispatch(&mut engine, &wire, 3400).unwrap();
        let after = engine.ground_runtime.clone().unwrap();
        let damage = after.get(ESCORT).unwrap().damage_wire();
        // 100 × .75 falloff, optional .5 FF, subtract 10, then × .5 area × .75 coverage.
        assert_eq!(damage.health_quarter_points, expected_health);
        assert_eq!(damage.last_attacker, 0x0380_0000 | u32::from(!friendly));
        assert_eq!(damage.last_location_and_type, 0xa6);
        assert_eq!(damage.last_user_data, 128 | 512); // Negative Z points away from blast.
        assert_eq!(damage.last_source_object, u32::MAX);
        let event = last_event(&engine);
        let rows = event["decoded"]["ground_grenade_checks"]
            .as_array()
            .unwrap();
        assert_eq!(rows.len(), 1); // Both registered collision roots overlap.
        assert_eq!(rows[0]["root"], "chassis");
        assert_eq!(
            rows[0]["projectile_id"],
            0x1380_0000_u32 | u32::from(!friendly)
        );
        dispatch(&mut engine, &wire, 3401).unwrap();
        assert!(engine.ground_runtime.as_ref() == Some(&after));
    }
}

#[test]
fn acquisition_frag_ground_accepts_wheel_only_overlap() {
    let mut engine = fixture(false);
    let active = engine.active.take();
    let peers = std::mem::take(&mut engine.peers);
    let mut document = support::fixture();
    document["templates"][0]["roots"][0]["translation"] = json!([0., 20., 0.]);
    let catalog = Catalog::from_json(&document.to_string()).unwrap();
    engine = engine
        .with_dynamic_collision_bodies(
            catalog,
            RuntimeBindings {
                character_template: "invented-character".to_owned(),
                vehicles: vec![crate::dynamic_collision_runtime::VehicleBinding {
                    vehicle_type_token: 12345,
                    faction: 1,
                    template: "invented-vehicle".to_owned(),
                }],
            },
        )
        .unwrap();
    engine.active = active;
    engine.peers = peers;
    let wire = terminal(&engine);
    dispatch(&mut engine, &wire, 3400).unwrap();
    assert_eq!(vehicle_damage(&engine).health_quarter_points, 502);
    assert_eq!(
        last_event(&engine)["decoded"]["ground_grenade_checks"][0]["root"],
        "wheel"
    );
}

#[test]
fn acquisition_frag_ground_uses_accepted_same_event_driver_pose() {
    let mut engine = fixture(true);
    let before = engine.current_ground_vehicle(ESCORT).unwrap().position;
    let moved = CharacterPosition {
        z: before.z - 1.,
        ..before
    };
    let actor = engine.active.as_mut().unwrap();
    actor.ground_seat = Some(ground_seats::PassengerSeat {
        vehicle_index: ESCORT,
        seat: 0,
    });
    let player = actor.reservation.player;
    engine
        .ground_runtime
        .as_mut()
        .unwrap()
        .begin_driver(ESCORT, 3300)
        .unwrap();

    let mut bits = replication::BitWriter::with_profile(engine.wire_profile);
    UpdateHeader {
        sequence: 200,
        nak_sequence: 0,
        client_index: 0,
        object_type_mask: (1 << 1) | (1 << 19),
        timestamp_ms: 3400,
    }
    .encode(&mut bits)
    .unwrap();
    bits.write(1, 1).unwrap();
    bits.write(ESCORT.into(), 16).unwrap();
    bits.write(0, 25).unwrap();
    bits.write(1, 1).unwrap(); // Ground position field25.
    moved.encode(&mut bits).unwrap();
    bits.write(0, 5).unwrap();
    bits.write(1, 1).unwrap();
    bits.write(player.into(), 16).unwrap();
    projectile::ProjectileOwnerFields {
        state: Some(2),
        position: Some(CharacterPosition {
            z: before.z + 2.,
            ..before
        }),
        rotation_xyzw: None,
    }
    .encode_fields(&mut bits)
    .unwrap();
    bits.write(0, 1).unwrap();
    dispatch(&mut engine, &bits.finish(), 3400).unwrap();
    assert_eq!(
        engine.current_ground_vehicle(ESCORT).unwrap().position,
        moved
    );
    // Three metres to the moved model origin yields 0.625 rather than 0.75.
    assert_eq!(
        vehicle_damage(&engine).health_quarter_points,
        568,
        "{}",
        last_event(&engine)
    );
}

#[test]
fn acquisition_frag_ground_cover_and_missing_sources_keep_the_terminal() {
    for reason in [
        None,
        Some("acquisition_frag_ground_requires_structural"),
        Some("dynamic_body_resources_unavailable"),
    ] {
        let mut engine = fixture(false);
        match reason {
            None => {
                let position = engine.current_ground_vehicle(ESCORT).unwrap().position;
                engine.collision_world = Some(scene(&engine, position.z + 1., Some(255)));
            }
            Some("acquisition_frag_ground_requires_structural") => {
                crate::acquisition_gameplay_engine::test_support::disable_structural_fixture(
                    engine.acquisition_gameplay.as_mut().unwrap(),
                );
            }
            _ => engine.dynamic_body_resources = None,
        }
        let wire = terminal(&engine);
        dispatch(&mut engine, &wire, 3400).unwrap();
        let source = engine.active.as_ref().unwrap();
        let event = last_event(&engine);
        assert_eq!(
            vehicle_damage(&engine).health_quarter_points,
            600,
            "{event}"
        );
        assert!(source.projectiles.get(3).unwrap().completed);
        assert!(engine.projectile_cleanup_pending(source, 3));
        assert_eq!(event["gameplay_denial"].as_str(), reason, "{event}");
    }
}

#[test]
fn acquisition_frag_ground_rejects_stale_fuse_and_rolls_back_on_record_failure() {
    let mut engine = fixture(false);
    let wire = terminal(&engine);
    let before = engine.ground_runtime.clone();
    engine.active.as_mut().unwrap().latest_owner_sequence = 200;
    dispatch(&mut engine, &wire, 3400).unwrap();
    assert!(
        !engine
            .active
            .as_ref()
            .unwrap()
            .projectiles
            .get(3)
            .unwrap()
            .completed
    );
    assert!(engine.ground_runtime == before);
    let position = engine.current_ground_vehicle(ESCORT).unwrap().position;
    let fresh = grenade_report_for_slot(
        &engine,
        201,
        CharacterPosition {
            z: position.z + 2.,
            ..position
        },
        3,
    );
    engine.max_log = engine.written;
    assert_eq!(
        dispatch(&mut engine, &fresh, 3401).unwrap_err(),
        "log_limit_reached"
    );
    let actor = engine.active.as_ref().unwrap();
    assert!(!actor.projectiles.get(3).unwrap().completed);
    assert!(!engine.projectile_cleanup_pending(actor, 3));
    assert!(engine.ground_runtime == before);
}
