//! Hospital field18 eligibility and capacity at the outer event commit.
use super::*;
use crate::domination_structural_world::{
    HospitalBlastCandidate, domination_hospital_blast::tests::frag_fixture,
};

fn add_peer(engine: &mut Engine<Vec<u8>>, label: &str, player: u16, created: bool) -> String {
    let mut peer = frag_fixture(false).active.take().unwrap();
    let id = format!("synthetic-hospital-blast-{label}");
    peer.reservation.id = id.clone();
    peer.reservation.player = player;
    peer.reservation.account += u32::from(player) + 1;
    peer.emplacement_creation = created.then(Vec::new);
    peer.pending_shared.clear();
    peer.hospital_bootstrap = 0..0;
    peer.replication_backlogged = false;
    engine.peers.insert(id.clone(), peer);
    id
}

fn packet(engine: &Engine<Vec<u8>>) -> Vec<u8> {
    let mut bits = replication::BitWriter::with_profile(engine.wire_profile);
    replication::UpdateHeader {
        sequence: 1,
        nak_sequence: 0,
        client_index: 0,
        object_type_mask: 1 << 13,
        timestamp_ms: 1100,
    }
    .encode(&mut bits)
    .unwrap();
    bits.write(1, 1).unwrap();
    bits.write(5, 16).unwrap();
    for field in 0..25 {
        bits.write(u32::from(field == 18), 1).unwrap();
        if field == 18 {
            replication::VehicleDamage {
                health_quarter_points: 672,
                conditions: 0,
                last_attacker: 0x0380_0000,
                last_location_and_type: 0xa2,
                last_source_object: u32::MAX,
                last_user_data: (128 | 512) << 20,
            }
            .encode(&mut bits)
            .unwrap();
        }
    }
    bits.write(0, 1).unwrap();
    bits.finish()
}

fn publish(engine: &mut Engine<Vec<u8>>, create_in_this_event: bool) -> Vec<u8> {
    let actor = engine.active.as_ref().unwrap();
    let event = EventInput {
        reservation_id: actor.reservation.id.clone(),
        world_id: actor.reservation.world,
        player_id: actor.reservation.player,
        transport: Transport::Udp,
        direction: Direction::ClientToServer,
        rt_type: 0x3d,
        plugin_id: Some(1),
        payload_hex: String::new(),
    };
    let packet = packet(engine);
    let mut plan = EventPlan::new(actor);
    plan.hospital_blast = Some(HospitalBlastCandidate {
        packet: Some(packet.clone()),
        ..Default::default()
    });
    if create_in_this_event {
        let sequence = actor.next_sequence().unwrap();
        let creations = engine
            .bunker_world
            .as_ref()
            .unwrap()
            .creations(sequence, 1100);
        plan.replication.emplacement_creation = Some(
            replication::DefensiveEmplacementCreation::encode_list_with_profile(
                sequence,
                1100,
                &creations,
                engine.wire_profile,
            )
            .unwrap(),
        );
    }
    let decoded =
        DecodedEvent::with_profile(&[], actor.reservation.player, engine.wire_profile, true);
    engine
        .commit_event(&event, &decoded, plan, Duration::from_millis(1100))
        .unwrap();
    packet
}

#[test]
fn hospital_blast_publication_requires_same_world_emplacement_creation() {
    let mut engine = frag_fixture(true);
    let ready = add_peer(&mut engine, "created", 1, true);
    let precreation = add_peer(&mut engine, "precreation", 2, false);
    let loading = add_peer(&mut engine, "created-loading", 3, true);
    let other = add_peer(&mut engine, "other-world", 4, true);
    engine.peers.get_mut(&loading).unwrap().ready_for_updates = false;
    engine.peers.get_mut(&other).unwrap().reservation.world += 1;
    let packet = publish(&mut engine, false);
    assert!(engine.peers[&ready].pending_shared.contains(&packet));
    assert!(engine.peers[&loading].pending_shared.contains(&packet));
    assert!(!engine.peers[&precreation].pending_shared.contains(&packet));
    assert!(!engine.peers[&other].pending_shared.contains(&packet));
}

#[test]
fn hospital_blast_publication_accepts_creation_committed_in_same_event() {
    let mut engine = frag_fixture(true);
    let actor = engine.active.as_mut().unwrap();
    actor.emplacement_creation = None;
    actor.pending_shared.clear();
    actor.hospital_bootstrap = 0..0;
    let packet = publish(&mut engine, true);
    let actor = engine.active.as_ref().unwrap();
    assert!(actor.emplacement_creation.is_some());
    assert!(actor.pending_shared.contains(&packet));
}

#[test]
fn hospital_blast_publication_full_recipient_retires_without_blocking_healthy() {
    let mut engine = frag_fixture(true);
    let full = add_peer(&mut engine, "full", 1, true);
    let healthy = add_peer(&mut engine, "healthy", 2, true);
    let precreation = add_peer(&mut engine, "full-precreation", 3, false);
    for id in [&full, &precreation] {
        engine.peers.get_mut(id).unwrap().pending_shared =
            std::iter::repeat_n(vec![0], multiplayer::MAX_SHARED_QUEUE).collect();
    }
    let packet = publish(&mut engine, false);
    assert!(engine.peers[&healthy].pending_shared.contains(&packet));
    assert_eq!(
        engine.peers[&full].ordinary_shared_len(),
        multiplayer::MAX_SHARED_QUEUE
    );
    assert!(engine.peers[&full].replication_backlogged);
    assert!(!engine.peers[&precreation].replication_backlogged);
    engine
        .expire_world_peers(Duration::from_millis(1101), 1)
        .unwrap();
    assert!(!engine.peers.contains_key(&full));
    assert!(engine.peers.contains_key(&healthy));
    assert!(engine.active.is_some());
}

#[test]
fn hospital_blast_publication_aim_budget_counts_actual_emplacement_recipients() {
    for created in [false, true] {
        let mut engine = frag_fixture(true);
        let id = add_peer(&mut engine, "aim-budget", 1, created);
        engine.peers.get_mut(&id).unwrap().pending_shared =
            std::iter::repeat_n(vec![0], multiplayer::MAX_SHARED_QUEUE - 1).collect();
        let mut plan = EventPlan::new(engine.active.as_ref().unwrap());
        plan.hospital_blast = Some(HospitalBlastCandidate {
            packet: Some(packet(&engine)),
            ..Default::default()
        });
        plan.hospital_aim_packets.push(Vec::new());
        engine.stage_turret_aim_budget(&mut plan);
        assert_eq!(plan.hospital_aim_packets.len(), usize::from(!created));
        assert_eq!(plan.gameplay_denial.is_some(), created);
    }
}
