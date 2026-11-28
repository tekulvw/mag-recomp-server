//! SYNTHETIC operator-kill checks; see docs/admin-cli.md. Every fixture is
//! built in memory and no game files are involved.
use super::*;
use crate::replication::{ObjectUpdate, UpdateHeader};
use crate::spawn_progression::tests::{dispatch, engine, ready, reserve};

const OPERATOR: &str = "synthetic staff";
const REASON: &str = "stuck in a parachute";

fn line(reservation: &str, world: u32, player: u16) -> Value {
    json!({"op":"admin_kill","v":BRIDGE_VERSION,"request_id":"synthetic-admin-kill",
        "reservation_id":reservation,"world_id":world,"player_id":player,
        "operator":OPERATOR,"reason":REASON})
}

fn send(engine: &mut Engine<Vec<u8>>, request: Value, now_ms: u64) -> Response {
    let request = parse_request(request.to_string().as_bytes()).expect("a valid admin_kill");
    engine.handle(request, Duration::from_millis(now_ms), 1)
}

fn find<'a>(engine: &'a Engine<Vec<u8>>, reservation: &str) -> &'a Active {
    engine
        .active
        .iter()
        .chain(engine.peers.values())
        .find(|actor| actor.reservation.id == reservation)
        .unwrap()
}

fn find_mut<'a>(engine: &'a mut Engine<Vec<u8>>, reservation: &str) -> &'a mut Active {
    engine
        .active
        .iter_mut()
        .chain(engine.peers.values_mut())
        .find(|actor| actor.reservation.id == reservation)
        .unwrap()
}

/// Kill one admitted character by its own reservation identity.
fn kill(engine: &mut Engine<Vec<u8>>, reservation: &str, now_ms: u64) -> Response {
    let actor = find(engine, reservation);
    let request = line(
        reservation,
        actor.reservation.world,
        actor.reservation.player,
    );
    send(engine, request, now_ms)
}

fn outcome(response: &Response) -> (&str, Option<&str>) {
    assert!(response.ok, "{:?}", response.error);
    let outcome = response.admin_kill.as_ref().expect("an admin_kill outcome");
    (outcome.result.as_str(), outcome.reason.as_deref())
}

