//! SYNTHETIC Domination bot engineers: a registry bot carrying the synthetic
//! repair tool repairs its side's destroyed structural objective.
use super::*;
use crate::Engine;

const TOOL: u16 = 2;

/// The repair fixture (objective 0 destroyed, owned by team 1) with one
/// registry bot of `squad`, carrying the repair tool, standing at it.
fn engineer_fixture(squad: u16) -> (Engine<Vec<u8>>, u16) {
    let mut engine = super::structural_repair::repair_fixture();
    let mut donor = crate::bot_registry::identity_tests::engine("Synthetic Engineer");
    let response = donor.handle(
        crate::Request::Reserve {
            v: crate::BRIDGE_VERSION,
            request_id: "synthetic-engineer-reserve".into(),
            reservation_id: "synthetic-engineer-0000".into(),
            world_id: 1,
            player_id: 0,
            account_id: 1,
            session_key: "aaaaaaaaaaaaaaaa".into(),
            access_key: "bbbbbbbbbbbbbbbb".into(),
            character: serde_json::from_value(json!({"name":"Synthetic Engineer","faction":2,
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
    let config = donor.bot_registry.config.clone().unwrap();
    engine.bot_registry = crate::bot_registry::BotRegistry::for_validated_population(config, 2);
    bot.target.squad = crate::replication::SquadAddress::from_index(squad).unwrap();
    let kit = &mut bot.target.loadout;
    for inventory in &mut kit.inventories {
        for slot in &mut inventory.slots {
            if *slot == Some(TOOL) {
                *slot = None;
            }
        }
    }
    kit.inventories[usize::from(kit.handle.inventory_index)].slots[5] = Some(TOOL);
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
    engine.bot_registry.bots.insert(id, bot);
    (engine, id)
}

fn repairs(engine: &mut Engine<Vec<u8>>, ms: u64) -> Vec<serde_json::Value> {
    let mut world = engine.domination_gameplay.take().unwrap();
    let result = engine.tick_domination_bot_repairs(&mut world, test_time(ms), 1);
    engine.domination_gameplay = Some(world);
    result.unwrap();
    String::from_utf8_lossy(&engine.writer)
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|row| row["op"] == "bot_repair")
        .collect()
}

#[test]
fn an_owning_engineer_is_sent_to_the_destroyed_objective_and_repairs_it() {
    let (mut engine, bot) = engineer_fixture(16);
    assert_eq!(
        engine.plan_repair_duties(),
        BTreeMap::from([(bot, 0)]),
        "the owner's engineer takes the duty"
    );
    engine.bot_registry.bots.get_mut(&bot).unwrap().task.repair = Some(0);
    let first = repairs(&mut engine, 1000);
    assert_eq!(first.len(), 1, "{first:?}");
    assert_eq!(first[0]["accepted"], true);
    assert_eq!(first[0]["queued"], true);
    assert_eq!(
        engine.objective_world.as_ref().unwrap().state(0),
        Some(ObjectiveState::Completed),
        "the repair runtime applies queued repairs on its own update"
    );
    // One submission per tool cooldown.
    assert_eq!(repairs(&mut engine, 1100).len(), 1);
    assert_eq!(repairs(&mut engine, 3000).len(), 2);
}

#[test]
fn engineers_of_the_other_side_or_without_the_tool_take_no_duty() {
    let (engine, _) = engineer_fixture(0);
    assert!(engine.plan_repair_duties().is_empty(), "attacker");
    let (mut engine, bot) = engineer_fixture(16);
    let kit = &mut engine
        .bot_registry
        .bots
        .get_mut(&bot)
        .unwrap()
        .target
        .loadout;
    kit.inventories[usize::from(kit.handle.inventory_index)].slots[5] = None;
    assert!(engine.plan_repair_duties().is_empty(), "no tool");
}

#[test]
fn an_engineer_away_from_the_objective_does_not_submit() {
    let (mut engine, bot) = engineer_fixture(16);
    let b = engine.bot_registry.bots.get_mut(&bot).unwrap();
    b.task.repair = Some(0);
    b.target.runtime.as_mut().unwrap().position.x = 40.;
    assert!(repairs(&mut engine, 1000).is_empty());
}

#[test]
fn engineer_slots_carry_the_repair_tool_in_place_of_the_self_heal_item() {
    let (engine, bot) = engineer_fixture(16);
    let mut kit = engine.bot_registry.bots[&bot].target.loadout.clone();
    let index = usize::from(kit.handle.inventory_index);
    kit.inventories[index].slots[5] = None;
    let policy = crate::bot_repair::Policy::default();
    let unchanged = crate::bot_repair::engineer_loadout(
        &policy,
        2,
        &engine.item_catalog,
        Some(TOOL),
        kit.clone(),
    );
    assert_eq!(
        unchanged.inventories[index].slots, kit.inventories[index].slots,
        "no healing slot"
    );
    let rifleman =
        crate::bot_repair::engineer_loadout(&policy, 0, &engine.item_catalog, Some(TOOL), kit);
    assert!(!rifleman.inventories[index].slots.contains(&Some(TOOL)));
}
