//! Independently authored Sabotage timing/roster fixtures; real 03ea dispatch.
use crate::protocol::ObjectiveInteractionKind as Kind;
use crate::replication::ObjectiveState as State;
use crate::*;

fn fixture() -> Engine<Vec<u8>> {
    let mut e = spawn_progression::tests::engine();
    spawn_progression::tests::reserve_faction(&mut e, 0, 1);
    spawn_progression::tests::reserve_faction(&mut e, 1, 0);
    for actor in e.active.iter_mut().chain(e.peers.values_mut()) {
        spawn_progression::tests::ready(actor);
        actor.life = CharacterLife::Alive {
            generation: 1,
            health_quarter_points: 400,
        };
    }
    e.match_started = true;
    e
}
fn focus(e: &mut Engine<Vec<u8>>, player: u16) {
    if let Some(a) = e.active.take() {
        e.peers.insert(a.reservation.id.clone(), a);
    }
    e.active = Some(
        e.peers
            .remove(&format!("synthetic-reservation-{player:04}"))
            .unwrap(),
    );
}
fn request(
    e: &mut Engine<Vec<u8>>,
    player: u16,
    index: u16,
    kind: Kind,
    ms: u64,
) -> Vec<OutboundMessage> {
    focus(e, player);
    let wire = protocol::ObjectiveInteraction {
        target_object_id: 0x0c80_0000 | u32::from(index),
        kind,
        actor_index: player,
        trigger_index: 0,
    }
    .encode();
    spawn_progression::tests::dispatch(e, &wire, ms).unwrap()
}
fn assert_ack(messages: &[OutboundMessage], player: u16, index: u16) {
    // Independent normal 03ea u32 target/u8 op/u16 actor/u8 trigger layout.
    let expected = format!("03ea0c80{index:04x}02{player:04x}00");
    assert!(
        messages
            .iter()
            .any(|m| m.player_id == player && m.payload_hex == expected),
        "expected {expected}, got {messages:?}"
    );
}
fn delta(e: &Engine<Vec<u8>>, index: u16) -> replication::ObjectiveControlDelta {
    e.objective_world
        .as_ref()
        .unwrap()
        .snapshot(1, 0)
        .objectives
        .into_iter()
        .find(|d| d.object_index == index)
        .unwrap()
}
fn tick(e: &mut Engine<Vec<u8>>, ms: u64) {
    e.tick_objectives(Duration::from_millis(ms), 1).unwrap();
}
fn activate(e: &mut Engine<Vec<u8>>, ms: u64) {
    assert_ack(&request(e, 0, 10, Kind::Activate, ms), 0, 10);
}

#[test]
fn sabotage_defender_real_03ea_deactivates_activated_objective() {
    let mut e = fixture();
    activate(&mut e, 100);
    tick(&mut e, 110);
    assert_eq!(delta(&e, 10).state_actor.state, State::Activated);
    assert_ack(&request(&mut e, 1, 10, Kind::Deactivate, 115), 1, 10);
    let d = delta(&e, 10);
    assert_eq!(
        (
            d.state_actor.state,
            d.expiration_time_ms,
            d.deactivation_time_ms
        ),
        (State::Deactivating, 130, 125)
    );
    tick(&mut e, 125);
    assert_eq!(delta(&e, 10).state_actor.state, State::Deactivated);
}

#[test]
fn sabotage_defender_completed_retake_and_attacker_contest_are_reversible() {
    let mut e = fixture();
    activate(&mut e, 100);
    tick(&mut e, 130);
    assert_eq!(delta(&e, 10).state_actor.state, State::Completed);
    assert_ack(&request(&mut e, 1, 10, Kind::Deactivate, 140), 1, 10);
    assert_eq!(delta(&e, 10).state_actor.state, State::Retaking);
    tick(&mut e, 150);
    assert_eq!(
        (
            delta(&e, 10).state_actor.state,
            delta(&e, 10).expiration_time_ms
        ),
        (State::Retaken, 170)
    );
    assert_ack(&request(&mut e, 0, 10, Kind::Activate, 155), 0, 10);
    assert_eq!(
        (
            delta(&e, 10).state_actor.state,
            delta(&e, 10).deactivation_time_ms
        ),
        (State::CancelRetaken, 165)
    );
    request(&mut e, 0, 10, Kind::Release, 160);
    assert_eq!(
        (
            delta(&e, 10).state_actor.state,
            delta(&e, 10).expiration_time_ms
        ),
        (State::Retaken, 170)
    );
    tick(&mut e, 170);
    assert_eq!(delta(&e, 10).state_actor.state, State::Deactivated);
}

