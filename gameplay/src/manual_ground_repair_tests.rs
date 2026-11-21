//! Authenticated manual Ground repair against source-shaped synthetic vehicles.
use super::*;

pub(crate) const TARGET: u32 = 0x0100_0004;

pub(crate) fn fixture() -> Engine<Vec<u8>> {
    let mut e = spawn_progression::tests::dynamic_bodies::acquisition_gameplay_tests::ground_firearm_fixture(0.,true);
    structural_repair_authority::tests::equip_synthetic_repair_kit(&mut e);
    e.bunker_world = Some(structural_repair_authority::tests::synthetic_repair_bunker_world());
    let batch = e.ground_vehicles.as_ref().unwrap().batch_count();
    let actor = e.active.as_mut().unwrap();
    actor.ground_vehicle_batch = batch;
    actor.reservation.character.account_id = Some("synthetic-ground-engineer".into());
    actor.reservation.character.progression = Some(progression::ProgressionSnapshot::default());
    actor.reservation.character.repair_rewards_enabled = Some(true);
    actor.reservation.character.frago_action_bonus_enabled = Some(true);
    actor.pending_shared.clear();
    let key = e.ground_runtime.as_ref().unwrap().get(4).unwrap().key;
    e.ground_runtime
        .as_mut()
        .unwrap()
        .apply_damage(key, 160, 0x0380_0007)
        .unwrap();
    e
}

pub(crate) fn repair(
    e: &mut Engine<Vec<u8>>,
    target: u32,
    ms: u64,
    transport: Transport,
) -> Result<Vec<OutboundMessage>, &'static str> {
    let mut wire = vec![0x02, 0x1d];
    wire.extend(target.to_be_bytes());
    let actor = e.active.as_ref().unwrap();
    let input = events::EventInput {
        reservation_id: actor.reservation.id.clone(),
        world_id: actor.reservation.world,
        player_id: actor.reservation.player,
        transport,
        direction: Direction::ClientToServer,
        rt_type: 0x3d,
        plugin_id: Some(1),
        payload_hex: protocol::encode_hex(&wire),
    };
    let decoded = DecodedEvent::with_profile(
        &wire,
        input.player_id,
        e.wire_profile,
        e.ground_driver_enabled(),
    );
    e.handle_event(input, &decoded, Duration::from_millis(ms), 3)
}

fn hp(e: &Engine<Vec<u8>>) -> u16 {
    e.ground_runtime
        .as_ref()
        .unwrap()
        .get(4)
        .unwrap()
        .health_quarter_points
}

#[test]
fn manual_escort_repair_restores_hp_and_publishes_work_once() {
    let mut e = fixture();
    let before = e.ground_runtime.as_ref().unwrap().get(4).unwrap().clone();
    let messages = repair(&mut e, TARGET, 4000, Transport::Tcp).unwrap();
    assert_eq!(hp(&e), 508);
    let after = e.ground_runtime.as_ref().unwrap().get(4).unwrap();
    assert_eq!(after.pose, before.pose);
    assert_eq!(after.body, before.body);
    assert_eq!(after.state, before.state);
    assert_eq!(after.last_attacker, 0x0380_0007);
    assert_eq!(
        e.active.as_ref().unwrap().reward_statistics.get(&35),
        Some(&1)
    );
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].payload_hex, "02040000012301");
    assert_eq!(e.pending_reward_batch().len(), 1);
    assert_eq!(
        e.pending_reward_batch()[0].kind,
        progression::RewardKind::RepairUnits
    );
    assert_eq!(e.active.as_ref().unwrap().pending_shared.len(), 1);
    repair(&mut e, TARGET, 4001, Transport::Tcp).unwrap();
    assert_eq!(hp(&e), 508);
    assert_eq!(e.pending_reward_batch().len(), 1);
    repair(&mut e, TARGET, 5250, Transport::Tcp).unwrap();
    assert_eq!(hp(&e), 576);
    assert_eq!(e.pending_reward_batch().len(), 2);
}

