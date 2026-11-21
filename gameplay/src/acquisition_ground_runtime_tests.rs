use super::*;

const ATTACKER: u32 = 0x03800007;
const GRENADE: u32 = 0x04000002;

fn runtime(state: u8, source_type: u8, maximum_quarter_points: u16) -> (Runtime, Key) {
    let creation = serde_json::from_value(serde_json::json!({
        "index": 4,
        "spawn_point_index": 12,
        "spawn_point_token": 99,
        "vehicle_type_token": 5,
        "faction": 0,
        "team": 0,
        "modifier": 0,
        "position": {"x": 2.0, "y": 4.0, "z": 8.0},
        "rotation": [0.0, 0.0, 0.0, 1.0],
        "health_quarter_points": maximum_quarter_points,
        "initial_state": state,
    }))
    .unwrap();
    let runtime = Runtime::from_creation(&creation, Some(source_type)).unwrap();
    let key = runtime.get(4).unwrap().key;
    (runtime, key)
}

#[test]
fn rocket_damage_keeps_area_direction_and_native_source_sentinel() {
    let (mut runtime, key) = runtime(2, 3, 600);
    runtime
        .apply_grenade_damage(key, 30.0, ATTACKER, GRENADE)
        .unwrap();
    let before = runtime.get(4).unwrap();
    assert_eq!(before.damage_wire().last_source_object, GRENADE);
    assert!(
        runtime
            .apply_classified_vehicle_blast_damage(key, 120.0, ATTACKER, 0, 0x1234)
            .is_err()
    );
    assert_eq!(runtime.get(4).unwrap().health_quarter_points, 480);
    runtime
        .apply_classified_vehicle_blast_damage(key, 120.0, ATTACKER, 5, 0x1234)
        .unwrap();
    let vehicle = runtime.get(4).unwrap();
    let damage = vehicle.damage_wire();
    assert_eq!(damage.health_quarter_points, 0);
    assert_eq!(damage.last_location_and_type, 0xa5);
    assert_eq!(damage.last_attacker, ATTACKER);
    assert_eq!(damage.last_source_object, u32::MAX);
    assert_eq!(damage.last_user_data, 0x1234);
    assert_eq!(vehicle.body, Body::Operational);
    assert!(vehicle.pending_death);
}

#[test]
fn heal_percent_accumulates_float_hp_and_caps_at_loaded_maximum() {
    // This ground operation also applies to operational non-Escort models.
    let (mut runtime, key) = runtime(2, 7, 600);
    runtime.apply_damage(key, 40, ATTACKER).unwrap();

    let first = runtime.heal_percent(key, 0.125).unwrap();
    assert_eq!(
        first,
        [LifecycleEvent {
            vehicle: key,
            effect: Effect::DamageChanged {
                health_quarter_points: 560,
                last_attacker: ATTACKER,
            },
        }]
    );
    assert_eq!(runtime.get(4).unwrap().health_hp, 140.1875);

    runtime.heal_percent(key, 0.125).unwrap();
    assert_eq!(runtime.get(4).unwrap().health_quarter_points, 561);
    assert_eq!(runtime.get(4).unwrap().health_hp, 140.375);

    runtime.heal_percent(key, f32::MAX).unwrap();
    assert_eq!(runtime.get(4).unwrap().health_hp, 150.0);
    assert_eq!(runtime.get(4).unwrap().health_quarter_points, 600);
    assert!(runtime.heal_percent(key, 1.5).unwrap().is_empty());
}

#[test]
fn heal_percent_preserves_bullet_metadata_and_ground_authority() {
    let (mut runtime, key) = runtime(2, 9, 600);
    runtime.enable_cycle(key, 0x0c800004).unwrap();
    runtime.begin_driver(4, 10).unwrap();
    runtime
        .apply_bullet_damage(key, 91.5, ATTACKER, 3, 0x1234)
        .unwrap();
    let before = runtime.get(4).unwrap().clone();
    assert_eq!(before.damage_wire().conditions, 3);

    // 58.5 HP + 1% of 150 HP is exactly the 40% boundary.
    runtime.heal_percent(key, 1.0).unwrap();
    let after = runtime.get(4).unwrap();
    let damage = after.damage_wire();
    assert_eq!(damage.conditions, 1);
    assert_eq!(damage.last_location_and_type, 0x13);
    assert_eq!(damage.last_attacker, ATTACKER);
    assert_eq!(damage.last_source_object, u32::MAX);
    assert_eq!(damage.last_user_data, 0x1234);
    assert_eq!(after.pose, before.pose);
    assert_eq!(after.state, before.state);
    assert_eq!(after.state_age_seconds, before.state_age_seconds);
    assert_eq!(after.body, before.body);
    assert_eq!(after.associated_objective, before.associated_objective);
    assert_eq!(after.spawn_counter, before.spawn_counter);
    assert_eq!(after.driver_position_at_ms, before.driver_position_at_ms);
    assert_eq!(
        after.driver_position_credit_m,
        before.driver_position_credit_m
    );

    // A second heal clears the light bit at exactly 70%.
    runtime.heal_percent(key, 30.0).unwrap();
    assert_eq!(runtime.get(4).unwrap().damage_wire().conditions, 0);
}

