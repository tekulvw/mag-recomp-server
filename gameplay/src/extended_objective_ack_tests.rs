use super::*;
use crate::extended_objective_reward_tests::{ack, callback, fixture};
use progression::{RewardEvent, RewardKind};
use replication::ObjectiveState;

const RECONNECTED: &str = "synthetic-reconnected-objective-credit";
const PEER: &str = "synthetic-objective-round-peer";

fn pending_destruction(engine: &mut Engine<Vec<u8>>) -> RewardEvent {
    callback(engine, ObjectiveState::Activated, ObjectiveState::Completed).unwrap();
    let [reward]: [RewardEvent; 1] = engine.pending_reward_batch().try_into().unwrap();
    assert_eq!(reward.kind, RewardKind::DefensiveEmplacementsDestroyed);
    assert_eq!(reward.frago, Some(true));
    reward
}

fn reserve(
    engine: &mut Engine<Vec<u8>>,
    id: &str,
    player: u16,
    account: u32,
    mut character: Character,
) {
    // These synthetic reservations exercise participation without spawning a kit.
    character.loadouts = None;
    let reply = engine.handle(
        Request::Reserve {
            v: BRIDGE_VERSION,
            request_id: "reserve-objective-ack-fixture".into(),
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
    assert!(reply.ok, "{:?}", reply.error);
}

fn release(engine: &mut Engine<Vec<u8>>, reward: &RewardEvent) {
    let reply = engine.handle(
        Request::Release {
            v: BRIDGE_VERSION,
            request_id: "release-objective-ack-fixture".into(),
            reservation_id: reward.reservation_id.clone(),
        },
        Duration::from_millis(140),
        1,
    );
    assert!(reply.ok, "{:?}", reply.error);
    assert!(
        engine
            .active
            .iter()
            .chain(engine.peers.values())
            .all(|actor| { actor.reservation.id != reward.reservation_id })
    );
}

fn actor_for<'a>(engine: &'a Engine<Vec<u8>>, id: &str) -> &'a Active {
    engine
        .active
        .iter()
        .chain(engine.peers.values())
        .find(|actor| actor.reservation.id == id)
        .unwrap()
}

fn detach_while_keeping_round() -> (Engine<Vec<u8>>, RewardEvent, Character, u32) {
    let mut engine = fixture(3, 4);
    let reward = pending_destruction(&mut engine);
    let original = &actor_for(&engine, &reward.reservation_id).reservation;
    let account = original.account;
    let character = original.character.clone();
    let mut peer = character.clone();
    peer.account_id = Some("synthetic-unrelated-round-peer".into());
    peer.character_id = Some(2);
    reserve(&mut engine, PEER, 1, 2, peer);
    release(&mut engine, &reward);
    assert_eq!(engine.reward_match_id.as_ref(), Some(&reward.match_id));
    (engine, reward, character, account)
}

