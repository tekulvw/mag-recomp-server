//! Synthetic receipt settlement after a terminal cohort has been archived.
use crate::{
    extended_objective_reward_tests::{callback, fixture},
    match_cohorts::ResultsCohort,
    replication::{BitReader, ObjectUpdate, ObjectiveState},
    spawn_progression::tests::ready,
    *,
};

use super::{ProgressionSnapshot, RewardEvent, RewardKind, RewardRecipient};
use std::collections::BTreeMap;

const SUCCESSOR: &str = "synthetic-cohort-successor";
const SLOT_OWNER: &str = "synthetic-cohort-slot-owner";
const OLD_PEER: &str = "synthetic-cohort-old-peer";

fn actor_mut<'a>(engine: &'a mut Engine<Vec<u8>>, id: &str) -> &'a mut Active {
    engine
        .active
        .iter_mut()
        .chain(engine.peers.values_mut())
        .find(|actor| actor.reservation.id == id)
        .unwrap()
}

fn actor<'a>(engine: &'a Engine<Vec<u8>>, id: &str) -> &'a Active {
    engine
        .active
        .iter()
        .chain(engine.peers.values())
        .find(|actor| actor.reservation.id == id)
        .unwrap()
}

fn reserve(
    engine: &mut Engine<Vec<u8>>,
    id: &str,
    player: u16,
    account: u32,
    mut character: Character,
) {
    character.loadouts = None;
    let response = engine.handle(
        Request::Reserve {
            v: BRIDGE_VERSION,
            request_id: "reserve-cohort-reward-fixture".into(),
            reservation_id: id.into(),
            world_id: 1,
            player_id: player,
            account_id: account,
            session_key: "aaaaaaaaaaaaaaaa".into(),
            access_key: "bbbbbbbbbbbbbbbb".into(),
            character,
        },
        Duration::from_millis(140),
        1,
    );
    assert!(response.ok, "{:?}", response.error);
}

fn archive(engine: &mut Engine<Vec<u8>>) -> String {
    let mut actors = std::mem::take(&mut engine.peers);
    if let Some(active) = engine.active.take() {
        actors.insert(active.reservation.id.clone(), active);
    }
    let match_id = engine.reward_match_id.take().unwrap();
    let mut round = std::mem::take(&mut engine.round);
    round.scores = Some([0, 1_000]);
    for actor in actors.values_mut() {
        actor.round_scores = round.scores;
    }
    engine.results_cohorts.insert(
        match_id.clone(),
        ResultsCohort {
            terminal: match_flow::MatchTerminal {
                world_id: 1,
                match_id: match_id.clone(),
                reason: "synthetic-round-ended".into(),
                reservation_ids: actors.keys().cloned().collect(),
            },
            actors,
            round,
            participants: std::mem::take(&mut engine.round_participants),
            profile: engine.wire_profile,
            game_type: engine.match_profile().game_type,
            reward_match_id: match_id.clone(),
            cursor: 0,
        },
    );
    match_id
}

fn archived_destruction() -> (Engine<Vec<u8>>, RewardEvent) {
    let mut engine = fixture(3, 4);
    callback(
        &mut engine,
        ObjectiveState::Activated,
        ObjectiveState::Completed,
    )
    .unwrap();
    let reward = engine.pending_reward_batch()[0].clone();
    let original = &engine.active.as_ref().unwrap().reservation;
    let character = original.character.clone();
    let account = original.account;
    let mut old_peer = character.clone();
    old_peer.account_id = Some("synthetic-old-peer-account".into());
    reserve(&mut engine, OLD_PEER, 1, 11, old_peer);
    let old_actor = actor_mut(&mut engine, &reward.reservation_id);
    ready(old_actor);
    old_actor.outbound_sequence = 10;
    let old_peer = actor_mut(&mut engine, OLD_PEER);
    ready(old_peer);
    old_peer.outbound_sequence = 20;
    assert_eq!(archive(&mut engine), reward.match_id);
    let mut other_character = character.clone();
    other_character.character_id = Some(reward.character_id + 1);
    // Same account string and same transport slot, different character.
    reserve(
        &mut engine,
        SLOT_OWNER,
        reward.player_id,
        account + 1,
        other_character,
    );
    reserve(&mut engine, SUCCESSOR, 2, account, character);
    assert_ne!(actor(&engine, SUCCESSOR).reward_match_id, reward.match_id);
    let successor = actor_mut(&mut engine, SUCCESSOR);
    ready(successor);
    successor.outbound_sequence = 30;
    successor.match_experience = 11;
    successor.reward_statistics.insert(42, 4);
    let slot_owner = actor_mut(&mut engine, SLOT_OWNER);
    ready(slot_owner);
    slot_owner.outbound_sequence = 40;
    (engine, reward)
}

