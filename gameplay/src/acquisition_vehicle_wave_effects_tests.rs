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

fn ground_vehicle(engine: &Engine<Vec<u8>>, index: u16) -> ground::Vehicle {
    engine
        .ground_runtime
        .as_ref()
        .unwrap()
        .get(index)
        .unwrap()
        .clone()
}

fn waves(engine: &Engine<Vec<u8>>) -> crate::acquisition_vehicle_waves::VehicleWaves {
    engine
        .acquisition_gameplay
        .as_ref()
        .unwrap()
        .vehicle_waves
        .clone()
        .unwrap()
}

fn log_ops(engine: &Engine<Vec<u8>>, op: &str) -> Vec<Value> {
    let log = String::from_utf8(engine.writer.clone()).unwrap();
    let mut rows = Vec::new();
    fn collect(value: &Value, op: &str, rows: &mut Vec<Value>) {
        match value {
            Value::Object(map) => {
                if map.get("op").and_then(Value::as_str) == Some(op) {
                    rows.push(value.clone());
                }
                map.values().for_each(|v| collect(v, op, rows));
            }
            Value::Array(items) => items.iter().for_each(|v| collect(v, op, rows)),
            _ => {}
        }
    }
    for line in log.lines() {
        if let Ok(value) = serde_json::from_str::<Value>(line) {
            collect(&value, op, &mut rows);
        }
    }
    rows
}

/// The motor-pool truck (vehicle 8, spawn point 21) respawned in place by
/// the spawner's own activation, the way a repaired motor pool does.
fn activation_respawn(engine: &mut Engine<Vec<u8>>) {
    let creation = engine
        .ground_vehicles
        .as_ref()
        .unwrap()
        .vehicle(8)
        .unwrap()
        .clone();
    let runtime = engine.ground_runtime.as_mut().unwrap();
    let key = runtime.get(8).unwrap().key;
    runtime
        .respawn_existing(
            key,
            ground::QualifiedRespawn {
                spawn_point_index: 21,
                spawn_point_token: 321,
                active: true,
                pose: ground::Pose {
                    position: creation.position,
                    rotation: creation.rotation,
                },
            },
            false,
        )
        .unwrap();
}

/// SYNTHETIC regression for the frozen Domination world: a vehicle queued
/// in its team wave that something else respawned before the wave was due
/// made every later world tick fail with a stale entry, forever, because the
/// failed tick rolled the drained queue back. The wave now drops the entry
/// once with a warning row and the world keeps advancing.
#[test]
fn a_queued_vehicle_respawned_before_its_wave_is_dropped_and_the_world_keeps_ticking() {
    let (mut engine, first, second) = wave_fixture();
    engine.tick_acquisition_objectives(first, 2).unwrap();
    assert_eq!(ground_vehicle(&engine, 8).state, 6);
    activation_respawn(&mut engine);
    let truck = ground_vehicle(&engine, 8);
    assert_eq!((truck.state, truck.spawn_counter), (2, 1));

    let interval = second - first;
    engine.tick_acquisition_objectives(second, 3).unwrap();
    let stale = log_ops(&engine, "acquisition_vehicle_wave_stale_entry");
    assert_eq!(stale.len(), 1, "{stale:?}");
    assert_eq!(stale[0]["vehicle"], 0x0100_0008);
    assert_eq!(stale[0]["state"], 2);
    assert_eq!(stale[0]["action"], "dropped");
    // The truck was not respawned a second time; the APC in the same wave was.
    let truck = ground_vehicle(&engine, 8);
    assert_eq!((truck.state, truck.spawn_counter), (2, 1));
    assert_eq!(ground_vehicle(&engine, 0).state, 2);
    assert_eq!(
        waves(&engine).next_timestamp(0),
        Ok((second + interval).as_millis() as u32)
    );
    // Later ticks and waves keep advancing, with no repeat warning.
    for step in 1..=3_u32 {
        engine
            .tick_acquisition_objectives(second + interval * step, 3 + u64::from(step))
            .unwrap();
    }
    assert_eq!(
        log_ops(&engine, "acquisition_vehicle_wave_stale_entry").len(),
        1
    );

    // Destroyed again, the truck requeues itself and returns at a later wave.
    let key = ground_vehicle(&engine, 8).key;
    engine
        .ground_runtime
        .as_mut()
        .unwrap()
        .apply_lethal(key, u32::MAX, true)
        .unwrap();
    let mut now = second + interval * 3;
    let mut returned = false;
    for step in 0..6 {
        now += interval;
        engine.tick_acquisition_objectives(now, 10 + step).unwrap();
        let truck = ground_vehicle(&engine, 8);
        if truck.state == 2 && !truck.is_dead() {
            assert_eq!(truck.key, key);
            assert_eq!(truck.body, ground::Body::Operational);
            returned = true;
            break;
        }
    }
    assert!(returned, "{:?}", ground_vehicle(&engine, 8));
    assert_eq!(
        log_ops(&engine, "acquisition_vehicle_wave_stale_entry").len(),
        1
    );
}

