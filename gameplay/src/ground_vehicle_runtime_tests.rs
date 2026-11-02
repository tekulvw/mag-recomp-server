//! Synthetic source-bound catalog and real replication planner checks.
use super::*;
use crate::ground_vehicles::Catalog;

fn catalog(engine: &Engine<Vec<u8>>) -> Value {
    json!({"format":"mag_ground_vehicles","schema_version":1,
        "source":{"spawn_catalog_sha256":engine.spawn_catalog.as_ref().unwrap().source_sha256().unwrap(),
            "native_elf_sha256":"a".repeat(64),"native_binding_sha256":"b".repeat(64),
            "vehicle_properties_sha256":"c".repeat(64)},
        "vehicles":[{"index":0,"spawn_point_index":0,"spawn_point_token":1000,
            "vehicle_type_token":12345,"faction":1,"team":0,"modifier":0,
            "position":{"x":100.,"y":3.,"z":100.},"rotation":[0.,0.,0.,1.],
            "health_quarter_points":600}]})
}
fn configured() -> Engine<Vec<u8>> {
    let engine = acquisition_entry::acquisition()
        .with_match_limits(1, 64)
        .unwrap();
    let vehicles = Catalog::from_json(
        &catalog(&engine).to_string(),
        engine.spawn_catalog.as_ref().unwrap(),
    )
    .unwrap();
    engine.with_ground_vehicles(vehicles).unwrap()
}

#[test]
fn ground_vehicle_catalog_requires_exact_native_spawn_binding_and_match_side() {
    let engine = acquisition_entry::acquisition();
    let spawns = engine.spawn_catalog.as_ref().unwrap();
    let value = catalog(&engine);
    assert!(Catalog::from_json(&value.to_string(), spawns).is_ok());
    for (field, bad) in [
        ("spawn_point_token", json!(9999)),
        ("faction", json!(0)),
        ("spawn_point_index", json!(99)),
        ("team", json!(1)),
    ] {
        let mut bad_value = value.clone();
        bad_value["vehicles"][0][field] = bad;
        assert!(
            Catalog::from_json(&bad_value.to_string(), spawns).is_err(),
            "{field}"
        );
    }
    let mut bad = value.clone();
    bad["source"]["spawn_catalog_sha256"] = json!("0".repeat(64));
    assert!(Catalog::from_json(&bad.to_string(), spawns).is_err());
    let mut bad = value.clone();
    bad["vehicles"]
        .as_array_mut()
        .unwrap()
        .push(value["vehicles"][0].clone());
    assert!(Catalog::from_json(&bad.to_string(), spawns).is_err());
    let mut unverified = acquisition_entry::acquisition();
    let mut source = spawn_json();
    source["maps"][0]["runtime_indices_verified"] = json!(false);
    source["maps"][0]["forward_spawns"] = json!([]);
    unverified.spawn_catalog = Some(spawns_from_value(source));
    assert!(
        Catalog::from_json(
            &catalog(&unverified).to_string(),
            unverified.spawn_catalog.as_ref().unwrap()
        )
        .is_err()
    );
}
fn spawns_from_value(value: Value) -> SpawnCatalog {
    spawns(value).unwrap()
}

#[test]
fn ground_vehicle_creation_replays_exactly_and_late_join_gets_its_own_snapshot() {
    let mut engine = configured();
    reserve(&mut engine, 0);
    ready(engine.active.as_mut().unwrap());
    let frames = owner_update(&mut engine, 10, 0);
    let packet = protocol::decode_hex(&frames[0].payload_hex);
    let header = ObjectUpdate::decode_with_profile(&packet, engine.wire_profile)
        .unwrap()
        .header;
    assert_eq!(header.object_type_mask, 1 << 1);
    assert_eq!(engine.active.as_ref().unwrap().ground_vehicle_batch, 1);
    let replay = owner_update(&mut engine, 20, header.sequence);
    assert_eq!(protocol::decode_hex(&replay[0].payload_hex), packet);
    assert_eq!(engine.active.as_ref().unwrap().ground_vehicle_batch, 1);
    reserve(&mut engine, 1);
    ready(engine.active.as_mut().unwrap());
    let frames = owner_update(&mut engine, 30, 0);
    let packet = protocol::decode_hex(&frames[0].payload_hex);
    assert_eq!(
        ObjectUpdate::decode_with_profile(&packet, engine.wire_profile)
            .unwrap()
            .header
            .object_type_mask,
        1 << 1
    );
    assert_eq!(engine.active.as_ref().unwrap().ground_vehicle_batch, 1);
}