fn journal(engine: &Engine<Vec<u8>>) -> Vec<Value> {
    String::from_utf8_lossy(&engine.writer)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

const FIRST: &str = "synthetic-reservation-0000";
const SECOND: &str = "synthetic-reservation-0001";

fn two_alive() -> Engine<Vec<u8>> {
    let mut engine = engine();
    reserve(&mut engine, 0);
    reserve(&mut engine, 1);
    for actor in engine.peers.values_mut().chain(engine.active.iter_mut()) {
        ready(actor);
        actor.life = CharacterLife::Alive {
            generation: 1,
            health_quarter_points: 400,
        };
    }
    engine
}

#[test]
fn the_worker_advertises_the_capability() {
    let engine = engine();
    let hello = engine.discovery_response("synthetic-hello".into());
    assert!(hello.capabilities.iter().any(|c| c == CAPABILITY));
    let described = engine.describe_worker("synthetic-hello".into(), false);
    assert!(described.capabilities.iter().any(|c| c == CAPABILITY));
}

#[test]
fn the_request_kills_exactly_the_named_character_as_its_own_kill_self() {
    let mut engine = two_alive();
    let progression = find(&engine, SECOND).reservation.character.progression;
    let response = kill(&mut engine, SECOND, 1000);
    assert_eq!(outcome(&response), ("killed", None));
    assert!(
        response.rewards.is_empty(),
        "no experience for an operator kill"
    );

    let victim = find(&engine, SECOND);
    assert!(matches!(
        victim.life,
        CharacterLife::AwaitingRespawn { generation: 1 }
    ));
    assert_eq!(victim.last_damage.health_quarter_points, 0);
    // Retail Kill Self statistics: one death and one suicide, nothing else.
    assert_eq!(victim.reward_statistics.get(&12), Some(&1));
    assert_eq!(victim.reward_statistics.get(&13), Some(&1));
    assert_eq!(victim.reservation.character.progression, progression);
    assert!(find(&engine, FIRST).life.is_alive());
    // The victim's own terminal update is among the replies.
    assert!(
        response
            .messages
            .iter()
            .any(|message| message.reservation_id == SECOND)
    );

    let rows = journal(&engine);
    let admin = rows
        .iter()
        .position(|row| row["op"] == "admin_kill")
        .expect("an admin_kill row");
    assert_eq!(rows[admin]["reservation_id"], SECOND);
    assert_eq!(rows[admin]["operator"], OPERATOR);
    assert_eq!(rows[admin]["reason"], REASON);
    assert_eq!(rows[admin]["life_before"], "alive");
    let event = &rows[admin + 1];
    assert_eq!(event["op"], "event");
    assert_eq!(event["reservation_id"], SECOND);
    assert_eq!(event["payload_hex"], "021503800001");
    assert_eq!(event["decoded"]["type"], "kill_self");
}

#[test]
fn identity_mismatches_and_unknown_reservations_are_not_in_match() {
    let mut engine = two_alive();
    let written = engine.written;
    for request in [
        line(SECOND, 2, 1),
        line(SECOND, 1, 0),
        line("synthetic-reservation-9999", 1, 1),
    ] {
        let response = send(&mut engine, request, 1000);
        assert!(!response.ok);
        assert_eq!(response.error.as_deref(), Some("not_in_match"));
        assert!(response.messages.is_empty() && response.admin_kill.is_none());
    }
    assert_eq!(engine.written, written, "a refusal records nothing");
    assert!(find(&engine, SECOND).life.is_alive());
}

#[test]
fn characters_that_are_not_placed_or_not_alive_are_refused() {
    let mut engine = two_alive();
    find_mut(&mut engine, SECOND).entered_game = false;
    assert_eq!(
        kill(&mut engine, SECOND, 1000).error.as_deref(),
        Some("not_in_match")
    );
    let mut engine = two_alive();
    let peer = find_mut(&mut engine, SECOND);
    peer.life = CharacterLife::AwaitingRespawn { generation: 1 };
    assert_eq!(
        kill(&mut engine, SECOND, 1000).error.as_deref(),
        Some("not_alive")
    );
    find_mut(&mut engine, SECOND).life = CharacterLife::AwaitingFirstSpawn;
    assert_eq!(
        kill(&mut engine, SECOND, 1000).error.as_deref(),
        Some("not_alive")
    );
    let mut engine = two_alive();
    engine.match_terminal = Some(match_flow::MatchTerminal {
        world_id: 1,
        match_id: "synthetic-ending".into(),
        reason: "synthetic".into(),
        reservation_ids: vec![FIRST.into(), SECOND.into()],
    });
    assert_eq!(
        kill(&mut engine, SECOND, 1000).error.as_deref(),
        Some("match_finished")
    );
    assert!(find(&engine, SECOND).life.is_alive());
}

#[test]
fn malformed_operator_fields_are_refused_by_the_parser() {
    let mut request = line(SECOND, 1, 1);
    request["operator"] = json!("");
    let error = parse_request(request.to_string().as_bytes()).err().unwrap();
    assert_eq!(error.code, "invalid_admin_operator");
    request["operator"] = json!("x".repeat(MAX_OPERATOR + 1));
    assert_eq!(
        parse_request(request.to_string().as_bytes())
            .err()
            .unwrap()
            .code,
        "invalid_admin_operator"
    );
    let mut request = line(SECOND, 1, 1);
    request["reason"] = json!("line\nbreak");
    assert_eq!(
        parse_request(request.to_string().as_bytes())
            .err()
            .unwrap()
            .code,
        "invalid_admin_reason"
    );
    request.as_object_mut().unwrap().remove("reason");
    assert!(parse_request(request.to_string().as_bytes()).is_ok());
    request["unexpected"] = json!(1);
    assert!(parse_request(request.to_string().as_bytes()).is_err());
}

#[test]
fn a_kill_self_the_planner_denies_still_delivers_its_replies_and_says_refused() {
    let mut engine = two_alive();
    // A seat that belongs to no supported vehicle family.
    find_mut(&mut engine, SECOND).mounted_seat = Some(3);
    let response = kill(&mut engine, SECOND, 1000);
    assert_eq!(
        outcome(&response),
        ("refused", Some("mounted_death_not_supported"))
    );
    assert!(find(&engine, SECOND).life.is_alive());
}

#[test]
fn a_seated_hospital_occupant_dies_and_leaves_the_seat() {
    use crate::seated_death::tests::{assert_seated_exit, frame_wires, terminal_for};
    let mut engine = crate::structures::domination_bunker_seat_tests::fixture_for_mode(1);
    engine.item_catalog = crate::human_kill_reward_tests::bot_firearm_items();
    let mut mount = 0x0216u16.to_be_bytes().to_vec();
    mount.extend(0x0d00_0005u32.to_be_bytes());
    mount.extend(12u32.to_be_bytes());
    mount.extend(0x0380_0000u32.to_be_bytes());
    dispatch(&mut engine, &mount, 1000).unwrap();
    let actor = engine.active.as_ref().unwrap();
    assert_eq!(
        (actor.mounted_bunker, actor.mounted_seat),
        (Some(5), Some(12))
    );
    let id = actor.reservation.id.clone();

    let response = kill(&mut engine, &id, 1100);
    assert_eq!(outcome(&response), ("killed", None));
    let terminal = terminal_for(
        frame_wires(&response.messages).iter().map(Vec::as_slice),
        0,
        engine.wire_profile,
    );
    assert_seated_exit(&terminal);
    let actor = find(&engine, &id);
    assert!(!actor.life.is_alive());
    assert_eq!((actor.mounted_bunker, actor.mounted_seat), (None, None));
}

#[test]
fn a_downed_character_gives_up_exactly_as_its_own_kill_self_would() {
    use crate::incapacitation_tests::{dispatch_pending_damage, patient, two_actors};
    let downed = || {
        let mut engine = two_actors(0.4);
        dispatch_pending_damage(&mut engine, 40.0, 1000);
        assert!(matches!(
            patient(&engine).life,
            CharacterLife::Incapacitated { generation: 4, .. }
        ));
        assert_eq!(patient(&engine).reward_statistics.get(&12), Some(&1));
        engine
    };
    let mut engine = downed();
    let response = kill(&mut engine, FIRST, 2000);
    assert_eq!(outcome(&response), ("gave_up", None));
    let actor = find(&engine, FIRST);
    assert!(matches!(
        actor.life,
        CharacterLife::AwaitingRespawn { generation: 4 }
    ));
    assert_eq!(actor.last_damage.conditions & 2, 0);
    // The death is the one the downing hit already counted.
    assert_eq!(actor.reward_statistics.get(&12), Some(&1));
    let rows = journal(&engine);
    assert!(
        rows.iter()
            .any(|row| row["op"] == "admin_kill" && row["life_before"] == "downed")
    );

    // The patient's own Kill Self from the same state leaves the same character.
    let mut own = downed();
    let healer = own.active.take().unwrap();
    own.active = own.peers.remove(FIRST);
    own.peers.insert(healer.reservation.id.clone(), healer);
    dispatch(&mut own, &kill_self_payload(0), 2000).unwrap();
    let theirs = find(&own, FIRST);
    assert_eq!(
        format!(
            "{:?}",
            (actor.life, actor.last_damage, &actor.reward_statistics)
        ),
        format!(
            "{:?}",
            (theirs.life, theirs.last_damage, &theirs.reward_statistics)
        )
    );
}

fn owner_update(engine: &mut Engine<Vec<u8>>, now_ms: u64) {
    let header = UpdateHeader {
        sequence: engine.active.as_ref().unwrap().latest_owner_sequence + 1,
        nak_sequence: 0,
        client_index: 0,
        object_type_mask: 0,
        timestamp_ms: now_ms as u32,
    };
    let bytes = ObjectUpdate { header, wire: &[] }
        .encode_empty_with_profile(engine.wire_profile)
        .unwrap();
    dispatch(engine, &bytes, now_ms).unwrap();
}

#[test]
fn the_killed_character_rejoins_by_itself_but_never_on_a_wave_inside_the_cutoff() {
    // The live case: an operator kill 2.15 s before the team's wave. That
    // life reached a client still playing the death, which started a drop
    // without a parachute and without new magazines.
    let mut engine = engine();
    reserve(&mut engine, 0);
    let actor = engine.active.as_mut().unwrap();
    ready(actor);
    actor.life = CharacterLife::AwaitingFirstSpawn;
    // First life: select point 1 and confirm; the wave spawns it.
    let mut select = 0x0219u16.to_be_bytes().to_vec();
    select.extend(1u16.to_be_bytes());
    dispatch(&mut engine, &select, 50).unwrap();
    dispatch(&mut engine, &0x0211u16.to_be_bytes(), 50).unwrap();
    owner_update(&mut engine, 150);
    assert!(engine.active.as_ref().unwrap().life.is_alive());
    engine.enable_world_ticks();
    // Waves at 2.36 s, 22.36 s, ...
    let team = usize::from(engine.active.as_ref().unwrap().team());
    engine.team_waves[team] = Some(TeamWaveClock {
        first_deadline: Duration::from_millis(2360),
        period: Duration::from_secs(20),
    });

    let killed_ms = 210;
    assert_eq!(outcome(&kill(&mut engine, FIRST, killed_ms)).0, "killed");
    assert!(matches!(
        engine.active.as_ref().unwrap().life,
        CharacterLife::AwaitingRespawn { generation: 1 }
    ));
    // No confirmation: the world tick queues it, past the close wave.
    let tick = engine.tick_world(
        Duration::from_millis(killed_ms + 10),
        1,
        "synthetic-tick".into(),
    );
    assert!(tick.ok, "{:?}", tick.error);
    let actor = engine.active.as_ref().unwrap();
    assert_eq!(
        actor.pending_spawn_deadline,
        Some(Duration::from_millis(22_360))
    );
    assert_eq!(
        actor.published_wave_deadline,
        Some(Duration::from_millis(22_360)),
        "the countdown shows the wave it spawns on"
    );
    owner_update(&mut engine, 2400);
    assert!(matches!(
        engine.active.as_ref().unwrap().life,
        CharacterLife::AwaitingRespawn { generation: 1 }
    ));
    owner_update(&mut engine, 22_400);
    assert!(matches!(
        engine.active.as_ref().unwrap().life,
        CharacterLife::Alive { generation: 2, .. }
    ));
}
