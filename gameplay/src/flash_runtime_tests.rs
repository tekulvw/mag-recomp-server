//! Synthetic native-wire Flash integration; no client artifacts.
use crate::hospital_rocket_runtime::tests as rockets;
use crate::spawn_progression::dispatch;
use crate::*;
use std::sync::Arc;
const PEER: &str = "synthetic-rocket-target";
const POS: replication::CharacterPosition = replication::CharacterPosition {
    x: 1.5,
    y: 0.5,
    z: 2.5,
};
pub(crate) fn fixture() -> Engine<Vec<u8>> {
    let mut engine = smoke_runtime_tests::fixture();
    engine.smoke_grenades = None;
    engine.flash_grenades = Some(Arc::new(flash_grenades::Catalog::synthetic(
        std::array::from_fn(|f| flash_grenades::Variant {
            faction: f as u8,
            item_id: 451,
            ammo_token: 0x1fe2_3480 + f as u32,
            initial_count: 3,
            fuse_ms: 100,
            projectile_lifetime_ms: 10000,
            inner_radius_m: 10.,
            outer_radius_m: 40.,
            material_penetration: 1.7,
        }),
    )));
    engine
        .domination_gameplay
        .as_mut()
        .unwrap()
        .state
        .filter_rows[24] = 0x0040_0601;
    let actor = engine.active.as_mut().unwrap();
    actor.grenade_remaining = 3;
    actor
        .tracking
        .server_position(POS, player_tracking::PositionOrigin::ServerSpawn, 1);
    actor.owner_fields.state = Some(replication::CharacterState {
        state: 1,
        stance: 0,
        user_data: [0; 3],
    });
    let peer = engine.peers.get_mut(PEER).unwrap();
    peer.owner_fields.state = Some(replication::CharacterState {
        state: 1,
        stance: 0,
        user_data: [0; 3],
    });
    peer.tracking.server_position(
        replication::CharacterPosition { x: 2., ..POS },
        player_tracking::PositionOrigin::ServerSpawn,
        1,
    );
    engine
}
fn report(engine: &Engine<Vec<u8>>, sequence: u32, yaw: Option<u16>) -> Vec<u8> {
    let mut b = replication::BitWriter::with_profile(engine.wire_profile);
    replication::UpdateHeader {
        sequence,
        nak_sequence: 0,
        client_index: 0,
        object_type_mask: 8,
        timestamp_ms: sequence,
    }
    .encode(&mut b)
    .unwrap();
    b.write(1, 1).unwrap();
    b.write(
        engine.active.as_ref().unwrap().reservation.player.into(),
        16,
    )
    .unwrap();
    for field in 0..22 {
        b.write(u32::from(field == 7 && yaw.is_some()), 1).unwrap();
        if field == 7
            && let Some(yaw) = yaw
        {
            b.write(yaw.into(), 10).unwrap();
        }
    }
    b.write(0, 1).unwrap();
    b.finish()
}
fn blind(messages: &[OutboundMessage]) -> Vec<&OutboundMessage> {
    messages
        .iter()
        .filter(|m| m.payload_hex.starts_with("0200"))
        .collect()
}
fn event(
    engine: &mut Engine<Vec<u8>>,
    wire: &[u8],
    transport: Transport,
    ms: u64,
) -> Result<Vec<OutboundMessage>, &'static str> {
    let actor = engine.active.as_ref().unwrap();
    let input = events::EventInput {
        reservation_id: actor.reservation.id.clone(),
        world_id: actor.reservation.world,
        player_id: actor.reservation.player,
        transport,
        direction: Direction::ClientToServer,
        rt_type: 0x3d,
        plugin_id: Some(1),
        payload_hex: protocol::encode_hex(wire),
    };
    let decoded = DecodedEvent::with_profile(wire, input.player_id, engine.wire_profile, false);
    engine.handle_event(input, &decoded, Duration::from_millis(ms), 1)
}
fn terminal(engine: &Engine<Vec<u8>>, sequence: u32) -> Vec<u8> {
    smoke_runtime_tests::owner_terminal(engine, sequence)
}
fn launch(engine: &mut Engine<Vec<u8>>, slot: u16, ms: u64) {
    let wire = rockets::launch_wire(engine, slot, 4);
    dispatch(engine, &wire, ms).unwrap();
}
#[test]
fn flash_authenticated_yaw_is_record_bound_fresh_and_not_spawn_or_tcp_presentation() {
    let mut e = fixture();
    assert!(e.active.as_ref().unwrap().flash_yaw.is_none());
    let wire = report(&e, 1100, Some(256));
    event(&mut e, &wire, Transport::Tcp, 1000).unwrap();
    assert_eq!(e.active.as_ref().unwrap().owner_fields.yaw, Some(256));
    assert!(e.active.as_ref().unwrap().flash_yaw.is_none());
    let wire = report(&e, 1200, Some(512));
    dispatch(&mut e, &wire, 1001).unwrap();
    assert_eq!(
        e.active
            .as_ref()
            .unwrap()
            .flash_yaw
            .and_then(|w| w.current(e.active.as_ref().unwrap())),
        Some(512)
    );
    let stale = report(&e, 1199, Some(0));
    dispatch(&mut e, &stale, 1002).unwrap();
    assert_eq!(e.active.as_ref().unwrap().flash_yaw.unwrap().yaw, 512);
    let wire = report(&e, 1300, Some(768));
    e.max_log = e.written;
    assert_eq!(
        dispatch(&mut e, &wire, 1003).unwrap_err(),
        "log_limit_reached"
    );
    assert_eq!(e.active.as_ref().unwrap().flash_yaw.unwrap().yaw, 512);
    let a = e.active.as_mut().unwrap();
    let w = a.flash_yaw.unwrap();
    a.mounted_bunker = Some(1);
    a.mounted_seat = Some(0);
    assert_eq!(w.current(a), None);
    a.mounted_seat = None;
    a.mounted_bunker = None;
    a.allocation_sequence += 1;
    assert_eq!(w.current(a), None);
    a.allocation_sequence -= 1;
    a.life = CharacterLife::Alive {
        generation: 5,
        health_quarter_points: 400,
    };
    assert_eq!(w.current(a), None);
}
#[test]
fn flash_native_launch_terminal_blinds_self_and_ally_once_without_damage_xp_or_volume() {
    let mut e = fixture();
    // Self body faces +Z, explosion differs vertically only: zero forward dot => weak.
    let yaw = report(&e, 1001, Some(0));
    dispatch(&mut e, &yaw, 999).unwrap();
    let source_team = e.active.as_ref().unwrap().team();
    let squad = e.active.as_ref().unwrap().squad_catalog.clone();
    let peer = e.peers.get_mut(PEER).unwrap();
    peer.squad_catalog = squad;
    assert_eq!(peer.team(), source_team);
    peer.flash_yaw = Some(flash_grenades::YawWitness {
        allocation: peer.allocation_sequence,
        generation: 4,
        yaw: 768,
    });
    let xp = e.active.as_ref().unwrap().match_experience;
    launch(&mut e, 3, 1000);
    assert_eq!(e.active.as_ref().unwrap().grenade_remaining, 2);
    let early = terminal(&e, 1050);
    assert!(blind(&dispatch(&mut e, &early, 1050).unwrap()).is_empty());
    let wire = terminal(&e, 1100);
    let messages = dispatch(&mut e, &wire, 1100).unwrap();
    let blind = blind(&messages);
    assert_eq!(
        blind.len(),
        2,
        "{messages:?} log={}",
        String::from_utf8_lossy(&e.writer)
    );
    assert!(blind.iter().any(|m| m.reservation_id == PEER));
    assert!(
        blind
            .iter()
            .any(|m| m.reservation_id == e.active.as_ref().unwrap().reservation.id)
    );
    for m in blind {
        assert_eq!(m.plugin_id, 1);
        assert_eq!(m.world_id, 1);
        assert_eq!(m.payload_hex.len(), 10);
    }
    let source = e.active.as_ref().unwrap();
    assert_eq!(source.life.damage_health().unwrap().1, 400);
    assert_eq!(e.peers[PEER].life.damage_health().unwrap().1, 400);
    assert_eq!(source.match_experience, xp);
    assert!(source.smoke_volumes.volumes().is_empty());
    assert!(source.grenade_damage.is_empty());
    assert!(
        source
            .projectile_publications
            .iter()
            .all(|p| !p.include_owner)
    );
    assert_eq!(
        messages
            .iter()
            .find(|m| m.reservation_id == PEER && m.payload_hex.starts_with("0200"))
            .unwrap()
            .payload_hex,
        "0200938000"
    );
    assert_eq!(messages.iter().find(|m|m.reservation_id==source.reservation.id && m.payload_hex.starts_with("0200")).unwrap().payload_hex,"0200803300");
    let replay = terminal(&e, 1101);
    assert!(blind_messages_empty(
        &dispatch(&mut e, &replay, 1101).unwrap()
    ));
}
fn blind_messages_empty(messages: &[OutboundMessage]) -> bool {
    blind(messages).is_empty()
}
#[test]
fn flash_missing_facing_is_audited_and_no_weak_packet_is_invented() {
    let mut e = fixture();
    launch(&mut e, 3, 1000);
    let wire = terminal(&e, 1100);
    assert!(blind(&dispatch(&mut e, &wire, 1100).unwrap()).is_empty());
    assert!(String::from_utf8_lossy(&e.writer).contains("flash_facing_witness_required"));
}
#[test]
fn flash_terminal_record_failure_rolls_back_parent_ammo_and_publication() {
    let mut e = fixture();
    launch(&mut e, 3, 1000);
    let wire = terminal(&e, 1100);
    e.max_log = e.written;
    let before = e.active.as_ref().unwrap().projectile_publications.len();
    assert_eq!(
        dispatch(&mut e, &wire, 1100).unwrap_err(),
        "log_limit_reached"
    );
    let source = e.active.as_ref().unwrap();
    assert!(!source.projectiles.get(3).unwrap().completed);
    assert_eq!(source.projectile_publications.len(), before);
    assert!(source.projectile_cleanup[3].is_none());
    assert_eq!(source.grenade_remaining, 2);
}
#[test]
fn flash_three_finite_uses_share_slots_and_do_not_require_old_flash_skill() {
    let mut e = fixture();
    e.active.as_mut().unwrap().reservation.character.skills = Some([1, 0, 0, 0, 0]);
    for (n, slot) in [0, 1, 2].into_iter().enumerate() {
        launch(&mut e, slot, 1000 + n as u64);
        assert_eq!(e.active.as_ref().unwrap().grenade_remaining, 2 - n as u8);
    }
    launch(&mut e, 3, 1004);
    assert_eq!(e.active.as_ref().unwrap().grenade_remaining, 0);
    assert!(e.active.as_ref().unwrap().projectiles.get(3).is_none());
    assert!(String::from_utf8_lossy(&e.writer).contains("grenade_ammo_empty"));
}

