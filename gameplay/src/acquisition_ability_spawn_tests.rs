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
