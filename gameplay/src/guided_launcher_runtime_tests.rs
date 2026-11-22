//! Native-wire interoperability checks over the existing typed projectile world.
use super::*;
use crate::spawn_progression::dispatch;
fn fixture() -> Engine<Vec<u8>> {
    let mut engine = super::tests::fixture();
    let variants = std::array::from_fn(|f| {
        let base = engine.infantry_rockets.as_ref().unwrap().variants()[f];
        crate::guided_launchers::Variant {
            faction: base.faction,
            item_id: base.item_id + 10,
            stock_sight_item_id: base.stock_sight_item_id + 10,
            ammo_token: base.ammo_token,
            weapon: infantry_rockets::Weapon {
                initial_magazine_count: 3,
                ..base.weapon
            },
            flight: infantry_rockets::Flight {
                radius_m: 0.05,
                capsule_spine_local_xyz: [0., 0., 0.05],
                speed_mps: 60.,
                armed_time_seconds: 0.1,
                auto_explode_distance_m: 150.,
                lifetime_seconds: 10.,
                age_grace_seconds: 2.,
            },
            blast: base.blast,
            steering: crate::guided_launcher_motion::Steering {
                guided_decision_min_seconds: 0.01,
                guided_decision_max_seconds: 0.1,
                guided_max_direction_change: 0.07,
                dumb_decision_min_seconds: 0.01,
                dumb_decision_max_seconds: 0.01,
                dumb_direction_change_min: 0.005,
                dumb_direction_change_max: 0.005,
                dumb_cruise_range_m: 2.,
                dumb_max_range_m: 150.,
            },
        }
    });
    let actor = engine.active.as_mut().unwrap();
    let v = variants[usize::from(actor.reservation.character.faction)];
    let kit = actor.spawned_loadout.as_mut().unwrap();
    kit.inventories[usize::from(kit.handle.inventory_index)].slots[3] = Some(v.item_id);
    kit.inventories[usize::from(kit.handle.inventory_index)].slots[21] =
        Some(v.stock_sight_item_id);
    actor.reservation.character.skills = Some([1 << 25, 0, 0, 0, 0]);
    engine.guided_launchers = Some(Arc::new(crate::guided_launchers::Catalog::synthetic(
        variants,
    )));
    engine
}