#[test]
fn detached_ack_preserves_pending_on_record_failure_and_restores_both_counters_once() {
    let (mut engine, reward, character, account) = detach_while_keeping_round();
    let key = (
        reward.world_id,
        reward.account_id.clone(),
        reward.character_id,
    );
    let before = engine.round_participants.get(&key).unwrap().clone();
    assert_eq!(
        engine.pending_reward_batch().as_slice(),
        std::slice::from_ref(&reward)
    );
    let max_log = engine.max_log;
    engine.max_log = engine.written;

    let failed = ack(&mut engine, &reward);
    assert_eq!(failed.error.as_deref(), Some("log_limit_reached"));
    assert!(failed.messages.is_empty());
    assert_eq!(
        engine.pending_reward_batch().as_slice(),
        std::slice::from_ref(&reward)
    );
    assert!(!engine.committed_rewards.contains_key(&reward.reward_id));
    let after_failure = &engine.round_participants[&key];
    assert_eq!(after_failure.match_experience, before.match_experience);
    assert_eq!(after_failure.statistics, before.statistics);

    engine.max_log = max_log;
    engine.failed = false;
    let committed = ack(&mut engine, &reward);
    assert!(committed.ok, "{:?}", committed.error);
    assert!(
        committed.messages.is_empty(),
        "XP routed to an unrelated peer"
    );
    assert!(engine.pending_reward_batch().is_empty());
    let participation = &engine.round_participants[&key];
    assert_eq!(participation.match_experience, 40);
    assert_eq!(participation.statistics.get(&42), Some(&1));
    assert_eq!(participation.statistics.get(&61), Some(&1));
    assert_eq!(participation.statistics.get(&3), Some(&20));
    assert!(actor_for(&engine, PEER).reward_statistics.is_empty());

    reserve(&mut engine, RECONNECTED, 2, account, character);
    let replay = ack(&mut engine, &reward);
    assert!(replay.ok, "{:?}", replay.error);
    assert!(replay.messages.is_empty());
    let mut changed_context = reward;
    changed_context.frago = None;
    let conflict = ack(&mut engine, &changed_context);
    assert_eq!(conflict.error.as_deref(), Some("reward_ack_conflict"));
    assert!(conflict.messages.is_empty());
    assert!(engine.pending_reward_batch().is_empty());
    let reconnected = actor_for(&engine, RECONNECTED);
    assert_eq!(reconnected.match_experience, 40);
    assert_eq!(
        reconnected.reward_statistics,
        [(3, 20), (42, 1), (61, 1)].into_iter().collect()
    );
}

#[test]
fn same_character_replacement_receives_the_pair_only_for_the_original_receipt() {
    let (mut engine, reward, character, account) = detach_while_keeping_round();
    reserve(&mut engine, RECONNECTED, 2, account, character);
    let mut wrong_recipient = reward.clone();
    wrong_recipient.reservation_id = RECONNECTED.into();
    wrong_recipient.player_id = 2;
    let rejected = ack(&mut engine, &wrong_recipient);
    assert_eq!(rejected.error.as_deref(), Some("reward_identity_mismatch"));
    assert!(rejected.messages.is_empty());
    assert_eq!(
        engine.pending_reward_batch().as_slice(),
        std::slice::from_ref(&reward)
    );

    let credited = ack(&mut engine, &reward);
    assert!(credited.ok, "{:?}", credited.error);
    let [message] = credited.messages.as_slice() else {
        panic!(
            "expected one combined stat update, got {:?}",
            credited.messages
        );
    };
    assert_eq!(message.reservation_id, RECONNECTED);
    assert_eq!(message.player_id, 2);
    assert_eq!(
        protocol::decode_hex(&message.payload_hex),
        [2, 4, 0, 2, 4, 2, 40, 42, 1, 61, 1, 3, 20]
    );
    assert!(engine.pending_reward_batch().is_empty());
    assert!(actor_for(&engine, PEER).reward_statistics.is_empty());
}

#[test]
fn previous_round_ack_updates_account_experience_without_touching_new_round_counters() {
    let mut engine = fixture(3, 4);
    let reward = pending_destruction(&mut engine);
    let original = &actor_for(&engine, &reward.reservation_id).reservation;
    let account = original.account;
    let character = original.character.clone();
    release(&mut engine, &reward);
    reserve(&mut engine, RECONNECTED, 2, account, character);
    let actor = engine.active.as_mut().unwrap();
    assert_ne!(actor.reward_match_id, reward.match_id);
    actor.match_experience = 11;
    actor.reward_statistics.extend([(3, 9), (42, 4), (61, 7)]);
    let new_round_stats = actor.reward_statistics.clone();

    let credited = ack(&mut engine, &reward);
    assert!(credited.ok, "{:?}", credited.error);
    assert!(credited.messages.is_empty());
    assert!(engine.pending_reward_batch().is_empty());
    let actor = actor_for(&engine, RECONNECTED);
    assert_eq!(
        actor.reservation.character.progression.unwrap().experience,
        40
    );
    assert_eq!(actor.match_experience, 11);
    assert_eq!(actor.reward_statistics, new_round_stats);
}
