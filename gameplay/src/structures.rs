//! Match-owned emplacement bodies. Placement and combat data load from disk;
//! request identity is bound to Horizon admission, never the repair wire.
use super::*;
use replication::{
    BitWriter, DefensiveEmplacementCreation, UpdateHeader, VehicleDamage, VehicleState,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    path::Path,
};
use structure_catalog::{StructureCatalog, StructureProfile};

#[derive(Clone, Copy, Debug)]
pub struct BunkerHealth {
    hp: f32,
    max: f32,
    destroyed: bool,
    rules: structure_catalog::NativeStructureRules,
    rebuild_percent: f32,
}
impl BunkerHealth {
    pub fn new(max: f32) -> Result<Self, &'static str> {
        if !max.is_finite() || !(1.0..=1023.75).contains(&max) {
            return Err("invalid_bunker_health");
        }
        Ok(Self {
            hp: max,
            max,
            destroyed: false,
            rules: structure_catalog::NativeStructureRules {
                light_percent: 70.0,
                heavy_percent: 40.0,
                critical_percent: 15.0,
                dead_health_floor: 1.0,
                min_rebuild_percent: 15.0,
            },
            rebuild_percent: 100.0,
        })
    }
    fn for_profile(
        profile: &StructureProfile,
        catalog: &StructureCatalog,
    ) -> Result<Self, &'static str> {
        let mut health = Self::new(profile.max_health)?;
        health.rules = catalog.rules();
        health.rebuild_percent = profile
            .repaired_health_percent
            .max(health.rules.min_rebuild_percent);
        Ok(health)
    }
    pub fn hp(self) -> f32 {
        self.hp
    }
    pub fn wire_health(self) -> u16 {
        (self.hp * 4.0).trunc() as u16
    }
    fn set(&mut self, hp: f32) {
        self.hp = if hp < self.rules.dead_health_floor {
            0.0
        } else {
            hp.min(self.max)
        };
    }
    pub fn damage(&mut self, amount: f32) -> Result<(), &'static str> {
        if !amount.is_finite() || amount < 0.0 {
            return Err("invalid_bunker_damage");
        }
        self.set((self.hp - amount).max(0.0));
        if self.hp == 0.0 {
            self.destroyed = true;
        }
        Ok(())
    }
    pub fn repair(&mut self, amount: f32) -> Result<(), &'static str> {
        if !amount.is_finite() || amount < 0.0 {
            return Err("invalid_bunker_repair");
        }
        self.set(self.hp + amount);
        if self.hp >= self.max * (self.rebuild_percent / 100.0) {
            self.destroyed = false;
        }
        Ok(())
    }
    pub fn conditions(self) -> u8 {
        // Native condition flags are independent health threshold bits.
        u8::from(self.hp < self.max * (self.rules.light_percent / 100.0))
            | (u8::from(self.hp < self.max * (self.rules.heavy_percent / 100.0)) << 1)
            | (u8::from(self.hp < self.max * (self.rules.critical_percent / 100.0)) << 2)
            | (u8::from(self.destroyed) << 3)
    }
}
#[derive(Deserialize)]
struct Catalog {
    format: String,
    schema_version: u8,
    vehicle_type_token: u32,
    maps: Vec<Map>,
    faction_definitions: Vec<Faction>,
}
#[derive(Deserialize)]
struct Faction {
    faction_id: u8,
    mount_points_for_comparison: Vec<Mount>,
}
#[derive(Deserialize)]
struct Mount {
    local_offset_xyz: [f32; 3],
}
#[derive(Deserialize)]
struct Map {
    name: String,
    runtime_indices_verified: bool,
    bunkers: Vec<Placement>,
}
#[derive(Deserialize)]
struct Placement {
    catalog_id: String,
    team: u8,
    active: bool,
    spawn_catalog_ordinal: u16,
    spawn_runtime_index: Option<u16>,
    calculated_vehicle_origin_xyz: [f32; 3],
    quaternion_xyzw: [f32; 4],
    vehicle_properties_source: Source,
}
#[derive(Deserialize)]
struct Source {
    properties_zip_member: String,
}
#[derive(Deserialize)]
struct HospitalAllocation {
    format: String,
    allocation_scope: String,
    fresh_family13_pool: bool,
    all_initial_class4_creations_succeed: bool,
    production_without_early_other_class4_creations: bool,
    maps: Vec<HospitalAllocationMap>,
}

#[derive(Deserialize)]
struct HospitalAllocationMap {
    map: String,
    objects: Vec<HospitalIdentity>,
}

#[derive(Deserialize)]
struct HospitalIdentity {
    object_index: u16,
    spawn_index: u16,
    spawn_token: String,
}

