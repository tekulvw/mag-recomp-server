//! Frozen native role and durable-ack consumers over synthetic authenticated peers.
use super::*;
use crate::match_flow::Outcome;
fn fixture() -> Engine<Vec<u8>> {
    let mut e = crate::strategic_engine_tests::fixture();
    for a in e.active.iter_mut().chain(e.peers.values_mut()) {
        a.reservation.character.leadership_progression_enabled = Some(true);
        a.reservation.character.account_id =
            Some(format!("synthetic-account-{}", a.reservation.player));
        a.reservation.character.progression = Some(progression::ProgressionSnapshot {
            experience: 0,
            level: 16,
            skill_points: 0,
        });
        a.claimed = Some(Claimed {
            wire: vec![],
            np_identity: [0; 36],
            game_info: protocol::GameInfo {
                client_index: a.reservation.player,
                level_name: "synthetic_arena".into(),
                start_time_ms: 0,
                attacking_faction: 0,
                defending_faction: 1,
                maximum_points: [20, 20],
                current_points: [20, 20],
                game_type: 2,
            },
        });
    }
    e
}
fn actor(e: &Engine<Vec<u8>>, player: u16) -> &Active {
    e.active
        .iter()
        .chain(e.peers.values())
        .find(|a| a.reservation.player == player)
        .unwrap()
}
fn reward(e: &Engine<Vec<u8>>, player: u16) -> progression::RewardEvent {
    let a = actor(e, player);
    let recipient = progression::RewardRecipient::from_active(a).unwrap();
    e.match_result_reward(
        a,
        &recipient,
        &Outcome {
            winner: 0,
            major: true,
            deadline: Duration::from_secs(10),
            finalized: false,
        },
    )
}
#[test]
fn terminal_receipts_capture_the_actual_held_native_role_on_both_sides() {
    let e = fixture();
    for (player, role) in [(0, 4), (1, 2), (2, 1), (3, 4), (4, 2), (5, 1)] {
        let r = serde_json::to_value(reward(&e, player)).unwrap();
        assert_eq!(
            r.pointer("/leadership/role").and_then(Value::as_u64),
            Some(role)
        );
        assert_eq!(
            r.pointer("/leadership/baseline/nominations")
                .and_then(Value::as_u64),
            Some(7)
        );
        assert_eq!(
            r["kind"],
            if player < 3 {
                "match_major_defeat"
            } else {
                "match_major_victory"
            }
        );
    }
}

fn tracked_fixture() -> Engine<Vec<u8>> {
    let mut e = fixture();
    let a = crate::active_ability_engine_tests::actor_mut(&mut e, 5);
    a.reservation.character.leadership = Some(leadership_runtime::LeadershipRecord {
        leadership_level: 1,
        nominations: 1,
        matches_since_last_command: 2,
        leadership_points: Some(10),
    });
    a.leadership_level = 1;
    a.nominations = 1;
    e
}
fn ack_value(r: &progression::RewardEvent, points: u32, level: u8, gained: u32) -> Value {
    json!({"v":2,"op":"progression_committed","request_id":"synthetic-lp-ack",
        "reservation_id":r.reservation_id,"world_id":r.world_id,"player_id":r.player_id,"reward_id":r.reward_id,
        "progression":{"experience":0,"level":16,"skill_points":0},"match_experience":0,
        "leadership":{"leadership_level":level,"nominations":1,"matches_since_last_command":2,"leadership_points":points},
        "match_leadership_points":gained,
    })
}
fn ack_request(r: &progression::RewardEvent, points: u32, level: u8, gained: u32) -> Request {
    serde_json::from_value(ack_value(r, points, level, gained)).unwrap()
}

