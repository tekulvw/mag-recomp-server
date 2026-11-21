//! Synthetic connected native objective rewards; no game files or accounts.
use super::*;
use crate::objective_catalog::{NativeClass, ObjectiveRewardRule};
use crate::progression::RewardKind as Kind;

pub(crate) fn fixture() -> Engine<Vec<u8>> {
    let mut e = domination_gameplay_engine::tests::structural_repair::repair_fixture();
    e.ground_runtime = Some(Default::default());
    std::sync::Arc::make_mut(
        &mut e
            .domination_gameplay
            .as_mut()
            .unwrap()
            .state
            .known_empty_vehicle_spawns,
    )
    .insert(2);
    e.domination_gameplay
        .as_mut()
        .unwrap()
        .state
        .ground_respawn_physics = Some(Default::default());
    let mut catalog = e.objective_catalog.clone().unwrap();
    for object in &mut catalog.objects {
        object.initial_state = replication::ObjectiveState::Deactivated as u8;
        object.reward = Some(ObjectiveRewardRule {
            category: if object.native_class == Some(NativeClass::Structural) {
                3
            } else {
                0
            },
            subtype: 0,
            demolition: object.native_class == Some(NativeClass::Structural),
        });
    }
    e.objective_world = Some(objective_control::ObjectiveWorld::new(&catalog));
    e.objective_catalog = Some(catalog.clone());
    e.domination_gameplay.as_mut().unwrap().state.objectives = std::sync::Arc::new(catalog);
    let spawns = e.spawn_catalog.as_ref().unwrap();
    let actor = e.active.as_mut().unwrap();
    actor.squad_catalog = std::sync::Arc::new(spawns.for_squad(0).unwrap());
    actor.claimed = Some(Claimed {
        wire: vec![],
        np_identity: [0; 36],
        game_info: protocol::GameInfo {
            client_index: actor.reservation.player,
            level_name: "synthetic".into(),
            start_time_ms: 0,
            attacking_faction: 1,
            defending_faction: 0,
            maximum_points: [1200, 1000],
            current_points: [1200, 1000],
            game_type: 2,
        },
    });
    actor.reservation.character.progression = Some(progression::ProgressionSnapshot::default());
    actor.reservation.character.frago_objective_bonus_enabled = Some(true);
    actor.tracking.server_position(
        replication::CharacterPosition {
            x: 4.,
            y: 2.,
            z: 1.,
        },
        player_tracking::PositionOrigin::ServerSpawn,
        0,
    );
    let state = &mut e.domination_gameplay.as_mut().unwrap().state;
    state.assignments[0] = state.assignments[16];
    let assignment = state.assignments[0].as_mut().unwrap();
    assignment.assignment.primary_index = 1;
    assignment.assignment.tagged_object = u32::MAX;
    e
}
pub(crate) fn interact(
    e: &mut Engine<Vec<u8>>,
    index: u16,
    kind: protocol::ObjectiveInteractionKind,
    ms: u64,
) -> Result<Vec<OutboundMessage>, &'static str> {
    let wire = protocol::ObjectiveInteraction {
        target_object_id: 0x0c80_0000 | u32::from(index),
        kind,
        actor_index: e.active.as_ref().unwrap().reservation.player,
        trigger_index: u8::from(index == 120),
    }
    .encode();
    let a = e.active.as_ref().unwrap();
    let input = events::EventInput {
        reservation_id: a.reservation.id.clone(),
        world_id: a.reservation.world,
        player_id: a.reservation.player,
        transport: Transport::Tcp,
        direction: Direction::ClientToServer,
        rt_type: 0x3d,
        plugin_id: Some(1),
        payload_hex: protocol::encode_hex(&wire),
    };
    let decoded = DecodedEvent::with_profile(&wire, input.player_id, e.wire_profile, false);
    e.handle_event(input, &decoded, Duration::from_millis(ms), 1)
}
fn tick(e: &mut Engine<Vec<u8>>, ms: u64) -> Result<Vec<OutboundMessage>, &'static str> {
    e.tick_domination_objectives(Duration::from_millis(ms), 1)
}
#[test]
fn objective_capability_and_character_admission_are_independent() {
    let mut e = fixture();
    assert_eq!(
        serde_json::to_value(&e.active.as_ref().unwrap().reservation.character).unwrap()["frago_objective_bonus_enabled"],
        json!(true)
    );
    let response = e.handle(
        Request::Hello {
            v: BRIDGE_VERSION,
            request_id: "objective-hello".into(),
        },
        Duration::ZERO,
        1,
    );
    assert!(response.ok, "{:?}", response.error);
    assert!(
        response
            .capabilities
            .iter()
            .any(|c| c == "frago_objective_bonus_v1")
    );
}
#[test]
fn native_capture_awards_activation_and_completion_with_frozen_object_context() {
    let mut e = fixture();
    interact(&mut e, 1, protocol::ObjectiveInteractionKind::Activate, 100).unwrap();
    e.domination_gameplay
        .as_mut()
        .unwrap()
        .set_manual_tag(
            0,
            0x0c80_0001,
            e.objective_world.as_ref().unwrap(),
            e.spawn_catalog.as_ref().unwrap(),
            replication::MatchPhase::Playing,
        )
        .unwrap();
    tick(&mut e, 137).unwrap();
    let first = e.pending_reward_batch();
    assert_eq!(first.len(), 1);
    assert_eq!(
        (first[0].kind, first[0].units, first[0].frago),
        (Kind::PrimaryObjectiveActivated, 1, Some(true))
    );
    e.active.as_mut().unwrap().tracking.server_position(
        replication::CharacterPosition {
            x: 100.,
            y: 2.,
            z: 1.,
        },
        player_tracking::PositionOrigin::ServerSpawn,
        138,
    );
    tick(&mut e, 180).unwrap();
    let awards = e.pending_reward_batch();
    assert_eq!(awards.len(), 2);
    assert_eq!(
        (awards[1].kind, awards[1].frago),
        (Kind::PrimaryObjectiveSecured, Some(true))
    );
    tick(&mut e, 181).unwrap();
    assert_eq!(e.pending_reward_batch(), awards);
}
#[test]
fn native_capture_reward_and_state_roll_back_on_record_failure() {
    let mut e = fixture();
    interact(&mut e, 1, protocol::ObjectiveInteractionKind::Activate, 100).unwrap();
    let cap = e.max_log;
    e.max_log = e.written;
    assert_eq!(tick(&mut e, 137).unwrap_err(), "log_limit_reached");
    assert!(e.pending_reward_batch().is_empty());
    assert_eq!(
        e.objective_world.as_ref().unwrap().state(1),
        Some(replication::ObjectiveState::Activating)
    );
    e.max_log = cap;
    e.failed = false;
    tick(&mut e, 137).unwrap();
    assert_eq!(e.pending_reward_batch().len(), 1);
}
#[test]
fn native_round_awards_ordered_squad_completion_before_match_result_once() {
    let mut e = fixture();
    // Defender end conditions include Deactivated; both ordered catalog rows qualify.
    let spawns = e.spawn_catalog.as_ref().unwrap();
    e.active.as_mut().unwrap().squad_catalog = std::sync::Arc::new(spawns.for_squad(16).unwrap());
    e.match_epoch = Some(Duration::from_millis(1));
    e.domination_round
        .as_mut()
        .unwrap()
        .apply_score(acquisition_round::ScoreIntent {
            team: 0,
            delta: -32767,
        })
        .unwrap();
    e.tick_owned_round(
        acquisition_match_flow::OwnedRound::Domination,
        Duration::from_millis(200),
        1,
    )
    .unwrap();
    let awards = e.pending_reward_batch();
    assert_eq!(
        awards.iter().map(|a| a.kind).collect::<Vec<_>>(),
        [
            Kind::SecondaryObjectiveCompleted,
            Kind::PrimaryObjectiveCompleted,
            Kind::MatchMajorVictory
        ]
    );
    assert!(awards[..2].iter().all(|a| a.frago == Some(true)));
    e.tick_owned_round(
        acquisition_match_flow::OwnedRound::Domination,
        Duration::from_millis(201),
        1,
    )
    .unwrap();
    assert_eq!(e.pending_reward_batch(), awards);
}

