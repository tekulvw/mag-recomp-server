use super::{RepairParameters, load_retail_repair_tool};
use crate::{
    CharacterLife, Engine, domination_repair_runtime,
    item_catalog::ItemCatalog,
    leadership_passives::LeadershipCatalog,
    leadership_runtime::RolePolicy,
    replication,
    resources::tests::retail_items,
    skill_catalog::SkillCatalog,
    spawn_progression::{engine as test_engine, ready, reserve},
};
use serde_json::{Value, json};
use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

const NOW: Duration = Duration::from_secs(3);

fn catalog_value() -> Value {
    json!({
        "schema_version": 1,
        "application_id": 20231,
        "native_rules": {
            "health_condition_threshold_percent": [70., 35., 12.],
            "dead_below_health": 1.,
            "minimum_rebuilt_health_percent": 10.
        },
        "resources": {
            "settings/mag_game_settings.lua": {"MAGGameRules": {"FriendlyFireDamageMultiplier": 0.25}},
            "settings/gameproperties.zpr": {"GameCreationPropertiesTable": {"m_gameCreationTypes": {
                "synthetic": {"GameCreationType": {"m_gameRules": {"FriendlyFireDamageMultiplier": 0.25}}}
            }}},
            "vehicle_damagemodel.lua": {"ArmorTypeInfoList": {"ArmorTypes": {
                "ArmorTypeInfo0": {"ArmorTypeInfo": {"rating": 1., "prevention": 0.5, "m_coverage": [1., 1., 1., 1.]}}
            }}},
            "vehicle_collision_info.lua": {"VehicleCollisionData": {"CollisionBodyMapping":
                (0..16).map(|i| (format!("CollisionBodyInfo{i}"), json!({"CollisionBodyInfo": {
                    "armorArea": 0, "hitpointArea": 0, "hitpointMult": 1.
                }}))).collect::<serde_json::Map<_, _>>()
            }}
        },
        "repair": {
            "item_id": 2,
            "parameters": {"activeitem_type": "kActiveItemRepairkit", "cooldown_time": 1.25},
            "globals": {"repairkit_repair_amount": 17., "repairkit_repair_distance": 4., "repairkit_repair_radius": 2.},
            "skill_modifiers": {}
        },
        "profiles": (0..3).map(|faction| json!({
            "vehicle_type": "DefensiveEmplacement", "faction": faction,
            "model": "synthetic", "properties_source": format!("synthetic-{faction}"),
            "damage_properties": {
                "MaxHitpoints": 200., "ArmorType": 1, "DamageCriticalHealthLossRate": 0.,
                "CanBeRepairedFromDestroyed": true, "RepairedHealthPct": 50., "TimeToRepairInSeconds": -1.
            }
        })).collect::<Vec<_>>(),
    })
}

fn load_catalog(raw: &Value) -> io::Result<RepairParameters> {
    with_catalog_file(raw, |path| load_retail_repair_tool(path))
}

pub(crate) fn synthetic_repair_bunker_world() -> crate::structures::BunkerWorld {
    // The empty bunker loader uses the prototype envelope. The test keeps
    // the same synthetic tool binding and rates in that envelope.
    synthetic_repair_bunker_world_with_parameters(17., 1.25)
}

pub(crate) fn synthetic_repair_bunker_world_with_parameters(
    amount: f32,
    cooldown_seconds: f32,
) -> crate::structures::BunkerWorld {
    let mut raw = catalog_value();
    raw["application_id"] = json!(20230);
    raw["repair"]["globals"]["repairkit_repair_amount"] = json!(amount);
    raw["repair"]["parameters"]["cooldown_time"] = json!(cooldown_seconds);
    with_catalog_file(&raw, |path| {
        crate::structures::BunkerWorld::without_bunkers(path)
    })
    .unwrap()
}

fn with_catalog_file<T>(
    raw: &Value,
    read: impl FnOnce(&std::path::Path) -> io::Result<T>,
) -> io::Result<T> {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "mag-synthetic-structural-repair-{}-{}.json",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&path, raw.to_string()).unwrap();
    let result = read(&path);
    std::fs::remove_file(path).unwrap();
    result
}

fn repair_items(binding_name: &str) -> Arc<ItemCatalog> {
    let retail = retail_items();
    let mut items = retail.raw().clone();
    items["items"]["FactionItemList0:2"]["name"] = json!(binding_name);
    items["items"]["FactionItemList0:2"]["definition"]["name"] = json!(binding_name);
    items["definitions"]["active_items"] = json!({binding_name: {"parameters": {}}});
    Arc::new(
        ItemCatalog::from_value_for_application(items, retail.character_blast_rules(), 20231)
            .unwrap(),
    )
}

