//! Current vehicle and actor authority through the concrete Acquisition tick.
use super::*;
use crate::ground_vehicles::arrival_static::Binding;
use std::sync::atomic::{AtomicUsize, Ordering};

pub(crate) fn delivery_fixture() -> Engine<Vec<u8>> {
    delivery_fixture_with_container(true)
}

/// `container` adds the synthetic roof over the first two Arrival triggers.
fn delivery_fixture_with_container(container: bool) -> Engine<Vec<u8>> {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "mag-delivery-fixture-{}-{}.json",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let mut instances = vec![
        json!({"id":"invented-floor", "vertices_xyz":[[950.,2.,980.],[1120.,2.,980.],[950.,2.,1080.],[1120.,2.,1080.]],
             "triangles":[[0,1,2],[1,2,3]]}),
    ];
    if container {
        instances.extend([
            json!({"id":"invented-witness", "vertices_xyz":[[999.5,7.,1039.5],[1000.5,7.,1039.5],[1000.,7.,1040.5]],
                 "triangles":[[0,1,2]]}),
            json!({"id":"invented-second-witness", "vertices_xyz":[[1029.5,7.,1039.5],[1030.5,7.,1039.5],[1030.,7.,1040.5]],
                 "triangles":[[0,1,2]]}),
        ]);
    }
    std::fs::write(
        &path,
        json!({
            "format":"mag_collision_world", "schema_version":1, "coordinate_space":"world",
            "metadata":{"map_name":"synthetic_arena","build":"synthetic","provenance":["synthetic"],
                "unknowns":[],"coverage":"provisional"},
            "instances":instances,
        })
        .to_string(),
    )
    .unwrap();
    let world = collision::CollisionWorld::load(&path, Default::default()).unwrap();
    let binding = Binding {
        path: path.clone(),
        sha256: world.source_sha256().to_owned(),
    };
    let static_scene = binding
        .load(std::path::Path::new("."), "synthetic_arena")
        .unwrap();
    std::fs::remove_file(path).unwrap();
    let mut engine =
        configured_capture_fixture(None, Some(static_scene)).with_collision_world(world);
    start_capture(&mut engine, 0, 100);
    engine
        .tick_acquisition_objectives(Duration::from_millis(3200), 2)
        .unwrap();
    engine.tick_acquisition_ground_registration(2).unwrap();
    let actor = engine.active.as_mut().unwrap();
    actor.ground_seat = Some(ground_seats::PassengerSeat {
        vehicle_index: 4,
        seat: 0,
    });
    actor.ground_vehicle_batch = 1;
    engine
}

pub(crate) fn position(engine: &mut Engine<Vec<u8>>, x: f32, z: f32) {
    position_vehicle(engine, 4, x, z);
}

fn position_vehicle(engine: &mut Engine<Vec<u8>>, index: u16, x: f32, z: f32) {
    let runtime = engine.ground_runtime.as_mut().unwrap();
    let object = runtime.get(index).unwrap();
    let mut pose = object.pose;
    pose.position.x = x;
    pose.position.z = z;
    runtime.force_pose(object.key, pose).unwrap();
}