#[test]
fn native_charge_credits_are_tier_independent_and_cancel_does_not_replant() {
    let mut e = acquisition_fixture();
    e.active.as_mut().unwrap().tracking.server_position(
        replication::CharacterPosition {
            x: 20.,
            y: 3.,
            z: 10.,
        },
        player_tracking::PositionOrigin::ServerSpawn,
        0,
    );
    interact(
        &mut e,
        120,
        protocol::ObjectiveInteractionKind::Activate,
        100,
    )
    .unwrap();
    acquisition_tick(&mut e, 110).unwrap();
    assert_eq!(
        e.pending_reward_batch()
            .iter()
            .map(|r| r.kind)
            .collect::<Vec<_>>(),
        [Kind::ChargePlanted]
    );
    // Successful defuse credits the defender, not the planting side.
    e.active.as_mut().unwrap().squad_catalog =
        std::sync::Arc::new(e.spawn_catalog.as_ref().unwrap().for_squad(16).unwrap());
    interact(
        &mut e,
        120,
        protocol::ObjectiveInteractionKind::Deactivate,
        111,
    )
    .unwrap();
    interact(
        &mut e,
        120,
        protocol::ObjectiveInteractionKind::Release,
        112,
    )
    .unwrap();
    assert_eq!(e.pending_reward_batch().len(), 1);
    interact(
        &mut e,
        120,
        protocol::ObjectiveInteractionKind::Deactivate,
        113,
    )
    .unwrap();
    acquisition_tick(&mut e, 123).unwrap();
    let rewards = e.pending_reward_batch();
    assert_eq!(
        rewards.iter().map(|r| r.kind).collect::<Vec<_>>(),
        [Kind::ChargePlanted, Kind::ChargeDefused]
    );
    assert_eq!(
        e.objective_world.as_ref().unwrap().state(120),
        Some(replication::ObjectiveState::Deactivated)
    );
}

