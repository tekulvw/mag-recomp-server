use super::*;
use crate::{
    blast_rules::CharacterBlastRules, bootstrap_loadouts::BootstrapLoadouts,
    item_catalog::ItemCatalog, retail_combat::RetailCombatCatalog,
};
use serde_json::{Map, Value, json};
use std::time::Duration;

struct Fixture {
    rules: Value,
    items: Value,
    combat: Value,
}

fn item_id(faction: u8) -> u16 {
    (u16::from(faction) + 1) * 1024 + 65
}

fn item(faction: u8, ordinal: u8, name: &str, kind: &str, group: &str) -> (String, Value) {
    let namespace = format!("FactionItemList{}", faction + 1);
    let reference = format!("{namespace}:{ordinal}");
    (
        reference.clone(),
        json!({
            "authored_ref": reference, "name": name, "item_type": kind,
            "faction_list": namespace, "local_ordinal": ordinal, "wire_item_id": null,
            "definition": {"group": group, "name": name},
        }),
    )
}

impl Fixture {
    fn new() -> Self {
        let catalog_sha = "a".repeat(64);
        let bindings_sha = "b".repeat(64);
        let bootstrap_sha = "c".repeat(64);
        let mut items = Map::new();
        let mut weapons = Map::new();
        let mut attachments = Map::new();
        let mut ammo = Map::new();
        let mut variants = Vec::new();
        for faction in 0..3 {
            let name = format!("synthetic_launcher_{faction}");
            let sight = format!("synthetic_sight_{faction}");
            let round = format!("synthetic_ammo_{faction}");
            let (weapon_ref, weapon_item) = item(faction, 15, &name, "kWeapon", "weapons");
            let (sight_ref, sight_item) =
                item(faction, 16, &sight, "kAttachmentItem", "attachments");
            items.insert(weapon_ref, weapon_item);
            items.insert(sight_ref, sight_item);
            weapons.insert(
                name.clone(),
                json!({
                    "parameters": {"id": name, "ammo_name": round, "category": "kWepCategoryRocketLauncher",
                        "fire_param_id": "kFireParamSecondary", "initial_fire_mode": "kSemiAuto",
                        "infinite_ammo": false, "bSupportLightAmmoPack": false,
                        "use_turret_mesh": false, "alternate_firepoint": "",
                        "required_sight_attachment": sight,
                        "attachment_list": {"WeaponAttachment0": {"WeaponAttachment": {"name": sight}}},
                        "magazine_capacity": 1, "initial_magazine_count": 4,
                        "fire_mode": {"refire_time": 0.55, "reload_time": 3.25,
                            "fire_modes": {"FireModeTypeClass0": {
                                "FireModeTypeClass": {"fire_mode_value": "kSemiAuto"}
                            }}
                        }
                    }
                }),
            );
            attachments.insert(sight.clone(), json!({"parameters":{"id": sight}}));
            ammo.insert(
                round.clone(),
                json!({
                    "parameters": {"id": round, "collisionType": "kWepProjCollisionShapeCapsule",
                        "propulsion_strategy": "kWepPropulsionStrategyRocketDumb",
                        "detonation_strategy": "kWepDetonationStrategyImpact",
                        "explosion_strategy": "kWepExplosionStrategyDebris",
                        "rocketDumbDirectionChangeRange": [0, 0],
                        "rocketDumbCruiseStraightRange": 2.5, "rocketAutoExplodeDistance": 75.,
                        "detonation_timer": 0, "capsuleHeight": [0, 0, 0.25], "launch_velocity": 56.,
                        "radius": 0.09375, "m_fArmedTime": 0.1875, "m_fLifetimeMax": 7.,
                        "half_damage_radius": 1.5, "full_damage_radius": 3.75, "MaterialPenetration": 1.25,
                        "damage": {"character": 87., "vehicle": 763., "vehicleAPierce": 0, "vehicleARedux": 0}
                    }
                }),
            );
            let ammo_token = [0xef41_fca6_u32, 0x9846_cc30, 0x014f_9d8a][usize::from(faction)];
            variants.push(json!({
                "faction": faction, "item_id": item_id(faction), "stock_sight_item_id": item_id(faction) + 1,
                "ammo_token": ammo_token,
                "weapon": {"magazine_size":1,"initial_magazine_count":4,"refire_seconds":0.55,"reload_seconds":3.25},
                "flight": {"radius_m":0.09375,"capsule_spine_local_xyz":[0,0,0.25],"speed_mps":56.,
                    "armed_time_seconds":0.1875,"auto_explode_distance_m":75.,"lifetime_seconds":7.,"age_grace_seconds":2.},
                "blast": {"half_radius_m":1.5,"outer_radius_m":3.75,"material_penetration":1.25,"character_hp":87.,"vehicle_hp":763.}
            }));
        }
        let rules = json!({
            "schema": "mag_retail_infantry_rockets_v1", "application_id":20231,
            "source": {"item_catalog_sha256": catalog_sha,"item_bindings_sha256": bindings_sha,
                "bootstrap_sha256": bootstrap_sha, "native_proof_sha256":"d".repeat(64),
                "retail_elf_sha256":RETAIL_ELF_SHA256,"retail_archive_sha256":RETAIL_GAME_SHA256},
            "variants": variants
        });
        let items = json!({
            "schema_version": 2, "application_id": 20231,
            "build": {"source_archive_sha256": RETAIL_GAME_SHA256},
            "items": items, "definitions": {"weapons":weapons,"attachments":attachments,"projectile_ammo":ammo}
        });
        let combat = json!({
            "schema_version":2,"application_id":20231,"status":"synthetic",
            "authority":{"bullet_damage":true,"armor_reduction":true,"body_damage":true,
                "grenade_damage":false,"grenade_physics":false,"healing_schedule":false,"ammo_tokens":false},
            "provenance":{"catalog_sha256":catalog_sha,"binding_report_sha256":bindings_sha,
                "source_archive_sha256":RETAIL_GAME_SHA256,"original_retail_elf_sha256":RETAIL_ELF_SHA256,
                "bootstrap_sha256":bootstrap_sha},
            "bullets":{},"armor":{},"grenades":{},"healing":{},"unsupported":{},
            "body":(0..16).map(|location| json!({"location":location,"armor_area":0,
                "multiplier":1.0,"source":{}})).collect::<Vec<_>>(),"limitations":[]
        });
        Self {
            rules,
            items,
            combat,
        }
    }

