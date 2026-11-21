//! Independent synthetic support receipts through the authenticated event path.
use super::*;
use crate::spawn_progression::dispatch;

fn position(actor: &mut Active, p: [f32; 3]) {
    actor.tracking.server_position(
        replication::CharacterPosition {
            x: p[0],
            y: p[1],
            z: p[2],
        },
        player_tracking::PositionOrigin::ServerSpawn,
        0,
    );
}
fn fixture() -> Engine<Vec<u8>> {
    let base =
        spawn_progression::tests::dynamic_bodies::acquisition_gameplay_tests::capture_fixture();
    let mut e = incapacitation_tests::two_actors_from_engine(
        base.with_incapacitation_rules(incapacitation_tests::rules())
            .unwrap(),
        0.4,
    );
    let batch = e.ground_vehicles.as_ref().map_or(0, |w| w.batch_count());
    for a in e.active.iter_mut().chain(e.peers.values_mut()) {
        a.ground_vehicle_batch = batch;
    }
    e.match_started = true;
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
    let healer = e.active.as_mut().unwrap();
    healer.reservation.character.account_id = Some("synthetic-healer".into());
    healer.reservation.character.frago_bonus_enabled = Some(true);
    healer.reservation.character.frago_action_bonus_enabled = Some(true);
    position(healer, [130., 3., 100.]);
    position(
        e.peers.get_mut("synthetic-reservation-0000").unwrap(),
        [104., 3., 103.],
    );
    e
}
fn revive(e: &mut Engine<Vec<u8>>) -> Result<Vec<OutboundMessage>, &'static str> {
    let p = &e.peers["synthetic-reservation-0000"];
    let mut model = p.last_damage;
    model.health_quarter_points = 0;
    model.last_location_and_type = 0x13;
    model.last_attacker = 0x0380_0002; // Unresolved attacker does not invent friendly damage.
    let (model, life) = e.plan_human_damage(p, 40., model, Duration::ZERO).unwrap();
    let p = e.peers.get_mut("synthetic-reservation-0000").unwrap();
    p.last_damage = model;
    p.life = life;
    dispatch(e, &[0x02, 0x1c, 0x03, 0x80, 0x00, 0x00], 1000)
}
fn healing_fixture() -> Engine<Vec<u8>> {
    let mut e = fixture();
    let healer = e.active.take().unwrap();
    let recipient = progression::RewardRecipient::from_active(&healer).unwrap();
    let mut patient = e.peers.remove("synthetic-reservation-0000").unwrap();
    patient.last_damage.health_quarter_points = 160;
    patient.last_damage.last_attacker = u32::MAX;
    patient.life = CharacterLife::Alive {
        generation: 4,
        health_quarter_points: 160,
    };
    patient.healing = Some(progression::HealingSession {
        effect: bot_registry::healing_reward_relationship_tests::effect(),
        healer: Some(recipient),
        effect_id: "synthetic-frago-heal".into(),
    });
    e.active = Some(patient);
    e.peers.insert(healer.reservation.id.clone(), healer);
    e
}
fn tick(e: &mut Engine<Vec<u8>>) -> Result<Vec<OutboundMessage>, &'static str> {
    let wire = replication::ObjectUpdate {
        header: replication::UpdateHeader {
            sequence: e.active.as_ref().unwrap().latest_owner_sequence + 1,
            nak_sequence: 0,
            client_index: 0,
            object_type_mask: 0,
            timestamp_ms: 1000,
        },
        wire: &[],
    }
    .encode_empty_with_profile(e.wire_profile)
    .unwrap();
    dispatch(e, &wire, 1000)
}
fn only_reward(e: &Engine<Vec<u8>>) -> progression::RewardEvent {
    let rewards = e.pending_reward_batch();
    assert_eq!(rewards.len(), 1);
    rewards[0].clone()
}

#[test]
fn revive_patient_context_freezes_frago_on_one_earned_receipt() {
    let mut e = fixture();
    revive(&mut e).unwrap();
    let reward = only_reward(&e);
    assert_eq!(reward.kind, progression::RewardKind::TeammateResuscitated);
    assert_eq!(reward.units, 1);
    assert_eq!(reward.frago, Some(true));
    assert_eq!(
        e.active.as_ref().unwrap().reward_statistics.get(&34),
        Some(&1)
    );
    e.acquisition_gameplay
        .as_mut()
        .unwrap()
        .state
        .squad_mut(0)
        .unwrap()
        .assignment
        .tagged_object = u32::MAX;
    dispatch(&mut e, &[0x02, 0x1c, 0x03, 0x80, 0, 0], 1001).unwrap();
    assert_eq!(only_reward(&e), reward);
}

#[test]
fn heal_uses_current_patient_context_and_actual_positive_whole_hp() {
    let mut e = healing_fixture();
    tick(&mut e).unwrap();
    let reward = only_reward(&e);
    assert_eq!(reward.kind, progression::RewardKind::TeammateHealed);
    assert_eq!(reward.units, 10);
    assert_eq!(reward.frago, Some(true));
    assert_eq!(
        e.active.as_ref().unwrap().last_damage.health_quarter_points,
        200
    );
    tick(&mut e).unwrap();
    assert_eq!(only_reward(&e), reward);
}

