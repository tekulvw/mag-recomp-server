//! Domination APC driver admission and authenticated owner flow using synthetic sources.
use super::*;

fn driver_setup(first_apc: u16) -> (Engine<Vec<u8>>, Value) {
    let (engine, mut value) = domination_apc_setup_at(first_apc);
    let apcs = first_apc..first_apc + 8;
    value["driver_seats"] = json!(
        apcs.clone()
            .map(|index| json!({
                "vehicle_index":index,"seat":0,"maximum_speed_mps":25.
            }))
            .collect::<Vec<_>>()
    );
    for (row, index) in apcs.enumerate() {
        value["vehicles"][usize::from(index)]["maximum_mount_distance_m"] = json!(8.);
        value["vehicles"][usize::from(index)]["maximum_mount_position_age_ms"] = json!(750);
        value["seat_visibility"]["vehicles"][row]["seats"]
            .as_array_mut()
            .unwrap()
            .insert(0, json!({"seat":0,"occupant_visible":false}));
    }
    (engine, value)
}

#[test]
fn domination_apc_driver_admission_uses_source_kind_in_each_layout() {
    for first_apc in [0, 8] {
        let (engine, source) = driver_setup(first_apc);
        let spawns = engine.spawn_catalog.as_ref().unwrap();
        let load = |value: &Value| Catalog::from_json(&value.to_string(), spawns);
        let catalog = load(&source).unwrap();
        let first_motor_pool = 8 - first_apc;
        assert!(catalog.driver_seat(first_motor_pool).is_none());
        for index in first_apc..first_apc + 8 {
            assert_eq!(catalog.driver_seat(index).unwrap().seat, 0);
            assert_eq!(catalog.seat_occupant_visible(index, 0), Some(false));
        }

        let mut partial = source.clone();
        partial["driver_seats"].as_array_mut().unwrap().pop();
        assert!(load(&partial).is_err());

        let mut foreign = source.clone();
        foreign["driver_seats"][0]["vehicle_index"] = json!(first_motor_pool);
        for (field, policy) in [
            ("maximum_mount_distance_m", json!(8.)),
            ("maximum_mount_position_age_ms", json!(750)),
            (
                "passenger_exit_candidates",
                source["vehicles"][usize::from(first_apc)]["passenger_exit_candidates"].clone(),
            ),
        ] {
            foreign["vehicles"][usize::from(first_motor_pool)][field] = policy;
        }
        // Isolate source-kind rejection independently of visibility coverage.
        foreign.as_object_mut().unwrap().remove("seat_visibility");
        assert_eq!(
            load(&foreign).unwrap_err().to_string(),
            "invalid_ground_driver_policy"
        );

        let mut gunner = source.clone();
        gunner["driver_seats"][0]["seat"] = json!(1);
        gunner.as_object_mut().unwrap().remove("seat_visibility");
        assert_eq!(
            load(&gunner).unwrap_err().to_string(),
            "invalid_ground_driver_policy"
        );

        let mut foreign_passenger = source.clone();
        let motor_pool = &mut foreign_passenger["vehicles"][usize::from(first_motor_pool)];
        motor_pool["passenger_seats"] = json!([2]);
        motor_pool["maximum_occupancy"] = json!(1);
        foreign_passenger
            .as_object_mut()
            .unwrap()
            .remove("seat_visibility");
        assert!(load(&foreign_passenger).is_err());

        let mut wrong_passenger = source.clone();
        wrong_passenger["vehicles"][usize::from(first_apc)]["passenger_seats"][0] = json!(1);
        wrong_passenger
            .as_object_mut()
            .unwrap()
            .remove("seat_visibility");
        assert!(load(&wrong_passenger).is_err());

        let mut wrong_capacity = source.clone();
        wrong_capacity["vehicles"][usize::from(first_apc)]["maximum_occupancy"] = json!(7);
        assert!(load(&wrong_capacity).is_err());

        let mut missing_policy = source.clone();
        let apc = missing_policy["vehicles"][usize::from(first_apc)]
            .as_object_mut()
            .unwrap();
        apc.remove("maximum_mount_distance_m");
        apc.remove("maximum_mount_position_age_ms");
        assert_eq!(
            load(&missing_policy).unwrap_err().to_string(),
            "invalid_ground_driver_policy"
        );

        let mut missing_visibility = source.clone();
        missing_visibility["seat_visibility"]["vehicles"][0]["seats"]
            .as_array_mut()
            .unwrap()
            .remove(0);
        assert_eq!(
            load(&missing_visibility).unwrap_err().to_string(),
            "invalid_ground_seat_visibility_inventory"
        );

        let (engine, legacy) = domination_apc_setup_at(first_apc);
        let catalog =
            Catalog::from_json(&legacy.to_string(), engine.spawn_catalog.as_ref().unwrap())
                .unwrap();
        assert!(catalog.driver_seat(first_apc).is_none());
    }
}

