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
fn blocked_bot_retries_after_team_wave_renews() {
    let mut engine = populated_engine("Synthetic Blocked Wave");
    engine.maximum_players = 8;
    engine.bot_registry.total = 3;
    assert!(reserve(&mut engine, 0, A, Duration::ZERO).ok);
    let human = actor(&mut engine, A).unwrap();
    ready(human);
    human.spawn_delta = Some(Vec::new());
    human.life = CharacterLife::Alive {
        generation: 1,
        health_quarter_points: 400,
    };
    let (&id, bot) = engine
        .bot_registry
        .bots
        .iter()
        .find(|(_, bot)| bot.target.squad.team() == 0)
        .unwrap();
    let generation = bot.target.runtime.unwrap().generation;
    let damage = replication::CharacterDamage {
        health_quarter_points: 0,
        ..bot.damage
    };
    let at = Duration::from_millis;
    engine.team_waves[0] = Some(TeamWaveClock {
        first_deadline: at(100),
        period: at(1000),
    });
    engine
        .commit_bot_damage(id, generation, damage, at(50))
        .unwrap();
    engine.tick_bots(at(100), 1).unwrap();
    assert_eq!(
        engine.bot_registry.bots[&id].health, 0,
        "body blocks the due spawn"
    );
    engine.team_waves[0]
        .as_mut()
        .unwrap()
        .advance_native_cycle(at(100))
        .unwrap();

    actor(&mut engine, A).unwrap().tracking.server_position(
        replication::CharacterPosition {
            x: 8.,
            y: 0.,
            z: 8.,
        },
        player_tracking::PositionOrigin::ServerSpawn,
        0,
    );
    engine.tick_bots(at(150), 1).unwrap();
    let bot = &engine.bot_registry.bots[&id];
    assert_eq!(
        bot.target.runtime.unwrap().generation,
        generation + 1,
        "the first clear tick admits the bot whose wave was already due"
    );
    assert!(bot.health > 0);
}

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

/// Drain every queued shared frame and return, per recipient, the Character
/// updates stamped `stamps` in the order they were released.
fn drain_stamped(engine: &mut Engine<Vec<u8>>, stamps: &[u32]) -> BTreeMap<String, Vec<Vec<u8>>> {
    let mut out = BTreeMap::<String, Vec<Vec<u8>>>::new();
    for _ in 0..64 {
        for message in engine.drain_shared(Vec::new(), 1).unwrap() {
            let wire = protocol::decode_hex(&message.payload_hex);
            if replication::ObjectUpdate::decode_with_profile(&wire, engine.wire_profile)
                .is_ok_and(|update| stamps.contains(&update.header.timestamp_ms))
            {
                out.entry(message.reservation_id).or_default().push(wire);
            }
        }
        if engine
            .peers
            .values()
            .chain(engine.active.iter())
            .all(|actor| actor.pending_shared.is_empty())
        {
            return out;
        }
    }
    panic!("shared queues did not drain");
}

#[test]
fn coalesced_drain_joins_one_ticks_character_deltas_per_recipient() {
    let movement = |engine: &Engine<Vec<u8>>, player: u16, stamp: u32| {
        let fields = player_tracking::ClientCharacterFields {
            position: Some(replication::CharacterPosition {
                x: f32::from(player),
                y: 1.,
                z: 3.,
            }),
            yaw: Some(100),
            ..Default::default()
        };
        replication::multiplayer::CharacterOwnerDelta {
            sequence: 2,
            timestamp_ms: stamp,
            player,
            fields: &fields,
        }
        .encode_with_profile(engine.wire_profile)
        .unwrap()
    };
    let objects = |engine: &Engine<Vec<u8>>, wire: &[u8]| {
        crate::character_coalescing::objects(wire, engine.wire_profile)
    };
    let mut released = Vec::new();
    let mut profile = None;
    for coalesce in [false, true] {
        let mut engine = populated_engine("Synthetic Coalesce");
        engine.maximum_players = 8;
        engine.coalesce_character_replication = coalesce;
        profile = Some(engine.wire_profile);
        assert!(reserve(&mut engine, 0, A, Duration::ZERO).ok);
        ready(actor(&mut engine, A).unwrap());
        assert!(reserve(&mut engine, 1, B, Duration::ZERO).ok);
        ready(actor(&mut engine, B).unwrap());
        drain_stamped(&mut engine, &[]);
        let level = replication::CharacterLevelUpdate {
            sequence: 2,
            timestamp_ms: 500,
            object_index: 100,
            mag_info: replication::MagCharacterInfo {
                nominations: 0,
                level: 4,
                leadership_level: 0,
                skill_specialty: 0,
                head_id: 0,
                voice_id: 0,
            },
        }
        .encode_with_profile(engine.wire_profile)
        .unwrap();
        // One tick's ten bot deltas, a row the coalescer does not measure, a
        // repeat of the first bot, then the next tick: two bots and A itself.
        let mut queued: Vec<_> = (100..110).map(|p| movement(&engine, p, 500)).collect();
        queued.push(level);
        queued.push(movement(&engine, 100, 500));
        queued.extend([110, 111, 0].map(|p| movement(&engine, p, 550)));
        for packet in &queued {
            for id in [A, B] {
                actor(&mut engine, id).unwrap().queue_shared(packet.clone());
            }
        }
        let first = [A, B].map(|id| actor(&mut engine, id).unwrap().outbound_sequence);
        let out = drain_stamped(&mut engine, &[500, 550]);
        for (index, id) in [A, B].into_iter().enumerate() {
            let updates = &out[id];
            let rows: Vec<_> = updates.iter().map(|wire| objects(&engine, wire)).collect();
            if coalesce {
                let mut expected =
                    vec![Some((100..110).collect::<Vec<_>>()), None, Some(vec![100])];
                // The recipient's own Character goes out alone.
                expected.extend(if id == A {
                    vec![Some(vec![110, 111]), Some(vec![0])]
                } else {
                    vec![Some(vec![110, 111, 0])]
                });
                assert_eq!(rows, expected, "{id}");
            } else {
                assert_eq!(updates.len(), queued.len(), "{id}");
            }
            // Released updates use the recipient's consecutive sequences and
            // are what its NAK history replays.
            let actor = actor(&mut engine, id).unwrap();
            for (offset, wire) in updates.iter().enumerate() {
                let header =
                    replication::ObjectUpdate::decode_with_profile(wire, actor.wire_profile)
                        .unwrap()
                        .header;
                assert_eq!(header.sequence, first[index] + 1 + offset as u32);
                assert!(actor.replication_history.contains(wire));
            }
            assert!(actor.pending_shared.is_empty());
            released.push((coalesce, id, updates.clone()));
        }
    }
    // Every row reaches each recipient exactly once either way.
    let rows = |coalesce: bool, id: &str| -> usize {
        released
            .iter()
            .filter(|(c, i, _)| *c == coalesce && *i == id)
            .flat_map(|(_, _, updates)| updates)
            .map(|wire| {
                crate::character_coalescing::objects(wire, profile.unwrap()).map_or(1, |o| o.len())
            })
            .sum()
    };
    for id in [A, B] {
        assert_eq!(rows(false, id), rows(true, id));
    }
}

