//! Authenticated event routing snapshots both objective timing slots.
use super::*;

fn fixture(native: bool) -> Engine<Vec<u8>> {
    let mut engine = crate::objective_reward_tests::fixture();
    engine.leadership = None;
    let skills = skill_catalog::SkillCatalog::from_value(json!({
        "schema_version":2,"application_id":20231,"table":{"Skills":{
            "MagCharacterSkill0":{"MagCharacterSkill":{
                "Default":{"Modifier":"kCharModSecureAndHoldTime","ModifierValue":-0.5},
                "Green":{"Available":true}}},
            "MagCharacterSkill1":{"MagCharacterSkill":{
                "Default":{"Modifier":"kCharModDemolitionTime","ModifierValue":0.25},
                "Green":{"Available":true}}}
        }}
    }))
    .unwrap();
    engine.active.as_mut().unwrap().modifiers = skills.resolve(0, &[3, 0, 0, 0, 0]).unwrap();
    if !native {
        engine.domination_gameplay = None;
        let mut catalog = engine.objective_catalog.clone().unwrap();
        for object in &mut catalog.objects {
            object.native_class = None;
        }
        engine.objective_world = Some(objective_control::ObjectiveWorld::new(&catalog));
        engine.objective_catalog = Some(catalog);
    }
    engine
}

fn secure_deadline(engine: &Engine<Vec<u8>>) -> u32 {
    engine
        .objective_world
        .as_ref()
        .unwrap()
        .snapshot(1, 0)
        .objectives
        .into_iter()
        .find(|object| object.object_index == 1)
        .unwrap()
        .expiration_time_ms
}

#[test]
fn action_timing_event_routes_secure_slot_on_both_objective_paths() {
    for native in [false, true] {
        let mut engine = fixture(native);
        crate::objective_reward_tests::interact(
            &mut engine,
            1,
            protocol::ObjectiveInteractionKind::Activate,
            100,
        )
        .unwrap();
        assert_eq!(
            engine.objective_world.as_ref().unwrap().state(1),
            Some(replication::ObjectiveState::Activating)
        );
        // 37 × (1 − .5) truncates to18; Demolition +.25 must not be selected.
        assert_eq!(secure_deadline(&engine), 118, "native={native}");
    }
}

#[test]
fn action_timing_event_record_refusal_preserves_objective_and_actor() {
    for native in [false, true] {
        let mut engine = fixture(native);
        let before = engine
            .objective_world
            .as_ref()
            .unwrap()
            .snapshot(1, 0)
            .encode_with_profile(engine.wire_profile)
            .unwrap();
        let actor = engine.active.as_ref().unwrap();
        let sequence = actor.next_sequence().unwrap();
        let modifiers = [
            actor.modifiers.value("kCharModSecureAndHoldTime"),
            actor.modifiers.value("kCharModDemolitionTime"),
        ];
        engine.max_log = engine.written;
        assert_eq!(
            crate::objective_reward_tests::interact(
                &mut engine,
                1,
                protocol::ObjectiveInteractionKind::Activate,
                100,
            )
            .unwrap_err(),
            "log_limit_reached"
        );
        assert_eq!(
            engine
                .objective_world
                .as_ref()
                .unwrap()
                .snapshot(1, 0)
                .encode_with_profile(engine.wire_profile)
                .unwrap(),
            before
        );
        let actor = engine.active.as_ref().unwrap();
        assert_eq!(actor.next_sequence().unwrap(), sequence);
        assert_eq!(
            [
                actor.modifiers.value("kCharModSecureAndHoldTime"),
                actor.modifiers.value("kCharModDemolitionTime")
            ],
            modifiers
        );
    }
}
