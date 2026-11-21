//! Synthetic Original userdata identity and per-invocation actor resolution.
use super::*;
use crate::domination_control::{Caller, Effect, Invocation};

fn request(target: u32, state: State, caller: Caller) -> Effect {
    Effect::SetState {
        objective_token: target,
        state,
        caller,
    }
}
fn actor(player: u16) -> Actor {
    Actor {
        player,
        generation: 1,
        team: 0,
    }
}

#[test]
fn original_resolves_current_source_objective_actor_for_each_effect() {
    let (runtime, mut world, _, mut round) = fixture();
    world
        .stage_native_state(0, ObjectiveState::Completed, Some(actor(7)))
        .unwrap();
    let mut owner = Spy {
        allow_callback_actor: true,
        ..Default::default()
    };
    let batch = InvocationEffects {
        context: Context {
            transition: Transition {
                objective_token: FIRST,
                event: Event::Completed,
            },
            caller_object_id: 0x0c80_0000,
            invocation: Invocation {
                binding_token: 77,
                entry_index: 0,
                delay_ms: 0,
            },
        },
        effects: vec![
            request(NEXT, State::Completed, Caller::Original),
            request(FIRST, State::Deactivated, Caller::None),
            request(NEXT, State::Locked, Caller::Original),
        ],
    };
    apply_effects(
        &runtime.setup,
        &mut world,
        &mut round,
        &mut owner,
        100,
        batch,
    )
    .unwrap();
    assert_eq!(owner.callback_actors.into_inner(), vec![0x0380_0007]);
}

#[test]
fn delayed_original_retains_objective_identity_and_reads_later_actor() {
    let (runtime, mut world, _, mut round) = fixture();
    let h = "1".repeat(64);
    let catalog=domination_control::Catalog::from_json(json!({
        "format":"mag_domination_control","schema_version":1,"game_archive_sha256":h,
        "metadata_sha256":h,"script_sha256":[h],"maps":[{
            "name":"synthetic","level_archive_sha256":h,"level_clv_sha256":h,
            "bindings":[{"binding_token":77,"entries":[{"delay_ms":5000,"rules":[{
                "conditions":[],"commands":[{"kind":"set_state","objective_token":NEXT,"state":"completed","caller":"original"}]}]}]}],
            "hooks":[{"objective_token":FIRST,"event":"completed","binding_token":77,"resolved":true}]
        }]}).to_string().as_bytes()).unwrap();
    let mut queue = Queue::new(
        Arc::new(catalog.map("synthetic").unwrap().clone()),
        NonZeroUsize::new(8).unwrap(),
    );
    let transition = Transition {
        objective_token: FIRST,
        event: Event::Completed,
    };
    queue
        .execute_event(transition, 0x0c80_0000, 100, &BTreeMap::new(), |_| Ok(()))
        .unwrap();
    world
        .stage_native_state(0, ObjectiveState::Completed, Some(actor(7)))
        .unwrap();
    world
        .stage_native_state(0, ObjectiveState::Deactivated, Some(actor(9)))
        .unwrap();
    let mut owner = Spy {
        allow_callback_actor: true,
        ..Default::default()
    };
    assert!(
        queue
            .consume_next_due(5099, &BTreeMap::new(), |_| Ok(()))
            .unwrap()
            .is_none()
    );
    queue
        .consume_next_due(5100, &BTreeMap::new(), |batch| {
            apply_effects(
                &runtime.setup,
                &mut world,
                &mut round,
                &mut owner,
                5100,
                batch,
            )
        })
        .unwrap()
        .unwrap();
    assert_eq!(owner.callback_actors.into_inner(), vec![0x0380_0009]);
    assert_eq!(queue.pending_count(), 0);
}

#[test]
fn cyclic_script_requests_are_bounded_without_installing_partial_state() {
    let (runtime, world, spawns, round) = fixture();
    let h = "1".repeat(64);
    let catalog=domination_control::Catalog::from_json(json!({
        "format":"mag_domination_control","schema_version":1,"game_archive_sha256":h,
        "metadata_sha256":h,"script_sha256":[h],"maps":[{
            "name":"synthetic","level_archive_sha256":h,"level_clv_sha256":h,
            "bindings":[
                {"binding_token":77,"entries":[{"delay_ms":0,"rules":[{"conditions":[],"commands":[{
                    "kind":"set_state","objective_token":FIRST,"state":"completed","caller":"none"}]}]}]},
                {"binding_token":78,"entries":[{"delay_ms":0,"rules":[{"conditions":[],"commands":[{
                    "kind":"set_state","objective_token":FIRST,"state":"deactivated","caller":"none"}]}]}]}],
            "hooks":[
                {"objective_token":FIRST,"event":"deactivated","binding_token":77,"resolved":true},
                {"objective_token":FIRST,"event":"completed","binding_token":78,"resolved":true}]
        }]}).to_string().as_bytes()).unwrap();
    let mut setup = (*runtime.setup).clone();
    setup.map = Arc::new(catalog.map("synthetic").unwrap().clone());
    let runtime = Runtime::new(Arc::new(setup), NonZeroUsize::new(8).unwrap());
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let started = runtime
            .stage_mission_start(
                frame(100, &world, &spawns, &round),
                Spy {
                    allow_strike_lifecycle: true,
                    precision_trigger: true,
                    ..Default::default()
                },
            )
            .unwrap();
        let result = started.runtime.stage_objective_update(
            frame(100, &started.world, &started.spawns, &started.round),
            &BTreeMap::new(),
            started.effects,
        );
        let error = result.err();
        assert_eq!(world.state(0), Some(ObjectiveState::Inactive));
        sender.send(error).unwrap();
    });
    assert_eq!(
        receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("script loop did not stop at its transition bound"),
        Some("native_objective_transition_limit")
    );
}
