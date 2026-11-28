//! SYNTHETIC respawn-queue scenarios on the progression test fixture. A dead
//! character rejoins the next team wave at its stored selection on its own;
//! no captured map data or game files are involved.
use super::*;
use crate::replication::{ObjectUpdate, UpdateHeader};
use crate::spawn_progression::tests::{dispatch, engine, ready, reserve};

// Fixture: points 0 (x 100) and 1 (x 120) are on-foot points for every squad,
// with point 0 as the squad default; team waves are 100 ms apart.
const POINT_0: [f32; 3] = [100., 3., 100.];
const POINT_1: [f32; 3] = [120., 3., 100.];

fn operator() -> Engine<Vec<u8>> {
    let mut engine = engine();
    reserve(&mut engine, 0);
    let actor = engine.active.as_mut().unwrap();
    ready(actor);
    actor.life = CharacterLife::AwaitingFirstSpawn;
    engine
}

fn actor(engine: &Engine<Vec<u8>>) -> &Active {
    engine.active.as_ref().unwrap()
}

fn owner_update(engine: &mut Engine<Vec<u8>>, now_ms: u64) {
    let header = UpdateHeader {
        sequence: actor(engine).latest_owner_sequence + 1,
        nak_sequence: 0,
        client_index: 0,
        object_type_mask: 0,
        timestamp_ms: now_ms as u32,
    };
    let bytes = ObjectUpdate { header, wire: &[] }
        .encode_empty_with_profile(engine.wire_profile)
        .unwrap();
    dispatch(engine, &bytes, now_ms).unwrap();
}

fn select(engine: &mut Engine<Vec<u8>>, point: u16, now_ms: u64) {
    let mut bytes = 0x0219u16.to_be_bytes().to_vec();
    bytes.extend(point.to_be_bytes());
    dispatch(engine, &bytes, now_ms).unwrap();
}

fn confirm(engine: &mut Engine<Vec<u8>>, now_ms: u64) {
    dispatch(engine, &0x0211u16.to_be_bytes(), now_ms).unwrap();
}

fn cancel(engine: &mut Engine<Vec<u8>>, now_ms: u64) {
    dispatch(engine, &0x0213u16.to_be_bytes(), now_ms).unwrap();
}

/// The life state a death leaves behind: the queue entry is gone.
fn die(engine: &mut Engine<Vec<u8>>) {
    let actor = engine.active.as_mut().unwrap();
    let CharacterLife::Alive { generation, .. } = actor.life else {
        panic!("only a living character dies: {:?}", actor.life);
    };
    actor.life = CharacterLife::AwaitingRespawn { generation };
    actor.pending_spawn_deadline = None;
}

/// The dead character's first owner update once its queued wave is due.
fn update_when_due(engine: &mut Engine<Vec<u8>>) {
    let due = actor(engine)
        .pending_spawn_deadline
        .expect("the dead character is queued");
    owner_update(engine, due.as_millis() as u64 + 10);
}

fn assert_alive_at(engine: &Engine<Vec<u8>>, generation: u32, position: [f32; 3]) {
    let actor = actor(engine);
    assert!(
        matches!(actor.life, CharacterLife::Alive { generation: g, .. } if g == generation),
        "{:?}",
        actor.life
    );
    let stored = actor.tracking.sample.unwrap().position;
    assert_eq!([stored.x, stored.y, stored.z], position);
}

/// First life: the client selects point 1 and confirms; the wave spawns it.
fn first_life_at_point_1(engine: &mut Engine<Vec<u8>>) {
    select(engine, 1, 50);
    confirm(engine, 50);
    owner_update(engine, 150);
    assert_alive_at(engine, 1, POINT_1);
}

#[test]
fn the_first_life_still_waits_for_the_clients_confirmation() {
    let mut engine = operator();
    select(&mut engine, 1, 50);
    for now in [150, 250, 350] {
        owner_update(&mut engine, now);
    }
    assert!(matches!(
        actor(&engine).life,
        CharacterLife::AwaitingFirstSpawn
    ));
    assert!(actor(&engine).pending_spawn_deadline.is_none());
}

