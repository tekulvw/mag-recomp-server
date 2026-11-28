//! Domination keeps the native capture-hold exit handshake that Sabotage and
//! Acquisition send. Without it the holder's client keeps its interaction
//! prompt after the objective leaves Activating.
use super::*;
use protocol::ObjectiveInteractionKind as Kind;
use replication::ObjectiveState;

const CAPTURE: u16 = 1;
const TEAMMATE: &str = "synthetic-reservation-0001";
const ENEMY: &str = "synthetic-reservation-0002";
const CAPTURER: &str = "synthetic-reservation-0000";

fn interactions(messages: &[OutboundMessage]) -> Vec<(String, protocol::ObjectiveInteraction)> {
    messages
        .iter()
        .filter_map(|m| {
            protocol::ObjectiveInteraction::decode(&protocol::decode_hex(&m.payload_hex))
                .ok()
                .map(|packet| (m.reservation_id.clone(), packet))
        })
        .collect()
}

fn exits(messages: &[OutboundMessage]) -> Vec<(String, protocol::ObjectiveInteraction)> {
    interactions(messages)
        .into_iter()
        .filter(|(_, packet)| packet.kind == Kind::ActivatingOnExit)
        .collect()
}

fn state(e: &Engine<Vec<u8>>) -> Option<ObjectiveState> {
    e.objective_world.as_ref().unwrap().state(CAPTURE)
}

/// Decode the control state rows of every queued family-12 update.
fn queued_objective_states(actor: &Active, profile: wire_profile::WireProfile) -> Vec<(u16, u8)> {
    use replication::{BitReader, ObjectUpdate};
    let mut rows = Vec::new();
    for packet in &actor.pending_shared {
        let Ok(update) = ObjectUpdate::decode_with_profile(packet, profile) else {
            continue;
        };
        if update.header.object_type_mask != 1 << 12 {
            continue;
        }
        let mut bits = BitReader::with_profile(packet, profile);
        bits.position = profile.header_bits();
        while bits.read(1).unwrap() != 0 {
            let index = bits.read(16).unwrap() as u16;
            for field in 0..11 {
                if bits.read(1).unwrap() == 0 {
                    continue;
                }
                match field {
                    2 => {
                        rows.push((index, bits.read(8).unwrap() as u8));
                        bits.read(32).unwrap();
                        bits.read(16).unwrap();
                    }
                    3 | 4 | 9 | 10 => {
                        bits.read(32).unwrap();
                    }
                    6 => {
                        bits.read(8).unwrap();
                    }
                    _ => panic!("unexpected objective field {field}"),
                }
            }
        }
    }
    rows
}

/// Promote one reserved human to the event slot, parking the current one.
fn act_as(e: &mut Engine<Vec<u8>>, reservation_id: &str) {
    if e.active.as_ref().unwrap().reservation.id == reservation_id {
        return;
    }
    let next = e.peers.remove(reservation_id).unwrap();
    let previous = e.active.replace(next).unwrap();
    e.peers.insert(previous.reservation.id.clone(), previous);
}

/// The capturer (squad 0, attacking team 0) plus a teammate on the same squad
/// and an enemy defender (squad 16), all standing inside the capture trigger.
fn three_human_fixture() -> Engine<Vec<u8>> {
    let mut e = fixture();
    // Admission may place a new human in any squad; give each one an order.
    let state = &mut e.domination_gameplay.as_mut().unwrap().state;
    state.assignments = [state.assignments[0]; 32];
    // Admission validates the unmodified authored catalog; the fixture's
    // Deactivated startup rows are only for the running world.
    let running = e.objective_catalog.replace(
        crate::domination_gameplay_engine::tests::structural_repair::repair_fixture()
            .objective_catalog
            .unwrap(),
    );
    let items = std::mem::replace(
        &mut e.item_catalog,
        crate::spawn_progression::tests::engine().item_catalog,
    );
    let spawns = e.spawn_catalog.clone().unwrap();
    for (player, squad) in [(1, 0), (2, 16)] {
        crate::spawn_progression::tests::reserve(&mut e, player);
        let actor = e.active.as_mut().unwrap();
        crate::spawn_progression::tests::ready(actor);
        actor.life = CharacterLife::Alive {
            generation: 1,
            health_quarter_points: 400,
        };
        actor.squad_catalog = std::sync::Arc::new(spawns.for_squad(squad).unwrap());
        actor.tracking.server_position(
            replication::CharacterPosition {
                x: 4.,
                y: 2.,
                z: 1.,
            },
            player_tracking::PositionOrigin::ServerSpawn,
            0,
        );
    }
    e.objective_catalog = running;
    e.item_catalog = items;
    act_as(&mut e, CAPTURER);
    assert_eq!(e.active.as_ref().unwrap().team(), 0);
    assert_eq!(e.peers[TEAMMATE].team(), 0);
    assert_eq!(e.peers[ENEMY].team(), 1);
    for actor in e.peers.values_mut().chain(e.active.iter_mut()) {
        actor.pending_shared.clear();
    }
    e
}

