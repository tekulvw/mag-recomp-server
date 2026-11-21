//! Connected scripted objective settlement through the Domination recorder.
use super::*;
use progression::RewardKind;
use replication::{MatchPhase, ObjectiveState};
use std::sync::Arc;

#[test]
fn scripted_destruction_and_tag_roll_back_then_settle_once() {
    let mut engine = objective_reward_tests::fixture();
    let catalog = engine.objective_catalog.as_mut().unwrap();
    let rule = catalog.objects[0].reward.as_mut().unwrap();
    rule.category = 3;
    rule.subtype = 3;
    let domination = engine.domination_gameplay.as_mut().unwrap();
    domination.state.objectives = Arc::new(catalog.clone());
    Arc::make_mut(&mut domination.state.root_poses)
        .insert(0, dynamic_collision::Pose::at([1., 2., 1.]));
    domination
        .set_manual_tag(
            0,
            0x0c80_0000,
            engine.objective_world.as_ref().unwrap(),
            engine.spawn_catalog.as_ref().unwrap(),
            MatchPhase::Playing,
        )
        .unwrap();

    let actor = engine.active.as_mut().unwrap();
    actor.life = CharacterLife::AwaitingRespawn { generation: 1 };
    actor
        .reservation
        .character
        .extended_objective_rewards_enabled = Some(true);
    let player = actor.reservation.player;
    let caller = domination_objective_runtime::Actor {
        player,
        generation: 1,
        team: actor.team(),
    };
    engine
        .objective_world
        .as_mut()
        .unwrap()
        .stage_native_state(0, ObjectiveState::Completed, Some(caller))
        .unwrap();
    let now = Duration::from_millis(100);
    let limit = engine.max_log;
    engine.max_log = engine.written;
    assert_eq!(
        engine.tick_domination_objectives(now, 1).unwrap_err(),
        "log_limit_reached"
    );
    assert_eq!(
        engine.objective_world.as_ref().unwrap().state(0),
        Some(ObjectiveState::Deactivated)
    );
    assert!(engine.pending_reward_batch().is_empty());
    assert!(
        !engine
            .active
            .as_ref()
            .unwrap()
            .reward_statistics
            .contains_key(&41)
    );

    engine.max_log = limit;
    engine.failed = false;
    engine.tick_domination_objectives(now, 1).unwrap();
    assert_eq!(
        engine.objective_world.as_ref().unwrap().state(0),
        Some(ObjectiveState::Completed)
    );
    let rewards = engine.pending_reward_batch();
    assert_eq!(rewards.len(), 1);
    assert_eq!(
        (rewards[0].player_id, rewards[0].kind, rewards[0].units),
        (player, RewardKind::DefensiveEmplacementsDestroyed, 1)
    );
    assert_eq!(
        engine.active.as_ref().unwrap().reward_statistics.get(&41),
        Some(&1)
    );

    engine
        .tick_domination_objectives(now + Duration::from_millis(1), 1)
        .unwrap();
    assert_eq!(engine.pending_reward_batch(), rewards);
    assert_eq!(
        engine.active.as_ref().unwrap().reward_statistics.get(&41),
        Some(&1)
    );
}