pub(crate) fn driver_engine(first_apc: u16) -> Engine<Vec<u8>> {
    let (engine, source) = driver_setup(first_apc);
    let catalog =
        Catalog::from_json(&source.to_string(), engine.spawn_catalog.as_ref().unwrap()).unwrap();
    let floor = ground_driver_test_floor(&engine);
    engine
        .with_ground_vehicles(catalog)
        .unwrap()
        .with_collision_world(floor)
}

pub(crate) fn ready_on_foot(engine: &mut Engine<Vec<u8>>, player: u16, now: u64) {
    reserve_faction(engine, player, 1);
    let actor = engine.active.as_mut().unwrap();
    ready(actor);
    actor.life = CharacterLife::Alive {
        generation: 1,
        health_quarter_points: 400,
    };
    owner_update(engine, now, 0);
}

fn ownership_field(wire: &[u8], profile: wire_profile::WireProfile, index: u16) -> u32 {
    let mut bits = BitReader::with_profile(wire, profile);
    bits.position = profile.header_bits();
    assert_eq!(bits.read(1), Ok(1));
    assert_eq!(bits.read(16), Ok(index.into()));
    if bits.read(1) == Ok(1) {
        assert_eq!(bits.read(1), Ok(1)); // Active field in a creation only.
    }
    assert_eq!(bits.read(1), Ok(1));
    bits.read(16).unwrap()
}

fn wire_packets(
    frames: &[OutboundMessage],
    profile: wire_profile::WireProfile,
    mask: u32,
) -> Vec<Vec<u8>> {
    frames
        .iter()
        .filter_map(|frame| {
            let wire = protocol::decode_hex(&frame.payload_hex);
            let update = ObjectUpdate::decode_with_profile(&wire, profile).ok()?;
            (update.header.object_type_mask == mask).then_some(wire)
        })
        .collect()
}

