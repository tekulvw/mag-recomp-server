//! Source-bound stock human Flash grenade interoperability. No assets are embedded.
use crate::{
    hospital_rockets::ammo_crc32,
    item_catalog::{DamageParameters, GrenadeParameters, ItemCatalog},
    wire_profile::WireProfile,
};
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeSet;
const APPLICATION: u32 = 20231;
const ELF: &str = "0089f0d6d17e48533ce7ff8d943dddd03487b925e7b04a1a055bfb907568694c";
const ARCHIVE: &str = "b49c9435be729832029ad7019874d478080dc9764ebde1809b2851d353bc769f";
#[derive(Clone, Debug)]
pub struct Catalog {
    variants: [Variant; 3],
    blind: Blind,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    schema: String,
    application_id: u32,
    source: Source,
    blind: Blind,
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
pub struct Blind {
    pub strong_duration_s: f32,
    pub weak_duration_s: f32,
    pub facing_angle_rad: f32,
    pub inner_scale: f32,
    pub outer_scale: f32,
    pub yaw_max: u16,
    pub ray_filter_mask: u32,
    pub message_id: u16,
    pub duration_integer_bits: u8,
    pub duration_fraction_bits: u8,
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
    pub inner_radius_m: f32,
    pub outer_radius_m: f32,
    pub material_penetration: f32,
}
impl Blind {
    fn valid(self) -> bool {
        self == Self::stock()
    }
    pub(crate) fn stock() -> Self {
        Self {
            strong_duration_s: 10.,
            weak_duration_s: 0.1,
            facing_angle_rad: 1.396_263_5,
            inner_scale: 0.5,
            outer_scale: 0.,
            yaw_max: 1023,
            ray_filter_mask: 0x0040_0601,
            message_id: 0x0200,
            duration_integer_bits: 5,
            duration_fraction_bits: 10,
        }
    }
    /// Body yaw uses native 1/1024 scale; the source delta is unadjusted body origin.
    pub(crate) fn duration(
        self,
        variant: Variant,
        distance: f32,
        delta: [f32; 3],
        yaw: Option<u16>,
        clear: bool,
    ) -> Result<f32, &'static str> {
        if !distance.is_finite() || distance < 0. || !delta.iter().all(|v| v.is_finite()) {
            return Err("invalid_flash_geometry");
        }
        let scale = crate::armor::native_radial_scale(
            distance,
            variant.inner_radius_m,
            variant.outer_radius_m,
        )?;
        if scale <= 0. {
            return Ok(0.);
        }
        let strong = if clear {
            let yaw = yaw
                .filter(|v| *v <= self.yaw_max)
                .ok_or("flash_facing_witness_required")?;
            let theta = f32::from(yaw) * (1. / 1024.) * std::f32::consts::TAU;
            let squared = delta.iter().map(|v| v * v).sum::<f32>();
            let (sin, cos) = theta.sin_cos();
            let forward_squared = sin * sin + cos * cos;
            let dot = if squared > f32::MIN_POSITIVE && forward_squared > f32::MIN_POSITIVE {
                (delta[0] * sin + delta[2] * cos) / (squared.sqrt() * forward_squared.sqrt())
            } else {
                0.
            };
            dot > self.facing_angle_rad.cos()
        } else {
            false
        };
        Ok(if strong {
            self.strong_duration_s
        } else {
            self.weak_duration_s
        } * scale)
    }
}
impl Catalog {
    #[cfg(test)]
    pub(crate) fn synthetic(variants: [Variant; 3]) -> Self {
        Self {
            variants,
            blind: Blind::stock(),
        }
    }
    pub fn from_bytes(
        raw: &[u8],
        items: &ItemCatalog,
        native_elf: &str,
    ) -> Result<Self, &'static str> {
        if raw.len() > 1 << 20 {
            return Err("invalid_flash_grenade_rules");
        }
        let doc: Document =
            serde_json::from_slice(raw).map_err(|_| "invalid_flash_grenade_rules")?;
        if doc.schema != "mag_retail_flash_grenades_v1"
            || doc.application_id != APPLICATION
            || items.application_id() != APPLICATION
            || !doc.source.matches(items, native_elf)
            || !doc.blind.valid()
        {
            return Err("flash_grenade_source_mismatch");
        }
        let mut tokens = BTreeSet::new();
        for (index, v) in doc.variants.iter().enumerate() {
            let namespace = format!("FactionItemList{}:", index + 1);
            if v.item_id != [1099, 2122, 3148][index]
                || usize::from(v.faction) != index
                || !items
                    .retail_authored_ref(v.item_id)
                    .is_some_and(|r| r.starts_with(&namespace))
            {
                return Err("flash_grenade_role_mismatch");
            }
            let weapon = items
                .weapon_parameters(v.item_id)
                .map_err(|_| "flash_grenade_role_mismatch")?;
            let name = weapon["ammo_name"]
                .as_str()
                .ok_or("flash_grenade_ammo_mismatch")?;
            let ammo = &items.raw()["definitions"]["projectile_ammo"][name]["parameters"];
            if !v.matches(weapon, name, ammo, doc.blind) || !tokens.insert(v.ammo_token) {
                return Err("flash_grenade_ammo_mismatch");
            }
        }
        Ok(Self {
            variants: doc.variants,
            blind: doc.blind,
        })
    }
    pub fn variant(&self, faction: u8, item: u16) -> Option<&Variant> {
        self.variants
            .get(usize::from(faction))
            .filter(|v| v.item_id == item)
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
            && self.native_proof_sha256
                == "2bf5bd79b62efddba0a3c50e6df9c26aae8fce0e25a61604a8b15c83e1885317"
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
    fn matches(self, weapon: &Value, name: &str, ammo: &Value, blind: Blind) -> bool {
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
            && self.initial_count == 3
            && weapon["initial_magazine_count"].as_u64() == Some(3)
            && name
                == [
                    "grenade_flash_green_ammo",
                    "grenade_flash_black_ammo",
                    "grenade_flash_red_ammo",
                ][usize::from(self.faction)]
            && self.ammo_token != 0
            && ammo_crc32(name) == Some(self.ammo_token)
            && ammo["id"].as_str() == Some(name)
            && ammo["propulsion_strategy"].as_str() == Some("kWepPropulsionStrategyThrown")
            && ammo["detonation_strategy"].as_str() == Some("kWepDetonationStrategyTimer")
            && ammo["explosion_strategy"].as_str() == Some("kWepExplosionStrategyDebris")
            && ammo["destroy_objective"].as_bool() == Some(false)
            && ammo["doHearingLoss"].as_bool() == Some(true)
            && ammo["damage"].as_object().is_some_and(|r| {
                r.len() == channels.len()
                    && channels
                        .iter()
                        .all(|k| r.get(*k).and_then(Value::as_f64) == Some(0.))
            })
            && ammo["MaterialPenetration"]
                .as_f64()
                .is_some_and(|v| v as f32 == self.material_penetration)
            && self.inner_radius_m == 10.
            && self.outer_radius_m == 40.
            && self.material_penetration == 1.7
            && self.fuse_ms == 3000
            && self.projectile_lifetime_ms == 30000
            && [
                ("half_damage_radius", self.inner_radius_m),
                ("full_damage_radius", self.outer_radius_m),
                ("m_fDuration", blind.strong_duration_s),
                ("m_fInitialDelay", blind.weak_duration_s),
            ]
            .into_iter()
            .all(|(k, v)| ammo[k].as_f64().is_some_and(|n| n as f32 == v))
            && [
                ("detonation_timer", self.fuse_ms),
                ("m_fLifetimeMax", self.projectile_lifetime_ms),
            ]
            .into_iter()
            .all(|(k, v)| ammo[k].as_f64() == Some(v as f64 / 1000.))
    }
    pub fn parameters(self) -> GrenadeParameters {
        GrenadeParameters {
            damage: DamageParameters {
                character: 0.,
                armor_pierce: 0.,
                armor_redux: 0.,
            },
            ammo_token: self.ammo_token,
            full_radius: 0.,
            half_radius: 0.,
            fuse_ms: self.fuse_ms,
            authored_lifetime_ms: self.projectile_lifetime_ms,
            initial_count: self.initial_count,
        }
    }
}
/// Direct application message; never an ObjectUpdate or a server-side status.
pub fn encode_blinded(duration: f32, profile: WireProfile) -> Result<Vec<u8>, &'static str> {
    if profile.application_id() != APPLICATION
        || !duration.is_finite()
        || duration <= 0.
        || duration >= 32.
    {
        return Err("invalid_flash_duration");
    }
    let whole = duration.trunc() as u32;
    let fraction = ((duration - whole as f32) * 1024.).trunc() as u32;
    let mut bits = crate::replication::BitWriter::with_profile(profile);
    bits.write(0x0200, 16)?;
    bits.write(1, 1)?;
    bits.write(0, 1)?;
    bits.write(whole, 5)?;
    bits.write(fraction, 10)?;
    Ok(bits.finish())
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct YawWitness {
    pub allocation: u64,
    pub generation: u32,
    pub yaw: u16,
}
impl YawWitness {
    pub(crate) fn current(self, actor: &crate::Active) -> Option<u16> {
        (actor.allocation_sequence==self.allocation && !actor.is_mounted() && matches!(actor.life,crate::CharacterLife::Alive {generation,..} if generation==self.generation)).then_some(self.yaw)
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flash_duration_uses_body_yaw_strict_cone_and_stance_distance() {
        let b = Blind::stock();
        let v = Variant {
            faction: 0,
            item_id: 1099,
            ammo_token: 1,
            initial_count: 3,
            fuse_ms: 3000,
            projectile_lifetime_ms: 30000,
            inner_radius_m: 10.,
            outer_radius_m: 40.,
            material_penetration: 1.7,
        };
        for (yaw, delta) in [
            (0, [0., 0., 1.]),
            (256, [1., 0., 0.]),
            (512, [0., 0., -1.]),
            (768, [-1., 0., 0.]),
        ] {
            assert_eq!(b.duration(v, 0., delta, Some(yaw), true), Ok(10.));
        }
        assert_eq!(b.duration(v, 10., [0., 0., 1.], Some(0), true), Ok(5.));
        assert_eq!(b.duration(v, 25., [0., 0., 1.], Some(0), true), Ok(2.5));
        assert_eq!(b.duration(v, 40., [0., 0., 1.], None, true), Ok(0.));
        assert_eq!(b.duration(v, 0., [0.; 3], Some(0), true), Ok(0.1));
        assert_eq!(b.duration(v, 0., [1., 0., 0.], Some(0), true), Ok(0.1));
        assert_eq!(b.duration(v, 0., [0., 0., -1.], Some(0), true), Ok(0.1));
        let angle = b.facing_angle_rad;
        assert_eq!(
            b.duration(v, 0., [angle.sin(), 0., angle.cos()], Some(0), true),
            Ok(0.1)
        );
        assert_eq!(b.duration(v, 10., [0., 0., 1.], None, false), Ok(0.05));
        assert_eq!(
            b.duration(v, 0., [0., 0., 1.], None, true),
            Err("flash_facing_witness_required")
        );
        assert_eq!(
            b.duration(v, 0., [0., 0., 1.], Some(1024), true),
            Err("flash_facing_witness_required")
        );
    }
    #[test]
    fn flash_native_fixed_5_10_vectors_and_bounds() {
        let p = crate::wire_profile::WireProfileCatalog::embedded()
            .profile(20231)
            .unwrap();
        assert_eq!(encode_blinded(10., p).unwrap(), vec![2, 0, 0x94, 0, 0]);
        assert_eq!(encode_blinded(0.1, p).unwrap(), vec![2, 0, 0x80, 0x33, 0]);
        assert_eq!(encode_blinded(0.0009, p).unwrap(), vec![2, 0, 0x80, 0, 0]);
        assert_eq!(encode_blinded(0.5, p).unwrap(), vec![2, 0, 0x81, 0, 0]);
        assert_eq!(
            encode_blinded(31.999_023, p).unwrap(),
            vec![2, 0, 0xbf, 0xff, 0x80]
        );
        for value in [0., -1., 32., f32::NAN, f32::INFINITY] {
            assert!(encode_blinded(value, p).is_err());
        }
    }
}

impl<W: std::io::Write> crate::Engine<W> {
    pub fn with_flash_grenades(
        mut self,
        catalog: std::sync::Arc<Catalog>,
    ) -> Result<Self, &'static str> {
        let collision = self
            .flash_collision()
            .ok_or("flash_grenades_require_character_collision")?;
        if self.wire_profile.application_id() != APPLICATION
            || !matches!(self.match_profile().game_type, 1 | 2)
            || self.dynamic_body_resources.is_none()
            || self.projectile_contacts.is_none()
            || self.collision_world.is_none()
            || collision.filter_rows[6] & (1 << 21) == 0
            || collision.filter_rows[24] != catalog.blind.ray_filter_mask
        {
            return Err("flash_grenades_require_character_collision");
        }
        self.flash_grenades = Some(catalog);
        Ok(self)
    }
    fn flash_collision(&self) -> Option<crate::domination_structural_world::BlastCollision<'_>> {
        self.acquisition_gameplay
            .as_ref()
            .and_then(|r| r.blast_collision())
            .or_else(|| {
                self.domination_gameplay
                    .as_ref()
                    .map(|r| r.frag_blast_collision())
            })
    }
    pub(crate) fn plan_flash(
        &self,
        source: &crate::Active,
        terminal: &crate::projectile::ProjectileTerminal,
        pose: crate::domination_structural_world::OnFootBlastPose,
        ground: Option<&crate::acquisition_ground_runtime::Runtime>,
        self_yaw: Option<YawWitness>,
    ) -> Result<(Vec<crate::OutboundMessage>, Vec<Value>), &'static str> {
        use crate::{
            dynamic_collision::{Geometry, Pose, geometry},
            dynamic_collision_runtime::OwnerCollisionView,
        };
        let catalog = self
            .flash_grenades
            .as_ref()
            .ok_or("flash_catalog_required")?;
        let variant = *catalog
            .variant(source.reservation.character.faction, terminal.item_id)
            .ok_or("flash_variant_required")?;
        let origin = terminal
            .position
            .ok_or("flash_terminal_position_required")?;
        let origin = [origin.x, origin.y, origin.z];
        let collision = self.flash_collision().ok_or("flash_collision_required")?;
        let contacts = self
            .projectile_contacts
            .as_deref()
            .ok_or("flash_contacts_required")?;
        let scene = self
            .collision_world
            .as_ref()
            .ok_or("flash_static_world_required")?;
        let snapshot = self.owner_collision_snapshot(
            collision.filter_rows[6] | collision.filter_rows[24],
            OwnerCollisionView {
                active_position: pose.position,
                ground,
            },
        )?;
        let world = source.reservation.world;
        let mut budget = collision.limits.max_support_points;
        let mut mesh_budget = 1_000_000;
        let sphere = Geometry::Sphere {
            radius: f64::from(variant.outer_radius_m),
        };
        let mut messages = Vec::new();
        let mut audit = Vec::new();
        for body in snapshot.bodies().iter().filter(|b| {
            b.target.kind == crate::acquisition_volume_damage::TargetKind::Character
                && collision.filter_rows[6] & (1 << b.layer) != 0
        }) {
            let Some(actor) = self.active.iter().chain(self.peers.values()).find(|a| {
                a.reservation.world == world
                    && u64::from(0x0380_0000 | u32::from(a.reservation.player))
                        == body.target.key.id
            }) else {
                continue;
            };
            if !actor.ready_for_updates
                || !actor.entered_game
                || actor.initial_objects.is_none()
                || actor.is_mounted()
                || !matches!(actor.life,crate::CharacterLife::Alive {generation,..} if generation==body.target.key.generation)
            {
                continue;
            }
            if !geometry::penetrates(
                &sphere,
                Pose::at(origin.map(f64::from)),
                &body.shape,
                body.pose,
                collision.limits,
                &mut budget,
            )? {
                continue;
            }
            let Some((target, anchor)) =
                self.domination_character_blast_anchor(body.target, world, pose)?
            else {
                continue;
            };
            let base = if actor.reservation.id == source.reservation.id {
                pose.position.map(|p| [p.x, p.y, p.z])
            } else {
                None
            }
            .unwrap_or(self.acquisition_character_position(target)?);
            let distance = origin
                .iter()
                .zip(anchor)
                .map(|(a, b)| (a - b) * (a - b))
                .sum::<f32>()
                .sqrt();
            let ray = collision.flash_clear_path(
                scene,
                &snapshot,
                (
                    origin.map(f64::from),
                    anchor.map(f64::from),
                    variant.material_penetration,
                ),
                contacts,
                &mut mesh_budget,
            );
            let duration = ray.and_then(|clear| {
                catalog.blind.duration(
                    variant,
                    distance,
                    std::array::from_fn(|i| origin[i] - base[i]),
                    if actor.reservation.id == source.reservation.id {
                        self_yaw
                    } else {
                        actor.flash_yaw
                    }
                    .and_then(|w| w.current(actor)),
                    clear,
                )
            });
            match duration {
                Ok(duration) if duration > 0. => {
                    let wire = encode_blinded(duration, self.wire_profile)?;
                    messages.push(crate::OutboundMessage {
                        reservation_id: actor.reservation.id.clone(),
                        world_id: world,
                        player_id: actor.reservation.player,
                        plugin_id: 1,
                        payload_hex: crate::protocol::encode_hex(&wire),
                    });
                    audit.push(serde_json::json!({
                        "character_id": target.character_id, "generation": target.generation,
                        "duration_s": duration, "distance": distance, "supported": true,
                    }));
                }
                Ok(_) => (),
                Err(error) => audit.push(serde_json::json!({
                    "character_id": target.character_id, "generation": target.generation,
                    "supported": false, "reason": error,
                })),
            }
        }
        Ok((messages, audit))
    }
}

#[derive(Clone, Debug)]
pub(crate) struct LaunchSource {
    round: u64,
    allocation: u64,
    life: u32,
    world: u32,
    reservation: String,
    player: u16,
}
impl LaunchSource {
    pub(crate) fn from_actor(actor: &crate::Active, round: u64, life: u32) -> Self {
        Self {
            round,
            allocation: actor.allocation_sequence,
            life,
            world: actor.reservation.world,
            reservation: actor.reservation.id.clone(),
            player: actor.reservation.player,
        }
    }
    pub(crate) fn current(&self, actor: &crate::Active, round: u64) -> bool {
        self.round == round
            && self.allocation == actor.allocation_sequence
            && self.world == actor.reservation.world
            && self.reservation == actor.reservation.id
            && self.player == actor.reservation.player
            && matches!(actor.life,crate::CharacterLife::Alive {generation,..} if generation==self.life)
    }
}
