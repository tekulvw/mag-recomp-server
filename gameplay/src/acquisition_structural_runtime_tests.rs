use super::*;
use crate::{
    CharacterLife, Direction, Transport,
    acquisition_gameplay_engine::{StructuralInput, objective_actors},
    events::EventInput,
    protocol::{ObjectiveInteraction, ObjectiveInteractionKind, encode_hex},
    replication::CharacterPosition,
};

const INDEX: u16 = 120;
const TARGET: u32 = 0x0c80_0078;

fn clock(ms: u64) -> Duration {
    Duration::from_millis(ms)
}

fn tick(engine: &mut Engine<Vec<u8>>, now: u64) -> Result<Vec<OutboundMessage>, &'static str> {
    engine.tick_acquisition_objectives(clock(now), 1)
}

fn state(engine: &Engine<Vec<u8>>) -> ObjectiveState {
    engine
        .objective_world
        .as_ref()
        .unwrap()
        .state(INDEX)
        .unwrap()
}

fn sequences(engine: &Engine<Vec<u8>>) -> Vec<ObjectiveSequenceDelta> {
    engine
        .acquisition_gameplay
        .as_ref()
        .unwrap()
        .current_structural_sequences()
}

fn start_arm(engine: &mut Engine<Vec<u8>>) {
    let active = engine.active.as_ref().unwrap();
    let actor = objective_actors(engine)[&active.reservation.player];
    let request = ObjectiveInteraction {
        kind: ObjectiveInteractionKind::Activate,
        target_object_id: TARGET,
        actor_index: actor.player,
        trigger_index: 1,
    };
    let event = EventInput {
        reservation_id: active.reservation.id.clone(),
        world_id: active.reservation.world,
        player_id: actor.player,
        transport: Transport::Udp,
        direction: Direction::ClientToServer,
        rt_type: 0x3d,
        plugin_id: Some(1),
        payload_hex: encode_hex(&request.encode()),
    };
    let now = clock(100);
    let leadership = engine.plan_leadership(now).unwrap();
    engine
        .event_acquisition_structural(
            &event,
            StructuralInput::Interaction(request, actor, 0.),
            leadership,
            now,
            1,
        )
        .unwrap();
    assert_eq!(state(engine), ObjectiveState::Activating);
    tick(engine, 110).unwrap();
    assert_eq!(state(engine), ObjectiveState::Activated);
}

fn static_scene() -> crate::collision::CollisionWorld {
    let scene = json!({
        "format":"mag_collision_world","schema_version":1,"coordinate_space":"world",
        "metadata":{"map_name":"synthetic_arena","build":"synthetic","coverage":"provisional",
            "provenance":["synthetic"],"unknowns":[]},
        "instances":[{"id":"distant_floor",
            "collision":{"filter_info":9,"convex_radius":0.,
                "child_filter_policy":"uniform","provenance":"synthetic"},
            "vertices_xyz":[[0.,0.,0.],[3.,0.,0.],[0.,0.,3.]],"triangles":[[0,1,2]]}]
    });
    crate::collision::CollisionWorld::from_bytes(
        &serde_json::to_vec(&scene).unwrap(),
        Default::default(),
    )
    .unwrap()
}

fn last_record(engine: &Engine<Vec<u8>>) -> Value {
    serde_json::from_slice(
        engine
            .writer
            .split(|&b| b == b'\n')
            .rfind(|r| !r.is_empty())
            .unwrap(),
    )
    .unwrap()
}