#[test]
fn native_reward_outbox_failure_preserves_the_retryable_transition() {
    let mut e = fixture();
    interact(&mut e, 1, protocol::ObjectiveInteractionKind::Activate, 100).unwrap();
    let recipient = progression::RewardRecipient::from_active(e.active.as_ref().unwrap()).unwrap();
    e.commit_rewards(
        (0..progression::MAX_PENDING_REWARDS)
            .map(|i| recipient.event(format!("filler-{i}"), Kind::EnemyFinished, 1))
            .collect(),
    );
    assert_eq!(tick(&mut e, 137).unwrap_err(), "reward_outbox_full");
    assert_eq!(
        e.objective_world.as_ref().unwrap().state(1),
        Some(replication::ObjectiveState::Activating)
    );
    assert_eq!(e.pending_rewards.len(), progression::MAX_PENDING_REWARDS);
}

#[test]
fn legacy_capabilities_keep_native_objective_base_rewards_only() {
    let mut e = fixture();
    let actor = e.active.as_mut().unwrap();
    actor.reservation.character.frago_objective_bonus_enabled = None;
    actor.reservation.character.frago_bonus_enabled = Some(true);
    actor.reservation.character.frago_action_bonus_enabled = Some(true);
    interact(&mut e, 1, protocol::ObjectiveInteractionKind::Activate, 100).unwrap();
    tick(&mut e, 137).unwrap();
    assert_eq!(e.pending_reward_batch().len(), 1);
    assert_eq!(e.pending_reward_batch()[0].frago, None);
}

pub(crate) fn acquisition_fixture() -> Engine<Vec<u8>> {
    let mut e = acquisition_structural::connected_tests::fixture();
    let mut catalog = e.objective_catalog.clone().unwrap();
    for object in &mut catalog.objects {
        object.initial_state = e
            .objective_world
            .as_ref()
            .unwrap()
            .state(object.object_index)
            .unwrap() as u8;
        if object.object_index == 120 {
            object.reward = Some(ObjectiveRewardRule {
                category: 3,
                subtype: 3,
                demolition: true,
            });
        }
    }
    e.objective_world = Some(objective_control::ObjectiveWorld::new(&catalog));
    e.objective_catalog = Some(catalog);
    frago_tests::configure_geometry(&mut e);
    let actor = e.active.as_mut().unwrap();
    actor.claimed = Some(Claimed {
        wire: vec![],
        np_identity: [0; 36],
        game_info: protocol::GameInfo {
            client_index: actor.reservation.player,
            level_name: "synthetic_arena".into(),
            start_time_ms: 0,
            attacking_faction: 1,
            defending_faction: 0,
            maximum_points: [1200, 1000],
            current_points: [1200, 1000],
            game_type: 1,
        },
    });
    actor.reservation.character.account_id = Some("synthetic-planter".into());
    actor.reservation.character.progression = Some(Default::default());
    actor.reservation.character.frago_objective_bonus_enabled = Some(true);
    let squad = actor.squad_catalog.squad_index();
    e.acquisition_gameplay
        .as_mut()
        .unwrap()
        .state
        .squad_mut(squad)
        .unwrap()
        .assignment = acquisition_objective_runtime::Assignment {
        primary_index: 120,
        tagged_object: u32::MAX,
    };
    e
}
fn acquisition_tick(
    e: &mut Engine<Vec<u8>>,
    ms: u64,
) -> Result<Vec<OutboundMessage>, &'static str> {
    e.tick_acquisition_objectives(Duration::from_millis(ms), 1)
}

