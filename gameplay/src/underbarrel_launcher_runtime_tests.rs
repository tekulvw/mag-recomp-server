use super::*;
use crate::spawn_progression::dispatch;

fn fixture() -> Engine<Vec<u8>> {
    let mut engine = tests::fixture();
    engine.underbarrel_launchers =
        Some(Arc::new(underbarrel_launchers::tests::synthetic_catalog()));
    equip_launcher(&mut engine, 0);
    engine
}

fn equip_launcher(engine: &mut Engine<Vec<u8>>, group: usize) -> underbarrel_launchers::Variant {
    let catalog = engine.underbarrel_launchers.as_ref().unwrap();
    let actor = engine.active.as_mut().unwrap();
    let v = catalog.variants()[3 * group + usize::from(actor.reservation.character.faction)];
    let kit = actor.spawned_loadout.as_mut().unwrap();
    let inventory =
        &mut kit.inventories[usize::from(actor.current_inventory_handle.unwrap().inventory_index)];
    inventory.slots[0] = Some(v.host_item_id);
    inventory.slots[17] = Some(v.attachment_item_id);
    v
}

fn wire(engine: &Engine<Vec<u8>>, gear: u8, slot: u16, power: u32) -> Vec<u8> {
    let player = engine.active.as_ref().unwrap().reservation.player;
    let mut bits = replication::BitWriter::with_profile(engine.wire_profile);
    bits.write(0x0210, 16).unwrap();
    for value in [1.25_f32, 2.5, 0.125, 0., 0., 0., 1., 0., 0., 0.] {
        bits.write(value.to_bits(), 32).unwrap();
    }
    for (value, width) in [
        (u32::from(gear), 8),
        (power, 10),
        (u32::MAX, 32),
        (0x0380_0000 | u32::from(player), 32),
        (u32::from(player), 16),
        (u32::from(slot), 16),
        (0, 6),
    ] {
        bits.write(value, width).unwrap();
    }
    bits.finish()
}
fn state(engine: &Engine<Vec<u8>>) -> &State {
    &engine.active.as_ref().unwrap().infantry_rocket_state
}
fn launch(engine: &mut Engine<Vec<u8>>, gear: u8, slot: u16, power: u32, now: u64) {
    let bytes = wire(engine, gear, slot, power);
    assert!(
        !dispatch(engine, &bytes, now).unwrap().is_empty(),
        "{}",
        String::from_utf8_lossy(&engine.writer)
    );
    assert!(state(engine).occupies(slot as u8));
}
fn rounds(engine: &Engine<Vec<u8>>) -> u16 {
    let actor = engine.active.as_ref().unwrap();
    let item = equipped_underbarrel(actor, engine.underbarrel_launchers.as_deref().unwrap())
        .unwrap()
        .attachment_item_id;
    let CharacterLife::Alive { generation, .. } = actor.life else {
        panic!("test owner alive")
    };
    underbarrel_rounds(state(engine), generation, item)
}

fn underbarrel_rounds(state: &State, generation: u32, item: u16) -> u16 {
    let index = state.underbarrel_budget_index(generation, item).unwrap();
    state.underbarrel_budgets[index].unwrap().rounds.remaining()
}

