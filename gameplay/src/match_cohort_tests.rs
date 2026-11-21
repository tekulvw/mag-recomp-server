//! Synthetic terminal members may finish results after the next cohort starts.
use super::*;

fn release(e: &mut Engine<Vec<u8>>, id: &str) -> Response {
    e.handle(
        Request::Release {
            v: BRIDGE_VERSION,
            request_id: "cohort-release".into(),
            reservation_id: id.into(),
        },
        Duration::from_secs(3),
        3,
    )
}
fn terminal() -> Engine<Vec<u8>> {
    let mut e = engine();
    reserve(&mut e, 0);
    reserve(&mut e, 1);
    for a in actors(&mut e) {
        ready(a);
        a.match_experience = 23;
        a.reservation.character.account_id =
            Some(format!("synthetic-account-{}", a.reservation.player));
        a.claimed = Some(Claimed {
            wire: vec![],
            np_identity: [0; 36],
            game_info: protocol::GameInfo {
                client_index: a.reservation.player,
                level_name: "synthetic_arena".into(),
                start_time_ms: 0,
                attacking_faction: 1,
                defending_faction: 0,
                maximum_points: [1000, 1000],
                current_points: [0, 1000],
                game_type: 0,
            },
        });
    }
    let mut next = e.round.clone();
    next.scores = Some([0, 1000]);
    next.outcome = Some(crate::match_flow::Outcome {
        winner: 1,
        major: true,
        deadline: Duration::from_secs(5),
        finalized: false,
    });
    e.commit_round_update(next, vec![], vec![], Duration::from_secs(2), 2)
        .unwrap();
    e.publish_round_update(
        Duration::from_secs(2),
        crate::match_flow::ScorePublication::Sabotage {
            objective_effect: false,
        },
    )
    .unwrap();
    e
}

#[test]
fn new_cohort_admits_while_old_result_peer_stays_connected() {
    let mut e = terminal();
    assert!(release(&mut e, "synthetic-reservation-0000").ok);
    let response = reserve_identity(&mut e, 0, 1, "synthetic-next-cohort-0000");
    assert!(response.ok, "next cohort blocked: {:?}", response.error);
    let next = e.active.as_ref().unwrap();
    assert_eq!(next.reservation.id, "synthetic-next-cohort-0000");
    assert_eq!(next.match_experience, 0);
    assert!(e.round.outcome.is_none());
    assert!(
        e.peers.is_empty(),
        "old result member entered the fresh gameplay world"
    );
    // The old release is still acknowledged while its old slot has a new owner.
    assert!(release(&mut e, "synthetic-reservation-0001").ok);
    assert_eq!(
        e.active.as_ref().unwrap().reservation.id,
        "synthetic-next-cohort-0000"
    );
    assert!(release(&mut e, "synthetic-reservation-0001").ok);
}

fn roll(e: &mut Engine<Vec<u8>>) -> String {
    let key = e.reward_match_id.clone().unwrap();
    assert!(release(e, "synthetic-reservation-0000").ok);
    assert!(reserve_identity(e, 0, 1, "synthetic-next-cohort-0000").ok);
    key
}
fn old_event(e: &mut Engine<Vec<u8>>, world: u32, player: u16, wire: &[u8], now: u64) -> Response {
    e.handle(
        Request::Event {
            v: BRIDGE_VERSION,
            request_id: "old-cohort-event".into(),
            reservation_id: "synthetic-reservation-0001".into(),
            world_id: world,
            player_id: player,
            transport: Transport::Udp,
            direction: Direction::ClientToServer,
            rt_type: 0x3d,
            plugin_id: Some(1),
            payload_hex: protocol::encode_hex(wire),
        },
        Duration::from_secs(now),
        now,
    )
}

