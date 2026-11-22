//! Authored retail combat metadata, isolated from prototype gameplay behavior.
//!
//! Retail combat resolves exact disk-bound inventory IDs. Schema3 adds frag blast
//! and healing authority; schema4 adds slot-owned melee and facing rules.
//! Projectile motion remains authenticated owner-reported.

use crate::item_catalog::{BulletAttachmentKind, ItemCatalog, TargetDamageParameters};
use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetailCombatCapability {
    BulletDamage,
    GrenadeDamage,
    GrenadePhysics,
    ArmorReduction,
    BodyDamage,
    HealingSchedule,
    AmmoTokens,
}

#[derive(Debug)]
pub enum RetailCombatError {
    Io(std::io::Error),
    Json(serde_json::Error),
    InvalidMetadata(String),
    PendingAuthority(RetailCombatCapability),
    UnsupportedItem(u16),
}

impl fmt::Display for RetailCombatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "retail combat metadata I/O: {error}"),
            Self::Json(error) => write!(f, "retail combat metadata JSON: {error}"),
            Self::InvalidMetadata(error) => write!(f, "invalid retail combat metadata: {error}"),
            Self::UnsupportedItem(item) => write!(f, "unsupported retail combat item: {item}"),
            Self::PendingAuthority(capability) => {
                write!(f, "retail combat authority pending: {capability:?}")
            }
        }
    }
}