#[test]
fn ground_vehicle_creation_does_not_commit_when_evidence_recording_fails() {
    let mut engine = configured();
    reserve(&mut engine, 0);
    ready(engine.active.as_mut().unwrap());
    engine.max_log = engine.written;
    let header = UpdateHeader {
        sequence: 1,
        nak_sequence: 0,
        client_index: 0,
        object_type_mask: 0,
        timestamp_ms: 10,
    };
    let packet = ObjectUpdate { header, wire: &[] }
        .encode_empty_with_profile(engine.wire_profile)
        .unwrap();
    assert!(dispatch(&mut engine, &packet, 10).is_err());
    assert_eq!(engine.active.as_ref().unwrap().ground_vehicle_batch, 0);
    assert!(
        engine
            .active
            .as_ref()
            .unwrap()
            .replication_history
            .is_empty()
    );
}

#[test]
fn apc_passenger_catalog_accepts_explicit_spawn_binding_and_exit_offsets() {
    let mut engine = acquisition_entry::acquisition();
    let mut source = apc_spawn_json(1);
    source["maps"][0]["locations"][1]["kind"] = json!("on_foot");
    engine.spawn_catalog = Some(spawns(source).unwrap());
    let mut value = catalog(&engine);
    value["vehicles"][0]["passenger_seats"] = json!([2, 3, 4, 5, 6, 7, 8, 9]);
    value["vehicles"][0]["maximum_occupancy"] = json!(8);
    value["passenger_spawns"] =
        json!([{ "spawn_point_index":0, "spawn_point_token":1000, "vehicle_index":0 }]);
    value["vehicles"][0]["passenger_exit_candidates"] =
        json!([{ "ranking_offset":[1.,0.,0.], "position_offset":[3.,0.,0.] }]);
    assert!(Catalog::from_json(&value.to_string(), engine.spawn_catalog.as_ref().unwrap()).is_ok());
}