fn ack_value(reward: &RewardEvent, experience: u32, level: u8, match_experience: u32) -> Value {
    let mut input = json!({"op":"progression_committed","v":BRIDGE_VERSION,"request_id":"cohort-reward-ack",
        "reservation_id":reward.reservation_id,"world_id":reward.world_id,"player_id":reward.player_id,
        "reward_id":reward.reward_id,"progression":{"experience":experience,"level":level,"skill_points":0},
        "match_experience":match_experience});
    if reward.frago == Some(true) {
        input["match_frago_points"] = json!(20);
    }
    input
}

fn ack(engine: &mut Engine<Vec<u8>>, input: Value) -> Response {
    engine.handle(
        serde_json::from_value(input).unwrap(),
        Duration::from_millis(140),
        1,
    )
}

fn level_messages(
    messages: &[OutboundMessage],
    profile: wire_profile::WireProfile,
) -> BTreeMap<String, (u32, u16)> {
    messages
        .iter()
        .filter_map(|message| {
            let wire = protocol::decode_hex(&message.payload_hex);
            let update = ObjectUpdate::decode_with_profile(&wire, profile).ok()?;
            let mut bits = BitReader::with_profile(&wire, profile);
            bits.position = profile.header_bits();
            assert_eq!(bits.read(1).unwrap(), 1);
            let object = bits.read(16).unwrap() as u16;
            if update.header.object_type_mask != 8
                || bits.read(18).ok()? != 0
                || bits.read(1).ok()? != 1
            {
                return None;
            }
            Some((
                message.reservation_id.clone(),
                (update.header.sequence, object),
            ))
        })
        .collect()
}

#[test]
fn old_ack_credits_its_own_match_and_levels_the_same_character_in_each_cohort() {
    let (mut engine, reward) = archived_destruction();
    let response = ack(&mut engine, ack_value(&reward, 40, 2, 40));
    assert!(response.ok, "{:?}", response.error);
    let original = &engine.results_cohorts[&reward.match_id].actors[&reward.reservation_id];
    assert_eq!(original.match_experience, 40);
    assert_eq!(
        original.reward_statistics,
        [(3, 20), (42, 1), (61, 1)].into_iter().collect()
    );
    let archived_peer = &engine.results_cohorts[&reward.match_id].actors[OLD_PEER];
    assert_eq!(archived_peer.match_experience, 0);
    assert!(archived_peer.reward_statistics.is_empty());
    let successor = actor(&engine, SUCCESSOR);
    assert_eq!(
        successor
            .reservation
            .character
            .progression
            .unwrap()
            .experience,
        40
    );
    assert_eq!(successor.match_experience, 11);
    assert_eq!(successor.reward_statistics, [(42, 4)].into_iter().collect());
    let slot_owner = actor(&engine, SLOT_OWNER);
    assert_eq!(
        slot_owner.reservation.character.progression,
        Some(ProgressionSnapshot::default())
    );
    assert_eq!(slot_owner.match_experience, 0);
    assert!(slot_owner.reward_statistics.is_empty());
    assert_eq!(
        level_messages(&response.messages, engine.wire_profile),
        BTreeMap::from([
            (reward.reservation_id.clone(), (11, reward.player_id)),
            (OLD_PEER.into(), (21, reward.player_id)),
            (SUCCESSOR.into(), (31, 2)),
            (SLOT_OWNER.into(), (41, 2)),
        ])
    );
    assert_eq!(response.messages.len(), 5);
    let stats = response
        .messages
        .iter()
        .find(|message| protocol::decode_hex(&message.payload_hex).starts_with(&[2, 4]))
        .unwrap();
    assert_eq!(stats.reservation_id, reward.reservation_id);
    assert_eq!(stats.player_id, reward.player_id);
}