impl std::error::Error for RetailCombatError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Json(error) => Some(error),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RetailCombatAuthority {
    pub bullet_damage: bool,
    #[serde(default)]
    pub melee_damage: bool,
    pub grenade_damage: bool,
    pub grenade_physics: bool,
    pub armor_reduction: bool,
    pub body_damage: bool,
    pub healing_schedule: bool,
    pub ammo_tokens: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AuthoredDamage {
    pub character: f64,
    #[serde(rename = "characterAPierce")]
    pub character_armor_pierce: f64,
    #[serde(rename = "characterARedux")]
    pub character_armor_reduction: f64,
    pub vehicle: f64,
    #[serde(rename = "vehicleAPierce")]
    pub vehicle_armor_pierce: f64,
    #[serde(rename = "vehicleARedux")]
    pub vehicle_armor_reduction: f64,
    pub objective: f64,
    #[serde(rename = "objectiveAPierce")]
    pub objective_armor_pierce: f64,
    #[serde(rename = "objectiveARedux")]
    pub objective_armor_reduction: f64,
    pub mine: f64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetailAmmoMetadata {
    pub group: String,
    pub name: String,
    pub source: Value,
    /// An extraction candidate only; this is never a verified runtime token.
    pub ammo_token_candidate: u32,
    pub ammo_token_verified_for_retail: bool,
}

macro_rules! authored_profile {
    ($name:ident { $($(#[$attribute:meta])* $field:ident: $ty:ty),* $(,)? }) => {
        #[derive(Clone, Debug, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub struct $name {
            pub authored_ref: String,
            pub name: String,
            pub definition: Value,
            pub source: Value,
            pub item_source: Value,
            $($(#[$attribute])* pub $field: $ty,)*
        }
    };
}

authored_profile!(RetailBulletMetadata {
    #[serde(default)]
    usage: RetailBulletUsage,
    ammo: RetailAmmoMetadata,
    damage: AuthoredDamage,
    effective_range: f64,
    max_range: f64,
    magazine_capacity: u32,
    initial_magazine_count: u32,
    #[serde(default)]
    firearm: Option<RetailFirearmSchedule>,
});

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RetailBulletUsage {
    #[default]
    Character,
    HospitalPrimary,
    GroundApcPrimary,
    GroundEscortPrimary,
}

/// Original disk weapon schedule, separate from configurable AI policy.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetailFirearmSchedule {
    pub initial_fire_mode: RetailFireMode,
    pub refire_seconds: f64,
    pub reload_seconds: f64,
    pub infinite_ammo: bool,
}
#[derive(Clone, Copy, Debug, Deserialize)]
pub enum RetailFireMode {
    #[serde(rename = "kAuto")]
    Auto,
    #[serde(rename = "kSemiAuto")]
    SemiAuto,
}

authored_profile!(RetailMeleeMetadata {
    ammo: RetailAmmoMetadata,
    damage: AuthoredDamage,
    effective_range: f64,
    max_range: f64,
    gear_slot: u8,
    fire_mode: u8,
    category: String,
    damage_type: u8,
    max_angle: f64,
    max_height: f64,
    refire_seconds: f64,
});
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetailMeleeDirection {
    pub front_dot_threshold: f64,
    pub back_dot_threshold: f64,
    pub front_multiplier: f64,
    pub side_multiplier: f64,
    pub back_multiplier: f64,
}

authored_profile!(RetailGrenadeMetadata {
    ammo: RetailAmmoMetadata,
    damage: AuthoredDamage,
    full_radius: f64,
    half_radius: f64,
    fuse_ms: u64,
    authored_lifetime_ms: u64,
    initial_count: u32,
    effective_range: f64,
    max_range: f64,
    #[serde(default)]
    material_penetration: Option<f64>,
    detonation_strategy: String,
    explosion_strategy: String,
    physics: RetailGrenadePhysicsMetadata,
});

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetailGrenadePhysicsMetadata {
    pub launch_velocity: f64,
    pub radius: f64,
    #[serde(rename = "capsuleHeight")]
    pub capsule_height: [f64; 3],
    #[serde(rename = "collisionType")]
    pub collision_type: String,
    pub mass: f64,
    pub friction: f64,
    pub restitution: f64,
    #[serde(rename = "linearDamping")]
    pub linear_damping: f64,
    #[serde(rename = "linearDampingOnGround")]
    pub linear_damping_on_ground: f64,
    #[serde(rename = "angularDamping")]
    pub angular_damping: f64,
    #[serde(rename = "angularDampingOnGround")]
    pub angular_damping_on_ground: f64,
    #[serde(rename = "fBounceDampingDefault")]
    pub bounce_damping_default: f64,
    #[serde(rename = "fBounceDampingCharacter")]
    pub bounce_damping_character: f64,
    pub propulsion_strategy: String,
    #[serde(rename = "m_fLoftAngle")]
    pub loft_angle: f64,
    pub random_rotation_range: f64,
}

authored_profile!(RetailArmorMetadata {
    armor_type: u8,
    rating: f64,
    prevention: f64,
    coverage: [f64; 4],
    armor_type_source: Value,
});

/// Authored selection geometry only. This does not establish a native player
/// anchor, forward basis, recipient shape/query or autonomous healing authority.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetailHealingRecipientSphere {
    pub forward_offset: f64,
    pub radius: f64,
    pub source: RetailHealingSphereSource,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetailHealingSphereSource {
    pub file: String,
    pub pointer: String,
    pub fields: [String; 2],
    pub sha256: String,
}

authored_profile!(RetailHealingMetadata {
    cooldown_seconds: f64,
    heal_percent: f64,
    duration_seconds: f64,
    global_source: Value,
    #[serde(default)]
    recipient_sphere: Option<RetailHealingRecipientSphere>,
});

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetailBodyMetadata {
    pub location: u8,
    pub armor_area: u8,
    pub multiplier: f64,
    #[serde(default)]
    pub incapacitated_multiplier: Option<f64>,
    pub source: Value,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnsupportedRetailItemMetadata {
    pub name: String,
    pub authored_ref: String,
    pub reason: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetailRuntimePolicy {
    pub projectile_motion: String,
    pub grenade_token_basis: String,
    pub projectile_expiry_grace_ms: u64,
    pub healing_tick_policy: String,
    pub healing_tick_seconds: u8,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CatalogMetadata {
    schema_version: u32,
    application_id: u32,
    status: String,
    authority: RetailCombatAuthority,
    #[serde(default)]
    runtime_policy: Option<RetailRuntimePolicy>,
    provenance: Value,
    #[serde(deserialize_with = "item_map")]
    bullets: BTreeMap<u16, RetailBulletMetadata>,
    #[serde(default, deserialize_with = "item_map")]
    melee: BTreeMap<u16, RetailMeleeMetadata>,
    #[serde(default)]
    melee_direction: Option<RetailMeleeDirection>,
    #[serde(deserialize_with = "item_map")]
    grenades: BTreeMap<u16, RetailGrenadeMetadata>,
    #[serde(deserialize_with = "item_map")]
    armor: BTreeMap<u16, RetailArmorMetadata>,
    body: Vec<RetailBodyMetadata>,
    #[serde(deserialize_with = "item_map")]
    healing: BTreeMap<u16, RetailHealingMetadata>,
    #[serde(deserialize_with = "item_map")]
    unsupported: BTreeMap<u16, UnsupportedRetailItemMetadata>,
    limitations: Vec<String>,
}

/// A validated metadata document. No gameplay catalog or fallback is embedded.
#[derive(Debug)]
pub struct RetailCombatCatalog(CatalogMetadata);

impl RetailCombatCatalog {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, RetailCombatError> {
        let document = std::fs::read_to_string(path).map_err(RetailCombatError::Io)?;
        Self::from_json(&document)
    }

    pub fn from_json(document: &str) -> Result<Self, RetailCombatError> {
        let metadata: CatalogMetadata =
            serde_json::from_str(document).map_err(RetailCombatError::Json)?;
        validate(&metadata).map_err(RetailCombatError::InvalidMetadata)?;
        Ok(Self(metadata))
    }

    pub fn schema_version(&self) -> u32 {
        self.0.schema_version
    }
    pub fn application_id(&self) -> u32 {
        self.0.application_id
    }
    pub fn status(&self) -> &str {
        &self.0.status
    }
    pub fn authority(&self) -> &RetailCombatAuthority {
        &self.0.authority
    }
    pub fn provenance(&self) -> &Value {
        &self.0.provenance
    }
    pub fn limitations(&self) -> &[String] {
        &self.0.limitations
    }
    pub fn bullets(&self) -> &BTreeMap<u16, RetailBulletMetadata> {
        &self.0.bullets
    }
    pub fn grenades(&self) -> &BTreeMap<u16, RetailGrenadeMetadata> {
        &self.0.grenades
    }
    pub fn armor(&self) -> &BTreeMap<u16, RetailArmorMetadata> {
        &self.0.armor
    }
    pub fn healing(&self) -> &BTreeMap<u16, RetailHealingMetadata> {
        &self.0.healing
    }
    pub fn body(&self) -> &[RetailBodyMetadata] {
        &self.0.body
    }
    pub fn unsupported(&self) -> &BTreeMap<u16, UnsupportedRetailItemMetadata> {
        &self.0.unsupported
    }

    pub fn bullet_metadata(&self, item_id: u16) -> Option<&RetailBulletMetadata> {
        self.bullet_by_usage(item_id, RetailBulletUsage::Character)
    }
    fn bullet_by_usage(
        &self,
        item_id: u16,
        usage: RetailBulletUsage,
    ) -> Option<&RetailBulletMetadata> {
        self.0
            .bullets
            .get(&item_id)
            .filter(|bullet| bullet.usage == usage)
    }
    pub(crate) fn is_mounted_bullet(&self, item_id: u16) -> bool {
        self.0
            .bullets
            .get(&item_id)
            .is_some_and(|row| row.usage != RetailBulletUsage::Character)
    }
    pub fn melee(&self) -> &BTreeMap<u16, RetailMeleeMetadata> {
        &self.0.melee
    }
    pub fn melee_for_damage(&self, id: u16) -> Result<&RetailMeleeMetadata, RetailCombatError> {
        if !self.0.authority.melee_damage {
            return Err(RetailCombatError::UnsupportedItem(id));
        }
        self.0
            .melee
            .get(&id)
            .ok_or(RetailCombatError::UnsupportedItem(id))
    }
    pub fn melee_direction(&self) -> Result<&RetailMeleeDirection, RetailCombatError> {
        self.0.melee_direction.as_ref().ok_or_else(|| {
            RetailCombatError::InvalidMetadata("melee facing rules unavailable".into())
        })
    }
    pub fn grenade_metadata(&self, item_id: u16) -> Option<&RetailGrenadeMetadata> {
        self.0.grenades.get(&item_id)
    }
    pub fn armor_metadata(&self, item_id: u16) -> Option<&RetailArmorMetadata> {
        self.0.armor.get(&item_id)
    }
    pub fn healing_metadata(&self, item_id: u16) -> Option<&RetailHealingMetadata> {
        self.0.healing.get(&item_id)
    }
    pub fn body_metadata(&self, location: u8) -> Option<&RetailBodyMetadata> {
        self.0.body.iter().find(|entry| entry.location == location)
    }

    pub fn runtime_policy(&self) -> Option<&RetailRuntimePolicy> {
        self.0.runtime_policy.as_ref()
    }
    /// Expected token from original construction/resources, not an observed
    /// initialized runtime row or support for arbitrary token override maps.
    pub fn projectile_creation_token(&self, item_id: u16) -> Result<u32, RetailCombatError> {
        Ok(self.grenade_for_damage(item_id)?.ammo.ammo_token_candidate)
    }

    pub fn bullet_for_damage(
        &self,
        item_id: u16,
    ) -> Result<&RetailBulletMetadata, RetailCombatError> {
        self.bullet_metadata(item_id)
            .ok_or(RetailCombatError::UnsupportedItem(item_id))
    }
    pub(crate) fn hospital_primary_for_damage(
        &self,
        item_id: u16,
    ) -> Result<&RetailBulletMetadata, RetailCombatError> {
        self.bullet_by_usage(item_id, RetailBulletUsage::HospitalPrimary)
            .ok_or(RetailCombatError::UnsupportedItem(item_id))
    }
    pub(crate) fn ground_apc_primary_for_damage(
        &self,
        item_id: u16,
    ) -> Result<&RetailBulletMetadata, RetailCombatError> {
        self.bullet_by_usage(item_id, RetailBulletUsage::GroundApcPrimary)
            .ok_or(RetailCombatError::UnsupportedItem(item_id))
    }
    pub(crate) fn ground_escort_primary_for_damage(
        &self,
        item_id: u16,
    ) -> Result<&RetailBulletMetadata, RetailCombatError> {
        self.bullet_by_usage(item_id, RetailBulletUsage::GroundEscortPrimary)
            .ok_or(RetailCombatError::UnsupportedItem(item_id))
    }
    pub fn grenade_for_damage(
        &self,
        item_id: u16,
    ) -> Result<&RetailGrenadeMetadata, RetailCombatError> {
        if !self.0.authority.grenade_damage {
            return Err(RetailCombatError::PendingAuthority(
                RetailCombatCapability::GrenadeDamage,
            ));
        }
        self.0
            .grenades
            .get(&item_id)
            .ok_or(RetailCombatError::UnsupportedItem(item_id))
    }
    /// Qualify the ordinary on-foot frag path separately from legacy grenade damage.
    pub fn validate_frag_character_blast(&self) -> Result<(), RetailCombatError> {
        if !self.0.authority.grenade_damage {
            return Err(RetailCombatError::PendingAuthority(
                RetailCombatCapability::GrenadeDamage,
            ));
        }
        if self.0.grenades.is_empty() {
            return Err(RetailCombatError::InvalidMetadata(
                "frag Character blast requires supported grenade profiles".into(),
            ));
        }
        for (item_id, grenade) in &self.0.grenades {
            if !grenade
                .material_penetration
                .is_some_and(|value| (0.0..=f64::from(f32::MAX)).contains(&value))
            {
                return Err(RetailCombatError::InvalidMetadata(format!(
                    "frag Character blast requires valid authored material penetration for item {item_id}"
                )));
            }
        }
        Ok(())
    }

    pub fn grenade_for_physics(
        &self,
        _item_id: u16,
    ) -> Result<&RetailGrenadePhysicsMetadata, RetailCombatError> {
        Err(RetailCombatError::PendingAuthority(
            RetailCombatCapability::GrenadePhysics,
        ))
    }
    pub fn armor_for_reduction(
        &self,
        item_id: u16,
    ) -> Result<&RetailArmorMetadata, RetailCombatError> {
        self.0
            .armor
            .get(&item_id)
            .ok_or(RetailCombatError::UnsupportedItem(item_id))
    }
    pub fn body_for_damage(&self, location: u8) -> Result<&RetailBodyMetadata, RetailCombatError> {
        self.body_metadata(location)
            .ok_or_else(|| RetailCombatError::InvalidMetadata("invalid body location".into()))
    }
    pub fn healing_for_schedule(
        &self,
        item_id: u16,
    ) -> Result<&RetailHealingMetadata, RetailCombatError> {
        if !self.0.authority.healing_schedule {
            return Err(RetailCombatError::PendingAuthority(
                RetailCombatCapability::HealingSchedule,
            ));
        }
        self.0
            .healing
            .get(&item_id)
            .ok_or(RetailCombatError::UnsupportedItem(item_id))
    }
    pub fn verified_ammo_token(&self, _item_id: u16) -> Result<u32, RetailCombatError> {
        Err(RetailCombatError::PendingAuthority(
            RetailCombatCapability::AmmoTokens,
        ))
    }
}

/// Unlike an ordinary BTreeMap deserializer this refuses duplicate JSON keys,
/// leading zeroes, signs, whitespace, and IDs wider than the wire's 12 bits.
fn item_map<'de, D, T>(deserializer: D) -> Result<BTreeMap<u16, T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct Items<T>(std::marker::PhantomData<T>);
    impl<'de, T: Deserialize<'de>> Visitor<'de> for Items<T> {
        type Value = BTreeMap<u16, T>;
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("a map of unique canonical decimal u12 item IDs")
        }
        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
            let mut items = BTreeMap::new();
            while let Some(key) = map.next_key::<String>()? {
                let id = key
                    .parse::<u16>()
                    .map_err(|_| de::Error::custom("invalid item ID"))?;
                if id > 4095 || id.to_string() != key {
                    return Err(de::Error::custom("item ID must be canonical decimal u12"));
                }
                if items.contains_key(&id) {
                    return Err(de::Error::custom("duplicate item ID"));
                }
                items.insert(id, map.next_value()?);
            }
            Ok(items)
        }
    }
    deserializer.deserialize_map(Items(std::marker::PhantomData))
}

fn nonnegative(values: &[f64]) -> Result<(), String> {
    if values
        .iter()
        .any(|value| !value.is_finite() || *value < 0.0 || *value > f32::MAX as f64)
    {
        return Err("authored numeric values must be finite and nonnegative".into());
    }
    Ok(())
}

fn unit(values: &[f64]) -> Result<(), String> {
    nonnegative(values)?;
    if values.iter().any(|value| *value > 1.0) {
        return Err("authored fraction exceeds 1".into());
    }
    Ok(())
}

fn damage(value: &AuthoredDamage) -> Result<(), String> {
    nonnegative(&[
        value.character,
        value.character_armor_pierce,
        value.vehicle,
        value.vehicle_armor_pierce,
        value.objective,
        value.objective_armor_pierce,
        value.mine,
    ])?;
    unit(&[
        value.character_armor_reduction,
        value.vehicle_armor_reduction,
        value.objective_armor_reduction,
    ])
}

fn ranges(effective: f64, maximum: f64) -> Result<(), String> {
    nonnegative(&[effective, maximum])?;
    if effective > maximum {
        return Err("effective range exceeds maximum range".into());
    }
    Ok(())
}

fn ammo(value: &RetailAmmoMetadata, expected_group: &str) -> Result<(), String> {
    if value.ammo_token_verified_for_retail {
        return Err("ammo token verification requires pending retail authority".into());
    }
    if value.group != expected_group || value.name.is_empty() {
        return Err("invalid authored ammo identity".into());
    }
    Ok(())
}

fn validate(value: &CatalogMetadata) -> Result<(), String> {
    if ![2, 3, 4].contains(&value.schema_version) || value.application_id != 20231 {
        return Err("requires schema_version 2/3/4 and application_id 20231".into());
    }
    let authority = &value.authority;
    let extended = value.schema_version >= 3;
    if !authority.bullet_damage
        || !authority.armor_reduction
        || !authority.body_damage
        || authority.melee_damage != (value.schema_version == 4)
        || authority.grenade_damage != extended
        || authority.healing_schedule != extended
        || authority.grenade_physics
        || authority.ammo_tokens
    {
        return Err("unsupported retail authority selection".into());
    }
    if extended {
        let policy = value
            .runtime_policy
            .as_ref()
            .ok_or("schema3 requires runtime policy")?;
        if policy.projectile_motion != "authenticated_owner_reports"
            || policy.grenade_token_basis != "expected_static_original_resources"
            || policy.healing_tick_policy != "guaranteed_ticks_with_bounded_catch_up"
            || policy.healing_tick_seconds != 1
            || policy.projectile_expiry_grace_ms > 10_000
        {
            return Err("unsupported runtime policy".into());
        }
    } else if value.runtime_policy.is_some() {
        return Err("schema2 cannot activate runtime policy".into());
    }
    let mut item_ids = BTreeSet::new();
    let mut authored_refs = BTreeSet::new();
    macro_rules! identities {
        ($items:expr) => {
            for (id, profile) in $items {
                if !item_ids.insert(*id)
                    || profile.authored_ref.is_empty()
                    || !authored_refs.insert(&profile.authored_ref)
                    || profile.name.is_empty()
                {
                    return Err("duplicate item identity or empty authored identity".into());
                }
            }
        };
    }
    identities!(&value.bullets);
    identities!(&value.melee);
    identities!(&value.grenades);
    identities!(&value.armor);
    identities!(&value.healing);
    identities!(&value.unsupported);

    for bullet in value.bullets.values() {
        if let Some(schedule) = bullet.firearm {
            for seconds in [schedule.refire_seconds, schedule.reload_seconds] {
                if !seconds.is_finite() || seconds <= 0.0 || seconds > 60.0 {
                    return Err("invalid firearm schedule".into());
                }
            }
        }
        ammo(&bullet.ammo, "hitscan_ammo")?;
        damage(&bullet.damage)?;
        ranges(bullet.effective_range, bullet.max_range)?;
        if !(1..=255).contains(&bullet.magazine_capacity)
            || !(1..=255).contains(&bullet.initial_magazine_count)
        {
            return Err("bullet magazine capacity and initial count must be positive".into());
        }
    }
    if value.schema_version == 4 {
        if value.melee.is_empty() {
            return Err("schema4 requires melee profiles".into());
        }
        let d = value
            .melee_direction
            .as_ref()
            .ok_or("schema4 requires melee direction rules")?;
        if !d.front_dot_threshold.is_finite()
            || !d.back_dot_threshold.is_finite()
            || d.front_dot_threshold < -1.0
            || d.front_dot_threshold >= d.back_dot_threshold
            || d.back_dot_threshold > 1.0
        {
            return Err("invalid melee dot thresholds".into());
        }
        nonnegative(&[d.front_multiplier, d.side_multiplier, d.back_multiplier])?;
        for p in value.melee.values() {
            ammo(&p.ammo, "hitscan_ammo")?;
            damage(&p.damage)?;
            ranges(p.effective_range, p.max_range)?;
            nonnegative(&[p.max_angle, p.max_height, p.refire_seconds])?;
            if p.gear_slot != 2
                || p.fire_mode != 0
                || p.category != "kWepCategoryKnife"
                || p.damage_type != 3
                || p.max_range <= 0.0
            {
                return Err("unsupported melee profile".into());
            }
        }
    } else if !value.melee.is_empty() || value.melee_direction.is_some() {
        return Err("melee requires schema4".into());
    }
    for grenade in value.grenades.values() {
        ammo(&grenade.ammo, "projectile_ammo")?;
        damage(&grenade.damage)?;
        ranges(grenade.effective_range, grenade.max_range)?;
        nonnegative(&[grenade.full_radius, grenade.half_radius])?;
        if grenade.half_radius > grenade.full_radius || grenade.full_radius == 0.0 {
            return Err("invalid authored grenade radius order".into());
        }
        if grenade.fuse_ms == 0 || grenade.fuse_ms > grenade.authored_lifetime_ms {
            return Err(
                "grenade fuse must be positive and no longer than authored lifetime".into(),
            );
        }
        // This schema projects only the timer/debris frag; volume smoke has no
        // combat profile and cannot be relabeled as a supported grenade route.
        if grenade.detonation_strategy != "kWepDetonationStrategyTimer"
            || grenade.explosion_strategy != "kWepExplosionStrategyDebris"
        {
            return Err("unsupported grenade strategy in frag metadata".into());
        }
        let physics = &grenade.physics;
        nonnegative(&[
            physics.launch_velocity,
            physics.radius,
            physics.mass,
            physics.friction,
            physics.restitution,
            physics.linear_damping,
            physics.linear_damping_on_ground,
            physics.angular_damping,
            physics.angular_damping_on_ground,
            physics.bounce_damping_default,
            physics.bounce_damping_character,
            physics.loft_angle,
            physics.random_rotation_range,
        ])?;
        nonnegative(&physics.capsule_height)?;
        if physics.collision_type != "kWepProjCollisionShapeCapsule"
            || physics.propulsion_strategy != "kWepPropulsionStrategyThrown"
        {
            return Err("unsupported authored grenade physics strategy".into());
        }
    }
    for armor in value.armor.values() {
        if !(1..=6).contains(&armor.armor_type) {
            return Err("armor_type outside authored 1..=6 domain".into());
        }
        nonnegative(&[armor.rating])?;
        unit(&[armor.prevention])?;
        unit(&armor.coverage)?;
    }
    if value.body.len() != 16 {
        return Err("requires all 16 authored body locations".into());
    }
    let mut locations = BTreeSet::new();
    for body in &value.body {
        if body.location >= 16 || body.armor_area >= 4 || !locations.insert(body.location) {
            return Err("body locations must be unique 0..15 and armor areas 0..3".into());
        }
        nonnegative(&[body.multiplier])?;
        if let Some(multiplier) = body.incapacitated_multiplier {
            nonnegative(&[multiplier])?;
        }
    }
    for healing in value.healing.values() {
        nonnegative(&[
            healing.cooldown_seconds,
            healing.heal_percent,
            healing.duration_seconds,
        ])?;
        if extended
            && (healing.duration_seconds.fract() != 0.0
                || !(1.0..=255.0).contains(&healing.duration_seconds))
        {
            return Err(
                "healing duration must resolve to positive bounded one-second ticks".into(),
            );
        }
        if let Some(sphere) = &healing.recipient_sphere {
            nonnegative(&[sphere.forward_offset, sphere.radius])?;
            if healing.name != "medkit"
                || sphere.radius == 0.0
                || sphere.source.file != "active_items.lua"
                || sphere.source.pointer != "/parameters/ActiveItemDataTable"
                || sphere.source.fields != ["medkit_heal_range", "medkit_heal_radius"]
                || sphere.source.sha256.len() != 64
                || !sphere.source.sha256.bytes().all(|b| b.is_ascii_hexdigit())
            {
                return Err("invalid medkit recipient sphere metadata".into());
            }
        }
        if healing.heal_percent > 100.0 {
            return Err("heal_percent exceeds 100".into());
        }
    }
    Ok(())
}

/// Qualified retail hitscan ammunition from an attested equipped item.
#[derive(Clone, Copy, Debug)]
pub struct RetailBulletProfile {
    item_id: u16,
    amount: f32,
    pierce: f32,
    redux: f32,
    effective_range: f32,
    max_range: f32,
    vehicle_damage: TargetDamageParameters,
}
impl RetailBulletProfile {
    pub fn from_equipped(
        catalog: &RetailCombatCatalog,
        kit: &crate::replication::CharacterLoadout,
        equipped: crate::replication::InventoryItem,
        gear: u8,
        fire_mode: u8,
    ) -> Result<Self, &'static str> {
        Self::resolve_equipped(catalog, kit, equipped, gear, fire_mode, None)
    }

    pub(crate) fn from_equipped_with_items(
        items: &ItemCatalog,
        kit: &crate::replication::CharacterLoadout,
        equipped: crate::replication::InventoryItem,
        gear: u8,
        fire_mode: u8,
        on_foot: bool,
    ) -> Result<Self, &'static str> {
        Self::resolve_equipped(
            items.retail_combat()?,
            kit,
            equipped,
            gear,
            fire_mode,
            on_foot.then_some(items),
        )
    }

    fn resolve_equipped(
        catalog: &RetailCombatCatalog,
        kit: &crate::replication::CharacterLoadout,
        equipped: crate::replication::InventoryItem,
        gear: u8,
        fire_mode: u8,
        items: Option<&ItemCatalog>,
    ) -> Result<Self, &'static str> {
        if equipped.inventory_update_id != kit.handle.update_id {
            return Err("stale_equipped_inventory");
        }
        if equipped.gear_slot != gear {
            return Err("hit_gear_not_equipped");
        }
        if equipped.ammo_pool != 0 || (fire_mode != 0 && items.is_none()) {
            return Err("unsupported_bullet_magazine");
        }
        let attachments: &[usize] = match gear {
            0 => &[15, 16, 17],
            1 => &[19, 20],
            _ => return Err("unsupported_bullet_weapon"),
        };
        let inventory = kit
            .inventories
            .get(usize::from(kit.handle.inventory_index))
            .ok_or("unsupported_inventory_index")?;
        let weapon = inventory.slots[usize::from(gear)];
        if fire_mode != 0
            && !items
                .zip(weapon)
                .and_then(|(items, weapon)| items.retail_firearm_mode_count(weapon))
                .is_some_and(|count| usize::from(fire_mode) < count)
        {
            return Err("unsupported_retail_bullet_fire_mode");
        }
        for &slot in attachments {
            let Some(attachment) = inventory.slots[slot] else {
                continue;
            };
            // The launcher initializes underbarrel row 17. Ordinary gear 0
            // still selects host ammo, just as for non-replacement attachments.
            let kind = items.zip(weapon).and_then(|(items, weapon)| {
                items.retail_firearm_attachment(gear, weapon, attachment)
            });
            if !matches!(
                (gear, slot, kind),
                (0, 16, Some(BulletAttachmentKind::Suppressor))
                    | (1, 20, Some(BulletAttachmentKind::Suppressor))
                    | (
                        0,
                        17,
                        Some(
                            BulletAttachmentKind::Bipod
                                | BulletAttachmentKind::ForwardGrip
                                | BulletAttachmentKind::Launcher
                        )
                    )
            ) {
                return Err("unsupported_bullet_attachment");
            }
        }
        let item_id = weapon.ok_or("missing_bullet_weapon")?;
        let p = catalog
            .bullet_for_damage(item_id)
            .map_err(|_| "unsupported_retail_bullet_weapon")?;
        Ok(Self::from_metadata(item_id, p))
    }
    pub(crate) fn from_hospital_primary(
        catalog: &RetailCombatCatalog,
        qualified: crate::hospital_turrets::QualifiedPrimary,
        request: &crate::bullet_hit::NetMessageHit,
    ) -> Result<Self, &'static str> {
        let equipped = qualified.equipped;
        if equipped.inventory_update_id != qualified.definition_version {
            return Err("stale_hospital_turret_inventory");
        }
        if request.gear_index != 0 || equipped.gear_slot != 0 {
            return Err("unsupported_hospital_turret_gear");
        }
        if equipped.ammo_pool != 0 {
            return Err("unsupported_hospital_turret_ammo_pool");
        }
        if request.fire_mode != 0 {
            return Err("unsupported_hospital_turret_fire_mode");
        }
        let metadata = catalog
            .hospital_primary_for_damage(qualified.item_id)
            .map_err(|_| "unsupported_hospital_primary_bullet")?;
        Ok(Self::from_metadata(qualified.item_id, metadata))
    }
    pub(crate) fn from_ground_primary(
        catalog: &RetailCombatCatalog,
        qualified: crate::ground_turrets::GroundQualifiedPrimary,
        request: &crate::bullet_hit::NetMessageHit,
    ) -> Result<Self, &'static str> {
        let crate::ground_turrets::GroundQualifiedPrimary {
            item_id,
            equipped,
            definition_version,
            kind,
        } = qualified;
        if equipped.inventory_update_id != definition_version {
            return Err("stale_ground_turret_inventory");
        }
        if request.gear_index != 0 || equipped.gear_slot != 0 {
            return Err("unsupported_ground_turret_gear");
        }
        if equipped.ammo_pool != 0 {
            return Err("unsupported_ground_turret_ammo_pool");
        }
        if request.fire_mode != 0 {
            return Err("unsupported_ground_turret_fire_mode");
        }
        use crate::ground_turrets::PolicyKind;
        let metadata = match kind {
            PolicyKind::Apc => catalog
                .ground_apc_primary_for_damage(item_id)
                .map_err(|_| "unsupported_ground_apc_primary_bullet"),
            PolicyKind::Escort => catalog
                .ground_escort_primary_for_damage(item_id)
                .map_err(|_| "unsupported_ground_escort_primary_bullet"),
        }?;
        Ok(Self::from_metadata(item_id, metadata))
    }
    fn from_metadata(item_id: u16, p: &RetailBulletMetadata) -> Self {
        Self {
            item_id,
            amount: p.damage.character as f32,
            pierce: p.damage.character_armor_pierce as f32,
            redux: p.damage.character_armor_reduction as f32,
            effective_range: p.effective_range as f32,
            max_range: p.max_range as f32,
            vehicle_damage: TargetDamageParameters {
                amount: p.damage.vehicle as f32,
                armor_pierce: p.damage.vehicle_armor_pierce as f32,
                armor_redux: p.damage.vehicle_armor_reduction as f32,
            },
        }
    }
    pub fn item_id(self) -> u16 {
        self.item_id
    }
    pub(crate) fn vehicle_damage(self) -> TargetDamageParameters {
        self.vehicle_damage
    }
    pub fn raw_character_damage(
        self,
        distance: f32,
        scale: f32,
        location: u8,
    ) -> Result<crate::armor::RawCharacterDamage, &'static str> {
        if !distance.is_finite() || distance < 0.0 {
            return Err("invalid_bullet_distance");
        }
        if !scale.is_finite() || !(0.0..=1.0).contains(&scale) {
            return Err("invalid_bullet_scale");
        }
        if location > 15 {
            return Err("invalid_damage_location");
        }
        // Native HandleMessageHit: separately rounded reported scale and range
        // products; native range interpolation uses fdivs followed by fmadds.
        let range = if distance < self.effective_range {
            1.0
        } else if distance >= self.max_range {
            0.0
        } else {
            (-0.5_f32).mul_add(
                (distance - self.effective_range) / (self.max_range - self.effective_range),
                1.0,
            )
        };
        let source = self.amount * scale;
        Ok(crate::armor::RawCharacterDamage {
            amount_hp: source * range,
            armor_rating_redux: self.pierce,
            armor_prevention_redux: self.redux,
            hit_location: location,
        })
    }
    pub fn adjusted_amount(
        self,
        distance: f32,
        scale: f32,
        armor: crate::armor::ArmorProfile,
        location: u8,
    ) -> Result<f32, &'static str> {
        armor.adjust(self.raw_character_damage(distance, scale, location)?)
    }
}