#[test]
fn rocket_death_guided_and_dumb_keep_the_guided_launch_item() {
    for target in [u32::MAX, 0x0180_0000] {
        let mut engine = fixture();
        tests::lethal_target(&mut engine);
        let request = wire(&engine, 3, target);
        assert!(!dispatch(&mut engine, &request, 1000).unwrap().is_empty());
        let flight = engine.active.as_ref().unwrap().infantry_rocket_state.slots[3]
            .as_ref()
            .unwrap()
            .flight
            .clone();
        assert_eq!(flight.source.item, 1099);
        engine
            .finish_infantry_rocket(
                &flight,
                [1.25, 2.5, 1.25],
                "contact",
                None,
                Duration::from_millis(1500),
                1,
            )
            .unwrap();
        tests::assert_rocket_death(&engine, &[1, 255, 0, 0, 0, 1, 2, 0, 0, 4, 75, 0]);
    }
}
fn wire(engine: &Engine<Vec<u8>>, slot: u16, target: u32) -> Vec<u8> {
    let player = engine.active.as_ref().unwrap().reservation.player;
    let mut bits = replication::BitWriter::with_profile(engine.wire_profile);
    bits.write(0x0210, 16).unwrap();
    for v in [1.25_f32, 2.5, 0.125, 0., 0., 0., 1., 0., 0., 0.] {
        bits.write(v.to_bits(), 32).unwrap();
    }
    for (v, n) in [
        (3, 8),
        (511, 10),
        (target, 32),
        (0x0380_0000 | u32::from(player), 32),
        (u32::from(player), 16),
        (u32::from(slot), 16),
        (0, 6),
    ] {
        bits.write(v, n).unwrap();
    }
    bits.finish()
}
fn aim(
    engine: &mut Engine<Vec<u8>>,
    target: u32,
    state: u8,
    now: u64,
) -> Result<Vec<OutboundMessage>, &'static str> {
    let mut bytes = vec![2, 0x29];
    bytes.extend(target.to_be_bytes());
    bytes.push(state);
    dispatch(engine, &bytes, now)
}
fn count_24(engine: &Engine<Vec<u8>>) -> usize {
    engine
        .active
        .as_ref()
        .unwrap()
        .pending_shared
        .iter()
        .filter(|wire| {
            replication::ObjectUpdate::decode_with_profile(wire, engine.wire_profile)
                .is_ok_and(|u| u.header.object_type_mask == 2)
        })
        .count()
}
#[test]
fn guided_unlocked_sentinel_uses_dumb_and_finite_refire_budget_without_lock_gate() {
    let mut e = fixture();
    for (slot, now) in [(0, 1000), (1, 1550), (2, 2100)] {
        let w = wire(&e, slot, u32::MAX);
        dispatch(&mut e, &w, now).unwrap();
        let f = &e.active.as_ref().unwrap().infantry_rocket_state.slots[usize::from(slot)]
            .as_ref()
            .unwrap()
            .flight;
        assert!(matches!(f.motion, FlightMotion::Guided(_)));
        assert!(f.guided_target.is_none());
    }
    assert_eq!(
        e.active
            .as_ref()
            .unwrap()
            .infantry_rocket_state
            .guided_budget
            .unwrap()
            .rounds
            .remaining(),
        0
    );
    let fourth = wire(&e, 3, u32::MAX);
    dispatch(&mut e, &fourth, 2650).unwrap();
    assert!(!e.active.as_ref().unwrap().infantry_rocket_state.occupies(3));
    let candidate = State::refill_guided_for(
        e.active.as_ref().unwrap(),
        e.guided_launchers.as_deref().unwrap(),
        None,
    )
    .unwrap()
    .unwrap();
    assert_eq!(candidate.guided_budget.unwrap().rounds.remaining(), 3);
}
#[test]
fn guided_target_is_captured_independently_of_aim_and_warning_counts_do_not_duplicate() {
    let mut e = fixture();
    let target = 0x0100_0000;
    aim(&mut e, target, 1, 900).unwrap();
    let before = count_24(&e);
    assert!(before > 0);
    aim(&mut e, target, 1, 901).unwrap();
    assert_eq!(count_24(&e), before);
    let w = wire(&e, 0, target);
    dispatch(&mut e, &w, 1000).unwrap();
    assert!(
        e.active.as_ref().unwrap().infantry_rocket_state.slots[0]
            .as_ref()
            .unwrap()
            .flight
            .guided_target
            .is_some()
    );
    let raised = count_24(&e);
    assert!(raised > before);
    aim(&mut e, target, 0, 1001).unwrap();
    assert_eq!(
        count_24(&e),
        raised,
        "in-flight state2 remains after aim clear"
    );
    let f = e.active.as_ref().unwrap().infantry_rocket_state.slots[0]
        .as_ref()
        .unwrap()
        .flight
        .clone();
    e.retire_infantry_rocket(&f, "synthetic_terminal", Duration::from_millis(1100), 1)
        .unwrap();
    e.tick_guided_warnings(Duration::from_millis(1100), 1)
        .unwrap();
    assert!(count_24(&e) > raised);
}
#[test]
fn guided_foreign_world_stale_allocation_and_zero_health_are_fenced() {
    let mut e = fixture();
    let target = e.guided_target(1, 0x0100_0000).unwrap();
    assert_eq!(
        e.guided_target(99, 0x0100_0000).unwrap_err(),
        "guided_target_world_mismatch"
    );
    assert!(e.guided_target(1, 0x0112_0000).is_err());
    e.ground_runtime.as_mut().unwrap().reset().unwrap();
    assert!(e.guided_anchor(&target).is_none());
    e.active.as_mut().unwrap().life = CharacterLife::Alive {
        generation: 4,
        health_quarter_points: 0,
    };
    aim(&mut e, 0x0100_0000, 2, 900).unwrap();
    assert_eq!(count_24(&e), 0);
}
#[test]
fn guided_record_failure_preserves_launch_budget_slot_and_aim_warning_state() {
    let mut e = fixture();
    let w = wire(&e, 0, 0x0100_0000);
    e.max_log = e.written;
    assert_eq!(dispatch(&mut e, &w, 1000).unwrap_err(), "log_limit_reached");
    let state = &e.active.as_ref().unwrap().infantry_rocket_state;
    assert!(state.guided_budget.is_none());
    assert!(!state.occupies(0));
    assert_eq!(count_24(&e), 0);
    let mut e = fixture();
    e.max_log = e.written;
    assert_eq!(
        aim(&mut e, 0x0100_0000, 2, 900).unwrap_err(),
        "log_limit_reached"
    );
    assert_eq!(e.guided_warnings, Default::default());
    assert_eq!(count_24(&e), 0);
}

