use super::*;
use ObjectiveState::*;
use serde_json::json;

const INDEX: u16 = 10;
const ATTACKER: secure::Actor = secure::Actor {
    player: 7,
    generation: 3,
    team: 0,
};
const DEFENDER: secure::Actor = secure::Actor {
    player: 8,
    generation: 4,
    team: 1,
};

fn world(initial: ObjectiveState) -> ObjectiveWorld {
    let mut catalog = trigger_box_tests::native_catalog();
    catalog["objects"][0]["initial_state"] = json!(initial as u8);
    if !matches!(initial, Deactivated | Activated) {
        for squad in catalog["squads"].as_array_mut().unwrap() {
            squad["initial_objective_index"] = json!(u16::MAX);
        }
    }
    ObjectiveWorld::new(&ObjectiveCatalog::from_json(&catalog.to_string()).unwrap())
}

fn step(world: &mut ObjectiveWorld) -> Option<NativeEntry> {
    let now = Duration::from_millis(100);
    world.begin_native_tick(now);
    world.tick_native_object(INDEX, now, &BTreeMap::new())?;
    let mut entries = world.take_native_entries();
    assert_eq!(entries.len(), 1);
    entries.pop()
}

fn trace(world: &mut ObjectiveWorld) -> Vec<(ObjectiveState, u16, Option<u16>)> {
    let mut entries = Vec::new();
    while let Some(entry) = step(world) {
        assert!(entries.len() < 6);
        entries.push((
            entry.delta.state_actor.state,
            entry.delta.state_actor.actor_index,
            entry.reward_actor,
        ));
    }
    entries
}

#[test]
fn secure_script_completion_enters_each_state_and_credits_only_completed() {
    let mut world = world(Deactivated);
    world
        .stage_native_state(INDEX, Completed, Some(ATTACKER))
        .unwrap();
    assert_eq!(world.state(INDEX), Some(Deactivated));
    assert_eq!(
        trace(&mut world),
        [
            (Activating, ATTACKER.player, None),
            (Activated, ATTACKER.player, None),
            (Completed, ATTACKER.player, Some(ATTACKER.player)),
        ]
    );
}

#[test]
fn secure_script_completion_from_inactive_clears_the_staged_attacker() {
    let mut world = world(Inactive);
    world
        .stage_native_state(INDEX, Completed, Some(ATTACKER))
        .unwrap();
    assert_eq!(
        trace(&mut world),
        [
            (Deactivated, u16::MAX, None),
            (Activating, u16::MAX, None),
            (Activated, u16::MAX, None),
            (Completed, u16::MAX, None),
        ]
    );
}

#[test]
fn secure_script_deactivation_from_completed_credits_the_last_defender_state() {
    let mut world = world(Completed);
    world
        .stage_native_state(INDEX, Deactivated, Some(DEFENDER))
        .unwrap();
    let entry = step(&mut world).unwrap();
    assert_eq!(entry.delta.state_actor.state, Retaking);
    assert_eq!(entry.delta.state_actor.actor_index, DEFENDER.player);
    assert_eq!(entry.reward_actor, None);
    world.stage_native_state(INDEX, Deactivated, None).unwrap();
    assert_eq!(
        trace(&mut world),
        [
            (Retaken, DEFENDER.player, None),
            (Deactivated, DEFENDER.player, Some(DEFENDER.player)),
        ]
    );
}

#[test]
fn secure_script_lock_keeps_a_pending_target_after_same_current_request() {
    let mut world = world(Deactivated);
    world
        .stage_native_state(INDEX, Locked, Some(ATTACKER))
        .unwrap();
    world
        .stage_native_state(INDEX, Deactivated, Some(DEFENDER))
        .unwrap();
    assert_eq!(
        trace(&mut world),
        [
            (Activating, ATTACKER.player, None),
            (Activated, ATTACKER.player, None),
            (Completed, ATTACKER.player, Some(ATTACKER.player)),
            (Locked, u16::MAX, None),
        ]
    );
}

#[test]
fn secure_script_from_activated_uses_the_defender_on_deactivation() {
    let mut world = world(Deactivated);
    world
        .stage_native_state(INDEX, Completed, Some(ATTACKER))
        .unwrap();
    assert_eq!(
        step(&mut world).unwrap().delta.state_actor.state,
        Activating
    );
    assert_eq!(step(&mut world).unwrap().delta.state_actor.state, Activated);
    world
        .stage_native_state(INDEX, Inactive, Some(DEFENDER))
        .unwrap();
    assert_eq!(
        trace(&mut world),
        [
            (Deactivating, DEFENDER.player, None),
            (Deactivated, DEFENDER.player, Some(DEFENDER.player)),
            (Inactive, u16::MAX, None),
        ]
    );
}

#[test]
fn secure_script_unresolved_duplicate_writes_raw_actor_but_preserves_the_latch() {
    let mut world = world(Deactivated);
    world
        .stage_native_state_with_caller(INDEX, Completed, Some(ATTACKER.player), Some(ATTACKER))
        .unwrap();
    world
        .stage_native_state_with_caller(INDEX, Completed, Some(33), None)
        .unwrap();
    assert_eq!(world.script_actor(INDEX), Some(33));
    assert_eq!(world.state(INDEX), Some(Deactivated));

    let activating = step(&mut world).unwrap();
    assert_eq!(activating.delta.state_actor.state, Activating);
    assert_eq!(activating.delta.state_actor.actor_index, ATTACKER.player);
    assert_eq!(activating.delta.expiration_time_ms, 100);
    assert_eq!(activating.delta.deactivation_time_ms, 0);
    assert_eq!(world.script_actor(INDEX), Some(ATTACKER.player));

    let activated = step(&mut world).unwrap();
    assert_eq!(activated.delta.state_actor.state, Activated);
    assert_eq!(activated.delta.expiration_time_ms, 100);
    assert_eq!(activated.reward_actor, None);

    let completed = step(&mut world).unwrap();
    assert_eq!(completed.delta.state_actor.state, Completed);
    assert_eq!(completed.delta.expiration_time_ms, 100);
    assert_eq!(completed.delta.deactivation_time_ms, 0);
    assert_eq!(completed.reward_actor, Some(ATTACKER.player));
}
