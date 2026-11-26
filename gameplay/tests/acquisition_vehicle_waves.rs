use mag_gameplay::{acquisition_ground_runtime::Key, acquisition_vehicle_waves::VehicleWaves};

fn vehicle(index: u16) -> Key {
    Key {
        index,
        generation: 1,
    }
}

#[test]
fn vehicle_clock_uses_interval_for_first_wave_and_never_drains_missed_waves() {
    let mut waves = VehicleWaves::start(1000, [20_000, 40_000]).unwrap();
    assert_eq!(waves.next_timestamp(0), Ok(21_000));
    assert_eq!(waves.next_timestamp(1), Ok(41_000));
    assert_eq!(waves.tick(20_999), [None, None]);
    assert_eq!(waves.tick(21_000), [Some(vec![]), None]);
    assert_eq!(waves.next_timestamp(0), Ok(41_000));
    assert_eq!(waves.tick(81_000), [Some(vec![]), Some(vec![])]);
    assert_eq!(waves.next_timestamp(0), Ok(101_000));
    assert_eq!(waves.next_timestamp(1), Ok(121_000));
    assert_eq!(waves.tick(81_001), [None, None]);
}

#[test]
fn queue_keeps_first_insertion_and_waits_for_the_following_wave() {
    let mut waves = VehicleWaves::start(0, [20_000; 2]).unwrap();
    // Ground update only observes the advanced timestamp after this wave.
    assert_eq!(waves.tick(20_000), [Some(vec![]), Some(vec![])]);
    waves.queue(0, vehicle(3)).unwrap();
    waves.queue(0, vehicle(1)).unwrap();
    waves.queue(0, vehicle(3)).unwrap();
    waves.queue(1, vehicle(8)).unwrap();
    assert!(waves.queue(1, vehicle(1)).is_err());
    assert_eq!(waves.tick(39_999), [None, None]);
    assert_eq!(
        waves.tick(40_000),
        [Some(vec![vehicle(3), vehicle(1)]), Some(vec![vehicle(8)])]
    );
    assert_eq!(waves.tick(60_000), [Some(vec![]), Some(vec![])]);
}

#[test]
fn interval_changes_take_effect_only_when_the_advertised_wave_renews() {
    let mut waves = VehicleWaves::start(1_000, [20_000, 40_000]).unwrap();
    waves.queue(0, vehicle(3)).unwrap();
    waves.set_intervals_ms([10_000, 60_000]).unwrap();
    assert_eq!(waves.next_timestamp(0), Ok(21_000));
    assert_eq!(waves.next_timestamp(1), Ok(41_000));
    let before = waves.clone();
    assert!(waves.set_intervals_ms([15_000, 0]).is_err());
    assert_eq!(waves, before);

    assert_eq!(waves.tick(20_999), [None, None]);
    assert_eq!(waves.tick(21_000), [Some(vec![vehicle(3)]), None]);
    assert_eq!(waves.next_timestamp(0), Ok(31_000));
    assert_eq!(waves.next_timestamp(1), Ok(41_000));

    waves.set_intervals_ms([20_000, 40_000]).unwrap();
    assert_eq!(waves.tick(31_000), [Some(vec![]), None]);
    assert_eq!(waves.next_timestamp(0), Ok(51_000));
    assert_eq!(waves.tick(41_000), [None, Some(vec![])]);
    assert_eq!(waves.next_timestamp(1), Ok(81_000));
}

#[test]
fn cancel_withdraws_one_vehicle_from_whichever_team_queued_it() {
    let mut waves = VehicleWaves::start(0, [100, 100]).unwrap();
    waves.queue(0, vehicle(3)).unwrap();
    waves.queue(0, vehicle(1)).unwrap();
    waves.queue(1, vehicle(8)).unwrap();
    assert_eq!(waves.cancel(8), Some((1, vehicle(8))));
    assert_eq!(waves.cancel(8), None);
    assert_eq!(waves.cancel(3), Some((0, vehicle(3))));
    // The vehicle can queue again, and the remaining order is kept.
    waves.queue(1, vehicle(8)).unwrap();
    assert_eq!(
        waves.tick(100),
        [Some(vec![vehicle(1)]), Some(vec![vehicle(8)])]
    );
}