fn apc_spawn_json(faction: u8) -> Value {
    let mut value = spawn_json();
    value["active"]["available"] = json!([0, 1]);
    value["match_profile"]["game_type"] = json!(1);
    value["match_profile"]["team_capacity"] = json!(32);
    value["match_profile"]["squad_capacity"] = json!(8);
    value["match_profile"]["attacking_faction"] = json!(faction);
    value["maps"][0]["owner_faction"] = json!((faction + 1) % 3);
    value["match_profile"]["has_objectives"] = json!(false);
    value["maps"][0]["forward_spawns"] = json!([]);
    value["maps"][0]["locations"] = json!([
        {"index":0,"token":1000,"kind":"vehicle_passenger","position":{"x":100.,"y":3.,"z":100.}},
        {"index":1,"token":1001,"kind":"vehicle_passenger","position":{"x":120.,"y":3.,"z":100.}}
    ]);
    value["maps"][0]["spawn_count"] = json!(2);
    for squad in value["maps"][0]["squads"].as_array_mut().unwrap() {
        squad["available"] = json!([0, 1]);
        squad["default_spawn"] = json!(0);
        if squad["team"] == 1 {
            squad["faction"] = json!((faction + 1) % 3);
        }
    }
    value
}
fn apc_engine(faction: u8, manual_mount: bool) -> Engine<Vec<u8>> {
    let mut engine = acquisition_entry::acquisition()
        .with_match_limits(1, 64)
        .unwrap();
    engine.spawn_catalog = Some(spawns(apc_spawn_json(faction)).unwrap());
    let mut value = catalog(&engine);
    value["vehicles"][0]["faction"] = json!(faction);
    value["vehicles"][0]["passenger_seats"] = json!([2, 3, 4, 5, 6, 7, 8, 9]);
    value["vehicles"][0]["maximum_occupancy"] = json!(8);
    if manual_mount {
        value["vehicles"][0]["maximum_mount_distance_m"] = json!(10.);
        value["vehicles"][0]["maximum_mount_position_age_ms"] = json!(100);
    }
    {
        value["vehicles"][0]["passenger_exit_candidates"] = json!([{"ranking_offset":[1.,0.,0.],"position_offset":[3.,0.,0.]},{"ranking_offset":[-1.,0.,0.],"position_offset":[-3.,0.,0.]}]);
    }
    let mut second = value["vehicles"][0].clone();
    second["index"] = json!(1);
    second["spawn_point_index"] = json!(1);
    second["spawn_point_token"] = json!(1001);
    value["vehicles"].as_array_mut().unwrap().push(second);
    value["passenger_spawns"] = json!([
        {"spawn_point_index":0,"spawn_point_token":1000,"vehicle_index":0},
        {"spawn_point_index":1,"spawn_point_token":1001,"vehicle_index":1}
    ]);
    let world =
        Catalog::from_json(&value.to_string(), engine.spawn_catalog.as_ref().unwrap()).unwrap();
    engine.with_ground_vehicles(world).unwrap()
}
fn spawn_apc(
    engine: &mut Engine<Vec<u8>>,
    player: u16,
    faction: u8,
    point: u16,
    now: u64,
) -> Vec<OutboundMessage> {
    reserve_faction(engine, player, faction);
    ready(engine.active.as_mut().unwrap());
    engine.active.as_mut().unwrap().claimed = Some(Claimed {
        wire: Vec::new(),
        np_identity: [player as u8; 36],
        game_info: protocol::GameInfo {
            client_index: player,
            level_name: "synthetic_arena".into(),
            start_time_ms: 0,
            attacking_faction: faction,
            defending_faction: (faction + 1) % 3,
            maximum_points: [100, 100],
            current_points: [100, 100],
            game_type: 1,
        },
    });
    engine.active.as_mut().unwrap().pending_shared.clear();
    owner_update(engine, now, 0); // vehicle creations precede mounted spawn
    engine.active.as_mut().unwrap().selected_spawn = Some(protocol::CharacterSpawnSelection {
        spawn_point_index: point,
    });
    engine.active.as_mut().unwrap().pending_spawn_deadline = Some(Duration::from_millis(now));
    owner_update(engine, now + 1, 0)
}
#[test]
fn apc_insertion_all_factions_emits_ground_state_and_replays_exactly() {
    for faction in 0..3 {
        let mut engine = apc_engine(faction, true);
        let frames = spawn_apc(&mut engine, 0, faction, 0, 100);
        let active = engine.active.as_ref().unwrap();
        assert_eq!(
            active.ground_seat,
            Some(ground_seats::PassengerSeat {
                vehicle_index: 0,
                seat: 9
            })
        );
        assert_eq!(active.mounted_seat, None);
        assert_eq!(active.owner_fields.state.unwrap().user_data, [1, 0, 9]);
        assert!(active.life.is_alive());
        assert_eq!(active.life.damage_health().unwrap().0, 1);
        assert_eq!(active.team(), 0);
        assert_eq!(active.reservation.character.faction, faction);
        assert_eq!(active.selected_spawn.unwrap().spawn_point_index, 0);
        assert!(active.pending_spawn_deadline.is_none());
        assert_eq!(active.current_inventory_handle.unwrap().inventory_index, 0);
        assert_eq!(
            active.spawned_loadout.as_ref().unwrap().primary.ammo_count,
            7
        );
        let packet = active.spawn_delta.clone().unwrap();
        assert!(
            frames
                .iter()
                .any(|f| protocol::decode_hex(&f.payload_hex) == packet)
        );
        let sequence = ObjectUpdate::decode_with_profile(&packet, engine.wire_profile)
            .unwrap()
            .header
            .sequence;
        assert_eq!(
            protocol::decode_hex(&owner_update(&mut engine, 102, sequence)[0].payload_hex),
            packet
        );
        let mut bits = BitReader::with_profile(&packet, engine.wire_profile);
        bits.position = engine.wire_profile.header_bits();
        assert_eq!(bits.read(1), Ok(1));
        assert_eq!(bits.read(16), Ok(0));
        assert_eq!(bits.read(4), Ok(0));
        assert_eq!(bits.read(1), Ok(1));
        assert_eq!(bits.read(8), Ok(14));
        for expected in [1, 0, 9] {
            assert_eq!(bits.read(8), Ok(expected));
        }
        let snapshot = engine
            .active
            .as_ref()
            .unwrap()
            .snapshot(Duration::from_millis(103))
            .unwrap();
        let mut bits = BitReader::with_profile(&snapshot, engine.wire_profile);
        bits.position = engine.wire_profile.header_bits();
        assert_eq!(bits.read(1), Ok(1));
        assert_eq!(bits.read(16), Ok(0));
        assert_eq!(bits.read(1), Ok(1));
        assert_eq!(bits.read(1), Ok(1)); // active
        assert_eq!(bits.read(1), Ok(1));
        assert_eq!(bits.read(16), Ok(0)); // owner
        assert_eq!(bits.read(1), Ok(0));
        assert_eq!(bits.read(1), Ok(1)); // info
        bits.read(17).unwrap();
        let name_length = bits.read(16).unwrap();
        for _ in 0..name_length {
            bits.read(8).unwrap();
        }
        for width in [16, 16, 32] {
            bits.read(width).unwrap();
        }
        for _ in 0..36 {
            bits.read(8).unwrap();
        }
        assert_eq!(bits.read(1), Ok(1));
        assert_eq!(bits.read(8), Ok(14));
        for expected in [1, 0, 9] {
            assert_eq!(bits.read(8), Ok(expected));
        }
    }
}
#[test]
fn apc_seats_descend_have_independent_capacity_and_require_exit_policy() {
    let mut engine = apc_engine(1, true);
    for player in 0..8 {
        spawn_apc(&mut engine, player, 1, 0, 100 + u64::from(player) * 10);
        assert_eq!(
            engine.active.as_ref().unwrap().ground_seat.unwrap().seat,
            9 - player as u8
        );
    }
    spawn_apc(&mut engine, 8, 1, 0, 200);
    assert!(engine.active.as_ref().unwrap().ground_seat.is_none());
    assert!(
        engine
            .active
            .as_ref()
            .unwrap()
            .pending_spawn_deadline
            .is_some()
    );
    spawn_apc(&mut engine, 9, 1, 1, 220);
    assert_eq!(
        engine.active.as_ref().unwrap().ground_seat,
        Some(ground_seats::PassengerSeat {
            vehicle_index: 1,
            seat: 9
        })
    );
    let engine = apc_engine(1, true);
    let mut value = catalog(&engine);
    value["vehicles"][0]["passenger_seats"] = json!([2, 3, 4, 5, 6, 7, 8, 9]);
    value["vehicles"][0]["maximum_occupancy"] = json!(8);
    value["passenger_spawns"] =
        json!([{"spawn_point_index":0,"spawn_point_token":1000,"vehicle_index":0}]);
    assert!(
        Catalog::from_json(&value.to_string(), engine.spawn_catalog.as_ref().unwrap()).is_err()
    );
}