#[test]
fn detached_old_ack_preserves_outbox_on_failure_and_updates_frozen_participation_once() {
    let (mut engine, reward) = archived_destruction();
    let old = engine.results_cohorts.get_mut(&reward.match_id).unwrap();
    let old_actor = old.actors.get_mut(&reward.reservation_id).unwrap();
    old_actor.match_experience = 12;
    old_actor.reward_statistics = [(42, 2), (61, 3)].into_iter().collect();
    let released = engine.handle(
        Request::Release {
            v: BRIDGE_VERSION,
            request_id: "release-cohort-reward-fixture".into(),
            reservation_id: reward.reservation_id.clone(),
        },
        Duration::from_millis(140),
        1,
    );
    assert!(released.ok, "{:?}", released.error);
    assert!(
        !engine.results_cohorts[&reward.match_id]
            .actors
            .contains_key(&reward.reservation_id)
    );
    let key = (
        reward.world_id,
        reward.account_id.clone(),
        reward.character_id,
    );
    let request = ack_value(&reward, 40, 2, 40);
    let mut wrong_identity = request.clone();
    wrong_identity["reservation_id"] = json!(SLOT_OWNER);
    assert_eq!(
        ack(&mut engine, wrong_identity).error.as_deref(),
        Some("reward_identity_mismatch")
    );
    let limit = engine.max_log;
    engine.max_log = engine.written;

    let failed = ack(&mut engine, request.clone());
    assert_eq!(failed.error.as_deref(), Some("log_limit_reached"));
    assert!(failed.messages.is_empty());
    assert_eq!(
        engine.pending_reward_batch().as_slice(),
        std::slice::from_ref(&reward)
    );
    assert!(!engine.committed_rewards.contains_key(&reward.reward_id));
    assert_eq!(actor(&engine, SUCCESSOR).outbound_sequence, 30);
    assert_eq!(actor(&engine, SLOT_OWNER).outbound_sequence, 40);
    assert_eq!(
        actor(&engine, SUCCESSOR).reservation.character.progression,
        Some(ProgressionSnapshot::default())
    );
    assert_eq!(
        engine.results_cohorts[&reward.match_id].participants[&key].statistics,
        [(42, 2), (61, 3)].into_iter().collect()
    );

    engine.max_log = limit;
    engine.failed = false;
    let committed = ack(&mut engine, request.clone());
    assert!(committed.ok, "{:?}", committed.error);
    assert_eq!(
        level_messages(&committed.messages, engine.wire_profile),
        BTreeMap::from([(SUCCESSOR.into(), (31, 2)), (SLOT_OWNER.into(), (41, 2))])
    );
    assert_eq!(committed.messages.len(), 2);
    assert!(engine.pending_reward_batch().is_empty());
    let replay = ack(&mut engine, request.clone());
    assert!(replay.ok);
    assert!(replay.messages.is_empty());
    let mut conflict = request;
    conflict["match_experience"] = json!(41);
    assert_eq!(
        ack(&mut engine, conflict).error.as_deref(),
        Some("reward_ack_conflict")
    );
    let participation = &engine.results_cohorts[&reward.match_id].participants[&key];
    assert_eq!(participation.match_experience, 40);
    assert_eq!(
        participation.statistics,
        [(3, 20), (42, 3), (61, 4)].into_iter().collect()
    );
}