#[test]
fn fuse_uses_selected_trigger_placement_and_rolls_back_failed_effects() {
    let mut engine = connected_tests::fixture_with_damage(20.);
    engine.item_catalog = crate::human_kill_reward_tests::combat_items_for_ground_test(0.);
    engine
        .active
        .as_mut()
        .unwrap()
        .spawned_loadout
        .as_mut()
        .unwrap()
        .inventories[1]
        .slots[12] = Some(3);
    start_arm(&mut engine);

    // Trigger index 1 has placement [24,3,10] and box center [20,3,10].
    // Stand at the placement, outside the box, after planting the charge.
    let active = engine.active.as_mut().unwrap();
    active.tracking.server_position(
        CharacterPosition {
            x: 24.,
            y: 3.,
            z: 10.,
        },
        crate::player_tracking::PositionOrigin::ServerSpawn,
        111,
    );
    let life = active.life.damage_health();
    let pending = active.pending_shared.clone();
    let ground = engine.ground_runtime.clone();
    let playback = sequences(&engine);
    let sequence = engine.sequence;

    assert_eq!(
        tick(&mut engine, 130).unwrap_err(),
        "domination_structural_static_collision_required"
    );
    assert_eq!(state(&engine), ObjectiveState::Activated);
    assert_eq!(engine.active.as_ref().unwrap().life.damage_health(), life);
    assert_eq!(engine.active.as_ref().unwrap().pending_shared, pending);
    assert_eq!(engine.ground_runtime, ground);
    assert_eq!(sequences(&engine), playback);
    assert_eq!(engine.sequence, sequence);

    engine.collision_world = Some(static_scene());
    let limit = engine.max_log;
    engine.max_log = engine.written;
    assert_eq!(tick(&mut engine, 130).unwrap_err(), "log_limit_reached");
    assert_eq!(state(&engine), ObjectiveState::Activated);
    assert_eq!(engine.active.as_ref().unwrap().life.damage_health(), life);
    assert_eq!(engine.active.as_ref().unwrap().pending_shared, pending);
    assert_eq!(engine.ground_runtime, ground);
    assert_eq!(sequences(&engine), playback);
    assert_eq!(engine.sequence, sequence);

    engine.failed = false;
    engine.max_log = limit;
    tick(&mut engine, 130).unwrap();
    assert_eq!(state(&engine), ObjectiveState::Completed);
    assert_ne!(engine.active.as_ref().unwrap().life.damage_health(), life);
    assert_ne!(sequences(&engine), playback);
    let record = last_record(&engine);
    let hit = record["effects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|effect| effect["source_object"] == TARGET && effect["on_character_hit"] == true)
        .unwrap();
    // The standing sample is one metre above the raised blast origin.
    // At that distance within the two-metre inner radius, 20 HP scales to 15 HP.
    assert_eq!(hit["raw_amount_hp"], 15.0);
}

#[test]
fn native_precision_consumes_each_intermediate_entry_in_order() {
    let mut engine = connected_tests::fixture();
    let world = engine.objective_world.as_mut().unwrap();
    assert!(world.stage_precision_strike(INDEX, None).unwrap());
    tick(&mut engine, 100).unwrap();
    assert_eq!(state(&engine), ObjectiveState::Completed);
    let record = last_record(&engine);
    let entries: Vec<_> = record["effects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|effect| {
            effect["op"] == "acquisition_structural_entry" && effect["object"] == INDEX
        })
        .map(|effect| {
            (
                effect["state"].as_u64().unwrap(),
                effect["explosion"].as_bool().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        entries,
        [
            (ObjectiveState::Activating as u64, false),
            (ObjectiveState::Activated as u64, false),
            (ObjectiveState::Completed as u64, true),
        ]
    );
}

#[test]
fn redundant_queued_state_does_not_cancel_a_pending_precision_strike() {
    let mut engine = connected_tests::fixture();
    // MotorPool callbacks can queue the child's current state. Like the
    // legacy provider, that no-op must leave a separately staged strike alone.
    crate::acquisition_gameplay_engine::test_support::structural_fixture(
        engine.acquisition_gameplay.as_mut().unwrap(),
    )
    .queued
    .insert(INDEX, (ObjectiveState::Deactivated, u16::MAX));
    assert!(
        engine
            .objective_world
            .as_mut()
            .unwrap()
            .stage_precision_strike(INDEX, None)
            .unwrap()
    );
    tick(&mut engine, 100).unwrap();
    assert_eq!(state(&engine), ObjectiveState::Completed);
}

#[test]
fn repairable_native_rows_require_a_tool_source() {
    let engine = connected_tests::fixture();
    let objectives = engine.objective_catalog.as_ref().unwrap();
    let mut setup = engine
        .acquisition_gameplay
        .as_ref()
        .unwrap()
        .setup
        .structural
        .clone()
        .unwrap();
    setup.validate_native_objectives(objectives).unwrap();

    setup.repair_tool = None;
    assert_eq!(
        setup.validate_native_objectives(objectives),
        Err("acquisition_structural_repair_catalog_required")
    );
}

#[test]
fn native_area_damage_requires_ground_seat_visibility_at_admission() {
    for (radius, damage, expected) in [
        (0., 5., Ok(())),
        (5., 0., Ok(())),
        (
            5.,
            5.,
            Err("acquisition_structural_ground_seat_visibility_required"),
        ),
    ] {
        let engine = connected_tests::fixture_with_damage_and_area(0., radius, damage);
        let setup = &engine.acquisition_gameplay.as_ref().unwrap().setup;
        let vehicles = engine.ground_vehicles.as_ref().unwrap();
        assert!(vehicles.driver_seat(4).is_some());
        assert_eq!(vehicles.seat_occupant_visible(4, 0), None);
        assert_eq!(setup.validate_ground_drivers(vehicles), expected);
    }
}

#[test]
fn startup_is_staged_before_later_native_script_requests() {
    let mut engine = connected_tests::fixture();
    engine.acquisition_gameplay.as_mut().unwrap().reset();
    {
        let structural = crate::acquisition_gameplay_engine::test_support::structural_fixture(
            engine.acquisition_gameplay.as_mut().unwrap(),
        );
        let world = engine.objective_world.as_mut().unwrap();
        structural.stage_mission_start(world).unwrap();
        assert!(
            world.has_script_transition(INDEX),
            "MissionStart must stage the direct target before callbacks"
        );
        // This request occurs after MissionStart and before the first Sim pass.
        world
            .stage_native_state(INDEX, ObjectiveState::Completed, None)
            .unwrap();
        structural.stage_mission_start(world).unwrap();
        for _ in 0..8 {
            world.tick_native_object(INDEX, clock(100), &BTreeMap::new());
            world.take_native_entries();
            if !world.has_script_transition(INDEX) {
                break;
            }
        }
    }
    assert_eq!(state(&engine), ObjectiveState::Completed);
}

fn registered_structural_operator() -> (Engine<Vec<u8>>, u16) {
    let mut engine = connected_tests::fixture();
    start_arm(&mut engine);
    register_structural_operator(engine)
}

fn register_structural_operator(mut engine: Engine<Vec<u8>>) -> (Engine<Vec<u8>>, u16) {
    engine.maximum_players = engine.admission_capacity();
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let identities =
        crate::bot_registry::identity_tests::engine(&format!("Structural Operator {id}"));
    engine.item_catalog = identities.item_catalog;
    engine.bot_names = identities.bot_names;
    let human = engine.active.as_mut().unwrap();
    human.tracking.server_position(
        CharacterPosition {
            x: 5.,
            y: 3.,
            z: 5.,
        },
        crate::player_tracking::PositionOrigin::ServerSpawn,
        111,
    );
    assert_eq!(human.team(), 0);
    let scene = json!({
        "format":"mag_collision_world","schema_version":1,"coordinate_space":"world",
        "metadata":{"map_name":"synthetic_arena","build":"synthetic","coverage":"provisional",
            "provenance":["synthetic"],"unknowns":[],
            "physics_regions":[{"min":[-2.,-2.,-2.],"max":[32.,12.,32.],
                "ground_verified":true,"obstacles_verified":true,"provenance":["invented floor"]}]},
        "instances":[{"id":"floor","vertices_xyz":[[0.,3.,0.],[30.,3.,0.],[30.,3.,30.],[0.,3.,30.]],
            "triangles":[[0,2,1],[0,3,2]]}]
    });
    engine.collision_world = Some(
        crate::collision::CollisionWorld::from_bytes(
            &serde_json::to_vec(&scene).unwrap(),
            Default::default(),
        )
        .unwrap(),
    );
    let population = serde_json::from_value(json!({
        "format":"mag_bot_population_v1","map_name":"synthetic_arena","firearms":{"enabled":false},
        "slots":[{"team":0,"position":{"x":10.,"y":3.5,"z":20.}},
            {"team":1,"position":{"x":20.,"y":3.5,"z":10.}}]
    }))
    .unwrap();
    let mut engine = engine.with_bot_population(population, 2).unwrap();
    engine.tick_bots(clock(112), 1).unwrap();
    let (&id, bot) = engine.bot_registry.bots.iter().next().unwrap();
    assert_eq!(bot.target.squad.team(), 1);
    let terrain =
        crate::bot_physics::WorldTerrain::new(engine.collision_world.as_ref(), engine.map_name())
            .unwrap();
    let motion = terrain
        .advance(
            bot.target.runtime.unwrap().physics_motion.unwrap(),
            [0.; 3],
            1.,
            Default::default(),
        )
        .unwrap();
    assert!(motion.grounded && !motion.blocked);
    let bot = engine.bot_registry.bots.get_mut(&id).unwrap();
    let runtime = bot.target.runtime.as_mut().unwrap();
    runtime.physics_motion = Some(motion);
    bot.task.operator = true;
    bot.task.objective = Some(INDEX);
    // This fixture isolates source action admission after normal operator selection.
    bot.task.set_life_for_test(runtime.generation);
    let config = engine.bot_registry.config.as_mut().unwrap();
    config.autonomous = true;
    config
        .objective_policy
        .as_mut()
        .unwrap()
        .interactions_enabled = true;
    (engine, id)
}

#[test]
fn registered_bot_defuses_at_the_human_charge_trigger() {
    let (mut engine, bot) = registered_structural_operator();
    assert_eq!(state(&engine), ObjectiveState::Activated);
    let rewards_before = engine.pending_reward_batch();
    tick(&mut engine, 113).unwrap();
    assert_eq!(state(&engine), ObjectiveState::Deactivating);
    let owner = engine
        .objective_world
        .as_ref()
        .unwrap()
        .interaction_owner(INDEX)
        .unwrap();
    assert_eq!(owner.0, bot);
    assert_eq!(owner.2, 1);
    tick(&mut engine, 121).unwrap();
    assert_eq!(state(&engine), ObjectiveState::Deactivated);
    tick(&mut engine, 130).unwrap();
    assert_eq!(state(&engine), ObjectiveState::Deactivated);
    assert_eq!(engine.pending_reward_batch(), rewards_before);
}

#[test]
fn registered_bot_source_structural_start_is_record_first() {
    let (mut engine, bot) = registered_structural_operator();
    let before = engine.active.as_ref().unwrap().pending_shared.clone();
    let prior_goal = CharacterPosition {
        x: 21.,
        y: 3.,
        z: 10.,
    };
    let target = engine.bot_registry.bots.get_mut(&bot).unwrap();
    target.task.goal = Some(prior_goal);
    target.target.runtime.as_mut().unwrap().phase = crate::development_target::BotPhase::Moving;
    engine.max_log = engine.written;
    assert_eq!(tick(&mut engine, 113).unwrap_err(), "log_limit_reached");
    assert_eq!(state(&engine), ObjectiveState::Activated);
    assert!(
        engine
            .objective_world
            .as_ref()
            .unwrap()
            .interaction_owner(INDEX)
            .is_none()
    );
    assert_eq!(engine.active.as_ref().unwrap().pending_shared, before);
    let bot = &engine.bot_registry.bots[&bot];
    assert_eq!(bot.task.goal, Some(prior_goal));
    assert!(matches!(
        bot.target.runtime.unwrap().phase,
        crate::development_target::BotPhase::Moving
    ));
}

#[test]
fn retasked_registered_bot_releases_source_structural_hold() {
    let (mut engine, bot) = registered_structural_operator();
    tick(&mut engine, 113).unwrap();
    assert_eq!(state(&engine), ObjectiveState::Deactivating);
    engine
        .bot_registry
        .bots
        .get_mut(&bot)
        .unwrap()
        .task
        .operator = false;
    tick(&mut engine, 114).unwrap();
    assert_eq!(state(&engine), ObjectiveState::Activated);
    let record = last_record(&engine);
    assert_eq!(record["op"], "acquisition_objective_tick");
    assert!(record["objective_snapshot"].is_string());
    assert!(record["effects"].as_array().unwrap().iter().any(|effect| {
        effect["op"] == "acquisition_structural_entry"
            && effect["object"] == INDEX
            && effect["state"] == ObjectiveState::Activated as u8
    }));
    assert!(
        engine
            .objective_world
            .as_ref()
            .unwrap()
            .interaction_owner(INDEX)
            .is_none()
    );
    tick(&mut engine, 130).unwrap();
    assert_eq!(state(&engine), ObjectiveState::Completed);
}

#[test]
fn registered_bot_defuse_does_not_survive_slot_reallocation() {
    let (mut engine, bot) = registered_structural_operator();
    tick(&mut engine, 113).unwrap();
    assert_eq!(state(&engine), ObjectiveState::Deactivating);
    // Simulate slot reuse with an identical life counter and feet.
    engine
        .bot_registry
        .bots
        .get_mut(&bot)
        .unwrap()
        .allocation_generation += 1;
    tick(&mut engine, 121).unwrap();
    assert_eq!(state(&engine), ObjectiveState::Activated);
    assert!(
        engine
            .objective_world
            .as_ref()
            .unwrap()
            .interaction_owner(INDEX)
            .is_none()
    );
}

#[test]
fn defender_bot_cannot_defuse_from_the_other_authored_box() {
    let (mut engine, bot) = registered_structural_operator();
    let runtime = engine
        .bot_registry
        .bots
        .get_mut(&bot)
        .unwrap()
        .target
        .runtime
        .as_mut()
        .unwrap();
    runtime.position.x = 10.;
    runtime.physics_motion.as_mut().unwrap().position[0] = 10.;
    tick(&mut engine, 113).unwrap();
    assert_eq!(state(&engine), ObjectiveState::Activated);
    assert!(
        engine
            .objective_world
            .as_ref()
            .unwrap()
            .interaction_owner(INDEX)
            .is_none()
    );
}

#[test]
fn replacement_human_must_start_its_own_source_defuse() {
    let (mut engine, slot) = registered_structural_operator();
    tick(&mut engine, 113).unwrap();
    assert_eq!(state(&engine), ObjectiveState::Deactivating);
    let original_life = engine.bot_registry.bots[&slot]
        .target
        .runtime
        .unwrap()
        .generation;
    engine.retire_bot_for_human(slot, clock(114), 1).unwrap();
    connected_tests::defender_at(&mut engine, slot, clock(114));
    let defender = std::sync::Arc::new(
        engine
            .spawn_catalog
            .as_ref()
            .unwrap()
            .for_squad(16)
            .unwrap(),
    );
    let actor = engine.active.as_mut().unwrap();
    assert_eq!(actor.reservation.player, slot);
    crate::spawn_progression::tests::ready(actor);
    actor.squad_catalog = defender;
    actor.life = CharacterLife::Alive {
        generation: original_life,
        health_quarter_points: 400,
    };
    actor.tracking.server_position(
        CharacterPosition {
            x: 20.,
            y: 3.,
            z: 10.,
        },
        crate::player_tracking::PositionOrigin::ServerSpawn,
        114,
    );
    assert_eq!(actor.team(), 1);
    actor.reservation.character.account_id = Some("synthetic-replacement-account".into());
    connected_tests::bind_repair_counter_squad(&mut engine, INDEX);
    let attempt =
        connected_tests::interact(&mut engine, ObjectiveInteractionKind::Deactivate, 1, 114)
            .unwrap();
    assert!(attempt.iter().any(|m| {
        ObjectiveInteraction::decode(&crate::protocol::decode_hex(&m.payload_hex))
            .is_ok_and(|packet| packet.kind == ObjectiveInteractionKind::Nak)
    }));
    let rewards_before = engine.pending_reward_batch();
    let cancelled = tick(&mut engine, 115).unwrap();
    assert!(!cancelled.iter().any(|m| {
        ObjectiveInteraction::decode(&crate::protocol::decode_hex(&m.payload_hex))
            .is_ok_and(|packet| packet.kind == ObjectiveInteractionKind::ActivatingOnExit)
    }));
    assert_eq!(state(&engine), ObjectiveState::Activated);
    assert_eq!(engine.pending_reward_batch(), rewards_before);
    let fresh =
        connected_tests::interact(&mut engine, ObjectiveInteractionKind::Deactivate, 1, 116)
            .unwrap();
    assert!(fresh.iter().any(|m| {
        ObjectiveInteraction::decode(&crate::protocol::decode_hex(&m.payload_hex))
            .is_ok_and(|packet| packet.kind == ObjectiveInteractionKind::Ack)
    }));
    assert_eq!(state(&engine), ObjectiveState::Deactivating);
    tick(&mut engine, 123).unwrap();
    assert_eq!(state(&engine), ObjectiveState::Deactivating);
    tick(&mut engine, 124).unwrap();
    assert_eq!(state(&engine), ObjectiveState::Deactivated);
    let awards = engine.pending_reward_batch();
    assert_eq!(awards.len(), rewards_before.len() + 1);
    assert!(awards.iter().any(|award| {
        award.player_id == slot && award.kind == crate::progression::RewardKind::ChargeDefused
    }));
}

#[test]
fn source_structural_defuse_cancels_when_bound_bot_stops_qualifying() {
    for condition in ["life", "health", "airborne", "disabled", "round", "world"] {
        let (mut engine, bot) = registered_structural_operator();
        tick(&mut engine, 113).unwrap();
        assert_eq!(state(&engine), ObjectiveState::Deactivating, "{condition}");
        let bot = engine.bot_registry.bots.get_mut(&bot).unwrap();
        let runtime = bot.target.runtime.as_mut().unwrap();
        match condition {
            "life" => runtime.generation += 1,
            "health" => bot.health = 0,
            "airborne" => runtime.physics_motion.as_mut().unwrap().grounded = false,
            "disabled" => {
                engine.bot_registry.config.as_mut().unwrap().autonomous = false;
            }
            "round" => engine.round_number += 1,
            "world" => engine.active.as_mut().unwrap().reservation.world += 1,
            _ => unreachable!(),
        }
        tick(&mut engine, 121).unwrap();
        assert_eq!(state(&engine), ObjectiveState::Activated, "{condition}");
    }
}

#[test]
fn source_structural_defuse_cannot_delay_the_human_fuse() {
    let (mut engine, _) = registered_structural_operator();
    tick(&mut engine, 130).unwrap();
    assert_eq!(state(&engine), ObjectiveState::Completed);
    let record = last_record(&engine);
    let effects = record["effects"].as_array().unwrap();
    assert!(
        !effects
            .iter()
            .any(|effect| effect["op"] == "acquisition_bot_defuse_started")
    );
}

#[test]
fn source_structural_bot_defuse_leaves_damage_activated_fuse_unchanged() {
    let mut engine = connected_tests::fixture();
    engine
        .objective_world
        .as_mut()
        .unwrap()
        .activate_destroyed_trigger(
            INDEX,
            1,
            crate::structure_damage::Source {
                player: 0,
                generation: 1,
            },
            clock(100),
            0.,
        )
        .unwrap();
    tick(&mut engine, 110).unwrap();
    assert_eq!(state(&engine), ObjectiveState::Activated);
    let (mut engine, _) = register_structural_operator(engine);
    tick(&mut engine, 113).unwrap();
    assert_eq!(state(&engine), ObjectiveState::Activated);
}
