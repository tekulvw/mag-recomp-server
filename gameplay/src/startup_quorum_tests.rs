use super::*;

// Startup policy fixtures are independently invented; these are human
// reservations in the real shared engine, without network services or accounts.
fn configure_quorum(engine: &mut Engine<Vec<u8>>, minimum: usize, teams: [usize; 2]) -> Response {
    let request = parse_request(
        json!({"v":2,"request_id":"synthetic-quorum",
        "op":"configure_match","application_id":20231,"minimum_players":minimum,
        "minimum_players_per_team":teams,"maximum_players":16})
        .to_string()
        .as_bytes(),
    )
    .expect("per-team startup configuration must be supported");
    engine.handle(request, Duration::ZERO, 1)
}
fn quorum_ready(actor: &mut Active) {
    ready(actor);
    actor.claimed = Some(Claimed {
        wire: Vec::new(),
        np_identity: [actor.reservation.player as u8; 36],
        game_info: protocol::GameInfo {
            client_index: actor.reservation.player,
            level_name: "synthetic_arena".into(),
            start_time_ms: 0,
            attacking_faction: 1,
            defending_faction: 0,
            maximum_points: [100, 100],
            current_points: [100, 100],
            game_type: 0,
        },
    });
}
fn poll_quorum(engine: &mut Engine<Vec<u8>>, now: Duration) {
    let response = engine.handle(
        Request::Hello {
            v: 2,
            request_id: "synthetic-poll".into(),
        },
        now,
        1,
    );
    assert!(response.ok, "{:?}", response.error);
}
#[test]
fn startup_quorum_waits_for_opposing_ready_humans_and_latches_after_start() {
    let mut engine = engine();
    assert!(configure_quorum(&mut engine, 2, [1, 1]).ok);
    reserve(&mut engine, 0);
    reserve(&mut engine, 1);
    for actor in actors(&mut engine) {
        quorum_ready(actor);
    }
    poll_quorum(&mut engine, Duration::ZERO);
    assert!(
        !engine.match_started,
        "two attackers cannot satisfy the defender quorum"
    );
    reserve_faction(&mut engine, 2, 0);
    poll_quorum(&mut engine, Duration::ZERO);
    assert!(
        !engine.match_started,
        "an unclaimed reservation is not a ready human"
    );
    quorum_ready(
        actors(&mut engine)
            .find(|a| a.reservation.player == 2)
            .unwrap(),
    );
    poll_quorum(&mut engine, Duration::ZERO);
    assert!(engine.match_started);
    let epoch = engine.match_epoch;
    actors(&mut engine)
        .find(|a| a.reservation.player == 2)
        .unwrap()
        .ready_for_updates = false;
    poll_quorum(&mut engine, Duration::from_secs(1));
    assert!(
        engine.match_started,
        "departure after start must not pause the match"
    );
    assert_eq!(engine.match_epoch, epoch);
}

