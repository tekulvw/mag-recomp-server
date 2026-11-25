//! Indexed hospital routing through real Engine event planning and recording.
use super::*;
use crate::spawn_progression::dispatch;

pub(crate) fn fixture() -> Engine<Vec<u8>> {
    let mut engine = bunker_team_tests::fixture(1, 1, true);
    engine.wire_profile = engine.wire_profiles.profile(20231).unwrap();
    let squads: Vec<_> = (0..32)
        .map(|i| {
            json!({"index":i,"team":i/16,"platoon":i%16/4,"squad":i%4,
        "faction":if i<16 {None} else {Some(0)},"available":if i<16 {vec![0]} else {vec![1,2]},
        "default_spawn":if i<16 {0} else {1}})
        })
        .collect();
    let spawns=spawn_catalog::SpawnCatalog::from_json(&json!({
        "format":"mag_spawn_catalog","schema_version":1,"source":"synthetic hospital events",
        "active":{"map_name":"hospital_test","squad_index":16,"default_spawn":1,"available":[1,2]},
        "match_profile":{"game_type":2,"auto_assign_teams":true,"attacking_faction":1,
            "has_bunkers":true,"has_objectives":false,"maximum_points":[111,222]},
        "maps":[{"name":"hospital_test","owner_faction":0,"spawn_count":3,
            "runtime_indices_verified":true,"runtime_indices_basis":"synthetic",
            "squads":squads,"locations":[
                {"index":0,"token":100,"kind":"insertion","position":{"x":0.,"y":0.,"z":0.}},
                {"index":1,"token":101,"kind":"bunker_passenger","position":{"x":0.,"y":0.,"z":0.}},
                {"index":2,"token":102,"kind":"bunker_passenger","position":{"x":0.,"y":0.,"z":0.}},
            ]}],
    }).to_string()).unwrap();
    let actor = engine.active.as_mut().unwrap();
    actor.squad_catalog = std::sync::Arc::new(spawns.for_squad(16).unwrap());
    actor.wire_profile = engine.wire_profile;
    actor.outbound_sequence = 4;
    actor.tracking.server_position(
        replication::CharacterPosition {
            x: 0.,
            y: 0.,
            z: 0.,
        },
        player_tracking::PositionOrigin::ServerSpawn,
        1,
    );
    engine.spawn_catalog = Some(spawns);
    engine.objective_catalog = None;
    engine.objective_world = None;
    engine.match_started = true;
    let world = engine.bunker_world.as_mut().unwrap();
    let mut bunker = world.bunkers.remove(&0).unwrap();
    bunker.repair_points = vec![[0., 0., 0.]];
    bunker.exit = Some(vehicle_exit::BunkerExitPolicy {
        transform: vehicle_exit::BunkerTransform {
            position: replication::CharacterPosition {
                x: 0.,
                y: 0.,
                z: 0.,
            },
            rotation_xyzw: [0., 0., 0., 1.],
        },
        candidates: vec![vehicle_exit::ExitCandidate {
            ranking_offset: [1., 0., 0.],
            position_offset: [2., 0., 0.],
        }],
    });
    bunker.domination = Some(DominationHospitalAccess {
        mount_radii: vec![2.],
        max_occupancy: 12,
        passenger_seats: 12,
    });
    bunker.spawn = 1;
    world.bunkers.insert(5, bunker.clone());
    bunker.spawn = 2;
    world.bunkers.insert(6, bunker);
    engine
}
fn mount(engine: &mut Engine<Vec<u8>>, index: u16, seat: u32) -> Vec<OutboundMessage> {
    let mut wire = 0x0216u16.to_be_bytes().to_vec();
    wire.extend((0x0d00_0000 | u32::from(index)).to_be_bytes());
    wire.extend(seat.to_be_bytes());
    wire.extend(0x0380_0000u32.to_be_bytes());
    dispatch(engine, &wire, 1000).unwrap()
}
fn peer(engine: &mut Engine<Vec<u8>>, index: u16, seat: u8) {
    let mut actor = fixture().active.take().unwrap();
    actor.reservation.id = "synthetic-hospital-peer".into();
    actor.reservation.player = 1;
    actor.mounted_bunker = Some(index);
    actor.mounted_seat = Some(seat);
    engine.peers.insert(actor.reservation.id.clone(), actor);
}
#[test]
fn same_passenger_seat_is_independent_between_hospitals_and_replays_once() {
    let mut engine = fixture();
    peer(&mut engine, 5, 12);
    assert!(mount(&mut engine, 5, 12).is_empty());
    let mounted = mount(&mut engine, 6, 12);
    assert!(!mounted.is_empty());
    assert_eq!(engine.active.as_ref().unwrap().mounted_bunker, Some(6));
    assert_eq!(engine.active.as_ref().unwrap().mounted_seat, Some(12));
    assert_eq!(
        engine
            .active
            .as_ref()
            .unwrap()
            .owner_fields
            .state
            .unwrap()
            .user_data,
        [13, 6, 12]
    );
    let replay = mount(&mut engine, 6, 12);
    assert_eq!(mounted[0].payload_hex, replay[0].payload_hex);
    assert_eq!(engine.active.as_ref().unwrap().seat_transitions.len(), 1);
    assert_eq!(engine.bunker_ownership_after(5, u16::MAX, None).owner, 1);
    assert_eq!(engine.bunker_ownership_after(6, u16::MAX, None).owner, 0);
    assert!(engine.alpha_owner().is_none());
}
#[test]
fn source_hospital_mount_preserves_side_health_range_and_supported_seat_guards() {
    for guard in [
        "side",
        "dead",
        "disabled",
        "range",
        "uncreated",
        "gunner",
        "seat",
    ] {
        let mut engine = fixture();
        let actor = engine.active.as_mut().unwrap();
        match guard {
            "side" => actor.reservation.character.faction = 1,
            "dead" => actor.life = CharacterLife::AwaitingRespawn { generation: 1 },
            "range" => actor.tracking.server_position(
                replication::CharacterPosition {
                    x: 99.,
                    y: 0.,
                    z: 0.,
                },
                player_tracking::PositionOrigin::ServerSpawn,
                1,
            ),
            "uncreated" => actor.emplacement_creation = None,
            "disabled" => {
                engine
                    .bunker_world
                    .as_mut()
                    .unwrap()
                    .bunkers
                    .get_mut(&5)
                    .unwrap()
                    .vehicle_state = replication::VehicleState::Destroyed
            }
            _ => (),
        }
        let seat = if guard == "gunner" {
            0
        } else if guard == "seat" {
            13
        } else {
            12
        };
        assert!(mount(&mut engine, 5, seat).is_empty(), "{guard}");
        assert!(
            engine.active.as_ref().unwrap().mounted_seat.is_none(),
            "{guard}"
        );
    }
}

