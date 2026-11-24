use super::*;
use crate::spawn_progression::dispatch;

pub(crate) fn fixture() -> Engine<Vec<u8>> {
    let mut engine = crate::domination_structural_world::combined_ground_rocket_fixture(true);
    let catalog = infantry_rockets::tests::synthetic_catalog();
    let actor = engine.active.as_mut().unwrap();
    let variant = catalog.variants()[usize::from(actor.reservation.character.faction)];
    let mut loadout_fixture = human_kill_reward_tests::fixture();
    let source = loadout_fixture.active.as_mut().unwrap();
    let mut kit = source.spawned_loadout.take().unwrap();
    let handle = source.current_inventory_handle.unwrap();
    kit.inventories[usize::from(handle.inventory_index)].slots
        [usize::from(infantry_rockets::GEAR_SLOT)] = Some(variant.item_id);
    actor.spawned_loadout = Some(kit);
    actor.current_inventory_handle = Some(handle);
    actor.mounted_bunker = None;
    actor.mounted_seat = None;
    actor.hospital_rocket_flights = Default::default();
    engine.infantry_rockets = Some(Arc::new(catalog));
    engine
}

fn wire(engine: &Engine<Vec<u8>>, slot: u16, power: u32) -> Vec<u8> {
    let player = engine.active.as_ref().unwrap().reservation.player;
    let mut bits = replication::BitWriter::with_profile(engine.wire_profile);
    bits.write(0x0210, 16).unwrap();
    for value in [1.25_f32, 2.5, 0.125, 0., 0., 0., 1., 0., 0., 0.] {
        bits.write(value.to_bits(), 32).unwrap();
    }
    for (value, width) in [
        (u32::from(infantry_rockets::GEAR_SLOT), 8),
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

pub(crate) fn rounds(engine: &Engine<Vec<u8>>) -> u16 {
    state(engine).budget.unwrap().rounds.remaining()
}
fn state(engine: &Engine<Vec<u8>>) -> &State {
    &engine.active.as_ref().unwrap().infantry_rocket_state
}
fn flight(engine: &Engine<Vec<u8>>, slot: usize) -> &Flight {
    &state(engine).slots[slot].as_ref().unwrap().flight
}
fn audit(engine: &Engine<Vec<u8>>, operation: &str) -> Vec<Value> {
    engine
        .writer
        .split(|byte| *byte == b'\n')
        .filter_map(|line| serde_json::from_slice::<Value>(line).ok())
        .filter(|row| row["op"] == operation)
        .collect()
}
pub(crate) fn launch(engine: &mut Engine<Vec<u8>>, slot: u16, power: u32, now: u64) {
    let request = wire(engine, slot, power);
    let packets = dispatch(engine, &request, now).unwrap();
    assert!(
        !packets.is_empty(),
        "{}",
        String::from_utf8_lossy(&engine.writer)
    );
    assert!(state(engine).occupies(slot as u8));
}

pub(super) fn lethal_target(engine: &mut Engine<Vec<u8>>) {
    let victim = engine.peers.get_mut("synthetic-rocket-target").unwrap();
    victim.life = CharacterLife::Alive {
        generation: 4,
        health_quarter_points: 4,
    };
    victim.last_damage.health_quarter_points = 4;
}

pub(super) fn assert_rocket_death(engine: &Engine<Vec<u8>>, expected: &[u8]) {
    let victim = &engine.peers["synthetic-rocket-target"];
    assert_eq!(victim.last_damage.health_quarter_points, 0);
    let notices: Vec<_> = victim
        .pending_notifications
        .iter()
        .filter_map(|publication| match publication {
            match_flow::Publication::Application(wire) if wire.starts_with(&[1, 255]) => {
                Some(wire.as_slice())
            }
            _ => None,
        })
        .collect();
    assert_eq!(notices, [expected]);
}

#[test]
fn rocket_death_stock_contact_keeps_launch_item_after_shooter_death() {
    let mut engine = fixture();
    lethal_target(&mut engine);
    launch(&mut engine, 3, 64, 1000);
    assert_eq!(flight(&engine, 3).source.item, 1089);
    let actor = engine.active.as_mut().unwrap();
    actor.life = CharacterLife::AwaitingRespawn { generation: 4 };
    actor.spawned_loadout.as_mut().unwrap().inventories[0].slots[3] = None;
    engine
        .tick_infantry_rockets(Duration::from_millis(1500), 1)
        .unwrap();
    // Independent 01ff literal: attacker0, defender1, Weapon2, index1089, flags0.
    assert_rocket_death(&engine, &[1, 255, 0, 0, 0, 1, 2, 0, 0, 4, 65, 0]);
    engine
        .tick_infantry_rockets(Duration::from_millis(1501), 1)
        .unwrap();
    assert_rocket_death(&engine, &[1, 255, 0, 0, 0, 1, 2, 0, 0, 4, 65, 0]);
}

#[test]
fn rocket_death_record_refusal_preserves_health_notice_and_flight() {
    let mut engine = fixture();
    lethal_target(&mut engine);
    launch(&mut engine, 3, 64, 1000);
    let queue = engine.peers["synthetic-rocket-target"]
        .pending_shared
        .clone();
    let limit = engine.max_log;
    engine.max_log = engine.written;
    assert_eq!(
        engine
            .tick_infantry_rockets(Duration::from_millis(1500), 1)
            .err(),
        Some("log_limit_reached")
    );
    let victim = &engine.peers["synthetic-rocket-target"];
    assert_eq!(victim.last_damage.health_quarter_points, 4);
    assert_eq!(victim.pending_shared, queue);
    assert!(victim.pending_notifications.is_empty());
    assert!(state(&engine).slots[3].as_ref().unwrap().cleanup.is_none());
    engine.max_log = limit;
    assert_eq!(
        engine
            .tick_infantry_rockets(Duration::from_millis(1501), 1)
            .err(),
        Some("recorder_unavailable")
    );
    assert_eq!(
        engine.peers["synthetic-rocket-target"]
            .last_damage
            .health_quarter_points,
        4
    );
    assert!(
        engine.peers["synthetic-rocket-target"]
            .pending_notifications
            .is_empty()
    );
}

#[test]
fn rocket_death_full_notice_fifo_defers_entire_terminal_then_health_precedes_notice() {
    let mut engine = fixture();
    lethal_target(&mut engine);
    launch(&mut engine, 3, 64, 1000);
    let victim = engine.peers.get_mut("synthetic-rocket-target").unwrap();
    for _ in 0..crate::match_flow::MAX_PENDING_NOTIFICATIONS {
        victim
            .pending_notifications
            .push_back(match_flow::Publication::Application(vec![9]));
    }
    engine
        .tick_infantry_rockets(Duration::from_millis(1500), 1)
        .unwrap();
    assert_eq!(
        engine.peers["synthetic-rocket-target"]
            .last_damage
            .health_quarter_points,
        4
    );
    assert_eq!(
        engine.peers["synthetic-rocket-target"]
            .pending_notifications
            .len(),
        crate::match_flow::MAX_PENDING_NOTIFICATIONS
    );
    assert!(audit(&engine, "infantry_rocket_terminal").is_empty());
    assert!(state(&engine).slots[3].as_ref().unwrap().cleanup.is_none());
    engine
        .peers
        .get_mut("synthetic-rocket-target")
        .unwrap()
        .pending_notifications
        .clear();
    engine
        .tick_infantry_rockets(Duration::from_millis(1501), 1)
        .unwrap();
    assert_rocket_death(&engine, &[1, 255, 0, 0, 0, 1, 2, 0, 0, 4, 65, 0]);
    let terminal = audit(&engine, "infantry_rocket_terminal");
    let health = terminal[0]["effects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|effect| effect["character_id"] == 0x0380_0001u32)
        .unwrap()["damage_publication_hex"]
        .as_str()
        .unwrap();
    // Pending reliable health prevents the application FIFO from overtaking it.
    assert!(
        engine
            .drain_notifications(Vec::new(), 1)
            .unwrap()
            .is_empty()
    );
    let mut messages = engine.drain_shared(Vec::new(), 1).unwrap();
    messages.extend(engine.drain_shared(Vec::new(), 1).unwrap());
    let victim: Vec<_> = messages
        .iter()
        .filter(|m| m.reservation_id == "synthetic-rocket-target")
        .collect();
    // The drain rewrites the sequence/NAK header, retaining the damage body.
    let hp = victim
        .iter()
        .position(|m| m.payload_hex.get(20..) == health.get(20..))
        .unwrap();
    let notice = victim
        .iter()
        .position(|m| m.payload_hex == "01ff00000001020000044100")
        .unwrap();
    assert!(hp < notice);
    assert_eq!(
        victim
            .iter()
            .filter(|m| m.payload_hex == "01ff00000001020000044100")
            .count(),
        1
    );
}

#[test]
fn rocket_death_replaced_allocation_never_inherits_notice() {
    let mut engine = fixture();
    lethal_target(&mut engine);
    launch(&mut engine, 3, 64, 1000);
    engine.active.as_mut().unwrap().allocation_sequence += 1;
    engine
        .tick_infantry_rockets(Duration::from_millis(1500), 1)
        .unwrap();
    assert_eq!(
        engine.peers["synthetic-rocket-target"]
            .last_damage
            .health_quarter_points,
        4
    );
    assert!(
        engine.peers["synthetic-rocket-target"]
            .pending_notifications
            .is_empty()
    );
    assert!(audit(&engine, "infantry_rocket_terminal").is_empty());
    assert_eq!(
        audit(&engine, "infantry_rocket_retired")[0]["reason"],
        "stale_infantry_rocket_source"
    );
}

#[test]
fn rocket_death_constructor_preserves_signed_native_equipment_index() {
    let mut engine = fixture();
    launch(&mut engine, 3, 64, 1000);
    let mut source = flight(&engine, 3).source.clone();
    // Synthetic signed16 boundary, independent of the current low-valued items.
    source.item = 0x8001;
    let notice = engine.plan_rocket_death_notice(&source, 1, 1).unwrap();
    assert_eq!(notice.audit()["payload_hex"], "01ff0000000102ffff800100");
}

#[test]
fn infantry_rocket_authenticated_launch_retries_budget_refill_and_refire() {
    let mut engine = fixture();
    let first = wire(&engine, 0, 64);
    launch(&mut engine, 0, 64, 1000);
    assert_eq!(rounds(&engine), 3);
    assert!(dispatch(&mut engine, &first, 1001).unwrap().is_empty());
    assert_eq!(rounds(&engine), 3);
    let too_soon = wire(&engine, 1, 65);
    assert!(dispatch(&mut engine, &too_soon, 1549).unwrap().is_empty());
    assert!(!state(&engine).occupies(1));
    launch(&mut engine, 1, 65, 1550);
    launch(&mut engine, 2, 66, 2100);
    launch(&mut engine, 3, 67, 2650);
    assert_eq!(rounds(&engine), 0);

    // Simulate an already delivered removal to isolate budget admission.
    engine.active.as_mut().unwrap().infantry_rocket_state.slots[0] = None;
    let empty = wire(&engine, 0, 68);
    assert!(dispatch(&mut engine, &empty, 3200).unwrap().is_empty());
    assert!(!state(&engine).occupies(0));

    let refill = State::refill_for(
        engine.active.as_ref().unwrap(),
        engine.infantry_rockets.as_deref().unwrap(),
    )
    .unwrap()
    .unwrap();
    engine.active.as_mut().unwrap().infantry_rocket_state = refill;
    assert_eq!(rounds(&engine), 4);
    assert!(state(&engine).occupies(1));
    assert_eq!(
        state(&engine)
            .snapshot_packets(3200, engine.wire_profile)
            .unwrap()
            .len(),
        3
    );
    launch(&mut engine, 0, 68, 3200);
    assert_eq!(rounds(&engine), 3);
}

#[test]
fn infantry_rocket_capsule_contact_applies_atomic_damage_after_arming() {
    let mut engine = fixture();
    launch(&mut engine, 3, 64, 1000);
    let generation = flight(&engine, 3).source.life_generation;
    engine.active.as_mut().unwrap().life = CharacterLife::AwaitingRespawn { generation };
    let ground_before = engine
        .ground_runtime
        .as_ref()
        .unwrap()
        .get(0)
        .unwrap()
        .health_quarter_points;
    let victim = "synthetic-rocket-target";
    let before = engine.peers[victim].last_damage.health_quarter_points;
    engine
        .tick_infantry_rockets(Duration::from_millis(1500), 1)
        .unwrap();
    let terminal = audit(&engine, "infantry_rocket_terminal");
    assert_eq!(terminal.len(), 1);
    assert_eq!(terminal[0]["terminal"], "contact");
    assert!(terminal[0]["damage_denial"].is_null(), "{terminal:?}");
    assert!(
        engine.peers[victim].last_damage.health_quarter_points < before,
        "{terminal:?}"
    );
    assert!(
        engine
            .ground_runtime
            .as_ref()
            .unwrap()
            .get(0)
            .unwrap()
            .health_quarter_points
            < ground_before,
        "{terminal:?}"
    );
    assert!(state(&engine).occupies(3));
    assert!(
        state(&engine)
            .snapshot_packets(1500, engine.wire_profile)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn infantry_rocket_prearm_contact_and_stale_allocation_retire_without_blast() {
    for stale in [false, true] {
        let mut engine = fixture();
        launch(&mut engine, 3, 511, 1000);
        if stale {
            engine.active.as_mut().unwrap().allocation_sequence += 1;
        }
        engine
            .tick_infantry_rockets(Duration::from_millis(1100), 1)
            .unwrap();
        assert!(audit(&engine, "infantry_rocket_terminal").is_empty());
        let retired = audit(&engine, "infantry_rocket_retired");
        assert_eq!(retired.len(), 1);
        assert_eq!(
            retired[0]["reason"],
            if stale {
                "stale_infantry_rocket_source"
            } else {
                "infantry_rocket_prearm_contact_unqualified"
            }
        );
        assert!(!state(&engine).occupies(3));
    }
}

#[test]
fn infantry_rocket_range_is_strict_and_requested_capsule_uses_the_local_spine() {
    let mut engine = fixture();
    launch(&mut engine, 3, 511, 1000);
    let mut source = flight(&engine, 3).clone();
    let Variant::Rocket(variant) = &mut source.variant else {
        panic!("rocket fixture");
    };
    variant.flight.auto_explode_distance_m = 56.;
    let step = source
        .step(2000, None, None, |shape| {
            assert_eq!(shape.spine, [[0., 0., -0.125], [0., 0., 0.125]]);
            Ok(None)
        })
        .unwrap();
    assert!(matches!(step, Step::Move(_)));
    let step = source.step(2001, None, None, |_| Ok(None)).unwrap();
    assert!(matches!(
        step,
        Step::Explode {
            reason: "range",
            ..
        }
    ));
}

#[test]
fn infantry_rocket_terminal_record_refusal_preserves_authoritative_damage_and_flight() {
    let mut engine = fixture();
    launch(&mut engine, 3, 64, 1000);
    let victim = "synthetic-rocket-target";
    let before = engine.peers[victim].last_damage.health_quarter_points;
    let health = engine
        .ground_runtime
        .as_ref()
        .unwrap()
        .get(0)
        .unwrap()
        .health_quarter_points;
    let queued = engine.peers[victim].pending_shared.clone();
    engine.max_log = engine.written;
    assert_eq!(
        engine
            .tick_infantry_rockets(Duration::from_millis(1500), 1)
            .unwrap_err(),
        "log_limit_reached"
    );
    assert_eq!(
        engine.peers[victim].last_damage.health_quarter_points,
        before
    );
    assert_eq!(
        engine
            .ground_runtime
            .as_ref()
            .unwrap()
            .get(0)
            .unwrap()
            .health_quarter_points,
        health
    );
    assert_eq!(engine.peers[victim].pending_shared, queued);
    assert_eq!(
        state(&engine)
            .snapshot_packets(1500, engine.wire_profile)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn infantry_rocket_launch_rotates_authored_velocity_and_adds_inherited_motion() {
    let engine = fixture();
    let actor = engine.active.as_ref().unwrap();
    let player = actor.reservation.player;
    let skills = skill_catalog::SkillCatalog::from_value(json!({
        "schema_version":2,"application_id":20231,
        "table":{"Skills":{"MagCharacterSkill0":{"MagCharacterSkill":{
            "Default":{"Modifier":"kCharModGrenadeThrowPower","ModifierValue":0.5},
            "Green":{"Available":true}
        }}}}
    }))
    .unwrap();
    let modifier = skills.resolve(0, &[1, 0, 0, 0, 0]).unwrap();
    let diagonal = std::f32::consts::FRAC_1_SQRT_2;
    let request = protocol::FireGrenade {
        position: [1.25, 2.5, 0.125],
        rotation_xyzw: [0., diagonal, 0., diagonal],
        initial_velocity: [1., 2., 3.],
        ammo_index: infantry_rockets::GEAR_SLOT,
        throw_power: 0.5,
        target_id: u32::MAX,
        shooter_id: 0x0380_0000 | u32::from(player),
        shooter_client_index: player,
        projectile_index: 3,
    };
    let LaunchDisposition::Created(launch) = engine
        .plan_infantry_rocket_launch(
            actor,
            request,
            &[0; REQUEST_BYTES],
            Duration::from_secs(1),
            1,
            &modifier,
        )
        .unwrap()
    else {
        panic!("first admissible shot must create a flight")
    };
    let velocity = launch.audit["velocity"].as_array().unwrap();
    for (value, expected) in velocity.iter().zip([43., 2., 3.]) {
        assert!((value.as_f64().unwrap() - expected).abs() < 1e-9);
    }
}

#[test]
fn infantry_rocket_terminal_defers_under_pressure_and_cleanup_waits_for_delivery() {
    let mut engine = fixture();
    launch(&mut engine, 3, 64, 1000);
    let victim = "synthetic-rocket-target";
    let queue = &mut engine.peers.get_mut(victim).unwrap().pending_shared;
    let original = queue.clone();
    queue.resize(multiplayer::MAX_SHARED_QUEUE, vec![]);
    let before = engine.peers[victim].last_damage.health_quarter_points;
    engine
        .tick_infantry_rockets(Duration::from_millis(1500), 1)
        .unwrap();
    assert!(audit(&engine, "infantry_rocket_terminal").is_empty());
    assert_eq!(
        engine.peers[victim].last_damage.health_quarter_points,
        before
    );
    assert!(state(&engine).slots[3].as_ref().unwrap().cleanup.is_none());

    engine.peers.get_mut(victim).unwrap().pending_shared = original;
    engine
        .tick_infantry_rockets(Duration::from_millis(1550), 1)
        .unwrap();
    assert_eq!(audit(&engine, "infantry_rocket_terminal").len(), 1);
    assert!(engine.peers[victim].last_damage.health_quarter_points < before);
    engine
        .tick_infantry_rockets(Duration::from_millis(2050), 1)
        .unwrap();
    assert!(state(&engine).occupies(3));
    assert!(audit(&engine, "infantry_rocket_retired").is_empty());

    // Deliver the queued terminal before allowing the slot removal.
    let messages = engine.drain_shared(vec![], 1).unwrap();
    assert!(!messages.is_empty());
    assert!(engine.queued_projectile_terminals().is_empty());
    engine
        .tick_infantry_rockets(Duration::from_millis(2550), 1)
        .unwrap();
    assert!(!state(&engine).occupies(3));
    assert_eq!(
        audit(&engine, "infantry_rocket_retired")[0]["reason"],
        "terminal_cleanup"
    );
}

#[test]
fn infantry_rocket_refused_launch_never_spends_a_round_or_reserves_a_slot() {
    let mut engine = fixture();
    let request = wire(&engine, 3, 64);
    engine.max_log = engine.written;
    assert_eq!(
        dispatch(&mut engine, &request, 1000).unwrap_err(),
        "log_limit_reached"
    );
    assert!(state(&engine).budget.is_none());
    assert!(!state(&engine).occupies(3));
}

#[test]
fn infantry_rocket_reuses_completed_frag_slot_only_after_removal_delivery() {
    let mut engine = fixture();
    let slot = 3;
    let frag_wire = hospital_rocket_runtime::tests::launch_wire(&engine, u16::from(slot), 4);
    let actor = engine.active.as_mut().unwrap();
    let generation = actor.life.damage_health().unwrap().0;
    let items = human_kill_reward_tests::grenade_items_for_ground_test(10.);
    actor
        .projectiles
        .admit_item_with_profile(
            slot,
            generation,
            0,
            1,
            &frag_wire,
            4,
            &items,
            engine.wire_profile,
        )
        .unwrap();
    assert_eq!(actor.projectiles.take_due(60_000, generation).len(), 1);
    assert!(actor.projectiles.get(slot).unwrap().completed);
    let removal = ProjectileRemoval {
        slot,
        object_index: actor.reservation.player,
    }
    .encode_with_profile(1, 60_000, engine.wire_profile)
    .unwrap();
    actor.queue_shared(removal);
    let request = wire(&engine, u16::from(slot), 64);
    assert!(dispatch(&mut engine, &request, 60_001).unwrap().is_empty());
    assert!(!state(&engine).occupies(slot));

    assert!(!engine.drain_shared(vec![], 1).unwrap().is_empty());
    assert!(!engine.projectile_cleanup_pending(engine.active.as_ref().unwrap(), slot));
    launch(&mut engine, u16::from(slot), 64, 60_002);
    assert_eq!(rounds(&engine), 3);
}
