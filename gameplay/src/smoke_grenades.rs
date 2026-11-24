//! Source-qualified, nondamaging retail smoke grenade equipment.

use crate::{
    hospital_rockets::ammo_crc32,
    item_catalog::{DamageParameters, GrenadeParameters, ItemCatalog},
};
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeSet;

pub const GEAR_SLOT: usize = 4;
const MARKSMAN: u8 = 1;
const APPLICATION: u32 = 20231;
const ELF: &str = "0089f0d6d17e48533ce7ff8d943dddd03487b925e7b04a1a055bfb907568694c";
const ARCHIVE: &str = "b49c9435be729832029ad7019874d478080dc9764ebde1809b2851d353bc769f";

#[derive(Clone, Debug)]
pub struct Catalog {
    variants: [Variant; 3],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    schema: String,
    application_id: u32,
    source: Source,
    variants: [Variant; 3],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    retail_elf_sha256: String,
    retail_archive_sha256: String,
    item_catalog_sha256: String,
    item_bindings_sha256: String,
    bootstrap_sha256: String,
    native_proof_sha256: String,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Variant {
    pub faction: u8,
    pub item_id: u16,
    pub ammo_token: u32,
    pub initial_count: u8,
    pub fuse_ms: u64,
    pub projectile_lifetime_ms: u64,
    pub volume_duration_ms: u64,
}

impl Catalog {
    #[cfg(test)]
    pub(crate) fn synthetic(variants: [Variant; 3]) -> Self {
        Self { variants }
    }

    pub fn from_bytes(
        raw: &[u8],
        items: &ItemCatalog,
        native_elf: &str,
    ) -> Result<Self, &'static str> {
        if raw.len() > 1 << 20 {
            return Err("invalid_smoke_grenade_rules");
        }
        let doc: Document =
            serde_json::from_slice(raw).map_err(|_| "invalid_smoke_grenade_rules")?;
        if doc.schema != "mag_retail_smoke_grenades_v1"
            || doc.application_id != APPLICATION
            || items.application_id() != APPLICATION
            || !doc.source.matches(items, native_elf)
        {
            return Err("smoke_grenade_source_mismatch");
        }
        let mut tokens = BTreeSet::new();
        for (index, variant) in doc.variants.iter().enumerate() {
            let namespace = format!("FactionItemList{}:", index + 1);
            if usize::from(variant.faction) != index
                || !items
                    .retail_authored_ref(variant.item_id)
                    .is_some_and(|item| item.starts_with(&namespace))
                || items
                    .bootstrap_loadout(variant.faction, MARKSMAN)
                    .is_some_and(|kit| {
                        kit.inventories[usize::from(MARKSMAN)].slots[GEAR_SLOT]
                            != Some(variant.item_id)
                    })
            {
                return Err("smoke_grenade_role_mismatch");
            }
            let weapon = items
                .weapon_parameters(variant.item_id)
                .map_err(|_| "smoke_grenade_role_mismatch")?;
            let ammo_name = weapon["ammo_name"]
                .as_str()
                .ok_or("smoke_grenade_ammo_mismatch")?;
            let ammo = &items.raw()["definitions"]["projectile_ammo"][ammo_name]["parameters"];
            if !variant.matches(weapon, ammo_name, ammo) || !tokens.insert(variant.ammo_token) {
                return Err("smoke_grenade_ammo_mismatch");
            }
        }
        Ok(Self {
            variants: doc.variants,
        })
    }

    pub fn variant(&self, faction: u8, item_id: u16) -> Option<&Variant> {
        self.variants
            .get(usize::from(faction))
            .filter(|row| row.item_id == item_id)
    }

    pub fn variants(&self) -> &[Variant] {
        &self.variants
    }
}

impl Source {
    fn matches(&self, items: &ItemCatalog, native_elf: &str) -> bool {
        let Ok(combat) = items.retail_combat() else {
            return false;
        };
        let provenance = combat.provenance();
        native_elf == ELF
            && self.retail_elf_sha256 == ELF
            && self.retail_archive_sha256 == ARCHIVE
            && items.raw()["build"]["source_archive_sha256"].as_str() == Some(ARCHIVE)
            && digest(&self.native_proof_sha256)
            && [
                ("original_retail_elf_sha256", &self.retail_elf_sha256),
                ("source_archive_sha256", &self.retail_archive_sha256),
                ("catalog_sha256", &self.item_catalog_sha256),
                ("binding_report_sha256", &self.item_bindings_sha256),
                ("bootstrap_sha256", &self.bootstrap_sha256),
            ]
            .into_iter()
            .all(|(key, value)| digest(value) && provenance[key].as_str() == Some(value))
    }
}

fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

