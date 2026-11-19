use super::*;

fn wave_fixture() -> (Engine<Vec<u8>>, Duration, Duration) {
    let mut engine = capture_fixture_with_reinforcements(None, None, true);
    reserve(&mut engine, 0);
    let actor = engine.active.as_mut().unwrap();
    ready(actor);
    actor.life = CharacterLife::Alive {
        generation: 1,
        health_quarter_points: 400,
    };
    engine.match_started = true;
    let epoch = Duration::from_millis(100);
    engine.match_epoch = Some(epoch);
    for index in [0, 8] {
        let runtime = engine.ground_runtime.as_mut().unwrap();
        let key = runtime.get(index).unwrap().key;
        runtime
            .configure(
                key,
                ground::Capabilities {
                    destruction: Some(ground::Destruction {
                        damage: 0.,
                        full_radius: 8.,
                        half_radius: 7.,
                    }),
                    ..Default::default()
                },
            )
            .unwrap();
        runtime.apply_lethal(key, u32::MAX, true).unwrap();
    }
    engine.tick_acquisition_objectives(epoch, 1).unwrap();
    let interval = Duration::from_millis(u64::from(engine.match_profile().spawn_wave_ms[0]));
    (engine, epoch + interval, epoch + interval * 2)
}

#[test]
fn acquisition_vehicle_waves_queue_on_first_advance_and_restore_existing_apc_and_truck_on_second() {
    let (mut engine, first, second) = wave_fixture();
    let initial_keys = [0, 8].map(|index| {
        engine
            .ground_runtime
            .as_ref()
            .unwrap()
            .get(index)
            .unwrap()
            .key
    });
    for key in initial_keys {
        assert_eq!(
            engine
                .ground_runtime
                .as_ref()
                .unwrap()
                .get(key.index)
                .unwrap()
                .state,
            5
        );
    }
    engine.tick_acquisition_objectives(first, 2).unwrap();
    for key in initial_keys {
        assert_eq!(
            engine
                .ground_runtime
                .as_ref()
                .unwrap()
                .get(key.index)
                .unwrap()
                .state,
            6
        );
    }
    assert!(!engine.ground_vehicle_is_operational(0));
    let queued = engine.ground_runtime.as_ref().unwrap();
    let wire = ground_vehicles::encode_runtime_delta(
        engine.ground_vehicles.as_ref().unwrap(),
        queued,
        &[(0, 1 << 5)],
        2,
        0,
    )
    .unwrap();
    let mut bits = BitReader::with_profile(&wire, engine.wire_profile);
    bits.position = engine.wire_profile.header_bits();
    assert_eq!(bits.read(1), Ok(1));
    assert_eq!(bits.read(16), Ok(0));
    for _ in 0..5 {
        assert_eq!(bits.read(1), Ok(0));
    }
    assert_eq!(bits.read(1), Ok(1));
    assert_eq!(bits.read(8), Ok(6));
    assert!(engine.current_ground_batch(0, 2, 0).is_ok());
    assert_eq!(engine.current_ground_vehicle(0).unwrap().initial_state, 6);
    engine.tick_acquisition_objectives(second, 3).unwrap();
    for key in initial_keys {
        let vehicle = engine
            .ground_runtime
            .as_ref()
            .unwrap()
            .get(key.index)
            .unwrap();
        assert_eq!(vehicle.key, key);
        assert_eq!(vehicle.state, 2);
        assert_eq!(vehicle.body, ground::Body::Operational);
        assert_eq!(vehicle.health_quarter_points, 600);
        assert_eq!(vehicle.spawn_counter, 1);
    }
    assert!(engine.ground_vehicle_is_operational(0));
    assert_eq!(
        engine
            .ground_vehicles
            .as_ref()
            .unwrap()
            .passenger_vehicle(20)
            .unwrap()
            .index,
        0
    );
    let batch_count = engine.ground_vehicles.as_ref().unwrap().batch_count();
    let actor = engine.active.as_mut().unwrap();
    actor.claimed = Some(Claimed {
        wire: Vec::new(),
        np_identity: [0; 36],
        game_info: protocol::GameInfo {
            client_index: 0,
            level_name: "synthetic_arena".into(),
            start_time_ms: 0,
            attacking_faction: 1,
            defending_faction: 0,
            maximum_points: [1200, 1000],
            current_points: [1200, 1000],
            game_type: 1,
        },
    });
    actor.life = CharacterLife::AwaitingRespawn { generation: 1 };
    actor.pending_shared.clear();
    actor.ground_vehicle_batch = batch_count;
    actor.objective_assigned = true;
    actor.objective_needs_clear = false;
    actor.selected_spawn = Some(protocol::CharacterSpawnSelection {
        spawn_point_index: 20,
    });
    actor.pending_spawn_deadline = Some(second);
    owner_update(&mut engine, second.as_millis() as u64 + 1, 0);
    let actor = engine.active.as_ref().unwrap();
    assert!(actor.life.is_alive());
    assert_eq!(actor.ground_seat.unwrap().vehicle_index, 0);
    engine.reset_objective_world();
    assert!(
        engine
            .acquisition_gameplay
            .as_ref()
            .unwrap()
            .vehicle_waves
            .is_none()
    );
    let fresh_epoch = second + Duration::from_secs(5);
    engine.match_epoch = Some(fresh_epoch);
    engine.tick_acquisition_objectives(fresh_epoch, 4).unwrap();
    let waves = engine
        .acquisition_gameplay
        .as_ref()
        .unwrap()
        .vehicle_waves
        .as_ref()
        .unwrap();
    assert_eq!(
        waves.next_timestamp(0),
        Ok((fresh_epoch + (second - first)).as_millis() as u32)
    );
    assert_eq!(
        engine
            .ground_runtime
            .as_ref()
            .unwrap()
            .get(0)
            .unwrap()
            .spawn_counter,
        0
    );
}

#[test]
fn acquisition_vehicle_wave_failed_record_preserves_queued_ground_and_clock() {
    let (mut engine, first, second) = wave_fixture();
    engine.tick_acquisition_objectives(first, 2).unwrap();
    let ground = engine.ground_runtime.clone();
    let waves = engine
        .acquisition_gameplay
        .as_ref()
        .unwrap()
        .vehicle_waves
        .clone();
    engine.max_log = engine.written;
    assert_eq!(
        engine.tick_acquisition_objectives(second, 3).err(),
        Some("log_limit_reached")
    );
    assert_eq!(engine.ground_runtime, ground);
    assert_eq!(
        engine.acquisition_gameplay.as_ref().unwrap().vehicle_waves,
        waves
    );
}