#[test]
fn flash_known_cover_forces_weak_without_yaw_while_unknown_material_emits_nothing() {
    for material in [Some(255), None] {
        let mut e = fixture();
        let mut face = serde_json::json!({"id":"flash-cover","vertices_xyz":[[1.75,0.,0.],[1.75,6.,0.],[1.75,0.,6.]],"triangles":[[0,1,2]],"collision":{"filter_info":9,"ray_child_filter_info":0,"convex_radius":0.,"child_filter_policy":"uniform","provenance":"synthetic"}});
        if let Some(material) = material {
            face["material_index"] = material.into();
        }
        e.collision_world=Some(collision::CollisionWorld::from_bytes(serde_json::json!({"format":"mag_collision_world","schema_version":1,"coordinate_space":"world","metadata":{"map_name":e.map_name(),"build":"synthetic","coverage":"provisional","provenance":["synthetic"],"unknowns":[]},"instances":[face]}).to_string().as_bytes(),Default::default()).unwrap());
        launch(&mut e, 3, 1000);
        let wire = terminal(&e, 1100);
        let messages = dispatch(&mut e, &wire, 1100).unwrap();
        if material.is_some() {
            let blind = blind(&messages);
            assert_eq!(blind.len(), 1);
            assert_eq!(blind[0].reservation_id, PEER);
            assert_eq!(blind[0].payload_hex, "0200803180");
        } else {
            assert!(blind(&messages).is_empty());
            assert!(
                String::from_utf8_lossy(&e.writer)
                    .contains("domination_structural_blast_material_penetration_required")
            );
        }
    }
}
#[test]
fn flash_peer_terminal_owner_non_echo_cleanup_and_slot_reuse_are_ordered() {
    let mut e = fixture();
    launch(&mut e, 3, 1000);
    let wire = terminal(&e, 1100);
    dispatch(&mut e, &wire, 1100).unwrap();
    e.queue_projectile_publications(Duration::from_millis(1100))
        .unwrap();
    let terminals = |actor: &Active| {
        actor
            .pending_shared
            .iter()
            .filter(|wire| {
                replication::ObjectUpdate::decode_with_profile(wire, e.wire_profile)
                    .ok()
                    .and_then(|u| {
                        projectile::decode_projectile_updates_with_profile(&u, e.wire_profile).ok()
                    })
                    .is_some_and(|reports| reports.iter().any(|r| r.fields.state == Some(2)))
            })
            .count()
    };
    assert_eq!(terminals(e.active.as_ref().unwrap()), 0);
    assert_eq!(terminals(&e.peers[PEER]), 1);
    let request = rockets::launch_wire(&e, 3, 4);
    dispatch(&mut e, &request, 1101).unwrap();
    assert_eq!(e.active.as_ref().unwrap().grenade_remaining, 2);
    smoke_runtime_tests::drain_shared(&mut e, 1601);
    smoke_runtime_tests::drain_shared(&mut e, 2102);
    // A new request fingerprint is required for a new launch in the reused slot.
    let mut new_request = request;
    new_request[2..6].copy_from_slice(&2_f32.to_bits().to_be_bytes());
    dispatch(&mut e, &new_request, 2103).unwrap();
    assert_eq!(e.active.as_ref().unwrap().grenade_remaining, 1);
    assert!(
        !e.active
            .as_ref()
            .unwrap()
            .projectiles
            .get(3)
            .unwrap()
            .completed
    );
}