#[test]
fn domination_apc_driver_moves_with_passengers_and_uses_current_exit_geometry() {
    let mut engine = driver_engine(0);
    let profile = engine.wire_profile;
    let vehicle = 0x0100_0000;
    ready_on_foot(&mut engine, 0, 10);
    assert!(ground_seat_event(&mut engine, true, vehicle, 0, 0, 751).is_empty());
    let actor = engine.active.as_mut().unwrap();
    actor.tracking.server_position(
        CharacterPosition {
            x: 100.,
            y: 3.,
            z: 100.,
        },
        player_tracking::PositionOrigin::ServerSpawn,
        760,
    );
    assert!(ground_seat_event(&mut engine, true, vehicle, 1, 0, 761).is_empty());
    assert!(ground_seat_event(&mut engine, true, vehicle | 8, 0, 0, 762).is_empty());
    assert!(ground_seat_event(&mut engine, true, vehicle, 0, 1, 763).is_empty());
    let grant = ground_seat_event(&mut engine, true, vehicle, 0, 0, 770);
    let grant = wire_packets(&grant, profile, (1 << 1) | (1 << 3));
    assert_eq!(ownership_field(&grant[0], profile, 0), 0);
    assert_eq!(engine.ground_driver_owner(0), Some(0));

    spawn_apc(&mut engine, 1, 1, 0, 780);
    // This Ground-only fixture has no objective snapshot to send after spawn.
    let passenger = engine.active.as_mut().unwrap();
    passenger.objective_delta = Some(Vec::new());
    passenger.objective_assigned = true;
    assert_eq!(engine.active.as_ref().unwrap().ground_seat.unwrap().seat, 9);
    assert!(ground_seat_event(&mut engine, true, vehicle, 0, 1, 800).is_empty());
    let report = |sequence, x| ground_driver_relay(profile, 0, sequence, x);
    dispatch(&mut engine, &report(20, 3232), 840).unwrap();
    assert_eq!(engine.current_ground_vehicle(0).unwrap().position.x, 100.);

    focus(&mut engine, 0);
    dispatch(&mut engine, &report(40, 3232), 870).unwrap();
    assert_eq!(engine.current_ground_vehicle(0).unwrap().position.x, 101.);
    let current = engine.ground_runtime.as_ref().unwrap().get(0).unwrap();
    assert_eq!(current.driver_controls.brake, Some(true));
    let passenger = &engine.peers["synthetic-reservation-0001"];
    assert!(passenger.pending_shared.iter().any(|wire| {
        ObjectUpdate::decode_with_profile(wire, profile)
            .ok()
            .and_then(|update| ground_vehicles::OwnerDynamics::decode(&update, profile).ok())
            .is_some_and(|reports| {
                reports.iter().any(|report| {
                    report.index == 0
                        && report.position.is_some_and(|position| position.x == 101.)
                        && report.brake == Some(true)
                })
            })
    }));
    dispatch(&mut engine, &report(39, 3248), 880).unwrap();
    assert_eq!(engine.current_ground_vehicle(0).unwrap().position.x, 101.);

    reserve_faction(&mut engine, 2, 1);
    ready(engine.active.as_mut().unwrap());
    let late = wire_packets(&owner_update(&mut engine, 890, 0), profile, 1 << 1);
    assert_eq!(ownership_field(&late[0], profile, 0), 0);
    // The first creation row includes the current transform and brake control.
    let mut bits = BitReader::with_profile(&late[0], profile);
    bits.position = profile.header_bits();
    assert_eq!(bits.read(1), Ok(1));
    assert_eq!(bits.read(16), Ok(0));
    for field in 0..30 {
        if bits.read(1).unwrap() == 0 {
            assert!(!matches!(field, 25 | 27));
            continue;
        }
        match field {
            0 => {
                bits.read(1).unwrap();
            }
            1 | 6 | 7 | 20 => {
                bits.read(16).unwrap();
            }
            3 => {
                bits.read(32).unwrap();
            }
            4 | 5 | 19 => {
                bits.read(8).unwrap();
            }
            18 => {
                for width in [8, 16, 32, 32, 32] {
                    bits.read(width).unwrap();
                }
            }
            25 => {
                assert_eq!(bits.read(16), Ok(3232));
                bits.read(17).unwrap();
                bits.read(16).unwrap();
            }
            26 => {
                for _ in 0..4 {
                    bits.read(11).unwrap();
                }
            }
            27 => assert_eq!(bits.read(1), Ok(1)),
            _ => panic!("unexpected vehicle field {field}"),
        }
    }

    focus(&mut engine, 0);
    let runtime = engine.ground_runtime.as_mut().unwrap();
    let current = runtime.get(0).unwrap();
    let (key, pose) = (current.key, current.pose);
    let mut raised = pose;
    raised.position.y = 10.;
    runtime.force_pose(key, raised).unwrap();
    assert!(ground_seat_event(&mut engine, false, vehicle, 0, 0, 900).is_empty());
    assert_eq!(engine.ground_driver_owner(0), Some(0));
    assert_eq!(engine.active.as_ref().unwrap().ground_seat.unwrap().seat, 0);
    engine
        .ground_runtime
        .as_mut()
        .unwrap()
        .force_pose(key, pose)
        .unwrap();

    let exit = ground_seat_event(&mut engine, false, vehicle, 0, 0, 910);
    let revoke = wire_packets(&exit, profile, (1 << 1) | (1 << 3));
    assert_eq!(
        ownership_field(&revoke[0], profile, 0),
        u32::from(ground_vehicles::SERVER_OWNER)
    );
    assert_eq!(engine.ground_driver_owner(0), None);
    let exit_position = engine
        .active
        .as_ref()
        .unwrap()
        .tracking
        .sample
        .unwrap()
        .position;
    assert_eq!((exit_position.x, exit_position.y), (102., 2.09375));
}