    fn items(&self) -> ItemCatalog {
        let armor = CharacterBlastRules {
            armor_rating_reduction: 0.0,
            armor_prevention_reduction: 0.0,
            hit_location: 0,
            damage_type: 10,
        };
        ItemCatalog::from_value_for_application(self.items.clone(), armor, 20231)
            .unwrap()
            .with_retail_combat(RetailCombatCatalog::from_json(&self.combat.to_string()).unwrap())
    }

    fn load(&self) -> Result<Catalog, &'static str> {
        Catalog::from_bytes(
            self.rules.to_string().as_bytes(),
            &self.items(),
            RETAIL_ELF_SHA256,
        )
    }
}

pub(crate) fn synthetic_catalog() -> Catalog {
    Fixture::new().load().unwrap()
}

#[test]
fn source_join_admits_three_stock_launchers_without_requiring_an_installed_bootstrap() {
    let catalog = synthetic_catalog();
    assert_eq!(catalog.variants().len(), 3);
    for faction in 0..3 {
        let variant = catalog.variant(faction, item_id(faction)).unwrap();
        assert_eq!(variant.stock_sight_item_id, item_id(faction) + 1);
        assert_eq!(variant.flight.capsule_spine_local_xyz, [0., 0., 0.25]);
        assert_eq!(variant.blast.vehicle_hp, 763.0);
        assert!(
            catalog
                .variant(faction, item_id((faction + 1) % 3))
                .is_none()
        );
    }
    assert!(catalog.variant(3, item_id(0)).is_none());
}

#[test]
fn mismatched_source_provenance_and_bad_schema_are_refused() {
    for field in [
        "item_catalog_sha256",
        "item_bindings_sha256",
        "bootstrap_sha256",
        "native_proof_sha256",
        "retail_archive_sha256",
        "retail_elf_sha256",
    ] {
        let mut fixture = Fixture::new();
        fixture.rules["source"][field] = json!("unexpected");
        assert_eq!(
            fixture.load().unwrap_err(),
            "infantry_rocket_source_mismatch",
            "{field}"
        );
    }
    let fixture = Fixture::new();
    assert_eq!(
        Catalog::from_bytes(
            fixture.rules.to_string().as_bytes(),
            &fixture.items(),
            "development"
        )
        .unwrap_err(),
        "infantry_rocket_source_mismatch"
    );
    let duplicate = fixture.rules.to_string().replacen(
        "\"application_id\":20231",
        "\"application_id\":20231,\"application_id\":20231",
        1,
    );
    assert_eq!(
        Catalog::from_bytes(duplicate.as_bytes(), &fixture.items(), RETAIL_ELF_SHA256).unwrap_err(),
        "invalid_infantry_rocket_rules"
    );
}