#[test]
fn stale_old_ack_cannot_rewind_newer_saved_progression_or_equal_balance_leadership() {
    let (mut engine, reward) = archived_destruction();
    let newer = ProgressionSnapshot {
        experience: 90,
        level: 5,
        skill_points: 0,
    };
    let leadership = leadership_runtime::LeadershipRecord {
        leadership_level: 3,
        nominations: 3,
        matches_since_last_command: 1,
        leadership_points: Some(27),
    };
    let successor = actor_mut(&mut engine, SUCCESSOR);
    successor.reservation.character.progression = Some(newer);
    successor.reservation.character.leadership = Some(leadership);
    successor.leadership_level = 3;
    successor.nominations = 3;
    let mut request = ack_value(&reward, 40, 2, 40);
    request["leadership"] = json!({"leadership_level":2,"nominations":1,
        "matches_since_last_command":7,"leadership_points":27});
    let response = ack(&mut engine, request);
    assert!(response.ok, "{:?}", response.error);
    let successor = actor(&engine, SUCCESSOR);
    assert_eq!(successor.reservation.character.progression, Some(newer));
    assert_eq!(successor.reservation.character.leadership, Some(leadership));
    assert_eq!(successor.outbound_sequence, 30);
    assert_eq!(actor(&engine, SLOT_OWNER).outbound_sequence, 40);
    assert_eq!(
        engine.results_cohorts[&reward.match_id].actors[&reward.reservation_id].match_experience,
        40
    );
}

#[test]
fn current_ack_carries_durable_state_back_without_changing_archived_match_statistics() {
    let (mut engine, old_reward) = archived_destruction();
    let new_reward = RewardRecipient::from_active(actor(&engine, SUCCESSOR))
        .unwrap()
        .event(
            "synthetic-cohort-new-kill".into(),
            RewardKind::EnemyKilled,
            1,
        );
    engine.commit_rewards(vec![new_reward.clone()]);
    let response = ack(&mut engine, ack_value(&new_reward, 70, 3, 41));
    assert!(response.ok, "{:?}", response.error);
    let old_actor =
        &engine.results_cohorts[&old_reward.match_id].actors[&old_reward.reservation_id];
    assert_eq!(
        old_actor
            .reservation
            .character
            .progression
            .unwrap()
            .experience,
        70
    );
    assert_eq!(old_actor.match_experience, 0);
    assert!(old_actor.reward_statistics.is_empty());
    assert_eq!(actor(&engine, SUCCESSOR).match_experience, 41);
    assert_eq!(engine.pending_reward_batch(), [old_reward]);
}

fn fill_cohort(engine: &mut Engine<Vec<u8>>, prefix: &str, template: &Character) {
    let mut character = template.clone();
    // Generate valid individual reservations without depending on the
    // synthetic map's smaller team and squad capacities.
    let mut actors = std::mem::take(&mut engine.peers);
    if let Some(actor) = engine.active.take() {
        actors.insert(actor.reservation.id.clone(), actor);
    }
    for slot in 0..world_capacity::MAX_ADMITTED_HUMANS as u16 {
        if actors
            .values()
            .any(|actor| actor.reservation.player == slot)
        {
            continue;
        }
        let id = format!("{prefix}-peer-{slot}");
        character.account_id = Some(format!("{prefix}-a{slot}"));
        reserve(engine, &id, slot, u32::from(slot) + 200, character.clone());
        let actor = engine
            .peers
            .remove(&id)
            .or_else(|| engine.active.take())
            .unwrap();
        actors.insert(id, actor);
    }
    engine.peers = actors;
    for actor in engine.active.iter_mut().chain(engine.peers.values_mut()) {
        ready(actor);
    }
}