#[test]
fn old_gameplay_and_identity_mismatch_cannot_touch_successor() {
    let mut e = terminal();
    let key = roll(&mut e);
    let active = e.active.as_mut().unwrap();
    active.life = CharacterLife::Alive {
        generation: 1,
        health_quarter_points: 400,
    };
    let wire = [0x02, 0x15, 0x03, 0x80, 0x00, 0x00];
    assert_eq!(
        old_event(&mut e, 1, 0, &wire, 4).error.as_deref(),
        Some("identity_mismatch")
    );
    assert_eq!(
        old_event(&mut e, 2, 1, &wire, 4).error.as_deref(),
        Some("identity_mismatch")
    );
    assert!(old_event(&mut e, 1, 1, &wire, 4).ok);
    assert!(matches!(
        e.active.as_ref().unwrap().life,
        CharacterLife::Alive {
            health_quarter_points: 400,
            ..
        }
    ));
    assert!(e.round.outcome.is_none());
    assert_eq!(e.results_cohorts[&key].actors.len(), 1);
    assert!(e.pending_rewards.is_empty());
}

#[test]
fn old_results_keep_order_deadline_and_exact_replay_after_slot_reuse() {
    let mut e = terminal();
    let key = roll(&mut e);
    // Finish the archived countdown while the successor remains unstarted.
    let messages = e
        .tick_results_cohorts(Duration::from_secs(6), 6, vec![])
        .unwrap();
    assert!(!messages.is_empty());
    assert!(
        messages
            .iter()
            .all(|m| m.reservation_id == "synthetic-reservation-0001")
    );
    let old = &e.results_cohorts[&key].actors["synthetic-reservation-0001"];
    assert!(old.end_announced && old.results_announced && old.postgame_announced);
    let replay = old.replication_history.last().unwrap().clone();
    let sequence = replication::ObjectUpdate::decode_with_profile(&replay, e.wire_profile)
        .unwrap()
        .header
        .sequence;
    let request = replication::ObjectUpdate {
        header: replication::UpdateHeader {
            sequence: 90,
            nak_sequence: sequence,
            client_index: 0,
            object_type_mask: 0,
            timestamp_ms: 7000,
        },
        wire: &[],
    }
    .encode_empty_with_profile(e.wire_profile)
    .unwrap();
    let response = old_event(&mut e, 1, 1, &request, 7);
    assert!(response.ok, "{:?}", response.error);
    assert_eq!(response.messages.len(), 1);
    assert_eq!(
        response.messages[0].payload_hex,
        protocol::encode_hex(&replay)
    );
    assert_eq!(
        e.results_cohorts[&key].actors["synthetic-reservation-0001"].outbound_sequence,
        sequence
    );
    assert_eq!(e.active.as_ref().unwrap().outbound_sequence, 0);
}

#[test]
fn rollover_record_failure_preserves_old_cohort_and_retry_installs_once() {
    let mut e = terminal();
    assert!(release(&mut e, "synthetic-reservation-0000").ok);
    let before = e.reward_match_id.clone();
    let max = e.max_log;
    e.max_log = e.written;
    let failed = reserve_identity(&mut e, 0, 1, "synthetic-next-cohort-0000");
    assert_eq!(failed.error.as_deref(), Some("log_limit_reached"));
    assert!(e.results_cohorts.is_empty());
    assert_eq!(e.reward_match_id, before);
    assert!(e.match_terminal.is_some());
    e.failed = false;
    e.max_log = max;
    assert!(reserve_identity(&mut e, 0, 1, "synthetic-next-cohort-0000").ok);
    assert_eq!(e.results_cohorts.len(), 1);
    assert!(reserve_identity(&mut e, 0, 1, "synthetic-next-cohort-0000").ok);
    assert_eq!(e.results_cohorts.len(), 1);
}

