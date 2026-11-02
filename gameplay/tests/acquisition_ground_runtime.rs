use mag_gameplay::acquisition_ground_runtime::*;
use mag_gameplay::ground_vehicles::Creation;
use mag_gameplay::replication::CharacterPosition;

fn pose(x: f32) -> Pose {
    Pose {
        position: CharacterPosition { x, y: 4., z: 8. },
        rotation: [0., 0., 0., 1.],
    }
}
fn runtime(state: u8) -> Runtime {
    runtime_with_source(state, 9)
}
fn runtime_with_source(state: u8, source_type: u8) -> Runtime {
    let creation: Creation = serde_json::from_value(serde_json::json!({
        "index":4,"spawn_point_index":12,"spawn_point_token":99,
        "vehicle_type_token":5,"faction":0,"team":0,"modifier":0,
        "position":{"x":2.,"y":4.,"z":8.},"rotation":[0.,0.,0.,1.],
        "health_quarter_points":800,"initial_state":state
    }))
    .unwrap();
    Runtime::from_creation(&creation, Some(source_type)).unwrap()
}
fn capabilities() -> Capabilities {
    Capabilities {
        destruction: Some(Destruction {
            damage: 300.,
            full_radius: 8.,
            half_radius: 7.,
        }),
        hulk_template: Some("synthetic-hulk".into()),
        delivered_inert_seconds: Some(12.),
    }
}
fn source() -> QualifiedRespawn {
    QualifiedRespawn {
        spawn_point_index: 12,
        spawn_point_token: 99,
        active: true,
        pose: pose(20.),
    }
}
fn effects(events: &[LifecycleEvent]) -> Vec<&Effect> {
    events.iter().map(|v| &v.effect).collect()
}

#[test]
fn hidden_authority_respawns_in_place_without_changing_incarnation_or_double_offset() {
    let mut r = runtime(0);
    let key = r.get(4).unwrap().key;
    assert_eq!(r.get(4).unwrap().body, Body::Inactive);
    r.enable_cycle(key, 0x0c800004).unwrap();
    let out = r.respawn_existing(key, source(), false).unwrap();
    let v = r.get(4).unwrap();
    assert_eq!(v.key, key);
    assert_eq!(v.pose, pose(20.));
    assert_eq!(v.state, 2);
    assert_eq!(v.body, Body::Inactive);
    assert_eq!(v.spawn_counter, 1);
    assert_eq!(
        effects(&out),
        vec![
            &Effect::DeactivateHulk,
            &Effect::ForcePosition(pose(20.).position),
            &Effect::ForceRotation(pose(20.).rotation),
            &Effect::DamageReset,
            &Effect::StateChanged(2),
            &Effect::KillSpawnBlockers,
            &Effect::SpawnCounterIncremented(1)
        ]
    );
    assert_eq!(
        effects(&r.update(key, 0., 20000).unwrap()),
        vec![&Effect::AddPhysics]
    );
    assert_eq!(r.get(4).unwrap().body, Body::Operational);
}
#[test]
fn healthy_noop_still_cleans_hulk_and_counts_but_does_not_move_reset_or_clear_blockers() {
    let mut r = runtime(2);
    let key = r.get(4).unwrap().key;
    let out = r.respawn_existing(key, source(), false).unwrap();
    assert_eq!(r.get(4).unwrap().pose, pose(2.));
    assert_eq!(
        effects(&out),
        vec![&Effect::DeactivateHulk, &Effect::SpawnCounterIncremented(1)]
    );
    r.enable_cycle(key, 0x0c800004).unwrap();
    r.disable_cycle(key).unwrap();
    assert!(!r.get(4).unwrap().spawn_cycle_enabled);
    assert!(!r.get(4).unwrap().spawning);
    assert_eq!(r.get(4).unwrap().associated_objective, Some(0x0c800004));
}
#[test]
fn lethal_is_pending_and_update_emits_ordered_effects_only_once() {
    let mut r = runtime(2);
    let key = r.get(4).unwrap().key;
    r.configure(key, capabilities()).unwrap();
    r.enable_cycle(key, 0x0c800004).unwrap();
    r.apply_lethal(key, 0x03800007, true).unwrap();
    let v = r.get(4).unwrap();
    assert_eq!(v.health_quarter_points, 0);
    assert_eq!(v.state, 2);
    assert_eq!(v.body, Body::Operational);
    let out = r.update(key, 0.1, 20000).unwrap();
    assert_eq!(
        effects(&out),
        vec![
            &Effect::StateChanged(5),
            &Effect::VehicleDestroyed {
                attacker: 0x03800007
            },
            &Effect::KillOccupants {
                attacker: 0x03800007,
                position: pose(2.).position
            },
            &Effect::DestructionExplosion(capabilities().destruction.unwrap()),
            &Effect::RequestFspDeactivation {
                objective: 0x0c800004,
                attacker: 0x03800007,
                forced: true
            },
            &Effect::DismountOccupants,
            &Effect::RemovePhysics,
            &Effect::SpawnDelaySet(1)
        ]
    );
    assert_eq!(r.get(4).unwrap().body, Body::Inactive);
    assert!(r.update(key, 0.1, 20000).unwrap().is_empty());
    assert_eq!(r.get(4).unwrap().spawn_delay_counter, 1);
    assert!(r.update(key, 1000., 60000).unwrap().is_empty());
    assert_eq!(r.get(4).unwrap().spawn_delay_counter, 0);
    // A type9 Escort never enters the generic reinforcement queue.
    assert!(r.get(4).unwrap().spawn_cycle_enabled);
}
#[test]
fn ordinary_death_needs_explicit_hulk_and_missing_effects_leave_authority_unchanged() {
    let mut r = runtime_with_source(2, 7);
    let key = r.get(4).unwrap().key;
    r.apply_lethal(key, u32::MAX, false).unwrap();
    let before = r.get(4).unwrap().clone();
    assert_eq!(
        r.update(key, 0.1, 20000),
        Err("ground_destruction_properties_unavailable")
    );
    assert_eq!(r.get(4).unwrap(), &before);
    let mut c = capabilities();
    c.hulk_template = None;
    r.configure(key, c).unwrap();
    assert_eq!(
        r.update(key, 0.1, 20000),
        Err("ground_hulk_template_unavailable")
    );
    r.configure(key, capabilities()).unwrap();
    r.enable_cycle(key, 0x0c800004).unwrap();
    let out = r.update(key, 0.1, 20000).unwrap();
    assert!(matches!(
        out[out.len() - 2].effect,
        Effect::ActivateHulk { .. }
    ));
    assert_eq!(
        r.get(4).unwrap().body,
        Body::Hulk {
            template: "synthetic-hulk".into()
        }
    );
}
#[test]
fn invalid_source_and_generation_are_rejected_without_counting() {
    let mut r = runtime(0);
    let key = r.get(4).unwrap().key;
    let mut input = source();
    input.active = false;
    assert_eq!(
        r.respawn_existing(key, input, false),
        Err("ground_respawn_source_inactive")
    );
    let mut input = source();
    input.spawn_point_token += 1;
    assert_eq!(
        r.respawn_existing(key, input, false),
        Err("ground_respawn_source_mismatch")
    );
    assert_eq!(r.get(4).unwrap().spawn_counter, 0);
    r.reset().unwrap();
    assert_eq!(r.get(4).unwrap().key.generation, 2);
    assert_eq!(
        r.apply_lethal(key, u32::MAX, true),
        Err("stale_ground_vehicle_generation")
    );
}
#[test]
fn delivered_timer_crossing_is_state_seven_and_not_a_respawn_timer() {
    let mut r = runtime(2);
    let key = r.get(4).unwrap().key;
    assert_eq!(r.deliver(key), Err("ground_delivery_timer_unavailable"));
    r.configure(key, capabilities()).unwrap();
    assert_eq!(
        effects(&r.deliver(key).unwrap()),
        vec![&Effect::StateChanged(8)]
    );
    assert!(r.update(key, 11., 20000).unwrap().is_empty());
    assert_eq!(r.get(4).unwrap().state, 8);
    assert_eq!(
        effects(&r.update(key, 1., 20000).unwrap()),
        vec![&Effect::StateChanged(7), &Effect::RemovePhysics]
    );
    assert_eq!(r.get(4).unwrap().state_age_seconds, 0.);
    assert!(r.update(key, 1., 20000).unwrap().is_empty());
}

