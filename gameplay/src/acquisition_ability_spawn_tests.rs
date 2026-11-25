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

/// The shared command stage under Acquisition's own world update: a cluster
/// over friendly bots (one dies), the invoker, an enemy and bare ground keeps
/// every world update, and a refused bomblet ends only its own bombardment.
#[test]
fn acquisition_cluster_bombardment_keeps_every_world_update() {
    use crate::{
        active_abilities::Ability,
        command_ability_world_tests::{
            LEADER, add_bots, arm_world, bombard, place_human, staged_rows,
        },
    };
    for qualified in [true, false] {
        let (mut engine, mut setup) = acquisition_setup_fixture(None, None, false, 4);
        let mut abilities = crate::active_ability_engine_tests::setup();
        abilities.map_name = engine.map_name().to_owned();
        let (passives, policy) = crate::active_ability_engine_tests::native_leadership();
        // The invented arena's strike and cover rows admit its layer-9 floor.
        setup.filter_rows[19] |= 1 | (1 << 9);
        setup.filter_rows[24] |= 1 | (1 << 9);
        engine = engine
            .with_leadership_passives(passives, policy)
            .unwrap()
            .with_active_abilities(abilities)
            .unwrap()
            .with_acquisition_gameplay(setup)
            .unwrap();
        prepare_capture_actor(&mut engine, LEADER, 100);
        arm_world(&mut engine, &mut [0; 32]);
        // An unqualified floor refuses every impact query.
        engine.collision_world = Some(crate::active_ability_engine_tests::strike_floor(
            engine.map_name(),
            qualified,
        ));
        place_human(&mut engine, LEADER, [100., 1., 100.]);
        let team = engine.active.as_ref().unwrap().team();
        let friendly = u16::from(team) * 16 + 1;
        let enemy = u16::from(1 - team) * 16 + 1;
        let bots = add_bots(
            &mut engine,
            &[
                (friendly, [101., 1., 101.], 8),
                (enemy, [102., 1., 98.], 400),
            ],
        );
        engine.enable_world_ticks();
        let ended = bombard(&mut engine, Ability::ClusterBomb, [100., 1., 100.], 1_000);
        for now in (ended + 50..=ended + 300).step_by(50) {
            let reply = engine.tick_world(Duration::from_millis(now), now / 1000, "after".into());
            assert!(reply.ok, "{:?}", reply.error);
        }
        let rows = staged_rows(&engine);
        let failures: Vec<_> = rows
            .iter()
            .filter(|row| row["op"] == "active_ability_effect_failed")
            .collect();
        let attacker = 0x0380_0000 | u32::from(LEADER);
        assert!(
            rows.iter()
                .filter(|row| row["on_character_hit"] == true)
                .all(|row| row["attacker"] == attacker)
        );
        if qualified {
            assert!(failures.is_empty(), "{failures:?}");
            let dead = &engine.bot_registry.bots[&bots[0]];
            assert_eq!(dead.health, 0);
            assert_eq!(dead.damage.last_attacker, attacker);
            assert!(engine.bot_registry.bots[&bots[1]].health < 400);
        } else {
            assert_eq!(failures.len(), 1, "{failures:?}");
            assert_eq!(failures[0]["error"], "air_strike_static_impact_ray_failed");
            assert!(rows.iter().all(|row| row["on_character_hit"].is_null()));
            assert_eq!(engine.bot_registry.bots[&bots[1]].health, 400);
        }
    }
}