fn started(mut engine: Engine<Vec<u8>>) -> (Engine<Vec<u8>>, RepairParameters) {
    reserve(&mut engine, 0);
    reserve(&mut engine, 1);
    let squad = Arc::new(engine.spawn_catalog.as_ref().unwrap().for_squad(0).unwrap());
    for actor in engine.peers.values_mut().chain(engine.active.iter_mut()) {
        ready(actor);
        actor.squad_catalog = Arc::clone(&squad);
        actor.life = CharacterLife::Alive {
            generation: 4,
            health_quarter_points: 400,
        };
        actor.owner_fields.state = Some(replication::CharacterState {
            state: 1,
            stance: 0,
            user_data: [0; 3],
        });
    }
    let tool = equip_synthetic_repair_kit(&mut engine);
    (engine, tool)
}

pub(crate) fn equip_synthetic_repair_kit(engine: &mut Engine<Vec<u8>>) -> RepairParameters {
    engine.item_catalog = repair_items("repairkit");
    let actor = engine.active.as_mut().unwrap();
    let handle = replication::InventoryHandle {
        update_id: 8,
        inventory_index: 1,
    };
    let repair_item = replication::InventoryItem {
        inventory_update_id: 8,
        gear_slot: 3,
        ammo_pool: 0,
        ammo_count: 0,
        reloading: false,
    };
    let mut inventories = std::array::from_fn(|_| replication::CharacterInventory {
        update_id: 1,
        slots: [None; 22],
    });
    inventories[1].update_id = 8;
    inventories[1].slots[3] = Some(2);
    actor.spawned_loadout = Some(replication::CharacterLoadout {
        inventories,
        handle,
        primary: repair_item,
        primary_equipped: true,
    });
    actor.current_inventory_handle = Some(handle);
    actor.observed_primary = Some(repair_item);
    actor.primary_equipped = true;
    load_catalog(&catalog_value()).unwrap()
}

fn denied(engine: &Engine<Vec<u8>>, tool: RepairParameters, error: &str) {
    assert_eq!(
        engine
            .plan_structural_repair_authority(Some(tool), NOW)
            .err(),
        Some(error)
    );
}

#[test]
fn loads_authored_retail_tool_without_a_hospital_world() {
    let tool = load_catalog(&catalog_value()).unwrap();
    assert_eq!(tool.item_id, 2);
    assert_eq!(tool.amount, 17.);
    assert_eq!(tool.distance, 4.);
    assert_eq!(tool.radius, 2.);
    assert_eq!(tool.cooldown_seconds, 1.25);
}

#[test]
fn loader_rejects_legacy_application_and_invalid_retail_tool_settings() {
    for (field, value) in [
        ("/application_id", json!(20230)),
        (
            "/repair/parameters/activeitem_type",
            json!("synthetic_tool"),
        ),
        ("/repair/parameters/cooldown_time", json!(0.)),
    ] {
        let mut raw = catalog_value();
        *raw.pointer_mut(field).unwrap() = value;
        assert_eq!(
            load_catalog(&raw).unwrap_err().kind(),
            io::ErrorKind::InvalidData,
            "{field}"
        );
    }
}

#[test]
fn requires_a_retail_repair_binding_for_the_catalog_item() {
    let (mut engine, tool) = started(test_engine());
    engine.item_catalog = repair_items("synthetic_tool");
    denied(&engine, tool, "repair_tool_not_equipped");
}

#[test]
fn requires_the_tool_in_the_observed_gear_slot_of_the_current_inventory() {
    let (mut engine, tool) = started(test_engine());
    engine
        .active
        .as_mut()
        .unwrap()
        .observed_primary
        .as_mut()
        .unwrap()
        .gear_slot = 2;
    denied(&engine, tool, "repair_tool_not_equipped");
    let actor = engine.active.as_mut().unwrap();
    actor.observed_primary.as_mut().unwrap().gear_slot = 3;
    actor
        .current_inventory_handle
        .as_mut()
        .unwrap()
        .inventory_index = 0;
    denied(&engine, tool, "repair_tool_not_equipped");
}

#[test]
fn uses_the_currently_selected_primary_or_secondary_generation() {
    let (mut engine, tool) = started(test_engine());
    let actor = engine.active.as_mut().unwrap();
    actor.primary_equipped = false;
    actor.observed_secondary = actor.observed_primary;
    actor
        .observed_secondary
        .as_mut()
        .unwrap()
        .inventory_update_id = 7;
    denied(&engine, tool, "repair_tool_not_equipped");
    let actor = engine.active.as_mut().unwrap();
    actor
        .observed_secondary
        .as_mut()
        .unwrap()
        .inventory_update_id = 8;
    actor.observed_primary = None;
    let (authority, _) = engine
        .plan_structural_repair_authority(Some(tool), NOW)
        .unwrap();
    assert_eq!(authority.actor.player, 1);
    assert_eq!(authority.actor.generation, 4);
    assert_eq!(authority.actor.team, 0);
    assert_eq!(authority.repair_rate, 1.25);
}