/// SYNTHETIC: the motor-pool spawner is destroyed (FSP deactivation) while
/// its truck waits in the wave, then repaired (FSP activation). The repair
/// respawns the truck directly and cancels its wave entry, so the wave never
/// meets a stale entry, and the truck serves on after the repair.
#[test]
fn motor_pool_destroy_and_repair_cancels_the_queued_wave_entry() {
    use crate::{
        acquisition_structural::Frame,
        domination_fsp::{SpawnIdentity, VehicleHook, VehicleRequest},
    };
    let (mut engine, first, second) = wave_fixture();
    engine.tick_acquisition_objectives(first, 2).unwrap();
    assert_eq!(ground_vehicle(&engine, 8).state, 6);
    let setup = engine.acquisition_gameplay.as_ref().unwrap().setup.clone();
    let stage = |engine: &mut Engine<Vec<u8>>, hook| {
        let mut ground = engine.ground_runtime.clone().unwrap();
        let mut spawns = engine.spawn_catalog.clone().unwrap();
        let mut queued = waves(engine);
        let mut native = engine.owned_acquisition_characters();
        let mut audit = Vec::new();
        let mut frame = Frame {
            engine,
            assignments: &[],
            ground: &mut ground,
            spawns: &mut spawns,
            native: &mut native,
            ground_changes: &mut BTreeMap::new(),
            audit: &mut audit,
            messages: &mut Vec::new(),
            sequences: &mut BTreeMap::new(),
            filter_rows: &setup.filter_rows,
            physics: setup.physics,
            ground_policy: None,
            hospitals: &mut None,
            hospital_packets: &mut Vec::new(),
            vehicle_waves: Some(&mut queued),
            now: first,
        };
        frame
            .stage_vehicle(
                VehicleRequest {
                    point: SpawnIdentity {
                        token: 321,
                        native_id: 21,
                    },
                    objective_token: 7,
                    hook,
                },
                7,
            )
            .unwrap();
        engine.ground_runtime = Some(ground);
        engine.acquisition_gameplay.as_mut().unwrap().vehicle_waves = Some(queued);
        audit
    };
    // Destroyed: the cycle stops and the truck keeps its place in the wave.
    let audit = stage(&mut engine, VehicleHook::OnFspDeactivate);
    assert!(audit.is_empty(), "{audit:?}");
    assert_eq!(ground_vehicle(&engine, 8).state, 6);
    assert_eq!(waves(&engine).cancel(8).map(|(team, _)| team), Some(0));
    // Repaired: the activation respawns the truck and withdraws its entry.
    let audit = stage(&mut engine, VehicleHook::OnFspActivate);
    assert!(
        audit.iter().any(|row| row["op"]
            == "acquisition_structural_motor_pool_wave_entry_cancelled"
            && row["vehicle"] == 0x0100_0008),
        "{audit:?}"
    );
    assert_eq!(waves(&engine).cancel(8), None);
    let truck = ground_vehicle(&engine, 8);
    assert_eq!((truck.state, truck.spawn_counter), (2, 1));

    let interval = second - first;
    for step in 0..3_u32 {
        engine
            .tick_acquisition_objectives(second + interval * step, 3 + u64::from(step))
            .unwrap();
    }
    assert!(log_ops(&engine, "acquisition_vehicle_wave_stale_entry").is_empty());
    let truck = ground_vehicle(&engine, 8);
    assert_eq!((truck.state, truck.spawn_counter), (2, 1));
    assert_eq!(truck.body, ground::Body::Operational);
    assert_eq!(ground_vehicle(&engine, 0).state, 2);
}