#[test]
fn archived_loadout_receipt_survives_selection_settlement_without_equipping_successor() {
    let mut e = terminal();
    let old = e.active.as_mut().unwrap();
    assert_eq!(old.reservation.player, 1);
    let receipt = loadout_selection::CommittedLoadoutSelection {
        selection_id: "old-desired-kit".into(),
        reservation_id: old.reservation.id.clone(),
        world_id: 1,
        player_id: 1,
        account_id: "synthetic-kit-account".into(),
        character_id: 2,
        revision: 8,
        inventory_index: 3,
    };
    old.pending_loadout_selection = Some(receipt.clone());
    roll(&mut e);
    assert_eq!(e.pending_loadout_selections(), vec![receipt.clone()]);
    let reply = e.handle(
        Request::LoadoutSelectionSettled {
            v: BRIDGE_VERSION,
            request_id: "old-kit-ack".into(),
            selection_id: receipt.selection_id,
            reservation_id: receipt.reservation_id,
            world_id: 1,
            player_id: 1,
            revision: 8,
            status: loadout_selection::SelectionStatus::Persisted,
        },
        Duration::from_secs(4),
        4,
    );
    assert!(reply.ok);
    assert!(e.pending_loadout_selections().is_empty());
    assert_eq!(e.active.as_ref().unwrap().selected_loadout, None);
    assert!(
        e.active
            .as_ref()
            .unwrap()
            .pending_loadout_selection
            .is_none()
    );
}

#[test]
fn wrong_world_cannot_be_admitted_while_results_use_original_subscription() {
    let mut e = terminal();
    roll(&mut e);
    // Release the live successor; the older results connection still owns world1.
    assert!(release(&mut e, "synthetic-next-cohort-0000").ok);
    let request = Request::Reserve {
        v: BRIDGE_VERSION,
        request_id: "wrong-world".into(),
        reservation_id: "synthetic-wrong-world".into(),
        world_id: 2,
        player_id: 0,
        account_id: 1,
        session_key: "aaaaaaaaaaaaaaaa".into(),
        access_key: "bbbbbbbbbbbbbbbb".into(),
        character: serde_json::from_value(
            json!({"name":"Wrong World","faction":1,"head":0,"voice":0,"universe_id":26}),
        )
        .unwrap(),
    };
    assert_eq!(
        e.handle(request, Duration::from_secs(4), 4)
            .error
            .as_deref(),
        Some("reservation_conflict")
    );
    assert!(!e.accepts_subscription_world(2));
    assert!(e.accepts_subscription_world(1));
}

#[test]
fn silent_results_expire_without_removing_live_successor() {
    let mut e = terminal();
    roll(&mut e);
    let messages = e
        .tick_results_cohorts(Duration::from_secs(121), 121, vec![])
        .unwrap();
    assert!(messages.is_empty());
    assert!(e.results_cohorts.is_empty());
    assert_eq!(
        e.active.as_ref().unwrap().reservation.id,
        "synthetic-next-cohort-0000"
    );
    assert!(release(&mut e, "synthetic-reservation-0001").ok);
    assert_eq!(
        reserve_identity(&mut e, 1, 1, "synthetic-reservation-0001")
            .error
            .as_deref(),
        Some("reservation_id_retired")
    );
}

#[test]
fn results_retention_bound_refuses_rollover_then_recovers_after_old_release() {
    let mut e = terminal();
    let mut current = "synthetic-next-cohort-0000".to_owned();
    assert!(reserve_identity(&mut e, 0, 1, &current).ok);
    for index in 1..8 {
        let mut next = e.round.clone();
        next.scores = Some([0, 1000]);
        next.outcome = Some(crate::match_flow::Outcome {
            winner: 1,
            major: false,
            deadline: Duration::from_secs(5),
            finalized: false,
        });
        e.commit_round_update(next, vec![], vec![], Duration::from_secs(2), 2)
            .unwrap();
        current = format!("synthetic-next-cohort-{index:04}");
        assert!(reserve_identity(&mut e, 0, 1, &current).ok);
    }
    assert_eq!(e.results_cohorts.len(), 8);
    let mut next = e.round.clone();
    next.scores = Some([0, 1000]);
    next.outcome = Some(crate::match_flow::Outcome {
        winner: 1,
        major: false,
        deadline: Duration::from_secs(5),
        finalized: false,
    });
    e.commit_round_update(next, vec![], vec![], Duration::from_secs(2), 2)
        .unwrap();
    let old_match = e.reward_match_id.clone();
    assert_eq!(
        reserve_identity(&mut e, 0, 1, "synthetic-ninth-next")
            .error
            .as_deref(),
        Some("results_cohort_limit")
    );
    assert_eq!(e.reward_match_id, old_match);
    assert_eq!(e.active.as_ref().unwrap().reservation.id, current);
    assert!(release(&mut e, "synthetic-reservation-0000").ok);
    assert!(release(&mut e, "synthetic-reservation-0001").ok);
    assert!(reserve_identity(&mut e, 0, 1, "synthetic-ninth-next").ok);
    assert_eq!(e.results_cohorts.len(), 8);
}