fn occupant_fixture() -> (
    Engine<Vec<u8>>,
    BTreeMap<u32, acquisition_character_effects::NativeState>,
) {
    let mut engine = crate::human_kill_reward_tests::enable_incoming_base(
        crate::human_kill_reward_tests::fixture(),
        0.25,
    );
    engine.bunker_world = fixture().bunker_world.take();
    let active = engine.active.as_mut().unwrap();
    active.mounted_bunker = Some(5);
    active.mounted_seat = Some(12);
    let peer = engine.peers.values_mut().next().unwrap();
    peer.mounted_bunker = Some(6);
    peer.mounted_seat = Some(12);
    let native = engine
        .active
        .iter()
        .chain(engine.peers.values())
        .map(|actor| {
            (
                0x0380_0000 | u32::from(actor.reservation.player),
                acquisition_character_effects::NativeState {
                    invulnerability: Some(0),
                    associated_vehicle: Some(protocol::VictimWasMounted::AssociatedVehicle),
                },
            )
        })
        .collect();
    (engine, native)
}

#[test]
fn hospital_occupant_damage_records_before_install_and_preserves_the_other_hospital() {
    let (mut engine, mut native) = occupant_fixture();
    let candidate = engine
        .prepare_domination_bunker_occupants(5, u32::MAX, &native, Duration::from_secs(1))
        .unwrap();
    assert_eq!(candidate.audit().len(), 1);
    assert_eq!(candidate.audit()[0]["bunker_index"], 5);
    assert!(engine.active.as_ref().unwrap().life.is_alive());
    assert_eq!(engine.active.as_ref().unwrap().mounted_bunker, Some(5));
    engine
        .record_acquisition_character_effects(candidate, &mut native, 1)
        .unwrap();
    assert!(!engine.active.as_ref().unwrap().life.is_alive());
    assert!(engine.active.as_ref().unwrap().mounted_seat.is_none());
    assert!(engine.active.as_ref().unwrap().mounted_bunker.is_none());
    let other = engine.peers.values().next().unwrap();
    assert!(other.life.is_alive());
    assert_eq!(other.mounted_bunker, Some(6));
    assert_eq!(other.mounted_seat, Some(12));
}