#[cfg(test)]
mod incapacitated_body_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn explicit_incapacitated_body_multiplier_must_be_finite_and_nonnegative() {
        let value = json!({"schema_version":2,"application_id":20231,"status":"synthetic",
            "authority":{"bullet_damage":true,"armor_reduction":true,"body_damage":true,
                "grenade_damage":false,"grenade_physics":false,"healing_schedule":false,"ammo_tokens":false},
            "provenance":{},"bullets":{},"armor":{},"grenades":{},"healing":{},"unsupported":{},
            "body":(0..16).map(|location| json!({"location":location,"armor_area":0,"multiplier":2.0,
                "incapacitated_multiplier":0.75,"source":{}})).collect::<Vec<_>>(),"limitations":[]});
        let mut metadata: CatalogMetadata = serde_json::from_value(value).unwrap();
        assert!(validate(&metadata).is_ok());
        for invalid in [-1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            metadata.body[0].incapacitated_multiplier = Some(invalid);
            assert!(validate(&metadata).is_err());
        }
        metadata.body[0].incapacitated_multiplier = Some(0.0);
        assert!(validate(&metadata).is_ok());
    }
}

#[cfg(test)]
pub(crate) mod hospital_primary_tests {
    use super::*;
    use serde_json::json;

    pub(crate) fn combat_catalog() -> RetailCombatCatalog {
        let bullet = json!({
            "authored_ref":"Synthetic:2","name":"synthetic_primary","definition":{},"source":{},"item_source":{},
            "ammo":{"group":"hitscan_ammo","name":"synthetic_bullet","source":{},
                "ammo_token_candidate":77,"ammo_token_verified_for_retail":false},
            "damage":{"character":40.0,"characterAPierce":0.0,"characterARedux":0.0,"vehicle":50.0,
                "vehicleAPierce":0.0,"vehicleARedux":0.0,"objective":0.0,"objectiveAPierce":0.0,
                "objectiveARedux":0.0,"mine":0.0},
            "effective_range":20.0,"max_range":80.0,"magazine_capacity":9,"initial_magazine_count":2
        });
        let mut mounted_bullet = bullet.clone();
        mounted_bullet["authored_ref"] = json!("Synthetic:4");
        mounted_bullet["name"] = json!("synthetic_mounted_primary");
        mounted_bullet["usage"] = json!("hospital_primary");
        mounted_bullet["damage"]["vehicle"] = json!(12.);
        let mut apc_bullet = bullet.clone();
        apc_bullet["authored_ref"] = json!("Synthetic:5");
        apc_bullet["name"] = json!("synthetic_apc_primary");
        apc_bullet["usage"] = json!("ground_apc_primary");
        apc_bullet["damage"]["character"] = json!(48.);
        apc_bullet["damage"]["characterAPierce"] = json!(2.);
        apc_bullet["damage"]["characterARedux"] = json!(0.125);
        let mut escort_bullet = bullet.clone();
        escort_bullet["authored_ref"] = json!("Synthetic:6");
        escort_bullet["name"] = json!("synthetic_escort_primary");
        escort_bullet["usage"] = json!("ground_escort_primary");
        RetailCombatCatalog::from_json(&json!({
            "schema_version":2,"application_id":20231,"status":"synthetic",
            "authority":{"bullet_damage":true,"armor_reduction":true,"body_damage":true,
                "grenade_damage":false,"grenade_physics":false,"healing_schedule":false,"ammo_tokens":false},
            "provenance":{},"bullets":{"2":bullet,"4":mounted_bullet,"5":apc_bullet,"6":escort_bullet},"armor":{
                "3":{"authored_ref":"Synthetic:3","name":"synthetic_target_armor",
                    "definition":{},"source":{},"item_source":{},"armor_type":1,"rating":2.0,
                    "prevention":0.25,"coverage":[1.0,1.0,1.0,1.0],"armor_type_source":{}}
            },"grenades":{},
            "healing":{},"unsupported":{},"limitations":[],
            "body":(0..16).map(|location| json!({"location":location,"armor_area":0,"multiplier":1.0,
                "incapacitated_multiplier":1.0,"source":{}})).collect::<Vec<_>>()
        }).to_string()).unwrap()
    }

