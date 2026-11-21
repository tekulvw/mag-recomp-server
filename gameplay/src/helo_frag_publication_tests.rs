//! Typed Helo field18 uses each recipient's actual creation and queue state.
use super::*;
use crate::domination_structural_world::hospital_rocket_helo_damage::tests::frag_fixture;

fn add_peer(
    engine: &mut Engine<Vec<u8>>,
    label: &str,
    player: u16,
    created: bool,
    ready: bool,
) -> String {
    let mut peer = frag_fixture(false).active.take().unwrap();
    let id = format!("synthetic-helo-frag-{label}");
    peer.reservation.id = id.clone();
    peer.reservation.player = player;
    peer.reservation.account += u32::from(player) + 1;
    peer.helo_creation_sent = created;
    peer.ready_for_updates = ready;
    peer.pending_shared.clear();
    peer.hospital_bootstrap = 0..0;
    peer.replication_backlogged = false;
    engine.peers.insert(id.clone(), peer);
    id
}

fn publish(engine: &mut Engine<Vec<u8>>, require_creation: bool) -> Vec<u8> {
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
    let now = Duration::from_millis(1100);
    let packet = crate::domination_helo_damage::damage_delta(
        0,
        None,
        Some(replication::VehicleDamage {
            health_quarter_points: 272,
            conditions: 0,
            last_attacker: 0x0380_0000 | u32::from(actor.reservation.player),
            last_location_and_type: 0xa2,
            last_source_object: u32::MAX,
            last_user_data: (128 | 512) << 20,
        }),
        now,
        engine.wire_profile,
    )
    .unwrap();
    let mut plan = EventPlan::new(actor);
    plan.helo_damage_requires_creation = require_creation;
    plan.helo_damage_relay.push(packet.clone());
    let decoded =
        DecodedEvent::with_profile(&[], actor.reservation.player, engine.wire_profile, true);
    engine.commit_event(&event, &decoded, plan, now).unwrap();
    packet
}

#[test]
fn helo_frag_publication_requires_same_world_ready_creation() {
    let mut engine = frag_fixture(true);
    let ready = add_peer(&mut engine, "ready", 1, true, true);
    let precreation = add_peer(&mut engine, "precreation", 2, false, true);
    let loading = add_peer(&mut engine, "loading", 3, true, false);
    let uninitialized = add_peer(&mut engine, "uninitialized", 4, true, true);
    let other_world = add_peer(&mut engine, "other-world", 5, true, true);
    engine
        .peers
        .get_mut(&uninitialized)
        .unwrap()
        .initial_objects = None;
    engine
        .peers
        .get_mut(&other_world)
        .unwrap()
        .reservation
        .world += 1;
    let source = engine.active.as_mut().unwrap();
    source.helo_creation_sent = true;
    source.ready_for_updates = true;
    source.pending_shared.clear();
    source.hospital_bootstrap = 0..0;
    let packet = publish(&mut engine, true);
    assert!(engine.peers[&ready].pending_shared.contains(&packet));
    assert!(
        engine
            .active
            .as_ref()
            .unwrap()
            .pending_shared
            .contains(&packet)
    );
    for id in [precreation, loading, uninitialized, other_world] {
        assert!(!engine.peers[&id].pending_shared.contains(&packet), "{id}");
    }
}

#[test]
fn helo_frag_publication_legacy_keeps_the_prior_recipient_rule() {
    let mut engine = frag_fixture(false);
    let ready = add_peer(&mut engine, "legacy-precreation", 1, false, true);
    let packet = publish(&mut engine, false);
    assert!(engine.peers[&ready].pending_shared.contains(&packet));
}

#[test]
fn helo_frag_publication_full_recipient_keeps_bounded_retirement_policy() {
    let mut engine = frag_fixture(true);
    let full = add_peer(&mut engine, "full", 1, true, true);
    let healthy = add_peer(&mut engine, "healthy", 2, true, true);
    let precreation = add_peer(&mut engine, "full-precreation", 3, false, true);
    for id in [&full, &precreation] {
        engine.peers.get_mut(id).unwrap().pending_shared =
            std::iter::repeat_n(vec![0], multiplayer::MAX_SHARED_QUEUE).collect();
    }
    let packet = publish(&mut engine, true);
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
    assert!(engine.peers.contains_key(&precreation));
    assert!(engine.active.is_some());
}

#[test]
fn helo_frag_publication_aim_budget_counts_only_actual_helo_recipients() {
    for created in [false, true] {
        let mut engine = frag_fixture(true);
        let peer = add_peer(&mut engine, "aim-budget", 1, created, true);
        engine.peers.get_mut(&peer).unwrap().pending_shared =
            std::iter::repeat_n(vec![0], multiplayer::MAX_SHARED_QUEUE - 1).collect();
        let mut plan = EventPlan::new(engine.active.as_ref().unwrap());
        plan.helo_damage_requires_creation = true;
        // Only packet counts matter to this budget pass; encoding is upstream.
        plan.helo_damage_relay.push(Vec::new());
        plan.hospital_aim_packets.push(Vec::new());
        engine.stage_turret_aim_budget(&mut plan);
        assert_eq!(plan.hospital_aim_packets.len(), usize::from(!created));
        assert_eq!(plan.gameplay_denial.is_some(), created);
    }
}