#[test]
fn unsupported_routes_and_actor_vehicle_guards_never_mutate_ground_or_work() {
    for guard in [
        "udp",
        "unknown",
        "family",
        "team",
        "loading",
        "dead_actor",
        "mounted",
        "kit",
        "closed",
        "full",
        "dying",
        "dead_vehicle",
        "delivered",
    ] {
        let mut e = fixture();
        let key = e.ground_runtime.as_ref().unwrap().get(4).unwrap().key;
        let mut target = TARGET;
        let mut transport = Transport::Tcp;
        match guard {
            "udp" => transport = Transport::Udp,
            "unknown" => target = 0x0100_001f,
            "family" => target = 0x0101_0004,
            "team" => {
                e.active.as_mut().unwrap().squad_catalog =
                    std::sync::Arc::new(e.spawn_catalog.as_ref().unwrap().for_squad(16).unwrap())
            }
            "loading" => e.active.as_mut().unwrap().ground_vehicle_batch = 0,
            "dead_actor" => {
                e.active.as_mut().unwrap().life = CharacterLife::AwaitingRespawn { generation: 1 }
            }
            "mounted" => {
                e.active.as_mut().unwrap().ground_seat = Some(ground_seats::PassengerSeat {
                    vehicle_index: 4,
                    seat: 0,
                })
            }
            "kit" => {
                e.active
                    .as_mut()
                    .unwrap()
                    .observed_primary
                    .as_mut()
                    .unwrap()
                    .inventory_update_id += 1
            }
            "closed" => e.match_started = false,
            "full" => {
                e.ground_runtime
                    .as_mut()
                    .unwrap()
                    .heal_percent(key, 100.)
                    .unwrap();
            }
            "dying" => {
                e.ground_runtime
                    .as_mut()
                    .unwrap()
                    .apply_damage(key, 360, 0x0380_0007)
                    .unwrap();
            }
            "dead_vehicle" => {
                e.ground_runtime
                    .as_mut()
                    .unwrap()
                    .apply_damage(key, 440, 0x0380_0007)
                    .unwrap();
            }
            "delivered" => {
                let g = e.ground_runtime.as_mut().unwrap();
                g.configure(
                    key,
                    acquisition_ground_runtime::Capabilities {
                        delivered_inert_seconds: Some(1.),
                        ..Default::default()
                    },
                )
                .unwrap();
                g.deliver(key).unwrap();
            }
            _ => unreachable!(),
        }
        let before = e.ground_runtime.clone();
        repair(&mut e, target, 4000, transport).unwrap();
        assert_eq!(e.ground_runtime, before, "{guard}");
        assert_eq!(
            e.active.as_ref().unwrap().repair_credit,
            repair_rewards::State::default(),
            "{guard}"
        );
        assert!(
            !e.active
                .as_ref()
                .unwrap()
                .reward_statistics
                .contains_key(&35),
            "{guard}"
        );
        assert!(e.pending_reward_batch().is_empty(), "{guard}");
        assert!(
            e.active.as_ref().unwrap().pending_shared.is_empty(),
            "{guard}"
        );
    }
}

