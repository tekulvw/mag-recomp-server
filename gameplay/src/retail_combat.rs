//! Authored retail combat metadata, isolated from prototype gameplay behavior.
//!
//! Retail combat resolves exact disk-bound inventory IDs. Schema3 adds frag blast
//! and healing authority; schema4 adds slot-owned melee and facing rules.
//! Projectile motion remains authenticated owner-reported.

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
    ammo: RetailAmmoMetadata,
    damage: AuthoredDamage,
    effective_range: f64,
    max_range: f64,
    magazine_capacity: u32,
    initial_magazine_count: u32,
    #[serde(default)]
    firearm: Option<RetailFirearmSchedule>,
});

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
        self.0.bullets.get(&item_id)
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
        self.0
            .bullets
            .get(&item_id)
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

/// Ordinary retail weapon profile. Ammo identity comes from its attested item
/// definition; this path never accepts a wire ammo token or consumes ammunition.
#[derive(Clone, Copy, Debug)]
pub struct RetailBulletProfile {
    item_id: u16,
    amount: f32,
    pierce: f32,
    redux: f32,
    effective_range: f32,
    max_range: f32,
}
impl RetailBulletProfile {
    pub fn from_equipped(
        catalog: &RetailCombatCatalog,
        kit: &crate::replication::CharacterLoadout,
        equipped: crate::replication::InventoryItem,
        gear: u8,
        magazine: u8,
    ) -> Result<Self, &'static str> {
        if equipped.inventory_update_id != kit.handle.update_id {
            return Err("stale_equipped_inventory");
        }
        if equipped.gear_slot != gear {
            return Err("hit_gear_not_equipped");
        }
        if equipped.ammo_pool != 0 || magazine != equipped.ammo_pool {
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
        if attachments.iter().any(|i| inventory.slots[*i].is_some()) {
            return Err("unsupported_bullet_attachment");
        }
        let item_id = inventory.slots[usize::from(gear)].ok_or("missing_bullet_weapon")?;
        let p = catalog
            .bullet_for_damage(item_id)
            .map_err(|_| "unsupported_retail_bullet_weapon")?;
        Ok(Self {
            item_id,
            amount: p.damage.character as f32,
            pierce: p.damage.character_armor_pierce as f32,
            redux: p.damage.character_armor_reduction as f32,
            effective_range: p.effective_range as f32,
            max_range: p.max_range as f32,
        })
    }
    pub fn item_id(self) -> u16 {
        self.item_id
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
