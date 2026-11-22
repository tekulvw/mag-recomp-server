//! Synthetic Domination native owner bootstrap and reliable replay coverage.
use super::*;

fn domination_control_catalog(count: usize) -> ObjectiveCatalog {
    let mut raw = objective_json();
    raw.as_object_mut().unwrap().remove("progression");
    raw["game_type"] = json!(2);
    raw["objects"] = json!(
        (0..count)
            .map(|index| json!({
                "token": format!("0x{:08x}", 0xa000 + index),
                "object_index": index,
                "initial_state": u8::from(index == 0),
            }))
            .collect::<Vec<_>>()
    );
    raw["squads"] = json!(
        (0..32)
            .map(|squad| json!({
                "team": squad / 16,
                "squad_index": squad,
                "initial_objective_index": 0,
                "authored_objectives": [{"object_index": 0, "order": 1}],
            }))
            .collect::<Vec<_>>()
    );
    ObjectiveCatalog::from_json(&raw.to_string()).unwrap()
}

fn domination_control_bootstrap_fixture() -> Engine<Vec<u8>> {
    let mut engine = engine();
    reserve(&mut engine, 0);
    let mut spawns_json = spawn_json();
    spawns_json["match_profile"]["game_type"] = json!(2);
    engine.spawn_catalog = Some(spawns(spawns_json).unwrap());
    let catalog = domination_control_catalog(106);
    let mut world = objective_control::ObjectiveWorld::new(&catalog);
    for mut row in world.snapshot(2, 500).objectives {
        row.state_actor.change_time_ms = 800 + u32::from(row.object_index);
        row.expiration_time_ms = 1_000 + u32::from(row.object_index);
        row.deactivation_time_ms = 2_000 + u32::from(row.object_index);
        row.health_percent = row.object_index.rem_euclid(101) as u8;
        world.install_acquisition_delta(row).unwrap();
    }
    engine.objective_catalog = Some(catalog);
    engine.objective_world = Some(world);
    let active = engine.active.as_mut().unwrap();
    ready(active);
    active.objective_delta = None;
    active.objective_assigned = false;
    engine
}

fn pull_domination_control_bootstrap(
    engine: &mut Engine<Vec<u8>>,
    nak_sequence: u32,
) -> Result<Vec<OutboundMessage>, &'static str> {
    let update = ObjectUpdate {
        header: UpdateHeader {
            sequence: engine.active.as_ref().unwrap().latest_owner_sequence + 1,
            nak_sequence,
            client_index: 0,
            object_type_mask: 0,
            timestamp_ms: 501,
        },
        wire: &[],
    };
    let wire = update
        .encode_empty_with_profile(engine.wire_profile)
        .unwrap();
    dispatch(engine, &wire, 501)
}

#[test]
fn native_domination_control_bootstrap_sends_106_current_rows_in_ordered_reliable_batches() {
    let mut engine = domination_control_bootstrap_fixture();
    let expected = engine.objective_world.as_ref().unwrap().snapshot(2, 501);
    let replies = pull_domination_control_bootstrap(&mut engine, 0).unwrap();
    assert_eq!(replies.len(), 2);
    let mut rows = expected.objectives.iter();
    for (offset, message) in replies.iter().enumerate() {
        let wire = protocol::decode_hex(&message.payload_hex);
        assert!(wire.len() <= 1397);
        let profile = engine.wire_profile;
        let header = ObjectUpdate::decode_with_profile(&wire, profile)
            .unwrap()
            .header;
        assert_eq!(header.object_type_mask, 1 << 12);
        assert_eq!(header.sequence, 2 + offset as u32);
        assert_eq!(header.timestamp_ms, 501);
        let mut bits = BitReader::with_profile(&wire, profile);
        bits.position = profile.header_bits();
        while bits.read(1).unwrap() == 1 {
            let row = rows.next().unwrap();
            assert_eq!(bits.read(16), Ok(u32::from(row.object_index)));
            for field in 0..11 {
                assert_eq!(bits.read(1), Ok(u32::from(matches!(field, 2 | 3 | 4 | 6))));
                match field {
                    2 => {
                        assert_eq!(bits.read(8), Ok(row.state_actor.state as u32));
                        assert_eq!(bits.read(32), Ok(row.state_actor.change_time_ms));
                        assert_eq!(bits.read(16), Ok(u32::from(row.state_actor.actor_index)));
                    }
                    3 => assert_eq!(bits.read(32), Ok(row.expiration_time_ms)),
                    4 => assert_eq!(bits.read(32), Ok(row.deactivation_time_ms)),
                    6 => assert_eq!(bits.read(8), Ok(u32::from(row.health_percent))),
                    _ => {}
                }
            }
        }
    }
    assert_eq!(rows.len(), 0);
    let active = engine.active.as_ref().unwrap();
    assert!(active.objective_delta.is_some());
    assert_eq!(active.outbound_sequence, 3);
    assert_eq!(active.replication_history.len(), 2);
    for (index, reply) in replies.iter().enumerate() {
        let replay = pull_domination_control_bootstrap(&mut engine, 2 + index as u32).unwrap();
        assert_eq!(replay.len(), 1);
        assert_eq!(replay[0].payload_hex, reply.payload_hex);
    }
}