    #[test]
    fn ground_escort_catalog_cannot_be_used_as_ordinary_or_other_mounted_damage() {
        let catalog = combat_catalog();
        assert!(catalog.bullet_metadata(6).is_none());
        assert!(catalog.bullet_for_damage(6).is_err());
        assert!(catalog.hospital_primary_for_damage(6).is_err());
        assert!(catalog.ground_apc_primary_for_damage(6).is_err());
        assert!(catalog.is_mounted_bullet(6));
        assert!(catalog.ground_escort_primary_for_damage(6).is_ok());
        for item in [2, 4, 5] {
            assert!(catalog.ground_escort_primary_for_damage(item).is_err());
        }
    }

    #[test]
    fn ground_primary_requires_the_catalog_usage_of_the_current_role() {
        use crate::ground_turrets::{GroundQualifiedPrimary, PolicyKind};

        let catalog = combat_catalog();
        let hit = crate::bullet_hit::NetMessageHit::decode(
            &crate::human_kill_reward_tests::native_hit(0x0380_0000, 10),
        )
        .unwrap();
        let grant = |kind, item_id| GroundQualifiedPrimary {
            kind,
            item_id,
            equipped: crate::replication::InventoryItem {
                inventory_update_id: 3,
                gear_slot: 0,
                ammo_pool: 0,
                ammo_count: 9,
                reloading: false,
            },
            definition_version: 3,
        };
        let escort =
            RetailBulletProfile::from_ground_primary(&catalog, grant(PolicyKind::Escort, 6), &hit)
                .unwrap();
        assert_eq!(escort.item_id(), 6);
        assert_eq!(escort.vehicle_damage().amount, 50.);
        assert!(matches!(
            RetailBulletProfile::from_ground_primary(&catalog, grant(PolicyKind::Escort, 5), &hit),
            Err("unsupported_ground_escort_primary_bullet")
        ));
        assert!(matches!(
            RetailBulletProfile::from_ground_primary(&catalog, grant(PolicyKind::Apc, 6), &hit),
            Err("unsupported_ground_apc_primary_bullet")
        ));
    }

