//! Domination's ordinary slot3 frag through the recorded owner event.
use super::*;
use crate::{
    projectile::ProjectileOwnerFields, replication::CharacterPosition, spawn_progression::dispatch,
};
use std::sync::Arc;

const APC: u16 = 0;
const MOTOR_POOL: u16 = 8;
const TARGET: CharacterPosition = CharacterPosition {
    x: 2.,
    y: 2.,
    z: 2.,
};
const ORIGIN: CharacterPosition = CharacterPosition { z: 4., ..TARGET };

fn fixture(target: u16, team: u8, enabled: bool) -> Engine<Vec<u8>> {
    fixture_with_drivers(target, team, enabled, false)
}

fn fixture_with_drivers(
    target: u16,
    team: u8,
    enabled: bool,
    use_drivers: bool,
) -> Engine<Vec<u8>> {
    let mut engine =
        crate::manual_ground_repair_domination_tests::fixture_with_drivers(0, team, use_drivers);
    let active = engine.active.take();
    engine = engine
        .with_dynamic_collision_bodies(
            dynamic_collision::Catalog::from_json(
                &crate::synthetic_dynamic_collision::fixture().to_string(),
            )
            .unwrap(),
            crate::dynamic_collision_runtime::RuntimeBindings {
                character_template: "invented-character".into(),
                vehicles: [(110, 0), (111, 1)]
                    .into_iter()
                    .map(|(vehicle_type_token, faction)| {
                        crate::dynamic_collision_runtime::VehicleBinding {
                            vehicle_type_token,
                            faction,
                            template: "invented-vehicle".into(),
                        }
                    })
                    .collect(),
            },
        )
        .unwrap();
    engine.active = active;
    engine.collision_world = Some(scene(&engine, ORIGIN.z + 20., Some(255)));
    engine.item_catalog = crate::human_kill_reward_tests::vehicle_grenade_items_with_cover(100.);
    let state = &mut engine.domination_gameplay.as_mut().unwrap().state;
    state.filter_rows[6] = 1 << 22;
    state.filter_rows[24] = (1 << 22) | (1 << 9) | 1;
    if enabled {
        state.frag_ground_areas = Some(Arc::new(
            crate::acquisition_frag_ground::Catalog::synthetic_domination(110, 111),
        ));
    }
    let mut areas = vec![json!({"armor_area":0,"multiplier":1.}); 16];
    areas[6] = json!({"armor_area":2,"multiplier":0.5});
    let profiles = [(110, 0), (111, 1)].map(|(token, faction)| {
        json!({
            "vehicle_type_token":token,"faction":faction,"max_health":150.,
            "critical_loss_percent_per_second":0.,
            "armor":{"rating":10.,"prevention":0.5,"coverage":[0.,0.,0.5,0.]}
        })
    });
    state.ground_damage = Arc::new(
        serde_json::from_value(json!({
            "friendly_fire_multiplier":0.5,
            "profiles":profiles,
            "areas":areas,
        }))
        .unwrap(),
    );

    let ground = engine.ground_runtime.as_mut().unwrap();
    let key = ground.get(target).unwrap().key;
    ground
        .force_pose(
            key,
            ground::Pose {
                position: TARGET,
                rotation: [0., 1., 0., 0.],
            },
        )
        .unwrap();
    let human = crate::human_kill_reward_tests::fixture();
    let equipped = human.active.as_ref().unwrap();
    let mut kit = equipped.spawned_loadout.clone().unwrap();
    kit.inventories[0].slots[4] = Some(4);
    let actor = engine.active.as_mut().unwrap();
    actor.current_inventory_handle = equipped.current_inventory_handle;
    actor.spawned_loadout = Some(kit);
    actor.grenade_remaining = 1;
    actor.tracking.server_position(
        CharacterPosition { x: 20., ..TARGET },
        player_tracking::PositionOrigin::ServerSpawn,
        900,
    );
    let launch = crate::hospital_rocket_runtime::tests::launch_wire(&engine, 3, 4);
    assert!(!dispatch(&mut engine, &launch, 1000).unwrap().is_empty());
    assert!(engine.active.as_ref().unwrap().projectiles.get(3).is_some());
    engine
}

fn scene(engine: &Engine<Vec<u8>>, z: f32, material: Option<u8>) -> collision::CollisionWorld {
    collision::CollisionWorld::from_bytes(
        &serde_json::to_vec(&json!({
            "format":"mag_collision_world","schema_version":1,"coordinate_space":"world",
            "metadata":{"map_name":engine.map_name(),"build":"synthetic",
                "coverage":"provisional","provenance":["synthetic"],"unknowns":[]},
            "instances":[{"id":"frag-wall","vertices_xyz":[[-3.,-3.,z],[7.,-3.,z],[2.,7.,z]],
                "triangles":[[0,1,2]],"material_index":material,
                "collision":{"filter_info":9,"ray_child_filter_info":0,"convex_radius":0.,
                    "provenance":"synthetic","child_filter_policy":"uniform"}}]
        }))
        .unwrap(),
        Default::default(),
    )
    .unwrap()
}

