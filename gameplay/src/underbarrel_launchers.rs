//! Strict binding for original retail stock AR underbarrel launchers.

use crate::item_catalog::{BulletAttachmentKind, ItemCatalog};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const ATTACHMENT_SLOT: usize = 17;
const APPLICATION: u32 = 20231;
const ELF: &str = "0089f0d6d17e48533ce7ff8d943dddd03487b925e7b04a1a055bfb907568694c";
const ARCHIVE: &str = "b49c9435be729832029ad7019874d478080dc9764ebde1809b2851d353bc769f";

#[derive(Clone, Debug)]
pub struct Catalog {
    variants: [Variant; 3],
    ammo: Ammo,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    schema: String,
    application_id: u32,
    attachment_slot: usize,
    source: Source,
    variants: [Variant; 3],
    ammo: Ammo,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    retail_elf_sha256: String,
    retail_archive_sha256: String,
    input_sha256: InputHashes,
    native_proof_sha256: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct InputHashes {
    items: String,
    item_bindings: String,
    all_factions_bootstrap_loadouts: String,
    skill_purchases: String,
    armory: String,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Variant {
    pub faction: u8,
    pub host_item_id: u16,
    pub attachment_item_id: u16,
    pub magazine_capacity: u8,
    pub initial_magazine_count: u8,
    pub refire_seconds: f32,
    pub reload_seconds: f32,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ammo {
    pub name: String,
    pub token: u32,
    pub authored: Authored,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
enum Propulsion {
    #[serde(rename = "kWepPropulsionStrategyThrown")]
    Thrown,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
enum Detonation {
    #[serde(rename = "kWepDetonationStrategyImpact")]
    Impact,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
enum Explosion {
    #[serde(rename = "kWepExplosionStrategyDebris")]
    Debris,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
enum CollisionShape {
    #[serde(rename = "kWepProjCollisionShapeCapsule")]
    Capsule,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Authored {
    propulsion_strategy: Propulsion,
    detonation_strategy: Detonation,
    explosion_strategy: Explosion,
    #[serde(rename = "collisionType")]
    collision_type: CollisionShape,
    pub launch_velocity: f32,
    #[serde(rename = "m_fArmedTime")]
    pub armed_time: f32,
    #[serde(rename = "m_fLifetimeMax")]
    pub lifetime: f32,
    pub detonation_timer: f32,
    pub radius: f64,
    pub mass: f32,
    pub friction: f32,
    pub restitution: f32,
    #[serde(rename = "linearDamping")]
    pub linear_damping: f32,
    #[serde(rename = "linearDampingOnGround")]
    pub linear_damping_on_ground: f32,
    #[serde(rename = "angularDamping")]
    pub angular_damping: f32,
    #[serde(rename = "angularDampingOnGround")]
    pub angular_damping_on_ground: f32,
    #[serde(rename = "fBounceDampingDefault")]
    pub bounce_damping_default: f32,
    #[serde(rename = "fBounceDampingCharacter")]
    pub bounce_damping_character: f32,
    #[serde(rename = "m_fBounceToExplode")]
    pub bounce_to_explode: f32,
    #[serde(rename = "m_fImpactDamage")]
    pub impact_damage: f32,
    pub full_damage_radius: f32,
    pub half_damage_radius: f32,
    #[serde(rename = "MaterialPenetration")]
    pub material_penetration: f32,
    #[serde(rename = "capsuleHeight")]
    pub capsule_height: [f64; 3],
    pub damage: Damage,
    pub destroy_objective: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Damage {
    pub character: f32,
    #[serde(rename = "characterAPierce")]
    pub character_apierce: f32,
    #[serde(rename = "characterARedux")]
    pub character_aredux: f32,
    pub vehicle: f32,
    #[serde(rename = "vehicleAPierce")]
    pub vehicle_apierce: f32,
    #[serde(rename = "vehicleARedux")]
    pub vehicle_aredux: f32,
    pub objective: f32,
    #[serde(rename = "objectiveAPierce")]
    pub objective_apierce: f32,
    #[serde(rename = "objectiveARedux")]
    pub objective_aredux: f32,
    pub mine: f32,
}

impl Catalog {
    pub fn from_bytes(
        raw: &[u8],
        items: &ItemCatalog,
        native_elf: &str,
    ) -> Result<Self, &'static str> {
        if raw.len() > 1 << 20 {
            return Err("invalid_underbarrel_launcher_rules");
        }
        let doc: Document =
            serde_json::from_slice(raw).map_err(|_| "invalid_underbarrel_launcher_rules")?;
        if doc.schema != "mag_retail_underbarrel_launchers_v1"
            || doc.application_id != APPLICATION
            || doc.attachment_slot != ATTACHMENT_SLOT
            || items.application_id() != APPLICATION
            || !doc.source.matches(items, native_elf)
        {
            return Err("underbarrel_launcher_source_mismatch");
        }
        if !doc.ammo.matches(items) {
            return Err("underbarrel_launcher_ammo_mismatch");
        }
        for (index, variant) in doc.variants.iter().enumerate() {
            if usize::from(variant.faction) != index || !variant.matches(items, &doc.ammo.name) {
                return Err("underbarrel_launcher_role_mismatch");
            }
        }
        Ok(Self {
            variants: doc.variants,
            ammo: doc.ammo,
        })
    }

    pub fn variant(&self, faction: u8, host: u16, attachment: u16) -> Option<&Variant> {
        self.variants
            .get(usize::from(faction))
            .filter(|row| row.host_item_id == host && row.attachment_item_id == attachment)
    }

    pub fn variants(&self) -> &[Variant] {
        &self.variants
    }

    pub fn ammo(&self) -> &Ammo {
        &self.ammo
    }
}

impl Source {
    fn matches(&self, items: &ItemCatalog, native_elf: &str) -> bool {
        let Ok(combat) = items.retail_combat() else {
            return false;
        };
        let values = &self.input_sha256;
        let provenance = combat.provenance();
        native_elf == ELF
            && self.retail_elf_sha256 == ELF
            && self.retail_archive_sha256 == ARCHIVE
            && items.raw()["build"]["source_archive_sha256"].as_str() == Some(ARCHIVE)
            && [
                &self.native_proof_sha256,
                &values.skill_purchases,
                &values.armory,
            ]
            .into_iter()
            .all(|value| digest(value))
            && [
                ("original_retail_elf_sha256", &self.retail_elf_sha256),
                ("source_archive_sha256", &self.retail_archive_sha256),
                ("catalog_sha256", &values.items),
                ("binding_report_sha256", &values.item_bindings),
                ("bootstrap_sha256", &values.all_factions_bootstrap_loadouts),
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
    pub fn initial_count(self) -> u8 {
        self.magazine_capacity
            .saturating_mul(self.initial_magazine_count)
    }

    fn matches(self, items: &ItemCatalog, ammo_name: &str) -> bool {
        let (Some(host), Some(attachment)) = (
            items.retail_authored_ref(self.host_item_id),
            items.retail_authored_ref(self.attachment_item_id),
        ) else {
            return false;
        };
        let namespace = format!("FactionItemList{}:", self.faction + 1);
        if !host.starts_with(&namespace)
            || !attachment.starts_with(&namespace)
            || items.retail_firearm_attachment(0, self.host_item_id, self.attachment_item_id)
                != Some(BulletAttachmentKind::Launcher)
            || items.bootstrap_loadout(self.faction, 0).is_some_and(|kit| {
                kit.inventories[0].slots[0] != Some(self.host_item_id)
                    || kit.inventories[0].slots[ATTACHMENT_SLOT].is_some()
            })
        {
            return false;
        }
        let raw = items.raw();
        let Some(name) = raw["items"][attachment]["name"].as_str() else {
            return false;
        };
        self.matches_weapon(
            &raw["definitions"]["weapons"][name]["parameters"],
            ammo_name,
        )
    }

    fn matches_weapon(self, weapon: &Value, ammo_name: &str) -> bool {
        self.magazine_capacity == 1
            && self.initial_magazine_count > 0
            && (0.001..=60.0).contains(&self.refire_seconds)
            && (0.001..=120.0).contains(&self.reload_seconds)
            && weapon["infinite_ammo"].as_bool() == Some(false)
            && weapon["ammo_name"].as_str() == Some(ammo_name)
            && weapon["magazine_capacity"].as_u64() == Some(u64::from(self.magazine_capacity))
            && weapon["initial_magazine_count"].as_u64()
                == Some(u64::from(self.initial_magazine_count))
            && weapon["fire_mode"]["refire_time"]
                .as_f64()
                .map(|value| value as f32)
                == Some(self.refire_seconds)
            && weapon["fire_mode"]["reload_time"]
                .as_f64()
                .map(|value| value as f32)
                == Some(self.reload_seconds)
    }
}

impl Ammo {
    fn matches(&self, items: &ItemCatalog) -> bool {
        let row = &items.raw()["definitions"]["projectile_ammo"][&self.name];
        let source = &row["parameters"];
        self.token != 0
            && row["native_token_u32"].as_u64() == Some(u64::from(self.token))
            && source["id"].as_str() == Some(self.name.as_str())
            && self.authored.matches(source)
    }
}

impl Authored {
    fn valid(self) -> bool {
        let damage = self.damage;
        [
            self.launch_velocity,
            self.armed_time,
            self.lifetime,
            self.detonation_timer,
            self.mass,
            self.friction,
            self.restitution,
            self.linear_damping,
            self.linear_damping_on_ground,
            self.angular_damping,
            self.angular_damping_on_ground,
            self.bounce_damping_default,
            self.bounce_damping_character,
            self.bounce_to_explode,
            self.impact_damage,
            self.full_damage_radius,
            self.half_damage_radius,
            self.material_penetration,
            damage.character,
            damage.character_apierce,
            damage.character_aredux,
            damage.vehicle,
            damage.vehicle_apierce,
            damage.vehicle_aredux,
            damage.objective,
            damage.objective_apierce,
            damage.objective_aredux,
            damage.mine,
        ]
        .into_iter()
        .all(|value| value.is_finite() && value >= 0.0)
            && self.launch_velocity > 0.0
            && self.mass > 0.0
            && self.radius.is_finite()
            && self.radius > 0.0
            && self
                .capsule_height
                .into_iter()
                .map(|axis| axis * axis)
                .sum::<f64>()
                <= 4.0
            && self.lifetime >= 0.001
            && self.armed_time < self.lifetime
            && self.lifetime <= 68.0
            && (0.0..=1.0).contains(&self.restitution)
            && self.full_damage_radius > self.half_damage_radius
            && self.half_damage_radius > 0.0
    }

    fn matches(&self, source: &Value) -> bool {
        if !self.valid() {
            return false;
        }
        let Ok(Value::Object(mut fields)) = serde_json::to_value(self) else {
            return false;
        };
        for (field, value) in &mut fields {
            *value = source[field].clone();
        }
        serde_json::from_value::<Self>(Value::Object(fields)).is_ok_and(|raw| &raw == self)
    }
}

/// The common receiver and contact checks apply to both infantry projectile types.
#[doc(hidden)]
pub fn map_support(result: Result<(), &'static str>) -> Result<(), &'static str> {
    result.map_err(|error| match error {
        "infantry_rocket_mode_required" => "underbarrel_launcher_mode_required",
        "infantry_rocket_contact_bindings_required" => {
            "underbarrel_launcher_contact_bindings_required"
        }
        "infantry_rocket_acquisition_structural_required" => {
            "underbarrel_launcher_acquisition_structural_required"
        }
        "infantry_rocket_acquisition_ground_required" => {
            "underbarrel_launcher_acquisition_ground_required"
        }
        "infantry_rocket_domination_ground_required" => {
            "underbarrel_launcher_domination_ground_required"
        }
        "infantry_rocket_domination_hospital_required" => {
            "underbarrel_launcher_domination_hospital_required"
        }
        "infantry_rocket_domination_helo_required" => {
            "underbarrel_launcher_domination_helo_required"
        }
        "infantry_rocket_world_resources_required" => {
            "underbarrel_launcher_world_resources_required"
        }
        _ => error,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;

    pub(crate) fn synthetic_catalog() -> Catalog {
        Catalog {
            variants: std::array::from_fn(|index| Variant {
                faction: index as u8,
                host_item_id: 101 + 100 * index as u16,
                attachment_item_id: 102 + 100 * index as u16,
                magazine_capacity: 1,
                initial_magazine_count: 3,
                refire_seconds: 0.5,
                reload_seconds: 2.0,
            }),
            ammo: Ammo {
                name: "synthetic_launcher_round".into(),
                token: 0x1234_4321,
                authored: Authored {
                    propulsion_strategy: Propulsion::Thrown,
                    detonation_strategy: Detonation::Impact,
                    explosion_strategy: Explosion::Debris,
                    collision_type: CollisionShape::Capsule,
                    launch_velocity: 24.0,
                    armed_time: 0.2,
                    lifetime: 10.0,
                    detonation_timer: 3.0,
                    radius: 0.08,
                    mass: 0.75,
                    friction: 0.5,
                    restitution: 0.4,
                    linear_damping: 0.2,
                    linear_damping_on_ground: 0.0,
                    angular_damping: 1.0,
                    angular_damping_on_ground: 0.0,
                    bounce_damping_default: 0.5,
                    bounce_damping_character: 0.5,
                    bounce_to_explode: 1.0,
                    impact_damage: 10.0,
                    full_damage_radius: 6.0,
                    half_damage_radius: 3.0,
                    material_penetration: 2.0,
                    capsule_height: [0.0, 0.0, 0.05],
                    damage: Damage {
                        character: 40.0,
                        character_apierce: 0.0,
                        character_aredux: 0.25,
                        vehicle: 80.0,
                        vehicle_apierce: 0.0,
                        vehicle_aredux: 0.125,
                        objective: 0.0,
                        objective_apierce: 0.0,
                        objective_aredux: 0.0,
                        mine: 0.0,
                    },
                    destroy_objective: false,
                },
            },
        }
    }

    #[test]
    fn rejects_unsafe_authored_flight_and_blast_bounds() {
        let base = synthetic_catalog().ammo().authored;
        assert!(base.valid());
        for bad in [
            Authored {
                radius: f64::INFINITY,
                ..base
            },
            Authored {
                capsule_height: [0.0, f64::NAN, 0.0],
                ..base
            },
            Authored {
                capsule_height: [1.5, 1.5, 0.0],
                ..base
            },
            Authored {
                lifetime: 68.5,
                ..base
            },
            Authored {
                armed_time: base.lifetime,
                ..base
            },
            Authored {
                full_damage_radius: base.half_damage_radius,
                ..base
            },
            Authored {
                restitution: 1.1,
                ..base
            },
            Authored {
                linear_damping: -1.0,
                ..base
            },
            Authored {
                damage: Damage {
                    vehicle_aredux: f32::INFINITY,
                    ..base.damage
                },
                ..base
            },
        ] {
            assert!(!bad.valid());
        }
    }

    #[test]
    fn validates_authored_armor_and_impact_as_separate_fields() {
        let original = json!({
            "id": "synthetic_round",
            "propulsion_strategy": "kWepPropulsionStrategyThrown",
            "detonation_strategy": "kWepDetonationStrategyImpact",
            "explosion_strategy": "kWepExplosionStrategyDebris",
            "collisionType": "kWepProjCollisionShapeCapsule",
            "launch_velocity": 9, "m_fArmedTime": 0.125, "m_fLifetimeMax": 4.0,
            "detonation_timer": 2, "radius": 0.2, "mass": 0.7, "friction": 1,
            "restitution": 0.25, "linearDamping": 0.15, "linearDampingOnGround": 0,
            "angularDamping": 1, "angularDampingOnGround": 0,
            "fBounceDampingDefault": 0.5, "fBounceDampingCharacter": 0.5,
            "m_fBounceToExplode": 1, "m_fImpactDamage": 10,
            "full_damage_radius": 4, "half_damage_radius": 2, "MaterialPenetration": 1.5,
            "capsuleHeight": [0, 0, 0.125], "destroy_objective": false,
            "damage": {
                "character": 15, "characterAPierce": 0, "characterARedux": 0.25,
                "vehicle": 20, "vehicleAPierce": 0, "vehicleARedux": 0.125,
                "objective": 25, "objectiveAPierce": 0, "objectiveARedux": 0, "mine": 4
            }
        });
        let mut projected = original.clone();
        projected.as_object_mut().unwrap().remove("id");
        let authored: Authored = serde_json::from_value(projected).unwrap();
        assert!(authored.matches(&original));

        let mut changed = original.clone();
        changed["damage"]["vehicleARedux"] = json!(0);
        assert!(!authored.matches(&changed));
        let mut changed = original;
        changed["m_fImpactDamage"] = json!(0);
        assert!(!authored.matches(&changed));
    }

    #[test]
    fn verifies_finite_launcher_weapon_not_the_stock_rifle_magazine() {
        let variant = *synthetic_catalog().variant(0, 101, 102).unwrap();
        let weapon = json!({
            "infinite_ammo": false, "ammo_name": "synthetic_round",
            "magazine_capacity": 1, "initial_magazine_count": 3,
            "fire_mode": {"refire_time": 0.5, "reload_time": 2.0}
        });
        assert!(variant.matches_weapon(&weapon, "synthetic_round"));
        assert_eq!(variant.initial_count(), 3);
        let multi_round = Variant {
            magazine_capacity: 2,
            ..variant
        };
        let mut different = weapon.clone();
        different["magazine_capacity"] = json!(2);
        assert!(!multi_round.matches_weapon(&different, "synthetic_round"));
        let mut different = weapon;
        different["infinite_ammo"] = json!(true);
        assert!(!variant.matches_weapon(&different, "synthetic_round"));
    }
}
