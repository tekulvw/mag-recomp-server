//! Source-qualified rules for Domination hospital secondary rockets.

use crate::item_catalog::ItemCatalog;
use serde::{Deserialize, de::IgnoredAny};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

const RETAIL_ELF_SHA256: &str = "0089f0d6d17e48533ce7ff8d943dddd03487b925e7b04a1a055bfb907568694c";
const RETAIL_GAME_SHA256: &str = "b49c9435be729832029ad7019874d478080dc9764ebde1809b2851d353bc769f";
const RETAIL_MAIN_SHA256: &str = "a2e8345cd9f5c650ac22544096ba83e8c9e6f0fffd0ef69eb8791fb265f9baad";
const TOKEN_OVERRIDE_SHA256: &str =
    "2ef0ffb844d509ced081e1a181659b5c8b5b73c97ce766a8541e846ddff7e8bf";
const MAX_RULE_BYTES: usize = 1 << 20;
const FACTION_COUNT: usize = 3;

/// Classify a blast origin with a validated vehicle pose and source thresholds.
pub(crate) fn vehicle_blast_location(
    pose: crate::dynamic_collision::Pose,
    blast_origin: [f64; 3],
    rear_threshold_z: f32,
    front_threshold_z: f32,
) -> u8 {
    let displacement: [f32; 3] =
        std::array::from_fn(|axis| blast_origin[axis] as f32 - pose.translation[axis] as f32);
    let local = |axis: usize| {
        (0..3).fold(0., |sum, world| {
            (pose.rotation[world][axis] as f32).mul_add(displacement[world], sum)
        })
    };
    let z = local(2);
    let area = if z > front_threshold_z {
        1
    } else if z < rear_threshold_z {
        3
    } else {
        2
    };
    area + if local(0) > 0. { 0 } else { 3 }
}

