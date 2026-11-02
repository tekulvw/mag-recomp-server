//! Retail type1 ground-vehicle creation from explicit external bindings.
use crate::replication::{BitWriter, CharacterPosition, UpdateHeader, VehicleDamage};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::{
    fs,
    io::{self, Read},
    path::Path,
};

/// A source-qualified world transform and two distinct native pool identities.
/// No authored row ordinal is implicitly converted to either index.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Creation {
    pub index: u16,
    pub spawn_point_index: u16,
    pub spawn_point_token: u32,
    pub vehicle_type_token: u32,
    pub faction: u8,
    pub team: u8,
    pub modifier: u8,
    pub position: CharacterPosition,
    /// Native shared Object::Rotation order is X,Y,Z,W.
    pub rotation: [f32; 4],
    pub health_quarter_points: u16,
    /// State0 is an explicitly qualified hidden dynamic creation; state2 is the
    /// existing operational static foundation. This is not a respawn timer.
    #[serde(default = "operational_state")]
    pub initial_state: u8,
    #[serde(default)]
    pub passenger_seats: Vec<u8>,
    #[serde(default)]
    pub maximum_occupancy: Option<u8>,
    #[serde(default)]
    pub maximum_mount_distance_m: Option<f32>,
    #[serde(default)]
    pub maximum_mount_position_age_ms: Option<u64>,
    #[serde(default)]
    pub passenger_exit_candidates: Vec<crate::vehicle_exit::ExitCandidate>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    spawn_catalog_sha256: String,
    native_elf_sha256: String,
    native_binding_sha256: String,
    vehicle_properties_sha256: String,
    #[serde(default)]
    creation_scope: Option<String>,
}
fn operational_state() -> u8 {
    2
}