#[test]
fn terminal_results_are_gated_by_own_rewards_instead_of_other_cohorts() {
    let mut e = terminal();
    let key = roll(&mut e);
    let pending = progression::RewardEvent {
        match_id: key.clone(),
        reward_id: "old-cohort-outstanding".into(),
        reservation_id: "synthetic-reservation-0001".into(),
        world_id: 1,
        player_id: 1,
        account_id: "synthetic-old-rewards".into(),
        character_id: 2,
        kind: progression::RewardKind::MatchMinorVictory,
        units: 1,
        leadership: None,
        frago: None,
    };
    e.commit_rewards(vec![pending]);
    // A fresh participant is ready in a separately completed round. The pending
    // old receipt cannot hold its final statistics or PostGame phase hostage.
    let a = e.active.as_mut().unwrap();
    ready(a);
    a.claimed = Some(Claimed {
        wire: vec![],
        np_identity: [0; 36],
        game_info: protocol::GameInfo {
            client_index: 0,
            level_name: "synthetic_arena".into(),
            start_time_ms: 0,
            attacking_faction: 1,
            defending_faction: 0,
            maximum_points: [1000, 1000],
            current_points: [0, 1000],
            game_type: 0,
        },
    });
    let mut next = e.round.clone();
    next.scores = Some([0, 1000]);
    next.outcome = Some(crate::match_flow::Outcome {
        winner: 1,
        major: false,
        deadline: Duration::from_secs(4),
        finalized: true,
    });
    e.commit_round_update(next, vec![], vec![], Duration::from_secs(4), 4)
        .unwrap();
    e.publish_round_update(
        Duration::from_secs(4),
        crate::match_flow::ScorePublication::Sabotage {
            objective_effect: false,
        },
    )
    .unwrap();
    assert!(
        e.active.as_ref().unwrap().results_announced
            && e.active.as_ref().unwrap().postgame_announced
    );
    assert_eq!(e.pending_reward_batch()[0].match_id, key);
}

#[test]
fn full_result_fifo_defers_postgame_instead_of_marking_it_delivered() {
    let mut e = terminal();
    let key = e.reward_match_id.clone().unwrap();
    let a = e.active.as_mut().unwrap();
    assert_eq!(a.reservation.player, 1);
    a.pending_notifications.clear();
    a.pending_shared.clear();
    for _ in 0..128 {
        a.pending_notifications
            .push_back(crate::match_flow::Publication::Application(
                protocol::CharacterStatsComplete.encode(),
            ));
    }
    assert!(reserve_identity(&mut e, 0, 1, "synthetic-next-cohort-0000").ok);
    e.tick_results_cohorts(Duration::from_secs(6), 6, vec![])
        .unwrap();
    let old = &e.results_cohorts[&key].actors["synthetic-reservation-0001"];
    assert!(
        !old.postgame_announced,
        "a full queue cannot mean PostGame was queued"
    );
    let response = e
        .tick_results_cohorts(Duration::from_secs(7), 7, vec![])
        .unwrap();
    assert!(e.results_cohorts[&key].actors["synthetic-reservation-0001"].postgame_announced);
    assert!(
        response
            .iter()
            .any(|m| m.reservation_id == "synthetic-reservation-0001")
    );
}

