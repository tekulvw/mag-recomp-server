//! Synthetic canonical Alpha mount authority and admission regressions.
use super::*;
use crate::spawn_progression::dispatch;

fn fixture(actor_team: u8, bunker_team: u8, same_faction: bool) -> Engine<Vec<u8>> {
    let mut engine = super::bunker_team_tests::fixture(actor_team, bunker_team, same_faction);
    engine.bunker_exit_policy = Some(vehicle_exit::BunkerExitPolicy {
        transform: vehicle_exit::BunkerTransform {
            position: replication::CharacterPosition {
                x: 0.,
                y: 0.,
                z: 0.,
            },
            rotation_xyzw: [0., 0., 0., 1.],
        },
        candidates: vec![vehicle_exit::ExitCandidate {
            ranking_offset: [1., 0., 0.],
            position_offset: [2., 0., 0.],
        }],
    });
    engine.active.as_mut().unwrap().outbound_sequence = 4;
    engine
}
fn mount(
    engine: &mut Engine<Vec<u8>>,
    vehicle: u32,
    seat: u32,
    character: u32,
) -> Vec<OutboundMessage> {
    let mut wire = 0x0216u16.to_be_bytes().to_vec();
    wire.extend(vehicle.to_be_bytes());
    wire.extend(seat.to_be_bytes());
    wire.extend(character.to_be_bytes());
    dispatch(engine, &wire, 1000).unwrap()
}
fn try_mount(engine: &mut Engine<Vec<u8>>) -> Vec<OutboundMessage> {
    mount(engine, 0x0d00_0000, 0, 0x0380_0000)
}

#[test]
fn alpha_mount_rejects_same_faction_opponents() {
    for (actor, bunker) in [(0, 1), (1, 0)] {
        let mut engine = fixture(actor, bunker, true);
        assert!(
            try_mount(&mut engine).is_empty(),
            "actor {actor}, bunker {bunker}"
        );
        assert!(engine.active.as_ref().unwrap().mounted_seat.is_none());
    }
}
#[test]
fn alpha_mount_accepts_allies_independently_of_faction() {
    for team in [0, 1] {
        for same_faction in [true, false] {
            let mut engine = fixture(team, team, same_faction);
            assert!(
                !try_mount(&mut engine).is_empty(),
                "team {team}, same faction {same_faction}"
            );
            assert_eq!(engine.active.as_ref().unwrap().mounted_seat, Some(0));
        }
    }
}
#[test]
fn alpha_mount_requires_configured_canonical_bunker_authority() {
    for absent_world in [true, false] {
        let mut engine = fixture(1, 1, true);
        if absent_world {
            engine.bunker_world = None;
        } else {
            engine.bunker_world.as_mut().unwrap().bunkers.remove(&0);
        }
        assert!(try_mount(&mut engine).is_empty());
        assert!(engine.active.as_ref().unwrap().mounted_seat.is_none());
    }
}
#[test]
fn alpha_mount_preserves_actor_target_and_exit_policy_guards() {
    for guard in [
        "unready",
        "not_entered",
        "dead",
        "not_created",
        "exit_policy",
        "wrong_vehicle",
        "wrong_character",
        "invalid_seat",
    ] {
        let mut engine = fixture(1, 1, true);
        let actor = engine.active.as_mut().unwrap();
        match guard {
            "unready" => actor.ready_for_updates = false,
            "not_entered" => actor.entered_game = false,
            "dead" => actor.life = CharacterLife::AwaitingRespawn { generation: 1 },
            "not_created" => actor.emplacement_creation = None,
            "exit_policy" => engine.bunker_exit_policy = None,
            _ => (),
        }
        let vehicle = if guard == "wrong_vehicle" {
            0x0d00_0001
        } else {
            0x0d00_0000
        };
        let character = if guard == "wrong_character" {
            0x0380_0001
        } else {
            0x0380_0000
        };
        let seat = if guard == "invalid_seat" { 9 } else { 0 };
        assert!(
            mount(&mut engine, vehicle, seat, character).is_empty(),
            "{guard}"
        );
        assert!(
            engine.active.as_ref().unwrap().mounted_seat.is_none(),
            "{guard}"
        );
    }
}
#[test]
fn alpha_mount_keeps_duplicate_replies_and_seat_exclusivity() {
    let mut engine = fixture(1, 1, true);
    let first = try_mount(&mut engine);
    assert!(!first.is_empty());
    let replay = try_mount(&mut engine);
    assert_eq!(first[0].payload_hex, replay[0].payload_hex);
    assert_eq!(engine.active.as_ref().unwrap().seat_transitions.len(), 1);
    let mut blocked = fixture(1, 1, true);
    let mut peer = fixture(1, 1, true).active.take().unwrap();
    peer.reservation.player = 1;
    peer.reservation.id = "synthetic-seat-peer".into();
    peer.mounted_seat = Some(0);
    blocked.peers.insert(peer.reservation.id.clone(), peer);
    assert!(try_mount(&mut blocked).is_empty());
    assert!(blocked.active.as_ref().unwrap().mounted_seat.is_none());
}
