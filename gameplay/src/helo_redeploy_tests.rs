//! Owned redeploy through the native KillSelf event with synthetic Helo occupants.

use crate::domination_helo::{Carrier, PassengerSeat};
use crate::replication::{CharacterPosition, VehicleState};
use crate::spawn_progression::dispatch;
use crate::*;

const PEER: &str = "synthetic-rocket-target";

fn carrier(engine: &mut Engine<Vec<u8>>, index: usize) -> &mut Carrier {
    &mut engine
        .domination_gameplay
        .as_mut()
        .unwrap()
        .state
        .helo
        .as_mut()
        .unwrap()
        .carriers[index]
}

fn board(carrier: &mut Carrier, player: u16) -> PassengerSeat {
    let object_index = carrier.source.object_index;
    let seat = carrier.motion.board_passenger(player).unwrap();
    PassengerSeat { object_index, seat }
}

fn begin_cycle(carrier: &mut Carrier) {
    carrier.motion.set_cycle(true, Some(0));
    carrier.motion.advance(0.).unwrap();
}

fn fixture() -> (Engine<Vec<u8>>, PassengerSeat, PassengerSeat) {
    let mut engine = hospital_rocket_runtime::tests::tick_fixture();
    domination_structural_world::hospital_rocket_helo_damage::tests::install_helo_fixture(
        &mut engine,
        CharacterPosition {
            x: 1.,
            y: 2.,
            z: 3.,
        },
    );
    let helo = carrier(&mut engine, 0);
    begin_cycle(helo);
    let seat = board(helo, 0);
    let peer_seat = board(helo, 1);
    let squad = engine.peers[PEER].squad_catalog.clone();
    let actor = engine.active.as_mut().unwrap();
    actor.squad_catalog = squad;
    actor.mounted_bunker = None;
    actor.mounted_seat = None;
    actor.helo_creation_sent = true;
    actor.helo_seat = Some(seat);
    actor.owner_fields.state = Some(seat.character_state());
    let peer = engine.peers.get_mut(PEER).unwrap();
    peer.helo_creation_sent = true;
    peer.helo_seat = Some(peer_seat);
    peer.owner_fields.state = Some(peer_seat.character_state());
    (engine, seat, peer_seat)
}

fn redeploy(
    engine: &mut Engine<Vec<u8>>,
    target: u16,
) -> Result<Vec<OutboundMessage>, &'static str> {
    let mut wire = 0x0215u16.to_be_bytes().to_vec();
    wire.extend((0x0380_0000 | u32::from(target)).to_be_bytes());
    dispatch(engine, &wire, 1_000)
}

fn notices(actor: &Active) -> Vec<&[u8]> {
    actor
        .pending_notifications
        .iter()
        .filter_map(|publication| match publication {
            match_flow::Publication::Application(wire) if wire.starts_with(&[1, 0xff]) => {
                Some(wire.as_slice())
            }
            _ => None,
        })
        .collect()
}

#[test]
fn helo_redeploy_immediately_releases_the_owned_seat_and_publishes_state_seven() {
    let (mut engine, seat, peer_seat) = fixture();
    let helo = carrier(&mut engine, 0);
    helo.current_pose = None;
    helo.live_basis = None;
    helo.damage.state = VehicleState::Critical;
    helo.damage.wire.health_quarter_points = 1;

    let frames = redeploy(&mut engine, 0).unwrap();
    let actor = engine.active.as_ref().unwrap();
    assert!(matches!(
        actor.life,
        CharacterLife::AwaitingRespawn { generation: 1 }
    ));
    assert_eq!(actor.helo_seat, None);
    assert_eq!(actor.last_damage.health_quarter_points, 0);
    let notice = protocol::decode_hex("01ff0000000000ffffffff40");
    assert_eq!(notices(actor), [notice.as_slice()]);

    // With no current carrier pose the body leaves at its last tracked pose.
    let terminal = crate::seated_death::tests::terminal_for(
        crate::seated_death::tests::frame_wires(&frames)
            .iter()
            .map(Vec::as_slice),
        0,
        engine.wire_profile,
    );
    crate::seated_death::tests::assert_seated_exit(&terminal);
    assert_eq!((terminal.bunker, terminal.ground_owner), (None, None));

    assert_eq!(engine.peers[PEER].helo_seat, Some(peer_seat));
    assert!(engine.peers[PEER].life.is_alive());
    let helo = carrier(&mut engine, 0);
    assert!(helo.motion.passengers().eq([(peer_seat.seat, 1)]));
    assert_eq!(helo.damage.wire.health_quarter_points, 1);
    assert_eq!(board(helo, 2), seat);
}