    #[test]
    fn ground_apc_primary_isolated_from_ordinary_and_hospital_rows() {
        let catalog = combat_catalog();
        assert!(catalog.bullet_for_damage(2).is_ok());
        assert!(catalog.hospital_primary_for_damage(4).is_ok());
        assert!(catalog.bullet_metadata(5).is_none());
        assert!(catalog.bullet_for_damage(5).is_err());
        assert!(catalog.hospital_primary_for_damage(5).is_err());
        for item in [2, 4, 6] {
            assert!(catalog.ground_apc_primary_for_damage(item).is_err());
        }
        assert!(catalog.ground_apc_primary_for_damage(5).is_ok());
    }

    #[test]
    fn ground_apc_primary_requires_matching_equipment_version_and_primary_slot() {
        let catalog = combat_catalog();
        let hit = crate::bullet_hit::NetMessageHit::decode(
            &crate::human_kill_reward_tests::native_hit(0x0380_0000, 10),
        )
        .unwrap();
        let equipped = crate::replication::InventoryItem {
            inventory_update_id: 2,
            gear_slot: 0,
            ammo_pool: 0,
            ammo_count: 9,
            reloading: false,
        };
        let from_grant = |item_id, equipped, definition_version, hit: &_| {
            let qualified = crate::ground_turrets::GroundQualifiedPrimary {
                item_id,
                equipped,
                definition_version,
                kind: crate::ground_turrets::PolicyKind::Apc,
            };
            RetailBulletProfile::from_ground_primary(&catalog, qualified, hit)
        };
        let raw = from_grant(5, equipped, 2, &hit)
            .unwrap()
            .raw_character_damage(1., 1., 3)
            .unwrap();
        assert_eq!(
            (
                raw.amount_hp,
                raw.armor_rating_redux,
                raw.armor_prevention_redux
            ),
            (48., 2., 0.125)
        );
        assert!(matches!(
            from_grant(4, equipped, 2, &hit),
            Err("unsupported_ground_apc_primary_bullet")
        ));
        assert!(matches!(
            from_grant(5, equipped, 3, &hit),
            Err("stale_ground_turret_inventory")
        ));
        assert!(matches!(
            from_grant(
                5,
                crate::replication::InventoryItem {
                    gear_slot: 1,
                    ..equipped
                },
                2,
                &hit
            ),
            Err("unsupported_ground_turret_gear")
        ));
        assert!(matches!(
            from_grant(
                5,
                crate::replication::InventoryItem {
                    ammo_pool: 1,
                    ..equipped
                },
                2,
                &hit
            ),
            Err("unsupported_ground_turret_ammo_pool")
        ));
        for secondary in [
            crate::bullet_hit::NetMessageHit {
                gear_index: 1,
                ..hit
            },
            crate::bullet_hit::NetMessageHit {
                fire_mode: 1,
                ..hit
            },
        ] {
            assert!(from_grant(5, equipped, 2, &secondary).is_err());
        }
    }

