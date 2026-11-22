//! Source-qualified rules for the retail starter infantry launchers.

use crate::{hospital_rockets::ammo_crc32, item_catalog::ItemCatalog};
use serde::Deserialize;
use serde_json::Value;
use std::{collections::BTreeSet, time::Duration};

const RETAIL_APPLICATION: u32 = 20231;
const RETAIL_ELF_SHA256: &str = "0089f0d6d17e48533ce7ff8d943dddd03487b925e7b04a1a055bfb907568694c";
const RETAIL_GAME_SHA256: &str = "b49c9435be729832029ad7019874d478080dc9764ebde1809b2851d353bc769f";
const MAX_RULE_BYTES: usize = 1 << 20;
const NATIVE_PROJECTILE_AGE_GRACE_SECONDS: f32 = 2.0;
const STOCK_HEAVY_KIT: u8 = 2;

pub const GEAR_SLOT: u8 = 3;
pub const SIGHT_SLOT: usize = 21;

#[derive(Clone, Debug)]
pub struct Catalog {
    variants: [Variant; 3],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Rules {
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

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Variant {
    pub faction: u8,
    pub item_id: u16,
    pub stock_sight_item_id: u16,
    pub ammo_token: u32,
    pub weapon: Weapon,
    pub flight: Flight,
    pub blast: Blast,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Weapon {
    pub magazine_size: u8,
    pub initial_magazine_count: u8,
    pub refire_seconds: f32,
    pub reload_seconds: f32,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Flight {
    pub radius_m: f64,
    /// Full authored capsule spine. Endpoints are symmetric at half this vector.
    pub capsule_spine_local_xyz: [f64; 3],
    pub speed_mps: f32,
    pub armed_time_seconds: f32,
    pub auto_explode_distance_m: f32,
    pub lifetime_seconds: f32,
    pub age_grace_seconds: f32,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Blast {
    pub half_radius_m: f32,
    pub outer_radius_m: f32,
    pub material_penetration: f32,
    pub character_hp: f32,
    pub vehicle_hp: f32,
}

impl Catalog {
    /// Binds the projection to loaded retail equipment and its checked combat
    /// provenance. The source proof digest identifies the offline native check.
    pub fn from_bytes(
        raw: &[u8],
        items: &ItemCatalog,
        native_elf: &str,
    ) -> Result<Self, &'static str> {
        if raw.len() > MAX_RULE_BYTES {
            return Err("invalid_infantry_rocket_rules");
        }
        let rules: Rules =
            serde_json::from_slice(raw).map_err(|_| "invalid_infantry_rocket_rules")?;
        if rules.schema != "mag_retail_infantry_rockets_v1"
            || rules.application_id != RETAIL_APPLICATION
            || items.application_id() != RETAIL_APPLICATION
            || !rules.source.matches(items, native_elf)
        {
            return Err("infantry_rocket_source_mismatch");
        }

        let mut ammo_tokens = BTreeSet::new();
        for (faction, variant) in rules.variants.iter().enumerate() {
            if usize::from(variant.faction) != faction {
                return Err("infantry_rocket_role_mismatch");
            }
            let weapon = items
                .weapon_parameters(variant.item_id)
                .map_err(|_| "infantry_rocket_role_mismatch")?;
            variant.validate_role(items, weapon)?;
            if !variant.weapon.matches(weapon) {
                return Err("infantry_rocket_weapon_mismatch");
            }

            let ammo_name = weapon["ammo_name"]
                .as_str()
                .ok_or("infantry_rocket_ammo_mismatch")?;
            let ammo = &items.raw()["definitions"]["projectile_ammo"][ammo_name]["parameters"];
            if variant.ammo_token == 0
                || ammo_crc32(ammo_name) != Some(variant.ammo_token)
                || !ammo_tokens.insert(variant.ammo_token)
                || ammo["id"].as_str() != Some(ammo_name)
                || !variant.flight.matches(ammo)
                || !variant.blast.matches(ammo)
            {
                return Err("infantry_rocket_ammo_mismatch");
            }
        }
        Ok(Self {
            variants: rules.variants,
        })
    }

    pub fn variant(&self, faction: u8, item_id: u16) -> Option<&Variant> {
        self.variants
            .get(usize::from(faction))
            .filter(|variant| variant.item_id == item_id)
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
        self.retail_elf_sha256 == RETAIL_ELF_SHA256
            && native_elf == RETAIL_ELF_SHA256
            && self.retail_archive_sha256 == RETAIL_GAME_SHA256
            && items.raw()["build"]["source_archive_sha256"].as_str() == Some(RETAIL_GAME_SHA256)
            && sha256_syntax(&self.native_proof_sha256)
            && [
                ("original_retail_elf_sha256", &self.retail_elf_sha256),
                ("source_archive_sha256", &self.retail_archive_sha256),
                ("catalog_sha256", &self.item_catalog_sha256),
                ("binding_report_sha256", &self.item_bindings_sha256),
                ("bootstrap_sha256", &self.bootstrap_sha256),
            ]
            .into_iter()
            .all(|(key, value)| sha256_syntax(value) && provenance[key].as_str() == Some(value))
    }
}

impl Variant {
    fn validate_role(&self, items: &ItemCatalog, weapon: &Value) -> Result<(), &'static str> {
        let invalid_role = "infantry_rocket_role_mismatch";
        let namespace = format!("FactionItemList{}:", self.faction + 1);
        if !items
            .retail_authored_ref(self.item_id)
            .is_some_and(|item| item.starts_with(&namespace))
        {
            return Err(invalid_role);
        }
        let sight_ref = items
            .retail_authored_ref(self.stock_sight_item_id)
            .filter(|sight| sight.starts_with(&namespace))
            .ok_or(invalid_role)?;
        let sight = &items.raw()["items"][sight_ref];
        let sight_name = sight["name"].as_str().ok_or(invalid_role)?;
        let attachments = &weapon["attachment_list"];
        if sight["item_type"].as_str() != Some("kAttachmentItem")
            || sight["definition"]["group"].as_str() != Some("attachments")
            || sight["definition"]["name"].as_str() != Some(sight_name)
            || items.raw()["definitions"]["attachments"][sight_name]["parameters"]["id"].as_str()
                != Some(sight_name)
            || weapon["required_sight_attachment"].as_str() != Some(sight_name)
            || !attachments
                .as_object()
                .is_some_and(|slots| slots.len() == 1)
            || attachments["WeaponAttachment0"]["WeaponAttachment"]["name"].as_str()
                != Some(sight_name)
        {
            return Err(invalid_role);
        }
        if let Some(kit) = items.bootstrap_loadout(self.faction, STOCK_HEAVY_KIT) {
            let inventory = &kit.inventories[usize::from(STOCK_HEAVY_KIT)];
            if inventory.slots[usize::from(GEAR_SLOT)] != Some(self.item_id)
                || inventory.slots[SIGHT_SLOT] != Some(self.stock_sight_item_id)
            {
                return Err(invalid_role);
            }
        }
        Ok(())
    }
}

impl Weapon {
    fn valid(self) -> bool {
        self.magazine_size == 1
            && self.initial_magazine_count > 0
            && bounded(&[
                (self.refire_seconds, 0.001, 60.0),
                (self.reload_seconds, 0.001, 120.0),
            ])
    }

    pub(crate) fn matches(self, weapon: &Value) -> bool {
        let fire = &weapon["fire_mode"];
        let modes = &fire["fire_modes"];
        self.valid()
            && weapon["category"].as_str() == Some("kWepCategoryRocketLauncher")
            && weapon["fire_param_id"].as_str() == Some("kFireParamSecondary")
            && weapon["initial_fire_mode"].as_str() == Some("kSemiAuto")
            && modes.as_object().is_some_and(|modes| modes.len() == 1)
            && modes["FireModeTypeClass0"]["FireModeTypeClass"]["fire_mode_value"].as_str()
                == Some("kSemiAuto")
            && weapon["infinite_ammo"].as_bool() == Some(false)
            && weapon["bSupportLightAmmoPack"].as_bool() == Some(false)
            && weapon["use_turret_mesh"].as_bool() == Some(false)
            && weapon["alternate_firepoint"].as_str() == Some("")
            && weapon["magazine_capacity"].as_u64() == Some(u64::from(self.magazine_size))
            && weapon["initial_magazine_count"].as_u64()
                == Some(u64::from(self.initial_magazine_count))
            && matches_numbers(&[
                (&fire["refire_time"], self.refire_seconds),
                (&fire["reload_time"], self.reload_seconds),
            ])
    }
}

impl Flight {
    fn matches(self, ammo: &Value) -> bool {
        let [x, y, z] = self.capsule_spine_local_xyz;
        let spine_length = x.hypot(y).hypot(z);
        let authored_spine = ammo["capsuleHeight"]
            .as_array()
            .filter(|array| array.len() == 3);
        let straight = ammo["rocketDumbCruiseStraightRange"].as_f64();
        (0.001..=1.0).contains(&self.radius_m)
            && (0.0..=2.0).contains(&spine_length)
            && spine_length > 0.0
            && authored_spine.is_some_and(|array| {
                array
                    .iter()
                    .zip(self.capsule_spine_local_xyz)
                    .all(|(value, projected)| value.as_f64() == Some(projected))
            })
            && ammo["radius"].as_f64() == Some(self.radius_m)
            && self.age_grace_seconds == NATIVE_PROJECTILE_AGE_GRACE_SECONDS
            && self.armed_time_seconds < self.lifetime_seconds
            && bounded(&[
                (self.speed_mps, 1.0, 500.0),
                (self.armed_time_seconds, 0.001, 60.0),
                (self.auto_explode_distance_m, 0.01, 1000.0),
                (self.lifetime_seconds, 0.01, 60.0),
            ])
            && straight.is_some_and(|value| {
                (0.0..f64::from(self.auto_explode_distance_m)).contains(&value)
            })
            && ammo["rocketDumbDirectionChangeRange"]
                .as_array()
                .is_some_and(|range| {
                    range.len() == 2 && range.iter().all(|value| value.as_f64() == Some(0.0))
                })
            && ammo["propulsion_strategy"].as_str() == Some("kWepPropulsionStrategyRocketDumb")
            && ammo["collisionType"].as_str() == Some("kWepProjCollisionShapeCapsule")
            && ammo["detonation_strategy"].as_str() == Some("kWepDetonationStrategyImpact")
            && ammo["explosion_strategy"].as_str() == Some("kWepExplosionStrategyDebris")
            && ammo["detonation_timer"].as_f64() == Some(0.0)
            && matches_numbers(&[
                (&ammo["launch_velocity"], self.speed_mps),
                (&ammo["m_fArmedTime"], self.armed_time_seconds),
                (
                    &ammo["rocketAutoExplodeDistance"],
                    self.auto_explode_distance_m,
                ),
                (&ammo["m_fLifetimeMax"], self.lifetime_seconds),
            ])
    }
}

impl Blast {
    pub(crate) fn matches(self, ammo: &Value) -> bool {
        let damage = &ammo["damage"];
        self.half_radius_m < self.outer_radius_m
            && bounded(&[
                (self.half_radius_m, 0.001, 100.0),
                (self.outer_radius_m, 0.001, 100.0),
                (self.material_penetration, 0.0, 20.0),
                (self.character_hp, 0.001, 10_000.0),
                (self.vehicle_hp, 0.001, 10_000.0),
            ])
            && damage["vehicleAPierce"].as_f64() == Some(0.0)
            && damage["vehicleARedux"].as_f64() == Some(0.0)
            && matches_numbers(&[
                (&ammo["half_damage_radius"], self.half_radius_m),
                (&ammo["full_damage_radius"], self.outer_radius_m),
                (&ammo["MaterialPenetration"], self.material_penetration),
                (&damage["character"], self.character_hp),
                (&damage["vehicle"], self.vehicle_hp),
            ])
    }
}

fn bounded(values: &[(f32, f32, f32)]) -> bool {
    values
        .iter()
        .all(|&(value, min, max)| (min..=max).contains(&value))
}

fn matches_numbers(values: &[(&Value, f32)]) -> bool {
    values
        .iter()
        .all(|&(source, value)| source.as_f64().map(|raw| raw as f32) == Some(value))
}

fn sha256_syntax(digest: &str) -> bool {
    digest.len() == 64
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Finite native-round cap and an authored replacement refire fence. The actor
/// owns the budget and installs a successful candidate only after recording.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RoundBudget {
    capacity: u16,
    remaining: u16,
    refire_seconds: f32,
    last_shot: Option<Duration>,
}

impl RoundBudget {
    pub fn new(weapon: Weapon) -> Result<Self, &'static str> {
        if !weapon.valid() {
            return Err("infantry_rocket_weapon_mismatch");
        }
        let capacity = u16::from(weapon.magazine_size) * u16::from(weapon.initial_magazine_count);
        Ok(Self {
            capacity,
            remaining: capacity,
            refire_seconds: weapon.refire_seconds,
            last_shot: None,
        })
    }

    /// A successful candidate consumes one round. The client handles reload
    /// timing. The server keeps the separately authored refire interval.
    pub fn after_shot(self, now: Duration) -> Result<Self, &'static str> {
        if let Some(last_shot) = self.last_shot {
            let elapsed = now
                .checked_sub(last_shot)
                .ok_or("infantry_rocket_clock_reversed")?;
            // Compare at authored f32 precision, including the exact elapsed
            // millisecond that rounds to the configured refire interval.
            if elapsed.as_secs_f32() < self.refire_seconds {
                return Err("infantry_rocket_refire");
            }
        }
        let remaining = self
            .remaining
            .checked_sub(1)
            .ok_or("infantry_rocket_ammo_empty")?;
        Ok(Self {
            remaining,
            last_shot: Some(now),
            ..self
        })
    }

    /// An independently admitted resupply restores the cap and keeps refire.
    pub fn refilled(self) -> Self {
        Self {
            remaining: self.capacity,
            ..self
        }
    }

    pub fn remaining(self) -> u16 {
        self.remaining
    }
}

#[cfg(test)]
#[path = "infantry_rockets_tests.rs"]
pub(crate) mod tests;