#[test]
fn a_death_rejoins_the_next_wave_at_the_previous_selection_without_a_new_request() {
    let mut engine = operator();
    first_life_at_point_1(&mut engine);
    for (life, died_ms) in [(2, 220), (3, 420)] {
        die(&mut engine);
        // No selection and no confirmation after the death.
        owner_update(&mut engine, died_ms);
        let deadline = actor(&engine)
            .pending_spawn_deadline
            .expect("a dead character rejoins the queue on its own");
        assert!(deadline > Duration::from_millis(died_ms));
        assert!(!actor(&engine).life.is_alive());
        owner_update(&mut engine, deadline.as_millis() as u64);
        assert_alive_at(&engine, life, POINT_1);
        assert_eq!(actor(&engine).selected_spawn.unwrap().spawn_point_index, 1);
    }
}

#[test]
fn a_selection_made_while_alive_applies_to_the_next_life() {
    let mut engine = engine_with_first_life();
    select(&mut engine, 0, 200);
    assert_eq!(actor(&engine).selected_spawn.unwrap().spawn_point_index, 0);
    assert!(actor(&engine).life.is_alive());
    die(&mut engine);
    owner_update(&mut engine, 220);
    update_when_due(&mut engine);
    assert_alive_at(&engine, 2, POINT_0);
}

fn engine_with_first_life() -> Engine<Vec<u8>> {
    let mut engine = operator();
    first_life_at_point_1(&mut engine);
    engine
}

#[test]
fn leaving_the_queue_holds_the_dead_player_until_it_confirms_again() {
    let mut engine = engine_with_first_life();
    die(&mut engine);
    owner_update(&mut engine, 220);
    assert!(actor(&engine).pending_spawn_deadline.is_some());
    cancel(&mut engine, 230);
    assert!(actor(&engine).pending_spawn_deadline.is_none());
    for now in [300, 400, 500] {
        owner_update(&mut engine, now);
    }
    assert!(!actor(&engine).life.is_alive());
    assert!(actor(&engine).pending_spawn_deadline.is_none());
    select(&mut engine, 0, 510);
    confirm(&mut engine, 510);
    owner_update(&mut engine, 600);
    assert_alive_at(&engine, 2, POINT_0);
    // The hold belonged to that death only.
    die(&mut engine);
    owner_update(&mut engine, 620);
    update_when_due(&mut engine);
    assert_alive_at(&engine, 3, POINT_0);
}

#[test]
fn a_squad_change_resets_the_stored_selection_to_the_new_squads_spawn() {
    let mut engine = engine_with_first_life();
    die(&mut engine);
    engine.active.as_mut().unwrap().claimed = Some(Claimed {
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
            game_type: 0,
        },
    });
    dispatch(&mut engine, &[0x02, 0x02, 1], 210).unwrap();
    assert_eq!(actor(&engine).squad_catalog.squad_index(), 1);
    assert!(actor(&engine).selected_spawn.is_none());
    owner_update(&mut engine, 220);
    update_when_due(&mut engine);
    assert_alive_at(&engine, 2, POINT_0);
}

#[test]
fn an_unusable_stored_selection_falls_back_to_the_squads_point_and_is_dropped() {
    let mut engine = engine_with_first_life();
    std::sync::Arc::make_mut(&mut engine.active.as_mut().unwrap().squad_catalog)
        .set_runtime_point_active(1, 101, false)
        .unwrap();
    die(&mut engine);
    owner_update(&mut engine, 220);
    update_when_due(&mut engine);
    assert_alive_at(&engine, 2, POINT_0);
    assert!(actor(&engine).selected_spawn.is_none());
}

#[test]
fn no_automatic_queue_once_the_match_has_ended() {
    let mut engine = engine_with_first_life();
    die(&mut engine);
    engine.match_terminal = Some(match_flow::MatchTerminal {
        world_id: 1,
        match_id: "synthetic-ending".into(),
        reason: "synthetic".into(),
        reservation_ids: vec![actor(&engine).reservation.id.clone()],
    });
    owner_update(&mut engine, 220);
    assert!(actor(&engine).pending_spawn_deadline.is_none());
}

fn world_tick(engine: &mut Engine<Vec<u8>>, now_ms: u64) {
    let response = engine.tick_world(Duration::from_millis(now_ms), 1, "synthetic-tick".into());
    assert!(response.ok, "{:?}", response.error);
}