#[test]
fn helo_redeploy_leaves_the_carrier_at_its_current_pose() {
    let (mut engine, _, _) = fixture();
    let pose = CharacterPosition {
        x: 40.,
        y: 12.,
        z: 30.,
    };
    let helo = carrier(&mut engine, 0);
    helo.current_pose = Some(crate::domination_helo::FlightPose {
        position: pose,
        forward: [0., 0., 1.],
    });
    let frames = redeploy(&mut engine, 0).unwrap();
    let terminal = crate::seated_death::tests::terminal_for(
        crate::seated_death::tests::frame_wires(&frames)
            .iter()
            .map(Vec::as_slice),
        0,
        engine.wire_profile,
    );
    crate::seated_death::tests::assert_seated_exit(&terminal);
    assert_eq!(terminal.position, Some(pose));
    assert_eq!(engine.active.as_ref().unwrap().helo_seat, None);
}

#[test]
fn a_downed_rider_update_leaves_the_carrier_whatever_killed_it() {
    // Carrier destruction and every other cause share this update builder.
    let (mut engine, _, _) = fixture();
    let pose = CharacterPosition {
        x: 20.,
        y: 9.,
        z: 25.,
    };
    carrier(&mut engine, 0).current_pose = Some(crate::domination_helo::FlightPose {
        position: pose,
        forward: [0., 0., 1.],
    });
    let player = engine.peers[PEER].reservation.player;
    let damage = engine.peers[PEER].last_damage;
    let update = engine.character_damage_update(9, 1, player, damage, true);
    let exit = update.seated_exit.expect("rider exit");
    assert_eq!(exit.position, pose);
    assert_eq!((exit.bunker_ownership, exit.ground_owner), (None, None));
}

#[test]
fn refused_helo_redeploy_history_budget_keeps_the_authoritative_rider() {
    let (mut engine, seat, _) = fixture();
    let before = carrier(&mut engine, 0)
        .motion
        .passengers()
        .collect::<Vec<_>>();
    engine.maximum_players = 1;
    engine.world_tick_enabled = false;
    engine.active.as_mut().unwrap().replication_history = vec![vec![]; MAX_REPLICATION_HISTORY];
    redeploy(&mut engine, 0).unwrap();
    assert!(engine.active.as_ref().unwrap().life.is_alive());
    assert_eq!(engine.active.as_ref().unwrap().helo_seat, Some(seat));
    assert_eq!(
        carrier(&mut engine, 0)
            .motion
            .passengers()
            .collect::<Vec<_>>(),
        before
    );
}

#[test]
fn helo_redeploy_rejects_stale_or_conflicting_occupancy() {
    for invalid in [
        "wrong carrier source",
        "missing creation",
        "wrong faction",
        "wrong squad",
        "wrong passenger seat",
        "duplicate fleet row",
        "duplicate actor mirror",
        "conflicting mount family",
    ] {
        let (mut engine, seat, peer_seat) = fixture();
        match invalid {
            "wrong carrier source" => carrier(&mut engine, 0).source.object_index = 1,
            "missing creation" => engine.active.as_mut().unwrap().helo_creation_sent = false,
            "wrong faction" => {
                engine
                    .active
                    .as_mut()
                    .unwrap()
                    .reservation
                    .character
                    .faction = 1
            }
            "wrong squad" => carrier(&mut engine, 0).source.squads = [2, 3],
            "wrong passenger seat" => {
                engine.active.as_mut().unwrap().helo_seat = Some(peer_seat);
            }
            "duplicate fleet row" => {
                let helo = carrier(&mut engine, 1);
                begin_cycle(helo);
                board(helo, 0);
            }
            "duplicate actor mirror" => {
                engine.peers.get_mut(PEER).unwrap().helo_seat = Some(seat);
            }
            "conflicting mount family" => {
                engine.active.as_mut().unwrap().mounted_seat = Some(0);
            }
            _ => unreachable!(),
        }
        let actor_seat = engine.active.as_ref().unwrap().helo_seat;
        redeploy(&mut engine, 0).unwrap();
        let actor = engine.active.as_ref().unwrap();
        assert!(actor.life.is_alive(), "{invalid}");
        assert_eq!(actor.helo_seat, actor_seat, "{invalid}");
        assert!(notices(actor).is_empty(), "{invalid}");
        assert!(
            carrier(&mut engine, 0)
                .motion
                .passengers()
                .eq([(peer_seat.seat, 1), (seat.seat, 0)]),
            "{invalid}"
        );
    }
}

#[test]
fn helo_redeploy_preserves_both_occupancy_mirrors_when_recording_fails() {
    let (mut engine, seat, peer_seat) = fixture();
    engine.max_log = engine.written;
    assert_eq!(redeploy(&mut engine, 0).err(), Some("log_limit_reached"));
    let actor = engine.active.as_ref().unwrap();
    assert!(actor.life.is_alive());
    assert_eq!(actor.helo_seat, Some(seat));
    assert!(notices(actor).is_empty());
    assert!(
        carrier(&mut engine, 0)
            .motion
            .passengers()
            .eq([(peer_seat.seat, 1), (seat.seat, 0)])
    );
}