#[test]
fn zero_ordinary_xp_ack_commits_leadership_and_emits_match_points_and_rank() {
    let mut e = tracked_fixture();
    let r = reward(&e, 5);
    e.commit_rewards(vec![r.clone()]);
    let reply = e.handle(ack_request(&r, 17, 2, 7), Duration::from_secs(1), 1);
    assert!(reply.ok, "{:?}", reply.error);
    let a = actor(&e, 5);
    assert_eq!(
        a.reservation
            .character
            .leadership
            .unwrap()
            .leadership_points,
        Some(17)
    );
    assert_eq!(a.leadership_level, 2);
    assert_eq!(a.reward_statistics.get(&7), Some(&7));
    assert!(
        reply
            .messages
            .iter()
            .any(|m| protocol::decode_hex(&m.payload_hex) == [2, 4, 0, 5, 1, 7, 7])
    );
    assert_eq!(a.reservation.character.progression.unwrap().experience, 0);
}

#[test]
fn a_newer_high_bit_point_balance_can_apply_the_native_zero_rank() {
    let mut e = tracked_fixture();
    let a = crate::active_ability_engine_tests::actor_mut(&mut e, 5);
    a.reservation
        .character
        .leadership
        .as_mut()
        .unwrap()
        .leadership_points = Some(i32::MAX as u32);
    a.reservation
        .character
        .leadership
        .as_mut()
        .unwrap()
        .leadership_level = 3;
    a.leadership_level = 3;
    let r = reward(&e, 5);
    e.commit_rewards(vec![r.clone()]);
    let reply = e.handle(
        ack_request(&r, 0x8000_0006, 0, 7),
        Duration::from_secs(1),
        1,
    );
    assert!(reply.ok, "{:?}", reply.error);
    assert_eq!(actor(&e, 5).leadership_level, 0);
    assert_eq!(actor(&e, 5).nominations, 0);
}
#[test]
fn role_changes_after_capture_do_not_rewrite_the_existing_receipt() {
    let mut e = fixture();
    let r = reward(&e, 0);
    e.commit_rewards(vec![r.clone()]);
    crate::active_ability_engine_tests::actor_mut(&mut e, 0).nominations = 0;
    e.reconcile_squad_leaders().unwrap();
    assert_eq!(reward(&e, 0).leadership.unwrap().role, 0);
    assert_eq!(reward(&e, 1).leadership.unwrap().role, 4);
    e.commit_rewards(vec![reward(&e, 0)]);
    assert_eq!(e.pending_reward_batch()[0], r);
}
#[test]
fn a_replacement_reservation_cannot_reuse_the_old_leader_slot_for_points() {
    let mut e = fixture();
    let a = crate::active_ability_engine_tests::actor_mut(&mut e, 0);
    a.reservation.id = "synthetic-replacement-0000".into();
    a.reservation.account = 99;
    a.reservation.character.account_id = Some("synthetic-other-account".into());
    a.reservation.character.character_id = Some(99);
    assert_eq!(reward(&e, 0).leadership.unwrap().role, 0);
}
#[test]
fn failed_ack_record_keeps_points_rank_statistics_and_outbox_for_retry() {
    let mut e = tracked_fixture();
    let r = reward(&e, 5);
    e.commit_rewards(vec![r.clone()]);
    let before = e.pending_reward_batch();
    let history = actor(&e, 5).replication_history.clone();
    let max = e.max_log;
    e.max_log = e.written;
    let response = e.handle(ack_request(&r, 17, 2, 7), Duration::from_secs(1), 1);
    assert!(!response.ok);
    assert_eq!(actor(&e, 5).leadership_level, 1);
    assert_eq!(actor(&e, 5).reward_statistics.get(&7), None);
    assert_eq!(
        actor(&e, 5)
            .reservation
            .character
            .leadership
            .unwrap()
            .leadership_points,
        Some(10)
    );
    assert_eq!(e.pending_reward_batch(), before);
    assert_eq!(actor(&e, 5).replication_history, history);
    e.failed = false;
    e.max_log = max;
    assert!(
        e.handle(ack_request(&r, 17, 2, 7), Duration::from_secs(1), 1)
            .ok
    );
    assert!(
        e.handle(ack_request(&r, 17, 2, 7), Duration::from_secs(1), 1)
            .messages
            .is_empty()
    );
    assert_eq!(actor(&e, 5).reward_statistics.get(&7), Some(&7));
    assert_eq!(
        e.handle(ack_request(&r, 18, 2, 8), Duration::from_secs(1), 1)
            .error
            .as_deref(),
        Some("reward_ack_conflict")
    );
}
#[test]
fn missing_leadership_snapshot_cannot_retire_a_qualified_terminal_reward() {
    let mut e = tracked_fixture();
    let r = reward(&e, 5);
    e.commit_rewards(vec![r.clone()]);
    let mut value = ack_value(&r, 17, 2, 7);
    value.as_object_mut().unwrap().remove("leadership");
    value
        .as_object_mut()
        .unwrap()
        .remove("match_leadership_points");
    let response = e.handle(
        serde_json::from_value(value).unwrap(),
        Duration::from_secs(1),
        1,
    );
    assert_eq!(
        response.error.as_deref(),
        Some("leadership_result_snapshot_required")
    );
    assert_eq!(e.pending_reward_batch(), [r]);
}

