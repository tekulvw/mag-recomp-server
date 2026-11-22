//! Optional source-qualified Skill25 launcher rules; the stock schema is unchanged.
use crate::{
    guided_launcher_motion::Steering,
    hospital_rockets::ammo_crc32,
    infantry_rockets::{self, Blast, Flight, Weapon},
    item_catalog::ItemCatalog,
};
use serde::Deserialize;
use serde_json::Value;
const ELF: &str = "0089f0d6d17e48533ce7ff8d943dddd03487b925e7b04a1a055bfb907568694c";
const ARCHIVE: &str = "b49c9435be729832029ad7019874d478080dc9764ebde1809b2851d353bc769f";
#[derive(Clone, Debug)]
pub struct Catalog {
    variants: [Variant; 3],
    pub(crate) guidance: Guidance,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    schema: String,
    application_id: u32,
    source: Source,
    guidance: Guidance,
    variants: [Variant; 3],
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Guidance {
    pub guided_max_range_m: f32,
    anchor_policy: String,
    pub anchor_y_m: Anchors,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Anchors {
    pub ground: f32,
    pub helo: f32,
    pub hospital: f32,
}
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Variant {
    pub faction: u8,
    pub item_id: u16,
    pub stock_sight_item_id: u16,
    pub ammo_token: u32,
    pub weapon: Weapon,
    pub flight: Flight,
    pub blast: Blast,
    pub steering: Steering,
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
    skill_catalog_sha256: String,
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
            && self.native_proof_sha256
                == "bff49759c5901d4328b66f82657a02b050ca2467a60dca399c4b6b0b4f6c51cb"
            && digest(&self.skill_catalog_sha256)
            && items
                .retail_armory()
                .and_then(|armory| armory.skill_purchases_sha256())
                == Some(self.skill_catalog_sha256.as_str())
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
    pub(crate) fn base(self) -> infantry_rockets::Variant {
        infantry_rockets::Variant {
            faction: self.faction,
            item_id: self.item_id,
            stock_sight_item_id: self.stock_sight_item_id,
            ammo_token: self.ammo_token,
            weapon: self.weapon,
            flight: self.flight,
            blast: self.blast,
        }
    }
    fn sight(&self, items: &ItemCatalog, weapon: &Value) -> Result<(), &'static str> {
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
        Ok(())
    }
    fn matches(self, ammo: &Value) -> bool {
        let numbers = [
            ("launch_velocity", self.flight.speed_mps),
            ("m_fArmedTime", self.flight.armed_time_seconds),
            ("m_fLifetimeMax", self.flight.lifetime_seconds),
            (
                "rocketAutoExplodeDistance",
                self.flight.auto_explode_distance_m,
            ),
            (
                "rocketGuidedMaxDirectionChange",
                self.steering.guided_max_direction_change,
            ),
            (
                "rocketDumbCruiseStraightRange",
                self.steering.dumb_cruise_range_m,
            ),
        ];
        ammo["propulsion_strategy"].as_str() == Some("kWepPropulsionStrategyRocketGuided")
            && ammo["collisionType"].as_str() == Some("kWepProjCollisionShapeCapsule")
            && ammo["detonation_strategy"].as_str() == Some("kWepDetonationStrategyImpact")
            && ammo["explosion_strategy"].as_str() == Some("kWepExplosionStrategyDebris")
            && ammo["detonation_timer"].as_f64() == Some(0.)
            && self.weapon.magazine_size == 1
            && self.weapon.initial_magazine_count == 3
            && self.flight.speed_mps == 60.
            && self.flight.armed_time_seconds == 0.1
            && self.flight.lifetime_seconds == 10.
            && self.flight.age_grace_seconds == 2.
            && self.flight.auto_explode_distance_m == 150.
            && self.flight.radius_m == 0.05
            && self.flight.capsule_spine_local_xyz == [0., 0., 0.05]
            && ammo["radius"].as_f64() == Some(self.flight.radius_m)
            && ammo["capsuleHeight"].as_array().is_some_and(|v| {
                v.len() == 3
                    && v.iter()
                        .zip(self.flight.capsule_spine_local_xyz)
                        .all(|(v, n)| v.as_f64() == Some(n))
            })
            && numbers
                .into_iter()
                .all(|(key, v)| ammo[key].as_f64().map(|n| n as f32) == Some(v))
            && [
                (
                    "rocketGuidedDecisionTimeRange",
                    [
                        self.steering.guided_decision_min_seconds,
                        self.steering.guided_decision_max_seconds,
                    ],
                ),
                (
                    "rocketDumbDecisionTimeRange",
                    [
                        self.steering.dumb_decision_min_seconds,
                        self.steering.dumb_decision_max_seconds,
                    ],
                ),
                (
                    "rocketDumbDirectionChangeRange",
                    [
                        self.steering.dumb_direction_change_min,
                        self.steering.dumb_direction_change_max,
                    ],
                ),
            ]
            .into_iter()
            .all(|(key, pair)| {
                ammo[key].as_array().is_some_and(|v| {
                    v.len() == 2
                        && v.iter()
                            .zip(pair)
                            .all(|(v, n)| v.as_f64().map(|n| n as f32) == Some(n))
                })
            })
            && self.steering
                == Steering {
                    guided_decision_min_seconds: 0.01,
                    guided_decision_max_seconds: 0.1,
                    guided_max_direction_change: 0.07,
                    dumb_decision_min_seconds: 0.01,
                    dumb_decision_max_seconds: 0.01,
                    dumb_direction_change_min: 0.005,
                    dumb_direction_change_max: 0.005,
                    dumb_cruise_range_m: 2.,
                    dumb_max_range_m: 150.,
                }
            && ammo["damage"]["objective"].as_f64() == Some(1200.)
            && self.blast.character_hp == 175.
            && self.blast.vehicle_hp == 1200.
            && self.blast.half_radius_m == 2.
            && self.blast.outer_radius_m == 5.
            && self.blast.material_penetration == 1.7
            && self.blast.matches(ammo)
    }
}
impl Catalog {
    #[cfg(test)]
    pub(crate) fn synthetic(variants: [Variant; 3]) -> Self {
        Self {
            variants,
            guidance: Guidance {
                guided_max_range_m: 150.,
                anchor_policy: "retail_constructor_seed".into(),
                anchor_y_m: Anchors {
                    ground: 0.8,
                    helo: -2.,
                    hospital: 1.,
                },
            },
        }
    }

