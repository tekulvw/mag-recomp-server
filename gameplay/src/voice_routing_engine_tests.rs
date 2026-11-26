//! Synthetic engine checks for voice routing: invented humans in a synthetic world, no game data.
use super::*;
use crate::bot_registry::identity_tests::populated_engine;
use crate::leadership_passives::LeadershipCatalog;
use crate::leadership_runtime::RolePolicy;
use crate::voice_routing::{TalkChannel, VoiceGroupsMember};

const A: &str = "synthetic-reservation-voice-a1";
const B: &str = "synthetic-reservation-voice-b1";
/// Player 3 is account 4, character 4, bound to the platoon leader role below.
const PLATOON_LEADER: u16 = 3;
const MEMBER: u16 = 5;

fn leadership() -> (LeadershipCatalog, RolePolicy) {
    let catalog = LeadershipCatalog::from_json(
        &json!({
            "schema_version":1,"application_id":20231,
            "source":{"member":"scripts/character_leadership_abilities.zpr","member_sha256":"a".repeat(64),
            "archive_sha256":"b".repeat(64),"extractor":"synthetic","projector":"synthetic"},
            "max_separation_range":30.,"refresh_hold_ms":3000,"removal_grace_ms":1000,
            "levels":[{"level":0,"name":"none","modifiers":[]},
            {"level":1,"name":"squad","modifiers":[{"kind":"kCharModReloadTime","value":0.1}]},
            {"level":2,"name":"platoon","modifiers":[{"kind":"kCharModAutoRepair","value":1.}]},
            {"level":3,"name":"officer","modifiers":[{"kind":"kCharModAutoHeal","value":1.}]}]
        })
        .to_string(),
    )
    .unwrap();
    let policy = RolePolicy::from_json(
        &json!({"schema_version":1,"application_id":20231,"first_member_squad_leader":false,
            "development_bindings":[{"account_id":4,"character_id":4,"native_role":2}]})
        .to_string(),
    )
    .unwrap();
    (catalog, policy)
}

fn voice_engine(voice: bool) -> Engine<Vec<u8>> {
    let (catalog, policy) = leadership();
    let mut engine = populated_engine("Synthetic Voice")
        .with_development_control()
        .with_leadership_passives(catalog, policy)
        .unwrap();
    engine.maximum_players = 8;
    if voice {
        engine = engine.with_voice_routing();
    }
    engine
}

fn reserve(engine: &mut Engine<Vec<u8>>, player: u16, id: &str) {
    let faction = engine.match_profile().defending_faction;
    let reply = engine.handle(
        Request::Reserve {
            v: BRIDGE_VERSION,
            request_id: format!("synthetic-reserve-{player}"),
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
        Duration::ZERO,
        1,
    );
    assert!(reply.ok, "{:?}", reply.error);
}

fn actor<'a>(engine: &'a mut Engine<Vec<u8>>, id: &str) -> &'a mut Active {
    engine
        .peers
        .values_mut()
        .chain(engine.active.iter_mut())
        .find(|a| a.reservation.id == id)
        .unwrap()
}

/// Placed: the character's objects exist, so its squad is known to every client.
fn place(engine: &mut Engine<Vec<u8>>, id: &str) {
    let actor = actor(engine, id);
    actor.ready_for_updates = true;
    actor.entered_game = true;
    actor.initial_objects = Some(Vec::new());
}

fn tick(engine: &mut Engine<Vec<u8>>, ms: u64) -> Response {
    engine.enable_world_ticks();
    let response = engine.tick_world(Duration::from_millis(ms), 1, "synthetic-tick".into());
    assert!(response.ok, "{:?}", response.error);
    response
}

fn entry(id: &str, player: u16, side: u8, listen: &[u16], transmit: &[u16]) -> VoiceGroupsMember {
    VoiceGroupsMember {
        reservation_id: id.into(),
        player_id: player,
        side,
        listen: listen.to_vec(),
        transmit: transmit.to_vec(),
    }
}

#[test]
fn routing_off_publishes_nothing_and_advertises_nothing() {
    let mut engine = voice_engine(false);
    reserve(&mut engine, PLATOON_LEADER, A);
    place(&mut engine, A);
    engine.subscribe_voice_groups(true);
    for ms in [100, 200, 300] {
        assert!(tick(&mut engine, ms).voice_groups.is_none());
    }
    let hello = engine.discovery_response("synthetic-hello".into());
    assert!(
        !hello
            .capabilities
            .iter()
            .any(|c| c == crate::voice_routing::CAPABILITY)
    );
    assert!(
        !serde_json::to_string(&tick(&mut engine, 400))
            .unwrap()
            .contains("voice_groups")
    );
}

