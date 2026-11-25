//! SYNTHETIC Domination bot operators: a registry bot at a secure-and-hold
//! point sends the player START and RELEASE through the native runtime.
use super::*;
use crate::Engine;
use crate::bot_objectives::ObjectivePolicy;

/// The repair fixture's world with point 1 open, and one attacking bot
/// operator standing in its trigger box.
fn capture_fixture(state: ObjectiveState) -> (Engine<Vec<u8>>, u16) {
    let mut engine = super::structural_repair::repair_fixture();
    let mut catalog = engine.objective_catalog.clone().unwrap();
    catalog.objects[1].initial_state = state as u8;
    engine.objective_world = Some(ObjectiveWorld::new(&catalog));
    engine.objective_catalog = Some(catalog);
    // Borrow a fully formed registry bot and population from another engine.
    let mut donor = crate::bot_registry::identity_tests::engine("Synthetic Operator");
    let response = donor.handle(
        crate::Request::Reserve {
            v: crate::BRIDGE_VERSION,
            request_id: "synthetic-operator-reserve".into(),
            reservation_id: "synthetic-operator-0000".into(),
            world_id: 1,
            player_id: 0,
            account_id: 1,
            session_key: "aaaaaaaaaaaaaaaa".into(),
            access_key: "bbbbbbbbbbbbbbbb".into(),
            character: serde_json::from_value(json!({"name":"Synthetic Operator","faction":2,
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
        x: 4.,
        y: 2.,
        z: 1.,
    };
    let mut motion = crate::bot_physics::Motion::new([4., 2., 1.]);
    motion.grounded = true;
    runtime.physics_motion = Some(motion);
    runtime.phase = crate::development_target::BotPhase::Complete;
    let generation = runtime.generation;
    bot.task.set_life_for_test(generation);
    bot.task.operator = true;
    bot.task.objective = Some(1);
    engine.bot_registry.bots.insert(id, bot);
    (engine, id)
}

/// The bot interaction pass of the Domination objective tick.
fn interact(engine: &mut Engine<Vec<u8>>, ms: u64) {
    let mut world = engine.domination_gameplay.take().unwrap();
    let result = engine.tick_domination_bot_interactions(&mut world, test_time(ms), 1);
    engine.domination_gameplay = Some(world);
    result.unwrap();
}

fn actions(engine: &Engine<Vec<u8>>) -> Vec<serde_json::Value> {
    String::from_utf8_lossy(&engine.writer)
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|row| row["op"] == "bot_objective_action")
        .collect()
}

#[test]
fn an_attacking_operator_secures_an_open_point_and_releases_when_retasked() {
    let (mut engine, bot) = capture_fixture(ObjectiveState::Deactivated);
    interact(&mut engine, 1000);
    let world = engine.objective_world.as_ref().unwrap();
    assert_eq!(world.state(1), Some(ObjectiveState::Activating));
    assert_eq!(world.interaction_owner(1).map(|o| o.0), Some(bot));
    let started = actions(&engine);
    assert_eq!(started.len(), 1, "{started:?}");
    assert_eq!(started[0]["accepted"], true);
    assert_eq!(
        started[0]["action"],
        protocol::ObjectiveInteractionKind::Activate as u8
    );
    // Retasked before the hold completes: the bot lets go like a player.
    engine
        .bot_registry
        .bots
        .get_mut(&bot)
        .unwrap()
        .task
        .operator = false;
    interact(&mut engine, 1010);
    let world = engine.objective_world.as_ref().unwrap();
    assert_eq!(world.state(1), Some(ObjectiveState::Deactivated));
    assert!(world.interaction_owner(1).is_none());
    assert_eq!(actions(&engine).len(), 2);
}

#[test]
fn a_held_point_progresses_while_the_operator_stays() {
    let (mut engine, bot) = capture_fixture(ObjectiveState::Deactivated);
    interact(&mut engine, 1000);
    let actors: BTreeMap<_, _> = engine.domination_bot_actors().into_iter().collect();
    assert!(actors.contains_key(&bot));
    // The native objective pass keeps a hold whose actor is still present.
    let tick = |actors: &BTreeMap<u16, ObjectiveActor>| {
        let mut world = engine.objective_world.clone().unwrap();
        world.begin_native_tick(test_time(1020));
        world.tick_native_object(1, test_time(1020), actors);
        world
    };
    let held = tick(&actors);
    assert_eq!(held.state(1), Some(ObjectiveState::Activating));
    assert_eq!(held.interaction_owner(1).map(|o| o.0), Some(bot));
    // Without the bot among the actors the hold is lost like a player's.
    let lost = tick(&BTreeMap::new());
    assert!(lost.interaction_owner(1).is_none());
}

#[test]
fn an_operator_outside_the_trigger_or_on_the_owning_side_does_not_start() {
    let (mut engine, bot) = capture_fixture(ObjectiveState::Deactivated);
    let runtime = engine
        .bot_registry
        .bots
        .get_mut(&bot)
        .unwrap()
        .target
        .runtime
        .as_mut()
        .unwrap();
    runtime.position.x = 9.;
    interact(&mut engine, 1000);
    assert!(actions(&engine).is_empty());
    let (mut engine, bot) = capture_fixture(ObjectiveState::Deactivated);
    engine.bot_registry.bots.get_mut(&bot).unwrap().target.squad =
        crate::replication::SquadAddress::from_index(16).unwrap();
    interact(&mut engine, 1000);
    assert!(
        actions(&engine).is_empty(),
        "the owner cannot secure its own point"
    );
}

#[test]
fn an_owning_operator_retakes_a_secured_point() {
    let (mut engine, bot) = capture_fixture(ObjectiveState::Completed);
    engine.bot_registry.bots.get_mut(&bot).unwrap().target.squad =
        crate::replication::SquadAddress::from_index(16).unwrap();
    interact(&mut engine, 1000);
    let started = actions(&engine);
    assert_eq!(started.len(), 1, "{started:?}");
    assert_eq!(
        started[0]["action"],
        protocol::ObjectiveInteractionKind::Deactivate as u8
    );
    assert_eq!(
        engine.objective_world.as_ref().unwrap().state(1),
        Some(ObjectiveState::Retaking)
    );
}

#[test]
fn an_operator_waiting_after_a_refusal_does_not_ask_again_early() {
    let (mut engine, bot) = capture_fixture(ObjectiveState::Deactivated);
    engine
        .bot_registry
        .bots
        .get_mut(&bot)
        .unwrap()
        .task
        .interaction_retry_at = test_time(2000);
    interact(&mut engine, 1000);
    assert!(actions(&engine).is_empty());
    interact(&mut engine, 2000);
    assert_eq!(actions(&engine).len(), 1);
}