#[test]
fn domination_apc_driver_switch_and_release_revoke_high_slot_without_ejecting_passenger() {
    let mut engine = driver_engine(8);
    let profile = engine.wire_profile;
    let vehicle = 0x0100_0008;
    ready_on_foot(&mut engine, 0, 10);
    assert!(!ground_seat_event(&mut engine, true, vehicle, 0, 0, 20).is_empty());
    assert_eq!(engine.ground_driver_owner(8), Some(0));
    let switched = ground_seat_event(&mut engine, true, vehicle, 2, 0, 25);
    let revoke = wire_packets(&switched, profile, (1 << 1) | (1 << 3));
    assert_eq!(
        ownership_field(&revoke[0], profile, 8),
        u32::from(ground_vehicles::SERVER_OWNER)
    );
    assert_eq!(engine.ground_driver_owner(8), None);
    assert_eq!(engine.active.as_ref().unwrap().ground_seat.unwrap().seat, 2);
    assert!(!ground_seat_event(&mut engine, true, vehicle, 0, 0, 30).is_empty());

    spawn_apc(&mut engine, 1, 1, 8, 40);
    let released = engine.handle(
        Request::Release {
            v: BRIDGE_VERSION,
            request_id: "synthetic-domination-driver-release".into(),
            reservation_id: "synthetic-reservation-0000".into(),
        },
        Duration::from_millis(70),
        1,
    );
    assert!(released.ok, "{:?}", released.error);
    assert_eq!(engine.ground_driver_owner(8), None);
    assert_eq!(
        engine.peers["synthetic-reservation-0001"].ground_seat,
        Some(ground_seats::PassengerSeat {
            vehicle_index: 8,
            seat: 9
        })
    );
    assert!(
        wire_packets(&released.messages, profile, 1 << 1)
            .iter()
            .any(|wire| {
                let mut bits = BitReader::with_profile(wire, profile);
                bits.position = profile.header_bits();
                bits.read(19) == Ok((1 << 18) | (8 << 2) | 1)
                    && bits.read(16) == Ok(u32::from(ground_vehicles::SERVER_OWNER))
            })
    );
}

#[test]
fn domination_apc_driver_dynamics_roll_back_on_record_failure() {
    let mut engine = driver_engine(8);
    let profile = engine.wire_profile;
    ready_on_foot(&mut engine, 0, 10);
    assert!(!ground_seat_event(&mut engine, true, 0x0100_0008, 0, 0, 20).is_empty());
    let before = engine.ground_runtime.clone();
    engine.max_log = engine.written;
    assert!(dispatch(&mut engine, &ground_driver_relay(profile, 8, 20, 3232), 120).is_err());
    assert_eq!(engine.ground_runtime, before);
    assert_eq!(engine.ground_driver_owner(8), Some(0));
}

