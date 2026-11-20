use mag_gameplay::active_abilities::{Ability, Side};
use mag_gameplay::active_ability_effects::{ClusterRules, Effect, EffectRules, Work};
use mag_gameplay::active_ability_sensor::{Detector, ObjectiveCursor, SensorRules};

fn cluster(successful: bool) -> Effect {
    Effect::new(
        Ability::ClusterBomb,
        Side::Attacker,
        123,
        successful,
        [40., 2., 70.],
        EffectRules::Cluster(ClusterRules {
            delay_ms: 40,
            damage_delay_ms: 5,
            interval_ms: 13,
            count: 2,
            scatter_radius: 8.,
            damage: 17.,
            inner_radius: 2.,
            inner_scale: 0.,
            outer_radius: 6.,
            outer_scale: 0.,
        }),
    )
    .unwrap()
}

#[test]
fn cluster_returns_one_sample_per_update_even_when_both_are_due() {
    let effect = cluster(true);
    assert_eq!(effect.prepare_tick(167).unwrap().1, Work::Idle);
    let (next, first) = effect.prepare_tick(181).unwrap();
    let Work::ClusterSample(first) = first else {
        panic!("expected first bomblet");
    };
    assert_eq!(first.ordinal, 0);
    assert_eq!(first.blast.damage, 17.);
    assert_eq!(first.blast.inner_scale, 0.5);
    assert_eq!(first.blast.outer_scale, 0.0);
    // Independent synthetic fixture for start 123 and scatter radius 8.
    assert!((first.xz[0] - 40.002_05).abs() < 0.000_02);
    assert!((first.xz[1] - 70.003_26).abs() < 0.000_02);
    assert!(!next.is_complete());
    let (last, second) = next.prepare_tick(181).unwrap();
    let Work::ClusterSample(second) = second else {
        panic!("expected second bomblet");
    };
    assert_eq!(second.ordinal, 1);
    assert_ne!(first.xz, second.xz);
    assert!(last.is_complete());
    assert_eq!(last.prepare_tick(200).unwrap().1, Work::Complete);
}

#[test]
fn failed_cluster_consumes_each_due_slot_without_a_sample() {
    let effect = cluster(false);
    let (first, work) = effect.prepare_tick(200).unwrap();
    assert_eq!(work, Work::Idle);
    assert!(!first.is_complete());
    let (last, work) = first.prepare_tick(200).unwrap();
    assert_eq!(work, Work::Complete);
    assert!(last.is_complete());
}

fn sensor(side: Side, successful: bool) -> Effect {
    Effect::new(
        Ability::Sensor,
        side,
        50,
        successful,
        [1., 2., 3.],
        EffectRules::Sensor(SensorRules {
            delay_ms: 5,
            duration_ms: 30,
            attacker_radius: 6.,
            defender_radius: 9.,
            speed_threshold: 2.,
        }),
    )
    .unwrap()
}

#[test]
fn attacker_scans_one_squad_with_strict_start_and_inclusive_end() {
    let effect = sensor(Side::Attacker, true);
    assert_eq!(effect.start_work(), None);
    assert_eq!(effect.prepare_tick(55).unwrap().1, Work::Idle);
    let mut current = effect.prepare_tick(55).unwrap().0;
    for expected in (16..32).chain([16]) {
        let (next, work) = current.prepare_tick(56).unwrap();
        let Work::ScanSquad(scan) = work else {
            panic!("expected bounded squad scan");
        };
        assert_eq!(scan.slot, expected);
        assert_eq!(scan.origin, [1., 2., 3.]);
        current = next;
    }
    assert!(matches!(
        current.prepare_tick(85).unwrap().1,
        Work::ScanSquad(_)
    ));
    assert_eq!(current.prepare_tick(86).unwrap().1, Work::Complete);
    let failed = sensor(Side::Attacker, false);
    assert_eq!(failed.prepare_tick(56).unwrap().1, Work::Idle);
    assert_eq!(failed.prepare_tick(86).unwrap().1, Work::Complete);
}