#[test]
fn guided_all_factions_require_owned_skill_and_matching_sight_without_stock_rules() {
    for faction in 0..3 {
        let mut e = fixture();
        let item = (u16::from(faction) + 1) * 1024 + 75;
        let v = *e
            .guided_launchers
            .as_ref()
            .unwrap()
            .variant(faction, item)
            .unwrap();
        let actor = e.active.as_mut().unwrap();
        actor.reservation.character.faction = faction;
        let kit = actor.spawned_loadout.as_mut().unwrap();
        let inv = usize::from(kit.handle.inventory_index);
        kit.inventories[inv].slots[3] = Some(v.item_id);
        kit.inventories[inv].slots[21] = Some(v.stock_sight_item_id);
        actor.reservation.character.skills = Some([0; 5]);
        e.infantry_rockets = None;
        let w = wire(&e, 0, u32::MAX);
        dispatch(&mut e, &w, 1000).unwrap();
        assert!(!e.active.as_ref().unwrap().infantry_rocket_state.occupies(0));
        let actor = e.active.as_mut().unwrap();
        actor.reservation.character.skills = Some([1 << 25, 0, 0, 0, 0]);
        actor.spawned_loadout.as_mut().unwrap().inventories[inv].slots[21] = None;
        dispatch(&mut e, &w, 1001).unwrap();
        assert!(!e.active.as_ref().unwrap().infantry_rocket_state.occupies(0));
        e.active
            .as_mut()
            .unwrap()
            .spawned_loadout
            .as_mut()
            .unwrap()
            .inventories[inv]
            .slots[21] = Some(v.stock_sight_item_id);
        dispatch(&mut e, &w, 1002).unwrap();
        assert!(e.active.as_ref().unwrap().infantry_rocket_state.occupies(0));
    }
}
#[test]
fn guided_unsupported_target_does_not_silently_select_dumb_or_consume_ammo() {
    let mut e = fixture();
    for target in [0x2100_0000, 0x0380_0001, 0x0100_ffff] {
        let w = wire(&e, 0, target);
        dispatch(&mut e, &w, 1000).unwrap();
        let state = &e.active.as_ref().unwrap().infantry_rocket_state;
        assert!(!state.occupies(0));
        assert!(state.guided_budget.is_none());
    }
}
#[test]
fn guided_refused_terminal_or_retire_keeps_independent_flight_warning() {
    for retire in [false, true] {
        let mut e = fixture();
        let w = wire(&e, 0, 0x0100_0000);
        dispatch(&mut e, &w, 1000).unwrap();
        let f = e.active.as_ref().unwrap().infantry_rocket_state.slots[0]
            .as_ref()
            .unwrap()
            .flight
            .clone();
        let ledger = e.guided_warnings.clone();
        let published = count_24(&e);
        e.max_log = e.written;
        let result = if retire {
            e.retire_infantry_rocket(&f, "synthetic_retire", Duration::from_millis(1100), 1)
        } else {
            e.finish_infantry_rocket(
                &f,
                [1.25, 2.5, 2.5],
                "synthetic_contact",
                None,
                Duration::from_millis(1100),
                1,
            )
            .map(|_| ())
        };
        assert_eq!(result.unwrap_err(), "log_limit_reached");
        assert_eq!(e.guided_warnings, ledger);
        let actor = e.active.as_ref().unwrap();
        assert_eq!(
            actor
                .infantry_rocket_state
                .guided_targets(actor, e.round_number)
                .count(),
            1
        );
        assert_eq!(count_24(&e), published);
        assert!(
            actor.infantry_rocket_state.slots[0]
                .as_ref()
                .unwrap()
                .cleanup
                .is_none()
        );
    }
}