#[derive(Debug)]
pub struct Catalog {
    variants: Vec<Variant>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Rules {
    schema: String,
    source: Source,
    variants: Vec<Variant>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    retail_elf_sha256: String,
    retail_archive_sha256: String,
    retail_main_sha256: String,
    item_catalog_sha256: String,
    hospital_turrets_sha256: String,
    launch_proof_sha256: String,
    token_override_sha256: String,
    vehicle_type_token: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HospitalRules {
    schema: String,
    source: HospitalSource,
    variants: Vec<HospitalRole>,
    #[serde(rename = "profiles")]
    _profiles: IgnoredAny,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HospitalSource {
    structure_sha256: String,
    item_catalog_sha256: String,
    retail_elf_sha256: String,
    retail_archive_sha256: String,
    vehicle_type_token: u32,
    seat_pose_sha256: String,
    wire_proof_sha256: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HospitalRole {
    faction: u8,
    primary_item_id: u16,
    secondary_item_id: u16,
    properties_source: String,
    seat_node: String,
    seat_index: u8,
    turret_index: u8,
    seat_pose_scope: String,
    occupant_visible: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Variant {
    pub faction: u8,
    pub secondary_item_id: u16,
    pub ammo_name: String,
    pub ammo_token: u32,
    pub muzzle: Muzzle,
    pub weapon: Weapon,
    pub flight: Flight,
    pub blast: Blast,
}

/// Authored local reference transforms, in body-to-firepoint parent order.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Muzzle {
    pub root: MuzzleNode,
    pub yaw: MuzzleNode,
    pub pitch: MuzzleNode,
    pub firepoint: MuzzleNode,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MuzzleNode {
    pub position_xyz: [f64; 3],
    pub rotation_xyzw: [f64; 4],
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Weapon {
    pub refire_seconds: f32,
    pub reload_seconds: f32,
    pub infinite_ammo: bool,
    pub magazine_size: u8,
    /// Authored count of magazines. This is independent of loaded rounds.
    pub initial_magazine_count: u8,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Flight {
    pub propulsion_strategy: Propulsion,
    pub detonation_strategy: Detonation,
    pub explosion_strategy: Explosion,
    pub collision_shape: CollisionShape,
    pub radius_m: f32,
    pub speed_mps: f32,
    pub lifetime_seconds: f32,
    pub age_grace_seconds: f32,
    pub armed_time_seconds: f32,
    pub detonation_timer_seconds: f32,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
pub enum Propulsion {
    #[serde(rename = "kWepPropulsionStrategyRocketStraight")]
    Straight,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
pub enum Detonation {
    #[serde(rename = "kWepDetonationStrategyImpact")]
    Impact,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
pub enum Explosion {
    #[serde(rename = "kWepExplosionStrategyDebris")]
    Debris,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
pub enum CollisionShape {
    #[serde(rename = "kWepProjCollisionShapeSphere")]
    Sphere,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Blast {
    pub half_radius_m: f32,
    pub outer_radius_m: f32,
    pub material_penetration: f32,
    pub character_hp: f32,
    pub vehicle_hp: f32,
    pub objective_hp: f32,
}

impl Catalog {
    /// Binds a projection to the exact hospital rules and the loaded retail item
    /// catalog. The caller supplies the hospital rules used to qualify placements
    /// and an item catalog already paired with its checked combat projection.
    ///
    /// The launch proof digest identifies the offline extraction evidence. Its
    /// syntax is checked here. The offline projector verifies the original model
    /// transforms and native launch proof against the owned assets.
    pub fn from_bytes(
        rules: &[u8],
        items: &ItemCatalog,
        hospital_rules: &[u8],
        native_elf: &str,
    ) -> Result<Self, &'static str> {
        if rules.len() > MAX_RULE_BYTES || hospital_rules.len() > MAX_RULE_BYTES {
            return Err("invalid_hospital_rocket_rules");
        }
        let rules: Rules =
            serde_json::from_slice(rules).map_err(|_| "invalid_hospital_rocket_rules")?;
        let hospital: HospitalRules = serde_json::from_slice(hospital_rules)
            .map_err(|_| "invalid_hospital_rocket_hospital_rules")?;
        let provenance = items.retail_combat()?.provenance();
        if rules.schema != "mag_domination_hospital_rockets_v1"
            || hospital.schema != "mag_domination_hospital_turrets_v1"
            || items.application_id() != 20231
            || !rules.source.matches(
                &hospital.source,
                hospital_rules,
                native_elf,
                items,
                provenance,
            )
        {
            return Err("hospital_rocket_source_mismatch");
        }
        if rules.variants.len() != FACTION_COUNT || hospital.variants.len() != FACTION_COUNT {
            return Err("invalid_hospital_rocket_role");
        }

        let mut ammo_tokens = BTreeSet::new();
        let common_age_grace = rules.variants[0].flight.age_grace_seconds;
        for (faction, (variant, role)) in rules.variants.iter().zip(&hospital.variants).enumerate()
        {
            let namespace = format!("FactionItemList{}:", faction + 1);
            if role.faction as usize != faction
                || variant.faction != role.faction
                || variant.secondary_item_id != role.secondary_item_id
                || !role.valid()
                || !items
                    .retail_authored_ref(variant.secondary_item_id)
                    .is_some_and(|reference| reference.starts_with(&namespace))
            {
                return Err("invalid_hospital_rocket_role");
            }
            if !variant.muzzle.valid() {
                return Err("invalid_hospital_rocket_muzzle");
            }
            if !variant.flight.valid()
                || variant.flight.age_grace_seconds != common_age_grace
                || !variant.blast.valid()
            {
                return Err("invalid_hospital_rocket_ballistics");
            }
            let weapon = items
                .weapon_parameters(variant.secondary_item_id)
                .map_err(|_| "hospital_rocket_weapon_mismatch")?;
            if !variant.weapon.matches(weapon, &variant.ammo_name) {
                return Err("hospital_rocket_weapon_mismatch");
            }
            let ammo =
                &items.raw()["definitions"]["projectile_ammo"][&variant.ammo_name]["parameters"];
            if variant.ammo_token == 0
                || ammo_crc32(&variant.ammo_name) != Some(variant.ammo_token)
                || !ammo_tokens.insert(variant.ammo_token)
                || ammo["id"].as_str() != Some(&variant.ammo_name)
                || !variant.flight.matches(ammo)
                || !variant.blast.matches(ammo)
            {
                return Err("hospital_rocket_ammo_mismatch");
            }
        }
        Ok(Self {
            variants: rules.variants,
        })
    }

    /// Enumerates the three qualified source variants in faction order.
    pub fn variants(&self) -> &[Variant] {
        &self.variants
    }

    /// Returns the bound rocket only for the faction and its exact secondary item.
    pub fn secondary_for(&self, faction: u8, item_id: u16) -> Option<&Variant> {
        self.variants
            .get(usize::from(faction))
            .filter(|variant| variant.secondary_item_id == item_id)
    }
}

impl Source {
    fn matches(
        &self,
        hospital: &HospitalSource,
        hospital_rules: &[u8],
        native_elf: &str,
        items: &ItemCatalog,
        provenance: &Value,
    ) -> bool {
        [
            &self.item_catalog_sha256,
            &self.launch_proof_sha256,
            &hospital.structure_sha256,
            &hospital.seat_pose_sha256,
            &hospital.wire_proof_sha256,
        ]
        .into_iter()
        .all(|digest| valid_sha256(digest))
            && self.retail_elf_sha256 == RETAIL_ELF_SHA256
            && self.retail_elf_sha256 == native_elf
            && self.retail_elf_sha256 == hospital.retail_elf_sha256
            && self.retail_archive_sha256 == RETAIL_GAME_SHA256
            && self.retail_archive_sha256 == hospital.retail_archive_sha256
            && items.raw()["build"]["source_archive_sha256"].as_str()
                == Some(self.retail_archive_sha256.as_str())
            && provenance["source_archive_sha256"].as_str()
                == Some(self.retail_archive_sha256.as_str())
            && self.retail_main_sha256 == RETAIL_MAIN_SHA256
            && self.token_override_sha256 == TOKEN_OVERRIDE_SHA256
            && self.item_catalog_sha256 == hospital.item_catalog_sha256
            && provenance["catalog_sha256"].as_str() == Some(self.item_catalog_sha256.as_str())
            && self.hospital_turrets_sha256 == format!("{:x}", Sha256::digest(hospital_rules))
            && self.vehicle_type_token != 0
            && self.vehicle_type_token == hospital.vehicle_type_token
    }
}

impl HospitalRole {
    fn valid(&self) -> bool {
        (1..4096).contains(&self.primary_item_id)
            && self.primary_item_id != self.secondary_item_id
            && self.properties_source.starts_with("vehicle/")
            && self.properties_source.ends_with(".zpr")
            && self.seat_node == "gunner_seat"
            && self.seat_index == 0
            && self.turret_index == 0
            && self.seat_pose_scope == "retail_root0_identity_fallback"
            && !self.occupant_visible
    }
}

impl Muzzle {
    fn valid(self) -> bool {
        const IDENTITY: [f64; 4] = [0.0, 0.0, 0.0, 1.0];
        [self.root, self.yaw, self.pitch, self.firepoint]
            .into_iter()
            .all(MuzzleNode::valid)
            && self.root.position_xyz == [0.0; 3]
            && self.root.rotation_xyzw == IDENTITY
            && self.firepoint.position_xyz != [0.0; 3]
            && self.firepoint.rotation_xyzw == IDENTITY
    }
}

impl MuzzleNode {
    fn valid(self) -> bool {
        self.position_xyz
            .into_iter()
            .all(|value| value.is_finite() && value.abs() <= 10_000.0)
            && self.rotation_xyzw.into_iter().all(f64::is_finite)
            && (self.rotation_xyzw.into_iter().map(|q| q * q).sum::<f64>() - 1.0).abs() < 1e-5
    }
}

impl Weapon {
    fn matches(self, weapon: &Value, name: &str) -> bool {
        let fire = &weapon["fire_mode"];
        let modes = &fire["fire_modes"];
        weapon["id"].as_str() == Some(name)
            && weapon["ammo_name"].as_str() == Some(name)
            && weapon["category"].as_str() == Some("kWepCategoryTurretRocket")
            && weapon["fire_param_id"].as_str() == Some("kFireParamSecondary")
            && weapon["initial_fire_mode"].as_str() == Some("kSemiAuto")
            && modes.as_object().is_some_and(|modes| modes.len() == 1)
            && modes["FireModeTypeClass0"]["FireModeTypeClass"]["fire_mode_value"].as_str()
                == Some("kSemiAuto")
            && weapon["use_turret_mesh"].as_bool() == Some(true)
            && weapon["model_name"].as_str() == Some("")
            && weapon["alternate_firepoint"].as_str() == Some("firepoint_02")
            && weapon["infinite_ammo"].as_bool() == Some(self.infinite_ammo)
            && self.magazine_size > 0
            && self.initial_magazine_count > 0
            && weapon["magazine_capacity"].as_u64() == Some(u64::from(self.magazine_size))
            && weapon["initial_magazine_count"].as_u64()
                == Some(u64::from(self.initial_magazine_count))
            && bounded(&[
                (self.refire_seconds, 0.001, 60.0),
                (self.reload_seconds, 0.001, 120.0),
            ])
            && matches_numbers(&[
                (&fire["refire_time"], self.refire_seconds),
                (&fire["reload_time"], self.reload_seconds),
            ])
    }
}

impl Flight {
    fn valid(self) -> bool {
        bounded(&[
            (self.radius_m, 0.001, 1.0),
            (self.speed_mps, 1.0, 500.0),
            (self.lifetime_seconds, 0.01, 60.0),
            (self.age_grace_seconds, 0.0, 10.0),
        ]) && self.armed_time_seconds == -1.0
            && self.detonation_timer_seconds == 0.0
    }

    fn matches(self, ammo: &Value) -> bool {
        ammo["propulsion_strategy"].as_str() == Some("kWepPropulsionStrategyRocketStraight")
            && ammo["detonation_strategy"].as_str() == Some("kWepDetonationStrategyImpact")
            && ammo["explosion_strategy"].as_str() == Some("kWepExplosionStrategyDebris")
            && ammo["collisionType"].as_str() == Some("kWepProjCollisionShapeSphere")
            && matches_numbers(&[
                (&ammo["radius"], self.radius_m),
                (&ammo["launch_velocity"], self.speed_mps),
                (&ammo["m_fLifetimeMax"], self.lifetime_seconds),
                (&ammo["m_fArmedTime"], self.armed_time_seconds),
                (&ammo["detonation_timer"], self.detonation_timer_seconds),
            ])
    }
}

impl Blast {
    fn valid(self) -> bool {
        self.half_radius_m < self.outer_radius_m
            && bounded(&[
                (self.half_radius_m, 0.001, 100.0),
                (self.outer_radius_m, 0.001, 100.0),
                (self.material_penetration, 0.0, 20.0),
                (self.character_hp, 0.001, 10_000.0),
                (self.vehicle_hp, 0.001, 10_000.0),
                (self.objective_hp, 0.001, 10_000.0),
            ])
    }

    fn matches(self, ammo: &Value) -> bool {
        let damage = &ammo["damage"];
        matches_numbers(&[
            (&ammo["half_damage_radius"], self.half_radius_m),
            (&ammo["full_damage_radius"], self.outer_radius_m),
            (&ammo["MaterialPenetration"], self.material_penetration),
            (&damage["character"], self.character_hp),
            (&damage["vehicle"], self.vehicle_hp),
            (&damage["objective"], self.objective_hp),
        ]) && [
            "vehicleAPierce",
            "vehicleARedux",
            "objectiveAPierce",
            "objectiveARedux",
        ]
        .iter()
        .all(|field| damage[*field].as_f64() == Some(0.0))
    }
}

fn valid_sha256(digest: &str) -> bool {
    digest.len() == 64
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn bounded(numbers: &[(f32, f32, f32)]) -> bool {
    numbers
        .iter()
        .all(|&(value, min, max)| (min..=max).contains(&value))
}

fn matches_numbers(numbers: &[(&Value, f32)]) -> bool {
    numbers.iter().all(|&(original, projected)| {
        original.as_f64().map(|value| value as f32) == Some(projected)
    })
}

/// Reflected CRC32 of the canonical resource name. The extraction verifies
/// that the pinned retail token-override table does not alias these names.
pub(crate) fn ammo_crc32(name: &str) -> Option<u32> {
    if name.is_empty() || !name.bytes().all(|byte| byte.is_ascii_graphic()) {
        return None;
    }
    let mut crc = u32::MAX;
    for byte in name.bytes() {
        let canonical = if byte == b'\\' {
            b'/'
        } else {
            byte.to_ascii_lowercase()
        };
        crc ^= u32::from(canonical);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 * (crc & 1));
        }
    }
    Some(!crc)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{blast_rules::CharacterBlastRules, retail_combat::RetailCombatCatalog};
    use serde_json::{Value, json};
    use sha2::{Digest, Sha256};

    const ELF: &str = "0089f0d6d17e48533ce7ff8d943dddd03487b925e7b04a1a055bfb907568694c";
    const GAME: &str = "b49c9435be729832029ad7019874d478080dc9764ebde1809b2851d353bc769f";
    const MAIN: &str = "a2e8345cd9f5c650ac22544096ba83e8c9e6f0fffd0ef69eb8791fb265f9baad";
    const OVERRIDES: &str = "2ef0ffb844d509ced081e1a181659b5c8b5b73c97ce766a8541e846ddff7e8bf";

    struct Fixture {
        rules: Value,
        items: Value,
        hospital: Value,
        catalog_sha: String,
    }

    impl Fixture {
        fn new() -> Self {
            use serde_json::Map;
            let catalog_sha = "a".repeat(64);
            let mut authored_items = Map::new();
            let mut weapons = Map::new();
            let mut ammo = Map::new();
            let mut hospital_variants = Vec::new();
            let mut rocket_variants = Vec::new();
            for faction in 0..3_u16 {
                let list = faction + 1;
                let secondary = (list << 10) + 50 + 23;
                let primary = secondary - 1;
                let name = format!("fixture_rocket_{faction}");
                let namespace = format!("FactionItemList{list}");
                let reference = format!("{namespace}:23");
                authored_items.insert(
                    reference.clone(),
                    json!({"name":name,"authored_ref":reference,"faction_list":namespace,
                        "local_ordinal":23,"wire_item_id":null,"item_type":"kWeapon",
                        "definition":{"group":"weapons","name":name}}),
                );
                weapons.insert(
                    name.clone(),
                    json!({"parameters":{"id":name,"ammo_name":name,
                        "category":"kWepCategoryTurretRocket","fire_param_id":"kFireParamSecondary",
                        "initial_fire_mode":"kSemiAuto","use_turret_mesh":true,"model_name":"",
                        "alternate_firepoint":"firepoint_02","magazine_capacity":4,
                        "initial_magazine_count":3,"infinite_ammo":true,
                        "fire_mode":{"refire_time":1.25,"reload_time":6.5,
                            "fire_modes":{"FireModeTypeClass0":{"FireModeTypeClass":{
                                "fire_mode_value":"kSemiAuto"}}}}}}),
                );
                ammo.insert(
                    name.clone(),
                    json!({"parameters":{"id":name,
                        "propulsion_strategy":"kWepPropulsionStrategyRocketStraight",
                        "detonation_strategy":"kWepDetonationStrategyImpact",
                        "explosion_strategy":"kWepExplosionStrategyDebris",
                        "collisionType":"kWepProjCollisionShapeSphere",
                        "radius":0.125,"launch_velocity":60.0,"m_fLifetimeMax":8.0,
                        "m_fArmedTime":-1.0,"detonation_timer":0.0,
                        "half_damage_radius":2.5,"full_damage_radius":6.0,"MaterialPenetration":1.5,
                        "damage":{"character":120.0,"vehicle":800.0,"objective":500.0,
                            "vehicleAPierce":0.0,"vehicleARedux":0.0,
                            "objectiveAPierce":0.0,"objectiveARedux":0.0}}}),
                );
                hospital_variants.push(json!({"faction":faction,
                    "primary_item_id":primary,"secondary_item_id":secondary,
                    "properties_source":format!("vehicle/fixture_{faction}.zpr"),
                    "seat_node":"gunner_seat","seat_index":0,"turret_index":0,
                    "seat_pose_scope":"retail_root0_identity_fallback","occupant_visible":false}));
                let identity = json!({"position_xyz":[0,0,0],"rotation_xyzw":[0,0,0,1]});
                rocket_variants.push(json!({"faction":faction,"secondary_item_id":secondary,
                    "ammo_name":name,"ammo_token":([0x262d3d5d_u32,0x512a0dcb,0xc8235c71][faction as usize]),
                    "muzzle":{"root":identity,"yaw":identity,
                        "pitch":{"position_xyz":[0.25,2.0,0.0],"rotation_xyzw":[0,0,0,1]},
                        "firepoint":{"position_xyz":[1.0,0.5,0.125],"rotation_xyzw":[0,0,0,1]}},
                    "weapon":{"refire_seconds":1.25,"reload_seconds":6.5,"infinite_ammo":true,
                        "magazine_size":4,"initial_magazine_count":3},
                    "flight":{"propulsion_strategy":"kWepPropulsionStrategyRocketStraight",
                        "detonation_strategy":"kWepDetonationStrategyImpact",
                        "explosion_strategy":"kWepExplosionStrategyDebris",
                        "collision_shape":"kWepProjCollisionShapeSphere","radius_m":0.125,
                        "speed_mps":60.0,"lifetime_seconds":8.0,"age_grace_seconds":1.5,
                        "armed_time_seconds":-1.0,"detonation_timer_seconds":0.0},
                    "blast":{"half_radius_m":2.5,"outer_radius_m":6.0,"material_penetration":1.5,
                        "character_hp":120.0,"vehicle_hp":800.0,"objective_hp":500.0}}));
            }
            let hospital = json!({"schema":"mag_domination_hospital_turrets_v1",
                "source":{"retail_elf_sha256":ELF,"retail_archive_sha256":GAME,
                    "item_catalog_sha256":catalog_sha,"vehicle_type_token":1234,
                    "structure_sha256":"c".repeat(64),"seat_pose_sha256":"d".repeat(64),
                    "wire_proof_sha256":"e".repeat(64)},
                "variants":hospital_variants,"profiles":[]});
            let rules = json!({"schema":"mag_domination_hospital_rockets_v1",
                "source":{"retail_elf_sha256":ELF,"retail_archive_sha256":GAME,
                    "retail_main_sha256":MAIN,"item_catalog_sha256":catalog_sha,
                    "hospital_turrets_sha256":format!("{:x}", Sha256::digest(encode(&hospital))),
                    "launch_proof_sha256":"f".repeat(64),"token_override_sha256":OVERRIDES,
                    "vehicle_type_token":1234},"variants":rocket_variants});
            let items = json!({"schema_version":2,"application_id":20231,
                "build":{"source_archive_sha256":GAME},"items":authored_items,
                "definitions":{"weapons":weapons,"projectile_ammo":ammo}});
            Self {
                rules,
                items,
                hospital,
                catalog_sha,
            }
        }

        fn items(&self) -> ItemCatalog {
            let blast = CharacterBlastRules {
                armor_rating_reduction: 0.0,
                armor_prevention_reduction: 0.0,
                hit_location: 0,
                damage_type: 10,
            };
            let combat = json!({"schema_version":2,"application_id":20231,"status":"synthetic",
                "authority":{"bullet_damage":true,"armor_reduction":true,"body_damage":true,
                    "grenade_damage":false,"grenade_physics":false,"healing_schedule":false,"ammo_tokens":false},
                "provenance":{"catalog_sha256":self.catalog_sha,"source_archive_sha256":GAME},
                "bullets":{},"armor":{},"grenades":{},"healing":{},"unsupported":{},
                "body":(0..16).map(|location| json!({"location":location,"armor_area":0,
                    "multiplier":1.0,"source":{}})).collect::<Vec<_>>(),"limitations":[]});
            ItemCatalog::from_value_for_application(self.items.clone(), blast, 20231)
                .unwrap()
                .with_retail_combat(RetailCombatCatalog::from_json(&combat.to_string()).unwrap())
        }

        fn load(&self) -> Result<Catalog, &'static str> {
            Catalog::from_bytes(
                &encode(&self.rules),
                &self.items(),
                &encode(&self.hospital),
                ELF,
            )
        }

        fn bind_changed_hospital(&mut self) {
            self.rules["source"]["hospital_turrets_sha256"] =
                json!(format!("{:x}", Sha256::digest(encode(&self.hospital))));
        }
    }

    pub(crate) fn synthetic_catalog() -> Catalog {
        Fixture::new().load().unwrap()
    }

    fn encode(value: &Value) -> Vec<u8> {
        serde_json::to_vec(value).unwrap()
    }

    #[test]
    fn catalog_loads_three_exact_source_qualified_secondaries() {
        let fixture = Fixture::new();
        let catalog = fixture.load().unwrap();
        assert_eq!(catalog.variants().len(), 3);
        for faction in 0..3 {
            let secondary = fixture.rules["variants"][usize::from(faction)]["secondary_item_id"]
                .as_u64()
                .unwrap() as u16;
            let variant = catalog.secondary_for(faction, secondary).unwrap();
            assert_eq!(variant.flight.explosion_strategy, Explosion::Debris);
            assert_eq!(variant.weapon.initial_magazine_count, 3);
            assert_eq!(variant.blast.vehicle_hp, 800.0);
            assert_eq!(variant.muzzle.firepoint.position_xyz, [1.0, 0.5, 0.125]);
            assert!(catalog.secondary_for(faction, secondary - 1).is_none());
            assert!(
                catalog
                    .secondary_for((faction + 1) % 3, secondary)
                    .is_none()
            );
        }
        assert!(catalog.secondary_for(3, 0).is_none());
    }

    #[test]
    fn catalog_rejects_source_drift_and_wrong_physical_role() {
        let mut fixture = Fixture::new();
        fixture.hospital["variants"][0]["properties_source"] = json!("vehicle/another.zpr");
        assert_eq!(
            fixture.load().unwrap_err(),
            "hospital_rocket_source_mismatch"
        );
        fixture.bind_changed_hospital();
        fixture.hospital["variants"][0]["seat_index"] = json!(1);
        fixture.bind_changed_hospital();
        assert_eq!(fixture.load().unwrap_err(), "invalid_hospital_rocket_role");

        let mut fixture = Fixture::new();
        fixture.rules["source"]["retail_main_sha256"] = json!("0".repeat(64));
        assert_eq!(
            fixture.load().unwrap_err(),
            "hospital_rocket_source_mismatch"
        );
    }

    #[test]
    fn catalog_rejects_foreign_slot_duplicate_and_noncanonical_roster() {
        let mut fixture = Fixture::new();
        fixture.rules["variants"][0]["secondary_item_id"] =
            fixture.rules["variants"][1]["secondary_item_id"].clone();
        assert_eq!(fixture.load().unwrap_err(), "invalid_hospital_rocket_role");

        let mut fixture = Fixture::new();
        fixture.rules["variants"].as_array_mut().unwrap().swap(0, 1);
        assert_eq!(fixture.load().unwrap_err(), "invalid_hospital_rocket_role");
    }

    #[test]
    fn catalog_rejects_unbound_weapon_ammo_and_damage() {
        let mut fixture = Fixture::new();
        fixture.items["definitions"]["weapons"]["fixture_rocket_0"]["parameters"]["category"] =
            json!("kWepCategoryTurret");
        assert_eq!(
            fixture.load().unwrap_err(),
            "hospital_rocket_weapon_mismatch"
        );

        let mut fixture = Fixture::new();
        fixture.rules["variants"][0]["ammo_token"] = json!(910);
        assert_eq!(fixture.load().unwrap_err(), "hospital_rocket_ammo_mismatch");

        let mut fixture = Fixture::new();
        fixture.rules["variants"][0]["blast"]["vehicle_hp"] = json!(801.0);
        assert_eq!(fixture.load().unwrap_err(), "hospital_rocket_ammo_mismatch");

        let mut fixture = Fixture::new();
        fixture.rules["variants"][0]["weapon"]["initial_magazine_count"] = json!(4);
        assert_eq!(
            fixture.load().unwrap_err(),
            "hospital_rocket_weapon_mismatch"
        );
    }

    #[test]
    fn catalog_rejects_unqualified_pose_strategies_and_bounds() {
        for (pointer, invalid) in [
            ("/muzzle/pitch/rotation_xyzw", json!([0, 0, 0, 2])),
            ("/muzzle/root/position_xyz", json!([0, 0.5, 0])),
            ("/muzzle/yaw/position_xyz", json!([1e99, 0, 0])),
            ("/muzzle/firepoint/position_xyz", json!([0, 0, 0])),
            (
                "/flight/detonation_strategy",
                json!("kWepDetonationStrategyTimed"),
            ),
            ("/flight/speed_mps", json!(0)),
            ("/flight/armed_time_seconds", json!(0)),
            ("/flight/detonation_timer_seconds", json!(1)),
            ("/blast/outer_radius_m", json!(2)),
        ] {
            let mut fixture = Fixture::new();
            *fixture.rules["variants"][0].pointer_mut(pointer).unwrap() = invalid;
            assert!(fixture.load().is_err(), "{pointer}");
        }
        let fixture = Fixture::new();
        let nonfinite = fixture
            .rules
            .to_string()
            .replace("\"speed_mps\":60.0", "\"speed_mps\":1e999");
        assert_eq!(
            Catalog::from_bytes(
                nonfinite.as_bytes(),
                &fixture.items(),
                &encode(&fixture.hospital),
                ELF
            )
            .unwrap_err(),
            "invalid_hospital_rocket_rules"
        );
    }
}