#[test]
fn underbarrel_switch_back_preserves_refire_and_spent_rounds() {
    let mut engine = fixture();
    engine.underbarrel_launchers = Some(Arc::new(
        underbarrel_launchers::tests::synthetic_upgraded_catalog(),
    ));
    launch(&mut engine, 17, 0, 511, 1000);
    equip_launcher(&mut engine, 1);
    launch(&mut engine, 17, 1, 511, 1100);
    equip_launcher(&mut engine, 2);
    launch(&mut engine, 17, 2, 511, 1200);
    equip_launcher(&mut engine, 0);
    let too_soon = wire(&engine, 17, 3, 511);
    assert!(dispatch(&mut engine, &too_soon, 1499).unwrap().is_empty());
    assert!(!state(&engine).occupies(3));
    launch(&mut engine, 17, 3, 511, 1500);
    assert_eq!(rounds(&engine), 1);

    engine.active.as_mut().unwrap().infantry_rocket_state.slots[2] = None;
    equip_launcher(&mut engine, 1);
    let too_soon = wire(&engine, 17, 2, 511);
    assert!(dispatch(&mut engine, &too_soon, 1849).unwrap().is_empty());
    launch(&mut engine, 17, 2, 511, 1850);
    assert_eq!(rounds(&engine), 1);

    engine.active.as_mut().unwrap().infantry_rocket_state.slots[0] = None;
    equip_launcher(&mut engine, 2);
    let too_soon = wire(&engine, 17, 0, 511);
    assert!(dispatch(&mut engine, &too_soon, 2199).unwrap().is_empty());
    launch(&mut engine, 17, 0, 511, 2200);
    assert_eq!(rounds(&engine), 1);
}

#[test]
fn underbarrel_refill_targets_only_the_equipped_life_budget() {
    let mut engine = fixture();
    engine.underbarrel_launchers = Some(Arc::new(
        underbarrel_launchers::tests::synthetic_upgraded_catalog(),
    ));
    let stock = equip_launcher(&mut engine, 0);
    launch(&mut engine, 17, 0, 511, 1000);
    let tier_two = equip_launcher(&mut engine, 1);
    launch(&mut engine, 17, 1, 511, 1100);
    let tier_four = equip_launcher(&mut engine, 2);
    launch(&mut engine, 17, 2, 511, 1200);
    let actor = engine.active.as_ref().unwrap();
    let CharacterLife::Alive {
        generation,
        health_quarter_points,
    } = actor.life
    else {
        panic!("test owner alive")
    };
    let catalog = engine.underbarrel_launchers.as_deref().unwrap();
    let staged = State::refill_underbarrel_for(actor, catalog, None)
        .unwrap()
        .unwrap();
    assert_eq!(
        underbarrel_rounds(&staged, generation, stock.attachment_item_id),
        2
    );
    assert_eq!(
        underbarrel_rounds(&staged, generation, tier_two.attachment_item_id),
        2
    );
    assert_eq!(
        underbarrel_rounds(&staged, generation, tier_four.attachment_item_id),
        3
    );
    assert_eq!(rounds(&engine), 2);
    engine.active.as_mut().unwrap().infantry_rocket_state = staged;
    let too_soon = wire(&engine, 17, 3, 511);
    assert!(dispatch(&mut engine, &too_soon, 2199).unwrap().is_empty());

    engine.active.as_mut().unwrap().life = CharacterLife::Alive {
        generation: generation + 1,
        health_quarter_points,
    };
    launch(&mut engine, 17, 3, 511, 2200);
    assert_eq!(rounds(&engine), 2);
    engine.active.as_mut().unwrap().infantry_rocket_state.slots[0] = None;
    equip_launcher(&mut engine, 0);
    launch(&mut engine, 17, 0, 511, 2201);
    assert_eq!(rounds(&engine), 2);
    engine.active.as_mut().unwrap().infantry_rocket_state.slots[1] = None;
    equip_launcher(&mut engine, 1);
    launch(&mut engine, 17, 1, 511, 2202);
    assert_eq!(rounds(&engine), 2);
}

#[test]
fn underbarrel_third_budget_is_not_installed_on_record_failure() {
    let mut engine = fixture();
    engine.underbarrel_launchers = Some(Arc::new(
        underbarrel_launchers::tests::synthetic_upgraded_catalog(),
    ));
    launch(&mut engine, 17, 0, 511, 1000);
    equip_launcher(&mut engine, 1);
    launch(&mut engine, 17, 1, 511, 1100);
    equip_launcher(&mut engine, 2);
    let bytes = wire(&engine, 17, 2, 511);
    engine.max_log = engine.written;
    assert_eq!(
        dispatch(&mut engine, &bytes, 1200).err(),
        Some("log_limit_reached")
    );
    assert!(!state(&engine).occupies(2));
    assert_eq!(
        state(&engine).underbarrel_budgets.iter().flatten().count(),
        2
    );
    for group in [0, 1] {
        equip_launcher(&mut engine, group);
        assert_eq!(rounds(&engine), 2);
    }
}