#[test]
fn completed_domination_capture_ends_the_holders_interaction_once() {
    let mut e = three_human_fixture();
    let profile = e.wire_profile;
    let started = interact(&mut e, CAPTURE, Kind::Activate, 100).unwrap();
    assert!(
        interactions(&started)
            .iter()
            .any(|(_, packet)| packet.kind == Kind::Ack)
    );
    assert!(exits(&started).is_empty(), "the hold has only started");
    assert_eq!(state(&e), Some(ObjectiveState::Activating));

    // The hold timer elapses in the ordinary objective tick.
    let captured = tick(&mut e, 137).unwrap();
    assert_eq!(state(&e), Some(ObjectiveState::Activated));
    let exit = exits(&captured);
    assert_eq!(
        exit.len(),
        1,
        "only the holder's interaction ends: {captured:?}"
    );
    assert_eq!(exit[0].0, CAPTURER);
    assert_eq!(
        exit[0].1,
        protocol::ObjectiveInteraction {
            target_object_id: 0x0c80_0000 | u32::from(CAPTURE),
            kind: Kind::ActivatingOnExit,
            actor_index: 0,
            trigger_index: 0,
        }
    );
    // The handshake travels with the recorded transaction.
    let log = String::from_utf8_lossy(&e.writer).into_owned();
    // The state-change receipt follows the transaction record.
    let last = log
        .lines()
        .rev()
        .find(|line| line.contains("\"op\":\"domination_gameplay\""))
        .unwrap();
    assert!(
        log.lines()
            .last()
            .unwrap()
            .contains("domination_objective_states")
    );
    assert!(last.contains(&exit_hex()));

    // The capturer, the teammate and the enemy all receive the new state.
    for id in [CAPTURER, TEAMMATE, ENEMY] {
        let actor = e
            .peers
            .get(id)
            .or(e.active.as_ref().filter(|a| a.reservation.id == id))
            .unwrap();
        assert!(
            queued_objective_states(actor, profile)
                .contains(&(CAPTURE, ObjectiveState::Activated as u8)),
            "{id} receives the captured state"
        );
    }

    // A later tick does not repeat the handshake.
    assert!(exits(&tick(&mut e, 138).unwrap()).is_empty());
}

#[test]
fn owning_team_cannot_reactivate_a_capture_while_the_enemy_can_contest_it() {
    let mut e = three_human_fixture();
    interact(&mut e, CAPTURE, Kind::Activate, 100).unwrap();
    tick(&mut e, 137).unwrap();
    assert_eq!(state(&e), Some(ObjectiveState::Activated));

    for id in [CAPTURER, TEAMMATE] {
        act_as(&mut e, id);
        let reply = interact(&mut e, CAPTURE, Kind::Activate, 138).unwrap();
        assert!(
            interactions(&reply)
                .iter()
                .any(|(to, packet)| to == id && packet.kind == Kind::Nak),
            "{id} cannot activate a point its team holds"
        );
        assert_eq!(state(&e), Some(ObjectiveState::Activated));
    }

    act_as(&mut e, ENEMY);
    let contest = interact(&mut e, CAPTURE, Kind::Deactivate, 139).unwrap();
    assert!(
        interactions(&contest)
            .iter()
            .any(|(to, packet)| to == ENEMY && packet.kind == Kind::Ack)
    );
    assert_eq!(state(&e), Some(ObjectiveState::Deactivating));
    // The enemy's own hold ends with the same handshake once it resolves.
    let resolved = tick(&mut e, 139 + 47).unwrap();
    assert_ne!(state(&e), Some(ObjectiveState::Deactivating));
    let exit = exits(&resolved);
    assert_eq!(exit.len(), 1, "{resolved:?}");
    assert_eq!(exit[0].0, ENEMY);
    assert_eq!(exit[0].1.actor_index, 2);
}

#[test]
fn range_cancelled_domination_hold_ends_the_holders_interaction() {
    let mut e = three_human_fixture();
    interact(&mut e, CAPTURE, Kind::Activate, 100).unwrap();
    e.active.as_mut().unwrap().tracking.server_position(
        replication::CharacterPosition {
            x: 100.,
            y: 2.,
            z: 1.,
        },
        player_tracking::PositionOrigin::ServerSpawn,
        110,
    );
    let cancelled = tick(&mut e, 120).unwrap();
    assert_eq!(state(&e), Some(ObjectiveState::Deactivated));
    let exit = exits(&cancelled);
    assert_eq!(exit.len(), 1, "{cancelled:?}");
    assert_eq!(exit[0].0, CAPTURER);
}

fn exit_hex() -> String {
    protocol::encode_hex(
        &protocol::ObjectiveInteraction {
            target_object_id: 0x0c80_0000 | u32::from(CAPTURE),
            kind: Kind::ActivatingOnExit,
            actor_index: 0,
            trigger_index: 0,
        }
        .encode(),
    )
}

#[test]
fn a_capturer_downed_mid_hold_gets_its_own_release_acknowledged() {
    // Live: a bot downed a human holding a Domination capture. Its client
    // released the capture as it went down; a Nak to that release was
    // followed by the client leaving its downed state while the server still
    // held it down.
    let mut e = three_human_fixture();
    interact(&mut e, CAPTURE, Kind::Activate, 100).unwrap();
    assert_eq!(state(&e), Some(ObjectiveState::Activating));
    e.active.as_mut().unwrap().life = CharacterLife::Incapacitated {
        generation: 1,
        downed: incapacitation::Downed {
            deadline: Duration::from_secs(30),
            accumulated_damage_hp: 0.,
        },
    };
    let reply = interact(&mut e, CAPTURE, Kind::Release, 105).unwrap();
    let replies = interactions(&reply);
    assert!(
        replies.iter().any(|(to, packet)| to == CAPTURER
            && *packet
                == protocol::ObjectiveInteraction {
                    target_object_id: 0x0c80_0000 | u32::from(CAPTURE),
                    kind: Kind::Ack,
                    actor_index: 0,
                    trigger_index: u8::MAX,
                }),
        "{reply:?}"
    );
    assert!(!replies.iter().any(|(_, packet)| packet.kind == Kind::Nak));
    // The server's own life-loss rule still ends the hold; a downed holder
    // has no client interaction left to end with the exit handshake.
    let ended = tick(&mut e, 110).unwrap();
    assert_eq!(state(&e), Some(ObjectiveState::Deactivated));
    assert!(exits(&ended).is_empty(), "{ended:?}");
}