impl HospitalAllocation {
    fn validate(&self, placements: &HospitalPlacements) -> io::Result<()> {
        let mut maps = self
            .maps
            .iter()
            .filter(|map| map.map == placements.map_name);
        let map = maps
            .next()
            .ok_or_else(|| invalid("missing qualified hospital allocation"))?;
        if self.format != "mag_domination_hospital_allocation_v1"
            || self.allocation_scope != placements.allocation_scope
            || !self.fresh_family13_pool
            || !self.all_initial_class4_creations_succeed
            || !self.production_without_early_other_class4_creations
            || maps.next().is_some()
            || map.objects.len() != placements.placements.len()
        {
            return Err(invalid("hospital allocation scope mismatch"));
        }
        let mut observed = BTreeMap::new();
        for object in &map.objects {
            let token = object
                .spawn_token
                .strip_prefix("0x")
                .and_then(|value| u32::from_str_radix(value, 16).ok())
                .ok_or_else(|| invalid("invalid hospital allocation spawn token"))?;
            if observed
                .insert(object.object_index, (object.spawn_index, token))
                .is_some()
            {
                return Err(invalid("duplicate hospital allocation index"));
            }
        }
        if placements
            .placements
            .iter()
            .any(|row| observed.get(&row.object_index) != Some(&(row.spawn_index, row.spawn_token)))
        {
            return Err(invalid("hospital allocation identity mismatch"));
        }
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HospitalPlacements {
    format: String,
    map_name: String,
    spawn_catalog_sha256: String,
    structure_catalog_sha256: String,
    source_proof_sha256: String,
    passenger_pose_sha256: String,
    passenger_pose_scope: String,
    allocation_scope: String,
    vehicle_type_token: u32,
    placements: Vec<HospitalPlacement>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HospitalPlacement {
    object_index: u16,
    spawn_index: u16,
    spawn_token: u32,
    properties_source: String,
    mounts: Vec<HospitalMount>,
    max_occupancy: u8,
    passenger_seats: u8,
    exit: vehicle_exit::BunkerExitPolicy,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HospitalMount {
    local_offset: [f32; 3],
    radius: f32,
}

#[derive(Clone)]
struct HospitalAccess {
    mount_radii: Vec<f32>,
    max_occupancy: u8,
    passenger_seats: u8,
    /// Domination hospitals take direct vehicle damage and decay. An
    /// Acquisition field hospital follows only its structural objective's
    /// forward spawn, so direct hits never change its body.
    direct_damage: bool,
}

pub struct IndexedHospital<'a> {
    pub index: u16,
    pub spawn_index: u16,
    pub team: u8,
    pub faction: u8,
    pub(crate) properties_source: &'a str,
    pub(crate) turret_qualified: bool,
    pub operational: bool,
    pub position: replication::CharacterPosition,
    pub max_occupancy: u8,
    pub passenger_seats: u8,
    pub exit_policy: &'a vehicle_exit::BunkerExitPolicy,
    mount_points: &'a [[f32; 3]],
    mount_radii: &'a [f32],
}

impl IndexedHospital<'_> {
    /// Original hospital passenger seats use the vehicle transform directly
    /// because the shared empty node name never resolves in these models.
    pub fn seat_position(&self, seat: u8) -> Option<replication::CharacterPosition> {
        ((seat == 0 && self.turret_qualified) || (1..=self.passenger_seats).contains(&seat))
            .then_some(self.position)
    }

    /// Retail hulk placement keeps the body origin and rotates around world Y.
    pub fn hulk_transform(&self, turret_yaw_rad: f32) -> Option<vehicle_exit::BunkerTransform> {
        if !turret_yaw_rad.is_finite() {
            return None;
        }
        let [x, y, z, w] = self.exit_policy.transform.rotation_xyzw;
        let forward_x = 2.0 * (x * z + w * y);
        let forward_z = 1.0 - 2.0 * (x * x + y * y);
        let body_yaw = forward_x.atan2(forward_z);
        let (sin, cos) = ((body_yaw + turret_yaw_rad) * 0.5).sin_cos();
        Some(vehicle_exit::BunkerTransform {
            position: self.position,
            rotation_xyzw: [0.0, sin, 0.0, cos],
        })
    }

    pub fn within_mount_range(&self, position: replication::CharacterPosition) -> bool {
        self.mount_points
            .iter()
            .zip(self.mount_radii)
            .any(|(at, radius)| {
                let delta = [position.x - at[0], position.y - at[1], position.z - at[2]];
                delta.iter().map(|d| d * d).sum::<f32>() <= radius * radius
            })
    }
}

impl HospitalPlacements {
    fn validate(&self, spawns: &spawn_catalog::SpawnCatalog) -> io::Result<()> {
        let policy = spawns.match_profile();
        if self.format != "mag_domination_hospital_placements_v1"
            || self.allocation_scope != "successful_production_fresh_level_load"
            || self.passenger_pose_scope != "retail_unresolved_passenger_node_zero_local_pose"
            || self.passenger_pose_sha256.len() != 64
            || !self
                .passenger_pose_sha256
                .as_bytes()
                .iter()
                .all(u8::is_ascii_hexdigit)
            || self.map_name != spawns.map_name()
            || spawns.source_sha256() != Some(self.spawn_catalog_sha256.as_str())
            || !matches!(policy.game_type, 1 | 2)
            || !policy.auto_assign_teams
            || !policy.has_bunkers
            || self.vehicle_type_token == 0
            || self.placements.is_empty()
            || (policy.game_type == 2 && self.placements.len() != 16)
        {
            return Err(invalid("invalid hospital source binding"));
        }
        let mut indices = BTreeSet::new();
        let mut spawn_indices = BTreeSet::new();
        for placement in &self.placements {
            if placement.object_index >= 16
                || !indices.insert(placement.object_index)
                || !spawn_indices.insert(placement.spawn_index)
                || !spawns.point(placement.spawn_index).is_some_and(|point| {
                    point.token == placement.spawn_token
                        && point.active
                        && matches!(point.kind, spawn_catalog::SpawnKind::BunkerPassenger)
                })
                || placement.mounts.is_empty()
                || placement.mounts.len() > 32
                || placement.max_occupancy == 0
                || placement.passenger_seats == 0
                || placement.passenger_seats > 32
                || placement.max_occupancy > placement.passenger_seats + 1
                || placement.mounts.iter().any(|mount| {
                    !mount.radius.is_finite()
                        || !(0.0..64.0).contains(&mount.radius)
                        || mount.radius == 0.0
                        || mount
                            .local_offset
                            .iter()
                            .any(|v| !v.is_finite() || v.abs() >= 2048.)
                })
            {
                return Err(invalid("invalid hospital placement"));
            }
            placement.exit.validate().map_err(invalid)?;
        }
        if spawns
            .map_pool()
            .locations
            .iter()
            .filter(|location| matches!(location.kind, spawn_catalog::SpawnKind::BunkerPassenger))
            .any(|location| !spawn_indices.contains(&location.index))
        {
            return Err(invalid("incomplete hospital coverage"));
        }
        Ok(())
    }
}

#[derive(Clone)]
struct Bunker {
    id: String,
    spawn: u16,
    faction: u8,
    team: u8,
    last_health_update: Duration,
    repair_points: Vec<[f32; 3]>,
    profile: StructureProfile,
    health: BunkerHealth,
    damage: VehicleDamage,
    exit: Option<vehicle_exit::BunkerExitPolicy>,
    hospital: Option<HospitalAccess>,
    vehicle_state: VehicleState,
    turret_destruction_started: bool,
}

impl Bunker {
    fn domination_damageable(&self) -> bool {
        self.hospital
            .as_ref()
            .is_some_and(|access| access.direct_damage)
            && self.vehicle_state == VehicleState::Operational
            && !self.health.destroyed
            && self.health.conditions() & 4 == 0
    }

    fn apply_vehicle_damage(
        &mut self,
        amount: f32,
        attacker: u32,
        location_and_type: u8,
        user_data: u32,
        now: Duration,
    ) -> Result<Option<VehicleDamage>, &'static str> {
        if amount == 0. {
            return Ok(None);
        }
        self.health.damage(amount)?;
        self.last_health_update = now;
        self.damage = VehicleDamage {
            last_location_and_type: location_and_type,
            health_quarter_points: self.health.wire_health(),
            conditions: self.health.conditions(),
            last_attacker: attacker,
            last_source_object: u32::MAX,
            last_user_data: user_data,
        };
        Ok(Some(self.damage))
    }
}

#[derive(Clone)]
pub struct BunkerWorld {
    catalog: StructureCatalog,
    bunkers: BTreeMap<u16, Bunker>,
    vehicle_token: u32,
    hospital_spawn_sha256: Option<String>,
    hospital_structure_sha256: Option<String>,
    pub(crate) hospital_turrets: BTreeMap<u16, crate::hospital_turrets::Runtime>,
    pending_placements: usize,
    commands: BTreeMap<String, (u16, f32)>,
}

#[derive(Default)]
pub(crate) struct HospitalDamagePatch {
    changes: Vec<(u16, HospitalDamageState)>,
}

pub(crate) struct HospitalBlastDose {
    pub(crate) damage: item_catalog::TargetDamageParameters,
    pub(crate) radial_scale: f32,
    pub(crate) source_team: u8,
    pub(crate) attacker: u32,
    pub(crate) location: u8,
    pub(crate) user_data: u32,
}

pub(crate) struct HospitalBlastHit {
    pub(crate) amount_hp: f32,
    pub(crate) before_quarters: u16,
    pub(crate) after_quarters: u16,
}

struct HospitalDamageState {
    health: BunkerHealth,
    damage: VehicleDamage,
    last_health_update: Duration,
}

impl HospitalDamagePatch {
    pub(crate) fn install(self, world: &mut BunkerWorld) {
        for (index, state) in self.changes {
            let bunker = world
                .bunkers
                .get_mut(&index)
                .expect("prepared hospital damage target");
            bunker.health = state.health;
            bunker.damage = state.damage;
            bunker.last_health_update = state.last_health_update;
        }
    }

    pub(crate) fn encode(
        &self,
        now: Duration,
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Option<Vec<u8>>, &'static str> {
        if self.changes.is_empty() {
            return Ok(None);
        }
        let deltas: Vec<_> = self
            .changes
            .iter()
            .map(|(index, state)| BunkerDelta {
                index: *index,
                damage: state.damage,
            })
            .collect();
        BunkerDelta::encode_list_with_profile(&deltas, 1, now.as_millis() as u32, profile).map(Some)
    }
}
fn invalid(e: impl ToString) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, e.to_string())
}
impl BunkerWorld {
    #[cfg(test)]
    pub(crate) fn set_synthetic_hospital_transform(
        &mut self,
        index: u16,
        transform: vehicle_exit::BunkerTransform,
    ) {
        self.bunkers
            .get_mut(&index)
            .unwrap()
            .exit
            .as_mut()
            .unwrap()
            .transform = transform;
    }