#[test]
fn acquisition_native_charge_uses_recorded_base_and_context_rewards_without_a_legacy_queue() {
    let mut e = acquisition_fixture();
    let request = protocol::ObjectiveInteraction {
        target_object_id: 0x0c80_0078,
        kind: protocol::ObjectiveInteractionKind::Activate,
        actor_index: e.active.as_ref().unwrap().reservation.player,
        trigger_index: 1,
    };
    let wire = request.encode();
    let a = e.active.as_ref().unwrap();
    let input = events::EventInput {
        reservation_id: a.reservation.id.clone(),
        world_id: a.reservation.world,
        player_id: a.reservation.player,
        transport: Transport::Udp,
        direction: Direction::ClientToServer,
        rt_type: 0x3d,
        plugin_id: Some(1),
        payload_hex: protocol::encode_hex(&wire),
    };
    let decoded = DecodedEvent::with_profile(&wire, input.player_id, e.wire_profile, false);
    e.handle_event(input, &decoded, Duration::from_millis(100), 1)
        .unwrap();
    e.tick_acquisition_objectives(Duration::from_millis(110), 1)
        .unwrap();
    let awards = e.pending_reward_batch();
    assert_eq!(awards.len(), 1);
    assert_eq!(
        (awards[0].kind, awards[0].frago),
        (Kind::ChargePlanted, Some(true))
    );
    assert!(
        e.objective_world
            .as_mut()
            .unwrap()
            .take_reward_actions()
            .is_empty()
    );
}

#[test]
fn rebuilt_structure_defuse_returns_to_repaired_and_earns_defuse_xp() {
    let mut e = fixture();
    let mut catalog = e.objective_catalog.clone().unwrap();
    catalog.objects[0].initial_state = replication::ObjectiveState::Repaired as u8;
    e.objective_world = Some(objective_control::ObjectiveWorld::new(&catalog));
    e.active.as_mut().unwrap().tracking.server_position(
        replication::CharacterPosition {
            x: 1.,
            y: 2.,
            z: 1.,
        },
        player_tracking::PositionOrigin::ServerSpawn,
        0,
    );
    interact(&mut e, 0, protocol::ObjectiveInteractionKind::Activate, 100).unwrap();
    tick(&mut e, 110).unwrap();
    e.active.as_mut().unwrap().squad_catalog =
        std::sync::Arc::new(e.spawn_catalog.as_ref().unwrap().for_squad(16).unwrap());
    interact(
        &mut e,
        0,
        protocol::ObjectiveInteractionKind::Deactivate,
        111,
    )
    .unwrap();
    tick(&mut e, 121).unwrap();
    assert_eq!(
        e.objective_world.as_ref().unwrap().state(0),
        Some(replication::ObjectiveState::Repaired)
    );
    assert_eq!(
        e.pending_reward_batch()
            .iter()
            .map(|r| r.kind)
            .collect::<Vec<_>>(),
        [Kind::ChargePlanted, Kind::ChargeDefused]
    );
}

#[test]
fn planted_fuse_can_credit_its_current_dead_character_at_completion() {
    let mut e = acquisition_fixture();
    let mut catalog = e.objective_catalog.clone().unwrap();
    catalog
        .objects
        .iter_mut()
        .find(|o| o.object_index == 120)
        .unwrap()
        .reward
        .as_mut()
        .unwrap()
        .category = 0;
    e.objective_catalog = Some(catalog);
    e.active.as_mut().unwrap().tracking.server_position(
        replication::CharacterPosition {
            x: 20.,
            y: 3.,
            z: 10.,
        },
        player_tracking::PositionOrigin::ServerSpawn,
        0,
    );
    interact(
        &mut e,
        120,
        protocol::ObjectiveInteractionKind::Activate,
        100,
    )
    .unwrap();
    acquisition_tick(&mut e, 110).unwrap();
    e.active.as_mut().unwrap().life = CharacterLife::AwaitingRespawn { generation: 1 };
    acquisition_tick(&mut e, 140).unwrap();
    assert_eq!(
        e.pending_reward_batch()
            .iter()
            .map(|r| r.kind)
            .collect::<Vec<_>>(),
        [
            Kind::ChargePlanted,
            Kind::PrimaryObjectiveActivated,
            Kind::PrimaryObjectiveSecured
        ]
    );
}