#[test]
fn acquisition_delivery_requires_trigger_and_positive_ray_then_completes_once() {
    let mut engine = delivery_fixture();
    let arrival = FIRST_CAPTURE + 1;
    let before = engine.acquisition_round.as_ref().unwrap().points();
    for (x, z) in [(1000., 1000.), (1001.5, 1040.)] {
        position(&mut engine, x, z);
        engine
            .tick_acquisition_objectives(Duration::from_millis(3300), 3)
            .unwrap();
        assert_eq!(
            engine
                .ground_runtime
                .as_ref()
                .unwrap()
                .get(4)
                .unwrap()
                .state,
            2
        );
        assert_eq!(
            engine.objective_world.as_ref().unwrap().state(arrival),
            Some(ObjectiveState::Deactivated)
        );
        assert!(engine.active.as_ref().unwrap().ground_seat.is_some());
    }
    position(&mut engine, 1000., 1040.);
    reserve(&mut engine, 1);
    let passenger = engine.active.as_mut().unwrap();
    ready(passenger);
    passenger.life = CharacterLife::Alive {
        generation: 1,
        health_quarter_points: 400,
    };
    passenger.ground_seat = Some(ground_seats::PassengerSeat {
        vehicle_index: 4,
        seat: 2,
    });
    passenger.ground_vehicle_batch = 1;
    engine
        .tick_acquisition_objectives(Duration::from_millis(3400), 4)
        .unwrap();
    let vehicle = engine.ground_runtime.as_ref().unwrap().get(4).unwrap();
    assert_eq!(vehicle.state, 8);
    assert_eq!(vehicle.inert_seconds, 12.);
    assert_eq!(
        engine.objective_world.as_ref().unwrap().state(arrival),
        Some(ObjectiveState::Locked)
    );
    let driver = &engine.peers["synthetic-reservation-0000"];
    let passenger = engine.active.as_ref().unwrap();
    for actor in [driver, passenger] {
        assert!(actor.ground_seat.is_none());
        assert_eq!(actor.owner_fields.state.unwrap().state, 1);
        assert_eq!(actor.tracking.sample.unwrap().position.y, 2.09375);
    }
    assert_eq!(driver.tracking.sample.unwrap().position.x, 1003.);
    assert_eq!(passenger.tracking.sample.unwrap().position.x, 997.);
    let points = engine.acquisition_round.as_ref().unwrap().points();
    assert_eq!(points, [before[0], before[1] - 600]);
    assert_eq!(engine.ground_driver_owner(4), None);
    assert!(driver.pending_shared.iter().any(|packet| {
        let profile = engine.wire_profile;
        if !replication::ObjectUpdate::decode_with_profile(packet, profile)
            .is_ok_and(|update| update.header.object_type_mask == 1 << 1)
        {
            return false;
        }
        let mut reader = replication::BitReader::with_profile(packet, profile);
        reader.position = profile.header_bits();
        reader.read(1) == Ok(1)
            && reader.read(16) == Ok(4)
            && reader.read(2) == Ok(1)
            && reader.read(16) == Ok(0x105)
            && reader.read(4) == Ok(1)
            && reader.read(8) == Ok(8)
    }));
    engine
        .tick_acquisition_objectives(Duration::from_millis(3500), 5)
        .unwrap();
    assert_eq!(engine.acquisition_round.as_ref().unwrap().points(), points);
}

#[test]
fn acquisition_delivery_defers_for_wrong_binding_or_unqualified_current_exit() {
    let mut engine = delivery_fixture();
    position(&mut engine, 1000., 1040.);
    let runtime = engine.ground_runtime.as_mut().unwrap();
    let key = runtime.get(4).unwrap().key;
    runtime
        .enable_cycle(key, 0x0c80_0000 | u32::from(FIRST_CAPTURE + 2))
        .unwrap();
    engine
        .tick_acquisition_objectives(Duration::from_millis(3300), 3)
        .unwrap();
    assert_eq!(
        engine
            .ground_runtime
            .as_ref()
            .unwrap()
            .get(4)
            .unwrap()
            .state,
        2
    );
    let runtime = engine.ground_runtime.as_mut().unwrap();
    runtime
        .enable_cycle(key, 0x0c80_0000 | u32::from(FIRST_CAPTURE))
        .unwrap();
    let mut pose = runtime.get(4).unwrap().pose;
    pose.position.y = 5.;
    runtime.force_pose(key, pose).unwrap();
    engine
        .tick_acquisition_objectives(Duration::from_millis(3400), 4)
        .unwrap();
    assert_eq!(
        engine
            .ground_runtime
            .as_ref()
            .unwrap()
            .get(4)
            .unwrap()
            .state,
        2
    );
    assert_eq!(engine.active.as_ref().unwrap().ground_seat.unwrap().seat, 0);
    assert_eq!(
        engine
            .objective_world
            .as_ref()
            .unwrap()
            .state(FIRST_CAPTURE + 1),
        Some(ObjectiveState::Deactivated)
    );
}