    pub fn load(
        placements: impl AsRef<Path>,
        structures: impl AsRef<Path>,
        map_name: &str,
    ) -> io::Result<Self> {
        let raw: Catalog =
            serde_json::from_reader(std::fs::File::open(placements)?).map_err(invalid)?;
        if raw.format != "mag_bunker_catalog" || raw.schema_version != 1 {
            return Err(invalid("invalid bunker catalog"));
        }
        let vehicle_token = raw.vehicle_type_token;
        let catalog = StructureCatalog::load(structures)?;
        let map = raw
            .maps
            .into_iter()
            .find(|m| m.name == map_name)
            .ok_or_else(|| invalid("missing bunker map"))?;
        let mut bunkers = BTreeMap::new();
        let mut spawns = BTreeSet::new();
        let mut pending_placements = 0;
        for p in map.bunkers.into_iter().filter(|p| p.active) {
            let Some(spawn) = p.spawn_runtime_index.or(map
                .runtime_indices_verified
                .then_some(p.spawn_catalog_ordinal))
            else {
                pending_placements += 1;
                continue;
            };
            if spawn >= 128
                || !spawns.insert(spawn)
                || p.team > 1
                || bunkers.len() >= 16
                || p.calculated_vehicle_origin_xyz
                    .iter()
                    .chain(&p.quaternion_xyzw)
                    .any(|x| !x.is_finite())
            {
                return Err(invalid("invalid bunker placement"));
            }
            let source = p
                .vehicle_properties_source
                .properties_zip_member
                .split("/scripts/")
                .nth(1)
                .ok_or_else(|| invalid("invalid property path"))?;
            let profile = catalog.profile_for_source(source).map_err(invalid)?.clone();
            let mounts = &raw
                .faction_definitions
                .iter()
                .find(|f| f.faction_id == profile.faction)
                .ok_or_else(|| invalid("missing faction mounts"))?
                .mount_points_for_comparison;
            let transform = vehicle_exit::BunkerTransform {
                position: replication::CharacterPosition {
                    x: p.calculated_vehicle_origin_xyz[0],
                    y: p.calculated_vehicle_origin_xyz[1],
                    z: p.calculated_vehicle_origin_xyz[2],
                },
                rotation_xyzw: p.quaternion_xyzw,
            };
            let repair_points = mounts
                .iter()
                .map(|m| {
                    let d = transform.rotate(m.local_offset_xyz);
                    [
                        transform.position.x + d[0],
                        transform.position.y + d[1],
                        transform.position.z + d[2],
                    ]
                })
                .collect();
            let health = BunkerHealth::for_profile(&profile, &catalog).map_err(invalid)?;
            bunkers.insert(
                bunkers.len() as u16,
                Bunker {
                    id: p.catalog_id,
                    spawn,
                    faction: profile.faction,
                    team: p.team,
                    last_health_update: Duration::ZERO,
                    repair_points,
                    profile,
                    health,
                    damage: VehicleDamage {
                        last_location_and_type: 0,
                        health_quarter_points: health.wire_health(),
                        conditions: 0,
                        last_attacker: u32::MAX,
                        last_source_object: u32::MAX,
                        last_user_data: 0,
                    },
                    exit: None,
                    hospital: None,
                    vehicle_state: VehicleState::Operational,
                    turret_destruction_started: false,
                },
            );
        }
        if bunkers.is_empty() {
            return Err(invalid("no verified active bunker placements"));
        }
        Ok(Self {
            catalog,
            bunkers,
            vehicle_token,
            hospital_spawn_sha256: None,
            hospital_structure_sha256: None,
            hospital_turrets: Default::default(),
            pending_placements,
            commands: Default::default(),
        })
    }
    /// Load independently qualified hospital placements for this exact spawn
    /// profile: a Domination or Acquisition profile whose defenders' field
    /// hospitals (spawn type 8) carry a mountable defensive emplacement.
    pub fn load_hospitals(
        placements: impl AsRef<Path>,
        structures: impl AsRef<Path>,
        source_proof: impl AsRef<Path>,
        spawns: &spawn_catalog::SpawnCatalog,
    ) -> io::Result<Self> {
        let raw: HospitalPlacements =
            serde_json::from_reader(std::fs::File::open(placements)?).map_err(invalid)?;
        raw.validate(spawns)?;
        let structures = std::fs::read(structures)?;
        let source = std::fs::read(source_proof)?;
        if raw.structure_catalog_sha256 != format!("{:x}", Sha256::digest(&structures))
            || raw.source_proof_sha256 != format!("{:x}", Sha256::digest(&source))
        {
            return Err(invalid("hospital prerequisite digest mismatch"));
        }
        let allocation: HospitalAllocation = serde_json::from_slice(&source)?;
        allocation.validate(&raw)?;
        let catalog = StructureCatalog::from_value_for_application(
            serde_json::from_slice(&structures)?,
            20231,
        )?;
        let faction = spawns.match_profile().defending_faction;
        let direct_damage = spawns.match_profile().game_type == 2;
        let mut bunkers = BTreeMap::new();
        for placement in raw.placements {
            let profile = catalog
                .profile_for_source(&placement.properties_source)
                .map_err(invalid)?
                .clone();
            if profile.faction != faction {
                return Err(invalid("wrong hospital faction"));
            }
            let health = BunkerHealth::for_profile(&profile, &catalog).map_err(invalid)?;
            let repair_points = placement
                .mounts
                .iter()
                .map(|mount| {
                    let delta = placement.exit.transform.rotate(mount.local_offset);
                    let position = placement.exit.transform.position;
                    [
                        position.x + delta[0],
                        position.y + delta[1],
                        position.z + delta[2],
                    ]
                })
                .collect();
            bunkers.insert(
                placement.object_index,
                Bunker {
                    id: format!("{:08x}", placement.spawn_token),
                    spawn: placement.spawn_index,
                    faction,
                    team: 1,
                    last_health_update: Duration::ZERO,
                    repair_points,
                    profile,
                    health,
                    damage: VehicleDamage {
                        last_location_and_type: 0,
                        health_quarter_points: health.wire_health(),
                        conditions: 0,
                        last_attacker: u32::MAX,
                        last_source_object: u32::MAX,
                        last_user_data: 0,
                    },
                    exit: Some(placement.exit),
                    hospital: Some(HospitalAccess {
                        mount_radii: placement.mounts.iter().map(|mount| mount.radius).collect(),
                        max_occupancy: placement.max_occupancy,
                        passenger_seats: placement.passenger_seats,
                        direct_damage,
                    }),
                    vehicle_state: VehicleState::Operational,
                    turret_destruction_started: false,
                },
            );
        }
        Ok(Self {
            catalog,
            bunkers,
            vehicle_token: raw.vehicle_type_token,
            hospital_spawn_sha256: Some(raw.spawn_catalog_sha256),
            hospital_structure_sha256: Some(raw.structure_catalog_sha256),
            hospital_turrets: Default::default(),
            pending_placements: 0,
            commands: BTreeMap::new(),
        })
    }
    pub(crate) fn validate_hospital_profile(
        &self,
        spawns: &spawn_catalog::SpawnCatalog,
    ) -> Result<(), &'static str> {
        if self.hospital_spawn_sha256.as_deref().is_none()
            || self.hospital_spawn_sha256.as_deref() != spawns.source_sha256()
        {
            return Err("bunker_world_hospital_profile_mismatch");
        }
        Ok(())
    }

    /// True when this world holds source-bound hospitals addressed by their
    /// own family-13 index, rather than the legacy single Alpha bunker.
    pub(crate) fn has_indexed_hospitals(&self) -> bool {
        self.bunkers
            .values()
            .any(|bunker| bunker.hospital.is_some())
    }

    /// The per-emplacement exit is addressed by the family-13 pool index.
    pub fn exit_policy(&self, index: u16) -> Option<&vehicle_exit::BunkerExitPolicy> {
        self.bunkers.get(&index)?.exit.as_ref()
    }

    pub(crate) fn vehicle_type_token(&self) -> u32 {
        self.vehicle_token
    }

    pub(crate) fn hospital_structure_sha256(&self) -> Option<&str> {
        self.hospital_structure_sha256.as_deref()
    }
    pub(crate) fn repair_tool_profile(&self) -> crate::structure_catalog::RepairParameters {
        self.catalog.repair()
    }

    pub(crate) fn damage_catalog(&self) -> &StructureCatalog {
        &self.catalog
    }

    /// Apply the inherited vehicle damage model while keeping the vehicle state
    /// and hulk transition for the following family-13 update pass.
    pub(crate) fn apply_domination_hospital_bullet(
        &mut self,
        index: u16,
        amount: f32,
        attacker: u32,
        request: &bullet_hit::NetMessageHit,
        now: Duration,
    ) -> Result<Option<HospitalUpdate>, &'static str> {
        let bunker = self
            .bunkers
            .get_mut(&index)
            .filter(|bunker| bunker.hospital.is_some())
            .ok_or("unknown_domination_hospital_bullet_target")?;
        if !bunker.domination_damageable() {
            return Err("domination_hospital_bullet_target_not_damageable");
        }
        if request.hit_location > 15 {
            return Err("invalid_domination_hospital_hit_location");
        }
        Ok(bunker
            .apply_vehicle_damage(
                amount,
                attacker,
                0x10 | request.hit_location,
                request.client_data,
                now,
            )?
            .map(|damage| HospitalUpdate {
                object_index: index,
                state: None,
                damage: Some(damage),
                kills_occupants: false,
                destroys_turret: false,
                restores_turret: false,
            }))
    }

    pub(crate) fn hospital_damageable(&self, index: u16) -> bool {
        self.bunkers
            .get(&index)
            .is_some_and(Bunker::domination_damageable)
    }

    pub(crate) fn hospitals(&self) -> impl Iterator<Item = IndexedHospital<'_>> {
        self.bunkers
            .keys()
            .filter_map(|&index| self.hospital(index))
    }

    pub(crate) fn damageable_domination_hospitals(
        &self,
    ) -> impl Iterator<Item = IndexedHospital<'_>> {
        self.bunkers.iter().filter_map(|(&index, bunker)| {
            bunker
                .domination_damageable()
                .then(|| self.hospital(index))
                .flatten()
        })
    }

    pub(crate) fn apply_classified_hospital_blast(
        &mut self,
        index: u16,
        dose: HospitalBlastDose,
        now: Duration,
    ) -> Result<Option<HospitalBlastHit>, &'static str> {
        let bunker = self
            .bunkers
            .get_mut(&index)
            .filter(|bunker| bunker.hospital.is_some())
            .ok_or("unknown_domination_hospital_blast_target")?;
        if !bunker.domination_damageable() {
            return Ok(None);
        }
        if !(1..=6).contains(&dose.location) {
            return Err("invalid_domination_hospital_hit_location");
        }
        let friendly_factor = if bunker.team == dose.source_team {
            self.catalog.friendly_fire_multiplier()
        } else {
            1.
        };
        let amount_hp = item_catalog::apply_vehicle_armor(
            dose.damage,
            dose.radial_scale,
            friendly_factor,
            self.catalog.armor(bunker.profile.armor_type)?,
            self.catalog.body(dose.location)?,
        )?;
        let before_quarters = bunker.health.wire_health();
        Ok(bunker
            .apply_vehicle_damage(
                amount_hp,
                dose.attacker,
                0xa0 | dose.location,
                dose.user_data,
                now,
            )?
            .map(|damage| HospitalBlastHit {
                amount_hp,
                before_quarters,
                after_quarters: damage.health_quarter_points,
            }))
    }

    pub(crate) fn hospital_damage_patch(
        &self,
        changed: &BTreeSet<u16>,
    ) -> Result<HospitalDamagePatch, &'static str> {
        let changes = changed
            .iter()
            .map(|index| {
                let bunker = self
                    .bunkers
                    .get(index)
                    .ok_or("unknown_domination_hospital_blast_target")?;
                Ok((
                    *index,
                    HospitalDamageState {
                        health: bunker.health,
                        damage: bunker.damage,
                        last_health_update: bunker.last_health_update,
                    },
                ))
            })
            .collect::<Result<_, _>>()?;
        Ok(HospitalDamagePatch { changes })
    }

    /// Family-13 Update advances the damage clock, then reconciles the
    /// defensive-emplacement body against the damage model's dead bit.
    pub(crate) fn advance_domination_hospital_damage(
        &mut self,
        now: Duration,
    ) -> Result<Vec<(HospitalUpdate, u32)>, &'static str> {
        let mut changes = Vec::new();
        let mut deaths = Vec::new();
        for (&index, bunker) in &mut self.bunkers {
            if !bunker
                .hospital
                .as_ref()
                .is_some_and(|access| access.direct_damage)
                || bunker.vehicle_state != VehicleState::Operational
            {
                continue;
            }
            let previous = bunker.damage;
            if !bunker.health.destroyed && bunker.health.conditions() & 4 != 0 {
                let dt = now.saturating_sub(bunker.last_health_update).as_secs_f32();
                let per_second = (bunker.profile.critical_loss_percent_per_second * 0.01)
                    * bunker.profile.max_health;
                bunker.health.damage(per_second * dt)?;
                bunker.last_health_update = now;
                bunker.damage.health_quarter_points = bunker.health.wire_health();
                bunker.damage.conditions = bunker.health.conditions();
            }
            if bunker.health.destroyed {
                deaths.push((bunker.spawn, bunker.damage.last_attacker));
            } else if previous.health_quarter_points != bunker.damage.health_quarter_points
                || previous.conditions != bunker.damage.conditions
            {
                changes.push((
                    HospitalUpdate {
                        object_index: index,
                        state: None,
                        damage: Some(bunker.damage),
                        kills_occupants: false,
                        destroys_turret: false,
                        restores_turret: false,
                    },
                    bunker.damage.last_attacker,
                ));
            }
        }
        for (spawn, attacker) in deaths {
            changes.push((self.apply_hospital_cycle(spawn, false, attacker)?, attacker));
        }
        Ok(changes)
    }

    pub fn hospital(&self, index: u16) -> Option<IndexedHospital<'_>> {
        let bunker = self.bunkers.get(&index)?;
        let exit = bunker.exit.as_ref()?;
        let access = bunker.hospital.as_ref()?;
        Some(IndexedHospital {
            index,
            spawn_index: bunker.spawn,
            team: bunker.team,
            faction: bunker.faction,
            properties_source: &bunker.profile.properties_source,
            turret_qualified: self.hospital_turrets.contains_key(&index),
            operational: bunker.vehicle_state == VehicleState::Operational
                && !bunker.health.destroyed,
            position: exit.transform.position,
            max_occupancy: access.max_occupancy,
            passenger_seats: access.passenger_seats,
            exit_policy: exit,
            mount_points: &bunker.repair_points,
            mount_radii: &access.mount_radii,
        })
    }

    pub fn hospital_at_spawn(&self, spawn_index: u16) -> Option<IndexedHospital<'_>> {
        self.bunkers.iter().find_map(|(&index, bunker)| {
            (bunker.spawn == spawn_index)
                .then(|| self.hospital(index))
                .flatten()
        })
    }

    /// Apply the type-13 body mutation to a staged world. The caller must stage
    /// occupant and turret destruction in the same transaction before installing
    /// this world or transmitting the update.
    pub fn apply_hospital_cycle(
        &mut self,
        spawn_index: u16,
        enabled: bool,
        damage_attribution: u32,
    ) -> Result<HospitalUpdate, &'static str> {
        let (&object_index, bunker) = self
            .bunkers
            .iter_mut()
            .find(|(_, bunker)| bunker.spawn == spawn_index && bunker.exit.is_some())
            .ok_or("unbound_domination_hospital")?;
        let state = if enabled {
            VehicleState::Operational
        } else {
            VehicleState::Destroyed
        };
        let state_changed = bunker.vehicle_state != state;
        let destroys_turret = !enabled && !bunker.turret_destruction_started;
        let restores_turret = enabled && bunker.turret_destruction_started;
        if enabled {
            bunker.turret_destruction_started = false;
            if state_changed {
                bunker.health = BunkerHealth::for_profile(&bunker.profile, &self.catalog)?;
                bunker.damage.last_location_and_type = 0;
                bunker.damage.last_attacker = u32::MAX;
            }
        } else {
            if state_changed {
                bunker.health.damage(bunker.health.hp())?;
            }
            bunker.damage.last_attacker = damage_attribution;
            bunker.turret_destruction_started = true;
        }
        bunker.vehicle_state = state;
        if (state_changed || !enabled)
            && let Some(turret) = self.hospital_turrets.get_mut(&object_index)
        {
            turret.revoke();
        }
        bunker.damage.health_quarter_points = bunker.health.wire_health();
        bunker.damage.conditions = bunker.health.conditions();
        Ok(HospitalUpdate {
            object_index,
            state: state_changed.then_some(state),
            damage: (!enabled || state_changed).then_some(bunker.damage),
            kills_occupants: !enabled,
            destroys_turret,
            restores_turret,
        })
    }

    /// A map with no defensive emplacements: combat rules are still loaded from
    /// the structure catalog, but no placements or bunker spawns exist.
    pub fn without_bunkers(structures: impl AsRef<Path>) -> io::Result<Self> {
        Ok(Self {
            catalog: StructureCatalog::load(structures)?,
            bunkers: BTreeMap::new(),
            vehicle_token: 0,
            hospital_spawn_sha256: None,
            hospital_structure_sha256: None,
            hospital_turrets: Default::default(),
            pending_placements: 0,
            commands: Default::default(),
        })
    }
    pub fn load_default() -> io::Result<Self> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("data");
        Self::load(
            root.join("prototype-bunkers.json"),
            root.join("prototype-structures.json"),
            ACTIVE_MAP_NAME,
        )
    }
    pub fn active_count(&self) -> usize {
        self.bunkers.len()
    }
    pub fn unresolved_count(&self) -> usize {
        self.pending_placements
    }
    pub fn reset(&mut self) {
        self.hospital_turrets
            .values_mut()
            .for_each(crate::hospital_turrets::Runtime::reset);
        for b in self.bunkers.values_mut() {
            b.vehicle_state = VehicleState::Operational;
            b.turret_destruction_started = false;
            b.health = BunkerHealth::for_profile(&b.profile, &self.catalog).unwrap();
            b.damage.health_quarter_points = b.health.wire_health();
            b.damage.conditions = 0;
            b.damage.last_attacker = u32::MAX;
            b.damage.last_source_object = u32::MAX;
            b.damage.last_user_data = 0;
        }
        self.commands.clear();
    }
    fn index(&self, id: u32) -> Option<u16> {
        if id & 0xff7f0000 != 0x0d000000 {
            return None;
        }
        let index = id as u16;
        self.bunkers.contains_key(&index).then_some(index)
    }
    pub(super) fn canonical_alpha_team(&self) -> Option<u8> {
        self.bunkers.get(&0).map(|bunker| bunker.team)
    }
    pub(super) fn creations(&self, sequence: u32, time: u32) -> Vec<DefensiveEmplacementCreation> {
        self.bunkers
            .iter()
            .map(|(&index, b)| DefensiveEmplacementCreation {
                sequence,
                timestamp_ms: time,
                object_index: index,
                owner_index: 0x105,
                vehicle_type_token: self.vehicle_token,
                faction: b.faction,
                modifier: 0,
                state: b.vehicle_state,
                turret_owners: [u16::MAX; 2],
                damage: b.damage,
                team: b.team,
                spawn_point_index: b.spawn,
            })
            .collect()
    }
    fn damage_amount(
        &self,
        index: u16,
        raw: f32,
        pierce: f32,
        redux: f32,
        location: u8,
    ) -> Result<f32, &'static str> {
        let b = self.bunkers.get(&index).ok_or("unknown_bunker")?;
        let armor = self.catalog.armor(b.profile.armor_type)?;
        let body = self.catalog.body(location)?;
        let rating = (armor.rating - pierce).max(0.0);
        let after = (raw - rating).max(0.0);
        let prevention = (armor.prevention - redux).max(0.0);
        let product = armor.coverage[body.armor_area].max(0.0) * prevention;
        Ok((after * (body.multiplier * (1.0 - product))).max(0.0))
    }
}
#[derive(Clone, Copy, Debug)]
pub struct HospitalUpdate {
    object_index: u16,
    state: Option<VehicleState>,
    damage: Option<VehicleDamage>,
    kills_occupants: bool,
    destroys_turret: bool,
    restores_turret: bool,
}