#[test]
fn terminal_objective_reserve_keeps_repeated_rows_atomic_above_ordinary_capacity() {
    let mut e = fixture();
    let hash = "1".repeat(64);
    let catalog=objective_catalog::ObjectiveCatalog::from_json(&json!({
        "schema_version":1,"format":"mag_retail_objective_catalog","application_id":20231,
        "protocol":1740,"game_type":2,"map_name":"synthetic","entity_family":12,"network_type":12,
        "source":{"authored_catalog_sha256":hash,"binding_capture_sha256":hash,
            "startup_proof_sha256":hash,"source_image_sha256":hash,"capture_qualification":"native_source_gameplay"},
        "objects":[{"token":"0x0000000b","object_index":1,"initial_state":1,"native_class":"structural",
            "reward":{"category":0,"subtype":0,"demolition":true}}],
        "squads":(0..32).map(|s|json!({"team":s/16,"squad_index":s,"initial_objective_index":1,
            "authored_objectives":(0..128).map(|i|json!({"object_index":1,"order":i})).collect::<Vec<_>>()})).collect::<Vec<_>>()
    }).to_string()).unwrap();
    e.objective_world = Some(objective_control::ObjectiveWorld::new(&catalog));
    e.objective_catalog = Some(catalog);
    let squad = std::sync::Arc::new(e.spawn_catalog.as_ref().unwrap().for_squad(16).unwrap());
    e.active.as_mut().unwrap().squad_catalog = squad.clone();
    for id in 1..3 {
        let mut a = fixture().active.take().unwrap();
        a.squad_catalog = squad.clone();
        a.reservation.id = format!("synthetic-terminal-{id}");
        a.reservation.player = id;
        a.reservation.character.account_id = Some(format!("synthetic-terminal-{id}"));
        a.reservation.character.character_id = Some(u32::from(id) + 1);
        a.reward_match_id = e.active.as_ref().unwrap().reward_match_id.clone();
        e.peers.insert(a.reservation.id.clone(), a);
    }
    e.match_epoch = Some(Duration::from_millis(1));
    e.domination_round
        .as_mut()
        .unwrap()
        .apply_score(acquisition_round::ScoreIntent {
            team: 0,
            delta: -32767,
        })
        .unwrap();
    let max = e.max_log;
    e.max_log = e.written;
    assert_eq!(
        e.tick_owned_round(
            acquisition_match_flow::OwnedRound::Domination,
            Duration::from_millis(200),
            1
        )
        .unwrap_err(),
        "log_limit_reached"
    );
    assert!(e.round.outcome.is_none());
    assert!(e.pending_rewards.is_empty());
    e.max_log = max;
    e.failed = false;
    e.tick_owned_round(
        acquisition_match_flow::OwnedRound::Domination,
        Duration::from_millis(200),
        1,
    )
    .unwrap();
    assert_eq!(e.pending_rewards.len(), 3 * 129);
    assert_eq!(
        e.pending_reward_batch().len(),
        progression::MAX_REWARD_BATCH
    );
    let ordinary = progression::RewardRecipient::from_active(e.active.as_ref().unwrap())
        .unwrap()
        .event("late-combat".into(), Kind::EnemyFinished, 1);
    assert_eq!(e.check_reward_capacity(&[ordinary]), Ok(()));
    let count = e.pending_rewards.len();
    e.tick_owned_round(
        acquisition_match_flow::OwnedRound::Domination,
        Duration::from_millis(201),
        1,
    )
    .unwrap();
    assert_eq!(e.pending_rewards.len(), count);
}