#[test]
fn sabotage_defender_duplicate_and_release_preserve_original_completion_deadline() {
    let mut e = fixture();
    activate(&mut e, 100);
    tick(&mut e, 110);
    assert_ack(&request(&mut e, 1, 10, Kind::Deactivate, 115), 1, 10);
    assert_ack(&request(&mut e, 1, 10, Kind::Deactivate, 120), 1, 10);
    assert_eq!(delta(&e, 10).deactivation_time_ms, 125);
    let release = request(&mut e, 1, 10, Kind::Release, 122);
    assert!(
        release
            .iter()
            .any(|m| m.payload_hex == "03ea0c80000a020001ff")
    );
    assert!(
        release
            .iter()
            .any(|m| m.payload_hex == "03ea0c80000a05000100")
    );
    let d = delta(&e, 10);
    assert_eq!(
        (
            d.state_actor.state,
            d.expiration_time_ms,
            d.deactivation_time_ms,
            d.state_actor.actor_index
        ),
        (State::Activated, 130, 0, 0)
    );
    tick(&mut e, 130);
    assert_eq!(delta(&e, 10).state_actor.state, State::Completed);
}

#[test]
fn sabotage_defender_lost_contact_life_or_allocation_releases_without_restarting_window() {
    for loss in ["range", "life", "disconnect"] {
        let mut e = fixture();
        activate(&mut e, 100);
        tick(&mut e, 110);
        request(&mut e, 1, 10, Kind::Deactivate, 115);
        match loss {
            "range" => e.active.as_mut().unwrap().tracking.server_position(
                replication::CharacterPosition {
                    x: 500.,
                    y: 3.,
                    z: 100.,
                },
                player_tracking::PositionOrigin::ServerSpawn,
                116,
            ),
            "life" => {
                e.active.as_mut().unwrap().life = CharacterLife::Alive {
                    generation: 2,
                    health_quarter_points: 400,
                }
            }
            _ => {
                e.active.take();
            }
        }
        tick(&mut e, 120);
        assert_eq!(
            (
                delta(&e, 10).state_actor.state,
                delta(&e, 10).expiration_time_ms
            ),
            (State::Activated, 130),
            "{loss}"
        );
        tick(&mut e, 130);
        assert_eq!(delta(&e, 10).state_actor.state, State::Completed, "{loss}");
    }
}

#[test]
fn sabotage_defender_deactivation_uses_own_timer_then_restores_overdue_window() {
    let mut e = fixture();
    activate(&mut e, 100);
    tick(&mut e, 110);
    request(&mut e, 1, 10, Kind::Deactivate, 125);
    tick(&mut e, 131);
    // Old Activated expiry130 does not complete while deactivating until135.
    assert_eq!(delta(&e, 10).state_actor.state, State::Deactivating);
    request(&mut e, 1, 10, Kind::Release, 132);
    assert_eq!(
        (
            delta(&e, 10).state_actor.state,
            delta(&e, 10).expiration_time_ms
        ),
        (State::Activated, 130)
    );
    tick(&mut e, 132);
    assert_eq!(delta(&e, 10).state_actor.state, State::Completed);
}

#[test]
fn sabotage_defender_contest_blocks_pair_lock_then_completed_pair_locks_once() {
    let mut e = fixture();
    activate(&mut e, 100);
    tick(&mut e, 130);
    request(&mut e, 1, 10, Kind::Deactivate, 140);
    assert_ack(&request(&mut e, 0, 11, Kind::Activate, 140), 0, 11);
    tick(&mut e, 170);
    assert_eq!(delta(&e, 10).state_actor.state, State::Deactivated);
    assert_eq!(delta(&e, 11).state_actor.state, State::Completed);
    assert_eq!(delta(&e, 20).state_actor.state, State::Inactive);
    activate(&mut e, 180);
    tick(&mut e, 210);
    assert_eq!(delta(&e, 10).state_actor.state, State::Locked);
    assert_eq!(delta(&e, 11).state_actor.state, State::Locked);
    for index in 20..24 {
        assert_eq!(delta(&e, index).state_actor.state, State::Completed);
    }
    let denied = request(&mut e, 1, 10, Kind::Deactivate, 211);
    assert!(
        denied
            .iter()
            .any(|m| m.payload_hex == "03ea0c80000a030001ff")
    );
    assert_eq!(delta(&e, 10).state_actor.state, State::Locked);
    tick(&mut e, 1209);
    assert_eq!(delta(&e, 30).state_actor.state, State::Locked);
    tick(&mut e, 1210);
    assert_eq!(delta(&e, 30).state_actor.state, State::Deactivated);
}