#[test]
fn native_domination_control_bootstrap_rejects_malformed_tail_and_record_refusal_atomically() {
    for bad_tail in [true, false] {
        let mut engine = domination_control_bootstrap_fixture();
        let error = if bad_tail {
            let world = engine.objective_world.as_mut().unwrap();
            let mut last = *world.snapshot(2, 501).objectives.last().unwrap();
            last.health_percent = 101;
            world.install_acquisition_delta(last).unwrap();
            "invalid_objective_control_delta"
        } else {
            engine.max_log = engine.written;
            "log_limit_reached"
        };
        assert_eq!(
            pull_domination_control_bootstrap(&mut engine, 0).err(),
            Some(error)
        );
        let active = engine.active.as_ref().unwrap();
        assert!(active.objective_delta.is_none());
        assert_eq!(active.outbound_sequence, 1);
        assert_eq!(active.latest_owner_sequence, 0);
        assert!(active.replication_history.is_empty());
        assert!(active.pending_shared.is_empty());
    }
}

#[test]
fn native_domination_control_bootstrap_preserves_single_packet_bytes() {
    let mut engine = domination_control_bootstrap_fixture();
    let catalog = domination_control_catalog(6);
    let mut world = objective_control::ObjectiveWorld::new(&catalog);
    let mut row = world.snapshot(2, 501).objectives[0];
    row.expiration_time_ms = 1234;
    world.install_acquisition_delta(row).unwrap();
    let expected = world
        .snapshot(2, 501)
        .encode_with_profile(engine.wire_profile)
        .unwrap();
    engine.objective_catalog = Some(catalog);
    engine.objective_world = Some(world);
    let replies = pull_domination_control_bootstrap(&mut engine, 0).unwrap();
    assert_eq!(replies.len(), 1);
    assert_eq!(protocol::decode_hex(&replies[0].payload_hex), expected);
}

#[test]
fn native_domination_control_bootstrap_waits_for_full_replay_capacity_in_bounded_mode() {
    let mut engine = domination_control_bootstrap_fixture();
    engine.maximum_players = 1;
    let actor = engine.active.as_mut().unwrap();
    actor.replication_history = vec![Vec::new(); MAX_REPLICATION_HISTORY - 1];
    assert!(
        pull_domination_control_bootstrap(&mut engine, 0)
            .unwrap()
            .is_empty()
    );
    let actor = engine.active.as_mut().unwrap();
    assert!(actor.objective_delta.is_none());
    assert_eq!(actor.outbound_sequence, 1);
    assert_eq!(actor.replication_history.len(), MAX_REPLICATION_HISTORY - 1);
    actor.replication_history.clear();
    assert_eq!(
        pull_domination_control_bootstrap(&mut engine, 0)
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn native_domination_control_bootstrap_stays_private_and_assignment_follows_both_packets() {
    let mut engine = domination_control_bootstrap_fixture();
    engine.maximum_players = 2;
    engine.enable_world_ticks();
    reserve(&mut engine, 1);
    for actor in actors(&mut engine) {
        ready(actor);
        actor.pending_shared.clear();
        if actor.reservation.player == 1 {
            actor.objective_delta = None;
            actor.objective_assigned = false;
        }
    }
    let profile = engine.wire_profile;
    let mut update = |sequence| {
        let wire = ObjectUpdate {
            header: UpdateHeader {
                sequence,
                nak_sequence: 0,
                client_index: 0,
                object_type_mask: 0,
                timestamp_ms: 500 + sequence,
            },
            wire: &[],
        }
        .encode_empty_with_profile(profile)
        .unwrap();
        engine.handle(
            Request::Event {
                v: BRIDGE_VERSION,
                request_id: format!("synthetic-late-objectives-{sequence}"),
                reservation_id: "synthetic-reservation-0001".into(),
                world_id: 1,
                player_id: 1,
                transport: Transport::Udp,
                direction: Direction::ClientToServer,
                rt_type: 0x3d,
                plugin_id: Some(1),
                payload_hex: protocol::encode_hex(&wire),
            },
            Duration::from_millis(u64::from(500 + sequence)),
            1,
        )
    };
    let response = update(1);
    assert!(response.ok, "{:?}", response.error);
    assert_eq!(response.messages.len(), 2);
    for (index, message) in response.messages.iter().enumerate() {
        assert_eq!(message.player_id, 1);
        let wire = protocol::decode_hex(&message.payload_hex);
        let header = ObjectUpdate::decode_with_profile(&wire, profile)
            .unwrap()
            .header;
        assert_eq!(header.sequence, 2 + index as u32);
        assert_eq!(header.object_type_mask, 1 << 12);
    }
    let response = update(2);
    assert!(response.ok, "{:?}", response.error);
    let owner_assignment = response
        .messages
        .iter()
        .find(|message| message.player_id == 1)
        .unwrap();
    let wire = protocol::decode_hex(&owner_assignment.payload_hex);
    let header = ObjectUpdate::decode_with_profile(&wire, profile)
        .unwrap()
        .header;
    assert_eq!(header.sequence, 4);
    assert_eq!(header.object_type_mask, 1 << 8);
}

#[test]
fn native_domination_control_bootstrap_validates_the_entire_sequence_range() {
    let mut engine = domination_control_bootstrap_fixture();
    engine.active.as_mut().unwrap().outbound_sequence = u32::MAX - 1;
    assert_eq!(
        pull_domination_control_bootstrap(&mut engine, 0).err(),
        Some("replication_sequence_exhausted")
    );
    let active = engine.active.as_ref().unwrap();
    assert_eq!(active.outbound_sequence, u32::MAX - 1);
    assert!(active.objective_delta.is_none());
    assert!(active.replication_history.is_empty());
}