fn mixed_terminal(engine: &Engine<Vec<u8>>, sequence: u32, yaw: u16, state: Option<u8>) -> Vec<u8> {
    let mut b = replication::BitWriter::with_profile(engine.wire_profile);
    replication::UpdateHeader {
        sequence,
        nak_sequence: 0,
        client_index: 0,
        object_type_mask: 8 | (1 << 19),
        timestamp_ms: sequence,
    }
    .encode(&mut b)
    .unwrap();
    let player = engine.active.as_ref().unwrap().reservation.player;
    b.write(1, 1).unwrap();
    b.write(player.into(), 16).unwrap();
    for field in 0..22 {
        b.write(u32::from(field == 7 || (field == 4 && state.is_some())), 1)
            .unwrap();
        if field == 4
            && let Some(state) = state
        {
            replication::CharacterState {
                state,
                stance: 0,
                user_data: [0; 3],
            }
            .encode(&mut b)
            .unwrap();
        }
        if field == 7 {
            b.write(yaw.into(), 10).unwrap();
        }
    }
    b.write(0, 1).unwrap();
    b.write(1, 1).unwrap();
    b.write(player.into(), 16).unwrap();
    projectile::ProjectileOwnerFields {
        state: Some(4),
        position: Some(replication::CharacterPosition { y: 2.5, ..POS }),
        rotation_xyzw: None,
    }
    .encode_fields(&mut b)
    .unwrap();
    b.write(0, 1).unwrap();
    b.finish()
}
#[test]
fn flash_same_recorded_terminal_uses_accepted_self_yaw_including_first_and_turn_away() {
    for (turn_away, refuse_record) in [(false, true), (false, false), (true, false)] {
        let mut e = fixture();
        let actor = e.active.as_mut().unwrap();
        actor.tracking.server_position(
            replication::CharacterPosition { z: 0., ..POS },
            player_tracking::PositionOrigin::ServerSpawn,
            1,
        );
        if turn_away {
            actor.flash_yaw = Some(flash_grenades::YawWitness {
                allocation: actor.allocation_sequence,
                generation: 4,
                yaw: 0,
            });
        }
        launch(&mut e, 3, 1000);
        let wire = mixed_terminal(&e, 1100, if turn_away { 512 } else { 0 }, None);
        if refuse_record {
            e.max_log = e.written;
            assert_eq!(
                dispatch(&mut e, &wire, 1100).unwrap_err(),
                "log_limit_reached"
            );
            assert!(e.active.as_ref().unwrap().flash_yaw.is_none());
            assert!(
                !e.active
                    .as_ref()
                    .unwrap()
                    .projectiles
                    .get(3)
                    .unwrap()
                    .completed
            );
            continue;
        }
        let messages = dispatch(&mut e, &wire, 1100).unwrap();
        let own = &e.active.as_ref().unwrap().reservation.id;
        let packet = messages
            .iter()
            .find(|m| &m.reservation_id == own && m.payload_hex.starts_with("0200"))
            .unwrap();
        assert_eq!(
            packet.payload_hex,
            if turn_away {
                "0200802c80"
            } else {
                "0200918000"
            }
        );
        assert_eq!(
            e.active.as_ref().unwrap().flash_yaw.unwrap().yaw,
            if turn_away { 512 } else { 0 }
        );
    }
}
#[test]
fn flash_reported_mounted_state_cannot_seed_on_foot_facing() {
    let mut e = fixture();
    launch(&mut e, 3, 1000);
    let wire = mixed_terminal(&e, 1100, 0, Some(14));
    let messages = dispatch(&mut e, &wire, 1100).unwrap();
    assert!(blind(&messages).is_empty());
    assert!(e.active.as_ref().unwrap().flash_yaw.is_none());
}