#[test]
fn domination_apc_driver_is_revoked_by_vehicle_destruction() {
    use crate::{
        acquisition_ground_effects::GroundFrame,
        acquisition_ground_runtime::{Capabilities, Destruction},
        acquisition_vehicle_waves::VehicleWaves,
        domination_structural_world::{BlastCollision, SequenceCollisionState},
    };

    let mut engine = driver_engine(0);
    let profile = engine.wire_profile;
    let spawn_hash = engine
        .spawn_catalog
        .as_ref()
        .unwrap()
        .source_sha256()
        .unwrap();
    let rules = character_damage_rules::Rules::from_json(&json!({
        "schema_version":1,"application_id":20231,"map_name":"synthetic_arena","game_type":2,
        "friendly_fire_multiplier":0.5,"provenance":{"spawn_catalog_sha256":spawn_hash,"source":"synthetic"}
    }).to_string()).unwrap();
    engine = engine.with_character_damage_rules(rules).unwrap();
    engine.item_catalog = crate::human_kill_reward_tests::combat_items_for_ground_test(0.);
    ready_on_foot(&mut engine, 0, 10);
    let actor = engine.active.as_mut().unwrap();
    let mut loadout = actor
        .reservation
        .character
        .loadouts
        .as_ref()
        .unwrap()
        .loadout(&engine.item_catalog, 0)
        .unwrap();
    loadout.inventories[0].slots[12] = Some(3);
    actor.current_inventory_handle = Some(loadout.handle);
    actor.spawned_loadout = Some(loadout);
    assert!(!ground_seat_event(&mut engine, true, 0x0100_0000, 0, 0, 20).is_empty());

    let mut ground = engine.ground_runtime.clone().unwrap();
    let key = ground.get(0).unwrap().key;
    // Isolate the destruction callback and occupants from unrelated blast targets.
    ground
        .configure(
            key,
            Capabilities {
                destruction: Some(Destruction {
                    damage: 0.,
                    full_radius: 0.,
                    half_radius: 0.,
                }),
                ..Default::default()
            },
        )
        .unwrap();
    ground.apply_lethal(key, u32::MAX, true).unwrap();
    let now = Duration::from_millis(30);
    let mut waves = VehicleWaves::start(0, [100, 100]).unwrap();
    let mut changes = std::collections::BTreeMap::new();
    let mut native = engine.owned_acquisition_characters();
    engine
        .stage_ground_updates(
            GroundFrame {
                now,
                dt: 0.,
                waves: &mut waves,
                policy: None,
                filter_row: 0,
                limits: Default::default(),
                domination_blast: Some((
                    BlastCollision {
                        filter_rows: &[0; 32],
                        limits: Default::default(),
                        sequences: &SequenceCollisionState::default(),
                        material_costs: &Default::default(),
                        terrain_materials: None,
                    },
                    0.,
                )),
            },
            &mut ground,
            &mut native,
            &mut changes,
        )
        .unwrap();
    let wire = engine
        .encode_ground_changes(&ground, &changes, now)
        .unwrap()
        .unwrap();
    assert_eq!(
        ownership_field(&wire, profile, 0),
        u32::from(ground_vehicles::SERVER_OWNER)
    );
    assert_eq!(ground.get(0).unwrap().state, 5);
    engine.ground_runtime = Some(ground);
    assert_eq!(engine.ground_driver_owner(0), None);
    assert!(!engine.active.as_ref().unwrap().life.is_alive());
    assert!(engine.active.as_ref().unwrap().ground_seat.is_none());
    dispatch(&mut engine, &ground_driver_relay(profile, 0, 20, 3232), 120).unwrap();
    assert_eq!(engine.current_ground_vehicle(0).unwrap().position.x, 100.);
}

