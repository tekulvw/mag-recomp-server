use super::*;

// Synthetic two-human sequence: gameplay evicts one client for a replication
// backlog while another keeps the shared world alive. Horizon only learns of
// the eviction through its next event, which is refused; its release must then
// be acknowledged so the account is not held pending, and the same account must
// be able to rejoin under a fresh reservation id.
fn evicted_event(engine: &mut Engine<Vec<u8>>, player: u16, now: Duration) -> Response {
    let request = parse_request(
        json!({"v":BRIDGE_VERSION,"op":"event","request_id":"synthetic-evicted-event",
        "reservation_id":format!("synthetic-reservation-{player:04}"),"world_id":1,
        "player_id":player,"transport":"tcp","direction":"client_to_server",
        "rt_type":61,"plugin_id":1,"payload_hex":"021c03800000"})
        .to_string()
        .as_bytes(),
    )
    .unwrap();
    engine.handle(request, now, 1)
}
fn release(engine: &mut Engine<Vec<u8>>, reservation_id: &str, now: Duration) -> Response {
    engine.handle(
        Request::Release {
            v: BRIDGE_VERSION,
            request_id: format!("synthetic-release-{reservation_id}"),
            reservation_id: reservation_id.into(),
        },
        now,
        1,
    )
}
fn present(engine: &mut Engine<Vec<u8>>, player: u16) -> bool {
    actors(engine).any(|a| a.reservation.player == player)
}

#[test]
fn backlog_evicted_reservation_release_is_acknowledged_and_account_rejoins() {
    let mut engine = engine();
    reserve(&mut engine, 0);
    reserve(&mut engine, 1);
    actors(&mut engine)
        .find(|a| a.reservation.player == 1)
        .unwrap()
        .replication_backlogged = true;

    let refused = evicted_event(&mut engine, 1, Duration::from_secs(1));
    assert!(!refused.ok, "an evicted reservation must not accept events");
    assert!(!present(&mut engine, 1));
    assert!(present(&mut engine, 0), "the other human keeps the world");

    // Before the fix this was Err("identity_mismatch"), which Horizon treats as
    // a definitive rejection and never retries, blocking the account's joins.
    let ack = release(
        &mut engine,
        "synthetic-reservation-0001",
        Duration::from_secs(1),
    );
    assert!(ack.ok, "{:?}", ack.error);
    let replay = release(
        &mut engine,
        "synthetic-reservation-0001",
        Duration::from_secs(2),
    );
    assert!(replay.ok, "release acknowledgement must be idempotent");

    let rejoin = reserve_identity(&mut engine, 1, 1, "synthetic-reservation-1001");
    assert!(rejoin.ok, "{:?}", rejoin.error);
    assert!(present(&mut engine, 1));
    assert_eq!(
        reserve_identity(&mut engine, 2, 1, "synthetic-reservation-0001")
            .error
            .as_deref(),
        Some("reservation_id_retired"),
        "an evicted reservation id must not be reused"
    );
}

#[test]
fn idle_expired_reservation_release_is_acknowledged_while_peers_remain() {
    let mut engine = engine();
    reserve(&mut engine, 0);
    reserve(&mut engine, 1);
    // Keep player 0 recent; player 1 has been silent past the idle timeout.
    let now = IDLE_TIMEOUT + Duration::from_secs(1);
    actors(&mut engine)
        .find(|a| a.reservation.player == 0)
        .unwrap()
        .last_event = now;
    engine.expire_world_peers(now, 2).unwrap();
    assert!(!present(&mut engine, 1) && present(&mut engine, 0));
    let ack = release(&mut engine, "synthetic-reservation-0001", now);
    assert!(ack.ok, "{:?}", ack.error);
}