#[test]
fn per_recipient_reply_budget_gives_each_recipient_its_own_allowance() {
    let hello = |engine: &mut Engine<Vec<u8>>| {
        engine
            .handle(
                Request::Hello {
                    v: 2,
                    request_id: "synthetic-hello".into(),
                },
                Duration::ZERO,
                1,
            )
            .capabilities
            .contains(&crate::reply_budget::CAPABILITY.to_owned())
    };
    let players: Vec<(u16, String)> = (0..32)
        .map(|p| (p, format!("synthetic-reservation-r{p:03}")))
        .collect();
    let mut released = Vec::new();
    for per_recipient in [false, true] {
        let mut engine = populated_engine("Synthetic Budget");
        engine.maximum_players = 64;
        engine.max_log = 64 << 20; // every released message is recorded
        if per_recipient {
            engine = engine.with_per_recipient_reply_budget();
        }
        assert_eq!(hello(&mut engine), per_recipient);
        for (player, id) in &players {
            assert!(reserve(&mut engine, *player, id, Duration::ZERO).ok);
            ready(actor(&mut engine, id).unwrap());
        }
        drain_stamped(&mut engine, &[]);
        // 40 frames per recipient that never join: level rows of distinct bots.
        for object_index in 100..140 {
            let packet = replication::CharacterLevelUpdate {
                sequence: 2,
                timestamp_ms: 700,
                object_index,
                mag_info: replication::MagCharacterInfo {
                    nominations: 0,
                    level: 3,
                    leadership_level: 0,
                    skill_specialty: 0,
                    head_id: 0,
                    voice_id: 0,
                },
            }
            .encode_with_profile(engine.wire_profile)
            .unwrap();
            for (_, id) in &players {
                actor(&mut engine, id).unwrap().queue_shared(packet.clone());
            }
        }
        let profile = engine.wire_profile;
        let reply: Vec<_> = engine
            .drain_shared(Vec::new(), 1)
            .unwrap()
            .into_iter()
            .filter(|m| {
                let wire = protocol::decode_hex(&m.payload_hex);
                replication::ObjectUpdate::decode_with_profile(&wire, profile)
                    .is_ok_and(|update| update.header.timestamp_ms == 700)
            })
            .collect();
        let mut per = BTreeMap::<String, usize>::new();
        for message in &reply {
            *per.entry(message.reservation_id.clone()).or_default() += 1;
        }
        let bytes: usize = reply.iter().map(|m| m.payload_hex.len() / 2).sum();
        if per_recipient {
            // Sixteen frames for every recipient in one reply.
            assert_eq!(reply.len(), 32 * 16);
            assert!(per.values().all(|&n| n == 16), "{per:?}");
            assert!(bytes <= crate::reply_budget::PER_RECIPIENT.payload_bytes);
        } else {
            // The legacy budget is shared: 128 messages for all 32 recipients.
            assert_eq!(reply.len(), 128);
            assert!(per.values().all(|&n| n == 4), "{per:?}");
            assert!(bytes <= crate::reply_budget::LEGACY_PAYLOAD_BYTES);
        }
        released.push(reply.len());
    }
    assert!(released[1] > released[0]);
}
