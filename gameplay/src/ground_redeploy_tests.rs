//! Native owned redeploy through actual synthetic Ground mount/insertion routes.
use super::domination_ground_driver::{driver_engine, ready_on_foot};
use super::*;

fn redeploy(
    engine: &mut Engine<Vec<u8>>,
    target: u16,
    now: u64,
) -> Result<Vec<OutboundMessage>, &'static str> {
    let mut wire = 0x0215u16.to_be_bytes().to_vec();
    wire.extend((0x0380_0000 | u32::from(target)).to_be_bytes());
    dispatch(engine, &wire, now)
}

fn notices(actor: &Active) -> Vec<String> {
    actor
        .pending_notifications
        .iter()
        .filter_map(|p| match p {
            match_flow::Publication::Application(wire) if wire.starts_with(&[1, 0xff]) => {
                Some(protocol::encode_hex(wire))
            }
            _ => None,
        })
        .collect()
}

fn terminal_packet(frames: &[OutboundMessage], profile: wire_profile::WireProfile) {
    let wire = frames
        .iter()
        .map(|f| protocol::decode_hex(&f.payload_hex))
        .find(|wire| {
            ObjectUpdate::decode_with_profile(wire, profile)
                .is_ok_and(|u| u.header.object_type_mask == 1 << 3)
        })
        .expect("terminal Character update");
    let mut bits = BitReader::with_profile(&wire, profile);
    bits.position = profile.header_bits();
    assert_eq!(bits.read(1), Ok(1));
    assert_eq!(bits.read(16), Ok(0));
    for field in 0..22 {
        assert_eq!(
            bits.read(1),
            Ok(u32::from(field == 4 || field == 8)),
            "field {field}"
        );
        if field == 4 {
            assert_eq!(bits.read(8), Ok(7));
            assert_eq!(bits.read(24), Ok(0));
        } else if field == 8 {
            assert_eq!(bits.read(8), Ok(0x10));
            assert_eq!(bits.read(16), Ok(0));
            assert_eq!(bits.read(32), Ok(0x0380_0000));
            assert_eq!(bits.read(32), Ok(u32::MAX));
            assert_eq!(bits.read(32), Ok(0));
        }
    }
    assert_eq!(bits.read(1), Ok(0));
}

#[test]
fn ground_passenger_redeploy_clears_seat_publishes_terminal_state_and_allows_reuse() {
    let mut engine = reinforced_domination_apc();
    engine.item_catalog = crate::human_kill_reward_tests::bot_firearm_items();
    let vehicle_before = engine.current_ground_vehicle(0).unwrap();
    assert_eq!(engine.active.as_ref().unwrap().ground_seat.unwrap().seat, 9);
    let frames = redeploy(&mut engine, 0, 200).unwrap();
    let actor = engine.active.as_ref().unwrap();
    assert!(matches!(
        actor.life,
        CharacterLife::AwaitingRespawn { generation: 1 }
    ));
    assert!(actor.ground_seat.is_none());
    assert!(actor.pending_spawn_deadline.is_none());
    assert_eq!(actor.last_damage.health_quarter_points, 0);
    // self0/self0, Unknown0, signed-1, associated vehicle flag0x40.
    assert_eq!(notices(actor), ["01ff0000000000ffffffff40"]);
    terminal_packet(&frames, engine.wire_profile);
    assert_eq!(
        engine.current_ground_vehicle(0).unwrap().position,
        vehicle_before.position
    );
    assert_eq!(
        engine
            .current_ground_vehicle(0)
            .unwrap()
            .health_quarter_points,
        vehicle_before.health_quarter_points
    );
    redeploy(&mut engine, 0, 201).unwrap();
    assert_eq!(notices(engine.active.as_ref().unwrap()).len(), 1);
    assert!(engine.pending_reward_batch().is_empty());
    spawn_apc(&mut engine, 1, 1, 0, 210);
    assert_eq!(engine.active.as_ref().unwrap().ground_seat.unwrap().seat, 9);
}

#[test]
fn ground_driver_redeploy_revokes_ownership_and_new_driver_can_mount() {
    let mut engine = driver_engine(0);
    ready_on_foot(&mut engine, 0, 10);
    ground_seat_event(&mut engine, true, 0x0100_0000, 0, 0, 20);
    assert_eq!(engine.ground_driver_owner(0), Some(0));
    redeploy(&mut engine, 0, 30).unwrap();
    assert!(matches!(
        engine.active.as_ref().unwrap().life,
        CharacterLife::AwaitingRespawn { .. }
    ));
    assert_eq!(engine.ground_driver_owner(0), None);
    assert!(
        engine
            .active
            .as_ref()
            .unwrap()
            .pending_shared
            .iter()
            .any(|wire| {
                ObjectUpdate::decode_with_profile(wire, engine.wire_profile)
                    .is_ok_and(|u| u.header.object_type_mask == 1 << 1)
            })
    );
    ready_on_foot(&mut engine, 1, 40);
    ground_seat_event(&mut engine, true, 0x0100_0000, 0, 1, 50);
    assert_eq!(engine.ground_driver_owner(0), Some(1));
}

