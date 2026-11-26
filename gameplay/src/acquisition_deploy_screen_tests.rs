//! A member whose client already holds the objective bootstrap but sits on
//! the deploy screen (not ready for updates) still receives every objective
//! change. Party members are admitted together, so the first to deploy starts
//! the round while the others are still choosing a kit and spawn.
use super::*;

/// The objective-type (1 << 12) packets queued for one recipient, in order.
fn objective_packets(e: &Engine<Vec<u8>>, player: u16) -> Vec<Vec<u8>> {
    queued(e, player)
        .into_iter()
        .filter(|packet| mask(e, packet) == 1 << 12)
        .collect()
}

fn queued(e: &Engine<Vec<u8>>, player: u16) -> Vec<Vec<u8>> {
    e.active
        .iter()
        .chain(e.peers.values())
        .find(|actor| actor.reservation.player == player)
        .unwrap()
        .pending_shared
        .iter()
        .cloned()
        .collect()
}

fn mask(e: &Engine<Vec<u8>>, packet: &[u8]) -> u32 {
    ObjectUpdate::decode_with_profile(packet, e.wire_profile)
        .unwrap()
        .header
        .object_type_mask
}

fn deploy_screen_member(e: &mut Engine<Vec<u8>>, player: u16) {
    reserve(e, player);
    let mut member = e.active.take().unwrap();
    // Bootstrapped (initial objects and objective catalog sent), then the
    // client reported ready=false while it picks a kit and spawn.
    ready(&mut member);
    member.ready_for_updates = false;
    member.entered_game = false;
    e.peers.insert(member.reservation.id.clone(), member);
}

fn set_ready(e: &mut Engine<Vec<u8>>, player: u16, ready: bool) {
    // The fixture has already completed its authored Ground registration.
    let batches = e.ground_vehicles.as_ref().unwrap().batch_count();
    let actor = e
        .active
        .iter_mut()
        .chain(e.peers.values_mut())
        .find(|actor| actor.reservation.player == player)
        .unwrap();
    actor.ready_for_updates = ready;
    actor.entered_game |= ready;
    actor.ground_vehicle_batch = batches;
}

#[test]
fn round_start_reaches_a_party_member_still_on_the_deploy_screen() {
    let mut e = capture_fixture();
    deploy_screen_member(&mut e, 1);
    // The other member deploys first; its entry starts the round.
    prepare_capture_actor(&mut e, 0, 100);
    let deployed = objective_packets(&e, 0);
    assert!(
        !deployed.is_empty(),
        "the round start publishes objective state"
    );
    assert_eq!(objective_packets(&e, 1), deployed);
    // Spawn availability changes at round start too; the deploy screen needs it.
    let spawns = |e: &Engine<Vec<u8>>, player| {
        queued(e, player)
            .into_iter()
            .filter(|packet| mask(e, packet) == 1 << 8)
            .collect::<Vec<_>>()
    };
    assert_eq!(spawns(&e, 1), spawns(&e, 0));

    // Once ready, the queued state drains to it in order.
    set_ready(&mut e, 1, true);
    let mut delivered = Vec::new();
    for _ in 0..64 {
        let drained = e.drain_shared(Vec::new(), 1).unwrap();
        if drained.is_empty() {
            break;
        }
        delivered.extend(
            drained
                .into_iter()
                .filter(|message| message.player_id == 1)
                .map(|message| protocol::decode_hex(&message.payload_hex))
                .filter(|packet| mask(&e, packet) == 1 << 12)
                .map(|mut packet| {
                    // Drain stamps each recipient's own reliable sequence.
                    packet[2..6].fill(0);
                    packet
                }),
        );
    }
    let expected: Vec<_> = deployed
        .into_iter()
        .map(|mut packet| {
            packet[2..6].fill(0);
            packet
        })
        .collect();
    assert_eq!(delivered, expected);
}

#[test]
fn capture_reaches_a_late_member_on_the_deploy_screen_like_a_ready_one() {
    let mut e = capture_fixture();
    prepare_capture_actor(&mut e, 0, 100);
    // A second member joins the running round and is sent the current
    // objective snapshot, then returns to the deploy screen.
    let first = e.active.take().unwrap();
    e.peers.insert(first.reservation.id.clone(), first);
    deploy_screen_member(&mut e, 1);
    let player = e.peers.remove("synthetic-reservation-0000").unwrap();
    e.active = Some(player);
    let before = [0, 1].map(|player| objective_packets(&e, player).len());

    let position = e.active.as_ref().unwrap().tracking.sample.unwrap().position;
    let outcome = e.objective_world.as_mut().unwrap().interact(
        protocol::ObjectiveInteraction {
            kind: protocol::ObjectiveInteractionKind::Activate,
            target_object_id: 0x0c80_0000 | u32::from(FIRST_CAPTURE),
            actor_index: 0,
            trigger_index: 0,
        },
        ObjectiveActor {
            player: 0,
            team: 0,
            generation: 1,
            position,
        },
        Duration::from_millis(200),
    );
    assert!(outcome.accepted, "{}", outcome.reason);
    for now in [250, 3300] {
        e.tick_acquisition_objectives(Duration::from_millis(now), 2)
            .unwrap();
    }
    assert_eq!(
        e.objective_world.as_ref().unwrap().state(FIRST_CAPTURE),
        Some(ObjectiveState::Completed)
    );
    let ready = objective_packets(&e, 0)[before[0]..].to_vec();
    assert!(!ready.is_empty(), "the capture publishes objective state");
    assert_eq!(objective_packets(&e, 1)[before[1]..], ready);
}

#[test]
fn a_loading_member_without_objectives_waits_for_its_own_snapshot() {
    let mut e = capture_fixture();
    reserve(&mut e, 1);
    let mut loading = e.active.take().unwrap();
    ready(&mut loading);
    loading.ready_for_updates = false;
    loading.objective_delta = None;
    e.peers.insert(loading.reservation.id.clone(), loading);
    prepare_capture_actor(&mut e, 0, 100);
    assert!(!objective_packets(&e, 0).is_empty());
    // Its bootstrap will carry the current state; earlier deltas would only
    // arrive after that snapshot and roll it back.
    assert!(objective_packets(&e, 1).is_empty());
}
