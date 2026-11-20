//! Integration through the command event and detached world transaction.
use super::*;
use crate::{
    active_abilities::Ability,
    active_ability_engine_tests::{
        activate, actor_mut, add_ready, configured, prepare_effect_tick, request, strike_floor,
    },
    objective_control::{ObjectiveWorld, trigger_box_tests::native_catalog},
    replication::{CharacterPosition, ObjectiveState},
};
use std::{collections::BTreeMap, sync::Arc};

const STRUCTURAL: u16 = 11;
const ARMING: ObjectiveState = ObjectiveState::Activating;

fn armed_world() -> Engine<Vec<u8>> {
    let mut engine = configured()
        .with_dynamic_collision_bodies(
            dynamic_collision::Catalog::from_json(
                &crate::synthetic_dynamic_collision::fixture().to_string(),
            )
            .unwrap(),
            dynamic_collision_runtime::RuntimeBindings {
                character_template: "invented-character".into(),
                vehicles: Vec::new(),
            },
        )
        .unwrap();
    let mut catalog = native_catalog();
    catalog["map_name"] = json!(engine.map_name());
    let catalog = objective_catalog::ObjectiveCatalog::from_json(&catalog.to_string()).unwrap();
    let mut objectives = ObjectiveWorld::new(&catalog);
    objectives
        .stage_native_state(STRUCTURAL, ObjectiveState::Deactivated, None)
        .unwrap();
    let now = Duration::from_millis(1);
    objectives.begin_native_tick(now);
    objectives.tick_native_object(STRUCTURAL, now, &BTreeMap::new());
    objectives.take_native_entries();
    assert_eq!(
        objectives.state(STRUCTURAL),
        Some(ObjectiveState::Deactivated)
    );
    engine.objective_world = Some(objectives);
    engine.collision_world = Some(strike_floor(engine.map_name(), true));

    let mut setup = (*engine.active_abilities.as_ref().unwrap().setup).clone();
    setup.damage_triggers = vec![
        serde_json::from_value(json!({
            "object_index":STRUCTURAL,"trigger_index":0,"trigger_type":4,
            "start_active":true,"damage_threshold":25.,"position":[100.,3.,100.],
            "box":{"centre":[100.,3.,100.],
                "axes":[[1.,0.,0.],[0.,1.,0.],[0.,0.,1.]],"half_extents":[1.,1.,1.]}
        }))
        .unwrap(),
    ];
    engine.active_abilities = Some(Arc::new(setup).runtime().unwrap());
    add_ready(&mut engine, 0);
    actor_mut(&mut engine, 0).tracking.server_position(
        CharacterPosition {
            x: 160.,
            y: 3.,
            z: 160.,
        },
        player_tracking::PositionOrigin::ServerSpawn,
        1,
    );
    engine.match_started = true;
    engine.reconcile_squad_leaders().unwrap();
    let mut command = request(16, Ability::ClusterBomb);
    command.position = [100., 1., 100.];
    let response = activate(&mut engine, command, 0, 20);
    assert!(response.ok, "{:?}", response.error);
    engine
}

fn strike(
    engine: &mut Engine<Vec<u8>>,
    millis: u64,
    outcome: &str,
    progressed: bool,
) -> acquisition_character_effects::Candidate {
    let candidate = prepare_effect_tick(engine, &BTreeMap::new(), millis).unwrap();
    let damage = candidate
        .audit()
        .iter()
        .find(|entry| entry["op"] == "structure_strike_damage")
        .unwrap();
    assert_eq!(damage["objective_index"], STRUCTURAL);
    assert_eq!(damage["raw_damage"], 17.);
    assert_eq!(damage["outcome"], outcome);
    assert_eq!(damage["owner_progressed"], progressed);
    candidate
}

fn state(engine: &Engine<Vec<u8>>) -> ObjectiveState {
    engine
        .objective_world
        .as_ref()
        .unwrap()
        .state(STRUCTURAL)
        .unwrap()
}