#[test]
fn friendly_last_attacker_suppresses_work_without_suppressing_manual_hp_restore() {
    for friendly in [false, true] {
        let mut e = fixture();
        let tools = std::sync::Arc::clone(&e.item_catalog);
        e.item_catalog=spawn_progression::tests::dynamic_bodies::acquisition_gameplay_tests::ground_firearm_fixture(0.,true).item_catalog;
        spawn_progression::reserve(&mut e, 1);
        e.item_catalog = tools;
        let source = e.active.as_mut().unwrap();
        spawn_progression::ready(source);
        source.squad_catalog = std::sync::Arc::new(
            e.spawn_catalog
                .as_ref()
                .unwrap()
                .for_squad(if friendly { 0 } else { 16 })
                .unwrap(),
        );
        let source = e.active.take().unwrap();
        e.peers.insert(source.reservation.id.clone(), source);
        e.active = e.peers.remove("synthetic-reservation-0000");
        let key = e.ground_runtime.as_ref().unwrap().get(4).unwrap().key;
        e.ground_runtime
            .as_mut()
            .unwrap()
            .apply_bullet_damage(key, 0.25, 0x0380_0001, 5, 42)
            .unwrap();
        repair(&mut e, TARGET, 4000, Transport::Tcp).unwrap();
        assert_eq!(hp(&e), 507);
        let model = e
            .ground_runtime
            .as_ref()
            .unwrap()
            .get(4)
            .unwrap()
            .damage_wire();
        assert_eq!(
            (
                model.last_attacker,
                model.last_location_and_type,
                model.last_user_data
            ),
            (0x0380_0001, 0x15, 42)
        );
        assert_eq!(model.last_source_object, u32::MAX);
        assert_eq!(e.pending_reward_batch().len(), usize::from(!friendly));
        assert_eq!(
            e.active.as_ref().unwrap().reward_statistics.get(&35),
            if friendly { None } else { Some(&1) }
        );
    }
}

#[test]
fn current_repaired_vehicle_pose_supplies_frago_context_and_missing_xp_opt_in_keeps_hp() {
    let mut e = fixture();
    frago_tests::configure_geometry(&mut e);
    e.acquisition_gameplay
        .as_mut()
        .unwrap()
        .state
        .squad_mut(0)
        .unwrap()
        .assignment = acquisition_objective_runtime::Assignment {
        primary_index: u16::MAX,
        tagged_object: 0x0c80_0064,
    };
    let key = e.ground_runtime.as_ref().unwrap().get(4).unwrap().key;
    e.ground_runtime
        .as_mut()
        .unwrap()
        .force_pose(
            key,
            acquisition_ground_runtime::Pose {
                position: replication::CharacterPosition {
                    x: 100.,
                    y: 3.,
                    z: 100.,
                },
                rotation: [0., 0., 0., 1.],
            },
        )
        .unwrap();
    e.active.as_mut().unwrap().tracking.server_position(
        replication::CharacterPosition {
            x: 130.,
            y: 3.,
            z: 100.,
        },
        player_tracking::PositionOrigin::ServerSpawn,
        0,
    );
    repair(&mut e, TARGET, 4000, Transport::Tcp).unwrap();
    let reward = e.pending_reward_batch().pop().unwrap();
    assert_eq!(reward.frago, Some(true));
    e.acquisition_gameplay
        .as_mut()
        .unwrap()
        .state
        .squad_mut(0)
        .unwrap()
        .assignment
        .tagged_object = u32::MAX;
    assert_eq!(e.pending_reward_batch()[0], reward);
    let mut e = fixture();
    e.active
        .as_mut()
        .unwrap()
        .reservation
        .character
        .repair_rewards_enabled = None;
    repair(&mut e, TARGET, 4000, Transport::Tcp).unwrap();
    assert_eq!(hp(&e), 508);
    assert!(e.pending_reward_batch().is_empty());
    assert_eq!(
        e.active.as_ref().unwrap().reward_statistics.get(&35),
        Some(&1)
    );
}

#[test]
fn recorder_failure_leaves_hp_credit_cooldown_and_queue_unchanged_then_retry_is_once() {
    let mut e = fixture();
    let before = e.ground_runtime.clone();
    let max = e.max_log;
    e.max_log = e.written;
    assert_eq!(
        repair(&mut e, TARGET, 4000, Transport::Tcp).err(),
        Some("log_limit_reached")
    );
    assert_eq!(e.ground_runtime, before);
    assert!(e.next_repair.is_empty());
    assert!(e.pending_reward_batch().is_empty());
    assert_eq!(
        e.active.as_ref().unwrap().repair_credit,
        repair_rewards::State::default()
    );
    assert!(e.active.as_ref().unwrap().pending_shared.is_empty());
    e.max_log = max;
    e.failed = false;
    repair(&mut e, TARGET, 4000, Transport::Tcp).unwrap();
    assert_eq!(hp(&e), 508);
    assert_eq!(e.pending_reward_batch().len(), 1);
}

