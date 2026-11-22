use crate::spawn_progression::dispatch;
use crate::structures::domination_bunker_seat_tests::fixture;
use crate::*;

fn request(
    engine: &mut Engine<Vec<u8>>,
    kind: u16,
    index: u16,
    seat: u8,
    now: u64,
) -> Result<Vec<OutboundMessage>, &'static str> {
    let player = engine.active.as_ref().unwrap().reservation.player;
    let mut wire = kind.to_be_bytes().to_vec();
    if kind == 0x0216 {
        wire.extend((0x0d00_0000 | u32::from(index)).to_be_bytes());
        wire.extend(u32::from(seat).to_be_bytes());
    }
    wire.extend((0x0380_0000 | u32::from(player)).to_be_bytes());
    dispatch(engine, &wire, now)
}

fn add_peer(engine: &mut Engine<Vec<u8>>, player: u16, index: u16, seat: u8) {
    let mut peer = fixture().active.take().unwrap();
    peer.reservation.id = format!("synthetic-hospital-observer-{player}");
    peer.reservation.player = player;
    peer.mounted_bunker = Some(index);
    peer.mounted_seat = Some(seat);
    peer.owner_fields.state = Some(replication::CharacterState {
        state: 14,
        stance: 0,
        user_data: [13, index as u8, seat],
    });
    engine.peers.insert(peer.reservation.id.clone(), peer);
}

fn ownership(actor: &Active, profile: wire_profile::WireProfile) -> Vec<(u16, u16, u16)> {
    actor
        .pending_shared
        .iter()
        .filter_map(|wire| {
            let update = replication::ObjectUpdate::decode_with_profile(wire, profile).ok()?;
            if update.header.object_type_mask != 1 << 13 {
                return None;
            }
            let mut bits = replication::BitReader::with_profile(wire, profile);
            bits.position = profile.header_bits();
            assert_eq!(bits.read(1), Ok(1));
            let index = bits.read(16).unwrap() as u16;
            let mut owner = None;
            let mut turret = None;
            for field in 0..25 {
                let present = bits.read(1).unwrap() != 0;
                if !present {
                    continue;
                }
                match field {
                    1 => owner = Some(bits.read(16).unwrap() as u16),
                    6 => turret = Some(bits.read(16).unwrap() as u16),
                    _ => return None,
                }
            }
            Some((index, owner?, turret?))
        })
        .collect()
}

#[test]
fn hospital_passenger_redeploy_publishes_owner_fallback_without_moving_other_occupants() {
    let mut engine = fixture();
    engine.item_catalog = human_kill_reward_tests::bot_firearm_items();
    request(&mut engine, 0x0216, 5, 12, 1000).unwrap();
    add_peer(&mut engine, 1, 5, 11);
    add_peer(&mut engine, 2, 6, 12);
    for actor in engine.active.iter_mut().chain(engine.peers.values_mut()) {
        actor.pending_shared.clear();
    }
    request(&mut engine, 0x0215, 0, 0, 1100).unwrap();
    let active = engine.active.as_ref().unwrap();
    assert!(matches!(
        active.life,
        CharacterLife::AwaitingRespawn { generation: 1 }
    ));
    assert!(active.mounted_bunker.is_none() && active.mounted_seat.is_none());
    assert_eq!(ownership(active, engine.wire_profile), [(5, 1, u16::MAX)]);
    for peer in engine.peers.values() {
        assert_eq!(ownership(peer, engine.wire_profile), [(5, 1, u16::MAX)]);
        assert!(peer.mounted_seat.is_some());
    }
    request(&mut engine, 0x0215, 0, 0, 1101).unwrap();
    assert_eq!(
        ownership(engine.active.as_ref().unwrap(), engine.wire_profile).len(),
        1
    );
}

#[test]
fn hospital_gunner_redeploy_revokes_grant_and_allows_a_fresh_takeover() {
    use hospital_turrets::tests::enable_synthetic_hospital_turrets;
    let mut engine = fixture();
    enable_synthetic_hospital_turrets(&mut engine);
    request(&mut engine, 0x0216, 5, 0, 1000).unwrap();
    let old_version = engine.bunker_world.as_ref().unwrap().hospital_turrets[&5]
        .definition(true)
        .version;
    assert!(engine.bunker_world.as_ref().unwrap().hospital_turrets[&5].has_grant());
    add_peer(&mut engine, 1, 5, 12);
    for actor in engine.active.iter_mut().chain(engine.peers.values_mut()) {
        actor.pending_shared.clear();
    }
    request(&mut engine, 0x0215, 0, 0, 1100).unwrap();
    assert!(matches!(
        engine.active.as_ref().unwrap().life,
        CharacterLife::AwaitingRespawn { .. }
    ));
    assert!(!engine.bunker_world.as_ref().unwrap().hospital_turrets[&5].has_grant());
    assert_eq!(
        ownership(engine.active.as_ref().unwrap(), engine.wire_profile),
        [(5, 1, u16::MAX)]
    );
    let previous = engine.active.take().unwrap();
    let mut incoming = fixture().active.take().unwrap();
    incoming.reservation.id = "synthetic-incoming-gunner".into();
    incoming.reservation.player = 2;
    engine.active = Some(incoming);
    engine
        .peers
        .insert(previous.reservation.id.clone(), previous);
    request(&mut engine, 0x0216, 5, 0, 1200).unwrap();
    let turret = &engine.bunker_world.as_ref().unwrap().hospital_turrets[&5];
    assert!(turret.grant_matches(engine.active.as_ref().unwrap()));
    assert!(turret.definition(true).version > old_version);
    assert_eq!(
        engine.peers["synthetic-hospital-observer-1"].mounted_seat,
        Some(12)
    );
}

