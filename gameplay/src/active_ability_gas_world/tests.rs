use super::*;
use crate::active_ability_effects::BlastFalloff;

fn cloud() -> GasVolume {
    GasVolume {
        origin: [0., 0., 0.],
        duration_ms: 30_000,
        damage_per_second: 30.,
        falloff: BlastFalloff {
            inner_radius: 20.,
            inner_scale: 0.75,
            outer_radius: 20.,
            outer_scale: 1.,
        },
    }
}

#[test]
fn volume_pulses_once_per_update_and_retains_remainder() {
    let mut timing = PulseTiming::new(Duration::ZERO, cloud().duration_ms);
    assert_eq!(timing.advance(Duration::from_millis(499)), Pulse::Waiting);
    assert_eq!(timing.advance(Duration::from_millis(1500)), Pulse::Damage);
    assert_eq!(timing.advance(Duration::from_millis(1501)), Pulse::Damage);
    assert_eq!(timing.advance(Duration::from_millis(1502)), Pulse::Damage);
    assert_eq!(timing.advance(Duration::from_millis(1503)), Pulse::Waiting);
    assert_eq!(timing.advance(Duration::from_millis(30000)), Pulse::Damage);
    assert_eq!(timing.advance(Duration::from_millis(30001)), Pulse::Expired);
}

#[test]
fn overlapping_clouds_share_cooldown_until_exactly_half_a_second() {
    let mut world = GasCooldowns::default();
    let victim = Target {
        character_id: 0x0380_0005,
        generation: 8,
        world: 3,
    };
    let first = Duration::from_secs(2);
    let allocation = TargetAllocation::Human(4, "first".into());
    world.mark_applied(1, victim, allocation.clone(), first);
    assert!(world.cooling_down(1, victim, &allocation, first));
    assert!(world.cooling_down(1, victim, &allocation, first + Duration::from_millis(499)));
    assert!(!world.cooling_down(1, victim, &allocation, first + Duration::from_millis(500)));
    assert!(!world.cooling_down(
        1,
        victim,
        &TargetAllocation::Human(5, "first".into()),
        first
    ));
    assert!(!world.cooling_down(2, victim, &allocation, first));
    assert!(!world.cooling_down(
        1,
        Target {
            generation: 9,
            ..victim
        },
        &allocation,
        first,
    ));
    world.mark_applied(1, victim, TargetAllocation::Bot(7), first);
    assert!(!world.cooling_down(1, victim, &TargetAllocation::Bot(8), first));
}

#[test]
fn overlapping_gas_hits_new_occupant_once_without_crediting_reused_source_slot() {
    use crate::{
        active_ability_engine_tests::{actor_mut, add_ready, configured},
        dynamic_collision, dynamic_collision_runtime, protocol, replication,
        synthetic_dynamic_collision,
    };
    let mut engine = configured()
        .with_dynamic_collision_bodies(
            dynamic_collision::Catalog::from_json(
                &synthetic_dynamic_collision::fixture().to_string(),
            )
            .unwrap(),
            dynamic_collision_runtime::RuntimeBindings {
                character_template: "invented-character".into(),
                vehicles: Vec::new(),
            },
        )
        .unwrap();
    engine.item_catalog = crate::human_kill_reward_tests::combat_items_for_ground_test(0.);
    add_ready(&mut engine, 0);
    let rules = crate::character_damage_rules::Rules::from_json(
        &json!({
            "schema_version":1, "application_id":20231, "map_name":engine.map_name(),
            "game_type":2, "friendly_fire_multiplier":0.25, "provenance":{
                "spawn_catalog_sha256":engine.spawn_catalog.as_ref().unwrap().source_sha256().unwrap(),
                "source":"synthetic",
            }
        }).to_string(),
    ).unwrap();
    engine = engine.with_character_damage_rules(rules).unwrap();
    let items = std::sync::Arc::clone(&engine.item_catalog);
    let actor = actor_mut(&mut engine, 0);
    let source = GasSource {
        allocation: None,
        attacker: 0x0380_0000,
        world: actor.reservation.world,
        team: actor.team(),
        reservation: actor.reservation.id.clone(),
    };
    actor.reservation.id = "synthetic-replacement".to_owned();
    actor
        .reservation
        .character
        .loadouts
        .as_mut()
        .unwrap()
        .inventories[0][12] = Some(3);
    actor.current_inventory_handle = Some(replication::InventoryHandle {
        update_id: 1,
        inventory_index: 0,
    });
    actor.spawned_loadout = Some(
        actor
            .reservation
            .character
            .replication_loadout(&items, 0, actor.wire_profile)
            .unwrap(),
    );

    let mut gas = GasWorld::default();
    let volume = GasVolume {
        origin: [100., 3., 100.],
        ..cloud()
    };
    gas.register(volume, source.clone(), Duration::ZERO)
        .unwrap();
    gas.register(volume, source, Duration::ZERO).unwrap();
    let mut native = BTreeMap::from([(
        0x0380_0000,
        NativeState {
            invulnerability: Some(0),
            associated_vehicle: Some(protocol::VictimWasMounted::NoAssociatedVehicle),
        },
    )]);
    let mut filter_rows = [0; 32];
    filter_rows[6] = 1 << 21;
    let now = Duration::from_millis(500);
    let candidate = engine
        .prepare_acquisition_character_callback(&native, now, |engine, native| {
            gas.stage(
                engine,
                native,
                now,
                BlastCollision {
                    filter_rows: &filter_rows,
                    limits: Default::default(),
                    sequences: &Default::default(),
                    material_costs: &Default::default(),
                    terrain_materials: None,
                },
            )
        })
        .unwrap();
    let hits = candidate
        .audit()
        .iter()
        .filter(|entry| entry["on_character_hit"] == true)
        .collect::<Vec<_>>();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0]["damage_source_type"], 7);
    assert_eq!(hits[0]["attacker"], u32::MAX);
    assert_eq!(hits[0]["calculation"]["incoming_damage_base"], 1.);
    assert_eq!(
        actor_mut(&mut engine, 0).life.damage_health(),
        Some((1, 400))
    );

    engine
        .install_acquisition_character_effects(candidate, &mut native)
        .unwrap();
    let damaged = actor_mut(&mut engine, 0);
    assert_eq!(damaged.last_damage.last_attacker, u32::MAX);
    assert_eq!(damaged.last_damage.last_location_and_type, 0x41);
    assert!(damaged.life.damage_health().unwrap().1 < 400);
}

#[test]
fn grenade_warmup_consumes_early_pulse_and_expiry_is_strict() {
    let mut timing = PulseTiming::grenade(
        Duration::ZERO,
        30_000,
        Duration::from_secs(1),
        Duration::from_millis(500),
    );
    for (ms, expected) in [
        (500, Pulse::Waiting),
        (999, Pulse::Waiting),
        (1_000, Pulse::Damage),
        (1_001, Pulse::Waiting),
        (1_500, Pulse::Damage),
        (30_000, Pulse::Damage),
        (30_001, Pulse::Expired),
    ] {
        assert_eq!(timing.advance(Duration::from_millis(ms)), expected, "{ms}");
    }
}