impl HospitalUpdate {
    pub fn object_index(self) -> u16 {
        self.object_index
    }

    pub fn kills_occupants(self) -> bool {
        self.kills_occupants
    }

    /// The first destruction also requires native hulk, physics and blast effects.
    pub fn destroys_turret(self) -> bool {
        self.destroys_turret
    }

    pub fn restores_turret(self) -> bool {
        self.restores_turret
    }

    pub fn encode(
        self,
        sequence: u32,
        timestamp_ms: u32,
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Option<Vec<u8>>, &'static str> {
        if self.state.is_none() && self.damage.is_none() {
            return Ok(None);
        }
        let mut bits = BitWriter::with_profile(profile);
        UpdateHeader {
            sequence,
            nak_sequence: 0,
            client_index: 0,
            object_type_mask: 1 << 13,
            timestamp_ms,
        }
        .encode(&mut bits)?;
        bits.write(1, 1)?;
        bits.write(self.object_index.into(), 16)?;
        for field in 0..25 {
            match (field, self.state, self.damage) {
                (5, Some(state), _) => {
                    bits.write(1, 1)?;
                    bits.write(state as u32, 8)?;
                }
                (18, _, Some(damage)) => {
                    bits.write(1, 1)?;
                    damage.encode(&mut bits)?;
                }
                _ => bits.write(0, 1)?,
            }
        }
        bits.write(0, 1)?;
        Ok(Some(bits.finish()))
    }
}