#[test]
fn same_character_reconnect_routes_the_frozen_award_to_its_new_reservation() {
    let mut e = tracked_fixture();
    let r = reward(&e, 5);
    e.commit_rewards(vec![r.clone()]);
    let a = crate::active_ability_engine_tests::actor_mut(&mut e, 5);
    a.reservation.id = "synthetic-reconnected-0009".into();
    a.reservation.player = 9;
    let reply = e.handle(ack_request(&r, 17, 2, 7), Duration::from_secs(1), 1);
    assert!(reply.ok, "{:?}", reply.error);
    assert_eq!(
        actor(&e, 9)
            .reservation
            .character
            .leadership
            .unwrap()
            .leadership_points,
        Some(17)
    );
    assert!(
        reply
            .messages
            .iter()
            .any(|m| m.reservation_id == "synthetic-reconnected-0009"
                && m.player_id == 9
                && protocol::decode_hex(&m.payload_hex) == [2, 4, 0, 9, 1, 7, 7])
    );
}
#[test]
fn a_different_character_reusing_the_slot_gets_no_leadership_from_an_old_ack() {
    let mut e = tracked_fixture();
    let r = reward(&e, 5);
    e.commit_rewards(vec![r.clone()]);
    let a = crate::active_ability_engine_tests::actor_mut(&mut e, 5);
    a.reservation.character.account_id = Some("synthetic-unrelated".into());
    a.reservation.character.character_id = Some(99);
    let reply = e.handle(ack_request(&r, 17, 2, 7), Duration::from_secs(1), 1);
    assert!(reply.ok, "{:?}", reply.error);
    assert_eq!(
        actor(&e, 5)
            .reservation
            .character
            .leadership
            .unwrap()
            .leadership_points,
        Some(10)
    );
    assert_eq!(actor(&e, 5).reward_statistics.get(&7), None);
    assert!(reply.messages.is_empty());
}
#[test]
fn older_saved_snapshots_and_previous_round_receipts_do_not_regress_current_state() {
    for old_round in [false, true] {
        let mut e = tracked_fixture();
        let mut r = reward(&e, 5);
        if old_round {
            r.match_id = "synthetic-older-round".into();
        }
        e.commit_rewards(vec![r.clone()]);
        let a = crate::active_ability_engine_tests::actor_mut(&mut e, 5);
        a.reservation
            .character
            .leadership
            .as_mut()
            .unwrap()
            .leadership_points = Some(20);
        a.leadership_level = 3;
        let reply = e.handle(ack_request(&r, 17, 2, 7), Duration::from_secs(1), 1);
        assert!(reply.ok, "{:?}", reply.error);
        assert_eq!(
            actor(&e, 5)
                .reservation
                .character
                .leadership
                .unwrap()
                .leadership_points,
            Some(20)
        );
        assert_eq!(actor(&e, 5).leadership_level, 3);
        if old_round {
            assert_eq!(actor(&e, 5).reward_statistics.get(&7), None);
        }
    }
}
#[test]
fn ordinary_xp_can_publish_a_rank_unlock_without_match_leadership_points() {
    let mut e = tracked_fixture();
    let a = crate::active_ability_engine_tests::actor_mut(&mut e, 5);
    a.reservation.character.progression = Some(progression::ProgressionSnapshot {
        experience: 10,
        level: 14,
        skill_points: 0,
    });
    a.reservation
        .character
        .leadership
        .as_mut()
        .unwrap()
        .leadership_points = Some(0);
    a.reservation
        .character
        .leadership
        .as_mut()
        .unwrap()
        .leadership_level = 0;
    a.leadership_level = 0;
    a.nominations = 0;
    let recipient = progression::RewardRecipient::from_active(a).unwrap();
    let r = recipient.event(
        "synthetic-level-unlock".into(),
        progression::RewardKind::EnemyKilled,
        1,
    );
    e.commit_rewards(vec![r.clone()]);
    let mut value = ack_value(&r, 0, 1, 0);
    value
        .as_object_mut()
        .unwrap()
        .remove("match_leadership_points");
    value["progression"] = json!({"experience":11,"level":15,"skill_points":0});
    value["match_experience"] = json!(1);
    let reply = e.handle(
        serde_json::from_value(value).unwrap(),
        Duration::from_secs(1),
        1,
    );
    assert!(reply.ok, "{:?}", reply.error);
    assert_eq!(actor(&e, 5).leadership_level, 1);
    assert_eq!(
        actor(&e, 5)
            .reservation
            .character
            .progression
            .unwrap()
            .level,
        15
    );
    assert_eq!(actor(&e, 5).reward_statistics.get(&7), None);
}
#[test]
fn initial_context_preserves_raw_admission_fallback_without_copying_live_nomination_mask() {
    let mut e = fixture();
    let a = crate::active_ability_engine_tests::actor_mut(&mut e, 5);
    a.reservation.character.leadership = None;
    a.nominations = 0;
    let value = serde_json::to_value(reward(&e, 5)).unwrap();
    let baseline = value["leadership"]["baseline"].as_object().unwrap();
    assert_eq!(baseline.len(), 3);
    assert_eq!(baseline["nominations"], 7);
    assert_eq!(baseline["leadership_level"], 3);
}
#[test]
fn invalid_or_incomplete_snapshot_keeps_the_terminal_receipt_pending() {
    for case in 0..3 {
        let mut e = tracked_fixture();
        let r = reward(&e, 5);
        e.commit_rewards(vec![r.clone()]);
        let mut value = ack_value(&r, 17, 2, 7);
        match case {
            0 => {
                value["leadership"]
                    .as_object_mut()
                    .unwrap()
                    .remove("leadership_points");
            }
            1 => value["match_leadership_points"] = json!(18),
            _ => value["match_leadership_points"] = json!(u32::MAX),
        }
        let reply = e.handle(
            serde_json::from_value(value).unwrap(),
            Duration::from_secs(1),
            1,
        );
        assert!(!reply.ok);
        assert_eq!(e.pending_reward_batch(), [r]);
        assert_eq!(actor(&e, 5).leadership_level, 1);
    }
}