fn ground_seat_event(
    engine: &mut Engine<Vec<u8>>,
    mount: bool,
    vehicle: u32,
    seat: u32,
    player: u16,
    now: u64,
) -> Vec<OutboundMessage> {
    let mut wire = if mount { 0x0216u16 } else { 0x0217u16 }
        .to_be_bytes()
        .to_vec();
    wire.extend(vehicle.to_be_bytes());
    wire.extend(seat.to_be_bytes());
    wire.extend((0x03800000 | u32::from(player)).to_be_bytes());
    if !mount {
        wire.extend(90u32.to_be_bytes());
    }
    dispatch(engine, &wire, now).unwrap()
}
#[test]
fn apc_mount_rejects_repeated_occupied_wrong_side_and_invalid_seats() {
    let mut engine = apc_engine(1, true);
    spawn_apc(&mut engine, 0, 1, 0, 100);
    assert!(ground_seat_event(&mut engine, true, 0x01000000, 9, 0, 103).is_empty());
    ground_seat_event(&mut engine, false, 0x01000000, 9, 0, 104);
    assert!(!ground_seat_event(&mut engine, true, 0x01000000, 8, 0, 105).is_empty());
    assert!(ground_seat_event(&mut engine, true, 0x01000000, 8, 0, 106).is_empty());
    spawn_apc(&mut engine, 1, 1, 1, 110);
    ground_seat_event(&mut engine, false, 0x01000001, 9, 1, 112);
    assert!(ground_seat_event(&mut engine, true, 0x01000000, 8, 1, 113).is_empty());
    assert!(ground_seat_event(&mut engine, true, 0x01000000, 1, 1, 114).is_empty());
    assert!(ground_seat_event(&mut engine, true, 0x01000000, 256, 1, 115).is_empty());
    // A defensive actor cannot take the attacker's vehicle, even after an on-foot exit.
    reserve_faction(&mut engine, 2, 2);
    ready(engine.active.as_mut().unwrap());
    engine.active.as_mut().unwrap().life = CharacterLife::Alive {
        generation: 1,
        health_quarter_points: 400,
    };
    owner_update(&mut engine, 120, 0);
    assert!(ground_seat_event(&mut engine, true, 0x01000000, 7, 2, 121).is_empty());
}
#[test]
fn apc_exit_is_vehicle_relative_replays_and_releases_occupancy() {
    let mut engine = apc_engine(1, true);
    spawn_apc(&mut engine, 0, 1, 0, 100);
    let frames = ground_seat_event(&mut engine, false, 0x01000000, 9, 0, 103);
    let packet = protocol::decode_hex(&frames[0].payload_hex);
    let active = engine.active.as_ref().unwrap();
    assert!(!active.is_mounted());
    assert_eq!(active.tracking.sample.unwrap().position.x, 103.);
    assert_eq!(active.owner_fields.state.unwrap().state, 1);
    assert_eq!(active.owner_fields.yaw, Some(256));
    assert_eq!(
        ground_seat_event(&mut engine, false, 0x01000000, 9, 0, 104)[0].payload_hex,
        frames[0].payload_hex
    );
    let sequence = ObjectUpdate::decode_with_profile(&packet, engine.wire_profile)
        .unwrap()
        .header
        .sequence;
    assert_eq!(
        protocol::decode_hex(&owner_update(&mut engine, 105, sequence)[0].payload_hex),
        packet
    );
    spawn_apc(&mut engine, 1, 1, 0, 110);
    assert_eq!(engine.active.as_ref().unwrap().ground_seat.unwrap().seat, 9);
}
#[test]
fn apc_evidence_failure_preserves_pending_life_and_vacant_seat() {
    let mut engine = apc_engine(1, true);
    reserve(&mut engine, 0);
    ready(engine.active.as_mut().unwrap());
    owner_update(&mut engine, 100, 0);
    let active = engine.active.as_mut().unwrap();
    active.pending_spawn_deadline = Some(Duration::from_millis(100));
    engine.max_log = engine.written;
    let packet = ObjectUpdate {
        header: UpdateHeader {
            sequence: 2,
            nak_sequence: 0,
            client_index: 0,
            object_type_mask: 0,
            timestamp_ms: 101,
        },
        wire: &[],
    }
    .encode_empty_with_profile(engine.wire_profile)
    .unwrap();
    assert!(dispatch(&mut engine, &packet, 101).is_err());
    let active = engine.active.as_ref().unwrap();
    assert!(!active.life.is_alive());
    assert!(active.ground_seat.is_none());
    assert_eq!(
        active.pending_spawn_deadline,
        Some(Duration::from_millis(100))
    );
    assert!(active.spawn_delta.is_none());
}
#[test]
fn apc_late_join_mount_snapshot_waits_for_creation_and_lifecycle_frees_seats() {
    let mut engine = apc_engine(1, true);
    spawn_apc(&mut engine, 0, 1, 0, 100);
    let snapshot = engine
        .active
        .as_ref()
        .unwrap()
        .snapshot(Duration::from_millis(102))
        .unwrap();
    reserve(&mut engine, 1);
    ready(engine.active.as_mut().unwrap());
    engine
        .active
        .as_mut()
        .unwrap()
        .queue_shared(snapshot.clone());
    assert!(engine.drain_shared(Vec::new(), 1).unwrap().is_empty());
    let messages = owner_update(&mut engine, 103, 0);
    let messages = engine.drain_shared(messages, 1).unwrap();
    let mine: Vec<_> = messages.iter().filter(|m| m.player_id == 1).collect();
    assert!(mine.len() >= 2);
    assert_eq!(
        ObjectUpdate::decode_with_profile(
            &protocol::decode_hex(&mine[0].payload_hex),
            engine.wire_profile
        )
        .unwrap()
        .header
        .object_type_mask,
        1 << 1
    );
    // Release removes the occupied actor rather than retaining an external seat table.
    let response = engine.handle(
        Request::Release {
            v: BRIDGE_VERSION,
            request_id: "synthetic-release-seat0".into(),
            reservation_id: "synthetic-reservation-0000".into(),
        },
        Duration::from_millis(104),
        1,
    );
    assert!(response.ok, "{:?}", response.error);
    engine.active = engine.peers.remove("synthetic-reservation-0001");
    assert_eq!(
        engine
            .ground_passenger_seat(engine.ground_vehicles.as_ref().unwrap().vehicle(0).unwrap())
            .unwrap()
            .seat,
        9
    );
    engine.active.as_mut().unwrap().ground_seat = Some(ground_seats::PassengerSeat {
        vehicle_index: 0,
        seat: 9,
    });
    let damage = replication::CharacterDamage {
        health_quarter_points: 0,
        ..engine.active.as_ref().unwrap().last_damage
    };
    engine.active.as_mut().unwrap().life = CharacterLife::Alive {
        generation: 1,
        health_quarter_points: 400,
    };
    engine
        .active
        .as_mut()
        .unwrap()
        .apply_peer_damage(1, damage, Duration::from_millis(105))
        .unwrap();
    assert!(engine.active.as_ref().unwrap().ground_seat.is_none());
    engine.active.as_mut().unwrap().ground_seat = Some(ground_seats::PassengerSeat {
        vehicle_index: 0,
        seat: 9,
    });
    engine.reset_objective_world();
    assert!(engine.active.as_ref().unwrap().ground_seat.is_none());
}