/// Explicit player spawn pool identity to ground vehicle network identity.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PassengerSpawn {
    pub spawn_point_index: u16,
    pub spawn_point_token: u32,
    pub vehicle_index: u16,
}
/// Source-bound native vehicle source; independent of human spawn availability.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VehicleSource {
    pub vehicle_index: u16,
    pub spawn_point_index: u16,
    pub spawn_point_token: u32,
    pub vehicle_type_token: u32,
    pub spawn_type: u8,
    pub user_class: u8,
    pub source_family: u8,
    pub authored_active: bool,
    pub team: u8,
}
/// Explicit qualified external bindings. No default vehicles or native IDs.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    format: String,
    schema_version: u32,
    source: Source,
    vehicles: Vec<Creation>,
    #[serde(default)]
    passenger_spawns: Vec<PassengerSpawn>,
    #[serde(default)]
    vehicle_sources: Vec<VehicleSource>,
}
impl Catalog {
    pub fn load(
        path: impl AsRef<Path>,
        spawns: &crate::spawn_catalog::SpawnCatalog,
    ) -> io::Result<Self> {
        let mut text = String::new();
        fs::File::open(path)?
            .take(1_048_577)
            .read_to_string(&mut text)?;
        Self::from_json(&text, spawns)
    }
    pub fn from_json(text: &str, spawns: &crate::spawn_catalog::SpawnCatalog) -> io::Result<Self> {
        if text.len() > 1_048_576 {
            return Err(io::Error::other("ground_vehicle_catalog_too_large"));
        }
        let mut value: Self = serde_json::from_str(text).map_err(io::Error::other)?;
        value.validate_for(spawns).map_err(io::Error::other)?;
        value.vehicles.sort_by_key(|v| v.index);
        Ok(value)
    }
    pub(crate) fn validate_for(
        &self,
        spawns: &crate::spawn_catalog::SpawnCatalog,
    ) -> Result<(), &'static str> {
        if self.format != "mag_ground_vehicles"
            || self.schema_version != 1
            || self.vehicles.is_empty()
            || self.vehicles.len() > 32
        {
            return Err("invalid_ground_vehicle_catalog");
        }
        if !spawns.map_pool().runtime_indices_verified {
            return Err("ground_vehicles_require_verified_spawn_pool");
        }
        for hash in [
            &self.source.spawn_catalog_sha256,
            &self.source.native_elf_sha256,
            &self.source.native_binding_sha256,
            &self.source.vehicle_properties_sha256,
        ] {
            if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err("invalid_ground_vehicle_provenance");
            }
        }
        if spawns.source_sha256() != Some(self.source.spawn_catalog_sha256.as_str()) {
            return Err("ground_vehicle_spawn_catalog_mismatch");
        }
        let policy = spawns.match_profile();
        if policy.game_type != 1 {
            return Err("ground_vehicles_require_acquisition");
        }
        if self.source.creation_scope.as_deref().is_some_and(|v| {
            !matches!(
                v,
                "native_requested_free_slot_hidden_state0" | "native_vehicle_source_points"
            )
        }) {
            return Err("invalid_ground_vehicle_creation_scope");
        }
        let mut source_points = std::collections::BTreeMap::new();
        if !self.vehicle_sources.is_empty()
            && self.source.creation_scope.as_deref() != Some("native_vehicle_source_points")
        {
            return Err("vehicle_source_qualification_required");
        }
        for source in &self.vehicle_sources {
            let vehicle = self
                .vehicle(source.vehicle_index)
                .ok_or("unknown_vehicle_source_identity")?;
            if source.vehicle_index < 4
                || !matches!(source.spawn_type, 7 | 9)
                || source.user_class != 1
                || source.source_family != 35
                || source.spawn_point_index != vehicle.spawn_point_index
                || source.spawn_point_token != vehicle.spawn_point_token
                || source.vehicle_type_token != vehicle.vehicle_type_token
                || source.team != vehicle.team
                || source.authored_active != (vehicle.initial_state == 2)
                || source_points.insert(source.vehicle_index, source).is_some()
            {
                return Err("invalid_native_vehicle_source");
            }
        }
        let mut ids = BTreeSet::new();
        let mut points = BTreeSet::new();
        for v in &self.vehicles {
            v.validate()?;
            let point = spawns
                .point(v.spawn_point_index)
                .ok_or("unknown_ground_vehicle_spawn")?;
            let faction = if v.team == 0 {
                policy.attacking_faction
            } else {
                policy.defending_faction
            };
            let source = source_points.get(&v.index);
            if self.source.creation_scope.as_deref() == Some("native_vehicle_source_points")
                && v.index >= 4
                && source.is_none()
            {
                return Err("missing_native_vehicle_source");
            }
            let source_active = source.map_or(point.active, |s| s.authored_active);
            if !ids.insert(v.index)
                || !points.insert(v.spawn_point_index)
                || point.token != v.spawn_point_token
                || (source_active != (v.initial_state == 2))
                || v.faction != faction
            {
                return Err("invalid_ground_vehicle_spawn_binding");
            }
        }
        if self.vehicles.iter().any(|v| v.initial_state == 0)
            && !matches!(
                self.source.creation_scope.as_deref(),
                Some("native_requested_free_slot_hidden_state0" | "native_vehicle_source_points")
            )
        {
            return Err("hidden_vehicle_requires_creation_qualification");
        }
        let mut passengers = BTreeSet::new();
        for binding in &self.passenger_spawns {
            let point = spawns
                .point(binding.spawn_point_index)
                .ok_or("unknown_passenger_spawn")?;
            if !passengers.insert(binding.spawn_point_index)
                || point.token != binding.spawn_point_token
                || !point.active
                || !matches!(
                    point.kind,
                    crate::spawn_catalog::SpawnKind::VehiclePassenger
                )
                || !self.vehicle(binding.vehicle_index).is_some_and(|v| {
                    !v.passenger_seats.is_empty()
                        && v.maximum_occupancy.is_some()
                        && v.exit_policy().is_some()
                })
            {
                return Err("invalid_passenger_spawn_binding");
            }
        }
        // Every advertised APC insertion point must resolve to a configured creation.
        for index in 0..128 {
            if spawns.point(index).is_some_and(|point| {
                point.active
                    && matches!(
                        point.kind,
                        crate::spawn_catalog::SpawnKind::VehiclePassenger
                    )
            }) && self.passenger_vehicle(index).is_none()
            {
                return Err("missing_passenger_spawn_binding");
            }
        }
        Ok(())
    }
    pub(crate) fn vehicle_source(&self, index: u16) -> Option<&VehicleSource> {
        self.vehicle_sources
            .iter()
            .find(|source| source.vehicle_index == index)
    }
    pub(crate) fn creations(&self) -> impl Iterator<Item = &Creation> {
        self.vehicles.iter()
    }
    pub(crate) fn vehicle(&self, index: u16) -> Option<&Creation> {
        self.vehicles.iter().find(|vehicle| vehicle.index == index)
    }
    pub(crate) fn passenger_vehicle(&self, point: u16) -> Option<&Creation> {
        self.passenger_spawns
            .iter()
            .find(|binding| binding.spawn_point_index == point)
            .and_then(|binding| self.vehicle(binding.vehicle_index))
    }
    pub(crate) fn batch_count(&self) -> usize {
        self.vehicles.len().div_ceil(16)
    }
    #[cfg(test)]
    pub(crate) fn batch(
        &self,
        index: usize,
        sequence: u32,
        timestamp: u32,
    ) -> Result<Vec<u8>, &'static str> {
        let first = index
            .checked_mul(16)
            .ok_or("invalid_ground_vehicle_batch")?;
        let vehicles = self
            .vehicles
            .get(first..self.vehicles.len().min(first.saturating_add(16)))
            .ok_or("invalid_ground_vehicle_batch")?;
        encode(sequence, timestamp, vehicles)
    }
}