#[test]
fn late_occupant_resolution_failure_rolls_back_health_seats_and_publications() {
    let (mut engine, mut native) = occupant_fixture();
    let peer = engine.peers.values_mut().next().unwrap();
    peer.mounted_bunker = Some(5);
    peer.mounted_seat = Some(11);
    native.remove(&(0x0380_0000 | u32::from(engine.active.as_ref().unwrap().reservation.player)));
    let before = engine.writer.len();
    assert_eq!(
        engine
            .prepare_domination_bunker_occupants(5, u32::MAX, &native, Duration::from_secs(1))
            .err(),
        Some("bunker_occupant_native_state_missing")
    );
    assert_eq!(engine.writer.len(), before);
    for actor in engine.active.iter().chain(engine.peers.values()) {
        assert!(actor.life.is_alive());
        assert_eq!(actor.mounted_bunker, Some(5));
        assert!(actor.mounted_seat.is_some());
        assert!(actor.pending_notifications.is_empty());
    }
}

#[test]
fn failed_occupant_evidence_record_leaves_authority_mounted_and_alive() {
    let (mut engine, mut native) = occupant_fixture();
    let candidate = engine
        .prepare_domination_bunker_occupants(5, u32::MAX, &native, Duration::from_secs(1))
        .unwrap();
    engine.max_log = engine.written;
    assert!(
        engine
            .record_acquisition_character_effects(candidate, &mut native, 1)
            .is_err()
    );
    assert!(engine.active.as_ref().unwrap().life.is_alive());
    assert_eq!(engine.active.as_ref().unwrap().mounted_bunker, Some(5));
    assert_eq!(engine.active.as_ref().unwrap().mounted_seat, Some(12));
}

#[test]
fn queued_hospital_respawn_uses_its_own_body_and_authored_passenger_capacity() {
    let mut engine = fixture();
    let seat_position = replication::CharacterPosition {
        x: 11.,
        y: 3.,
        z: 17.,
    };
    engine
        .bunker_world
        .as_mut()
        .unwrap()
        .bunkers
        .get_mut(&6)
        .unwrap()
        .exit
        .as_mut()
        .unwrap()
        .transform
        .position = seat_position;
    let actor = engine.active.as_mut().unwrap();
    actor.squad_spawn = 2;
    actor.selected_spawn = Some(protocol::CharacterSpawnSelection {
        spawn_point_index: 2,
    });
    actor.life = CharacterLife::AwaitingRespawn { generation: 1 };
    actor.pending_spawn_deadline = Some(Duration::from_millis(100));
    actor.mounted_seat = None;
    actor.mounted_bunker = None;
    actor.round_scores = Some([111, 222]);
    for tick in 100..108 {
        let header = replication::UpdateHeader {
            sequence: engine.active.as_ref().unwrap().latest_owner_sequence + 1,
            nak_sequence: 0,
            client_index: 0,
            object_type_mask: 0,
            timestamp_ms: tick,
        };
        let packet = replication::ObjectUpdate { header, wire: &[] }
            .encode_empty_with_profile(engine.wire_profile)
            .unwrap();
        dispatch(&mut engine, &packet, u64::from(tick)).unwrap();
        if engine.active.as_ref().unwrap().life.is_alive() {
            break;
        }
    }
    let actor = engine.active.as_ref().unwrap();
    assert!(actor.life.is_alive(), "{:?}", actor.life);
    assert_eq!(actor.mounted_bunker, Some(6));
    assert_eq!(actor.mounted_seat, Some(12));
    assert_eq!(actor.owner_fields.state.unwrap().user_data, [13, 6, 12]);
    assert_eq!(actor.tracking.sample.unwrap().position, seat_position);
}