/// The wave a character that dies at `now_ms` rejoins by itself.
fn wave_after_death(engine: &Engine<Vec<u8>>, now_ms: u64) -> Duration {
    engine.team_waves[usize::from(actor(engine).team())]
        .expect("the first life started the team clock")
        .wave_after_death(Duration::from_millis(now_ms))
        .unwrap()
}

#[test]
fn a_death_spawns_on_its_wave_without_waiting_for_an_owner_update() {
    let mut engine = engine_with_first_life();
    engine.enable_world_ticks();
    die(&mut engine);
    let died_ms = 210;
    let wave = wave_after_death(&engine, died_ms + 10);
    // The world tick that first sees the death queues it for that wave.
    world_tick(&mut engine, died_ms + 10);
    assert_eq!(actor(&engine).pending_spawn_deadline, Some(wave));
    // A dead client reports about once a second; its first update after the
    // wave starts the new life instead of queueing for the following one.
    owner_update(&mut engine, wave.as_millis() as u64 + 30);
    assert_alive_at(&engine, 2, POINT_1);
}

#[test]
fn the_world_tick_leaves_a_cancelled_or_first_life_character_out_of_the_queue() {
    let mut engine = operator();
    engine.enable_world_ticks();
    // Before the first confirmation the world clock queues nothing.
    world_tick(&mut engine, 40);
    assert!(actor(&engine).pending_spawn_deadline.is_none());
    first_life_at_point_1(&mut engine);
    die(&mut engine);
    cancel(&mut engine, 205);
    for now in [210, 260, 310, 360] {
        world_tick(&mut engine, now);
    }
    assert!(actor(&engine).pending_spawn_deadline.is_none());
    assert!(!actor(&engine).life.is_alive());
}

#[test]
fn the_world_tick_does_not_queue_once_the_match_has_ended() {
    let mut engine = engine_with_first_life();
    engine.enable_world_ticks();
    die(&mut engine);
    engine.match_terminal = Some(match_flow::MatchTerminal {
        world_id: 1,
        match_id: "synthetic-ending".into(),
        reason: "synthetic".into(),
        reservation_ids: vec![actor(&engine).reservation.id.clone()],
    });
    let _ = engine.tick_world(Duration::from_millis(220), 1, "synthetic-tick".into());
    assert!(actor(&engine).pending_spawn_deadline.is_none());
}

// Respawn queue cutoff: a dead character that rejoins by itself never takes a
// wave within `RESPAWN_QUEUE_CUTOFF` (3.5 s) of joining, because its client is
// still playing the death; a confirmation from its open respawn screen does.

/// A first life at point 1, world ticks on, then team waves at 1 s, 21 s, ...
fn alive_with_twenty_second_waves() -> Engine<Vec<u8>> {
    let mut engine = engine_with_first_life();
    engine.enable_world_ticks();
    let team = usize::from(actor(&engine).team());
    engine.team_waves[team] = Some(TeamWaveClock {
        first_deadline: Duration::from_millis(1000),
        period: Duration::from_secs(20),
    });
    engine
}

fn kill_self(engine: &mut Engine<Vec<u8>>, now_ms: u64) {
    let player = actor(engine).reservation.player;
    dispatch(
        engine,
        &crate::admin_kill::kill_self_payload(player),
        now_ms,
    )
    .unwrap();
    assert!(
        matches!(actor(engine).life, CharacterLife::AwaitingRespawn { .. }),
        "{:?}",
        actor(engine).life
    );
}

fn assert_dead(engine: &Engine<Vec<u8>>) {
    assert!(
        matches!(actor(engine).life, CharacterLife::AwaitingRespawn { .. }),
        "{:?}",
        actor(engine).life
    );
}

#[test]
fn a_kill_just_before_a_wave_skips_it_and_counts_down_to_the_following_wave() {
    // The live case: killed 350 ms before the wave, spawned on it without
    // a respawn screen, and the client never recovered.
    let mut engine = alive_with_twenty_second_waves();
    die(&mut engine);
    world_tick(&mut engine, 650);
    let actor_now = actor(&engine);
    assert_eq!(
        actor_now.pending_spawn_deadline,
        Some(Duration::from_secs(21))
    );
    assert_eq!(
        actor_now.published_wave_deadline,
        Some(Duration::from_secs(21)),
        "the countdown shows the wave it spawns on"
    );
    owner_update(&mut engine, 1050);
    assert_dead(&engine);
    // The world tick's rollover publication keeps the queued wave.
    world_tick(&mut engine, 1100);
    assert_eq!(
        actor(&engine).published_wave_deadline,
        Some(Duration::from_secs(21))
    );
    owner_update(&mut engine, 21_030);
    assert_alive_at(&engine, 2, POINT_1);
}