    pub fn variant(&self, faction: u8, item: u16) -> Option<&Variant> {
        self.variants
            .get(usize::from(faction))
            .filter(|v| v.item_id == item)
    }
    pub fn from_bytes(raw: &[u8], items: &ItemCatalog, elf: &str) -> Result<Self, &'static str> {
        if raw.len() > 1 << 20 {
            return Err("invalid_guided_launcher_rules");
        }
        let doc: Document =
            serde_json::from_slice(raw).map_err(|_| "invalid_guided_launcher_rules")?;
        let g = &doc.guidance;
        if doc.schema != "mag_retail_guided_launchers_v1"
            || doc.application_id != 20231
            || items.application_id() != 20231
            || !doc.source.matches(items, elf)
            || g.anchor_policy != "retail_constructor_seed"
            || g.guided_max_range_m != 150.
            || g.anchor_y_m.ground != 0.8
            || g.anchor_y_m.helo != -2.
            || g.anchor_y_m.hospital != 1.
        {
            return Err("guided_launcher_source_mismatch");
        }
        for (f, v) in doc.variants.iter().enumerate() {
            if usize::from(v.faction) != f
                || v.item_id != [1089, 2111, 3137][f]
                || v.stock_sight_item_id != [1091, 2113, 3139][f]
            {
                return Err("guided_launcher_role_mismatch");
            }
            let weapon = items
                .weapon_parameters(v.item_id)
                .map_err(|_| "guided_launcher_role_mismatch")?;
            v.sight(items, weapon)?;
            if !v.weapon.matches(weapon) {
                return Err("guided_launcher_weapon_mismatch");
            }
            let name = weapon["ammo_name"]
                .as_str()
                .ok_or("guided_launcher_ammo_mismatch")?;
            let ammo = &items.raw()["definitions"]["projectile_ammo"][name]["parameters"];
            if ammo["id"].as_str() != Some(name)
                || name != ["rocket_green", "rocket_black", "rocket_red"][f]
                || ammo_crc32(name) != Some(v.ammo_token)
                || !v.matches(ammo)
            {
                return Err("guided_launcher_ammo_mismatch");
            }
        }
        Ok(Self {
            variants: doc.variants,
            guidance: doc.guidance,
        })
    }
}
pub(crate) fn equipped<'a>(actor: &crate::Active, catalog: &'a Catalog) -> Option<&'a Variant> {
    let kit = actor.equipped_loadout()?;
    let slots = &kit
        .inventories
        .get(usize::from(kit.handle.inventory_index))?
        .slots;
    let v = catalog.variant(actor.reservation.character.faction, slots[3]?)?;
    (slots[21] == Some(v.stock_sight_item_id)).then_some(v)
}
pub(crate) fn owns_skill(actor: &crate::Active) -> bool {
    actor
        .reservation
        .character
        .skills
        .is_some_and(|s| s[0] & (1 << 25) != 0)
}