#[test]
fn sabotage_defender_selected_trigger_and_authored_deactivation_permission_are_fenced() {
    for disabled in [false, true] {
        let mut e = fixture();
        let catalog = e.objective_catalog.as_mut().unwrap();
        let rule = catalog
            .objects
            .iter_mut()
            .find(|o| o.object_index == 10)
            .unwrap()
            .interaction
            .as_mut()
            .unwrap();
        rule.trigger_count = 2;
        rule.can_deactivate = !disabled;
        e.objective_world = Some(objective_control::ObjectiveWorld::new(catalog));
        activate(&mut e, 100);
        tick(&mut e, 110);
        focus(&mut e, 1);
        let mut wire = protocol::decode_hex("03ea0c80000a01000100");
        if !disabled {
            *wire.last_mut().unwrap() = 1;
        }
        let replies = spawn_progression::tests::dispatch(&mut e, &wire, 115).unwrap();
        assert!(
            replies
                .iter()
                .any(|m| m.payload_hex == "03ea0c80000a030001ff")
        );
        assert_eq!(delta(&e, 10).state_actor.state, State::Activated);
    }
}

#[test]
fn sabotage_defender_record_refusal_installs_neither_hold_nor_publication_and_retries() {
    let mut e = fixture();
    activate(&mut e, 100);
    tick(&mut e, 110);
    focus(&mut e, 1);
    let old = delta(&e, 10);
    let queues: Vec<_> = e
        .peers
        .values()
        .chain(e.active.iter())
        .map(|a| a.pending_shared.clone())
        .collect();
    let cap = e.max_log;
    e.max_log = e.written;
    let wire = protocol::decode_hex("03ea0c80000a01000100");
    assert_eq!(
        spawn_progression::tests::dispatch(&mut e, &wire, 115).unwrap_err(),
        "log_limit_reached"
    );
    assert_eq!(delta(&e, 10), old);
    assert_eq!(
        e.peers
            .values()
            .chain(e.active.iter())
            .map(|a| a.pending_shared.clone())
            .collect::<Vec<_>>(),
        queues
    );
    e.max_log = cap;
    e.failed = false;
    assert_ack(
        &spawn_progression::tests::dispatch(&mut e, &wire, 115).unwrap(),
        1,
        10,
    );
    let cap = e.max_log;
    e.max_log = e.written;
    assert_eq!(
        e.tick_objectives(Duration::from_millis(125), 1)
            .unwrap_err(),
        "log_limit_reached"
    );
    assert_eq!(delta(&e, 10).state_actor.state, State::Deactivating);
    e.max_log = cap;
    e.failed = false;
    tick(&mut e, 125);
    assert_eq!(delta(&e, 10).state_actor.state, State::Deactivated);
}

#[test]
fn sabotage_defender_shared_publication_defers_behind_reliable_fifo_without_losing_state() {
    let mut e = fixture();
    activate(&mut e, 100);
    tick(&mut e, 110);
    for a in e.peers.values_mut().chain(e.active.iter_mut()) {
        a.pending_shared.clear();
    }
    focus(&mut e, 0);
    let older = replication::ObjectUpdate {
        header: replication::UpdateHeader {
            sequence: 1,
            nak_sequence: 0,
            client_index: 0,
            object_type_mask: 0,
            timestamp_ms: 100,
        },
        wire: &[],
    }
    .encode_empty_with_profile(e.wire_profile)
    .unwrap();
    for _ in 0..127 {
        e.active.as_mut().unwrap().queue_shared(older.clone());
    }
    assert_ack(&request(&mut e, 1, 10, Kind::Deactivate, 115), 1, 10);
    // This helper enters the real event planner directly. Ordinary multiplayer
    // peer relay lives outside it; test the recorded tick's shared publication.
    tick(&mut e, 125);
    let expected = replication::ObjectiveControlSnapshot {
        sequence: 1,
        timestamp_ms: 125,
        objectives: vec![delta(&e, 10)],
    }
    .encode_with_profile(e.wire_profile)
    .unwrap();
    e.drain_shared(Vec::new(), 1).unwrap();
    let peer = e.peers.get("synthetic-reservation-0000").unwrap();
    assert!(!peer.replication_backlogged);
    assert_eq!(peer.pending_shared.len(), 1);
    assert!(
        peer.pending_shared
            .back()
            .is_some_and(|packet| packet == &expected)
    );
    let remaining = peer.pending_shared.len();
    e.drain_shared(Vec::new(), 1).unwrap();
    let peer = e.peers.get("synthetic-reservation-0000").unwrap();
    assert!(peer.pending_shared.len() < remaining);
    assert!(peer.pending_shared.is_empty());
    assert_eq!(delta(&e, 10).state_actor.state, State::Deactivated);
}