#[derive(Clone, Copy)]
struct BunkerDelta {
    index: u16,
    damage: VehicleDamage,
}
impl BunkerDelta {
    fn encode_with_profile(
        self,
        seq: u32,
        time: u32,
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        Self::encode_list_with_profile(&[self], seq, time, profile)
    }
    fn encode_list_with_profile(
        deltas: &[Self],
        seq: u32,
        time: u32,
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        let mut bits = BitWriter::with_profile(profile);
        UpdateHeader {
            sequence: seq,
            nak_sequence: 0,
            client_index: 0,
            object_type_mask: 1 << 13,
            timestamp_ms: time,
        }
        .encode(&mut bits)?;
        for delta in deltas {
            bits.write(1, 1)?;
            bits.write(delta.index.into(), 16)?;
            for field in 0..25 {
                bits.write(u32::from(field == 18), 1)?;
                if field == 18 {
                    delta.damage.encode(&mut bits)?;
                }
            }
        }
        bits.write(0, 1)?;
        Ok(bits.finish())
    }
}
impl<W: Write> Engine<W> {
    /// Durability has priority over a periodic clock reply on valid owner updates.
    /// This query does not advance health or consume queued commands.
    pub(super) fn bunker_update_due(&self, now: Duration) -> bool {
        self.active
            .as_ref()
            .is_some_and(|a| a.entered_game && a.ready_for_updates)
            && (!self.pending_bunker_damage.is_empty()
                || self.bunker_world.as_ref().is_some_and(|world| {
                    world.bunkers.values().any(|b| {
                        b.hospital.is_none()
                            && !b.health.destroyed
                            && b.health.conditions() & 4 != 0
                            && now > b.last_health_update
                    })
                }))
    }
    pub(super) fn tick_bunkers(
        &mut self,
        now: Duration,
        wall: u64,
    ) -> Result<Option<Vec<OutboundMessage>>, &'static str> {
        let Some(actor) = self
            .active
            .as_ref()
            .filter(|a| a.entered_game && a.ready_for_updates)
        else {
            return Ok(None);
        };
        let Some(world) = self.bunker_world.as_ref() else {
            return Ok(None);
        };
        let mut changes = Vec::new();
        for (&index, bunker) in &world.bunkers {
            if bunker.hospital.is_some()
                || bunker.health.destroyed
                || bunker.health.conditions() & 4 == 0
                || now <= bunker.last_health_update
            {
                continue;
            }
            let mut next = bunker.clone();
            let per_second = (bunker.profile.critical_loss_percent_per_second * 0.01)
                * bunker.profile.max_health;
            let amount = per_second * now.saturating_sub(bunker.last_health_update).as_secs_f32();
            next.health.damage(amount)?;
            next.last_health_update = now;
            next.damage.health_quarter_points = next.health.wire_health();
            next.damage.conditions = next.health.conditions();
            changes.push((index, next, amount));
        }
        if changes.is_empty() {
            return Ok(None);
        }
        let deltas: Vec<_> = changes
            .iter()
            .map(|(index, b, _)| BunkerDelta {
                index: *index,
                damage: b.damage,
            })
            .collect();
        let packet = BunkerDelta::encode_list_with_profile(
            &deltas,
            actor.next_sequence()?,
            now.as_millis() as u32,
            self.wire_profile,
        )?;
        let recipient = (
            actor.reservation.id.clone(),
            actor.reservation.world,
            actor.reservation.player,
        );
        self.record(json!({"op":"bunker_critical_decay","changes":changes.iter().map(|(i,b,a)|json!({"object_index":i,"amount_hp":a,"health_hp":b.health.hp(),"conditions":b.damage.conditions})).collect::<Vec<_>>()}),wall)?;
        self.active.as_mut().unwrap().remember(packet.clone())?;
        for (index, next, _) in changes {
            self.bunker_world
                .as_mut()
                .unwrap()
                .bunkers
                .insert(index, next);
        }
        Ok(Some(vec![OutboundMessage {
            reservation_id: recipient.0,
            world_id: recipient.1,
            player_id: recipient.2,
            plugin_id: 1,
            payload_hex: protocol::encode_hex(&packet),
        }]))
    }
    pub fn with_bunker_world(mut self, world: BunkerWorld) -> Self {
        self.bunker_world = Some(world);
        self
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn queue_development_bunker_damage(
        &mut self,
        id: &str,
        world: u32,
        player: u16,
        command: &str,
        target: u32,
        amount: f32,
        wall: u64,
    ) -> Result<Vec<OutboundMessage>, &'static str> {
        if !self.development_control {
            return Err("development_control_disabled");
        }
        let actor = self.active.as_ref().ok_or("no_reservation")?;
        if actor.reservation.id != id
            || actor.reservation.world != world
            || actor.reservation.player != player
        {
            return Err("identity_mismatch");
        }
        if !actor.ready_for_updates || !actor.entered_game {
            return Err("structure_damage_requires_ready_player");
        }
        let w = self
            .bunker_world
            .as_ref()
            .ok_or("bunker_world_unavailable")?;
        let index = w.index(target).ok_or("unknown_bunker")?;
        if let Some(prior) = w.commands.get(command) {
            return if *prior == (index, amount) {
                Ok(Vec::new())
            } else {
                Err("structure_damage_id_conflict")
            };
        }
        if w.commands.len() >= 64 || self.pending_bunker_damage.len() >= 16 {
            return Err("structure_damage_queue_full");
        }
        self.record(json!({"op":"development_structure_damage_queued","reservation_id":id,"target_object_id":target,"damage_id":command,"amount_hp":amount}),wall)?;
        self.bunker_world
            .as_mut()
            .unwrap()
            .commands
            .insert(command.into(), (index, amount));
        self.pending_bunker_damage
            .push_back((index, amount, command.into()));
        Ok(Vec::new())
    }
    fn event_domination_hospital_hit(
        &mut self,
        request: &bullet_hit::NetMessageHit,
        index: u16,
        now: Duration,
        wall: u64,
    ) -> Result<Option<Vec<OutboundMessage>>, &'static str> {
        let active = self.active.as_ref().ok_or("no_reservation")?;
        let world = active.reservation.world;
        let attacker = 0x0380_0000 | u32::from(active.reservation.player);
        let result = (|| {
            if self.domination_gameplay.is_none() || !self.match_started {
                return Err("domination_hospital_damage_consumer_required");
            }
            let current = self
                .bunker_world
                .as_ref()
                .ok_or("bunker_world_unavailable")?;
            let bunker = current
                .bunkers
                .get(&index)
                .ok_or("unknown_domination_hospital_bullet_target")?;
            let (item_id, damage) = self.retail_vehicle_bullet_parameters(active, request)?;
            let catalog = current.damage_catalog();
            let friendly = if bunker.team == active.team() {
                catalog.friendly_fire_multiplier()
            } else {
                1.0
            };
            let amount = item_catalog::apply_vehicle_armor(
                damage,
                request.penetration_fraction(),
                friendly,
                catalog.armor(bunker.profile.armor_type)?,
                catalog.body(request.hit_location)?,
            )?;
            let mut candidate = current.clone();
            let packet = candidate
                .apply_domination_hospital_bullet(index, amount, attacker, request, now)?
                .map(|update| update.encode(1, now.as_millis() as u32, self.wire_profile))
                .transpose()?
                .flatten();
            Ok::<_, &'static str>((candidate, packet, item_id, amount))
        })();
        let mut audit = json!({
            "op": "domination_hospital_bullet_hit",
            "target_id": request.defender,
            "attacker_id": attacker,
            "hit_location": request.hit_location,
            "client_data": request.client_data,
        });
        match result {
            Ok((candidate, packet, item_id, amount)) => {
                audit["weapon_item"] = json!(item_id);
                audit["amount_hp"] = json!(amount);
                self.record(audit, wall)?;
                self.bunker_world = Some(candidate);
                if let Some(active) = self.active.as_mut() {
                    active.last_event = now;
                }
                if let Some(packet) = packet {
                    for recipient in self
                        .active
                        .iter_mut()
                        .chain(self.peers.values_mut())
                        .filter(|actor| {
                            actor.reservation.world == world && actor.emplacement_creation.is_some()
                        })
                    {
                        recipient.queue_shared(packet.clone());
                    }
                }
            }
            Err(code) => {
                audit["gameplay_denial"] = json!(code);
                self.record(audit, wall)?;
            }
        }
        Ok(Some(Vec::new()))
    }

    pub(super) fn structure_event(
        &mut self,
        message: &Result<protocol::GameMessage<'_>, &'static str>,
        now: Duration,
        wall: u64,
    ) -> Result<Option<Vec<OutboundMessage>>, &'static str> {
        let Ok(message) = message else {
            return Ok(None);
        };
        if let protocol::GameMessage::Hit(request) = message
            && let Some(index) = self.bunker_world.as_ref().and_then(|world| {
                world
                    .index(request.defender)
                    .filter(|&i| world.hospital(i).is_some())
            })
        {
            return self.event_domination_hospital_hit(request, index, now, wall);
        }
        let active = self.active.as_ref().unwrap();
        let w = self.bunker_world.as_ref();
        let repair = matches!(message, protocol::GameMessage::Repair(_));
        let mut pending = false;
        let critical = false;
        let mut raw = None;
        let mut repair_mount_distance = None;
        let mut repair_leadership = None;
        let mut repair_credit_input = None;
        let (target, amount, location, userdata, kind) = match message {
            protocol::GameMessage::Repair(r) => (r.target_object_id, 0.0, 0, 0, 0),
            protocol::GameMessage::Hit(r) if r.defender & 0xff000000 == 0x0d000000 => {
                let profile = self.bullet_profile(active, r);
                let result = profile.and_then(|p| self.item_catalog.vehicle_bullet(p.item_id()));
                raw = Some((r.penetration_fraction(), result));
                (r.defender, 0.0, r.hit_location, r.client_data, 1)
            }
            protocol::GameMessage::ObjectUpdate(update)
                if update.header.nak_sequence == 0 && !self.pending_bunker_damage.is_empty() =>
            {
                let &(index, hp, _) = self.pending_bunker_damage.front().unwrap();
                pending = true;
                (0x0d800000 | u32::from(index), hp, 0, 0, 1)
            }
            _ => return Ok(None),
        };
        let result = (|| {
            let w = w.ok_or("bunker_world_unavailable")?;
            let index = w.index(target).ok_or("unknown_bunker")?;
            let bunker = &w.bunkers[&index];
            // Indexed hospitals are changed by the recorded Domination vehicle
            // lifecycle. Legacy hit/pending-damage handling cannot supply its
            // occupant, hulk and peer-publication effects.
            if bunker.hospital.is_some() && !repair {
                return Err("domination_hospital_damage_consumer_required");
            }
            let retail_hospital_repair = repair
                && self.runtime_application_id == Some(20231)
                && w.hospital_spawn_sha256.is_some()
                && bunker.hospital.is_some();
            if !active.entered_game
                || !active.ready_for_updates
                || (!pending && !critical && !active.life.is_alive())
            {
                return Err("bunker_requires_living_ready_actor");
            }
            let mut next = bunker.clone();
            let mut amount = amount;
            if repair {
                if active.is_mounted() || bunker.team != active.team() {
                    return Err("repair_requires_allied_onfoot_actor");
                }
                if retail_hospital_repair
                    && (bunker.vehicle_state != VehicleState::Operational
                        || active.emplacement_creation.is_none())
                {
                    return Err("retail_hospital_not_operational");
                }
                // The retail tool keeps its cadence on the client. Bridge
                // processing times do not measure the spacing of those uses.
                if !retail_hospital_repair
                    && now
                        < self
                            .next_repair
                            .get(&active.reservation.id)
                            .copied()
                            .unwrap_or_default()
                {
                    return Err("repair_cooldown");
                }
                let handle = active
                    .current_inventory_handle
                    .ok_or("repair_inventory_unavailable")?;
                let kit = active
                    .equipped_loadout()
                    .filter(|_| {
                        retail_hospital_repair || active.wire_profile.catalog_item_ids_valid()
                    })
                    .ok_or("repair_inventory_unavailable")?;
                let observed = if active.primary_equipped {
                    active.observed_primary
                } else {
                    active.observed_secondary
                }
                .ok_or("repair_tool_not_equipped")?;
                let repair_item = w.catalog.repair().item_id;
                if (retail_hospital_repair && !self.item_catalog.is_retail_repair_tool(repair_item))
                    || observed.inventory_update_id != handle.update_id
                    || kit.inventories[usize::from(handle.inventory_index)]
                        .slots
                        .get(usize::from(observed.gear_slot))
                        .copied()
                        .flatten()
                        != Some(repair_item)
                {
                    return Err("repair_tool_not_equipped");
                }
                if bunker.health.conditions() & 4 != 0 && !bunker.health.destroyed {
                    return Err("critical_bunker_repair_requires_destruction");
                }
                if bunker.health.hp() == bunker.profile.max_health {
                    return Err("bunker_already_healthy");
                }
                if bunker.health.destroyed && !bunker.profile.can_repair_destroyed {
                    return Err("bunker_cannot_rebuild");
                }
                // Native repair selects the vehicle/turret through the client's
                // authored body raycast. Entry points are not repair surfaces;
                // retain their distance only as diagnostics, never an admission gate.
                repair_mount_distance = active
                    .tracking
                    .sample
                    .map(|sample| {
                        let pos = [sample.position.x, sample.position.y, sample.position.z];
                        bunker
                            .repair_points
                            .iter()
                            .map(|p| {
                                p.iter()
                                    .zip(pos)
                                    .map(|(a, b)| (*a - b).powi(2))
                                    .sum::<f32>()
                                    .sqrt()
                            })
                            .fold(f32::INFINITY, f32::min)
                    })
                    .filter(|distance| distance.is_finite());
                if retail_hospital_repair {
                    repair_leadership = Some(self.plan_leadership(now)?);
                }
                let modifier = repair_leadership
                    .as_ref()
                    .map_or(active.modifiers, |plan| plan.modifiers(active))
                    .value("kCharModRepairAmount");
                amount = w.catalog.repair_amount(&bunker.profile, modifier)?;
                next.health.repair(amount)?;
                // The health change is measured before quarter-point wire projection.
                if retail_hospital_repair
                    && next.health.hp() > bunker.health.hp()
                    && !self.attacker_is_friendly(
                        bunker.damage.last_attacker,
                        active.reservation.world,
                        active.team(),
                    )
                {
                    let position = w.hospital(index).map(|b| frago::point(b.position));
                    repair_credit_input = Some((modifier, position));
                }
            } else {
                if bunker.health.hp() == 0.0 {
                    return Err("bunker_destroyed");
                }
                if (active.ground_seat.is_some()
                    || active.mounted_seat.is_some_and(|seat| seat != 0))
                    && !pending
                {
                    return Err("bunker_hit_requires_armed_actor");
                }
                if bunker.health.conditions() & 4 != 0 && !pending && !critical {
                    return Err("critical_bunker_damage_over_time");
                }
                if let Some((scale, params)) = raw {
                    let params = params?;
                    amount = w.damage_amount(
                        index,
                        (params.amount * scale)
                            * if bunker.team == active.team() {
                                w.catalog.friendly_fire_multiplier()
                            } else {
                                1.0
                            },
                        params.armor_pierce,
                        params.armor_redux,
                        location,
                    )?;
                }
                next.health.damage(amount)?;
            }
            next.last_health_update = now;
            if !repair && !critical {
                next.damage = VehicleDamage {
                    last_location_and_type: (kind << 4) | location,
                    health_quarter_points: next.health.wire_health(),
                    conditions: next.health.conditions(),
                    last_attacker: 0x03800000 | u32::from(active.reservation.player),
                    last_source_object: u32::MAX,
                    last_user_data: userdata,
                };
            }
            next.damage.health_quarter_points = next.health.wire_health();
            next.damage.conditions = next.health.conditions();
            let packet = BunkerDelta {
                index,
                damage: next.damage,
            }
            .encode_with_profile(
                if retail_hospital_repair {
                    1
                } else {
                    active.next_sequence()?
                },
                now.as_millis() as u32,
                self.wire_profile,
            )?;
            Ok::<_, &'static str>((index, next, amount, packet, retail_hospital_repair))
        })();
        let id = active.reservation.id.clone();
        let world = active.reservation.world;
        let player = active.reservation.player;
        let event_type = if repair {
            "repair"
        } else if critical {
            "bunker_critical_decay"
        } else {
            "bunker_hit"
        };
        match result {
            Err(code) => {
                self.record(
                    json!({"op":"structure_event", "reservation_id":id, "type":event_type,
                        "target_object_id":target, "gameplay_denial":code}),
                    wall,
                )?;
                if pending && code != "bunker_requires_living_ready_actor" {
                    self.pending_bunker_damage.pop_front();
                }
                Ok(Some(Vec::new()))
            }
            Ok((index, next, amount, packet, retail_hospital_repair)) => {
                let repair_credit = repair_credit_input
                    .map(|(modifier, position)| self.plan_repair_credit(active, modifier, position))
                    .transpose()?;
                let mut audit = json!({
                    "op":"structure_event", "reservation_id":id, "type":event_type,
                    "target_object_id":target, "catalog_id":next.id, "amount_hp":amount,
                    "health_hp":next.health.hp(), "health_quarter_points":next.damage.health_quarter_points,
                    "conditions":next.damage.conditions, "repair_mount_distance":repair_mount_distance,
                    "policy":"native_vehicle_armor_float_health_client_repair_target",
                });
                if let Some(leadership) = &repair_leadership {
                    audit["leadership_changes"] = json!(leadership.audit());
                }
                if let Some(credit) = &repair_credit {
                    audit["repair_credit"] = credit.audit();
                }
                self.record(audit, wall)?;
                if !retail_hospital_repair {
                    self.active.as_mut().unwrap().remember(packet.clone())?;
                }
                self.active.as_mut().unwrap().last_event = now;
                self.bunker_world
                    .as_mut()
                    .unwrap()
                    .bunkers
                    .insert(index, next);
                if repair && !retail_hospital_repair {
                    self.next_repair.insert(
                        id.clone(),
                        now + Duration::from_secs_f32(
                            self.bunker_world
                                .as_ref()
                                .unwrap()
                                .catalog
                                .repair()
                                .cooldown_seconds,
                        ),
                    );
                }
                if pending {
                    self.pending_bunker_damage.pop_front();
                }
                if retail_hospital_repair {
                    if let Some(leadership) = repair_leadership {
                        self.commit_leadership(leadership);
                    }
                    for recipient in self
                        .active
                        .iter_mut()
                        .chain(self.peers.values_mut())
                        .filter(|peer| {
                            peer.reservation.world == world && peer.emplacement_creation.is_some()
                        })
                    {
                        recipient.queue_shared(packet.clone());
                    }
                    let messages = repair_credit
                        .map_or_else(Vec::new, |plan| self.install_repair_credit(plan));
                    return Ok(Some(messages));
                }
                Ok(Some(vec![OutboundMessage {
                    reservation_id: id,
                    world_id: world,
                    player_id: player,
                    plugin_id: 1,
                    payload_hex: protocol::encode_hex(&packet),
                }]))
            }
        }
    }
}