#[test]
fn full_hospital_respawn_preserves_pending_actor_position() {
    let mut engine = fixture();
    for seat in 1..=12 {
        let mut actor = fixture().active.take().unwrap();
        actor.reservation.id = format!("synthetic-hospital-passenger-{seat}");
        actor.reservation.player = u16::from(seat);
        actor.mounted_bunker = Some(6);
        actor.mounted_seat = Some(seat);
        engine.peers.insert(actor.reservation.id.clone(), actor);
    }
    let active = engine.active.as_mut().unwrap();
    active.selected_spawn = Some(protocol::CharacterSpawnSelection {
        spawn_point_index: 2,
    });
    active.squad_spawn = 2;
    active.life = CharacterLife::AwaitingRespawn { generation: 1 };
    active.pending_spawn_deadline = Some(Duration::from_millis(100));
    active.round_scores = Some([111, 222]);
    let original = replication::CharacterPosition {
        x: 21.,
        y: 4.,
        z: 29.,
    };
    active
        .tracking
        .server_position(original, player_tracking::PositionOrigin::ServerSpawn, 1);
    let header = replication::UpdateHeader {
        sequence: active.latest_owner_sequence + 1,
        nak_sequence: 0,
        client_index: 0,
        object_type_mask: 0,
        timestamp_ms: 100,
    };
    let packet = replication::ObjectUpdate { header, wire: &[] }
        .encode_empty_with_profile(engine.wire_profile)
        .unwrap();
    dispatch(&mut engine, &packet, 100).unwrap();
    let active = engine.active.as_ref().unwrap();
    assert_eq!(active.tracking.sample.unwrap().position, original);
    assert_eq!(active.tracking.sample.unwrap().received_at_ms, 1);
    assert_eq!(
        active.pending_spawn_deadline,
        Some(Duration::from_millis(100))
    );
    assert!(matches!(active.life, CharacterLife::AwaitingRespawn { .. }));
    assert_eq!(active.mounted_bunker, None);
    assert_eq!(active.mounted_seat, None);
}

#[test]
fn indexed_hospital_zero_is_not_a_legacy_alpha_occupant() {
    let mut engine = fixture();
    let hospital = engine
        .bunker_world
        .as_mut()
        .unwrap()
        .bunkers
        .remove(&5)
        .unwrap();
    engine
        .bunker_world
        .as_mut()
        .unwrap()
        .bunkers
        .insert(0, hospital);
    peer(&mut engine, 0, 12);
    assert_eq!(engine.alpha_owner(), None);
    assert_eq!(engine.alpha_ownership_after(u16::MAX, None).owner, 0x105);
    assert_eq!(engine.bunker_ownership_after(0, u16::MAX, None).owner, 1);
    engine.active.as_mut().unwrap().emplacement_creation = None;
    for tick in 100..120 {
        let header = replication::UpdateHeader {
            sequence: engine.active.as_ref().unwrap().latest_owner_sequence + 1,
            nak_sequence: 0,
            client_index: 0,
            object_type_mask: 0,
            timestamp_ms: tick,
        };
        let packet = replication::ObjectUpdate { header, wire: &[] }
            .encode_empty_with_profile(engine.wire_profile)
            .unwrap();
        dispatch(&mut engine, &packet, u64::from(tick)).unwrap();
        if engine
            .active
            .as_ref()
            .unwrap()
            .emplacement_creation
            .is_some()
        {
            break;
        }
    }
    assert!(
        engine
            .active
            .as_ref()
            .unwrap()
            .emplacement_creation
            .is_some()
    );
}