#[test]
fn ack_level_publications_respect_the_shared_bridge_reply_limit() {
    let (mut engine, reward) = archived_destruction();
    engine.maximum_players = world_capacity::MAX_ADMITTED_HUMANS;
    engine.world_tick_enabled = true;
    let successor = actor(&engine, SUCCESSOR);
    let account = successor.reservation.account;
    let character = successor.reservation.character.clone();
    fill_cohort(&mut engine, "synthetic-cohort-middle", &character);
    let middle = archive(&mut engine);
    reserve(
        &mut engine,
        "synthetic-cohort-latest",
        2,
        account,
        character.clone(),
    );
    fill_cohort(&mut engine, "synthetic-cohort-latest", &character);

    engine.world_tick_enabled = false;
    let response = ack(&mut engine, ack_value(&reward, 40, 2, 40));
    assert!(response.ok, "{:?}", response.error);
    assert!(
        response.messages.len() <= 128,
        "bridge rejected {} messages",
        response.messages.len()
    );
    let mut delivered = level_messages(&response.messages, engine.wire_profile);
    assert_eq!(response.messages.len(), delivered.len() + 1);
    assert!(delivered.len() < 2 * world_capacity::MAX_ADMITTED_HUMANS + 2);

    let drained = engine
        .tick_results_cohorts(Duration::from_millis(140), 1, Vec::new())
        .unwrap();
    for (id, (sequence, object)) in level_messages(&drained, engine.wire_profile) {
        let want = if engine.results_cohorts[&middle].actors.contains_key(&id) {
            (2, 2)
        } else if id == OLD_PEER {
            (21, reward.player_id)
        } else {
            (11, reward.player_id)
        };
        assert_eq!((sequence, object), want);
        assert!(delivered.insert(id, (sequence, object)).is_none());
    }
    assert_eq!(delivered.len(), 2 * world_capacity::MAX_ADMITTED_HUMANS + 2);
}

#[test]
fn world_stream_publishes_archived_reward_after_end_and_before_final_statistics() {
    use replication::{MatchPhase, multiplayer::MatchPhaseUpdate};

    let (mut engine, reward) = archived_destruction();
    engine.world_tick_enabled = true;
    let request = ack_value(&reward, 40, 2, 40);
    let premature = ack(&mut engine, request.clone());
    assert_eq!(premature.error.as_deref(), Some("reward_end_not_queued"));
    assert!(premature.messages.is_empty());
    assert_eq!(
        engine.pending_reward_batch().as_slice(),
        std::slice::from_ref(&reward)
    );

    let old = engine.results_cohorts.get_mut(&reward.match_id).unwrap();
    old.round.outcome = Some(match_flow::Outcome {
        winner: 1,
        major: true,
        deadline: Duration::from_millis(130),
        finalized: false,
    });
    let profile = old.profile;
    match_flow::queue_round_publications(
        &old.round,
        profile,
        old.actors.values_mut().collect(),
        false,
        Duration::from_millis(100),
        match_flow::ScorePublication::Sabotage {
            objective_effect: false,
        },
    )
    .unwrap();

    let response = ack(&mut engine, request);
    assert!(response.ok, "{:?}", response.error);
    assert_eq!(
        level_messages(&response.messages, profile),
        BTreeMap::from([(SUCCESSOR.into(), (31, 2)), (SLOT_OWNER.into(), (41, 2))])
    );
    assert_eq!(response.messages.len(), 2);
    assert_eq!(
        engine.results_cohorts[&reward.match_id].actors[&reward.reservation_id].outbound_sequence,
        10
    );
    let streamed = engine
        .tick_results_cohorts(Duration::from_millis(140), 1, Vec::new())
        .unwrap();
    let original: Vec<_> = streamed
        .iter()
        .filter(|message| message.reservation_id == reward.reservation_id)
        .map(|message| protocol::decode_hex(&message.payload_hex))
        .collect();
    assert_eq!(
        original[0],
        MatchPhaseUpdate {
            sequence: 11,
            timestamp_ms: 100,
            phase: MatchPhase::EndCountdown,
        }
        .encode_with_profile(profile)
        .unwrap()
    );
    assert_eq!(&original[1][..2], &[0x01, 0xfe]);
    assert_eq!(&original[2][..2], &[0x02, 0x04]);
    assert_eq!(
        level_messages(&streamed, profile)[&reward.reservation_id],
        (12, reward.player_id)
    );
    assert_eq!(&original[4][..2], &[0x02, 0x04]);
    assert_eq!(original[original.len() - 2], [0x02, 0x05]);
    assert_eq!(
        original.last().unwrap(),
        &MatchPhaseUpdate {
            sequence: 13,
            timestamp_ms: 140,
            phase: MatchPhase::PostGame,
        }
        .encode_with_profile(profile)
        .unwrap()
    );
}