#[test]
fn memberships_publish_on_subscribe_placement_squad_change_and_leave() {
    let mut engine = voice_engine(true);
    let hello = engine.discovery_response("synthetic-hello".into());
    assert!(
        hello
            .capabilities
            .iter()
            .any(|c| c == crate::voice_routing::CAPABILITY)
    );
    reserve(&mut engine, PLATOON_LEADER, A);
    reserve(&mut engine, MEMBER, B);
    assert!(
        tick(&mut engine, 100).voice_groups.is_none(),
        "no subscriber asked"
    );

    engine.subscribe_voice_groups(true);
    let first = tick(&mut engine, 200).voice_groups.unwrap();
    assert_eq!(first.revision, 1);
    // Neither has a squad yet: team only, nothing to transmit.
    assert_eq!(
        first.members,
        vec![
            entry(A, PLATOON_LEADER, 0, &[0], &[]),
            entry(B, MEMBER, 0, &[0], &[])
        ]
    );
    assert!(
        tick(&mut engine, 300).voice_groups.is_none(),
        "never per tick"
    );

    // Placement in squad 0 (platoon 0, side 0). The synthetic world has one platoon a side,
    // so the bound platoon leader reaches the team instead of company and broadcast.
    place(&mut engine, A);
    place(&mut engine, B);
    let placed = tick(&mut engine, 400).voice_groups.unwrap();
    assert_eq!(placed.revision, 2);
    assert_eq!(
        placed.members,
        vec![
            entry(A, PLATOON_LEADER, 0, &[0, 2, 4, 12, 20], &[0, 4, 20]),
            entry(B, MEMBER, 0, &[0, 12, 20], &[20])
        ]
    );
    assert!(tick(&mut engine, 500).voice_groups.is_none());

    // A squad change on the other side is a membership change.
    let catalog = actor(&mut engine, B).squad_catalog.for_squad(16).unwrap();
    actor(&mut engine, B).squad_catalog = std::sync::Arc::new(catalog);
    let moved = tick(&mut engine, 600).voice_groups.unwrap();
    assert_eq!(moved.revision, 3);
    assert_eq!(moved.members[1], entry(B, MEMBER, 1, &[1, 16, 36], &[36]));

    // A leave drops the member at a new revision.
    let release = engine.handle(
        Request::Release {
            v: BRIDGE_VERSION,
            request_id: "synthetic-release".into(),
            reservation_id: A.into(),
        },
        Duration::from_millis(650),
        1,
    );
    assert!(release.ok, "{:?}", release.error);
    let left = tick(&mut engine, 700).voice_groups.unwrap();
    assert_eq!(left.revision, 4);
    assert_eq!(left.members, vec![entry(B, MEMBER, 1, &[1, 16, 36], &[36])]);

    // A new or resumed subscription gets the full table again.
    engine.subscribe_voice_groups(true);
    let resumed = tick(&mut engine, 800).voice_groups.unwrap();
    assert_eq!((resumed.revision, resumed.members), (5, left.members));
    // A subscriber that did not ask gets none.
    engine.subscribe_voice_groups(false);
    let catalog = actor(&mut engine, B).squad_catalog.for_squad(0).unwrap();
    actor(&mut engine, B).squad_catalog = std::sync::Arc::new(catalog);
    assert!(tick(&mut engine, 900).voice_groups.is_none());
}

fn talk_channel_event(engine: &mut Engine<Vec<u8>>, body: &[u8]) -> Value {
    let mut bytes = crate::voice_routing::CHANNEL_SELECT.to_be_bytes().to_vec();
    bytes.extend_from_slice(body);
    let before = engine.writer.len();
    crate::spawn_progression::tests::dispatch(engine, &bytes, 1000).unwrap();
    let log = std::str::from_utf8(&engine.writer[before..]).unwrap();
    log.lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|record| record["op"] == "event")
        .map(|record| record["decoded"].clone())
        .unwrap_or_else(|| panic!("no event summary in {log}"))
}

#[test]
fn talk_channel_is_recorded_only_with_routing_on() {
    let mut engine = voice_engine(true);
    reserve(&mut engine, PLATOON_LEADER, A);
    place(&mut engine, A);
    let summary = talk_channel_event(&mut engine, &[2]);
    assert_eq!(summary["type"], "talk_channel");
    assert_eq!(summary["channel"], 2);
    assert_eq!(summary["squad_index"], 0);
    assert_eq!(summary["held_role_ability_level"], 2);
    assert_eq!(
        engine.active.as_ref().unwrap().talk_channel,
        Some(TalkChannel::Platoon)
    );
    // The record is advisory: transmit sets never follow it.
    engine.subscribe_voice_groups(true);
    let frame = tick(&mut engine, 1100).voice_groups.unwrap();
    assert_eq!(frame.members[0].transmit, vec![0, 4, 20]);
    // A body that is not one known channel byte stays unknown and changes nothing.
    let unknown = talk_channel_event(&mut engine, &[9]);
    assert_eq!(unknown["type"], "unknown");
    assert_eq!(
        engine.active.as_ref().unwrap().talk_channel,
        Some(TalkChannel::Platoon)
    );

    let mut engine = voice_engine(false);
    reserve(&mut engine, PLATOON_LEADER, A);
    place(&mut engine, A);
    let summary = talk_channel_event(&mut engine, &[2]);
    assert_eq!(summary, json!({"type":"unknown","message_type":601}));
    assert_eq!(engine.active.as_ref().unwrap().talk_channel, None);
}

#[test]
fn talk_channel_decodes_one_known_byte() {
    fn decode(bytes: &[u8]) -> Option<protocol::GameMessage<'_>> {
        protocol::GameMessage::decode(bytes).ok()
    }
    assert!(matches!(
        decode(&[0x02, 0x59, 0x05]),
        Some(protocol::GameMessage::TalkChannel(
            TalkChannel::TeamBroadcast
        ))
    ));
    assert!(matches!(
        decode(&[0x02, 0x59, 0x00]),
        Some(protocol::GameMessage::TalkChannel(TalkChannel::None))
    ));
    for body in [&[][..], &[8][..], &[1, 0][..]] {
        let mut bytes = vec![0x02, 0x59];
        bytes.extend_from_slice(body);
        assert!(matches!(
            decode(&bytes),
            Some(protocol::GameMessage::Unknown {
                message_type: 0x0259,
                ..
            })
        ));
    }
}