#[test]
fn ground_gunner_redeploy_revokes_grant_and_frees_the_seat() {
    use crate::ground_turrets::tests::{equip_synthetic_ground_turret, opt_in};
    let mut engine = driver_engine(8);
    opt_in(&mut engine, 8);
    ready_on_foot(&mut engine, 0, 10);
    equip_synthetic_ground_turret(&mut engine, 8);
    assert_eq!(
        engine.ground_gunner_kind(engine.active.as_ref().unwrap()),
        Some(crate::ground_turrets::PolicyKind::Apc)
    );
    redeploy(&mut engine, 0, 200).unwrap();
    assert!(matches!(
        engine.active.as_ref().unwrap().life,
        CharacterLife::AwaitingRespawn { .. }
    ));
    assert!(engine.active.as_ref().unwrap().ground_seat.is_none());
    assert_eq!(
        engine.ground_turret_snapshot(8).unwrap().turret_owner,
        Some(u16::MAX)
    );
    ready_on_foot(&mut engine, 1, 210);
    ground_seat_event(&mut engine, true, 0x0100_0008, 1, 1, 220);
    assert_eq!(
        engine.ground_gunner_kind(engine.active.as_ref().unwrap()),
        Some(crate::ground_turrets::PolicyKind::Apc)
    );
}

#[test]
fn ground_redeploy_requires_owned_target_and_failed_record_preserves_mount() {
    let mut engine = reinforced_domination_apc();
    engine.item_catalog = crate::human_kill_reward_tests::bot_firearm_items();
    let seat = engine.active.as_ref().unwrap().ground_seat;
    redeploy(&mut engine, 1, 200).unwrap();
    assert!(engine.active.as_ref().unwrap().life.is_alive());
    assert_eq!(engine.active.as_ref().unwrap().ground_seat, seat);
    engine.max_log = engine.written;
    assert_eq!(
        redeploy(&mut engine, 0, 210).err(),
        Some("log_limit_reached")
    );
    assert!(engine.active.as_ref().unwrap().life.is_alive());
    assert_eq!(engine.active.as_ref().unwrap().ground_seat, seat);
    assert!(notices(engine.active.as_ref().unwrap()).is_empty());
}

#[test]
fn ground_redeploy_does_not_infer_a_current_mount_from_invalid_seat_state() {
    for invalid in [0, 1, 2] {
        let mut engine = reinforced_domination_apc();
        match invalid {
            0 => {
                engine
                    .active
                    .as_mut()
                    .unwrap()
                    .ground_seat
                    .as_mut()
                    .unwrap()
                    .seat = 1
            }
            1 => engine.ground_runtime = None,
            _ => {
                engine.active.as_mut().unwrap().helo_seat =
                    Some(crate::domination_helo::PassengerSeat {
                        object_index: 0,
                        seat: 2,
                    })
            }
        }
        redeploy(&mut engine, 0, 200).unwrap();
        assert!(engine.active.as_ref().unwrap().life.is_alive());
        assert!(engine.active.as_ref().unwrap().ground_seat.is_some());
        assert!(notices(engine.active.as_ref().unwrap()).is_empty());
    }
}

#[test]
fn owned_onfoot_redeploy_publishes_unknown_source_notice_and_suicide_once() {
    let mut engine = crate::human_kill_reward_tests::fixture();
    redeploy(&mut engine, 1, 1000).unwrap();
    let actor = engine.active.as_ref().unwrap();
    assert!(matches!(
        actor.life,
        CharacterLife::AwaitingRespawn { generation: 4 }
    ));
    assert_eq!(notices(actor), ["01ff0001000100ffffffff00"]);
    assert_eq!(actor.reward_statistics.get(&12), Some(&1));
    assert_eq!(actor.reward_statistics.get(&13), Some(&1));
    assert!(engine.pending_reward_batch().is_empty());
    redeploy(&mut engine, 1, 1100).unwrap();
    assert_eq!(notices(engine.active.as_ref().unwrap()).len(), 1);
    assert_eq!(
        engine.active.as_ref().unwrap().reward_statistics.get(&12),
        Some(&1)
    );
}

#[test]
fn downed_giveup_preserves_the_original_collapse_notice_and_counters() {
    let mut engine = crate::human_kill_reward_tests::fixture();
    crate::human_kill_reward_tests::hit(&mut engine, 99, 1000).unwrap();
    let shooter = engine.active.take().unwrap();
    engine.active = engine.peers.remove("synthetic-reservation-0000");
    engine.peers.insert(shooter.reservation.id.clone(), shooter);
    assert!(matches!(
        engine.active.as_ref().unwrap().life,
        CharacterLife::Incapacitated { .. }
    ));
    let original = notices(engine.active.as_ref().unwrap());
    assert_eq!(original, ["01ff00010000020000000200"]);
    let rewards = engine.pending_reward_batch().len();
    redeploy(&mut engine, 0, 1100).unwrap();
    assert!(matches!(
        engine.active.as_ref().unwrap().life,
        CharacterLife::AwaitingRespawn { generation: 4 }
    ));
    assert_eq!(notices(engine.active.as_ref().unwrap()), original);
    assert_eq!(
        engine.active.as_ref().unwrap().reward_statistics.get(&12),
        Some(&1)
    );
    assert!(
        !engine
            .active
            .as_ref()
            .unwrap()
            .reward_statistics
            .contains_key(&13)
    );
    assert_eq!(engine.pending_reward_batch().len(), rewards);
}