impl Variant {
    fn matches(self, weapon: &Value, ammo_name: &str, ammo: &Value) -> bool {
        let channels = [
            "character",
            "characterAPierce",
            "characterARedux",
            "vehicle",
            "vehicleAPierce",
            "vehicleARedux",
            "objective",
            "objectiveAPierce",
            "objectiveARedux",
            "mine",
        ];
        weapon["category"].as_str() == Some("kWepCategoryGrenade")
            && weapon["infinite_ammo"].as_bool() == Some(false)
            && weapon["magazine_capacity"].as_u64() == Some(1)
            && self.initial_count > 0
            && weapon["initial_magazine_count"].as_u64() == Some(u64::from(self.initial_count))
            && self.ammo_token != 0
            && ammo_crc32(ammo_name) == Some(self.ammo_token)
            && ammo["id"].as_str() == Some(ammo_name)
            && ammo["propulsion_strategy"].as_str() == Some("kWepPropulsionStrategyThrown")
            && ammo["detonation_strategy"].as_str() == Some("kWepDetonationStrategyTimer")
            && ammo["explosion_strategy"].as_str() == Some("kWepExplosionStrategyVolume")
            && ammo["m_eAmmoPersistance"].as_str() == Some("kWepPersistanceGrenadeSmoke")
            && ammo["destroy_objective"].as_bool() == Some(false)
            && ammo["doHearingLoss"].as_bool() == Some(false)
            && ammo["damage"].as_object().is_some_and(|row| {
                row.len() == channels.len()
                    && channels
                        .iter()
                        .all(|channel| row.get(*channel).and_then(Value::as_f64) == Some(0.0))
            })
            && self.fuse_ms < self.projectile_lifetime_ms
            && [
                ("detonation_timer", self.fuse_ms),
                ("m_fLifetimeMax", self.projectile_lifetime_ms),
                ("m_fDuration", self.volume_duration_ms),
            ]
            .into_iter()
            .all(|(key, ms)| {
                (1..=300_000).contains(&ms)
                    && ammo[key]
                        .as_f64()
                        .is_some_and(|seconds| seconds * 1000.0 == ms as f64)
            })
    }