    #[test]
    fn mounted_primary_rows_are_unavailable_to_ordinary_damage_and_schedule() {
        let catalog = combat_catalog();
        assert!(catalog.bullet_metadata(2).is_some());
        assert!(catalog.bullet_for_damage(2).is_ok());
        assert!(catalog.bullet_metadata(4).is_none());
        assert!(matches!(
            catalog.bullet_for_damage(4),
            Err(RetailCombatError::UnsupportedItem(4))
        ));
        use crate::replication::{
            CharacterInventory, CharacterLoadout, InventoryHandle, InventoryItem,
        };
        let equipped = InventoryItem {
            inventory_update_id: 1,
            gear_slot: 0,
            ammo_pool: 0,
            ammo_count: 9,
            reloading: false,
        };
        let mut slots = [None; 22];
        slots[0] = Some(4);
        let mut loadout = CharacterLoadout {
            inventories: std::array::from_fn(|_| CharacterInventory {
                update_id: 1,
                slots,
            }),
            handle: InventoryHandle {
                update_id: 1,
                inventory_index: 0,
            },
            primary: equipped,
            primary_equipped: true,
        };
        for item in [4, 5, 6] {
            loadout.inventories[0].slots[0] = Some(item);
            assert!(matches!(
                RetailBulletProfile::from_equipped(&catalog, &loadout, equipped, 0, 0),
                Err("unsupported_retail_bullet_weapon")
            ));
        }
    }