#[cfg(test)]
#[path = "bunker_team_tests.rs"]
mod bunker_team_tests;

#[cfg(test)]
#[path = "hospital_repair_credit_tests.rs"]
mod hospital_repair_credit_tests;

#[cfg(test)]
#[path = "alpha_mount_team_tests.rs"]
mod alpha_mount_team_tests;

#[cfg(test)]
mod domination_placement_tests {
    use super::*;

    #[test]
    fn domination_bindings_require_exact_spawn_profile_and_explicit_ids() {
        let squads: Vec<_> = (0..32)
            .map(|index| {
                json!({
                    "index":index,"team":index/16,"platoon":index%16/4,"squad":index%4,
                    "faction":if index <16 {None} else {Some(0)},"available":[0],"default_spawn":0,
                })
            })
            .collect();
        let locations: Vec<_> = (0..17).map(|index| json!({
            "index":index,"token":100+index,"kind":if index == 0 { "insertion" } else { "bunker_passenger" },
            "position":{"x":0.,"y":0.,"z":0.},
        })).collect();
        let catalog = spawn_catalog::SpawnCatalog::from_json(&json!({
            "format":"mag_spawn_catalog","schema_version":1,"source":"synthetic",
            "active":{"map_name":"hospital_test","squad_index":0,"default_spawn":0,"available":[0]},
            "match_profile":{"game_type":2,"auto_assign_teams":true,"attacking_faction":1,
                "has_bunkers":true,"has_objectives":false},
            "maps":[{"name":"hospital_test","owner_faction":0,"spawn_count":17,
                "runtime_indices_verified":true,"runtime_indices_basis":"synthetic",
                "squads":squads,"locations":locations}],
        }).to_string()).unwrap();
        let mut raw = HospitalPlacements {
            format: "mag_domination_hospital_placements_v1".into(),
            spawn_catalog_sha256: catalog.source_sha256().unwrap().into(),
            map_name: catalog.map_name().into(),
            structure_catalog_sha256: "a".repeat(64),
            source_proof_sha256: "b".repeat(64),
            passenger_pose_sha256: "c".repeat(64),
            passenger_pose_scope: "retail_unresolved_passenger_node_zero_local_pose".into(),
            allocation_scope: "successful_production_fresh_level_load".into(),
            vehicle_type_token: 1,
            placements: (0..16)
                .rev()
                .map(|object_index| HospitalPlacement {
                    object_index,
                    spawn_index: object_index + 1,
                    spawn_token: 101 + u32::from(object_index),
                    properties_source: "synthetic".into(),
                    mounts: vec![HospitalMount {
                        local_offset: [0., 0., 0.],
                        radius: 2.0,
                    }],
                    max_occupancy: 12,
                    passenger_seats: 12,
                    exit: vehicle_exit::BunkerExitPolicy {
                        transform: vehicle_exit::BunkerTransform {
                            position: replication::CharacterPosition {
                                x: 1.,
                                y: 0.,
                                z: 1.,
                            },
                            rotation_xyzw: [0., 0., 0., 1.],
                        },
                        candidates: vec![vehicle_exit::ExitCandidate {
                            ranking_offset: [1., 0., 0.],
                            position_offset: [1., 0., 0.],
                        }],
                    },
                })
                .collect(),
        };
        assert!(raw.validate(&catalog).is_ok());
        raw.placements[0].object_index = raw.placements[1].object_index;
        assert!(raw.validate(&catalog).is_err());
        raw.placements[0].object_index = 15;
        raw.placements[0].spawn_token = 100;
        assert!(raw.validate(&catalog).is_err());
        raw.placements[0].spawn_token = 116;
        raw.spawn_catalog_sha256 = "c".repeat(64);
        assert!(raw.validate(&catalog).is_err());
        raw.spawn_catalog_sha256 = catalog.source_sha256().unwrap().into();
        raw.placements.pop();
        assert!(raw.validate(&catalog).is_err());
    }
    #[test]
    fn acquisition_bindings_cover_each_field_hospital_without_domination_count() {
        let profile = |game_type: u8| {
            let squads: Vec<_> = (0..32)
                .map(|index| {
                    json!({
                        "index":index,"team":index/16,"platoon":index%16/4,"squad":index%4,
                        "faction":if index <16 {None} else {Some(0)},"available":[0],
                        "default_spawn":0,
                    })
                })
                .collect();
            let locations: Vec<_> = (0..9)
                .map(|index| {
                    json!({
                        "index":index,"token":100+index,
                        "kind":if index == 0 { "insertion" } else { "bunker_passenger" },
                        "position":{"x":0.,"y":0.,"z":0.},
                    })
                })
                .collect();
            spawn_catalog::SpawnCatalog::from_json(&json!({
                "format":"mag_spawn_catalog","schema_version":1,"source":"synthetic",
                "active":{"map_name":"hospital_test","squad_index":0,"default_spawn":0,"available":[0]},
                "match_profile":{"game_type":game_type,"auto_assign_teams":true,"attacking_faction":1,
                    "has_bunkers":true,"has_objectives":false},
                "maps":[{"name":"hospital_test","owner_faction":0,"spawn_count":9,
                    "runtime_indices_verified":true,"runtime_indices_basis":"synthetic",
                    "squads":squads,"locations":locations}],
            }).to_string())
        };
        assert!(profile(0).is_err_and(|error| {
            error
                .to_string()
                .contains("automatic_bunker_assignment_unsupported")
        }));
        let catalog = profile(1).unwrap();
        let mut raw = HospitalPlacements {
            format: "mag_domination_hospital_placements_v1".into(),
            spawn_catalog_sha256: catalog.source_sha256().unwrap().into(),
            map_name: catalog.map_name().into(),
            structure_catalog_sha256: "a".repeat(64),
            source_proof_sha256: "b".repeat(64),
            passenger_pose_sha256: "c".repeat(64),
            passenger_pose_scope: "retail_unresolved_passenger_node_zero_local_pose".into(),
            allocation_scope: "successful_production_fresh_level_load".into(),
            vehicle_type_token: 1,
            placements: (0..8)
                .map(|object_index| HospitalPlacement {
                    object_index,
                    spawn_index: object_index + 1,
                    spawn_token: 101 + u32::from(object_index),
                    properties_source: "synthetic".into(),
                    mounts: vec![HospitalMount {
                        local_offset: [0., 0., 0.],
                        radius: 2.0,
                    }],
                    max_occupancy: 12,
                    passenger_seats: 12,
                    exit: vehicle_exit::BunkerExitPolicy {
                        transform: vehicle_exit::BunkerTransform {
                            position: replication::CharacterPosition {
                                x: 1.,
                                y: 0.,
                                z: 1.,
                            },
                            rotation_xyzw: [0., 0., 0., 1.],
                        },
                        candidates: vec![vehicle_exit::ExitCandidate {
                            ranking_offset: [1., 0., 0.],
                            position_offset: [1., 0., 0.],
                        }],
                    },
                })
                .collect(),
        };
        raw.validate(&catalog).unwrap();
        // Every hospital point still needs its own emplacement.
        raw.placements.pop();
        assert!(raw.validate(&catalog).is_err());
    }