#[test]
fn apc_allocation_uses_external_seats_and_capacity_without_authored_defaults() {
    let mut engine = apc_engine(1, true);
    reserve(&mut engine, 0);
    let mut vehicle = engine
        .ground_vehicles
        .as_ref()
        .unwrap()
        .vehicle(0)
        .unwrap()
        .clone();
    vehicle.passenger_seats = vec![2, 4, 9];
    vehicle.maximum_occupancy = Some(2);
    assert_eq!(engine.ground_passenger_seat(&vehicle).unwrap().seat, 9);
    engine.active.as_mut().unwrap().ground_seat = Some(ground_seats::PassengerSeat {
        vehicle_index: 0,
        seat: 9,
    });
    assert_eq!(engine.ground_passenger_seat(&vehicle).unwrap().seat, 4);
    reserve(&mut engine, 1);
    engine.active.as_mut().unwrap().ground_seat = Some(ground_seats::PassengerSeat {
        vehicle_index: 0,
        seat: 4,
    });
    assert!(engine.ground_passenger_seat(&vehicle).is_none());
}

#[test]
fn apc_manual_mount_requires_explicit_near_current_position() {
    for policy in [true, false] {
        for position_case in [
            "near",
            "far",
            "stale",
            "future",
            "missing",
            "stale_sequence",
        ] {
            let mut engine = apc_engine(1, policy);
            spawn_apc(&mut engine, 0, 1, 0, 100);
            ground_seat_event(&mut engine, false, 0x01000000, 9, 0, 103);
            let active = engine.active.as_mut().unwrap();
            match position_case {
                "far" => active.tracking.sample.as_mut().unwrap().position.x = 1000.,
                "stale" => active.tracking.sample.as_mut().unwrap().received_at_ms = 0,
                "future" => active.tracking.sample.as_mut().unwrap().received_at_ms = 1000,
                "missing" => active.tracking.sample = None,
                "stale_sequence" => {
                    let sample = active.tracking.sample.as_mut().unwrap();
                    sample.origin = player_tracking::PositionOrigin::ClientReported;
                    sample.client_sequence = Some(999);
                }
                _ => (),
            }
            let packets = ground_seat_event(&mut engine, true, 0x01000000, 8, 0, 104);
            assert_eq!(
                !packets.is_empty(),
                policy && position_case == "near",
                "{policy} {position_case}"
            );
        }
    }
}
#[test]
fn apc_same_vehicle_seat_switch_preserves_capacity_and_denies_exact_repeat() {
    let mut engine = apc_engine(1, false);
    spawn_apc(&mut engine, 0, 1, 0, 100);
    engine.active.as_mut().unwrap().tracking.sample = None;
    assert!(!ground_seat_event(&mut engine, true, 0x01000000, 8, 0, 103).is_empty());
    assert_eq!(engine.active.as_ref().unwrap().ground_seat.unwrap().seat, 8);
    assert!(ground_seat_event(&mut engine, true, 0x01000000, 8, 0, 104).is_empty());
    assert!(ground_seat_event(&mut engine, true, 0x01000001, 9, 0, 105).is_empty());
}