#[test]
fn roles_and_ballistics_have_to_match_loaded_item_definitions() {
    for (field, replacement) in [
        ("/variants/0/item_id", json!(item_id(1))),
        ("/variants/0/stock_sight_item_id", json!(item_id(0))),
        ("/variants/0/stock_sight_item_id", json!(item_id(1) + 1)),
        ("/variants/0/weapon/initial_magazine_count", json!(8)),
        (
            "/variants/0/flight/capsule_spine_local_xyz",
            json!([0, 0, 0.5]),
        ),
        ("/variants/0/flight/armed_time_seconds", json!(0.0625)),
        ("/variants/0/flight/auto_explode_distance_m", json!(200)),
        ("/variants/0/flight/age_grace_seconds", json!(0.5)),
        ("/variants/0/ammo_token", json!(0xef41_fca7_u32)),
        ("/variants/0/blast/vehicle_hp", json!(1000)),
    ] {
        let mut fixture = Fixture::new();
        *fixture.rules.pointer_mut(field).unwrap() = replacement;
        assert!(fixture.load().is_err(), "{field}");
    }
    for (field, value) in [
        ("collisionType", json!("kWepProjCollisionShapeSphere")),
        ("rocketDumbDirectionChangeRange", json!([0., 0.25])),
        ("detonation_timer", json!(0.5)),
    ] {
        let mut fixture = Fixture::new();
        fixture.items["definitions"]["projectile_ammo"]["synthetic_ammo_0"]["parameters"][field] =
            value;
        assert_eq!(
            fixture.load().unwrap_err(),
            "infantry_rocket_ammo_mismatch",
            "{field}"
        );
    }
    let mut fixture = Fixture::new();
    fixture.rules["variants"][0]["blast"]["objective_hp"] = json!(800);
    assert_eq!(fixture.load().unwrap_err(), "invalid_infantry_rocket_rules");
}

#[test]
fn loaded_bootstrap_must_keep_the_stock_sight_binding() {
    let fixture = Fixture::new();
    let mut rigged = [None; 22];
    rigged[0] = Some(item_id(0));
    rigged[1] = Some(item_id(0));
    rigged[2] = Some(item_id(0));
    rigged[3] = Some(item_id(0));
    let stock = json!({"update_id":1,"items":rigged,"primary_ammo":1});
    let empty = json!({"update_id":1,"items":vec![None::<u16>;22],"primary_ammo":0});
    let mut bootstrap = json!({
        "schema_version":1,"application_id":20231,
        "provenance":{"verified_binding":{"validated":true,"positions_compared":667,
            "blank_positions_compared":25,"blank_zero_empirically_verified":true}},
        "factions":[{"faction":0,"class_ref":"CharacterClassInfo0",
            "loadouts":[stock,stock,stock,empty,empty]}]
    });
    for field in [
        "original_retail_elf_sha256",
        "snapshot_sha256",
        "catalog_sha256",
        "rules_sha256",
        "owner_attestation_sha256",
        "gear_slots_sha256",
        "comparator_sha256",
    ] {
        bootstrap["provenance"][field] = json!("e".repeat(64));
    }
    let items = fixture
        .items()
        .with_bootstrap_loadouts(BootstrapLoadouts::from_json(&bootstrap.to_string()).unwrap());
    assert_eq!(
        Catalog::from_bytes(
            fixture.rules.to_string().as_bytes(),
            &items,
            RETAIL_ELF_SHA256
        )
        .unwrap_err(),
        "infantry_rocket_role_mismatch"
    );
}

#[test]
fn finite_round_budget_debits_only_the_candidate_and_refills_without_erasing_refire() {
    let weapon = synthetic_catalog().variants()[0].weapon;
    let start = RoundBudget::new(weapon).unwrap();
    let first = start.after_shot(Duration::from_secs(10)).unwrap();
    assert_eq!(start.remaining(), 4);
    assert_eq!(first.remaining(), 3);
    assert_eq!(
        first
            .refilled()
            .after_shot(Duration::from_millis(10_549))
            .unwrap_err(),
        "infantry_rocket_refire"
    );
    let second = first.after_shot(Duration::from_millis(10_550)).unwrap();
    let third = second.after_shot(Duration::from_millis(11_100)).unwrap();
    let empty = third.after_shot(Duration::from_millis(11_650)).unwrap();
    assert_eq!(empty.remaining(), 0);
    assert_eq!(
        empty.after_shot(Duration::from_secs(30)).unwrap_err(),
        "infantry_rocket_ammo_empty"
    );
    assert_eq!(empty.refilled().remaining(), 4);
    assert_eq!(
        first.after_shot(Duration::from_secs(9)).unwrap_err(),
        "infantry_rocket_clock_reversed"
    );
}