#[test]
fn type_nine_ordinary_death_invokes_no_hulk_world_activation() {
    let mut r = runtime(2);
    let key = r.get(4).unwrap().key;
    let mut properties = capabilities();
    properties.hulk_template = None;
    r.configure(key, properties).unwrap();
    r.enable_cycle(key, 0x0c800004).unwrap();
    r.apply_lethal(key, u32::MAX, false).unwrap();
    let events = r.update(key, 0.1, 20000).unwrap();
    assert_eq!(r.get(4).unwrap().state, 4);
    assert_eq!(r.get(4).unwrap().body, Body::Inactive);
    assert!(
        !events
            .iter()
            .any(|event| matches!(event.effect, Effect::ActivateHulk { .. }))
    );
}
#[test]
fn nonenumerated_dead_respawn_resets_damage_and_pose_but_preserves_delivered_state() {
    let mut r = runtime(2);
    let key = r.get(4).unwrap().key;
    r.configure(key, capabilities()).unwrap();
    r.deliver(key).unwrap();
    r.apply_lethal(key, 0x03800001, true).unwrap();
    let initial_noop = r.respawn_existing(key, source(), true).unwrap();
    assert_eq!(
        effects(&initial_noop),
        vec![&Effect::DeactivateHulk, &Effect::SpawnCounterIncremented(1)]
    );
    assert_eq!(r.get(4).unwrap().health_quarter_points, 0);
    let events = r.respawn_existing(key, source(), false).unwrap();
    assert_eq!(r.get(4).unwrap().state, 8);
    assert_eq!(r.get(4).unwrap().health_quarter_points, 800);
    assert_eq!(r.get(4).unwrap().last_attacker, u32::MAX);
    assert!(!r.get(4).unwrap().vaporize_on_death);
    assert!(
        !events
            .iter()
            .any(|event| matches!(event.effect, Effect::StateChanged(_)))
    );
}

#[test]
fn delivery_rejects_hidden_or_dead_authority_even_with_timer_properties() {
    let mut r = runtime(0);
    let key = r.get(4).unwrap().key;
    r.configure(key, capabilities()).unwrap();
    assert_eq!(
        r.deliver(key),
        Err("ground_delivery_requires_operational_vehicle")
    );
    r.respawn_existing(key, source(), false).unwrap();
    assert_eq!(
        r.deliver(key),
        Err("ground_delivery_requires_operational_vehicle")
    );
    r.update(key, 0., 20000).unwrap();
    r.apply_lethal(key, u32::MAX, true).unwrap();
    assert_eq!(
        r.deliver(key),
        Err("ground_delivery_requires_operational_vehicle")
    );
}