#[test]
fn acquisition_capture_drains_both_awards_when_one_tick_crosses_both_deadlines() {
    let mut e =
        spawn_progression::tests::dynamic_bodies::acquisition_gameplay_tests::capture_fixture();
    spawn_progression::reserve(&mut e, 0);
    let actor = e.active.as_mut().unwrap();
    spawn_progression::ready(actor);
    actor.life = CharacterLife::Alive {
        generation: 1,
        health_quarter_points: 400,
    };
    actor.reservation.character.account_id = Some("synthetic-acq-capturer".into());
    actor.reservation.character.progression = Some(Default::default());
    actor.reservation.character.frago_objective_bonus_enabled = Some(true);
    e.objective_catalog
        .as_mut()
        .unwrap()
        .objects
        .iter_mut()
        .find(|o| o.object_index == 100)
        .unwrap()
        .reward = Some(ObjectiveRewardRule {
        category: 1,
        subtype: 12,
        demolition: false,
    });
    e.objective_world = Some(objective_control::ObjectiveWorld::new(
        e.objective_catalog.as_ref().unwrap(),
    ));
    frago_tests::configure_geometry(&mut e);
    e.match_started = true;
    e.match_epoch = Some(Duration::ZERO);
    e.tick_acquisition_objectives(Duration::ZERO, 1).unwrap();
    interact(
        &mut e,
        100,
        protocol::ObjectiveInteractionKind::Activate,
        100,
    )
    .unwrap();
    let max = e.max_log;
    e.max_log = e.written;
    assert_eq!(
        e.tick_acquisition_objectives(Duration::from_millis(3100), 1)
            .unwrap_err(),
        "log_limit_reached"
    );
    assert!(e.pending_rewards.is_empty());
    e.max_log = max;
    e.failed = false;
    e.tick_acquisition_objectives(Duration::from_millis(3100), 1)
        .unwrap();
    let rewards = e.pending_reward_batch();
    assert_eq!(
        rewards.iter().map(|r| r.kind).collect::<Vec<_>>(),
        [
            Kind::PrimaryObjectiveActivated,
            Kind::PrimaryObjectiveSecured
        ]
    );
    assert!(rewards.iter().all(|r| r.frago == Some(true)));
    assert!(
        e.objective_world
            .as_mut()
            .unwrap()
            .take_reward_actions()
            .is_empty()
    );
    e.tick_acquisition_objectives(Duration::from_millis(3101), 1)
        .unwrap();
    assert_eq!(e.pending_reward_batch(), rewards);
}

fn activated_entry(e: &Engine<Vec<u8>>) -> objective_control::NativeEntry {
    let mut delta = e
        .objective_world
        .as_ref()
        .unwrap()
        .snapshot(1, 0)
        .objectives
        .into_iter()
        .find(|d| d.object_index == 120)
        .unwrap();
    delta.state_actor = replication::ObjectiveStateActor {
        state: replication::ObjectiveState::Activated,
        actor_index: e.active.as_ref().unwrap().reservation.player,
        change_time_ms: u32::MAX,
    };
    objective_control::NativeEntry {
        class: NativeClass::Structural,
        previous: replication::ObjectiveState::Activating,
        delta,
        actor_generation: Some(999),
        reward_actor: Some(delta.state_actor.actor_index),
        active_trigger_index: Some(1),
    }
}
#[test]
fn native_callback_requires_an_allocated_character_and_keeps_long_receipt_ids_bounded() {
    let mut e = acquisition_fixture();
    let entry = activated_entry(&e);
    let claim = e.active.as_mut().unwrap().claimed.take();
    e.stage_native_objective_rewards(entry, None, frago::Frame::default())
        .unwrap();
    assert!(e.pending_rewards.is_empty());
    let a = e.active.as_mut().unwrap();
    a.claimed = claim;
    a.entered_game = false;
    e.stage_native_objective_rewards(entry, None, frago::Frame::default())
        .unwrap();
    assert!(e.pending_rewards.is_empty());
    let a = e.active.as_mut().unwrap();
    a.entered_game = true;
    a.reservation.id = "r".repeat(64);
    a.life = CharacterLife::AwaitingRespawn { generation: 2 };
    e.stage_native_objective_rewards(entry, None, frago::Frame::default())
        .unwrap();
    let awards = e.pending_reward_batch();
    assert_eq!(awards.len(), 1);
    assert!(awards[0].reward_id.len() <= 128);
    assert_eq!(awards[0].reservation_id, "r".repeat(64));
}