#[test]
fn full_reward_outbox_prevents_hp_or_cooldown_commit() {
    let mut e = fixture();
    let before = e.ground_runtime.clone();
    let recipient = progression::RewardRecipient::from_active(e.active.as_ref().unwrap()).unwrap();
    e.commit_rewards(
        (0..progression::MAX_PENDING_REWARDS)
            .map(|i| {
                recipient.event(
                    format!("synthetic-ground-pending-{i}"),
                    progression::RewardKind::EnemyFinished,
                    1,
                )
            })
            .collect(),
    );
    assert_eq!(
        repair(&mut e, TARGET, 4000, Transport::Tcp).err(),
        Some("reward_outbox_full")
    );
    assert_eq!(e.ground_runtime, before);
    assert!(e.next_repair.is_empty());
    assert_eq!(
        e.active.as_ref().unwrap().repair_credit,
        repair_rewards::State::default()
    );
    e.pending_rewards.clear();
    e.reward_order.clear();
    repair(&mut e, TARGET, 4000, Transport::Tcp).unwrap();
    assert_eq!(hp(&e), 508);
    assert_eq!(e.pending_reward_batch().len(), 1);
}

fn set_modifier(e: &mut Engine<Vec<u8>>, value: f32) {
    let catalog=skill_catalog::SkillCatalog::from_value(json!({
        "schema_version":2,"application_id":20231,"table":{"Skills":{
            "MagCharacterSkill1":{"MagCharacterSkill":{"Default":{"Modifier":"kCharModRepairAmount","ModifierValue":value},
                "Green":{"Available":true},"Black":{"Available":true},"Red":{"Available":true}}}
        }}
    })).unwrap();
    let actor = e.active.as_mut().unwrap();
    actor.modifiers = catalog
        .resolve(actor.reservation.character.faction, &[2, 0, 0, 0, 0])
        .unwrap();
}

#[test]
fn subquarter_float_repair_counts_work_while_zero_applied_hp_does_not() {
    let mut e = fixture();
    set_modifier(&mut e, -0.99264705);
    repair(&mut e, TARGET, 4000, Transport::Tcp).unwrap();
    let vehicle = e.ground_runtime.as_ref().unwrap().get(4).unwrap();
    assert!(vehicle.health_hp() > 110.0);
    assert_eq!(vehicle.health_quarter_points, 440);
    let actor = e.active.as_ref().unwrap();
    assert!(actor.repair_credit.carry > 0.0);
    assert_eq!(actor.repair_credit.sequence, 1);
    assert!(e.pending_reward_batch().is_empty());
    assert_eq!(actor.pending_shared.len(), 1);
    let mut e = fixture();
    set_modifier(&mut e, -1.0);
    let before = e.ground_runtime.clone();
    repair(&mut e, TARGET, 4000, Transport::Tcp).unwrap();
    assert_eq!(e.ground_runtime, before);
    assert_eq!(
        e.active.as_ref().unwrap().repair_credit,
        repair_rewards::State::default()
    );
    assert!(e.active.as_ref().unwrap().pending_shared.is_empty());
    assert!(e.pending_reward_batch().is_empty());
}

#[test]
fn effective_modifier_scales_hp_and_independent_work_carry() {
    let mut e = fixture();
    set_modifier(&mut e, 0.5);
    repair(&mut e, TARGET, 4000, Transport::Tcp).unwrap();
    assert_eq!(hp(&e), 542);
    assert_eq!(e.active.as_ref().unwrap().repair_credit.carry, 0.5);
    assert_eq!(e.pending_reward_batch()[0].units, 1);
    repair(&mut e, TARGET, 5250, Transport::Tcp).unwrap();
    assert_eq!(hp(&e), 600);
    assert_eq!(e.active.as_ref().unwrap().repair_credit.carry, 0.0);
    assert_eq!(
        e.active.as_ref().unwrap().reward_statistics.get(&35),
        Some(&3)
    );
    assert_eq!(e.pending_reward_batch()[1].units, 2);
}

