use super::*;
use crate::{
    Direction, Transport,
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
