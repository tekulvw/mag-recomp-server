//! Source-qualified, retail poison grenade equipment.

use crate::{
    hospital_rockets::ammo_crc32,
    item_catalog::{DamageParameters, GrenadeParameters, ItemCatalog},
};
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeSet;

const POISON_SKILL: usize = 50;
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
    skill_catalog_sha256: String,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Variant {
    pub faction: u8,
    pub item_id: u16,
    pub ammo_token: u32,
    pub initial_count: u8,
    pub fuse_ms: u64,
    pub projectile_lifetime_ms: u64,
    pub volume_duration_ms: u64,
    pub warmup_ms: u64,
    pub interval_ms: u64,
    pub inner_radius_m: f32,
    pub inner_scale: f32,
    pub outer_radius_m: f32,
    pub outer_scale: f32,
    pub character_hp_per_pulse: f32,
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
            return Err("invalid_gas_grenade_rules");
        }
        let doc: Document = serde_json::from_slice(raw).map_err(|_| "invalid_gas_grenade_rules")?;
        if doc.schema != "mag_retail_gas_grenades_v1"
            || doc.application_id != APPLICATION
            || items.application_id() != APPLICATION
            || !doc.source.matches(items, native_elf)
        {
            return Err("gas_grenade_source_mismatch");
        }
        let mut tokens = BTreeSet::new();
        for (index, variant) in doc.variants.iter().enumerate() {
            let namespace = format!("FactionItemList{}:", index + 1);
            if usize::from(variant.faction) != index
                || !items
                    .retail_authored_ref(variant.item_id)
                    .is_some_and(|item| item.starts_with(&namespace))
            {
                return Err("gas_grenade_role_mismatch");
            }
            let weapon = items
                .weapon_parameters(variant.item_id)
                .map_err(|_| "gas_grenade_role_mismatch")?;
            let ammo_name = weapon["ammo_name"]
                .as_str()
                .ok_or("gas_grenade_ammo_mismatch")?;
            let ammo = &items.raw()["definitions"]["projectile_ammo"][ammo_name]["parameters"];
            if !variant.matches(weapon, ammo_name, ammo) || !tokens.insert(variant.ammo_token) {
                return Err("gas_grenade_ammo_mismatch");
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
        weapon["bSupportLightAmmoPack"].as_bool() == Some(false)
            && weapon["category"].as_str() == Some("kWepCategoryGrenade")
            && weapon["infinite_ammo"].as_bool() == Some(false)
            && weapon["magazine_capacity"].as_u64() == Some(1)
            && self.initial_count == 2
            && weapon["initial_magazine_count"].as_u64() == Some(u64::from(self.initial_count))
            && self.ammo_token != 0
            && ammo_crc32(ammo_name) == Some(self.ammo_token)
            && ammo["id"].as_str() == Some(ammo_name)
            && ammo["propulsion_strategy"].as_str() == Some("kWepPropulsionStrategyThrown")
            && ammo["detonation_strategy"].as_str() == Some("kWepDetonationStrategyTimer")
            && ammo["explosion_strategy"].as_str() == Some("kWepExplosionStrategyVolume")
            && ammo["m_eAmmoPersistance"].as_str() == Some("kWepPersistanceGrenadeGasPoison")
            && ammo["destroy_objective"].as_bool() == Some(false)
            && ammo["doHearingLoss"].as_bool() == Some(false)
            && ammo["damage"].as_object().is_some_and(|row| {
                row.len() == channels.len()
                    && channels.iter().all(|channel| {
                        row.get(*channel).and_then(Value::as_f64)
                            == Some(if *channel == "character" {
                                f64::from(self.character_hp_per_pulse)
                            } else {
                                0.0
                            })
                    })
            })
            && self.character_hp_per_pulse.is_finite()
            && self.character_hp_per_pulse > 0.0
            && self.inner_radius_m.is_finite()
            && self.inner_radius_m > 0.0
            && self.outer_radius_m.is_finite()
            && self.outer_radius_m > self.inner_radius_m
            && self.inner_scale == 0.5
            && self.outer_scale == 0.0
            && ammo["half_damage_radius"].as_f64() == Some(f64::from(self.inner_radius_m))
            && ammo["full_damage_radius"].as_f64() == Some(f64::from(self.outer_radius_m))
            && self.interval_ms == 500
            && self.warmup_ms < self.volume_duration_ms
            && self.fuse_ms < self.projectile_lifetime_ms
            && [
                ("detonation_timer", self.fuse_ms),
                ("m_fLifetimeMax", self.projectile_lifetime_ms),
                ("m_fInitialDelay", self.warmup_ms),
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

impl<W: std::io::Write> crate::Engine<W> {
    pub(crate) fn tick_gas_grenades(
        &mut self,
        now: std::time::Duration,
        wall: u64,
    ) -> Result<Vec<crate::OutboundMessage>, &'static str> {
        use crate::domination_structural_world::BlastCollision;
        if self.gas_grenades.is_none() || !self.round_combat_allowed(now) {
            return Ok(Vec::new());
        }
        let mut staged = std::collections::BTreeMap::new();
        let mut pulses = Vec::new();
        for actor in self.active.iter().chain(self.peers.values()) {
            if actor
                .smoke_volumes
                .volumes()
                .iter()
                .any(|volume| volume.poison.is_some())
            {
                let mut volumes = actor.smoke_volumes.clone();
                pulses.extend(volumes.advance_poison(now, self.round_number));
                staged.insert(actor.reservation.id.clone(), volumes);
            }
        }
        let messages = if pulses.is_empty() {
            Vec::new()
        } else {
            let collision = self
                .poison_grenade_overlap()
                .ok_or("gas_grenade_world_required")?;
            let filter_rows = *collision.filter_rows;
            let limits = collision.limits;
            let world = pulses[0].source.world;
            let mut native = self.owned_acquisition_characters();
            let candidate =
                self.prepare_acquisition_character_callback(&native, now, |engine, native| {
                    engine.stage_gas_pulses(
                        pulses,
                        native,
                        now,
                        BlastCollision {
                            filter_rows: &filter_rows,
                            limits,
                            sequences: &Default::default(),
                            material_costs: &Default::default(),
                            terrain_materials: None,
                        },
                    )
                })?;
            if candidate.check_extra_shared(self, world, 0).is_err() {
                return Ok(Vec::new());
            }
            self.record_acquisition_character_effects(candidate, &mut native, wall)?
        };
        for actor in self.active.iter_mut().chain(self.peers.values_mut()) {
            if let Some(volumes) = staged.remove(&actor.reservation.id) {
                actor.smoke_volumes = volumes;
            }
        }
        Ok(messages)
    }

    pub fn with_gas_grenades(
        mut self,
        catalog: std::sync::Arc<Catalog>,
    ) -> Result<Self, &'static str> {
        if self.wire_profile.application_id() != APPLICATION
            || !matches!(self.match_profile().game_type, 1 | 2)
            || self.dynamic_body_resources.is_none()
            || self.character_damage_rules.is_none()
            || self
                .poison_grenade_overlap()
                .is_none_or(|collision| collision.filter_rows[6] & (1 << 21) == 0)
        {
            return Err("gas_grenades_require_character_collision");
        }
        self.gas_grenades = Some(catalog);
        Ok(self)
    }

    fn poison_grenade_overlap(
        &self,
    ) -> Option<crate::domination_structural_world::BlastCollision<'_>> {
        self.acquisition_gameplay
            .as_ref()
            .and_then(|runtime| runtime.blast_collision())
            .or_else(|| {
                self.domination_gameplay
                    .as_ref()
                    .map(|world| world.frag_blast_collision())
            })
    }
}

#[derive(Clone, Debug)]
pub(crate) struct LaunchSource {
    pub round: u64,
    pub gas: crate::active_ability_gas_world::GasSource,
}
impl LaunchSource {
    pub(crate) fn from_actor(actor: &crate::Active, round: u64) -> Self {
        Self {
            round,
            gas: crate::active_ability_gas_world::GasSource {
                allocation: Some(actor.allocation_sequence),
                attacker: 0x0380_0000 | u32::from(actor.reservation.player),
                team: actor.team(),
                world: actor.reservation.world,
                reservation: actor.reservation.id.clone(),
            },
        }
    }
    pub(crate) fn current(&self, actor: &crate::Active, round: u64) -> bool {
        self.round == round
            && self.gas.world == actor.reservation.world
            && self.gas.reservation == actor.reservation.id
            && self.gas.attacker == 0x0380_0000 | u32::from(actor.reservation.player)
            && self.gas.allocation == Some(actor.allocation_sequence)
    }
}

pub(crate) fn owns_poison_skill(actor: &crate::Active) -> bool {
    actor
        .reservation
        .character
        .skills
        .is_some_and(|skills| skills[POISON_SKILL / 32] & (1 << (POISON_SKILL % 32)) != 0)
}