    #[test]
    fn mounted_primary_cannot_be_saved_in_an_unselected_kit() {
        let mounted_items = [4, 5, 6].into_iter().map(|id| {
                    let reference = format!("FactionItemList0:{id}");
                    (reference.clone(), json!({
                        "authored_ref":reference,"faction_list":"FactionItemList0",
                        "local_ordinal":id,"wire_item_id":null,"name":"synthetic_mounted_primary",
                        "item_type":"kWeapon","definition":{"group":"weapons","name":"synthetic_mounted_primary"}
                    }))
                }).collect::<serde_json::Map<_,_>>();
        let items = crate::item_catalog::ItemCatalog::from_value_for_application(
            json!({
                "schema_version":2,"application_id":20231,
                "items": mounted_items,
                "definitions":{"weapons":{"synthetic_mounted_primary":{"parameters":{"magazine_capacity":9}}}},
                "resources":{}
            }),
            crate::blast_rules::CharacterBlastRules {
                armor_rating_reduction: 0.,
                armor_prevention_reduction: 0.,
                hit_location: 3,
                damage_type: 10,
            },
            20231,
        ).unwrap().with_retail_combat(combat_catalog());
        let mut saved = crate::loadouts::SavedLoadouts {
            selected_index: 0,
            update_ids: [1; 5],
            inventories: [[None; 22]; 5],
        };
        for item in [4, 5, 6] {
            saved.inventories[4][1] = Some(item);
            assert_eq!(
                items.validate_saved_item_bindings(&saved),
                Err("saved_mounted_bullet_weapon")
            );
        }
    }

    #[test]
    fn mounted_profile_requires_qualified_item_version_gear_pool_and_fire_mode() {
        use crate::hospital_turrets::QualifiedPrimary;

        let catalog = combat_catalog();
        let hit = crate::bullet_hit::NetMessageHit::decode(
            &crate::human_kill_reward_tests::native_hit(0x0380_0000, 10),
        )
        .unwrap();
        let qualified = || QualifiedPrimary {
            item_id: 4,
            equipped: crate::replication::InventoryItem {
                inventory_update_id: 2,
                gear_slot: 0,
                ammo_pool: 0,
                ammo_count: 9,
                reloading: false,
            },
            definition_version: 2,
        };
        let profile =
            RetailBulletProfile::from_hospital_primary(&catalog, qualified(), &hit).unwrap();
        assert_eq!(profile.item_id(), 4);
        assert_eq!(
            profile.raw_character_damage(1., 1., 3).unwrap().amount_hp,
            40.
        );

        for item_id in [2, 5] {
            let mut wrong = qualified();
            wrong.item_id = item_id;
            assert!(matches!(
                RetailBulletProfile::from_hospital_primary(&catalog, wrong, &hit),
                Err("unsupported_hospital_primary_bullet")
            ));
        }
        let mut stale = qualified();
        stale.equipped.inventory_update_id = 1;
        assert!(matches!(
            RetailBulletProfile::from_hospital_primary(&catalog, stale, &hit),
            Err("stale_hospital_turret_inventory")
        ));
        let mut wrong_slot = qualified();
        wrong_slot.equipped.gear_slot = 1;
        assert!(matches!(
            RetailBulletProfile::from_hospital_primary(&catalog, wrong_slot, &hit),
            Err("unsupported_hospital_turret_gear")
        ));
        let mut unbound = qualified();
        unbound.equipped.ammo_pool = 1;
        assert!(matches!(
            RetailBulletProfile::from_hospital_primary(&catalog, unbound, &hit),
            Err("unsupported_hospital_turret_ammo_pool")
        ));
        let mut secondary = hit;
        secondary.gear_index = 1;
        assert!(matches!(
            RetailBulletProfile::from_hospital_primary(&catalog, qualified(), &secondary),
            Err("unsupported_hospital_turret_gear")
        ));
        let mut wrong_mode = hit;
        wrong_mode.fire_mode = 1;
        assert!(matches!(
            RetailBulletProfile::from_hospital_primary(&catalog, qualified(), &wrong_mode),
            Err("unsupported_hospital_turret_fire_mode")
        ));
    }

    fn indexed_hospital_combat() -> crate::Engine<Vec<u8>> {
        use crate::hospital_turrets::tests::{
            enable_synthetic_hospital_turrets, equip_synthetic_hospital_turret,
        };
        let mut engine = crate::human_kill_reward_tests::fixture();
        let hospital = crate::structures::domination_bunker_seat_tests::fixture();
        engine.bunker_world = hospital.bunker_world;
        let items = std::mem::replace(&mut engine.item_catalog, hospital.item_catalog);
        let items = std::sync::Arc::try_unwrap(items).ok().unwrap();
        engine.item_catalog = std::sync::Arc::new(items.with_retail_combat(combat_catalog()));
        engine.match_started = true;
        engine
            .active
            .as_mut()
            .unwrap()
            .reservation
            .character
            .faction = 0;
        enable_synthetic_hospital_turrets(&mut engine);
        equip_synthetic_hospital_turret(&mut engine, 5);
        engine
    }

    #[test]
    fn ground_apc_primary_uses_armored_hostile_hit_transaction() {
        use crate::{
            CharacterLife,
            ground_turrets::tests::{equip_synthetic_ground_turret, opt_in},
            human_kill_reward_tests::{fixture, hit},
            progression::RewardKind,
            spawn_progression::tests::ground_vehicle_runtime::domination_ground_driver::driver_engine,
        };

        let victim_id = "synthetic-reservation-0000";
        let setup = || {
            let mut engine = fixture();
            let grounds = driver_engine(8);
            engine.ground_vehicles = grounds.ground_vehicles;
            engine.ground_runtime = grounds.ground_runtime;
            let vehicle = engine.current_ground_vehicle(8).unwrap();
            let batch_count = engine.ground_vehicles.as_ref().unwrap().batch_count();
            let catalog = engine.spawn_catalog.as_ref().unwrap();
            let for_team = |team| {
                let squad = catalog
                    .map_pool()
                    .squads
                    .iter()
                    .find(|s| s.team == team)
                    .unwrap();
                std::sync::Arc::new(catalog.for_squad(squad.index).unwrap())
            };
            let shooter = engine.active.as_mut().unwrap();
            shooter.squad_catalog = for_team(vehicle.team);
            shooter.reservation.character.faction = vehicle.faction;
            shooter.ground_vehicle_batch = batch_count;
            engine.peers.get_mut(victim_id).unwrap().squad_catalog = for_team(1 - vehicle.team);
            let items = std::mem::replace(&mut engine.item_catalog, grounds.item_catalog);
            engine.item_catalog = std::sync::Arc::new(
                std::sync::Arc::try_unwrap(items)
                    .ok()
                    .unwrap()
                    .with_retail_combat(combat_catalog()),
            );
            engine.match_started = true;
            opt_in(&mut engine, 8);
            equip_synthetic_ground_turret(&mut engine, 8);
            engine.peers.get_mut(victim_id).unwrap().life = CharacterLife::Alive {
                generation: 4,
                health_quarter_points: 200,
            };

            engine
        };
        let mut failed = setup();
        failed.max_log = failed.written;
        assert!(matches!(
            hit(&mut failed, 70, 1000),
            Err("log_limit_reached")
        ));
        assert_eq!(failed.peers[victim_id].life.damage_health(), Some((4, 200)));
        assert!(failed.pending_reward_batch().is_empty());

        let mut engine = setup();
        hit(&mut engine, 71, 1001).unwrap();
        let victim = &engine.peers[victim_id];
        assert_eq!(victim.life.damage_health(), Some((4, 32)));
        assert_eq!(victim.last_damage.last_attacker, 0x0380_0001);
        assert_eq!(victim.last_damage.last_source_object, u32::MAX);

        hit(&mut engine, 72, 1002).unwrap();
        assert!(matches!(
            engine.peers[victim_id].life,
            CharacterLife::Incapacitated { generation: 4, .. }
        ));
        assert_eq!(engine.peers[victim_id].reward_statistics.get(&12), Some(&1));
        assert_eq!(
            engine.active.as_ref().unwrap().reward_statistics.get(&10),
            Some(&1)
        );
        assert_eq!(
            engine
                .pending_reward_batch()
                .iter()
                .filter(|event| event.kind == RewardKind::EnemyKilled && event.player_id == 1)
                .count(),
            1
        );
    }