impl<W: io::Write> crate::Engine<W> {
    /// Current owned authority layered on immutable creation resources.
    pub(crate) fn current_ground_vehicle(&self, index: u16) -> Option<Creation> {
        let mut creation = self.ground_vehicles.as_ref()?.vehicle(index)?.clone();
        let current = self.ground_runtime.as_ref()?.get(index)?;
        creation.position = current.pose.position;
        creation.rotation = current.pose.rotation;
        creation.initial_state = current.state;
        creation.health_quarter_points = current.health_quarter_points;
        Some(creation)
    }
    pub(crate) fn ground_vehicle_is_operational(&self, index: u16) -> bool {
        self.ground_runtime
            .as_ref()
            .and_then(|runtime| runtime.get(index))
            .is_some_and(|current| {
                current.state == 2
                    && !current.is_dead()
                    && current.body == crate::acquisition_ground_runtime::Body::Operational
            })
    }
    pub(crate) fn ground_vehicle_creation_complete(&self, completed_batches: usize) -> bool {
        self.ground_vehicles.as_ref().is_none_or(|catalog| {
            self.ground_runtime.is_some() && completed_batches >= catalog.batch_count()
        })
    }
    /// Creation replay for this recipient uses current authority, while the
    /// external catalog still admits only source-qualified initial states0/2.
    pub(crate) fn current_ground_batch(
        &self,
        index: usize,
        sequence: u32,
        timestamp_ms: u32,
    ) -> Result<Vec<u8>, &'static str> {
        let catalog = self
            .ground_vehicles
            .as_ref()
            .ok_or("ground_vehicle_catalog_required")?;
        let runtime = self
            .ground_runtime
            .as_ref()
            .ok_or("ground_vehicle_runtime_required")?;
        let first = index
            .checked_mul(16)
            .ok_or("invalid_ground_vehicle_batch")?;
        let resources = catalog
            .vehicles
            .get(first..catalog.vehicles.len().min(first.saturating_add(16)))
            .ok_or("invalid_ground_vehicle_batch")?;
        let vehicles = resources
            .iter()
            .map(|source| {
                self.current_ground_vehicle(source.index)
                    .ok_or("ground_vehicle_runtime_required")
            })
            .collect::<Result<Vec<_>, _>>()?;
        encode_list(sequence, timestamp_ms, &vehicles, |vehicle, bits| {
            vehicle.validate_runtime()?;
            let current = runtime
                .get(vehicle.index)
                .ok_or("ground_vehicle_runtime_required")?;
            vehicle.encode_fields_with_damage(bits, vehicle.damage(current.last_attacker))
        })
    }
    pub fn with_ground_vehicles(mut self, catalog: Catalog) -> Result<Self, &'static str> {
        if self.wire_profile.application_id() != 20231
            || self.active.is_some()
            || !self.peers.is_empty()
        {
            return Err("ground_vehicle_configuration_requires_idle_retail_engine");
        }
        catalog.validate_for(
            self.spawn_catalog
                .as_ref()
                .ok_or("spawn_catalog_unavailable")?,
        )?;
        if let Some(resources) = &self.dynamic_body_resources {
            resources.validate_vehicles(&catalog)?;
        }
        let runtime = crate::acquisition_ground_runtime::Runtime::from_catalog(&catalog)?;
        self.ground_runtime = Some(runtime);
        self.ground_vehicles = Some(catalog);
        Ok(self)
    }
}
impl Creation {
    pub fn object_id(&self) -> u32 {
        0x01000000 | u32::from(self.index)
    }
    pub(crate) fn exit_policy(&self) -> Option<crate::vehicle_exit::BunkerExitPolicy> {
        (!self.passenger_exit_candidates.is_empty()).then(|| {
            crate::vehicle_exit::BunkerExitPolicy {
                transform: crate::vehicle_exit::BunkerTransform {
                    position: self.position,
                    rotation_xyzw: self.rotation,
                },
                candidates: self.passenger_exit_candidates.clone(),
            }
        })
    }
    fn validate(&self) -> Result<(), &'static str> {
        self.validate_common()?;
        if !matches!(self.initial_state, 0 | 2)
            || (self.initial_state == 0 && (self.index < 4 || !self.passenger_seats.is_empty()))
            || self.health_quarter_points == 0
        {
            return Err("invalid_ground_vehicle_creation");
        }
        Ok(())
    }
    fn validate_runtime(&self) -> Result<(), &'static str> {
        self.validate_common()?;
        if !matches!(self.initial_state, 0 | 2 | 4 | 5 | 7 | 8) {
            return Err("invalid_ground_vehicle_runtime_state");
        }
        Ok(())
    }
    fn validate_common(&self) -> Result<(), &'static str> {
        let norm = self
            .rotation
            .iter()
            .map(|v| f64::from(*v).powi(2))
            .sum::<f64>();
        if self.index >= 32
            || self.spawn_point_index >= 128
            || self.vehicle_type_token == 0
            || self.faction > 2
            || self.team > 1
            || self.modifier >= 64
            || self.health_quarter_points >= 4096
            || self.rotation.iter().any(|v| !v.is_finite())
            || (norm - 1.).abs() > 0.002
        {
            return Err("invalid_ground_vehicle_creation");
        }
        if self
            .maximum_mount_distance_m
            .is_some_and(|distance| !distance.is_finite() || distance <= 0.)
            || self.maximum_mount_position_age_ms == Some(0)
            || self.maximum_mount_distance_m.is_some()
                != self.maximum_mount_position_age_ms.is_some()
        {
            return Err("invalid_ground_mount_policy");
        }
        let seats: BTreeSet<_> = self.passenger_seats.iter().copied().collect();
        if seats.len() != self.passenger_seats.len()
            || seats.iter().any(|seat| *seat >= 16)
            || self
                .maximum_occupancy
                .is_some_and(|count| count == 0 || usize::from(count) > seats.len())
            || (self.passenger_seats.is_empty() != self.maximum_occupancy.is_none())
        {
            return Err("invalid_ground_vehicle_passenger_policy");
        }
        self.position.validate()?;
        if let Some(policy) = self.exit_policy() {
            policy.validate()?;
        }
        Ok(())
    }
    fn encode_fields(&self, bits: &mut BitWriter) -> Result<(), &'static str> {
        self.validate()?;
        self.encode_fields_with_damage(bits, self.damage(u32::MAX))
    }
    fn damage(&self, last_attacker: u32) -> VehicleDamage {
        VehicleDamage {
            last_location_and_type: 0,
            health_quarter_points: self.health_quarter_points,
            conditions: 0,
            last_attacker,
            last_source_object: u32::MAX,
            last_user_data: 0,
        }
    }
    fn encode_fields_with_damage(
        &self,
        bits: &mut BitWriter,
        damage: VehicleDamage,
    ) -> Result<(), &'static str> {
        let fields = [0, 1, 3, 4, 5, 6, 7, 18, 19, 20, 25, 26]
            .into_iter()
            .fold(0, |mask, field| mask | (1 << field));
        self.encode_masked_fields(bits, damage, fields)
    }
    fn encode_masked_fields(
        &self,
        bits: &mut BitWriter,
        damage: VehicleDamage,
        mask: u32,
    ) -> Result<(), &'static str> {
        bits.write(1, 1)?;
        bits.write(self.index.into(), 16)?;
        for field in 0..30 {
            let present = mask & (1 << field) != 0;
            bits.write(u32::from(present), 1)?;
            if !present {
                continue;
            }
            match field {
                0 => bits.write(1, 1)?,
                1 => bits.write(0x105, 16)?,
                3 => bits.write(self.vehicle_type_token, 32)?,
                4 => bits.write(u32::from((self.modifier << 2) | self.faction), 8)?,
                5 => bits.write(u32::from(self.initial_state), 8)?,
                6 | 7 => bits.write(u16::MAX.into(), 16)?,
                18 => damage.encode(bits)?,
                19 => bits.write(self.team.into(), 8)?,
                20 => bits.write(self.spawn_point_index.into(), 16)?,
                25 => self.position.encode(bits)?,
                26 => {
                    for value in self.rotation {
                        bits.write(u32::from(value < 0.), 1)?;
                        let magnitude = value.abs();
                        let integer = magnitude as u32;
                        bits.write(integer, 2)?;
                        bits.write(((magnitude - integer as f32) * 256.) as u32, 8)?;
                    }
                }
                _ => (),
            }
        }
        Ok(())
    }
}

