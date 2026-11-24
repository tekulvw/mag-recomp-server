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

#[test]
fn acknowledged_release_before_reserve_fences_a_delayed_admission() {
    for with_peer in [false, true] {
        let mut engine = engine();
        if with_peer {
            reserve(&mut engine, 0);
        }
        let id = "synthetic-delayed-reserve-0001";
        let ack = release(&mut engine, id, Duration::from_secs(1));
        assert!(
            ack.ok,
            "release must fence future admission: {:?}",
            ack.error
        );
        let delayed = reserve_identity(&mut engine, 1, 1, id);
        assert_eq!(delayed.error.as_deref(), Some("reservation_id_retired"));
        assert_eq!(
            present(&mut engine, 0),
            with_peer,
            "release changed an unrelated current owner"
        );
        assert!(!present(&mut engine, 1));
        assert!(release(&mut engine, id, Duration::from_secs(2)).ok);
    }
}

#[test]
fn unknown_release_record_failure_does_not_acknowledge_or_tombstone() {
    let mut engine = engine();
    let max = engine.max_log;
    engine.max_log = engine.written;
    let id = "synthetic-delayed-reserve-0001";
    let reply = release(&mut engine, id, Duration::from_secs(1));
    assert_eq!(reply.error.as_deref(), Some("log_limit_reached"));
    assert!(!engine.released_reservations.contains(id));
    engine.failed = false;
    engine.max_log = max;
    assert!(release(&mut engine, id, Duration::from_secs(2)).ok);
    assert_eq!(
        reserve_identity(&mut engine, 1, 1, id).error.as_deref(),
        Some("reservation_id_retired")
    );
}

// A client that has just deployed into a 64-bot match drains its bootstrap
// while bots keep publishing and notices wait behind the shared FIFO. Live
// matches evicted such joiners at the former 128-entry bounds.
#[test]
fn joiner_burst_above_the_former_bound_is_kept_and_eviction_names_its_queue() {
    let mut engine = engine();
    reserve(&mut engine, 0);
    reserve(&mut engine, 1);
    let joiner = actors(&mut engine)
        .find(|a| a.reservation.player == 1)
        .unwrap();
    for _ in 0..200 {
        joiner.queue_shared(vec![0]);
        joiner.queue_round(match_flow::Publication::Application(vec![0]));
    }
    assert!(
        !joiner.replication_backlogged,
        "a 200-entry burst evicted the joiner"
    );
    const { assert!(multiplayer::MAX_SHARED_QUEUE >= 512) };
    const { assert!(match_flow::MAX_PENDING_NOTIFICATIONS >= 512) };

    for _ in 0..multiplayer::MAX_SHARED_QUEUE {
        joiner.queue_shared(vec![0]);
    }
    assert!(joiner.replication_backlogged);
    let refused = evicted_event(&mut engine, 1, Duration::from_secs(1));
    assert!(!refused.ok);
    assert!(present(&mut engine, 0), "the other human keeps the world");
    let eviction = String::from_utf8_lossy(&engine.writer)
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .find(|record| record["op"] == "replication_backlog_released")
        .expect("eviction record");
    assert_eq!(eviction["pending_shared"], multiplayer::MAX_SHARED_QUEUE);
    assert_eq!(eviction["pending_notifications"], 200);
}