#[test]
fn sender_identity_and_client_plugin_route_are_authoritative() {
    for guard in ["sender", "plugin", "rt"] {
        let mut e = fixture();
        let before = e.ground_runtime.clone();
        let wire = [0x02, 0x1d, 0x01, 0, 0, 4];
        let actor = e.active.as_ref().unwrap();
        let mut event = events::EventInput {
            reservation_id: actor.reservation.id.clone(),
            world_id: actor.reservation.world,
            player_id: actor.reservation.player,
            transport: Transport::Tcp,
            direction: Direction::ClientToServer,
            rt_type: 0x3d,
            plugin_id: Some(1),
            payload_hex: protocol::encode_hex(&wire),
        };
        match guard {
            "sender" => event.player_id += 1,
            "plugin" => event.plugin_id = Some(2),
            "rt" => event.rt_type = 0x3c,
            _ => unreachable!(),
        }
        let decoded = DecodedEvent::with_profile(
            &wire,
            actor.reservation.player,
            e.wire_profile,
            e.ground_driver_enabled(),
        );
        let result = e.handle_event(event, &decoded, Duration::from_millis(4000), 3);
        if guard == "sender" {
            assert_eq!(result.err(), Some("identity_mismatch"));
        } else {
            assert!(result.unwrap().is_empty());
        }
        assert_eq!(e.ground_runtime, before);
        assert!(e.next_repair.is_empty());
        assert_eq!(
            e.active.as_ref().unwrap().repair_credit,
            repair_rewards::State::default()
        );
    }
}

#[test]
fn acquisition_profile_tool_and_outer_bridge_deliver_the_recorded_health_update() {
    let mut e = fixture();
    let donor = acquisition_structural::connected_tests::fixture();
    let structural = donor
        .acquisition_gameplay
        .as_ref()
        .unwrap()
        .setup
        .structural
        .clone()
        .unwrap();
    acquisition_gameplay_engine::test_support::install_structural_fixture(
        e.acquisition_gameplay.as_mut().unwrap(),
        structural,
    );
    e.bunker_world = None; // Production Acquisition loads its tool with Structural inputs.
    e.maximum_players = 1;
    e.minimum_players_per_team = [0; 2];
    e.enable_world_ticks();
    let actor = e.active.as_ref().unwrap();
    let request = Request::Event {
        v: BRIDGE_VERSION,
        request_id: "manual-ground-bridge".into(),
        reservation_id: actor.reservation.id.clone(),
        world_id: actor.reservation.world,
        player_id: actor.reservation.player,
        transport: Transport::Tcp,
        direction: Direction::ClientToServer,
        rt_type: 0x3d,
        plugin_id: Some(1),
        payload_hex: "021d01000004".into(),
    };
    let response = e.handle(request, Duration::from_millis(4000), 3);
    assert!(response.ok, "{:?}", response.error);
    assert_eq!(hp(&e), 508);
    assert_eq!(response.rewards.len(), 1);
    assert_eq!(response.messages.len(), 2);
    assert_eq!(response.messages[0].payload_hex, "02040000012301");
    let health = protocol::decode_hex(&response.messages[1].payload_hex);
    let update = replication::ObjectUpdate::decode_with_profile(&health, e.wire_profile).unwrap();
    assert_eq!(update.header.object_type_mask, 1 << 1);
    assert!(update.header.sequence >= 2);
    let mut bits = replication::BitReader::with_profile(&health, e.wire_profile);
    bits.position = e.wire_profile.header_bits();
    assert_eq!(bits.read(1), Ok(1));
    assert_eq!(bits.read(16), Ok(4));
    for _ in 0..18 {
        assert_eq!(bits.read(1), Ok(0));
    }
    assert_eq!(bits.read(1), Ok(1));
    assert_eq!(bits.read(8), Ok(0));
    assert_eq!(bits.read(16), Ok(508));
    assert!(e.active.as_ref().unwrap().pending_shared.is_empty());
}