#[test]
fn timed_cluster_crosses_the_threshold_then_ordinary_objective_ticks_arm_and_complete() {
    let mut engine = armed_world();
    let first = strike(&mut engine, 47, "Accumulated", false);
    engine
        .record_acquisition_character_effects(first, &mut BTreeMap::new(), 1)
        .unwrap();
    assert_eq!(state(&engine), ObjectiveState::Deactivated);

    let second = strike(&mut engine, 55, "Destroyed", true);
    assert_eq!(state(&engine), ObjectiveState::Deactivated);
    engine
        .record_acquisition_character_effects(second, &mut BTreeMap::new(), 1)
        .unwrap();
    assert_eq!(state(&engine), ARMING);
    let third = prepare_effect_tick(&mut engine, &BTreeMap::new(), 63).unwrap();
    assert!(
        third
            .audit()
            .iter()
            .all(|entry| entry["op"] != "structure_strike_damage")
    );
    engine
        .record_acquisition_character_effects(third, &mut BTreeMap::new(), 1)
        .unwrap();

    let world = engine.objective_world.as_mut().unwrap();
    for (millis, expected) in [
        (64, ARMING),
        (65, ObjectiveState::Activated),
        (94, ObjectiveState::Activated),
        (95, ObjectiveState::Completed),
    ] {
        let now = Duration::from_millis(millis);
        world.begin_native_tick(now);
        world.tick_native_object(STRUCTURAL, now, &BTreeMap::new());
        assert_eq!(world.state(STRUCTURAL), Some(expected), "at {millis}ms");
    }
    assert!(world.take_reward_actions().is_empty());
}

#[test]
fn failed_threshold_query_and_record_keep_accumulated_health_and_pending_strike_slot() {
    let mut engine = armed_world();
    let first = strike(&mut engine, 47, "Accumulated", false);
    engine
        .record_acquisition_character_effects(first, &mut BTreeMap::new(), 1)
        .unwrap();
    engine.collision_world = Some(strike_floor(engine.map_name(), false));
    assert!(matches!(
        prepare_effect_tick(&mut engine, &BTreeMap::new(), 55),
        Err("air_strike_static_impact_ray_failed")
    ));
    assert_eq!(state(&engine), ObjectiveState::Deactivated);
    engine.collision_world = Some(strike_floor(engine.map_name(), true));
    let second = strike(&mut engine, 55, "Destroyed", true);
    let written = engine.writer.len();
    engine.max_log = engine.written;
    assert_eq!(
        engine
            .record_acquisition_character_effects(second, &mut BTreeMap::new(), 1)
            .unwrap_err(),
        "log_limit_reached"
    );
    assert_eq!(engine.writer.len(), written);
    assert_eq!(state(&engine), ObjectiveState::Deactivated);
    // A read-only preparation after recorder failure can still inspect the
    // restored phase and damage ledger. Re-enable the synthetic recorder only
    // because world snapshot construction requires an available Engine.
    engine.failed = false;
    let _retry = strike(&mut engine, 55, "Destroyed", true);
    assert_eq!(state(&engine), ObjectiveState::Deactivated);
}

#[test]
fn replaced_invoker_can_destroy_trigger_health_but_cannot_arm_its_owner() {
    let mut engine = armed_world();
    let first = strike(&mut engine, 47, "Accumulated", false);
    engine
        .record_acquisition_character_effects(first, &mut BTreeMap::new(), 1)
        .unwrap();
    actor_mut(&mut engine, 0).reservation.id = "synthetic-replacement".into();
    let second = strike(&mut engine, 55, "Destroyed", false);
    let damage = second
        .audit()
        .iter()
        .find(|entry| entry["op"] == "structure_strike_damage")
        .unwrap();
    assert_eq!(damage["attacker"], u32::MAX);
    engine
        .record_acquisition_character_effects(second, &mut BTreeMap::new(), 1)
        .unwrap();
    assert_eq!(state(&engine), ObjectiveState::Deactivated);
    let third = strike(&mut engine, 63, "Ignored", false);
    engine
        .record_acquisition_character_effects(third, &mut BTreeMap::new(), 1)
        .unwrap();
    assert_eq!(state(&engine), ObjectiveState::Deactivated);
}

#[test]
fn trigger_damage_and_owner_callback_do_not_require_a_source_position_sample() {
    let mut engine = armed_world();
    actor_mut(&mut engine, 0).tracking.sample = None;
    let mut triggers = crate::structure_damage_world::World::new(
        &engine
            .active_abilities
            .as_ref()
            .unwrap()
            .setup
            .damage_triggers,
    )
    .unwrap();
    let mut objectives = engine.objective_world.take().unwrap();
    let first = engine
        .stage_structure_strike(
            &mut triggers,
            Some(&mut objectives),
            &[(STRUCTURAL, 0)],
            17.,
            0x0380_0000,
            Duration::from_millis(47),
        )
        .unwrap();
    assert_eq!(first[0]["outcome"], "Accumulated");
    let second = engine
        .stage_structure_strike(
            &mut triggers,
            Some(&mut objectives),
            &[(STRUCTURAL, 0)],
            17.,
            0x0380_0000,
            Duration::from_millis(55),
        )
        .unwrap();
    assert_eq!(second[0]["owner_progressed"], true);
    assert_eq!(objectives.state(STRUCTURAL), Some(ARMING));
}