fn owner_terminal(engine: &Engine<Vec<u8>>, sequence: u32) -> Vec<u8> {
    projectile_replication::ProjectileOwnerDelta {
        header: replication::UpdateHeader {
            sequence,
            nak_sequence: 0,
            client_index: 0,
            object_type_mask: 1 << 19,
            timestamp_ms: 1100,
        },
        update: &projectile::ProjectileOwnerUpdate {
            slot: 3,
            object_index: engine.active.as_ref().unwrap().reservation.player,
            fields: ProjectileOwnerFields {
                state: Some(2),
                position: Some(ORIGIN),
                rotation_xyzw: None,
            },
        },
    }
    .encode_with_profile(engine.wire_profile)
    .unwrap()
}

fn damage_wire(engine: &Engine<Vec<u8>>, index: u16) -> replication::VehicleDamage {
    engine
        .ground_runtime
        .as_ref()
        .unwrap()
        .get(index)
        .unwrap()
        .damage_wire()
}

fn event(engine: &Engine<Vec<u8>>) -> Value {
    engine
        .writer
        .split(|&byte| byte == b'\n')
        .rev()
        .filter_map(|row| serde_json::from_slice::<Value>(row).ok())
        .find(|row| row["op"] == "event")
        .unwrap()
}

#[test]
fn domination_frag_ground_apc_and_motor_pool_use_current_native_area() {
    for (index, source_team, expected) in [(APC, 1, 502), (MOTOR_POOL, 1, 558)] {
        let mut engine = fixture(index, source_team, true);
        let wire = owner_terminal(&engine, 200);
        dispatch(&mut engine, &wire, 1100).unwrap();
        let hit = damage_wire(&engine, index);
        // 100 × .75 falloff, optional .5 FF, subtract 10, then × .5 area × .75 coverage.
        assert_eq!(hit.health_quarter_points, expected);
        assert_eq!(hit.last_location_and_type, 0xa6);
        assert_eq!(hit.last_attacker, 0x0380_0000);
        assert_eq!(hit.last_user_data, 128 | 512);
        assert_eq!(hit.last_source_object, u32::MAX);
        let after = engine.ground_runtime.clone();
        dispatch(&mut engine, &wire, 1101).unwrap();
        assert!(engine.ground_runtime == after);
    }
}

#[test]
fn domination_frag_ground_absent_option_keeps_legacy_vehicle_damage() {
    let mut engine = fixture(APC, 1, false);
    let wire = owner_terminal(&engine, 200);
    dispatch(&mut engine, &wire, 1100).unwrap();
    let hit = damage_wire(&engine, APC);
    assert_eq!(hit.health_quarter_points, 340);
    assert_eq!(hit.last_location_and_type, 0xa0);
    assert_eq!(hit.last_source_object, 0x1380_0000);
}

#[test]
fn domination_frag_ground_reads_the_accepted_same_event_driver_pose() {
    let mut engine = fixture_with_drivers(APC, 0, true, true);
    let moved = CharacterPosition {
        z: TARGET.z - 1.,
        ..TARGET
    };
    let actor = engine.active.as_mut().unwrap();
    actor.ground_seat = Some(ground_seats::PassengerSeat {
        vehicle_index: APC,
        seat: 0,
    });
    let player = actor.reservation.player;
    engine
        .ground_runtime
        .as_mut()
        .unwrap()
        .begin_driver(APC, 1000)
        .unwrap();

    let mut bits = replication::BitWriter::with_profile(engine.wire_profile);
    replication::UpdateHeader {
        sequence: 200,
        nak_sequence: 0,
        client_index: 0,
        object_type_mask: (1 << 1) | (1 << 19),
        timestamp_ms: 1100,
    }
    .encode(&mut bits)
    .unwrap();
    bits.write(1, 1).unwrap();
    bits.write(APC.into(), 16).unwrap();
    bits.write(0, 25).unwrap();
    bits.write(1, 1).unwrap(); // Ground field25 position.
    moved.encode(&mut bits).unwrap();
    bits.write(0, 5).unwrap();
    bits.write(1, 1).unwrap();
    bits.write(player.into(), 16).unwrap();
    ProjectileOwnerFields {
        state: Some(2),
        position: Some(ORIGIN),
        rotation_xyzw: None,
    }
    .encode_fields(&mut bits)
    .unwrap();
    bits.write(0, 1).unwrap();
    dispatch(&mut engine, &bits.finish(), 1100).unwrap();
    assert_eq!(engine.current_ground_vehicle(APC).unwrap().position, moved);
    // Three metres to the new origin yields .625 falloff instead of .75.
    assert_eq!(
        damage_wire(&engine, APC).health_quarter_points,
        568,
        "{}",
        event(&engine)
    );
}