/// A single bounded retail type1 list. Equipment/angles retain native defaults;
/// this foundation does not fabricate armed inventory, driving or seat state.
pub fn encode(
    sequence: u32,
    timestamp_ms: u32,
    vehicles: &[Creation],
) -> Result<Vec<u8>, &'static str> {
    encode_list(sequence, timestamp_ms, vehicles, |vehicle, bits| {
        vehicle.encode_fields(bits)
    })
}
/// Updates existing type1 objects only. No creation, active, model or source
/// relationship fields are emitted. Change order is retained; duplicate IDs and
/// unsupported/empty field masks reject the whole packet before publication.
pub(crate) fn encode_runtime_delta(
    catalog: &Catalog,
    runtime: &crate::acquisition_ground_runtime::Runtime,
    changes: &[(u16, u32)],
    sequence: u32,
    timestamp_ms: u32,
) -> Result<Vec<u8>, &'static str> {
    let allowed = (1 << 5) | (1 << 18) | (1 << 25) | (1 << 26);
    if changes.is_empty() || changes.len() > 32 {
        return Err("invalid_ground_vehicle_delta");
    }
    let mut masks = std::collections::BTreeMap::new();
    let mut vehicles = Vec::with_capacity(changes.len());
    for &(index, mask) in changes {
        if mask == 0 || mask & !allowed != 0 {
            return Err("unsupported_ground_vehicle_delta_fields");
        }
        if masks.insert(index, mask).is_some() {
            return Err("duplicate_ground_vehicle_delta");
        }
        let mut vehicle = catalog
            .vehicle(index)
            .ok_or("unknown_ground_vehicle")?
            .clone();
        let current = runtime
            .get(index)
            .ok_or("ground_vehicle_runtime_required")?;
        if current.key.index != index {
            return Err("ground_vehicle_runtime_identity_mismatch");
        }
        vehicle.position = current.pose.position;
        vehicle.rotation = current.pose.rotation;
        vehicle.initial_state = current.state;
        vehicle.health_quarter_points = current.health_quarter_points;
        vehicle.validate_runtime()?;
        vehicles.push(vehicle);
    }
    encode_list(sequence, timestamp_ms, &vehicles, |vehicle, bits| {
        let current = runtime
            .get(vehicle.index)
            .ok_or("ground_vehicle_runtime_required")?;
        vehicle.encode_masked_fields(
            bits,
            vehicle.damage(current.last_attacker),
            masks[&vehicle.index],
        )
    })
}
fn encode_list(
    sequence: u32,
    timestamp_ms: u32,
    vehicles: &[Creation],
    mut fields: impl FnMut(&Creation, &mut BitWriter) -> Result<(), &'static str>,
) -> Result<Vec<u8>, &'static str> {
    if sequence <= 1 || vehicles.is_empty() || vehicles.len() > 32 {
        return Err("invalid_ground_vehicle_list");
    }
    let profile = crate::wire_profile::WireProfileCatalog::embedded().profile(20231)?;
    let mut bits = BitWriter::with_profile(profile);
    UpdateHeader {
        sequence,
        nak_sequence: 0,
        client_index: 0,
        object_type_mask: 1 << 1,
        timestamp_ms,
    }
    .encode(&mut bits)?;
    let mut ids = BTreeSet::new();
    for vehicle in vehicles {
        if !ids.insert(vehicle.index) {
            return Err("duplicate_ground_vehicle");
        }
        fields(vehicle, &mut bits)?;
    }
    bits.write(0, 1)?;
    Ok(bits.finish())
}

#[cfg(test)]
#[path = "ground_vehicle_tests.rs"]
mod tests;