#[test]
fn archived_reply_reports_its_own_terminal_instead_of_successor_terminal() {
    let mut e = terminal();
    let key = roll(&mut e);
    let original = e.results_cohorts[&key].terminal.match_id.clone();
    let mut next = e.round.clone();
    next.scores = Some([0, 1000]);
    next.outcome = Some(crate::match_flow::Outcome {
        winner: 1,
        major: false,
        deadline: Duration::from_secs(5),
        finalized: false,
    });
    e.commit_round_update(next, vec![], vec![], Duration::from_secs(4), 4)
        .unwrap();
    assert_ne!(e.match_terminal.as_ref().unwrap().match_id, original);
    let response = old_event(&mut e, 1, 1, &[0x02, 0x15, 0x03, 0x80, 0, 0], 4);
    assert!(response.ok);
    assert_eq!(response.match_terminal.unwrap().match_id, original);
}

#[test]
fn released_and_expired_results_keep_match_statistics_for_late_receipts() {
    for expire in [false, true] {
        let mut e = terminal();
        e.active.as_mut().unwrap().reward_statistics.insert(12, 7);
        let key = roll(&mut e);
        e.commit_rewards(vec![progression::RewardEvent {
            match_id: key.clone(),
            reward_id: "late-history-reward".into(),
            reservation_id: "synthetic-reservation-0001".into(),
            world_id: 1,
            player_id: 1,
            account_id: "synthetic-account-1".into(),
            character_id: 2,
            kind: progression::RewardKind::EnemyKilled,
            units: 1,
            leadership: None,
            frago: None,
        }]);
        if expire {
            e.tick_results_cohorts(Duration::from_secs(121), 121, vec![])
                .unwrap();
        } else {
            assert!(release(&mut e, "synthetic-reservation-0001").ok);
        }
        // Keep this snapshot until its deferred reward is acknowledged. An
        // ordinary release also preserves it immediately before later pruning.
        {
            let p = &e.results_cohorts[&key].participants[&(1, "synthetic-account-1".into(), 2)];
            assert_eq!(p.match_experience, 23);
            assert_eq!(p.statistics.get(&12), Some(&7));
        }
    }
}

#[test]
fn partial_reservation_log_budget_never_installs_terminal_rollover() {
    let mut e = terminal();
    assert!(release(&mut e, "synthetic-reservation-0000").ok);
    let original = e.reward_match_id.clone();
    // Enough for the old separate archive record, insufficient for this saved
    // loadout admission. The reservation and rollover must share one commit.
    e.max_log = e.written + 512;
    let reply = reserve_identity(&mut e, 0, 1, "synthetic-next-cohort-0000");
    assert_eq!(reply.error.as_deref(), Some("log_limit_reached"));
    assert!(e.results_cohorts.is_empty());
    assert_eq!(e.reward_match_id, original);
    assert!(e.match_terminal.is_some());
}

#[test]
fn final_terminal_release_preserves_receipts_history_and_old_identity_retirement() {
    let mut e = terminal();
    let original = e.reward_match_id.clone().unwrap();
    e.commit_rewards(vec![progression::RewardEvent {
        match_id: original.clone(),
        reward_id: "sole-terminal-pending".into(),
        reservation_id: "synthetic-reservation-0001".into(),
        world_id: 1,
        player_id: 1,
        account_id: "synthetic-account-1".into(),
        character_id: 2,
        kind: progression::RewardKind::EnemyKilled,
        units: 1,
        leadership: None,
        frago: None,
    }]);
    assert!(release(&mut e, "synthetic-reservation-0000").ok);
    assert!(release(&mut e, "synthetic-reservation-0001").ok);
    assert_eq!(e.reward_match_id.as_deref(), Some(original.as_str()));
    assert!(e.match_terminal.is_some());
    assert_eq!(
        reserve_identity(&mut e, 0, 1, "synthetic-reservation-0000")
            .error
            .as_deref(),
        Some("reservation_id_retired")
    );
    assert!(reserve_identity(&mut e, 0, 1, "synthetic-next-cohort-0000").ok);
    assert_eq!(
        e.results_cohorts[&original].participants[&(1, "synthetic-account-1".into(), 2)]
            .match_experience,
        23
    );
}