#[test]
fn indexed_hospital_rejects_legacy_damage_and_critical_decay() {
    let mut engine = fixture();
    let before = engine.bunker_world.as_ref().unwrap().bunkers[&5]
        .health
        .hp();
    let mut hit = vec![0; bullet_hit::WIRE_LEN];
    hit[..2].copy_from_slice(&bullet_hit::MESSAGE_ID.to_be_bytes());
    hit[9..13].copy_from_slice(&0x0d00_0005u32.to_be_bytes());
    hit[37..].copy_from_slice(&1000u16.to_be_bytes());
    dispatch(&mut engine, &hit, 1000).unwrap();
    assert!(
        std::str::from_utf8(&engine.writer)
            .unwrap()
            .contains("domination_hospital_damage_consumer_required")
    );
    assert_eq!(
        engine.bunker_world.as_ref().unwrap().bunkers[&5]
            .health
            .hp(),
        before
    );
    let hospital = engine
        .bunker_world
        .as_mut()
        .unwrap()
        .bunkers
        .get_mut(&5)
        .unwrap();
    hospital.health.damage(before - 1.).unwrap();
    hospital.profile.critical_loss_percent_per_second = 50.;
    assert_ne!(hospital.health.conditions() & 4, 0);
    let before = hospital.health.hp();
    assert!(
        engine
            .tick_bunkers(Duration::from_secs(2), 2)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        engine.bunker_world.as_ref().unwrap().bunkers[&5]
            .health
            .hp(),
        before
    );
    assert!(engine.active.as_ref().unwrap().pending_shared.is_empty());
}

/// The fixture's pool plus an on-foot HQ insertion (point 3) that defending
/// squads also own; each defending squad is routed to its hospital (1 or 2).
fn catalog_with_hq() -> spawn_catalog::SpawnCatalog {
    let squads: Vec<_> = (0..32)
        .map(|i| {
            json!({"index":i,"team":i/16,"platoon":i%16/4,"squad":i%4,
            "faction":if i<16 {None} else {Some(0)},
            "available":if i<16 {vec![0]} else {vec![3,1,2]},"default_spawn":if i<16 {0} else {3}})
        })
        .collect();
    spawn_catalog::SpawnCatalog::from_json(&json!({
        "format":"mag_spawn_catalog","schema_version":1,"source":"synthetic hospital HQ fallback",
        "active":{"map_name":"hospital_test","squad_index":16,"default_spawn":3,"available":[3,1,2]},
        "match_profile":{"game_type":2,"auto_assign_teams":true,"attacking_faction":1,
            "has_bunkers":true,"has_objectives":false,"maximum_points":[111,222]},
        "maps":[{"name":"hospital_test","owner_faction":0,"spawn_count":4,
            "runtime_indices_verified":true,"runtime_indices_basis":"synthetic",
            "squads":squads,"locations":[
                {"index":0,"token":100,"kind":"insertion","position":{"x":0.,"y":0.,"z":0.}},
                {"index":1,"token":101,"kind":"bunker_passenger","position":{"x":0.,"y":0.,"z":0.}},
                {"index":2,"token":102,"kind":"bunker_passenger","position":{"x":0.,"y":0.,"z":0.}},
                {"index":3,"token":103,"kind":"insertion","position":{"x":5.,"y":0.,"z":7.}},
            ]}],
    }).to_string()).unwrap()
}

fn tick_until_alive(engine: &mut Engine<Vec<u8>>) {
    for tick in 100..108 {
        let header = replication::UpdateHeader {
            sequence: engine.active.as_ref().unwrap().latest_owner_sequence + 1,
            nak_sequence: 0,
            client_index: 0,
            object_type_mask: 0,
            timestamp_ms: tick,
        };
        let packet = replication::ObjectUpdate { header, wire: &[] }
            .encode_empty_with_profile(engine.wire_profile)
            .unwrap();
        dispatch(engine, &packet, u64::from(tick)).unwrap();
        if engine.active.as_ref().unwrap().life.is_alive() {
            break;
        }
    }
}

