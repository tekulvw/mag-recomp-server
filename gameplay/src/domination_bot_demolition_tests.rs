//! SYNTHETIC Domination demolition teams: an attacking registry bot picks the
//! synthetic structure that holds the defenders' forward spawn, arms it
//! through the player START path, and the structure's spawn closes when it
//! is destroyed and reopens when the defenders repair it.
use super::*;
use crate::Engine;
use crate::bot_objectives::ObjectivePolicy;

/// The repair fixture with structure 0 standing (its forward spawn, point 2,
/// open) and one attacking bot of squad 0, whose lane is point 1, standing
/// in the structure's trigger.
fn demolition_fixture() -> (Engine<Vec<u8>>, u16) {
    let mut engine = super::structural_repair::repair_fixture();
    let mut catalog = engine.objective_catalog.clone().unwrap();
    catalog.objects[0].initial_state = ObjectiveState::Deactivated as u8;
    catalog.objects[1].initial_state = ObjectiveState::Deactivated as u8;
    engine.objective_world = Some(ObjectiveWorld::new(&catalog));
    engine.objective_catalog = Some(catalog);
    engine.ground_runtime = Some(Default::default());
    let gameplay = engine.domination_gameplay.as_mut().unwrap();
    gameplay.state.assignments[0] = gameplay.state.assignments[16];
    Arc::make_mut(&mut gameplay.state.known_empty_vehicle_spawns).insert(2);
    gameplay.state.ground_respawn_physics = Some(Default::default());
    Arc::make_mut(&mut gameplay.state.root_poses)
        .insert(0, crate::dynamic_collision::Pose::at([1., 2., 1.]));
    Arc::make_mut(&mut gameplay.state.trigger_positions).insert((0, 0), [1., 2., 1.]);
    let mut donor = crate::bot_registry::identity_tests::engine("Synthetic Sapper");
    let response = donor.handle(
        crate::Request::Reserve {
            v: crate::BRIDGE_VERSION,
            request_id: "synthetic-sapper-reserve".into(),
            reservation_id: "synthetic-sapper-0000".into(),
            world_id: 1,
            player_id: 0,
            account_id: 1,
            session_key: "aaaaaaaaaaaaaaaa".into(),
            access_key: "bbbbbbbbbbbbbbbb".into(),
            character: serde_json::from_value(json!({"name":"Synthetic Sapper","faction":2,
                "head":0,"voice":0,"universe_id":26,"character_id":7}))
            .unwrap(),
        },
        Duration::ZERO,
        1,
    );
    assert!(response.ok, "{:?}", response.error);
    crate::spawn_progression::ready(donor.active.as_mut().unwrap());
    donor
        .reconcile_bot_population(Duration::ZERO, 1, &mut Default::default())
        .unwrap();
    let (&id, bot) = donor.bot_registry.bots.iter().next().unwrap();
    let mut bot = bot.clone();
    let mut config = donor.bot_registry.config.clone().unwrap();
    config.autonomous = true;
    let mut policy = ObjectivePolicy::bundled();
    policy.interactions_enabled = true;
    config.objective_policy = Some(policy);
    engine.bot_registry = crate::bot_registry::BotRegistry::for_validated_population(config, 2);
    bot.target.squad = crate::replication::SquadAddress::from_index(0).unwrap();
    let runtime = bot.target.runtime.as_mut().unwrap();
    runtime.position = crate::replication::CharacterPosition {
        x: 1.,
        y: 2.,
        z: 1.,
    };
    let mut motion = crate::bot_physics::Motion::new([1., 2., 1.]);
    motion.grounded = true;
    runtime.physics_motion = Some(motion);
    runtime.phase = crate::development_target::BotPhase::Complete;
    let generation = runtime.generation;
    bot.task.set_life_for_test(generation);
    engine.bot_registry.bots.insert(id, bot);
    (engine, id)
}

fn plan(engine: &mut Engine<Vec<u8>>, ms: u64) -> (BTreeMap<u16, u16>, Vec<serde_json::Value>) {
    let mut receipts = Vec::new();
    let duties = engine.plan_demolition_duties(test_time(ms), &Default::default(), &mut receipts);
    (duties, receipts)
}

fn interact(engine: &mut Engine<Vec<u8>>, ms: u64) {
    let mut world = engine.domination_gameplay.take().unwrap();
    let result = engine.tick_domination_bot_interactions(&mut world, test_time(ms), 1);
    engine.domination_gameplay = Some(world);
    result.unwrap();
}

fn rows(engine: &Engine<Vec<u8>>, op: &str) -> Vec<serde_json::Value> {
    String::from_utf8_lossy(&engine.writer)
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|row| row["op"] == op)
        .collect()
}

#[test]
fn the_runtime_reports_which_structure_holds_a_defender_spawn() {
    let (engine, _) = demolition_fixture();
    let hints = engine
        .domination_gameplay
        .as_ref()
        .unwrap()
        .runtime
        .structure_hints();
    assert_eq!(
        hints.get(&0),
        Some(&crate::bot_demolition::Hint {
            closes_spawn_of: Some(1),
            spawn_point: Some(2),
            ..Default::default()
        })
    );
    assert!(!hints.contains_key(&1), "a secure-and-hold point");
}