#[test]
fn guided_warning_budget_refusal_is_before_record_and_launch_commit() {
    let mut e = fixture();
    for _ in 0..crate::multiplayer::MAX_SHARED_QUEUE {
        e.active.as_mut().unwrap().queue_shared(vec![0]);
    }
    let written = e.written;
    let ledger = e.guided_warnings.clone();
    assert_eq!(
        aim(&mut e, 0x0100_0000, 2, 900).unwrap_err(),
        "guided_warning_publication_budget_exceeded"
    );
    assert_eq!(e.written, written);
    assert_eq!(e.guided_warnings, ledger);
    assert!(!e.failed);
    let w = wire(&e, 0, 0x0100_0000);
    assert_eq!(
        dispatch(&mut e, &w, 1000).unwrap_err(),
        "guided_warning_publication_budget_exceeded"
    );
    assert_eq!(e.written, written);
    let state = &e.active.as_ref().unwrap().infantry_rocket_state;
    assert!(!state.occupies(0));
    assert!(state.guided_budget.is_none());
}
#[test]
fn guided_aim_cleanup_late_observer_and_reused_target_do_not_transfer_warning() {
    let mut e = fixture();
    let batches = e.active.as_ref().unwrap().ground_vehicle_batch;
    e.active.as_mut().unwrap().ground_vehicle_batch = 0;
    aim(&mut e, 0x0100_0000, 2, 900).unwrap();
    assert_eq!(count_24(&e), 0);
    e.active.as_mut().unwrap().ground_vehicle_batch = batches;
    e.tick_guided_warnings(Duration::from_millis(950), 1)
        .unwrap();
    let count = count_24(&e);
    assert!(count > 0);
    e.tick_guided_warnings(Duration::from_millis(1000), 1)
        .unwrap();
    assert_eq!(count_24(&e), count);
    let ledger = e.guided_warnings.clone();
    e.ground_runtime.as_mut().unwrap().reset().unwrap();
    e.tick_guided_warnings(Duration::from_millis(1050), 1)
        .unwrap();
    assert_ne!(e.guided_warnings, ledger);
    assert_eq!(
        count_24(&e),
        count,
        "old allocation must not clear or warn the fresh allocation"
    );
    aim(&mut e, 0x0100_0000, 1, 1100).unwrap();
    let count = count_24(&e);
    assert!(count > 0);
    e.active.as_mut().unwrap().life = CharacterLife::AwaitingRespawn { generation: 4 };
    e.tick_guided_warnings(Duration::from_millis(1150), 1)
        .unwrap();
    assert!(count_24(&e) > count);
}
#[test]
fn guided_steering_loss_preserves_flight_warning_and_aim_cannot_retarget_it() {
    let mut e = fixture();
    let w = wire(&e, 0, 0x0100_0000);
    dispatch(&mut e, &w, 1000).unwrap();
    let count = count_24(&e);
    let slot = e.active.as_mut().unwrap().infantry_rocket_state.slots[0]
        .as_mut()
        .unwrap();
    let original = slot.flight.guided_target.clone();
    let FlightMotion::Guided(motion) = &slot.flight.motion else {
        panic!()
    };
    let crate::guided_launcher_motion::Advance::Flying(lost) = motion
        .advance(1100, || None, |_, _, _| Ok(None::<Contact<OwnedContact>>))
        .unwrap()
    else {
        panic!()
    };
    assert!(lost.lost());
    slot.flight.motion = FlightMotion::Guided(lost);
    aim(&mut e, 0x0100_0000, 1, 1101).unwrap();
    assert_eq!(count_24(&e), count);
    assert_eq!(
        e.active.as_ref().unwrap().infantry_rocket_state.slots[0]
            .as_ref()
            .unwrap()
            .flight
            .guided_target,
        original
    );
    e.tick_guided_warnings(Duration::from_millis(1150), 1)
        .unwrap();
    assert_eq!(count_24(&e), count);
}