#[test]
fn underbarrel_ammo_is_independent_of_rockets_and_refills_merge_atomically() {
    let mut engine = fixture();
    // The native producer clamps power; higher ten-bit values are not normal fire.
    let oversized = wire(&engine, 17, 0, 700);
    assert!(dispatch(&mut engine, &oversized, 999).unwrap().is_empty());
    assert!(
        state(&engine)
            .underbarrel_budgets
            .iter()
            .all(Option::is_none)
    );
    launch(&mut engine, 17, 0, 511, 1000);
    assert_eq!(rounds(&engine), 2);
    let duplicate = wire(&engine, 17, 0, 511);
    assert!(dispatch(&mut engine, &duplicate, 1001).unwrap().is_empty());
    assert_eq!(rounds(&engine), 2);
    launch(&mut engine, 3, 1, 64, 1100);
    assert_eq!(tests::rounds(&engine), 3);
    assert_eq!(rounds(&engine), 2);
    let too_soon = wire(&engine, 17, 2, 510);
    assert!(dispatch(&mut engine, &too_soon, 1499).unwrap().is_empty());
    assert!(!state(&engine).occupies(2));
    launch(&mut engine, 17, 2, 510, 1500);
    let rocket_slot = wire(&engine, 17, 1, 509);
    assert!(
        dispatch(&mut engine, &rocket_slot, 2000)
            .unwrap()
            .is_empty()
    );
    assert_eq!(rounds(&engine), 1);
    launch(&mut engine, 17, 3, 509, 2000);
    assert_eq!(rounds(&engine), 0);
    engine.active.as_mut().unwrap().infantry_rocket_state.slots[0] = None;
    let empty = wire(&engine, 17, 0, 508);
    assert!(dispatch(&mut engine, &empty, 2500).unwrap().is_empty());
    assert_eq!(rounds(&engine), 0);
    let actor = engine.active.as_ref().unwrap();
    let rocket = State::refill_for(actor, engine.infantry_rockets.as_deref().unwrap()).unwrap();
    let combined = State::refill_underbarrel_for(
        actor,
        engine.underbarrel_launchers.as_deref().unwrap(),
        rocket,
    )
    .unwrap()
    .unwrap();
    assert_eq!(combined.budget.unwrap().rounds.remaining(), 4);
    assert_eq!(
        combined.underbarrel_budgets[0].unwrap().rounds.remaining(),
        3
    );
    assert_eq!(rounds(&engine), 0); // Candidate only; no mutation before record.
    assert_eq!(tests::rounds(&engine), 3);
}

#[test]
fn underbarrel_launch_rolls_back_on_record_failure_and_denies_wrong_equipment() {
    let mut engine = fixture();
    let bytes = wire(&engine, 17, 0, 511);
    engine.max_log = engine.written;
    assert_eq!(
        dispatch(&mut engine, &bytes, 1000).err(),
        Some("log_limit_reached")
    );
    assert!(!state(&engine).occupies(0));
    assert!(
        state(&engine)
            .underbarrel_budgets
            .iter()
            .all(Option::is_none)
    );
    let mut engine = fixture();
    let actor = engine.active.as_mut().unwrap();
    let kit = actor.spawned_loadout.as_mut().unwrap();
    kit.inventories[usize::from(actor.current_inventory_handle.unwrap().inventory_index)].slots
        [0] = Some(999);
    let wrong = wire(&engine, 17, 0, 511);
    assert!(dispatch(&mut engine, &wrong, 1000).unwrap().is_empty());
    assert!(!state(&engine).occupies(0));
    assert!(
        state(&engine)
            .underbarrel_budgets
            .iter()
            .all(Option::is_none)
    );
}