#[test]
fn acquisition_delivery_rejection_and_log_failure_leave_seat_and_escort_installed() {
    let mut engine = delivery_fixture();
    position(&mut engine, 1000., 1040.);
    let actor = engine.active.as_mut().unwrap();
    actor.ground_seat.as_mut().unwrap().seat = 2;
    engine
        .tick_acquisition_objectives(Duration::from_millis(3300), 3)
        .unwrap();
    assert_eq!(
        engine
            .ground_runtime
            .as_ref()
            .unwrap()
            .get(4)
            .unwrap()
            .state,
        2
    );
    engine
        .active
        .as_mut()
        .unwrap()
        .ground_seat
        .as_mut()
        .unwrap()
        .seat = 0;
    let before = engine.ground_runtime.clone();
    let score = engine.acquisition_round.as_ref().unwrap().points();
    engine.max_log = engine.written;
    assert_eq!(
        engine
            .tick_acquisition_objectives(Duration::from_millis(3400), 4)
            .err(),
        Some("log_limit_reached")
    );
    assert_eq!(engine.ground_runtime, before);
    assert_eq!(engine.acquisition_round.as_ref().unwrap().points(), score);
    assert_eq!(engine.active.as_ref().unwrap().ground_seat.unwrap().seat, 0);
}

fn mount_driver(engine: &mut Engine<Vec<u8>>, index: u16, now_ms: u64) {
    let vehicle = engine.current_ground_vehicle(index).unwrap();
    let active = engine.active.as_mut().unwrap();
    active.tracking.server_position(
        vehicle.position,
        player_tracking::PositionOrigin::ServerExit,
        now_ms,
    );
    assert!(active.ground_seat.is_none());
    let request: Vec<_> = 0x0216u16
        .to_be_bytes()
        .into_iter()
        .chain(vehicle.object_id().to_be_bytes())
        .chain(0u32.to_be_bytes())
        .chain((0x0380_0000u32 | u32::from(active.reservation.player)).to_be_bytes())
        .collect();
    assert!(!dispatch(engine, &request, now_ms).unwrap().is_empty());
    assert_eq!(engine.ground_driver_owner(index), Some(0));
}

fn objective_transitions(
    packets: &std::collections::VecDeque<Vec<u8>>,
    profile: wire_profile::WireProfile,
    wanted: u16,
) -> Vec<u32> {
    let mut states = Vec::new();
    for packet in packets {
        if !replication::ObjectUpdate::decode_with_profile(packet, profile)
            .is_ok_and(|update| update.header.object_type_mask == 1 << 12)
        {
            continue;
        }
        let mut reader = replication::BitReader::with_profile(packet, profile);
        reader.position = profile.header_bits();
        while reader.read(1) == Ok(1) {
            let index = reader.read(16).unwrap() as u16;
            for field in 0..11 {
                if reader.read(1) == Ok(0) {
                    continue;
                }
                match field {
                    2 => {
                        let state = reader.read(8).unwrap();
                        let _change_time = reader.read(32).unwrap();
                        let _actor = reader.read(16).unwrap();
                        if index == wanted {
                            states.push(state);
                        }
                    }
                    3 | 4 => {
                        reader.read(32).unwrap();
                    }
                    6 => {
                        reader.read(8).unwrap();
                    }
                    _ => panic!("unexpected synthetic objective field {field}"),
                }
            }
        }
    }
    // The final reconciliation snapshot may repeat the latest state. Preserve
    // every actual transition, including a Completed entry immediately locked.
    states.dedup();
    states
}