/// SYNTHETIC: APC 0 is the spawn point of squad 1 only; every other squad,
/// including squad 0 where the first human lands, uses APC 1.
fn squad_owned_apc_engine() -> Engine<Vec<u8>> {
    let mut engine = driver_engine(0);
    let mut spawn = domination_apc_spawn_json(0);
    spawn["active"]["default_spawn"] = json!(1);
    spawn["active"]["available"] = json!([1]);
    for squad in spawn["maps"][0]["squads"].as_array_mut().unwrap() {
        let owns_apc_zero = squad["index"] == 1;
        squad["default_spawn"] = json!(if owns_apc_zero { 0 } else { 1 });
        squad["available"] = json!([if owns_apc_zero { 0 } else { 1 }]);
    }
    engine.spawn_catalog = Some(spawns(spawn).unwrap());
    engine
}

fn on_foot_at_apcs(engine: &mut Engine<Vec<u8>>, now: u64) {
    engine.active.as_mut().unwrap().tracking.server_position(
        CharacterPosition {
            x: 100.,
            y: 3.,
            z: 100.,
        },
        player_tracking::PositionOrigin::ServerSpawn,
        now,
    );
}

fn last_denial(engine: &Engine<Vec<u8>>) -> Option<String> {
    let log = String::from_utf8(engine.writer.clone()).unwrap();
    log.lines()
        .rev()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|record| record["decoded"]["type"] == "mount_seat")
        .and_then(|record| record["gameplay_denial"].as_str().map(str::to_owned))
}

#[test]
fn a_squad_apc_seats_only_its_own_squad_while_its_seats_are_held() {
    let mut engine = squad_owned_apc_engine();
    ready_on_foot(&mut engine, 0, 10);
    assert_eq!(
        engine.active.as_ref().unwrap().squad_catalog.squad_index(),
        0
    );
    on_foot_at_apcs(&mut engine, 760);
    // Squad 0 does not own APC 0: its eight held seats fill the whole APC.
    assert!(ground_seat_event(&mut engine, true, 0x0100_0000, 2, 0, 770).is_empty());
    assert_eq!(
        last_denial(&engine).as_deref(),
        Some("ground_seats_reserved_for_other_squads")
    );
    assert_eq!(engine.active.as_ref().unwrap().ground_seat, None);
    // Its own APC 1 seats it.
    assert!(!ground_seat_event(&mut engine, true, 0x0100_0001, 2, 0, 780).is_empty());
    assert_eq!(
        engine
            .active
            .as_ref()
            .unwrap()
            .ground_seat
            .unwrap()
            .vehicle_index,
        1
    );
}

#[test]
fn a_member_of_the_owning_squad_can_take_a_squad_apc_seat() {
    let mut engine = squad_owned_apc_engine();
    ready_on_foot(&mut engine, 0, 10);
    let actor = engine.active.as_mut().unwrap();
    actor.squad_catalog = std::sync::Arc::new(actor.squad_catalog.for_squad(1).unwrap());
    on_foot_at_apcs(&mut engine, 760);
    assert!(!ground_seat_event(&mut engine, true, 0x0100_0000, 2, 0, 770).is_empty());
    assert_eq!(
        engine.active.as_ref().unwrap().ground_seat,
        Some(crate::ground_seats::PassengerSeat {
            vehicle_index: 0,
            seat: 2,
        })
    );
}

#[test]
fn an_apc_that_no_squad_spawns_at_seats_anyone_on_its_side() {
    let mut engine = squad_owned_apc_engine();
    ready_on_foot(&mut engine, 0, 10);
    on_foot_at_apcs(&mut engine, 760);
    // APC 2 is no squad's spawn point, so it holds no seats.
    assert!(!ground_seat_event(&mut engine, true, 0x0100_0002, 2, 0, 770).is_empty());
    assert_eq!(
        engine
            .active
            .as_ref()
            .unwrap()
            .ground_seat
            .unwrap()
            .vehicle_index,
        2
    );
}

#[path = "vehicle_collision_tests.rs"]
mod vehicle_collision;