    fn hospital_world() -> BunkerWorld {
        let mut world = bunker_team_tests::fixture(1, 1, true)
            .bunker_world
            .take()
            .unwrap();
        let mut bunker = world.bunkers.remove(&0).unwrap();
        bunker.exit = Some(vehicle_exit::BunkerExitPolicy {
            transform: vehicle_exit::BunkerTransform {
                position: replication::CharacterPosition {
                    x: 1.,
                    y: 0.,
                    z: 1.,
                },
                rotation_xyzw: [0., 0., 0., 1.],
            },
            candidates: vec![vehicle_exit::ExitCandidate {
                ranking_offset: [1., 0., 0.],
                position_offset: [1., 0., 0.],
            }],
        });
        bunker.hospital = Some(HospitalAccess {
            mount_radii: vec![],
            max_occupancy: 12,
            passenger_seats: 12,
            direct_damage: true,
        });
        world.bunkers.insert(11, bunker);
        world
    }

    #[test]
    fn hospital_bullets_stage_health_then_one_presence_transition() {
        let mut world = hospital_world();
        world
            .bunkers
            .get_mut(&11)
            .unwrap()
            .profile
            .critical_loss_percent_per_second = 3.0;
        let request = bullet_hit::NetMessageHit {
            gear_index: 0,
            fire_mode: 0,
            hit_location: 2,
            client_data: 123,
            defender: 0x0d80_000b,
            hit_origin: [1.; 3],
            hit_dest: [2.; 3],
            penetration_milli: 1000,
        };
        let attacker = 0x0380_0007;
        let at_hit = Duration::from_secs(1);
        let update = world
            .apply_domination_hospital_bullet(11, 190.0, attacker, &request, at_hit)
            .unwrap()
            .unwrap();
        assert!(update.state.is_none() && !update.kills_occupants());
        let damage = update.damage.unwrap();
        assert_eq!((damage.health_quarter_points, damage.conditions), (40, 7));
        assert_eq!(
            (damage.last_attacker, damage.last_user_data),
            (attacker, 123)
        );
        assert!(
            world
                .apply_domination_hospital_bullet(11, 1.0, attacker, &request, at_hit)
                .is_err()
        );
        let decay = world
            .advance_domination_hospital_damage(Duration::from_secs(2))
            .unwrap();
        assert_eq!(decay[0].0.damage.unwrap().health_quarter_points, 16);
        assert!(!decay[0].0.kills_occupants());
        let death = world
            .advance_domination_hospital_damage(Duration::from_secs(3))
            .unwrap();
        assert_eq!(death[0].1, attacker);
        assert_eq!(death[0].0.state, Some(VehicleState::Destroyed));
        assert_eq!(death[0].0.damage.unwrap().last_attacker, attacker);
        assert!(death[0].0.kills_occupants() && death[0].0.destroys_turret());
        assert!(
            world
                .advance_domination_hospital_damage(Duration::from_secs(4))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn domination_cycle_restores_damage_and_reports_external_destruction() {
        let mut world = hospital_world();
        let disable = world.apply_hospital_cycle(0, false, 0x0c800007).unwrap();
        assert_eq!(disable.object_index(), 11);
        assert!(disable.kills_occupants());
        assert!(disable.destroys_turret());
        assert_eq!(disable.state, Some(VehicleState::Destroyed));
        assert_eq!(world.bunkers[&11].health.hp(), 0.0);
        assert_eq!(world.bunkers[&11].damage.last_attacker, 0x0c800007);
        let repeated = world.apply_hospital_cycle(0, false, 0x0c800007).unwrap();
        assert!(!repeated.destroys_turret());
        let restore = world.apply_hospital_cycle(0, true, 0x0c800007).unwrap();
        assert!(!restore.kills_occupants());
        assert!(restore.restores_turret());
        assert_eq!(restore.state, Some(VehicleState::Operational));
        assert_eq!(world.bunkers[&11].health.hp(), 200.0);
        assert_eq!(world.bunkers[&11].damage.conditions, 0);
        assert_eq!(world.bunkers[&11].damage.last_attacker, u32::MAX);
        assert!(world.apply_hospital_cycle(1, false, 0x0c800007).is_err());
        let again = world.apply_hospital_cycle(0, false, 0x0c800007).unwrap();
        assert!(again.kills_occupants());
        assert!(again.destroys_turret());
    }
}

#[cfg(test)]
#[path = "domination_bunker_seat_tests.rs"]
pub(crate) mod domination_bunker_seat_tests;

#[cfg(test)]
impl BunkerWorld {
    /// SYNTHETIC: the hospital fixture's operational hospital, rebound to
    /// `spawn` for `team`/`faction` and placed at `position` with one exit
    /// 2 m along +x.
    pub(crate) fn synthetic_hospital(
        spawn: u16,
        team: u8,
        faction: u8,
        position: replication::CharacterPosition,
    ) -> Self {
        let mut world = domination_bunker_seat_tests::fixture()
            .bunker_world
            .take()
            .unwrap();
        let mut hospital = world.bunkers.remove(&5).unwrap();
        world.bunkers.clear();
        hospital.spawn = spawn;
        hospital.team = team;
        hospital.faction = faction;
        hospital.exit.as_mut().unwrap().transform.position = position;
        world.bunkers.insert(5, hospital);
        world
    }
}
