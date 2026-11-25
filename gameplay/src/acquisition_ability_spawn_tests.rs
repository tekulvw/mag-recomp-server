//! Acquisition with the squad-ability runtime installed: idle ability state
//! must neither refresh every squad order nor starve a queued respawn.
use super::*;

/// An invented Acquisition match whose squad-ability runtime is installed
/// before the objective runtime, as the live worker composes it.
fn ability_fixture() -> Engine<Vec<u8>> {
    let (engine, setup) = acquisition_setup_fixture(None, None, false, 4);
    let mut abilities = crate::active_ability_engine_tests::setup();
    abilities.map_name = engine.map_name().to_owned();
    let (passives, policy) = crate::active_ability_engine_tests::native_leadership();
    engine
        .with_leadership_passives(passives, policy)
        .unwrap()
        .with_active_abilities(abilities)
        .unwrap()
        .with_acquisition_gameplay(setup)
        .unwrap()
}

fn events(engine: &Engine<Vec<u8>>) -> Vec<Value> {
    std::str::from_utf8(&engine.writer)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn objective_ticks(engine: &Engine<Vec<u8>>) -> usize {
    events(engine)
        .iter()
        .filter(|event| event["op"] == "acquisition_objective_tick")
        .count()
}

fn is_objective_clear(engine: &Engine<Vec<u8>>, packet: &[u8]) -> bool {
    let Ok(update) = ObjectUpdate::decode_with_profile(packet, engine.wire_profile) else {
        return false;
    };
    let clear = replication::SquadObjectiveAssignment {
        sequence: update.header.sequence,
        timestamp_ms: update.header.timestamp_ms,
        squad_index: engine.active.as_ref().unwrap().squad_catalog.squad_index(),
        objective_index: u16::MAX,
    }
    .encode_with_profile(engine.wire_profile)
    .unwrap();
    packet == clear.as_slice()
}

#[test]
fn acquisition_idle_ability_ticks_do_not_refresh_assignments_or_clear_orders() {
    let mut engine = ability_fixture();
    prepare_capture_actor(&mut engine, 0, 100);
    // The startup tick changed objectives; the client takes that refresh.
    for at in 101..=110 {
        let actor = engine.active.as_ref().unwrap();
        if actor.objective_assigned && !actor.objective_needs_clear {
            break;
        }
        owner_update(&mut engine, at, 0);
    }
    let actor = engine.active.as_ref().unwrap();
    assert!(actor.objective_assigned && !actor.objective_needs_clear);
    let committed = objective_ticks(&engine);
    for step in 1..=20_u64 {
        let now = 100 + step * 50;
        engine
            .tick_acquisition_objectives(Duration::from_millis(now), step + 1)
            .unwrap();
        let actor = engine.active.as_ref().unwrap();
        assert!(actor.objective_assigned, "idle tick {step} re-armed orders");
        assert!(
            !actor.objective_needs_clear,
            "idle tick {step} re-armed clear"
        );
        let replies = owner_update(&mut engine, now + 1, 0);
        for reply in replies {
            let packet = protocol::decode_hex(&reply.payload_hex);
            assert!(
                !is_objective_clear(&engine, &packet),
                "idle tick {step} answered an owner update with an order clear"
            );
        }
    }
    // Vehicle-wave rollovers still commit; an unchanged ability state never
    // rides along as if it were a gameplay result.
    let ticks: Vec<_> = events(&engine)
        .into_iter()
        .filter(|event| event["op"] == "acquisition_objective_tick")
        .skip(committed)
        .collect();
    assert!(ticks.iter().all(|tick| {
        tick["effects"]
            .as_array()
            .unwrap()
            .iter()
            .all(|effect| effect["op"] != "active_ability_update")
    }));
}

#[test]
fn acquisition_confirmed_respawn_spawns_while_ability_runtime_ticks() {
    let mut engine = ability_fixture();
    prepare_capture_actor(&mut engine, 0, 100);
    let actor = engine.active.as_mut().unwrap();
    actor.claimed = Some(Claimed {
        wire: Vec::new(),
        np_identity: [0; 36],
        game_info: protocol::GameInfo {
            client_index: 0,
            level_name: "synthetic_arena".into(),
            start_time_ms: 0,
            attacking_faction: 1,
            defending_faction: 0,
            maximum_points: [1200, 1000],
            current_points: [1200, 1000],
            game_type: 1,
        },
    });
    actor.life = CharacterLife::AwaitingRespawn { generation: 1 };
    // A fresh order refresh is pending, as after any real objective change.
    actor.objective_assigned = false;
    actor.objective_needs_clear = true;
    queue_respawn(&mut engine, 200);
    let deadline = engine
        .active
        .as_ref()
        .unwrap()
        .pending_spawn_deadline
        .expect("confirmed respawn queues a wave deadline");
    let deadline_ms = deadline.as_millis() as u64;
    let period = u64::from(engine.match_profile().spawn_wave_ms[0]);
    let mut now = 200;
    let mut first_due_update = None;
    let mut spawned_at = None;
    // Interleave 50 ms objective ticks with client object updates, the live
    // cadence, for at most one full wave past the deadline.
    while now <= deadline_ms + period {
        now += 50;
        engine
            .tick_acquisition_objectives(Duration::from_millis(now), now)
            .unwrap();
        let update = now + 1;
        if update >= deadline_ms {
            first_due_update.get_or_insert(update);
            // An objective change re-armed every order just before the wave.
            let actor = engine.active.as_mut().unwrap();
            actor.objective_assigned = false;
            actor.objective_needs_clear = true;
        }
        owner_update(&mut engine, update, 0);
        if engine.active.as_ref().unwrap().life.is_alive() {
            spawned_at = Some(update);
            break;
        }
    }
    let spawned_at = spawned_at.expect("a due queued respawn was never consumed");
    assert_eq!(
        Some(spawned_at),
        first_due_update,
        "the due respawn waited behind a squad-order refresh"
    );
    let actor = engine.active.as_ref().unwrap();
    assert!(actor.pending_spawn_deadline.is_none());
    assert!(actor.spawn_delta.is_some());
    // The new life re-arms its own order clear and assignment.
    assert!(actor.objective_needs_clear);
    assert!(!actor.objective_assigned);
    // With the wave consumed, the team deadline rolls over for this player.
    let reservation = actor.reservation.id.clone();
    engine
        .queue_team_wave_deadlines(Duration::from_millis(spawned_at + 1), spawned_at + 1)
        .unwrap();
    let published = events(&engine).into_iter().rev().find(|event| {
        event["op"] == "team_wave_deadlines_published"
            && event["recipients"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["reservation_id"] == reservation.as_str())
    });
    let published = published.expect("the team wave deadline stopped publishing");
    assert!(published["recipients"][0]["deadline_ms"].as_u64().unwrap() > spawned_at);
}

/// Horizon numbers DME worlds from 0, so the first live Acquisition match runs
/// in world 0. Every world tick must keep succeeding there while a human enters
/// and spawns: one failed tick ends the world stream, Horizon then releases
/// the player with `world_stream_gap`, and the client is left at the map origin
/// below the terrain.
#[test]
fn acquisition_human_spawns_in_horizon_world_zero_without_failing_a_world_tick() {
    let mut engine = ability_fixture();
    reserve(&mut engine, 0);
    let actor = engine.active.as_mut().unwrap();
    actor.reservation.world = 0;
    ready(actor);
    actor.claimed = Some(Claimed {
        wire: Vec::new(),
        np_identity: [0; 36],
        game_info: protocol::GameInfo {
            client_index: 0,
            level_name: "synthetic_arena".into(),
            start_time_ms: 0,
            attacking_faction: 1,
            defending_faction: 0,
            maximum_points: [1200, 1000],
            current_points: [1200, 1000],
            game_type: 1,
        },
    });
    actor.life = CharacterLife::AwaitingRespawn { generation: 1 };
    engine.match_started = true;
    engine.match_epoch = Some(Duration::from_millis(100));
    engine.enable_world_ticks();
    let tick = |engine: &mut Engine<Vec<u8>>, now: u64| {
        let response = engine.tick_world(Duration::from_millis(now), now, "synthetic-world".into());
        assert!(
            response.ok,
            "world tick at {now} ms failed in world 0: {:?}",
            response.error
        );
        assert!(response.messages.iter().all(|m| m.world_id == 0));
    };
    tick(&mut engine, 100);
    dispatch_in_world_zero(&mut engine, &0x0211u16.to_be_bytes(), 200);
    let deadline = engine
        .active
        .as_ref()
        .unwrap()
        .pending_spawn_deadline
        .expect("the confirmed spawn queues a wave deadline")
        .as_millis() as u64;
    let period = u64::from(engine.match_profile().spawn_wave_ms[0]);
    let mut now = 200;
    while now <= deadline + period && !engine.active.as_ref().unwrap().life.is_alive() {
        now += 50;
        tick(&mut engine, now);
        let header = UpdateHeader {
            sequence: engine.active.as_ref().unwrap().latest_owner_sequence + 1,
            nak_sequence: 0,
            client_index: 0,
            object_type_mask: 0,
            timestamp_ms: (now + 1) as u32,
        };
        let update = ObjectUpdate { header, wire: &[] }
            .encode_empty_with_profile(engine.wire_profile)
            .unwrap();
        dispatch_in_world_zero(&mut engine, &update, now + 1);
    }
    let actor = engine.active.as_ref().unwrap();
    assert!(actor.life.is_alive(), "the queued human spawn never landed");
    // The spawn lands at the authored ground position of the chosen point.
    let point = actor
        .selected_spawn
        .map_or(actor.squad_spawn, |s| s.spawn_point_index);
    let authored = actor
        .squad_catalog
        .as_ref()
        .alpha(point)
        .expect("the spawn point is catalogued")
        .position;
    assert_eq!(actor.tracking.sample.unwrap().position, authored);
    // The spawned player captures the first objective. Its completion runs the
    // authored damage volume against the dynamic collision world inside the
    // world tick, which must resolve in world 0 too.
    let position = actor.tracking.sample.unwrap().position;
    let CharacterLife::Alive { generation, .. } = actor.life else {
        unreachable!()
    };
    now += 50;
    let outcome = engine.objective_world.as_mut().unwrap().interact(
        protocol::ObjectiveInteraction {
            kind: protocol::ObjectiveInteractionKind::Activate,
            target_object_id: 0x0c80_0000 | u32::from(FIRST_CAPTURE),
            actor_index: 0,
            trigger_index: 0,
        },
        ObjectiveActor {
            player: 0,
            team: 0,
            generation,
            position,
        },
        Duration::from_millis(now),
    );
    assert!(outcome.accepted, "{}", outcome.reason);
    let finish = now + 3200;
    while now < finish
        && engine
            .objective_world
            .as_ref()
            .unwrap()
            .state(FIRST_CAPTURE)
            != Some(ObjectiveState::Completed)
    {
        now += 50;
        tick(&mut engine, now);
    }
    assert_eq!(
        engine
            .objective_world
            .as_ref()
            .unwrap()
            .state(FIRST_CAPTURE),
        Some(ObjectiveState::Completed)
    );
    for _ in 0..4 {
        now += 50;
        tick(&mut engine, now);
    }
}

/// Deliver a client event the way Horizon does for a world-0 reservation.
fn dispatch_in_world_zero(engine: &mut Engine<Vec<u8>>, bytes: &[u8], now_ms: u64) {
    let actor = engine.active.as_ref().unwrap();
    let input = events::EventInput {
        reservation_id: actor.reservation.id.clone(),
        world_id: 0,
        player_id: actor.reservation.player,
        transport: Transport::Udp,
        direction: Direction::ClientToServer,
        rt_type: 0x3d,
        plugin_id: Some(1),
        payload_hex: protocol::encode_hex(bytes),
    };
    let decoded = DecodedEvent::with_profile(
        bytes,
        actor.reservation.player,
        engine.wire_profile,
        engine.ground_driver_enabled(),
    );
    engine
        .handle_event(input, &decoded, Duration::from_millis(now_ms), 1)
        .unwrap();
}