#[test]
fn heal_range_uses_the_accepted_patient_pose_in_this_owner_update() {
    for (previous, current, expected) in [(100., 130., None), (130., 100., Some(true))] {
        let mut e = healing_fixture();
        position(e.active.as_mut().unwrap(), [previous, 3., 100.]);
        let fields = player_tracking::ClientCharacterFields {
            position: Some(replication::CharacterPosition {
                x: current,
                y: 3.,
                z: 100.,
            }),
            ..Default::default()
        };
        let wire = replication::multiplayer::CharacterOwnerDelta {
            sequence: 1,
            timestamp_ms: 1000,
            player: 0,
            fields: &fields,
        }
        .encode_with_profile(e.wire_profile)
        .unwrap();
        dispatch(&mut e, &wire, 1000).unwrap();
        assert_eq!(only_reward(&e).frago, expected);
        assert_eq!(
            e.active
                .as_ref()
                .unwrap()
                .tracking
                .sample
                .unwrap()
                .position
                .x,
            current
        );
    }
}

#[test]
fn support_can_qualify_by_current_healer_pose_without_patient_in_range() {
    let mut e = healing_fixture();
    position(e.active.as_mut().unwrap(), [130., 3., 100.]);
    position(
        e.peers.get_mut("synthetic-reservation-0001").unwrap(),
        [100., 3., 100.],
    );
    tick(&mut e).unwrap();
    assert_eq!(only_reward(&e).frago, Some(true));
}

#[test]
fn support_acknowledges_one_immutable_bonus_snapshot_to_the_current_healer() {
    for heal in [false, true] {
        let mut e = if heal { healing_fixture() } else { fixture() };
        if heal {
            tick(&mut e).unwrap();
        } else {
            revive(&mut e).unwrap();
        }
        let reward = only_reward(&e);
        let ack = json!({"op":"progression_committed","v":BRIDGE_VERSION,"request_id":"support-ack",
            "reservation_id":reward.reservation_id,"world_id":reward.world_id,"player_id":reward.player_id,
            "reward_id":reward.reward_id,"progression":{"experience":20,"level":1,"skill_points":0},
            "match_experience":20,"match_frago_points":10});
        for _ in 0..2 {
            let result = e.handle(
                serde_json::from_value(ack.clone()).unwrap(),
                Duration::from_millis(1100),
                3,
            );
            assert!(result.ok, "{:?}", result.error);
        }
        assert!(e.pending_reward_batch().is_empty());
        let healer = e
            .active
            .iter()
            .chain(e.peers.values())
            .find(|a| a.reservation.player == 1)
            .unwrap();
        assert_eq!(healer.reward_statistics.get(&3), Some(&10));
        assert_eq!(healer.match_experience, 20);
        assert_eq!(
            healer.reward_statistics.get(&34),
            if heal { None } else { Some(&1) }
        );
    }
}

#[test]
fn support_requires_its_own_admission_independently_of_kill_admission() {
    for flag in [None, Some(false)] {
        let mut e = fixture();
        e.active
            .as_mut()
            .unwrap()
            .reservation
            .character
            .frago_action_bonus_enabled = flag;
        revive(&mut e).unwrap();
        assert_eq!(only_reward(&e).frago, None);
        let mut e = healing_fixture();
        e.peers
            .get_mut("synthetic-reservation-0001")
            .unwrap()
            .reservation
            .character
            .frago_action_bonus_enabled = flag;
        tick(&mut e).unwrap();
        assert_eq!(only_reward(&e).frago, None);
    }
    let mut e = fixture();
    e.active
        .as_mut()
        .unwrap()
        .reservation
        .character
        .frago_bonus_enabled = None;
    revive(&mut e).unwrap();
    assert_eq!(only_reward(&e).frago, Some(true));
}

#[test]
fn heal_does_not_acquire_context_from_a_disconnected_or_replaced_credit_identity() {
    for changed in 0..7 {
        let mut e = healing_fixture();
        let healer = e.peers.get_mut("synthetic-reservation-0001").unwrap();
        match changed {
            0 => healer.reservation.id = "replacement".into(),
            1 => healer.reservation.world += 1,
            2 => healer.reservation.player += 1,
            3 => healer.reservation.character.account_id = Some("replacement".into()),
            4 => healer.reservation.character.character_id = Some(999),
            5 => healer.reward_match_id = "new-round".into(),
            _ => {
                e.peers.remove("synthetic-reservation-0001");
            }
        }
        tick(&mut e).unwrap();
        let reward = only_reward(&e);
        assert_eq!(reward.frago, None, "identity mutation {changed}");
        assert_eq!(reward.account_id, "synthetic-healer");
        assert_eq!(
            e.active.as_ref().unwrap().last_damage.health_quarter_points,
            200
        );
    }
}

#[test]
fn support_out_of_range_or_missing_geometry_still_commits_base_reward() {
    for missing in [false, true] {
        let mut e = healing_fixture();
        if missing {
            e.objective_catalog.as_mut().unwrap().frago = None;
        } else {
            position(e.active.as_mut().unwrap(), [104., 9., 103.]);
        }
        tick(&mut e).unwrap();
        assert_eq!(only_reward(&e).frago, None);
        assert_eq!(
            e.active.as_ref().unwrap().last_damage.health_quarter_points,
            200
        );
    }
}

#[test]
fn record_failure_leaves_support_hp_stat_and_outbox_uncommitted() {
    let mut e = fixture();
    e.max_log = e.written;
    assert_eq!(revive(&mut e).err(), Some("log_limit_reached"));
    assert!(e.pending_reward_batch().is_empty());
    assert!(
        !e.active
            .as_ref()
            .unwrap()
            .reward_statistics
            .contains_key(&34)
    );
    assert!(matches!(
        e.peers["synthetic-reservation-0000"].life,
        CharacterLife::Incapacitated { .. }
    ));
}