#[test]
fn defender_surveillance_starts_immediately_and_clears_after_duration() {
    let effect = sensor(Side::Defender, true);
    assert_eq!(effect.start_work().unwrap().detector.radius(), 9.);
    assert_eq!(effect.prepare_tick(80).unwrap().1, Work::Idle);
    let (next, work) = effect.prepare_tick(81).unwrap();
    assert!(next.is_complete());
    assert_eq!(work, Work::ClearSurveillance);
    assert_eq!(next.prepare_tick(82).unwrap().1, Work::Complete);
    assert_eq!(sensor(Side::Defender, false).start_work(), None);
}

#[test]
fn detector_uses_strict_three_dimensional_radius_and_inclusive_speed() {
    let detector = Detector::new(4., 2.).unwrap();
    let origin = [5., 5., 5.];
    assert!(detector.detects(origin, [5., 8., 5.], 2.));
    assert!(!detector.detects(origin, [5., 9., 5.], 2.));
    assert!(!detector.detects(origin, [5., 8., 5.], 1.9));
    assert!(!detector.detects(origin, [f32::NAN, 8., 5.], 2.));
    assert!(!detector.detects(origin, [5., 8., 5.], f32::INFINITY));
}

#[test]
fn each_objective_cursor_limits_slots_and_wraps_after_the_tail() {
    let mut cursor = ObjectiveCursor::default();
    assert_eq!(cursor.take(9), 0..4);
    assert_eq!(cursor.take(9), 4..8);
    assert_eq!(cursor.take(9), 8..9);
    assert_eq!(cursor.take(9), 0..4);
    assert_eq!(cursor.take(1), 0..1);
    assert_eq!(cursor.take(0), 0..0);
}

#[test]
fn defender_surveillance_scans_primary_secondary_and_emplacement_objectives() {
    use mag_gameplay::active_ability_sensor::objective_supports_surveillance;

    assert!(objective_supports_surveillance(0, 2));
    assert!(objective_supports_surveillance(3, 7));
    assert!(objective_supports_surveillance(4, 3));
    assert!(!objective_supports_surveillance(4, 7));
}

#[test]
fn repeated_activations_keep_the_original_instance_progress_where_native_does() {
    let (cluster, _) = cluster(true).prepare_tick(200).unwrap();
    let (cluster, _) = cluster.prepare_tick(200).unwrap();
    let cluster_rules = EffectRules::Cluster(ClusterRules {
        delay_ms: 40,
        damage_delay_ms: 5,
        interval_ms: 13,
        count: 2,
        scatter_radius: 8.,
        damage: 17.,
        inner_radius: 2.,
        inner_scale: 0.,
        outer_radius: 6.,
        outer_scale: 0.,
    });
    let failed = cluster
        .rearm(
            Ability::ClusterBomb,
            Side::Attacker,
            400,
            false,
            [40., 2., 70.],
            cluster_rules,
        )
        .unwrap();
    assert_eq!(failed.prepare_tick(400).unwrap().1, Work::Complete);
    let successful = failed
        .rearm(
            Ability::ClusterBomb,
            Side::Attacker,
            500,
            true,
            [40., 2., 70.],
            cluster_rules,
        )
        .unwrap();
    assert!(matches!(
        successful.prepare_tick(545).unwrap().1,
        Work::ClusterSample(sample) if sample.ordinal == 0
    ));

    let (sensor, _) = sensor(Side::Attacker, true).prepare_tick(56).unwrap();
    let (sensor, _) = sensor.prepare_tick(86).unwrap();
    let resumed = sensor
        .rearm(
            Ability::Sensor,
            Side::Attacker,
            100,
            true,
            [1., 2., 3.],
            EffectRules::Sensor(SensorRules {
                delay_ms: 5,
                duration_ms: 30,
                attacker_radius: 6.,
                defender_radius: 9.,
                speed_threshold: 2.,
            }),
        )
        .unwrap();
    assert!(matches!(
        resumed.prepare_tick(106).unwrap().1,
        Work::ScanSquad(scan) if scan.slot == 17
    ));
}