    #[test]
    fn mounted_hit_uses_current_indexed_authority_armor_and_hostile_kill_transaction() {
        use crate::{CharacterLife, human_kill_reward_tests::hit, progression::RewardKind};
        let mut engine = indexed_hospital_combat();
        let victim_id = "synthetic-reservation-0000";
        let replies = hit(&mut engine, 71, 1000).unwrap();
        assert!(
            !replies.is_empty(),
            "{}",
            String::from_utf8_lossy(&engine.writer)
                .lines()
                .last()
                .unwrap()
        );
        let victim = &engine.peers[victim_id];
        assert_eq!(victim.life.damage_health(), Some((4, 6))); // (40 - 2) * .75 = 28.5 HP.
        assert_eq!(victim.last_damage.last_attacker, 0x0380_0001);
        assert_eq!(victim.last_damage.last_source_object, u32::MAX);
        assert_eq!(victim.last_damage.last_user_data, 71);

        hit(&mut engine, 72, 1001).unwrap();
        let victim = &engine.peers[victim_id];
        assert!(matches!(
            victim.life,
            CharacterLife::Incapacitated { generation: 4, .. }
        ));
        assert_eq!(victim.reward_statistics.get(&12), Some(&1));
        assert_eq!(
            engine.active.as_ref().unwrap().reward_statistics.get(&10),
            Some(&1)
        );
        let rewards = engine.pending_reward_batch();
        assert_eq!(
            rewards
                .iter()
                .filter(|event| event.kind == RewardKind::EnemyKilled && event.player_id == 1)
                .count(),
            1
        );
    }

    #[test]
    fn mounted_hit_rejects_foreign_grant_and_rolls_back_when_recording_fails() {
        use crate::human_kill_reward_tests::hit;
        let mut engine = indexed_hospital_combat();
        let victim_id = "synthetic-reservation-0000";
        engine.active.as_mut().unwrap().mounted_bunker = Some(6);
        hit(&mut engine, 80, 1000).unwrap();
        assert_eq!(engine.peers[victim_id].life.damage_health(), Some((4, 120)));

        engine.active.as_mut().unwrap().mounted_bunker = Some(5);
        let packet = crate::human_kill_reward_tests::native_hit(0x0380_0000, 81);
        let request = crate::bullet_hit::NetMessageHit::decode(&packet).unwrap();
        assert!(
            engine
                .bullet_profile(engine.active.as_ref().unwrap(), &request)
                .is_ok()
        );
        let recorded = engine.writer.len();
        engine.max_log = engine.written;
        assert!(matches!(
            hit(&mut engine, 81, 1001),
            Err("log_limit_reached")
        ));
        assert_eq!(engine.writer.len(), recorded);
        assert_eq!(engine.peers[victim_id].life.damage_health(), Some((4, 120)));
        assert!(engine.pending_reward_batch().is_empty());
    }

    #[test]
    fn indexed_hospital_hit_requires_native_client_route_active_round_and_character() {
        use crate::{BRIDGE_VERSION, Direction, Request, Transport, protocol};

        let mut engine = indexed_hospital_combat();
        let packet = crate::human_kill_reward_tests::native_hit(0x0380_0000, 91);
        let mut target = crate::bullet_hit::NetMessageHit::decode(&packet).unwrap();
        for defender in [0x0100_0000, 0x0200_0000, 0x0d80_0006] {
            target.defender = defender;
            assert!(matches!(
                engine.bullet_profile(engine.active.as_ref().unwrap(), &target),
                Err("hospital_primary_requires_character_target")
            ));
        }
        assert!(serde_json::from_value::<Direction>(json!("server_to_client")).is_err());
        for (time, plugin_id, started, denial, expected_hp) in [
            (1000, Some(2), true, None, 120),
            (
                1001,
                Some(1),
                false,
                Some("hospital_primary_requires_active_round"),
                120,
            ),
            (1002, Some(1), true, None, 6),
        ] {
            engine.match_started = started;
            let actor = engine.active.as_ref().unwrap();
            let reply = engine.handle(
                Request::Event {
                    v: BRIDGE_VERSION,
                    request_id: format!("synthetic-denied-mounted-hit-{time}"),
                    reservation_id: actor.reservation.id.clone(),
                    world_id: actor.reservation.world,
                    player_id: actor.reservation.player,
                    transport: Transport::Tcp,
                    direction: Direction::ClientToServer,
                    rt_type: 0x3d,
                    plugin_id,
                    payload_hex: protocol::encode_hex(&packet),
                },
                std::time::Duration::from_millis(time),
                1,
            );
            assert!(reply.ok, "{:?}", reply.error);
            let record: Value = serde_json::from_slice(
                engine
                    .writer
                    .split(|&byte| byte == b'\n')
                    .rfind(|line| !line.is_empty())
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(record["gameplay_denial"].as_str(), denial);
            assert_eq!(
                engine.peers["synthetic-reservation-0000"]
                    .life
                    .damage_health(),
                Some((4, expected_hp))
            );
        }
    }
}

#[cfg(test)]
mod frag_character_metadata_tests {
    use super::*;

    #[test]
    fn legacy_frag_damage_remains_available_but_character_blast_requires_penetration() {
        let items = crate::human_kill_reward_tests::grenade_items_for_ground_test(22.);
        let catalog = items.retail_combat().unwrap();
        assert!(
            catalog
                .grenade_for_damage(4)
                .unwrap()
                .material_penetration
                .is_none()
        );
        assert!(matches!(
            catalog.validate_frag_character_blast(),
            Err(RetailCombatError::InvalidMetadata(_))
        ));
    }

    #[test]
    fn frag_character_blast_checks_every_enabled_profile_and_authority() {
        let items = crate::human_kill_reward_tests::grenade_items_for_ground_test(22.);
        let source = items.retail_combat().unwrap();
        let initial = &source.0;
        let mut metadata = CatalogMetadata {
            schema_version: initial.schema_version,
            application_id: initial.application_id,
            status: initial.status.clone(),
            authority: initial.authority.clone(),
            runtime_policy: initial.runtime_policy.clone(),
            provenance: initial.provenance.clone(),
            bullets: initial.bullets.clone(),
            melee: initial.melee.clone(),
            melee_direction: initial.melee_direction.clone(),
            grenades: initial.grenades.clone(),
            armor: initial.armor.clone(),
            body: initial.body.clone(),
            healing: initial.healing.clone(),
            unsupported: initial.unsupported.clone(),
            limitations: initial.limitations.clone(),
        };
        let mut second = metadata.grenades[&4].clone();
        second.material_penetration = Some(1.25);
        metadata.grenades.insert(5, second);
        let mut catalog = RetailCombatCatalog(metadata);
        for invalid in [None, Some(-1.), Some(f64::INFINITY), Some(f64::MAX)] {
            catalog.0.grenades.get_mut(&4).unwrap().material_penetration = invalid;
            assert!(
                catalog.validate_frag_character_blast().is_err(),
                "{invalid:?}"
            );
        }
        catalog.0.grenades.get_mut(&4).unwrap().material_penetration = Some(0.);
        assert!(catalog.validate_frag_character_blast().is_ok());
        catalog.0.grenades.clear();
        assert!(catalog.validate_frag_character_blast().is_err());
        catalog.0.authority.grenade_damage = false;
        assert!(matches!(
            catalog.validate_frag_character_blast(),
            Err(RetailCombatError::PendingAuthority(
                RetailCombatCapability::GrenadeDamage
            ))
        ));
    }
}