#[test]
fn acquisition_delivery_completes_both_stages_publishes_victory_and_requeues() {
    let mut engine = delivery_fixture();
    let a = FIRST_CAPTURE;
    let b = FIRST_CAPTURE + 2;
    let profile = engine.wire_profile;
    let actor = engine.active.as_mut().unwrap();
    actor.ground_seat = None;
    actor.pending_shared.clear();
    assert_eq!(
        engine.objective_world.as_ref().unwrap().state(b),
        Some(ObjectiveState::Inactive)
    );
    mount_driver(&mut engine, 4, 3300);
    // The fixture supplies motion directly to isolate the objective loop. The
    // normal mount request above exercises the same driver authority as live play.
    position_vehicle(&mut engine, 4, 1000., 1040.);
    engine
        .tick_acquisition_objectives(Duration::from_millis(3400), 3)
        .unwrap();
    assert_eq!(
        engine.acquisition_round.as_ref().unwrap().points(),
        [1200, 400]
    );
    assert_eq!(
        engine.objective_world.as_ref().unwrap().state(a),
        Some(ObjectiveState::Locked)
    );
    assert_eq!(
        engine.objective_world.as_ref().unwrap().state(a + 1),
        Some(ObjectiveState::Locked)
    );
    assert_eq!(
        engine.objective_world.as_ref().unwrap().state(b),
        Some(ObjectiveState::Deactivated)
    );
    assert_eq!(primary_assignment(&engine), b);
    assert_eq!(
        objective_transitions(
            &engine.active.as_ref().unwrap().pending_shared,
            profile,
            a + 1
        ),
        [
            ObjectiveState::Completed as u32,
            ObjectiveState::Locked as u32
        ]
    );

    let on_foot = replication::CharacterPosition {
        x: 100.,
        y: 3.,
        z: 100.,
    };
    engine.active.as_mut().unwrap().tracking.server_position(
        on_foot,
        player_tracking::PositionOrigin::ServerExit,
        3500,
    );
    let request = protocol::ObjectiveInteraction {
        kind: protocol::ObjectiveInteractionKind::Activate,
        target_object_id: 0x0c80_0000 | u32::from(b),
        actor_index: 0,
        trigger_index: 0,
    };
    assert!(
        !dispatch(&mut engine, &request.encode(), 3500)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        engine.objective_world.as_ref().unwrap().state(b),
        Some(ObjectiveState::Activating)
    );
    engine
        .tick_acquisition_objectives(Duration::from_millis(6500), 4)
        .unwrap();
    engine.tick_acquisition_ground_registration(4).unwrap();
    assert_eq!(
        engine.objective_world.as_ref().unwrap().state(b),
        Some(ObjectiveState::Completed)
    );
    assert_eq!(primary_assignment(&engine), b + 1);
    mount_driver(&mut engine, 5, 6600);
    position_vehicle(&mut engine, 5, 1030., 1040.);
    engine
        .tick_acquisition_objectives(Duration::from_millis(6700), 5)
        .unwrap();
    assert_eq!(
        engine.acquisition_round.as_ref().unwrap().points(),
        [1200, 0]
    );
    assert_eq!(
        engine.objective_world.as_ref().unwrap().state(b + 1),
        Some(ObjectiveState::Locked)
    );
    assert_eq!(
        objective_transitions(
            &engine.active.as_ref().unwrap().pending_shared,
            profile,
            b + 1
        ),
        [
            ObjectiveState::Deactivated as u32,
            ObjectiveState::Completed as u32,
            ObjectiveState::Locked as u32
        ]
    );

    engine
        .tick_match_clock(Duration::from_millis(6700), 5)
        .unwrap();
    let first_match = engine.match_terminal.as_ref().unwrap();
    assert_eq!(first_match.reason, "attacker_victory");
    let first_id = first_match.match_id.clone();
    let terminal_wire = protocol::encode_hex(
        &protocol::EndOfGame {
            winning_team: 0,
            major_victory: true,
            end_time_ms: 8700,
        }
        .encode_with_profile(profile)
        .unwrap(),
    );
    let shared = engine.drain_shared(Vec::new(), 5).unwrap();
    let notices = engine.drain_notifications(shared, 5).unwrap();
    assert_eq!(
        notices
            .iter()
            .filter(|n| n.payload_hex == terminal_wire)
            .count(),
        1
    );
    engine
        .tick_match_clock(Duration::from_millis(8699), 6)
        .unwrap();
    assert!(!engine.round.outcome.as_ref().unwrap().finalized);
    engine
        .tick_match_clock(Duration::from_millis(8700), 6)
        .unwrap();
    assert!(engine.round.outcome.as_ref().unwrap().finalized);
    assert_eq!(engine.match_terminal.as_ref().unwrap().match_id, first_id);

    let response = engine.handle(
        Request::Release {
            v: BRIDGE_VERSION,
            request_id: "synthetic-full-delivery-release".into(),
            reservation_id: "synthetic-reservation-0000".into(),
        },
        Duration::from_millis(8701),
        6,
    );
    assert!(response.ok, "{:?}", response.error);
    reserve(&mut engine, 1); // Admission installs the new round; release retains old receipts.
    assert!(engine.match_terminal.is_none());
    assert_eq!(
        engine.acquisition_round.as_ref().unwrap().points(),
        [1200, 1000]
    );
    assert_eq!(
        engine.objective_world.as_ref().unwrap().state(a),
        Some(ObjectiveState::Inactive)
    );
    assert_eq!(
        engine
            .ground_runtime
            .as_ref()
            .unwrap()
            .get(4)
            .unwrap()
            .state,
        0
    );
    assert_eq!(
        engine
            .ground_runtime
            .as_ref()
            .unwrap()
            .get(5)
            .unwrap()
            .state,
        0
    );
    start_capture(&mut engine, 1, 9000);
    engine
        .tick_acquisition_objectives(Duration::from_millis(12100), 7)
        .unwrap();
    engine.tick_acquisition_ground_registration(7).unwrap();
    assert!(engine.ground_vehicle_is_operational(4));
    assert_eq!(primary_assignment(&engine), a + 1);
}