#[test]
fn subscribed_results_are_drained_only_by_the_ordered_world_stream() {
    let mut e = terminal();
    let key = roll(&mut e);
    e.enable_world_ticks();
    let reply = old_event(&mut e, 1, 1, &[0x02, 0x15, 0x03, 0x80, 0, 0], 6);
    assert!(reply.ok);
    assert!(
        reply.messages.is_empty(),
        "event RPC consumed the stream's pending terminal FIFO"
    );
    assert!(!e.results_cohorts[&key].actors["synthetic-reservation-0001"].postgame_announced);
    let tick = e.tick_world(Duration::from_secs(6), 6, "ordered-results-tick".into());
    assert!(tick.ok, "{:?}", tick.error);
    assert!(
        tick.messages
            .iter()
            .any(|m| m.reservation_id == "synthetic-reservation-0001")
    );
}

#[test]
fn archived_event_decodes_with_its_original_wire_profile() {
    let mut e = terminal();
    let key = roll(&mut e);
    let profile = e.results_cohorts[&key].profile;
    e.wire_profile = wire_profile::WireProfile::prototype();
    let mut request = profile.wire_message_type(0x000a).to_be_bytes().to_vec();
    request.extend_from_slice(&123u32.to_be_bytes());
    let reply = old_event(&mut e, 1, 1, &request, 4);
    assert!(reply.ok);
    let want = protocol::encode_hex(
        &protocol::TimeResponse {
            server_time_ms: 4000,
            client_time_ms: 123,
        }
        .encode_with_profile(profile),
    );
    assert!(reply.messages.iter().any(|m| m.payload_hex == want));
}

#[test]
fn duplicate_old_reward_ack_never_announces_successors_terminal() {
    let mut e = terminal();
    let old = e.reward_match_id.clone().unwrap();
    let reward = progression::RewardEvent {
        match_id: old,
        reward_id: "old-metadata-reward".into(),
        reservation_id: "synthetic-reservation-0001".into(),
        world_id: 1,
        player_id: 1,
        account_id: "synthetic-account-1".into(),
        character_id: 2,
        kind: progression::RewardKind::EnemyKilled,
        units: 1,
        leadership: None,
        frago: None,
    };
    e.commit_rewards(vec![reward.clone()]);
    roll(&mut e);
    let request = || Request::ProgressionCommitted {
        v: BRIDGE_VERSION,
        request_id: "old-metadata-ack".into(),
        reservation_id: reward.reservation_id.clone(),
        world_id: 1,
        player_id: 1,
        reward_id: reward.reward_id.clone(),
        progression: progression::ProgressionSnapshot {
            experience: 30,
            level: 1,
            skill_points: 0,
        },
        match_experience: 30,
        leadership: None,
        match_leadership_points: None,
        match_frago_points: None,
    };
    assert!(e.handle(request(), Duration::from_secs(4), 4).ok);
    let mut next = e.round.clone();
    next.scores = Some([0, 1000]);
    next.outcome = Some(crate::match_flow::Outcome {
        winner: 1,
        major: false,
        deadline: Duration::from_secs(5),
        finalized: false,
    });
    e.commit_round_update(next, vec![], vec![], Duration::from_secs(4), 4)
        .unwrap();
    let reply = e.handle(request(), Duration::from_secs(4), 4);
    assert!(reply.ok);
    assert!(reply.messages.is_empty());
    assert!(reply.match_terminal.is_none());
}