#[test]
fn fixed_terminal_release_preserves_pending_desired_kit_during_successor_reserve() {
    let mut engine = fixture(3, 4);
    engine.world_tick_enabled = true;
    assert!(engine.match_profiles.is_none());
    let original = engine.active.as_mut().unwrap();
    let reservation = &original.reservation;
    let character = reservation.character.clone();
    let account = reservation.account;
    let mut peer = character.clone();
    peer.account_id = Some("synthetic-kit-old-peer".into());
    let receipt = loadout_selection::CommittedLoadoutSelection {
        layout_revision: 0,
        selection_id: "synthetic-released-desired-kit".into(),
        reservation_id: reservation.id.clone(),
        world_id: reservation.world,
        player_id: reservation.player,
        account_id: character.account_id.clone().unwrap(),
        character_id: character.character_id.unwrap(),
        revision: 5,
        inventory_index: 3,
    };
    original.pending_loadout_selection = Some(receipt.clone());
    reserve(&mut engine, OLD_PEER, 1, account + 1, peer);
    let mut terminal = engine.round.clone();
    terminal.scores = Some([0, 1_000]);
    terminal.outcome = Some(match_flow::Outcome {
        winner: 1,
        major: true,
        deadline: Duration::from_secs(3),
        finalized: false,
    });
    engine
        .commit_round_update(terminal, vec![], vec![], Duration::from_millis(130), 1)
        .unwrap();
    assert_eq!(
        engine.pending_loadout_selections().as_slice(),
        std::slice::from_ref(&receipt)
    );

    let release = engine.handle(
        Request::Release {
            v: BRIDGE_VERSION,
            request_id: "release-fixed-terminal-kit".into(),
            reservation_id: receipt.reservation_id.clone(),
        },
        Duration::from_millis(140),
        1,
    );
    assert!(release.ok, "{:?}", release.error);
    assert_eq!(
        engine.pending_loadout_selections().as_slice(),
        std::slice::from_ref(&receipt)
    );
    reserve(
        &mut engine,
        SUCCESSOR,
        receipt.player_id,
        account,
        character,
    );
    assert_eq!(
        engine.pending_loadout_selections().as_slice(),
        std::slice::from_ref(&receipt)
    );

    let settled = engine.handle(
        Request::LoadoutSelectionSettled {
            v: BRIDGE_VERSION,
            request_id: "settle-fixed-released-kit".into(),
            selection_id: receipt.selection_id,
            reservation_id: receipt.reservation_id,
            world_id: receipt.world_id,
            player_id: receipt.player_id,
            revision: receipt.revision,
            status: loadout_selection::SelectionStatus::Persisted,
        },
        Duration::from_millis(160),
        1,
    );
    assert!(settled.ok, "{:?}", settled.error);
    assert!(engine.pending_loadout_selections().is_empty());
    assert!(actor(&engine, SUCCESSOR).selected_loadout.is_none());
}