#[test]
fn an_attacking_squad_sends_a_bot_to_the_spawn_structure_near_its_lane() {
    let (mut engine, bot) = demolition_fixture();
    // No defender life uses the forward spawn yet: an idle spawn's value,
    // 40, less 0.5 per metre for the 3 m to the lane objective.
    let (duties, receipts) = plan(&mut engine, 1000);
    assert_eq!(duties, BTreeMap::from([(bot, 0)]));
    let picked = receipts
        .iter()
        .find(|r| r["op"] == "bot_demolition_team")
        .unwrap();
    assert_eq!(picked["score"], 38.5);
    // The living human defender's squad spawns there: the full spawn value.
    let (mut engine, bot) = demolition_fixture();
    engine.active.as_mut().unwrap().squad_spawn = 2;
    let (duties, receipts) = plan(&mut engine, 1000);
    assert_eq!(duties, BTreeMap::from([(bot, 0)]));
    let picked = receipts
        .iter()
        .find(|r| r["op"] == "bot_demolition_team")
        .unwrap();
    assert_eq!(picked["score"], 98.5);
    assert_eq!(picked["objective"], 0);
    // The defenders' own squads have no structure of the other side here.
    engine.bot_registry.bots.get_mut(&bot).unwrap().target.squad =
        crate::replication::SquadAddress::from_index(16).unwrap();
    let (duties, _) = plan(&mut engine, 2000);
    assert!(duties.is_empty());
}

#[test]
fn the_demolition_policy_switch_turns_teams_off() {
    let (mut engine, _) = demolition_fixture();
    engine
        .bot_registry
        .config
        .as_mut()
        .unwrap()
        .demolition
        .enabled = false;
    assert!(plan(&mut engine, 1000).0.is_empty());
    // A mode outside the policy's game types is off as well.
    let (mut engine, _) = demolition_fixture();
    engine
        .bot_registry
        .config
        .as_mut()
        .unwrap()
        .demolition
        .game_types = vec![1];
    assert!(plan(&mut engine, 1000).0.is_empty());
}

#[test]
fn the_operator_arms_the_structure_and_its_spawn_follows_destruction_and_repair() {
    let (mut engine, bot) = demolition_fixture();
    let (duties, _) = plan(&mut engine, 1000);
    assert_eq!(duties.get(&bot), Some(&0));
    // The objective planner makes the team's nearest member the operator.
    let task = &mut engine.bot_registry.bots.get_mut(&bot).unwrap().task;
    task.operator = true;
    task.objective = Some(0);
    interact(&mut engine, 1000);
    let started = rows(&engine, "bot_objective_action");
    assert_eq!(started.len(), 1, "{started:?}");
    assert_eq!(started[0]["accepted"], true);
    assert_eq!(
        started[0]["action"],
        protocol::ObjectiveInteractionKind::Activate as u8
    );
    assert_eq!(
        engine.objective_world.as_ref().unwrap().state(0),
        Some(ObjectiveState::Activating)
    );
    // Arm (10 ms), then the fuse (30 ms) destroys the structure.
    for ms in [1015, 1030, 1060, 1100] {
        engine.tick_domination_objectives(test_time(ms), 1).unwrap();
    }
    assert_eq!(
        engine.objective_world.as_ref().unwrap().state(0),
        Some(ObjectiveState::Completed)
    );
    assert!(
        !engine.spawn_catalog.as_ref().unwrap().point_active(2),
        "the defenders' forward spawn closes with its structure"
    );
    // Destroyed, it is no longer a target, and the team ends.
    let (duties, receipts) = plan(&mut engine, 1200);
    assert!(duties.is_empty());
    assert!(
        receipts
            .iter()
            .any(|r| r["op"] == "bot_demolition_team" && r["reason"] == "unavailable")
    );
    // The defenders repair it (the fixture's human defender carries the
    // tool; bot engineers submit the same request) and the spawn reopens.
    for ms in (1300..8000).step_by(1100) {
        super::structural_repair::repair(&mut engine, ms).unwrap();
        engine.tick_domination_objectives(test_time(ms), 1).unwrap();
        if engine.objective_world.as_ref().unwrap().state(0) == Some(ObjectiveState::Repaired) {
            break;
        }
    }
    assert_eq!(
        engine.objective_world.as_ref().unwrap().state(0),
        Some(ObjectiveState::Repaired)
    );
    assert!(engine.spawn_catalog.as_ref().unwrap().point_active(2));
    // A repaired structure may be armed again.
    let world = engine.objective_world.as_ref().unwrap();
    let object = &engine.objective_catalog.as_ref().unwrap().objects[0];
    assert!(
        crate::bot_objectives::operator_action(object, ObjectiveState::Repaired, 0, world)
            .is_some()
    );
}