#[test]
fn sabotage_defender_retaking_release_and_successful_counter_retake_follow_distinct_edges() {
    let mut e = fixture();
    activate(&mut e, 100);
    tick(&mut e, 130);
    request(&mut e, 1, 10, Kind::Deactivate, 140);
    let wrong = request(&mut e, 0, 10, Kind::Release, 142);
    assert!(
        wrong
            .iter()
            .any(|m| m.payload_hex == "03ea0c80000a030000ff")
    );
    assert_eq!(delta(&e, 10).state_actor.state, State::Retaking);
    request(&mut e, 1, 10, Kind::Release, 143);
    assert_eq!(delta(&e, 10).state_actor.state, State::Completed);
    request(&mut e, 1, 10, Kind::Deactivate, 145);
    tick(&mut e, 155);
    request(&mut e, 0, 10, Kind::Activate, 160);
    tick(&mut e, 170);
    assert_eq!(delta(&e, 10).state_actor.state, State::Completed);
    tick(&mut e, 200);
    assert_eq!(delta(&e, 10).state_actor.state, State::Completed);
}

#[test]
fn sabotage_defender_release_preserves_original_attacker_capture_reward_identity() {
    let mut e = fixture();
    let catalog = e.objective_catalog.as_mut().unwrap();
    catalog
        .objects
        .iter_mut()
        .find(|o| o.object_index == 10)
        .unwrap()
        .reward = Some(objective_catalog::ObjectiveRewardRule {
        category: 1,
        subtype: 12,
        demolition: false,
    });
    e.objective_world = Some(objective_control::ObjectiveWorld::new(catalog));
    for a in e.active.iter_mut().chain(e.peers.values_mut()) {
        a.reservation.character.account_id =
            Some(format!("synthetic-account-{}", a.reservation.player));
    }
    activate(&mut e, 100);
    tick(&mut e, 110);
    request(&mut e, 1, 10, Kind::Deactivate, 115);
    request(&mut e, 1, 10, Kind::Release, 120);
    tick(&mut e, 130);
    let awards = e.pending_reward_batch();
    assert_eq!(awards.len(), 2);
    assert_eq!(
        awards[0].kind,
        progression::RewardKind::PrimaryObjectiveActivated
    );
    assert_eq!(
        awards[1].kind,
        progression::RewardKind::PrimaryObjectiveSecured
    );
    assert!(awards.iter().all(|a| a.player_id == 0
        && a.reservation_id == "synthetic-reservation-0000"
        && a.account_id == "synthetic-account-0"));
    tick(&mut e, 140);
    assert_eq!(e.pending_reward_batch().len(), 2);
}

#[test]
fn a_holder_that_goes_down_or_dies_gets_its_own_release_acknowledged() {
    // A client that loses its life mid-hold sends Release itself. The server
    // ends the hold on its own, so the release is acknowledged with the
    // no-trigger sentinel instead of refused, and the objective still rolls
    // back on the next tick.
    let downed = CharacterLife::Incapacitated {
        generation: 1,
        downed: incapacitation::Downed {
            deadline: Duration::from_secs(30),
            accumulated_damage_hp: 0.,
        },
    };
    for life in [downed, CharacterLife::AwaitingRespawn { generation: 1 }] {
        let mut e = fixture();
        activate(&mut e, 100);
        assert_eq!(delta(&e, 10).state_actor.state, State::Activating);
        e.active.as_mut().unwrap().life = life;
        let reply = request(&mut e, 0, 10, Kind::Release, 105);
        assert!(
            reply
                .iter()
                .any(|m| m.payload_hex == "03ea0c80000a020000ff"),
            "{life:?}: {reply:?}"
        );
        assert!(
            !reply
                .iter()
                .any(|m| m.payload_hex.starts_with("03ea0c80000a03")),
            "{life:?}: no Nak"
        );
        tick(&mut e, 106);
        assert_eq!(
            delta(&e, 10).state_actor.state,
            State::Deactivated,
            "{life:?}"
        );
    }
}

#[test]
fn a_living_out_of_game_actor_release_is_still_refused() {
    let mut e = fixture();
    activate(&mut e, 100);
    e.active.as_mut().unwrap().entered_game = false;
    let reply = request(&mut e, 0, 10, Kind::Release, 105);
    assert!(
        reply
            .iter()
            .any(|m| m.payload_hex == "03ea0c80000a030000ff"),
        "{reply:?}"
    );
}