#[test]
fn underbarrel_native_capsule_uses_gravity_and_preserves_prearm_contact() {
    let mut engine = fixture();
    launch(&mut engine, 17, 3, 511, 1000);
    let mut flight = state(&engine).slots[3].as_ref().unwrap().flight.clone();
    let origin = flight.motion.position();
    let Step::Move(motion) = flight
        .step(1100, None, |shape| {
            assert_eq!(shape.radius, flight.variant.radius());
            Ok(None)
        })
        .unwrap()
    else {
        panic!("prearm free flight");
    };
    assert!(motion.position()[1] < origin[1]);
    assert!(motion.position()[2] > origin[2]);
    flight.motion = motion;
    let Step::Move(motion) = flight
        .step(1150, None, |shape| {
            Ok(Some(OwnedContact {
                obstacle: projectile_scene::OwnedObstacleId::Static {
                    instance: "wall".into(),
                    triangle: 1,
                },
                fraction: 0.5,
                center: shape.end,
                normal: Some([0., 0., -1.]),
            }))
        })
        .unwrap()
    else {
        panic!("prearm contact must survive");
    };
    flight.motion = motion;
    assert!(
        matches!(
            flight
                .step(1200, None, |shape| Ok(Some(OwnedContact {
                    obstacle: projectile_scene::OwnedObstacleId::Static {
                        instance: "wall".into(),
                        triangle: 1
                    },
                    fraction: 0.,
                    center: shape.start,
                    normal: None,
                })))
                .unwrap(),
            Step::Move(_)
        ),
        "old overlap cannot become armed detonation"
    );
}

#[test]
fn underbarrel_terminal_damage_and_publication_retry_as_one_transaction() {
    for reject_record in [false, true] {
        let mut engine = fixture();
        launch(&mut engine, 17, 3, 511, 1000);
        let flight = state(&engine).slots[3].as_ref().unwrap().flight.clone();
        let victim = "synthetic-rocket-target";
        let before = engine.peers[victim].last_damage.health_quarter_points;
        let queued = engine.peers[victim].pending_shared.clone();
        if reject_record {
            engine.max_log = engine.written;
        }
        let result = engine.finish_infantry_rocket(
            &flight,
            [1.25, 2.5, 1.25],
            "contact",
            None,
            Duration::from_millis(1500),
            1,
        );
        if reject_record {
            assert_eq!(result.err(), Some("log_limit_reached"));
            assert_eq!(
                engine.peers[victim].last_damage.health_quarter_points,
                before
            );
            assert_eq!(engine.peers[victim].pending_shared, queued);
            assert!(state(&engine).slots[3].as_ref().unwrap().cleanup.is_none());
        } else {
            result.unwrap();
            let audit: Vec<Value> = engine
                .writer
                .split(|b| *b == b'\n')
                .filter_map(|line| serde_json::from_slice::<Value>(line).ok())
                .filter(|row| row["op"] == "infantry_rocket_terminal")
                .collect();
            assert_eq!(audit.len(), 1);
            assert!(audit[0]["damage_denial"].is_null(), "{audit:?}");
            assert!(
                engine.peers[victim].last_damage.health_quarter_points < before,
                "{audit:?}"
            );
            assert!(
                state(&engine)
                    .snapshot_packets(1500, engine.wire_profile)
                    .unwrap()
                    .is_empty()
            );
            assert_eq!(rounds(&engine), 2);
            engine
                .tick_infantry_rockets(Duration::from_millis(2000), 1)
                .unwrap();
            assert!(
                state(&engine).occupies(3),
                "terminal must drain before removal"
            );
            engine.drain_shared(vec![], 1).unwrap();
            engine
                .tick_infantry_rockets(Duration::from_millis(2500), 1)
                .unwrap();
            assert!(!state(&engine).occupies(3));
        }
    }
}
