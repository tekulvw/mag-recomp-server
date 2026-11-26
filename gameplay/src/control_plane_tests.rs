use super::*;

// Synthetic sequence from a live incident: a worker keeps running while the
// Horizon process that admitted its players restarts. The new Horizon may give
// the worker's world another ID. Everything the old process admitted is gone
// with its DME transports, so the worker must not refuse the new subscription
// or keep sending frames, rewards and a terminal that name the old world.

const FIRST: &str = "synthetic-control-plane-first";
const SECOND: &str = "synthetic-control-plane-second";

fn journal(engine: &Engine<Vec<u8>>) -> Vec<Value> {
    String::from_utf8_lossy(&engine.writer)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn ops(engine: &Engine<Vec<u8>>, op: &str) -> Vec<Value> {
    journal(engine)
        .into_iter()
        .filter(|row| row["op"] == op)
        .collect()
}

fn pending_reward(engine: &mut Engine<Vec<u8>>, reservation: &str, world: u32) {
    let reward = progression::RewardEvent {
        match_id: "synthetic-match".into(),
        reward_id: format!("synthetic-reward-{reservation}"),
        reservation_id: reservation.into(),
        world_id: world,
        player_id: 0,
        account_id: "synthetic-account".into(),
        character_id: 1,
        kind: progression::RewardKind::EnemyKilled,
        units: 1,
        leadership: None,
        frago: None,
    };
    engine.reward_order.push_back(reward.reward_id.clone());
    engine
        .pending_rewards
        .insert(reward.reward_id.clone(), reward);
}

#[test]
fn the_same_control_plane_keeps_its_players_through_a_reconnect() {
    let mut engine = engine();
    assert_eq!(
        engine.bind_control_plane(FIRST, 1, Duration::ZERO, 1),
        Ok(None)
    );
    reserve(&mut engine, 0);
    reserve(&mut engine, 1);
    assert_eq!(
        engine.bind_control_plane(FIRST, 1, Duration::from_secs(1), 2),
        Ok(None),
        "a resubscription from the same Horizon process must not release anyone"
    );
    assert_eq!(actors(&mut engine).count(), 2);
    assert_eq!(ops(&engine, "control_plane_bound").len(), 1);
}

#[test]
fn a_new_control_plane_releases_the_old_admissions_and_takes_another_world_id() {
    let mut engine = engine();
    engine
        .bind_control_plane(FIRST, 1, Duration::ZERO, 1)
        .unwrap();
    reserve(&mut engine, 0);
    reserve(&mut engine, 1);
    pending_reward(&mut engine, "synthetic-reservation-0000", 1);
    // Before: the old reservations bind the worker to world 1 and refuse world 7.
    assert!(!engine.accepts_subscription_world(7));

    let summary = engine
        .bind_control_plane(SECOND, 7, Duration::from_secs(1), 2)
        .unwrap()
        .expect("held state is released");
    assert_eq!(summary["event"], "control_plane_rebound");
    assert_eq!(summary["released_reservations"], 2);
    assert_eq!(summary["dropped_rewards"], 1);
    assert_eq!(summary["previous_world_ids"], json!([1]));
    assert_eq!(actors(&mut engine).count(), 0);
    assert!(engine.pending_reward_batch().is_empty());
    assert!(engine.accepts_subscription_world(7));

    // The journal keeps the dropped receipt and names each released reservation.
    let rebound = ops(&engine, "control_plane_rebound");
    assert_eq!(rebound.len(), 1);
    assert_eq!(rebound[0]["rewards"].as_array().unwrap().len(), 1);
    let expired = ops(&engine, "expired");
    assert_eq!(expired.len(), 2);
    assert!(
        expired
            .iter()
            .all(|row| row["cause"] == "control_plane_rebound")
    );

    // Old identities stay fenced; the new Horizon admits under fresh ones.
    assert_eq!(
        reserve_identity(&mut engine, 0, 1, "synthetic-reservation-0000")
            .error
            .as_deref(),
        Some("reservation_id_retired")
    );
    let fresh = reserve_identity(&mut engine, 0, 1, "synthetic-reservation-2000");
    assert!(fresh.ok, "{:?}", fresh.error);

    // Its own reconnect afterwards changes nothing.
    assert_eq!(
        engine.bind_control_plane(SECOND, 1, Duration::from_secs(2), 3),
        Ok(None)
    );
    assert_eq!(actors(&mut engine).count(), 1);
}

#[test]
fn a_new_control_plane_moves_a_finished_matchs_terminal_to_its_world() {
    let mut engine = engine();
    engine
        .bind_control_plane(FIRST, 1, Duration::ZERO, 1)
        .unwrap();
    engine.match_terminal = Some(match_flow::MatchTerminal {
        world_id: 1,
        match_id: "synthetic-match".into(),
        reason: "defender_victory".into(),
        reservation_ids: vec!["synthetic-reservation-0000".into()],
    });
    // Every frame carries the terminal; it named the old world.
    engine
        .bind_control_plane(SECOND, 4, Duration::from_secs(1), 2)
        .unwrap()
        .expect("a stale terminal is held state");
    let terminal = engine.match_terminal.as_ref().unwrap();
    assert_eq!(terminal.world_id, 4);
    assert!(terminal.reservation_ids.is_empty());
}

#[test]
fn a_new_control_plane_with_the_same_world_id_still_releases_the_old_admissions() {
    let mut engine = engine();
    reserve(&mut engine, 0);
    // A worker served by a Horizon without control_plane_rebind_v1 has no name
    // yet; the first named subscriber still owns none of its reservations.
    let summary = engine
        .bind_control_plane(SECOND, 1, Duration::from_secs(1), 2)
        .unwrap()
        .expect("held state is released");
    assert_eq!(summary["replaced"], false);
    assert_eq!(actors(&mut engine).count(), 0);
}

#[test]
fn a_malformed_control_plane_is_refused() {
    let mut engine = engine();
    assert_eq!(
        engine.bind_control_plane("short", 1, Duration::ZERO, 1),
        Err("invalid_control_plane")
    );
    assert_eq!(
        engine.bind_control_plane("synthetic control plane", 1, Duration::ZERO, 1),
        Err("invalid_control_plane")
    );
}

#[test]
fn a_draining_worker_refuses_new_admissions_and_says_so_on_frames() {
    let mut engine = engine();
    reserve(&mut engine, 0);
    assert!(engine.begin_drain(1));
    assert!(
        !engine.begin_drain(2),
        "a second signal does not restart the drain"
    );
    assert!(engine.is_draining());
    assert_eq!(engine.drain_humans(), 1);
    assert!(!engine.drained());
    assert_eq!(
        reserve_identity(&mut engine, 1, 1, "synthetic-reservation-0001")
            .error
            .as_deref(),
        Some(worker_drain::REFUSAL)
    );
    engine.enable_world_ticks();
    let frame = engine.tick_world(Duration::from_millis(100), 3, "synthetic-tick".into());
    assert_eq!(frame.worker_draining, Some(true));
    assert_eq!(ops(&engine, "worker_draining").len(), 1);
}
