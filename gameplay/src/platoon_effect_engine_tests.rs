//! Connected command, scheduler, world query and transaction regressions.
use super::*;
use crate::{
    active_abilities::{Ability, Side},
    active_ability_effects::{
        BlastFalloff, EffectRules, GasBombardmentRules, GuidedArtilleryRules, StrafingRunRules,
    },
    active_ability_engine_tests::{
        activate, actor_mut, configured, prepare_effect_tick, request, strike_floor,
    },
    platoon_ability_engine_tests::{last_decision, platoon_engine},
};

use std::{collections::BTreeMap, sync::Arc};

const CALLER: u32 = 0x0380_0000;

fn command_engine(rules: EffectRules) -> Engine<Vec<u8>> {
    let mut engine = platoon_engine();
    let setup = Arc::make_mut(&mut engine.active_abilities.as_mut().unwrap().setup);
    for mode in &mut setup.rules.modes {
        if mode.players == 256 && mode.side == Side::Defender {
            mode.platoon = vec![rules.ability()];
        }
    }
    setup
        .effects
        .push((Side::Defender, 1, rules.ability(), rules));

    let geometry = configured()
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
    engine.dynamic_body_resources = geometry.dynamic_body_resources;
    if rules.ability() == Ability::StrafingRun {
        let spawns = spawn_catalog::SpawnCatalog::from_json(
            &json!({
                "format":"mag_spawn_catalog","schema_version":1,"source":"synthetic alternate spawn",
                "active":{"map_name":engine.map_name(),"squad_index":16,"default_spawn":0,"available":[0,1]},
                "match_profile":{"game_type":2,"auto_assign_teams":true,"team_capacity":8,
                    "squad_capacity":4,"has_bunkers":false,"has_objectives":false,"attacking_faction":0},
                "maps":[{"name":engine.map_name(),"owner_faction":1,"spawn_count":2,
                    "locations":[
                        {"index":0,"token":71,"kind":"on_foot","position":{"x":100.,"y":3.,"z":100.}},
                        {"index":1,"token":72,"kind":"alternate","position":{"x":90.,"y":3.,"z":100.}}
                    ],
                    "squads":(0..2).flat_map(|team| (0..2).map(move |squad| json!({
                        "index":team*16+squad,"team":team,"platoon":0,"squad":squad,
                        "faction":if team==1 { Some(1) } else { None },"available":[0,1],"default_spawn":0
                    }))).collect::<Vec<_>>() }]
            })
            .to_string(),
        )
        .unwrap();
        engine.spawn_catalog = Some(spawns);
    }
    engine.item_catalog = crate::human_kill_reward_tests::combat_items_for_ground_test(0.);
    let rules = character_damage_rules::Rules::from_json(
        &json!({
            "schema_version":1,"application_id":20231,"map_name":engine.map_name(),
            "game_type":2,"friendly_fire_multiplier":0.5,"provenance":{
                "spawn_catalog_sha256":engine.spawn_catalog.as_ref().unwrap().source_sha256().unwrap(),
                "source":"synthetic"}
        })
        .to_string(),
    )
    .unwrap();
    engine = engine.with_character_damage_rules(rules).unwrap();
    let items = Arc::clone(&engine.item_catalog);
    for slot in 0..2 {
        let actor = actor_mut(&mut engine, slot);
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
        actor.spawned_loadout =
            actor
                .reservation
                .character
                .replication_loadout(&items, 0, actor.wire_profile);
        actor.tracking.server_position(
            replication::CharacterPosition {
                x: 100.,
                y: 3.,
                z: 100.,
            },
            player_tracking::PositionOrigin::ServerExit,
            20,
        );
    }
    engine.collision_world = Some(strike_floor(engine.map_name(), true));
    engine
}

fn cast(engine: &mut Engine<Vec<u8>>, ability: Ability) {
    let mut command = request(16, ability);
    command.position = [100., 3., 100.];
    assert!(activate(engine, command, 0, 20).ok);
    let decision = last_decision(engine);
    assert_eq!(decision["decoded"]["accepted"], true, "{decision}");
}

fn native_states() -> BTreeMap<u32, acquisition_character_effects::NativeState> {
    (CALLER..=CALLER + 1)
        .map(|id| {
            (
                id,
                acquisition_character_effects::NativeState {
                    invulnerability: Some(0),
                    associated_vehicle: Some(protocol::VictimWasMounted::NoAssociatedVehicle),
                },
            )
        })
        .collect()
}

fn commit_tick(
    engine: &mut Engine<Vec<u8>>,
    native: &mut BTreeMap<u32, acquisition_character_effects::NativeState>,
    millis: u64,
) -> Vec<Value> {
    let update = prepare_effect_tick(engine, native, millis).unwrap();
    let audit = update.audit().to_vec();
    engine
        .record_acquisition_character_effects(update, native, 1)
        .unwrap();
    audit
}