#[test]
fn owned_round_commits_frozen_terminal_context_once_and_rolls_back_record_failure() {
    let mut e = fixture();
    e.match_epoch = Some(Duration::ZERO);
    let mut round = acquisition_round::Round::new(acquisition_round::Rules {
        starting_points: [30, 20],
        major_percent: 50,
        end_delay_ms: 1000,
    });
    round
        .apply_score(acquisition_round::ScoreIntent {
            team: 1,
            delta: -20,
        })
        .unwrap();
    e.domination_round = Some(round);
    let limit = e.max_log;
    e.max_log = e.written;
    assert_eq!(
        e.tick_owned_round(
            acquisition_match_flow::OwnedRound::Domination,
            Duration::from_secs(1),
            1
        )
        .unwrap_err(),
        "log_limit_reached"
    );
    assert!(e.pending_reward_batch().is_empty());
    assert!(e.round.outcome.is_none());
    e.failed = false;
    e.max_log = limit;
    e.tick_owned_round(
        acquisition_match_flow::OwnedRound::Domination,
        Duration::from_secs(1),
        1,
    )
    .unwrap();
    let first = e.pending_reward_batch();
    assert_eq!(first.len(), 6);
    for r in &first {
        assert!(r.leadership.is_some());
        assert_eq!(
            r.leadership.unwrap().role,
            match r.player_id {
                0 | 3 => 4,
                1 | 4 => 2,
                _ => 1,
            }
        );
    }
    e.tick_owned_round(
        acquisition_match_flow::OwnedRound::Domination,
        Duration::from_millis(1050),
        1,
    )
    .unwrap();
    assert_eq!(e.pending_reward_batch(), first);
}
#[test]
fn zero_leadership_award_still_publishes_an_explicit_match_stat() {
    let mut e = tracked_fixture();
    let mut r = reward(&e, 5);
    r.leadership.as_mut().unwrap().role = 0;
    e.commit_rewards(vec![r.clone()]);
    let reply = e.handle(ack_request(&r, 10, 1, 0), Duration::from_secs(1), 1);
    assert!(reply.ok, "{:?}", reply.error);
    assert!(
        reply
            .messages
            .iter()
            .any(|m| protocol::decode_hex(&m.payload_hex) == [2, 4, 0, 5, 1, 7, 0])
    );
    assert_eq!(actor(&e, 5).reward_statistics.get(&7), Some(&0));
}