fn await_respawn_at(engine: &mut Engine<Vec<u8>>, selected: u16) {
    let spawns = catalog_with_hq();
    let actor = engine.active.as_mut().unwrap();
    actor.squad_catalog = std::sync::Arc::new(spawns.for_squad(16).unwrap());
    actor.squad_spawn = selected;
    actor.selected_spawn = Some(protocol::CharacterSpawnSelection {
        spawn_point_index: selected,
    });
    actor.life = CharacterLife::AwaitingRespawn { generation: 1 };
    actor.pending_spawn_deadline = Some(Duration::from_millis(100));
    actor.mounted_seat = None;
    actor.mounted_bunker = None;
    actor.round_scores = Some([111, 222]);
    engine.spawn_catalog = Some(spawns);
}

#[test]
fn a_destroyed_selected_hospital_respawns_the_player_at_the_squads_hq_point() {
    let mut engine = fixture();
    await_respawn_at(&mut engine, 2);
    engine
        .bunker_world
        .as_mut()
        .unwrap()
        .apply_domination_cycle(2, false, u32::MAX)
        .unwrap();
    tick_until_alive(&mut engine);
    let actor = engine.active.as_ref().unwrap();
    assert!(actor.life.is_alive(), "{:?}", actor.life);
    assert_eq!(actor.mounted_bunker, None);
    assert_eq!(actor.mounted_seat, None);
    let at = actor.tracking.sample.unwrap().position;
    assert_eq!((at.x, at.z), (5., 7.));
}

#[test]
fn an_operational_selected_hospital_still_seats_the_player() {
    let mut engine = fixture();
    await_respawn_at(&mut engine, 2);
    tick_until_alive(&mut engine);
    let actor = engine.active.as_ref().unwrap();
    assert!(actor.life.is_alive(), "{:?}", actor.life);
    assert_eq!(actor.mounted_bunker, Some(6));
}

#[test]
fn a_hospital_destroyed_by_damage_retires_its_spawn_until_restored() {
    let mut world = fixture().bunker_world.take().unwrap();
    let mut spawns = catalog_with_hq();
    for squad in 16..32u16 {
        let hospital = 1 + squad % 2;
        spawns.set_runtime_squad_default(squad, hospital).unwrap();
    }
    let category_zero = std::array::from_fn(|squad| if squad < 16 { 0 } else { 3 });
    let retire = |spawns: &mut spawn_catalog::SpawnCatalog, world: &BunkerWorld| {
        crate::domination_gameplay_provider::retire_destroyed_hospital_spawns(
            spawns,
            world,
            &category_zero,
        )
        .unwrap()
    };
    assert!(retire(&mut spawns, &world).is_empty());
    world.apply_domination_cycle(2, false, u32::MAX).unwrap();
    assert_eq!(retire(&mut spawns, &world), [2]);
    assert!(!spawns.point_active(2));
    assert!(spawns.point_active(1));
    for squad in 16..32u16 {
        let expected = if squad % 2 == 1 { 3 } else { 1 };
        assert_eq!(spawns.default_for_squad(squad), Some(expected), "{squad}");
    }
    // Retired once; the other hospital and the attackers are untouched.
    assert!(retire(&mut spawns, &world).is_empty());
    assert_eq!(spawns.default_for_squad(0), Some(0));
    // The FSP activation that restores the hospital reactivates its point.
    world.apply_domination_cycle(2, true, u32::MAX).unwrap();
    spawns.set_runtime_point_active(2, 102, true).unwrap();
    spawns.set_runtime_squad_default(17, 2).unwrap();
    assert!(retire(&mut spawns, &world).is_empty());
    assert!(spawns.for_squad(17).unwrap().alpha(2).is_some());
}