#[test]
fn guided_constructor_anchors_and_static_target_identity_follow_current_roots() {
    let mut e = fixture();
    let ground = e.guided_target(1, 0x0100_0000).unwrap();
    let gy = e
        .ground_runtime
        .as_ref()
        .unwrap()
        .get(0)
        .unwrap()
        .pose
        .position
        .y;
    assert!(
        (e.guided_anchor(&ground).unwrap()[1] - f64::from(gy) - f64::from(0.8_f32)).abs() < 1e-9
    );
    let hospital = e.guided_target(1, 0x0d00_0005).unwrap();
    let y = e
        .bunker_world
        .as_ref()
        .unwrap()
        .domination_bunker(5)
        .unwrap()
        .position
        .y;
    assert_eq!(e.guided_anchor(&hospital).unwrap()[1], f64::from(y) + 1.);
    crate::domination_structural_world::hospital_rocket_helo_damage::tests::install_helo_fixture(
        &mut e,
        CharacterPosition {
            x: 10.,
            y: 10.,
            z: 10.,
        },
    );
    let helo = e.guided_target(1, 0x0200_0000).unwrap();
    assert_eq!(e.guided_anchor(&helo).unwrap(), [10., 8., 10.]);
    e.domination_gameplay
        .as_mut()
        .unwrap()
        .state
        .helo
        .as_mut()
        .unwrap()
        .carriers[0]
        .current_pose
        .as_mut()
        .unwrap()
        .position
        .x = 12.;
    assert_eq!(e.guided_target(1, 0x0200_0000).unwrap(), helo);
    assert_eq!(e.guided_anchor(&helo).unwrap(), [12., 8., 10.]);
    let spawn = e
        .bunker_world
        .as_ref()
        .unwrap()
        .domination_bunker(5)
        .unwrap()
        .spawn_index;
    e.bunker_world
        .as_mut()
        .unwrap()
        .apply_domination_cycle(spawn, false, 0)
        .unwrap();
    e.bunker_world
        .as_mut()
        .unwrap()
        .apply_domination_cycle(spawn, true, 0)
        .unwrap();
    assert_eq!(e.guided_target(1, 0x0d00_0005).unwrap(), hospital);
}
#[test]
fn guided_capsule_and_snapshot_rotation_use_current_heading() {
    let mut e = fixture();
    let w = wire(&e, 0, 0x0100_0000);
    dispatch(&mut e, &w, 1000).unwrap();
    let flight = e.active.as_ref().unwrap().infantry_rocket_state.slots[0]
        .as_ref()
        .unwrap()
        .flight
        .clone();
    let step = flight
        .step(1100, None, Some([20., 2.5, 30.]), |shape| {
            let forward = [
                shape.rotation[0][2],
                shape.rotation[1][2],
                shape.rotation[2][2],
            ];
            assert!((forward.iter().map(|v| v * v).sum::<f64>() - 1.).abs() < 1e-6);
            Ok(None)
        })
        .unwrap();
    let Step::Move(FlightMotion::Guided(motion)) = step else {
        panic!()
    };
    assert!(motion.heading()[0] > 0.);
    let slot = e.active.as_mut().unwrap().infantry_rocket_state.slots[0]
        .as_mut()
        .unwrap();
    slot.flight.motion = FlightMotion::Guided(motion.clone());
    let snapshot = e
        .active
        .as_ref()
        .unwrap()
        .infantry_rocket_state
        .snapshot_packets(1100, e.wire_profile)
        .unwrap();
    replication::ObjectUpdate::decode_with_profile(&snapshot[0], e.wire_profile).unwrap();
    let mut bits = replication::BitReader::with_profile(&snapshot[0], e.wire_profile);
    bits.position = e.wire_profile.header_bits();
    assert_eq!(bits.read(1).unwrap(), 1);
    bits.read(16).unwrap();
    let mut q = [0_f32; 4];
    for field in 0..=6 {
        if bits.read(1).unwrap() == 0 {
            continue;
        }
        match field {
            0 => {
                bits.read(1).unwrap();
            }
            1 => {
                bits.read(16).unwrap();
            }
            4 => {
                bits.read(32).unwrap();
            }
            5 => {
                bits.read(16).unwrap();
                bits.read(17).unwrap();
                bits.read(16).unwrap();
            }
            6 => {
                for v in &mut q {
                    let sign = if bits.read(1).unwrap() == 0 { 1. } else { -1. };
                    *v =
                        sign * (bits.read(2).unwrap() as f32 + bits.read(8).unwrap() as f32 / 256.);
                }
            }
            _ => panic!("unexpected field"),
        }
    }
    let norm = q.iter().map(|v| v * v).sum::<f32>().sqrt();
    let rotation = rigid_rotation(q.map(|v| v / norm)).unwrap();
    for (value, expected) in [rotation[0][2], rotation[1][2], rotation[2][2]]
        .into_iter()
        .zip(motion.heading())
    {
        assert!((value - expected).abs() < 0.01);
    }
}