#[test]
fn platoon_guided_point_query_spends_once_without_character_damage() {
    let mut engine = command_engine(EffectRules::GuidedArtillery(GuidedArtilleryRules {
        delay_ms: 5,
        damage_delay_ms: 2,
        effective_radius: 8.,
    }));
    let mut native = native_states();
    cast(&mut engine, Ability::GuidedArtillery);

    // No static scene is necessary. The first due callback still needs the
    // dynamic overlap, even though the only actors in this fixture are characters.
    engine.collision_world = None;
    let resources = engine.dynamic_body_resources.take();
    commit_tick(&mut engine, &mut native, 26);
    assert!(matches!(
        prepare_effect_tick(&mut engine, &native, 27),
        Err("dynamic_body_resources_unavailable")
    ));
    engine.dynamic_body_resources = resources;
    let audit = commit_tick(&mut engine, &mut native, 27);
    assert!(
        audit
            .iter()
            .all(|event| event["on_character_hit"].is_null())
    );
    assert_eq!(
        actor_mut(&mut engine, 0).life.damage_health(),
        Some((1, 400))
    );
    assert_eq!(
        actor_mut(&mut engine, 1).life.damage_health(),
        Some((1, 400))
    );

    engine.dynamic_body_resources = None;
    commit_tick(&mut engine, &mut native, 28);
}

#[test]
fn platoon_gas_command_registers_a_delayed_chemical_pulse() {
    let mut engine = command_engine(EffectRules::GasBombardment(GasBombardmentRules {
        delay_ms: 7,
        damage_delay_ms: 0,
        duration_ms: 1_000,
        interval_ms: 900,
        num_bomblets: 1,
        bomblet_damage: 5.,
        bomblet: BlastFalloff {
            outer_radius: 4.,
            outer_scale: 1.,
            inner_radius: 2.,
            inner_scale: 0.5,
        },
        gas_damage_per_second: 50.,
        gas: BlastFalloff {
            outer_radius: 8.,
            outer_scale: 1.,
            inner_radius: 8.,
            inner_scale: 0.75,
        },
    }));
    let mut native = native_states();
    cast(&mut engine, Ability::GasBombardment);
    let first = commit_tick(&mut engine, &mut native, 27);
    assert!(first.iter().any(|event| event["damage_source_type"] == 4));
    let before_pulse = actor_mut(&mut engine, 0).life.damage_health().unwrap().1;
    let early = commit_tick(&mut engine, &mut native, 526);
    assert!(
        early
            .iter()
            .all(|event| event["op"] != "gas_bombardment_pulse")
    );
    assert_eq!(
        actor_mut(&mut engine, 0).life.damage_health().unwrap().1,
        before_pulse
    );

    let pulse = commit_tick(&mut engine, &mut native, 527);
    assert!(
        pulse
            .iter()
            .any(|event| event["op"] == "gas_bombardment_pulse")
    );
    assert!(pulse.iter().any(|event| event["damage_source_type"] == 7
        && event["attacker"] == CALLER
        && event["outcome"] == "Applied"));
    assert!(actor_mut(&mut engine, 0).life.damage_health().unwrap().1 < before_pulse);
}

#[test]
fn platoon_strafe_retries_one_due_impact_after_spatial_rollback() {
    let mut engine = command_engine(EffectRules::StrafingRun(StrafingRunRules {
        delay_ms: 5,
        damage_delay_ms: 1,
        duration_ms: 60,
        interval_ms: 20,
        strafe_length: 2.,
        deviation_radius: 0.,
        bomblet_damage: 30.,
        bomblet: BlastFalloff {
            outer_radius: 6.,
            outer_scale: 0.,
            inner_radius: 2.,
            inner_scale: 0.5,
        },
    }));
    let mut native = native_states();
    cast(&mut engine, Ability::StrafingRun);
    commit_tick(&mut engine, &mut native, 21);
    let early = commit_tick(&mut engine, &mut native, 26);
    assert!(
        early
            .iter()
            .all(|event| event["on_character_hit"].is_null())
    );

    engine.collision_world = Some(strike_floor(engine.map_name(), false));
    assert!(matches!(
        prepare_effect_tick(&mut engine, &native, 27),
        Err("air_strike_static_impact_ray_failed")
    ));
    assert_eq!(
        actor_mut(&mut engine, 0).life.damage_health(),
        Some((1, 400))
    );

    engine.collision_world = Some(strike_floor(engine.map_name(), true));
    let impact = prepare_effect_tick(&mut engine, &native, 27).unwrap();
    assert!(
        impact
            .audit()
            .iter()
            .any(|event| event["damage_source_type"] == 4
                && event["attacker"] == CALLER
                && event["outcome"] == "Applied")
    );
    assert_eq!(
        actor_mut(&mut engine, 0).life.damage_health(),
        Some((1, 400))
    );
    engine
        .record_acquisition_character_effects(impact, &mut native, 1)
        .unwrap();
    assert!(actor_mut(&mut engine, 0).life.damage_health().unwrap().1 < 400);
    assert!(
        commit_tick(&mut engine, &mut native, 28)
            .iter()
            .all(|event| event["on_character_hit"].is_null())
    );
}