#[test]
fn a_self_kill_just_before_a_wave_skips_it() {
    let mut engine = alive_with_twenty_second_waves();
    kill_self(&mut engine, 18_000);
    world_tick(&mut engine, 18_010);
    assert_eq!(
        actor(&engine).pending_spawn_deadline,
        Some(Duration::from_secs(41)),
        "the wave at 21 s is 2.99 s away"
    );
    owner_update(&mut engine, 21_030);
    assert_dead(&engine);
    owner_update(&mut engine, 41_030);
    assert_alive_at(&engine, 2, POINT_1);
}

#[test]
fn a_death_further_from_the_wave_than_the_cutoff_still_takes_it() {
    let mut engine = alive_with_twenty_second_waves();
    kill_self(&mut engine, 17_000);
    world_tick(&mut engine, 17_010);
    assert_eq!(
        actor(&engine).pending_spawn_deadline,
        Some(Duration::from_secs(21))
    );
    owner_update(&mut engine, 21_030);
    assert_alive_at(&engine, 2, POINT_1);
}

#[test]
fn without_world_ticks_the_owner_update_requeue_applies_the_same_cutoff() {
    let mut engine = engine_with_first_life();
    let team = usize::from(actor(&engine).team());
    engine.team_waves[team] = Some(TeamWaveClock {
        first_deadline: Duration::from_millis(1000),
        period: Duration::from_secs(20),
    });
    die(&mut engine);
    owner_update(&mut engine, 700);
    assert_eq!(
        actor(&engine).pending_spawn_deadline,
        Some(Duration::from_secs(21))
    );
    owner_update(&mut engine, 1050);
    assert_dead(&engine);
}

#[test]
fn a_confirmation_from_the_respawn_screen_takes_the_next_wave_again() {
    // Retail queues a dead character only on its client's confirmation, and
    // any confirmation before a wave takes that wave. Once the respawn
    // screen is open the cutoff no longer applies.
    let mut engine = alive_with_twenty_second_waves();
    kill_self(&mut engine, 17_600);
    world_tick(&mut engine, 17_610);
    assert_eq!(
        actor(&engine).pending_spawn_deadline,
        Some(Duration::from_secs(41))
    );
    // The client's respawn screen opens 2.5 s later and confirms by itself.
    select(&mut engine, 1, 20_200);
    confirm(&mut engine, 20_200);
    assert_eq!(
        actor(&engine).pending_spawn_deadline,
        Some(Duration::from_secs(21))
    );
    assert_eq!(
        actor(&engine).published_wave_deadline,
        Some(Duration::from_secs(21))
    );
    owner_update(&mut engine, 21_030);
    assert_alive_at(&engine, 2, POINT_1);
}

#[test]
fn a_repeated_confirmation_never_moves_a_queued_spawn_later() {
    let mut engine = alive_with_twenty_second_waves();
    kill_self(&mut engine, 10_000);
    world_tick(&mut engine, 10_010);
    assert_eq!(
        actor(&engine).pending_spawn_deadline,
        Some(Duration::from_secs(21))
    );
    for now in [12_600, 20_900] {
        confirm(&mut engine, now);
        assert_eq!(
            actor(&engine).pending_spawn_deadline,
            Some(Duration::from_secs(21))
        );
    }
}

#[test]
fn every_later_death_waits_for_the_cutoff_too() {
    // A client that never confirms (as a client stuck by an early life does)
    // still waits past the cutoff after each death.
    let mut engine = alive_with_twenty_second_waves();
    for (generation, died_ms, due_ms) in [(2, 18_500, 41_000), (3, 59_000, 81_000)] {
        kill_self(&mut engine, died_ms);
        world_tick(&mut engine, died_ms + 10);
        assert_eq!(
            actor(&engine).pending_spawn_deadline,
            Some(Duration::from_millis(due_ms))
        );
        owner_update(&mut engine, due_ms + 30);
        assert_alive_at(&engine, generation, POINT_1);
    }
}