#[test]
fn leadership_rank_delta_accepts_a_character_level_from_the_retail_catalog() {
    let mut e = tracked_fixture();
    crate::active_ability_engine_tests::actor_mut(&mut e, 5)
        .reservation
        .character
        .progression
        .as_mut()
        .unwrap()
        .level = 60;
    let r = reward(&e, 5);
    e.commit_rewards(vec![r.clone()]);
    let mut value = ack_value(&r, 17, 2, 7);
    value["progression"]["level"] = json!(60);
    let reply = e.handle(
        serde_json::from_value(value).unwrap(),
        Duration::from_secs(1),
        1,
    );
    assert!(reply.ok, "{:?}", reply.error);
    assert_eq!(actor(&e, 5).leadership_level, 2);
}

#[test]
fn an_old_round_ack_refreshes_newer_account_state_without_new_round_points() {
    let mut e = tracked_fixture();
    let mut r = reward(&e, 5);
    r.match_id = "synthetic-deferred-round".into();
    e.commit_rewards(vec![r.clone()]);
    let a = crate::active_ability_engine_tests::actor_mut(&mut e, 5);
    a.match_experience = 9;
    a.reward_statistics.insert(7, 2);
    a.reservation
        .character
        .progression
        .as_mut()
        .unwrap()
        .experience = 20;
    let mut value = ack_value(&r, 17, 2, 7);
    value["progression"]["experience"] = json!(30);
    value["match_experience"] = json!(100);
    let reply = e.handle(
        serde_json::from_value(value).unwrap(),
        Duration::from_secs(1),
        1,
    );
    assert!(reply.ok, "{:?}", reply.error);
    let a = actor(&e, 5);
    assert_eq!(a.reservation.character.progression.unwrap().experience, 30);
    assert_eq!(
        a.reservation
            .character
            .leadership
            .unwrap()
            .leadership_points,
        Some(17)
    );
    assert_eq!(a.leadership_level, 2);
    assert_eq!(a.match_experience, 9);
    assert_eq!(a.reward_statistics.get(&7), Some(&2));
    assert_eq!(a.reward_statistics.get(&4), None);
}
#[test]
fn legacy_admission_without_leadership_progression_support_retains_xp_only_receipts() {
    let mut e = fixture();
    let a = crate::active_ability_engine_tests::actor_mut(&mut e, 5);
    let old = &a.reservation.character;
    a.reservation.character = serde_json::from_value(
        json!({"name":"Synthetic Legacy","faction":old.faction,"head":0,"voice":0,"universe_id":26,
            "account_id":old.account_id,"character_id":old.character_id,
            "leadership":{"leadership_level":3,"nominations":7,"matches_since_last_command":0}
        }),
    )
    .unwrap();
    assert!(reward(&e, 5).leadership.is_none());
}