#[test]
fn startup_quorum_ignores_stale_unclaimed_backlogged_and_duplicate_humans() {
    let mut engine = engine();
    assert!(configure_quorum(&mut engine, 2, [1, 1]).ok);
    reserve(&mut engine, 0);
    reserve_faction(&mut engine, 1, 0);
    for actor in actors(&mut engine) {
        quorum_ready(actor);
    }
    let now = Duration::from_secs(119);
    assert!(engine.quorum_satisfied(now, None));
    assert!(!engine.quorum_satisfied(Duration::from_secs(120), None));
    let defender = actors(&mut engine).find(|a| a.team() == 1).unwrap();
    defender.replication_backlogged = true;
    assert!(!engine.quorum_satisfied(now, None));
    let defender = actors(&mut engine).find(|a| a.team() == 1).unwrap();
    defender.replication_backlogged = false;
    defender.claimed = None;
    assert!(!engine.quorum_satisfied(now, None));
    quorum_ready(actors(&mut engine).find(|a| a.team() == 1).unwrap());
    let account = actors(&mut engine)
        .find(|a| a.team() == 0)
        .unwrap()
        .reservation
        .account;
    actors(&mut engine)
        .find(|a| a.team() == 1)
        .unwrap()
        .reservation
        .account = account;
    assert!(
        !engine.quorum_satisfied(now, None),
        "one identity cannot fill both team minima"
    );
}
#[test]
fn startup_quorum_pending_ready_replaces_old_state_and_loss_before_start_waits() {
    let mut engine = engine();
    assert!(configure_quorum(&mut engine, 3, [1, 1]).ok);
    reserve(&mut engine, 0);
    reserve(&mut engine, 1);
    reserve_faction(&mut engine, 2, 0);
    for actor in actors(&mut engine) {
        quorum_ready(actor);
    }
    actors(&mut engine)
        .find(|a| a.team() == 1)
        .unwrap()
        .ready_for_updates = false;
    assert!(!engine.quorum_satisfied(Duration::ZERO, None));
    let defender = engine
        .peers
        .values()
        .chain(engine.active.iter())
        .find(|a| a.team() == 1)
        .unwrap();
    assert!(
        engine.quorum_satisfied(Duration::ZERO, Some(defender)),
        "initial Ready must predict the same Playing phase as commit"
    );
    actors(&mut engine)
        .find(|a| a.reservation.player == 0)
        .unwrap()
        .ready_for_updates = false;
    let defender = engine
        .peers
        .values()
        .chain(engine.active.iter())
        .find(|a| a.team() == 1)
        .unwrap();
    assert!(
        !engine.quorum_satisfied(Duration::ZERO, Some(defender)),
        "pending actor must never count twice"
    );
    poll_quorum(&mut engine, Duration::ZERO);
    assert!(!engine.match_started);
}
#[test]
fn startup_quorum_validates_capacity_and_keeps_legacy_defaults() {
    let mut engine = engine();
    assert!(engine.match_started);
    assert!(configure_quorum(&mut engine, 1, [0, 0]).ok);
    assert!(engine.match_started);
    assert_eq!(
        configure_quorum(&mut engine, 1, [9, 0]).error.as_deref(),
        Some("invalid_team_quorum")
    );
    assert!(
        engine.match_started,
        "rejected configuration must not change policy"
    );
    assert!(configure_quorum(&mut engine, 1, [1, 1]).ok);
    assert!(
        !engine.match_started,
        "solo shortcut must not bypass explicit team minima"
    );
    assert_eq!(
        parse_request(
            json!({"v":2,"request_id":"synthetic-invalid","op":"configure_match",
        "application_id":20231,"minimum_players":1,"maximum_players":8,
        "minimum_players_per_team":[5,4]})
            .to_string()
            .as_bytes()
        )
        .err()
        .unwrap()
        .code,
        "invalid_match_limits"
    );
}
#[test]
fn startup_quorum_empty_world_reset_retains_policy_for_the_next_cohort() {
    let mut engine = engine();
    assert!(configure_quorum(&mut engine, 1, [1, 1]).ok);
    reserve(&mut engine, 0);
    reserve_faction(&mut engine, 1, 0);
    for actor in actors(&mut engine) {
        quorum_ready(actor);
    }
    poll_quorum(&mut engine, Duration::ZERO);
    assert!(engine.match_started);
    for player in [0, 1] {
        let response = engine.handle(
            Request::Release {
                v: 2,
                request_id: format!("synthetic-release-{player}"),
                reservation_id: format!("synthetic-reservation-{player:04}"),
            },
            Duration::from_secs(1),
            1,
        );
        assert!(response.ok, "{:?}", response.error);
    }
    assert!(!engine.match_started);
    reserve(&mut engine, 0);
    quorum_ready(actors(&mut engine).next().unwrap());
    poll_quorum(&mut engine, Duration::from_secs(2));
    assert!(
        !engine.match_started,
        "new cohort must obtain its own defender"
    );
}

#[test]
fn startup_quorum_real_ready_events_start_once_and_late_join_keeps_the_epoch() {
    let mut engine = engine();
    assert!(configure_quorum(&mut engine, 1, [1, 1]).ok);
    reserve(&mut engine, 0);
    reserve_faction(&mut engine, 1, 0);
    for actor in actors(&mut engine) {
        quorum_ready(actor);
        actor.initial_objects = None;
        actor.ready_for_updates = false;
        actor.outbound_sequence = 0;
    }
    let send_ready = |engine: &mut Engine<Vec<u8>>, player: u16, millis| {
        let mut bytes = engine
            .wire_profile
            .wire_message_type(0x0005)
            .to_be_bytes()
            .to_vec();
        bytes.push(0x80);
        let response = engine.handle(
            Request::Event {
                v: 2,
                request_id: format!("synthetic-ready-{player}"),
                reservation_id: format!("synthetic-reservation-{player:04}"),
                world_id: 1,
                player_id: player,
                transport: Transport::Tcp,
                direction: Direction::ClientToServer,
                rt_type: 0x3d,
                plugin_id: Some(1),
                payload_hex: bytes.iter().map(|b| format!("{b:02x}")).collect(),
            },
            Duration::from_millis(millis),
            1,
        );
        assert!(response.ok, "{:?}", response.error);
        response.messages.len()
    };
    assert!(send_ready(&mut engine, 0, 10) > 0);
    assert!(!engine.match_started);
    assert!(send_ready(&mut engine, 1, 20) > 0);
    assert!(engine.match_started);
    let epoch = engine.match_epoch;
    send_ready(&mut engine, 1, 30);
    reserve(&mut engine, 2);
    quorum_ready(
        actors(&mut engine)
            .find(|a| a.reservation.player == 2)
            .unwrap(),
    );
    poll_quorum(&mut engine, Duration::from_millis(40));
    assert_eq!(engine.match_epoch, epoch);
    let records: Vec<Value> = String::from_utf8(engine.writer.clone())
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let starts: Vec<_> = records
        .iter()
        .filter(|row| row["op"] == "match_started")
        .collect();
    assert_eq!(starts.len(), 1);
    assert_eq!(starts[0]["ready_players_per_team"], json!([1, 1]));
}