#[test]
fn heal_percent_retains_the_grenade_source() {
    let (mut runtime, key) = runtime(2, 9, 600);
    runtime
        .apply_grenade_damage(key, 50.0, ATTACKER, GRENADE)
        .unwrap();
    runtime.heal_percent(key, 1.5).unwrap();

    let damage = runtime.get(4).unwrap().damage_wire();
    assert_eq!(damage.health_quarter_points, 409);
    assert_eq!(damage.last_source_object, GRENADE);
}

#[test]
fn heal_percent_resolves_conditions_after_untyped_damage() {
    let (mut runtime, key) = runtime(2, 9, 600);
    runtime.apply_damage(key, 369, ATTACKER).unwrap();
    assert_eq!(runtime.get(4).unwrap().damage_wire().conditions, 0);
    runtime.heal_percent(key, 1.5).unwrap();

    let damage = runtime.get(4).unwrap().damage_wire();
    assert_eq!(damage.health_quarter_points, 240);
    assert_eq!(damage.conditions, 1);
    assert_eq!(damage.last_location_and_type, 0);
    assert_eq!(damage.last_attacker, ATTACKER);

    runtime.apply_damage(key, 1, ATTACKER).unwrap();
    assert_eq!(runtime.get(4).unwrap().damage_wire().conditions, 0);
}

#[test]
fn heal_percent_ignores_nonpositive_and_sub_epsilon_amounts() {
    let (mut runtime, key) = runtime(2, 9, 400);
    runtime.apply_damage(key, 40, ATTACKER).unwrap();
    let before = runtime.clone();
    for percent in [0.0, -1.0, 0.00025] {
        assert!(runtime.heal_percent(key, percent).unwrap().is_empty());
        assert_eq!(runtime, before);
    }

    // 0.0005% of 100 HP is exactly the model's 0.0005 HP threshold.
    assert_eq!(runtime.heal_percent(key, 0.0005).unwrap().len(), 1);
    assert!(runtime.get(4).unwrap().health_hp > 90.0);
}

#[test]
fn heal_percent_rejects_nonfinite_input_and_stale_generation() {
    let (mut runtime, key) = runtime(2, 9, 600);
    for percent in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert!(runtime.heal_percent(key, percent).is_err());
    }

    runtime.reset().unwrap();
    let before = runtime.clone();
    assert_eq!(
        runtime.heal_percent(key, 1.5),
        Err("stale_ground_vehicle_generation")
    );
    assert_eq!(runtime, before);
}

#[test]
fn heal_percent_skips_critical_dead_inactive_and_delivered_bodies() {
    let (mut critical, key) = runtime(2, 9, 600);
    critical.apply_damage(key, 511, ATTACKER).unwrap();
    let mut dying = critical.clone();
    dying.update(key, 0.0, 0).unwrap();
    assert_eq!(dying.get(4).unwrap().state, 3);

    let (mut dead, _) = runtime(2, 9, 600);
    dead.apply_lethal(key, ATTACKER, false).unwrap();

    let (mut inactive, _) = runtime(0, 9, 600);
    inactive.apply_damage(key, 40, ATTACKER).unwrap();
    let pose = inactive.get(4).unwrap().pose;
    inactive
        .respawn_existing(
            key,
            QualifiedRespawn {
                spawn_point_index: 12,
                spawn_point_token: 99,
                active: true,
                pose,
            },
            true,
        )
        .unwrap();
    assert_eq!(inactive.get(4).unwrap().state, 2);
    assert_eq!(inactive.get(4).unwrap().body, Body::Inactive);

    let (mut delivered, _) = runtime(2, 9, 600);
    delivered.apply_damage(key, 40, ATTACKER).unwrap();
    delivered
        .configure(
            key,
            Capabilities {
                delivered_inert_seconds: Some(12.0),
                ..Capabilities::default()
            },
        )
        .unwrap();
    delivered.deliver(key).unwrap();
    assert_eq!(delivered.get(4).unwrap().state, 8);
    assert_eq!(delivered.get(4).unwrap().body, Body::Operational);

    for mut runtime in [critical, dying, dead, inactive, delivered] {
        let before = runtime.clone();
        assert!(runtime.heal_percent(key, 1.5).unwrap().is_empty());
        assert_eq!(runtime, before);
    }
}