#[test]
fn requires_live_ready_onfoot_actor_in_open_round() {
    let (mut engine, tool) = started(test_engine());
    engine.active.as_mut().unwrap().life = CharacterLife::AwaitingRespawn { generation: 4 };
    denied(&engine, tool, "structural_repair_requires_live_actor");

    let (mut engine, tool) = started(test_engine());
    engine.active.as_mut().unwrap().ready_for_updates = false;
    denied(&engine, tool, "structural_repair_requires_combat_actor");
    let actor = engine.active.as_mut().unwrap();
    actor.ready_for_updates = true;
    actor.mounted_seat = Some(0);
    denied(&engine, tool, "structural_repair_requires_combat_actor");
    engine.active.as_mut().unwrap().mounted_seat = None;
    engine.round.outcome = Some(crate::match_flow::Outcome {
        winner: 1,
        major: false,
        deadline: NOW,
        finalized: false,
    });
    denied(&engine, tool, "structural_repair_requires_combat_actor");
}

#[test]
fn ready_actor_requires_a_source_and_current_inventory() {
    let (mut engine, tool) = started(test_engine());
    assert_eq!(
        engine.plan_structural_repair_authority(None, NOW).err(),
        Some("structural_repair_tool_source_required")
    );
    engine.active.as_mut().unwrap().current_inventory_handle = None;
    denied(&engine, tool, "repair_inventory_unavailable");
}

#[test]
fn repair_contribution_uses_owned_repair_modifier_in_leadership_plan() {
    let catalog = LeadershipCatalog::from_json(&json!({
        "schema_version": 1, "application_id": 20231,
        "source": {"member": "scripts/character_leadership_abilities.zpr",
            "member_sha256": "a".repeat(64), "archive_sha256": "b".repeat(64),
            "extractor": "synthetic", "projector": "synthetic"},
        "max_separation_range": 15., "refresh_hold_ms": 400, "removal_grace_ms": 200,
        "levels": [
            {"level": 0, "name": "none", "modifiers": []},
            {"level": 1, "name": "synthetic squad", "modifiers": [{"kind": "kCharModDemolitionTime", "value": 0.5}]},
            {"level": 2, "name": "synthetic platoon", "modifiers": [{"kind": "kCharModReloadTime", "value": 0.1}]},
            {"level": 3, "name": "synthetic officer", "modifiers": [{"kind": "kCharModAutoHeal", "value": 0.1}]}
        ]
    }).to_string()).unwrap();
    let policy = RolePolicy::from_json(
        &json!({
            "schema_version": 1, "application_id": 20231,
            "first_member_squad_leader": true, "development_bindings": []
        })
        .to_string(),
    )
    .unwrap();
    let (mut engine, tool) = started(
        test_engine()
            .with_leadership_passives(catalog, policy)
            .unwrap(),
    );
    let skills = SkillCatalog::from_value(json!({
        "schema_version": 2, "application_id": 20231,
        "table": {"Skills": {"MagCharacterSkill1": {"MagCharacterSkill": {
            "Default": {"Modifier": "kCharModRepairAmount", "ModifierValue": 0.25},
            "Green": {"Available": true}
        }}}}
    }))
    .unwrap();
    engine.active.as_mut().unwrap().modifiers = skills.resolve(0, &[2, 0, 0, 0, 0]).unwrap();
    let (authority, leadership) = engine
        .plan_structural_repair_authority(Some(tool), NOW)
        .unwrap();
    let mut repair = domination_repair_runtime::StructuralRepair::new(
        replication::ObjectiveState::Completed,
        domination_repair_runtime::Rules {
            owner_team: 0,
            can_repair: true,
            repair_ms: 5_000,
            minimum_repair_ms: 1_000,
        },
    )
    .unwrap();

    // 1.25 seconds * (1 + .25 owned) * 100 / 5 seconds.
    assert_eq!(repair.submit(authority, 30.).unwrap().amount, 31.25);
    assert!(engine.active.as_ref().unwrap().leadership.is_none());
    engine.commit_leadership(leadership);
    let actor = engine.active.as_ref().unwrap();
    assert_eq!(actor.leadership.unwrap().state.level(), 1);
    assert_eq!(actor.modifiers.value("kCharModRepairAmount"), 0.25);
}

#[test]
fn only_timing_races_become_repair_denials() {
    for code in super::STRUCTURAL_REPAIR_RACE_DENIALS {
        assert!(super::is_structural_repair_race_denial(code));
    }
    // Off-team tools, unknown targets, bad state and sequence exhaustion
    // are not timing races and still end the event as an error.
    for code in [
        "structural_repair_requires_live_owner",
        "structural_repair_tool_source_required",
        "objective_invalid_target",
        "unknown_native_objective",
        "invalid_structural_damage_model_health",
        "sequence_exhausted",
    ] {
        assert!(!super::is_structural_repair_race_denial(code), "{code}");
    }
}
