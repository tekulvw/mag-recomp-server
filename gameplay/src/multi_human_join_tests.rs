use super::*;
use crate::bot_registry::identity_tests::populated_engine;

// Synthetic two-human sequence in a bot-populated world. A is in game; B joins
// later and, like a real client, reports ready and then not-ready while it
// loads the map. Bot traffic during that window must not fill B's bounded
// reliable queue: B gets one coherent snapshot per bot once it is ready again.
fn ready(actor: &mut Active) {
    actor.ready_for_updates = true;
    actor.entered_game = true;
    actor.initial_objects = Some(Vec::new());
    actor.emplacement_creation = Some(Vec::new());
    actor.objective_delta = Some(Vec::new());
    actor.objective_assigned = true;
    actor.objective_needs_clear = false;
    actor.outbound_sequence = 1;
    actor.tracking.server_position(
        replication::CharacterPosition {
            x: 2.,
            y: 0.,
            z: 2.,
        },
        player_tracking::PositionOrigin::ServerSpawn,
        0,
    );
}
fn reserve(engine: &mut Engine<Vec<u8>>, player: u16, id: &str, now: Duration) -> Response {
    let faction = engine.match_profile().defending_faction;
    engine.handle(
        Request::Reserve {
            v: BRIDGE_VERSION,
            request_id: format!("synthetic-reserve-{id}"),
            reservation_id: id.into(),
            world_id: 1,
            player_id: player,
            account_id: u32::from(player) + 1,
            session_key: "aaaaaaaaaaaaaaaa".into(),
            access_key: "bbbbbbbbbbbbbbbb".into(),
            character: serde_json::from_value(json!({"name":format!("Synthetic {player}"),
                "faction":faction,"head":0,"voice":0,"universe_id":26,
                "character_id":u32::from(player)+1}))
            .unwrap(),
        },
        now,
        1,
    )
}
fn tick(engine: &mut Engine<Vec<u8>>, now: Duration) {
    engine.enable_world_ticks();
    let response = engine.tick_world(now, 1, "synthetic-tick".into());
    assert!(response.ok, "{:?}", response.error);
}
fn actor<'a>(engine: &'a mut Engine<Vec<u8>>, id: &str) -> Option<&'a mut Active> {
    engine
        .peers
        .values_mut()
        .chain(engine.active.iter_mut())
        .find(|a| a.reservation.id == id)
}
const A: &str = "synthetic-reservation-a001";
const B: &str = "synthetic-reservation-b001";

#[test]
fn loading_second_human_is_not_evicted_by_bot_traffic_and_resyncs_when_ready() {
    let mut engine = populated_engine("Synthetic Loading");
    engine.maximum_players = 8;
    assert!(reserve(&mut engine, 0, A, Duration::ZERO).ok);
    ready(actor(&mut engine, A).unwrap());
    for ms in (50..1000).step_by(50) {
        tick(&mut engine, Duration::from_millis(ms));
    }
    assert!(reserve(&mut engine, 1, B, Duration::from_millis(1000)).ok);
    {
        let b = actor(&mut engine, B).unwrap();
        ready(b);
        b.ready_for_updates = false; // loading the map after its first ready
    }
    let queued_before = actor(&mut engine, B).unwrap().pending_shared.len();
    let bot = *engine.bot_registry.bots.keys().next().unwrap();
    let delta = engine.bot_registry.bots[&bot]
        .snapshot(Duration::from_millis(1000), engine.wire_profile)
        .unwrap();
    // More than a whole bounded queue of bot deltas while B loads; A keeps draining.
    for burst in 0..10u64 {
        for _ in 0..20 {
            engine.broadcast_bot_packet(delta.clone());
        }
        tick(&mut engine, Duration::from_millis(1050 + burst * 50));
    }
    let b = actor(&mut engine, B).expect("loading human must not be evicted");
    assert!(
        !b.replication_backlogged,
        "bot deltas backlogged a loading human"
    );
    assert!(
        !b.pending_shared.iter().any(|packet| *packet == delta),
        "a loading human must not queue bot deltas it will be re-snapshotted for"
    );
    assert!(b.pending_shared.len() < queued_before + 10);
    assert!(
        !actor(&mut engine, A).unwrap().replication_backlogged,
        "the in-game human still receives and drains bot deltas"
    );

    // Ready again: B receives one coherent snapshot per current bot.
    actor(&mut engine, B).unwrap().ready_for_updates = true;
    tick(&mut engine, Duration::from_millis(1600));
    let known = engine
        .bot_registry
        .known
        .get(B)
        .cloned()
        .unwrap_or_default();
    let bots: BTreeSet<_> = engine.bot_registry.bots.keys().copied().collect();
    assert_eq!(
        known, bots,
        "a ready human is snapshotted every current bot"
    );
}

#[test]
fn second_human_reserve_reply_carries_no_peer_messages() {
    // Horizon parses reserve replies as control replies, which must not carry
    // reliable messages; a piggybacked peer packet made it refuse the admission.
    let mut engine = populated_engine("Synthetic Reserve Reply");
    engine.maximum_players = 8;
    assert!(reserve(&mut engine, 0, A, Duration::ZERO).ok);
    ready(actor(&mut engine, A).unwrap());
    for ms in (50..1000).step_by(50) {
        tick(&mut engine, Duration::from_millis(ms));
    }
    let bot = *engine.bot_registry.bots.keys().next().unwrap();
    let queued = engine.bot_registry.bots[&bot]
        .snapshot(Duration::from_millis(1000), engine.wire_profile)
        .unwrap();
    actor(&mut engine, A).unwrap().queue_shared(queued.clone());

    let admitted = reserve(&mut engine, 1, B, Duration::from_millis(1000));
    assert!(admitted.ok, "{:?}", admitted.error);
    assert!(
        admitted.messages.is_empty(),
        "a reserve reply must not carry another recipient's reliable packets"
    );
    assert!(
        actor(&mut engine, A)
            .unwrap()
            .pending_shared
            .iter()
            .any(|packet| *packet == queued),
        "the peer's packet stays queued for the world stream"
    );

    // The next world tick delivers it to A as usual.
    engine.enable_world_ticks();
    let streamed = engine.tick_world(Duration::from_millis(1050), 1, "synthetic-tick".into());
    assert!(streamed.ok, "{:?}", streamed.error);
    assert!(streamed.messages.iter().any(|m| m.player_id == 0));
}