#[test]
fn detached_domination_uses_candidate_helo_tag_position_for_objective_context() {
    let mut e = fixture();
    let source:domination_helo::FleetSource=serde_json::from_value(json!({
        "schema":"synthetic","map":"synthetic","spawn_catalog_sha256":"1".repeat(64),
        "source_receipt_sha256":vec!["1".repeat(64);3],
        "carriers":(0..4).map(|i|json!({"object_index":i,"spawn_index":i,"spawn_token":i+1,
            "vehicle_type_token":1,"health_quarter_points":400,"faction":0,"passenger_start":1,
            "squads":[0,1],"start_delay_max_seconds":0.,"acceleration_seconds":1.,"braking_seconds":1.,
            "route":{"inbound_flight_seconds":20.,"outbound_flight_seconds":15.,"hover_position_delay_seconds":0.5,
                "drop_point_index":1,"knots":[
                    {"position_xyz":[20.,10.,20.],"normalized_parameter":0.,"tangent_xyz":[0.,0.,10.]},
                    {"position_xyz":[20.,10.,30.],"normalized_parameter":0.3,"tangent_xyz":[0.,0.,10.]},
                    {"position_xyz":[20.,10.,40.],"normalized_parameter":1.,"tangent_xyz":[0.,0.,10.]}]}})).collect::<Vec<_>>()
    })).unwrap();
    let mut fleet = domination_helo::FleetRuntime::from_source(source, 1).unwrap();
    fleet.carriers[0].current_pose = Some(domination_helo::FlightPose {
        position: replication::CharacterPosition {
            x: 2.,
            y: 2.,
            z: 1.,
        },
        forward: [0., 0., 1.],
    });
    e.domination_gameplay.take();
    e.active.as_mut().unwrap().tracking.server_position(
        replication::CharacterPosition {
            x: 100.,
            y: 2.,
            z: 1.,
        },
        player_tracking::PositionOrigin::ServerSpawn,
        0,
    );
    let a = e.active.as_ref().unwrap();
    let reward = progression::RewardRecipient::from_active(a).unwrap().event(
        "helo-context".into(),
        Kind::PrimaryObjectiveSecured,
        1,
    );
    let assignment = Some(acquisition_objective_runtime::Assignment {
        primary_index: u16::MAX,
        tagged_object: 0x0280_0000,
    });
    assert_eq!(
        e.frago_objective_reward(
            reward.clone(),
            a,
            Some([2., 2., 1.]),
            assignment,
            frago::Frame::default()
        )
        .frago,
        None
    );
    assert_eq!(
        e.frago_objective_reward(
            reward,
            a,
            Some([2., 2., 1.]),
            assignment,
            frago::Frame {
                helo: Some(&fleet),
                ..Default::default()
            }
        )
        .frago,
        Some(true)
    );
}

#[test]
fn acquisition_capture_uses_staged_ground_tag_at_its_callback() {
    let mut e = acquisition_fixture();
    let mut runtime = e.acquisition_gameplay.clone().unwrap();
    let actor = e.active.as_ref().unwrap();
    let recipient = progression::RewardRecipient::from_active(actor).unwrap();
    runtime
        .state
        .squad_mut(actor.squad_catalog.squad_index())
        .unwrap()
        .assignment = acquisition_objective_runtime::Assignment {
        primary_index: u16::MAX,
        tagged_object: 0x0100_0004,
    };
    let key = e.ground_runtime.as_ref().unwrap().get(4).unwrap().key;
    let mut ground = e.ground_runtime.clone().unwrap();
    ground
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
    let action = objective_control::ObjectiveRewardAction {
        object_index: 100,
        started_at: Duration::from_millis(100),
        recipient,
        kind: Kind::PrimaryObjectiveActivated,
    };
    e.stage_acquisition_capture_rewards(vec![action], &runtime, &ground)
        .unwrap();
    assert_eq!(e.pending_reward_batch()[0].frago, Some(true));
}

#[test]
fn precision_cascade_does_not_invent_plant_xp_from_its_wire_actor() {
    let mut e = acquisition_fixture();
    let source = structure_damage::Source {
        player: e.active.as_ref().unwrap().reservation.player,
        generation: 1,
    };
    assert!(
        e.objective_world
            .as_mut()
            .unwrap()
            .stage_precision_strike(120, Some(source))
            .unwrap()
    );
    e.tick_acquisition_objectives(Duration::from_millis(100), 1)
        .unwrap();
    assert_eq!(
        e.objective_world.as_ref().unwrap().state(120),
        Some(replication::ObjectiveState::Completed)
    );
    assert!(
        e.pending_reward_batch().is_empty(),
        "Precision's intermediate Activated is not an OnActivated callback; its final latch is not yet qualified"
    );
}