#[test]
fn domination_frag_ground_material_cover_and_unavailable_source_keep_terminal() {
    for (static_world, error) in [
        (true, None),
        (false, Some("frag_ground_requires_static_world")),
    ] {
        let mut engine = fixture(APC, 1, true);
        engine.collision_world = static_world.then(|| scene(&engine, TARGET.z + 1., Some(255)));
        let wire = owner_terminal(&engine, 200);
        dispatch(&mut engine, &wire, 1100).unwrap();
        let audit = event(&engine);
        let actor = engine.active.as_ref().unwrap();
        assert_eq!(
            damage_wire(&engine, APC).health_quarter_points,
            600,
            "{audit}"
        );
        assert!(actor.projectiles.get(3).unwrap().completed);
        assert!(engine.projectile_cleanup_pending(actor, 3));
        assert_eq!(audit["gameplay_denial"].as_str(), error, "{audit}");
    }
}

#[test]
fn domination_frag_ground_rejects_stale_terminal_and_rolls_back_on_record_refusal() {
    let mut engine = fixture(APC, 1, true);
    let before = engine.ground_runtime.clone();
    engine.active.as_mut().unwrap().latest_owner_sequence = 200;
    let stale = owner_terminal(&engine, 200);
    dispatch(&mut engine, &stale, 1100).unwrap();
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
    let fresh = owner_terminal(&engine, 201);
    engine.max_log = engine.written;
    assert_eq!(
        dispatch(&mut engine, &fresh, 1101).unwrap_err(),
        "log_limit_reached"
    );
    let source = engine.active.as_ref().unwrap();
    assert!(!source.projectiles.get(3).unwrap().completed);
    assert!(!engine.projectile_cleanup_pending(source, 3));
    assert!(engine.ground_runtime == before);
}

#[test]
fn domination_frag_ground_preserves_existing_helo_frag_damage() {
    let helo_health = |enabled| {
        let mut engine = fixture(APC, 1, enabled);
        add_helo(&mut engine);
        let wire = owner_terminal(&engine, 200);
        dispatch(&mut engine, &wire, 1100).unwrap();
        let state = &engine.domination_gameplay.as_ref().unwrap().state;
        state.helo.as_ref().unwrap().carriers[0]
            .damage
            .wire
            .health_quarter_points
    };
    let legacy = helo_health(false);
    assert!(legacy < 400, "fixture must hit the operational Helo");
    assert_eq!(helo_health(true), legacy);
}

fn add_helo(engine: &mut Engine<Vec<u8>>) {
    crate::domination_structural_world::hospital_rocket_helo_damage::tests::install_helo_fixture(
        engine,
        CharacterPosition {
            x: ORIGIN.x + 2.,
            ..ORIGIN
        },
    );
}

#[test]
fn domination_frag_ground_keeps_terminal_when_helo_also_lacks_scene() {
    let mut engine = fixture(APC, 1, true);
    add_helo(&mut engine);
    engine.collision_world = None;
    let wire = owner_terminal(&engine, 200);
    dispatch(&mut engine, &wire, 1100).unwrap();
    let source = engine.active.as_ref().unwrap();
    assert!(source.projectiles.get(3).unwrap().completed);
    assert!(engine.projectile_cleanup_pending(source, 3));
    assert_eq!(damage_wire(&engine, APC).health_quarter_points, 600);
    let audit = event(&engine);
    assert_eq!(
        audit["decoded"]["ground_grenade_checks"][0]["error"],
        "frag_ground_requires_static_world"
    );
    assert_eq!(
        audit["decoded"]["helo_grenade_checks"][0]["error"],
        "domination_structural_static_collision_required"
    );

    let mut legacy = fixture(APC, 1, false);
    add_helo(&mut legacy);
    legacy.collision_world = None;
    let wire = owner_terminal(&legacy, 200);
    assert_eq!(
        dispatch(&mut legacy, &wire, 1100).unwrap_err(),
        "domination_structural_static_collision_required"
    );
    assert!(
        !legacy
            .active
            .as_ref()
            .unwrap()
            .projectiles
            .get(3)
            .unwrap()
            .completed
    );
}