#[test]
fn hospital_redeploy_failed_record_and_full_reliable_queue_keep_seats_and_grants() {
    for full_queue in [false, true] {
        let mut engine = fixture();
        hospital_turrets::tests::enable_synthetic_hospital_turrets(&mut engine);
        request(&mut engine, 0x0216, 5, 0, 1000).unwrap();
        add_peer(&mut engine, 1, 6, 12);
        if full_queue {
            let peer = engine.peers.values_mut().next().unwrap();
            peer.pending_shared = std::collections::VecDeque::from(vec![vec![0]; 128]);
        } else {
            engine.max_log = engine.written;
        }
        let result = request(&mut engine, 0x0215, 0, 0, 1100);
        if full_queue {
            result.unwrap();
        } else {
            assert_eq!(result.err(), Some("log_limit_reached"));
        }
        let actor = engine.active.as_ref().unwrap();
        assert!(actor.life.is_alive());
        assert_eq!(
            (actor.mounted_bunker, actor.mounted_seat),
            (Some(5), Some(0))
        );
        assert!(engine.bunker_world.as_ref().unwrap().hospital_turrets[&5].grant_matches(actor));
        assert_eq!(engine.peers.values().next().unwrap().mounted_seat, Some(12));
    }
}

#[test]
fn hospital_redeploy_refuses_unqualified_or_duplicate_mounts() {
    for bad in [0, 1, 2] {
        let mut engine = fixture();
        request(&mut engine, 0x0216, 5, 12, 1000).unwrap();
        match bad {
            0 => engine.active.as_mut().unwrap().mounted_bunker = Some(0),
            1 => add_peer(&mut engine, 1, 5, 12),
            _ => {
                engine.active.as_mut().unwrap().ground_seat = Some(ground_seats::PassengerSeat {
                    vehicle_index: 0,
                    seat: 2,
                })
            }
        }
        request(&mut engine, 0x0215, 0, 0, 1100).unwrap();
        assert!(engine.active.as_ref().unwrap().life.is_alive());
        assert!(engine.active.as_ref().unwrap().mounted_seat.is_some());
    }
}

#[test]
fn refused_hospital_redeploy_history_budget_keeps_current_ownership() {
    let mut engine = fixture();
    request(&mut engine, 0x0216, 5, 12, 1000).unwrap();
    engine.active.as_mut().unwrap().pending_shared.clear();
    engine.maximum_players = 1;
    engine.world_tick_enabled = false;
    engine.active.as_mut().unwrap().replication_history = vec![vec![]; MAX_REPLICATION_HISTORY];
    request(&mut engine, 0x0215, 0, 0, 1100).unwrap();
    let actor = engine.active.as_ref().unwrap();
    assert!(actor.life.is_alive());
    assert_eq!(
        (actor.mounted_bunker, actor.mounted_seat),
        (Some(5), Some(12))
    );
    assert!(ownership(actor, engine.wire_profile).is_empty());
}

#[test]
fn hospital_redeploy_reserves_owner_ordered_health_and_mandatory_ownership_together() {
    for queued in [126, 127] {
        let mut engine = fixture();
        hospital_turrets::tests::enable_synthetic_hospital_turrets(&mut engine);
        request(&mut engine, 0x0216, 5, 0, 1000).unwrap();
        let actor = engine.active.as_mut().unwrap();
        actor.ordered_health_pending = true;
        actor.pending_shared = std::collections::VecDeque::from(vec![vec![0]; queued]);
        request(&mut engine, 0x0215, 0, 0, 1100).unwrap();
        let actor = engine.active.as_ref().unwrap();
        assert!(!actor.replication_backlogged, "queue {queued}");
        if queued == 126 {
            assert!(matches!(actor.life, CharacterLife::AwaitingRespawn { .. }));
            assert_eq!(actor.ordinary_shared_len(), 128);
            assert_eq!(
                ownership(actor, engine.wire_profile),
                [(5, 0x105, u16::MAX)]
            );
        } else {
            assert!(actor.life.is_alive());
            assert_eq!(actor.ordinary_shared_len(), 127);
            assert!(ownership(actor, engine.wire_profile).is_empty());
            assert!(
                engine.bunker_world.as_ref().unwrap().hospital_turrets[&5].grant_matches(actor)
            );
        }
    }
}