    pub fn parameters(self) -> GrenadeParameters {
        GrenadeParameters {
            damage: DamageParameters {
                character: 0.0,
                armor_pierce: 0.0,
                armor_redux: 0.0,
            },
            ammo_token: self.ammo_token,
            full_radius: 0.0,
            half_radius: 0.0,
            fuse_ms: self.fuse_ms,
            authored_lifetime_ms: self.projectile_lifetime_ms,
            initial_count: self.initial_count,
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;

    /// The worker-wide native executable identity the rules must name.
    pub(crate) const SYNTHETIC_ELF: &str = ELF;

    /// The retail wire ID of an invented smoke item for `faction`.
    pub(crate) fn synthetic_item_id(faction: u8) -> u16 {
        ((u16::from(faction) + 1) << 10 | 20) + 50
    }

    /// Invented, source-shaped smoke rules with an item catalog and combat
    /// provenance they bind to. Every hash is a placeholder, not a claim.
    pub(crate) fn synthetic_rules_and_items() -> (Vec<u8>, ItemCatalog) {
        let (catalog_sha, bindings_sha, bootstrap_sha) =
            ("a".repeat(64), "b".repeat(64), "c".repeat(64));
        let mut items = serde_json::Map::new();
        let mut weapons = serde_json::Map::new();
        let mut ammo = serde_json::Map::new();
        let mut variants = Vec::new();
        for faction in 0..3_u8 {
            let namespace = format!("FactionItemList{}", faction + 1);
            let reference = format!("{namespace}:20");
            let name = format!("synthetic_smoke_{faction}");
            let round = format!("synthetic_smoke_ammo_{faction}");
            items.insert(
                reference.clone(),
                json!({"authored_ref": reference, "name": name, "item_type": "kWeapon",
                    "faction_list": namespace, "local_ordinal": 20, "wire_item_id": null,
                    "definition": {"group": "weapons", "name": name}}),
            );
            weapons.insert(
                name.clone(),
                json!({"parameters": {"id": name, "ammo_name": round,
                    "category": "kWepCategoryGrenade", "infinite_ammo": false,
                    "magazine_capacity": 1, "initial_magazine_count": 2}}),
            );
            ammo.insert(
                round.clone(),
                json!({"parameters": {"id": round,
                    "propulsion_strategy": "kWepPropulsionStrategyThrown",
                    "detonation_strategy": "kWepDetonationStrategyTimer",
                    "explosion_strategy": "kWepExplosionStrategyVolume",
                    "m_eAmmoPersistance": "kWepPersistanceGrenadeSmoke",
                    "destroy_objective": false, "doHearingLoss": false,
                    "detonation_timer": 2.0, "m_fLifetimeMax": 9.0, "m_fDuration": 6.0,
                    "damage": {"character": 0, "characterAPierce": 0, "characterARedux": 0,
                        "vehicle": 0, "vehicleAPierce": 0, "vehicleARedux": 0,
                        "objective": 0, "objectiveAPierce": 0, "objectiveARedux": 0, "mine": 0}}}),
            );
            variants.push(
                json!({"faction": faction, "item_id": synthetic_item_id(faction),
                "ammo_token": ammo_crc32(&round).unwrap(), "initial_count": 2,
                "fuse_ms": 2000, "projectile_lifetime_ms": 9000, "volume_duration_ms": 6000}),
            );
        }
        let rules = json!({"schema": "mag_retail_smoke_grenades_v1", "application_id": APPLICATION,
            "source": {"retail_elf_sha256": ELF, "retail_archive_sha256": ARCHIVE,
                "item_catalog_sha256": catalog_sha, "item_bindings_sha256": bindings_sha,
                "bootstrap_sha256": bootstrap_sha, "native_proof_sha256": "d".repeat(64)},
            "variants": variants});
        let items = json!({"schema_version": 2, "application_id": APPLICATION,
            "build": {"source_archive_sha256": ARCHIVE}, "items": items,
            "definitions": {"weapons": weapons, "projectile_ammo": ammo}});
        let combat = json!({"schema_version": 2, "application_id": APPLICATION, "status": "synthetic",
            "authority": {"bullet_damage": true, "armor_reduction": true, "body_damage": true,
                "grenade_damage": false, "grenade_physics": false, "healing_schedule": false,
                "ammo_tokens": false},
            "provenance": {"catalog_sha256": catalog_sha, "binding_report_sha256": bindings_sha,
                "source_archive_sha256": ARCHIVE, "original_retail_elf_sha256": ELF,
                "bootstrap_sha256": bootstrap_sha},
            "bullets": {}, "armor": {}, "grenades": {}, "healing": {}, "unsupported": {},
            "body": (0..16).map(|location| json!({"location": location, "armor_area": 0,
                "multiplier": 1.0, "source": {}})).collect::<Vec<_>>(), "limitations": []});
        let armor = crate::blast_rules::CharacterBlastRules {
            armor_rating_reduction: 0.0,
            armor_prevention_reduction: 0.0,
            hit_location: 0,
            damage_type: 10,
        };
        let items = ItemCatalog::from_value_for_application(items, armor, APPLICATION)
            .unwrap()
            .with_retail_combat(
                crate::retail_combat::RetailCombatCatalog::from_json(&combat.to_string()).unwrap(),
            );
        (serde_json::to_vec(&rules).unwrap(), items)
    }

    #[test]
    fn synthetic_rules_bind_only_to_the_retail_executable() {
        let (rules, items) = synthetic_rules_and_items();
        let catalog = Catalog::from_bytes(&rules, &items, ELF).unwrap();
        for faction in 0..3 {
            let variant = catalog
                .variant(faction, synthetic_item_id(faction))
                .unwrap();
            assert_eq!(variant.volume_duration_ms, 6000);
            assert!(
                catalog
                    .variant(faction, synthetic_item_id((faction + 1) % 3))
                    .is_none()
            );
        }
        assert_eq!(
            Catalog::from_bytes(&rules, &items, &"3".repeat(64)).unwrap_err(),
            "smoke_grenade_source_mismatch"
        );
    }

    #[test]
    fn only_exact_nondamaging_timed_volume_matches_source() {
        let ammo_name = "synthetic_smoke";
        let variant = Variant {
            faction: 1,
            item_id: 42,
            ammo_token: ammo_crc32(ammo_name).unwrap(),
            initial_count: 3,
            fuse_ms: 3000,
            projectile_lifetime_ms: 10_000,
            volume_duration_ms: 7000,
        };
        let weapon = json!({
            "category": "kWepCategoryGrenade", "infinite_ammo": false,
            "magazine_capacity": 1, "initial_magazine_count": 3,
        });
        let ammo = json!({
            "id": ammo_name,
            "propulsion_strategy": "kWepPropulsionStrategyThrown",
            "detonation_strategy": "kWepDetonationStrategyTimer",
            "explosion_strategy": "kWepExplosionStrategyVolume",
            "m_eAmmoPersistance": "kWepPersistanceGrenadeSmoke",
            "destroy_objective": false, "doHearingLoss": false,
            "detonation_timer": 3.0, "m_fLifetimeMax": 10.0, "m_fDuration": 7.0,
            "damage": {
                "character": 0, "characterAPierce": 0, "characterARedux": 0,
                "vehicle": 0, "vehicleAPierce": 0, "vehicleARedux": 0,
                "objective": 0, "objectiveAPierce": 0, "objectiveARedux": 0, "mine": 0
            }
        });
        assert!(variant.matches(&weapon, ammo_name, &ammo));
        assert_eq!(variant.parameters().initial_count, 3);
        assert_eq!(variant.parameters().damage.character, 0.0);
        for (field, value) in [
            ("detonation_timer", json!(3.1)),
            ("m_fDuration", json!(7.1)),
            ("destroy_objective", json!(true)),
            ("explosion_strategy", json!("kWepExplosionStrategyDebris")),
        ] {
            let mut changed = ammo.clone();
            changed[field] = value;
            assert!(!variant.matches(&weapon, ammo_name, &changed), "{field}");
        }
        let mut damaging_ammo = ammo;
        damaging_ammo["damage"]["vehicle"] = json!(1);
        assert!(!variant.matches(&weapon, ammo_name, &damaging_ammo));
    }
}