#[test]
fn escort_delivery_records_gunner_revoke_with_state_and_rolls_back_on_failure() {
    for fail_record in [false, true] {
        let mut engine = delivery_fixture();
        position(&mut engine, 1000., 1040.);
        crate::ground_turrets::tests::opt_in(&mut engine, 4);
        reserve(&mut engine, 1);
        let vehicle = engine.current_ground_vehicle(4).unwrap();
        let gunner = engine.active.as_mut().unwrap();
        ready(gunner);
        gunner.life = CharacterLife::Alive {
            generation: 1,
            health_quarter_points: 400,
        };
        gunner.ground_vehicle_batch = 1;
        gunner.tracking.server_position(
            vehicle.position,
            player_tracking::PositionOrigin::ServerExit,
            3250,
        );
        let mount: Vec<_> = 0x0216u16
            .to_be_bytes()
            .into_iter()
            .chain(vehicle.object_id().to_be_bytes())
            .chain(1u32.to_be_bytes())
            .chain(0x0380_0001u32.to_be_bytes())
            .collect();
        dispatch(&mut engine, &mount, 3250).unwrap();
        let gunner = engine.active.as_ref().unwrap();
        assert_eq!(gunner.ground_seat.unwrap().seat, 1);
        assert_eq!(
            engine.ground_gunner_kind(gunner),
            Some(crate::ground_turrets::PolicyKind::Escort)
        );
        let turret = engine
            .ground_runtime
            .as_ref()
            .unwrap()
            .get(4)
            .unwrap()
            .turret
            .as_ref()
            .unwrap();
        let version = turret.definition(true).version;
        let (ready_turret, _) = turret
            .report(crate::hospital_turret_wire::Fields {
                primary: Some(replication::InventoryItem {
                    inventory_update_id: version,
                    gear_slot: 0,
                    ammo_pool: 0,
                    ammo_count: 20,
                    reloading: false,
                }),
                ..Default::default()
            })
            .unwrap();
        engine
            .ground_runtime
            .as_mut()
            .unwrap()
            .install_turret(4, ready_turret);
        let before = engine.ground_runtime.clone();
        let initial_score = engine.acquisition_round.as_ref().unwrap().points();
        let initial_log = engine.writer.clone();
        let driver_queue = engine.peers["synthetic-reservation-0000"]
            .pending_shared
            .clone();
        if fail_record {
            engine.max_log = engine.written;
        }
        let result = engine.tick_acquisition_objectives(Duration::from_millis(3400), 4);
        if fail_record {
            assert_eq!(result.err(), Some("log_limit_reached"));
            assert_eq!(engine.ground_runtime, before);
            assert_eq!(engine.writer, initial_log);
            assert_eq!(
                engine.acquisition_round.as_ref().unwrap().points(),
                initial_score
            );
            assert_eq!(engine.active.as_ref().unwrap().ground_seat.unwrap().seat, 1);
            assert_eq!(
                engine.peers["synthetic-reservation-0000"].pending_shared,
                driver_queue
            );
            continue;
        }
        result.unwrap();
        let vehicle = engine.ground_runtime.as_ref().unwrap().get(4).unwrap();
        assert_eq!(vehicle.state, 8);
        let turret = vehicle.turret.as_ref().unwrap();
        assert!(!turret.has_grant());
        let (remount, definition) = turret.mount(engine.active.as_ref().unwrap()).unwrap();
        assert_eq!(definition.version, version + 1);
        assert!(
            remount
                .primary_for(engine.active.as_ref().unwrap())
                .is_none()
        );
        assert!(engine.active.as_ref().unwrap().ground_seat.is_none());
        let driver = &engine.peers["synthetic-reservation-0000"];
        assert!(driver.ground_seat.is_none());
        assert_eq!(
            engine.acquisition_round.as_ref().unwrap().points(),
            [initial_score[0], initial_score[1] - 600]
        );
        assert!(
            driver
                .pending_shared
                .iter()
                .skip(driver_queue.len())
                .any(|packet| {
                    let profile = engine.wire_profile;
                    if !replication::ObjectUpdate::decode_with_profile(packet, profile)
                        .is_ok_and(|update| update.header.object_type_mask == 1 << 1)
                    {
                        return false;
                    }
                    let mut reader = replication::BitReader::with_profile(packet, profile);
                    reader.position = profile.header_bits();
                    reader.read(1) == Ok(1)
                        && reader.read(16) == Ok(4)
                        && reader.read(2) == Ok(1)
                        && reader.read(16) == Ok(0x105)
                        && reader.read(4) == Ok(1)
                        && reader.read(8) == Ok(8)
                        && reader.read(1) == Ok(1)
                        && reader.read(16) == Ok(u32::from(u16::MAX))
                })
        );
    }
}

fn arrival_contact(
    engine: &mut Engine<Vec<u8>>,
    arrival: u16,
    now_ms: u64,
) -> Vec<OutboundMessage> {
    let request = protocol::ObjectiveInteraction {
        kind: protocol::ObjectiveInteractionKind::Activate,
        target_object_id: 0x0c80_0000 | u32::from(arrival),
        actor_index: engine.active.as_ref().unwrap().reservation.player,
        trigger_index: 0,
    };
    dispatch(engine, &request.encode(), now_ms).unwrap()
}

fn reply_kind(messages: &[OutboundMessage]) -> protocol::ObjectiveInteractionKind {
    let bytes = protocol::decode_hex(&messages.last().unwrap().payload_hex);
    protocol::ObjectiveInteraction::decode(&bytes).unwrap().kind
}

fn last_record(engine: &Engine<Vec<u8>>, op: &str) -> Value {
    String::from_utf8_lossy(&engine.writer)
        .lines()
        .rev()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|row| {
            row["op"] == op
                || row["effects"]
                    .as_array()
                    .is_some_and(|effects| effects.iter().any(|effect| effect["op"] == op))
        })
        .unwrap_or_else(|| panic!("no {op} record"))
}

fn delivery_rule(engine: &Engine<Vec<u8>>) -> Value {
    let row = last_record(engine, "acquisition_delivery");
    row["effects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|effect| effect["op"] == "acquisition_delivery")
        .unwrap()
        .clone()
}

fn vehicle_state(engine: &Engine<Vec<u8>>, index: u16) -> u8 {
    engine
        .ground_runtime
        .as_ref()
        .unwrap()
        .get(index)
        .unwrap()
        .state
}

#[test]
fn driver_arrival_contact_delivers_by_trigger_overlap_without_container_collision() {
    let mut engine = delivery_fixture_with_container(false);
    let a = FIRST_CAPTURE;
    let b = FIRST_CAPTURE + 2;
    position(&mut engine, 1000., 1040.);
    // Overlap alone, without the client contact, keeps the server-contact rule
    // that also needs the upward ray, and nothing is above this trigger.
    engine
        .tick_acquisition_objectives(Duration::from_millis(3300), 3)
        .unwrap();
    assert_eq!(vehicle_state(&engine, 4), 2);
    assert_eq!(
        engine.objective_world.as_ref().unwrap().state(a + 1),
        Some(ObjectiveState::Deactivated)
    );

    let reply = arrival_contact(&mut engine, a + 1, 3350);
    assert_eq!(reply_kind(&reply), protocol::ObjectiveInteractionKind::Ack);
    let request = last_record(&engine, "acquisition_arrival_request");
    assert_eq!(request["accepted"], true);
    assert_eq!(request["vehicle"], 0x0100_0004);
    engine
        .tick_acquisition_objectives(Duration::from_millis(3400), 4)
        .unwrap();
    assert_eq!(vehicle_state(&engine, 4), 8);
    let delivery = delivery_rule(&engine);
    assert_eq!(delivery["rule"], "trigger_overlap");
    assert_eq!(delivery["requested"], true);
    assert_eq!(delivery["driver"], 0);
    // The delivery counts against the defenders and opens the next stage.
    assert_eq!(
        engine.acquisition_round.as_ref().unwrap().points(),
        [1200, 400]
    );
    assert_eq!(
        engine.objective_world.as_ref().unwrap().state(a + 1),
        Some(ObjectiveState::Locked)
    );
    assert_eq!(
        engine.objective_world.as_ref().unwrap().state(b),
        Some(ObjectiveState::Deactivated)
    );
    assert_eq!(primary_assignment(&engine), b);
    assert!(engine.active.as_ref().unwrap().ground_seat.is_none());
    assert!(
        engine
            .acquisition_gameplay
            .as_ref()
            .unwrap()
            .arrival_requests
            .is_empty()
    );
}

#[test]
fn early_arrival_contact_waits_for_the_chassis_to_reach_the_trigger() {
    let mut engine = delivery_fixture_with_container(false);
    let arrival = FIRST_CAPTURE + 1;
    position(&mut engine, 1000., 1020.);
    let reply = arrival_contact(&mut engine, arrival, 3300);
    assert_eq!(reply_kind(&reply), protocol::ObjectiveInteractionKind::Ack);
    engine
        .tick_acquisition_objectives(Duration::from_millis(3350), 3)
        .unwrap();
    assert_eq!(vehicle_state(&engine, 4), 2);
    assert_eq!(
        engine.objective_world.as_ref().unwrap().state(arrival),
        Some(ObjectiveState::Deactivated)
    );
    position(&mut engine, 1000., 1040.);
    engine
        .tick_acquisition_objectives(Duration::from_millis(3400), 4)
        .unwrap();
    assert_eq!(vehicle_state(&engine, 4), 8);
    assert_eq!(delivery_rule(&engine)["rule"], "trigger_overlap");
}

#[test]
fn arrival_contact_keeps_the_retail_ray_when_the_container_has_collision() {
    let mut engine = delivery_fixture();
    let arrival = FIRST_CAPTURE + 1;
    // Inside the trigger but outside the roof witness: the retail ray misses.
    position(&mut engine, 1001.5, 1040.);
    let reply = arrival_contact(&mut engine, arrival, 3300);
    assert_eq!(reply_kind(&reply), protocol::ObjectiveInteractionKind::Ack);
    engine
        .tick_acquisition_objectives(Duration::from_millis(3350), 3)
        .unwrap();
    assert_eq!(vehicle_state(&engine, 4), 2);
    position(&mut engine, 1000., 1040.);
    engine
        .tick_acquisition_objectives(Duration::from_millis(3400), 4)
        .unwrap();
    assert_eq!(vehicle_state(&engine, 4), 8);
    assert_eq!(delivery_rule(&engine)["rule"], "retail_ray");
}

#[test]
fn arrival_contact_from_a_passenger_or_on_foot_actor_does_not_deliver() {
    let mut engine = delivery_fixture_with_container(false);
    let arrival = FIRST_CAPTURE + 1;
    position(&mut engine, 1000., 1040.);
    engine
        .active
        .as_mut()
        .unwrap()
        .ground_seat
        .as_mut()
        .unwrap()
        .seat = 2;
    let reply = arrival_contact(&mut engine, arrival, 3300);
    assert_eq!(reply_kind(&reply), protocol::ObjectiveInteractionKind::Nak);
    let request = last_record(&engine, "acquisition_arrival_request");
    assert_eq!(request["accepted"], false);
    assert_eq!(request["reason"], "acquisition_arrival_actor_not_driver");

    // On foot, the request takes the generic objective route, where an
    // Arrival has no interaction.
    let active = engine.active.as_mut().unwrap();
    active.ground_seat = None;
    let records = engine.writer.len();
    let reply = arrival_contact(&mut engine, arrival, 3350);
    assert_eq!(reply_kind(&reply), protocol::ObjectiveInteractionKind::Nak);
    assert!(
        !String::from_utf8_lossy(&engine.writer[records..]).contains("acquisition_arrival_request")
    );
    engine
        .tick_acquisition_objectives(Duration::from_millis(3400), 4)
        .unwrap();
    assert_eq!(vehicle_state(&engine, 4), 2);
    assert!(
        engine
            .acquisition_gameplay
            .as_ref()
            .unwrap()
            .arrival_requests
            .is_empty()
    );
}

#[test]
fn arrival_contact_for_another_chain_or_outside_the_trigger_is_refused_or_waits() {
    let mut engine = delivery_fixture_with_container(false);
    // The second chain's capture is not complete, so its Arrival is closed.
    let reply = arrival_contact(&mut engine, FIRST_CAPTURE + 3, 3300);
    assert_eq!(reply_kind(&reply), protocol::ObjectiveInteractionKind::Nak);
    assert_eq!(
        last_record(&engine, "acquisition_arrival_request")["reason"],
        "acquisition_arrival_unavailable"
    );
    // Far from the trigger, an admitted contact never delivers.
    position(&mut engine, 1000., 1000.);
    let reply = arrival_contact(&mut engine, FIRST_CAPTURE + 1, 3350);
    assert_eq!(reply_kind(&reply), protocol::ObjectiveInteractionKind::Ack);
    for (step, now) in [3400, 3500, 3600].into_iter().enumerate() {
        engine
            .tick_acquisition_objectives(Duration::from_millis(now), 4 + step as u64)
            .unwrap();
        assert_eq!(vehicle_state(&engine, 4), 2);
    }
    assert_eq!(
        engine
            .objective_world
            .as_ref()
            .unwrap()
            .state(FIRST_CAPTURE + 1),
        Some(ObjectiveState::Deactivated)
    );
}
