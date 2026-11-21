//! Retail type1 ground-vehicle creation from explicit external bindings.
use crate::hospital_turret_wire::{Fields, InventoryDefinition};
use crate::replication::{
    BitReader, BitWriter, CharacterPosition, ObjectUpdate, UpdateHeader, VehicleDamage,
};
use crate::wire_profile::WireProfile;
use serde::Deserialize;
use std::collections::BTreeSet;
use std::{
    fs,
    io::{self, Read},
    path::Path,
};

#[path = "acquisition_arrival_static.rs"]
pub mod arrival_static;

#[cfg(test)]
#[path = "ground_turret_wire_tests.rs"]
mod gunner_wire_tests;

pub(crate) const SERVER_OWNER: u16 = 0x105;
const VEHICLE_FIELDS: usize = 25;
const GROUND_FIELDS: usize = 30;

/// Sparse dynamics from a retail type1 owner report. Rotation components are
/// decoded at their wire precision. The authority layer must validate and
/// normalize a rotation before applying it to the simulation.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct OwnerDynamics {
    pub index: u16,
    pub position: Option<CharacterPosition>,
    pub rotation_xyzw: Option<[f32; 4]>,
    pub brake: Option<bool>,
    pub throttle: Option<f32>,
    pub smoke_screen: Option<bool>,
}

impl OwnerDynamics {
    pub(crate) fn has_changes(self) -> bool {
        self.position.is_some()
            || self.rotation_xyzw.is_some()
            || self.brake.is_some()
            || self.throttle.is_some()
            || self.smoke_screen.is_some()
    }

    pub(crate) fn encode_relay(
        self,
        profile: crate::wire_profile::WireProfile,
        timestamp_ms: u32,
    ) -> Result<Vec<u8>, &'static str> {
        let mut bits = BitWriter::with_profile(profile);
        UpdateHeader {
            sequence: 1,
            nak_sequence: 0,
            client_index: 0,
            object_type_mask: 1 << 1,
            timestamp_ms,
        }
        .encode(&mut bits)?;
        bits.write(1, 1)?;
        bits.write(self.index.into(), 16)?;
        bits.write(0, 25)?;
        bits.write(u32::from(self.position.is_some()), 1)?;
        if let Some(position) = self.position {
            position.encode(&mut bits)?;
        }
        bits.write(u32::from(self.rotation_xyzw.is_some()), 1)?;
        if let Some(quaternion) = self.rotation_xyzw {
            for value in quaternion {
                write_signed_fixed(&mut bits, value, 10)?;
            }
        }
        write_optional_bool(&mut bits, self.brake)?;
        bits.write(u32::from(self.throttle.is_some()), 1)?;
        if let Some(throttle) = self.throttle {
            write_signed_fixed(&mut bits, throttle, 9)?;
        }
        write_optional_bool(&mut bits, self.smoke_screen)?;
        bits.write(0, 1)?;
        Ok(bits.finish())
    }

    /// Decode a standalone type1 report for offline qualification.
    pub fn decode(
        update: &ObjectUpdate<'_>,
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Vec<Self>, &'static str> {
        update.validate_profile(profile)?;
        if update.header.object_type_mask != 1 << 1 {
            return Err("expected_ground_vehicle_owner_list");
        }
        let mut bits = BitReader::with_profile(update.wire, profile);
        bits.position = profile.header_bits();
        let reports = Self::decode_list(&mut bits)?;
        let remaining = update.wire.len() * 8 - bits.position;
        if remaining >= 8 || bits.read(remaining)? != 0 {
            return Err("trailing_client_update_bits");
        }
        Ok(reports)
    }

    pub(crate) fn decode_list(bits: &mut BitReader<'_>) -> Result<Vec<Self>, &'static str> {
        Ok(decode_owner_list(bits, false)?
            .into_iter()
            .map(|report| report.dynamics)
            .collect())
    }

    fn read_fields(index: u16, bits: &mut BitReader<'_>) -> Result<Self, &'static str> {
        let mut report = Self {
            index,
            ..Self::default()
        };
        if bits.read(1)? != 0 {
            let x = bits.read(16)? as f32 / 32.;
            let y = read_signed_fixed(bits, 16, 32.)?;
            let z = bits.read(16)? as f32 / 32.;
            report.position = Some(CharacterPosition { x, y, z });
        }
        if bits.read(1)? != 0 {
            let mut rotation = [0.; 4];
            for value in &mut rotation {
                *value = read_signed_fixed(bits, 10, 256.)?;
            }
            report.rotation_xyzw = Some(rotation);
        }
        if bits.read(1)? != 0 {
            report.brake = Some(bits.read(1)? != 0);
        }
        if bits.read(1)? != 0 {
            report.throttle = Some(read_signed_fixed(bits, 9, 256.)?);
        }
        if bits.read(1)? != 0 {
            report.smoke_screen = Some(bits.read(1)? != 0);
        }
        Ok(report)
    }
}

/// One Ground object and its independent driver and turret components.
#[derive(Clone, Copy, Debug)]
pub(crate) struct OwnerUpdate {
    pub dynamics: OwnerDynamics,
    pub turret: Fields,
}

pub(crate) fn decode_owner_updates(
    bits: &mut BitReader<'_>,
) -> Result<Vec<OwnerUpdate>, &'static str> {
    decode_owner_list(bits, true)
}

fn decode_owner_list(
    bits: &mut BitReader<'_>,
    allow_turret: bool,
) -> Result<Vec<OwnerUpdate>, &'static str> {
    if bits.profile.application_id() != 20231 {
        return Err("unsupported_client_object_type");
    }
    let mut reports = Vec::new();
    let mut seen = 0;
    while bits.read(1)? != 0 {
        let index = bits.read(16)? as u16;
        claim_owner_index(index, &mut seen)?;
        let mut turret = Fields::default();
        if allow_turret {
            for field in 0..VEHICLE_FIELDS {
                if bits.read(1)? != 0 && !turret.read_present(field, bits)? {
                    return Err("unsupported_client_ground_vehicle_field");
                }
            }
        } else if bits.read(VEHICLE_FIELDS)? != 0 {
            return Err("unsupported_client_ground_vehicle_field");
        }
        reports.push(OwnerUpdate {
            dynamics: OwnerDynamics::read_fields(index, bits)?,
            turret,
        });
    }
    Ok(reports)
}

fn claim_owner_index(index: u16, seen: &mut u32) -> Result<(), &'static str> {
    let id = 1u32
        .checked_shl(index.into())
        .ok_or("ground_vehicle_owner_index_out_of_range")?;
    if *seen & id != 0 {
        return Err("duplicate_ground_vehicle_owner_update");
    }
    *seen |= id;
    Ok(())
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct TurretUpdate {
    pub index: u16,
    pub driver_owner: Option<u16>,
    pub turret_owner: Option<u16>,
    pub definition: Option<InventoryDefinition>,
    pub fields: Fields,
}

impl TurretUpdate {
    /// Write one Ground row. The containing update terminates its family list.
    pub(crate) fn write_row(self, bits: &mut BitWriter) -> Result<(), &'static str> {
        if bits.profile.application_id() != 20231 {
            return Err("unsupported_ground_turret_wire_profile");
        }
        claim_owner_index(self.index, &mut 0)?;
        bits.write(1, 1)?;
        bits.write(self.index.into(), 16)?;
        for field in 0..GROUND_FIELDS {
            if !self.write_field(field, bits)? {
                bits.write(0, 1)?;
            }
        }
        Ok(())
    }

    /// Write only a turret or owner field when assembling a complete Ground row.
    pub(crate) fn write_field(
        self,
        field: usize,
        bits: &mut BitWriter,
    ) -> Result<bool, &'static str> {
        let owner = match field {
            1 => self.driver_owner,
            6 => self.turret_owner,
            _ => return self.fields.write_field(field, self.definition, bits),
        };
        bits.write(u32::from(owner.is_some()), 1)?;
        if let Some(owner) = owner {
            bits.write(owner.into(), 16)?;
        }
        Ok(true)
    }

    /// At most 32 complete sparse rows fit into one retail replication packet.
    pub(crate) fn encode_rows(
        sequence: u32,
        timestamp_ms: u32,
        rows: &[Self],
        profile: WireProfile,
    ) -> Result<Vec<Vec<u8>>, &'static str> {
        if profile.application_id() != 20231 {
            return Err("unsupported_ground_turret_wire_profile");
        }
        if sequence == 0 {
            return Err("invalid_ground_turret_sequence");
        }
        if rows.is_empty() {
            return Ok(Vec::new());
        }
        let mut bits = BitWriter::with_profile(profile);
        UpdateHeader {
            sequence,
            nak_sequence: 0,
            client_index: 0,
            object_type_mask: 1 << 1,
            timestamp_ms,
        }
        .encode(&mut bits)?;
        let mut seen = 0;
        for row in rows {
            claim_owner_index(row.index, &mut seen)?;
            row.write_row(&mut bits)?;
        }
        bits.write(0, 1)?;
        Ok(vec![bits.finish()])
    }
}

pub(crate) fn write_owner_field(
    bits: &mut BitWriter,
    index: u16,
    owner: u16,
) -> Result<(), &'static str> {
    bits.write(1, 1)?;
    bits.write(index.into(), 16)?;
    bits.write(1, 2)?; // Active field absent, owner field present.
    bits.write(owner.into(), 16)?;
    bits.write(0, 29) // Remaining 28 fields and end of type1 list.
}

pub(crate) fn encode_owner_field(
    profile: crate::wire_profile::WireProfile,
    timestamp_ms: u32,
    index: u16,
    owner: u16,
) -> Result<Vec<u8>, &'static str> {
    let mut bits = BitWriter::with_profile(profile);
    UpdateHeader {
        sequence: 1,
        nak_sequence: 0,
        client_index: 0,
        object_type_mask: 1 << 1,
        timestamp_ms,
    }
    .encode(&mut bits)?;
    write_owner_field(&mut bits, index, owner)?;
    Ok(bits.finish())
}

fn write_signed_fixed(
    bits: &mut BitWriter,
    value: f32,
    magnitude_bits: usize,
) -> Result<(), &'static str> {
    bits.write(u32::from(value.is_sign_negative()), 1)?;
    bits.write((value.abs() * 256.) as u32, magnitude_bits)
}

fn write_optional_bool(bits: &mut BitWriter, value: Option<bool>) -> Result<(), &'static str> {
    bits.write(u32::from(value.is_some()), 1)?;
    if let Some(value) = value {
        bits.write(u32::from(value), 1)?;
    }
    Ok(())
}

fn read_signed_fixed(
    bits: &mut BitReader<'_>,
    magnitude_bits: usize,
    scale: f32,
) -> Result<f32, &'static str> {
    let negative = bits.read(1)? != 0;
    let magnitude = bits.read(magnitude_bits)? as f32 / scale;
    Ok(if negative { -magnitude } else { magnitude })
}

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
/// Operator seat and movement budget for a source-qualified Ground vehicle.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DriverSeat {
    pub vehicle_index: u16,
    pub seat: u8,
    pub maximum_speed_mps: f32,
}

/// Full visibility inventory for the seats this profile admits.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SeatVisibility {
    archive_sha256: String,
    registry_member_sha256: String,
    vehicles: Vec<VehicleSeatVisibility>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct VehicleSeatVisibility {
    vehicle_index: u16,
    properties_member_sha256: String,
    seats: Vec<OccupantVisibility>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OccupantVisibility {
    seat: u8,
    occupant_visible: bool,
}

/// Explicit qualified external bindings. No default vehicles or native IDs.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    #[serde(skip)]
    raw_sha256: String,
    #[serde(skip)]
    turret_policies: std::collections::BTreeMap<u16, crate::ground_turrets::Policy>,
    format: String,
    schema_version: u32,
    source: Source,
    vehicles: Vec<Creation>,
    #[serde(default)]
    passenger_spawns: Vec<PassengerSpawn>,
    #[serde(default)]
    vehicle_sources: Vec<VehicleSource>,
    #[serde(default)]
    driver_seats: Vec<DriverSeat>,
    #[serde(default)]
    seat_visibility: Option<SeatVisibility>,
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
        use sha2::Digest;
        value.raw_sha256 = format!("{:x}", sha2::Sha256::digest(text.as_bytes()));
        value.vehicles.sort_by_key(|v| v.index);
        Ok(value)
    }
    pub(crate) fn has_turrets(&self) -> bool {
        !self.turret_policies.is_empty()
    }

    pub(crate) fn turret_policy(&self, index: u16) -> Option<crate::ground_turrets::Policy> {
        self.turret_policies.get(&index).copied()
    }

    pub(crate) fn set_turret_policies(
        &mut self,
        policies: std::collections::BTreeMap<u16, crate::ground_turrets::Policy>,
    ) {
        self.turret_policies = policies;
    }

    pub(crate) fn raw_sha256(&self) -> &str {
        &self.raw_sha256
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
        let domination = policy.game_type == 2;
        let scope = self.source.creation_scope.as_deref();
        let domination_scope = scope == Some("native_domination_vehicle_source_points");
        if domination != domination_scope || (!domination && policy.game_type != 1) {
            return Err("ground_vehicles_require_matching_profile_scope");
        }
        let domination_drivers = domination && !self.driver_seats.is_empty();
        if domination
            && (self.vehicles.len() != 16
                || self.vehicle_sources.len() != 16
                || self.passenger_spawns.len() != 8
                || (domination_drivers && self.driver_seats.len() != 8))
        {
            return Err("incomplete_domination_ground_vehicle_population");
        }
        if scope.is_some_and(|v| {
            !matches!(
                v,
                "native_requested_free_slot_hidden_state0"
                    | "native_vehicle_source_points"
                    | "native_domination_vehicle_source_points"
            )
        }) {
            return Err("invalid_ground_vehicle_creation_scope");
        }
        let mut source_points = std::collections::BTreeMap::new();
        if !self.vehicle_sources.is_empty()
            && !matches!(
                scope,
                Some("native_vehicle_source_points" | "native_domination_vehicle_source_points")
            )
        {
            return Err("vehicle_source_qualification_required");
        }
        for source in &self.vehicle_sources {
            let vehicle = self
                .vehicle(source.vehicle_index)
                .ok_or("unknown_vehicle_source_identity")?;
            let source_kind_valid = if domination {
                source.authored_active
                    && match (source.spawn_type, source.user_class, source.team) {
                        (3, 0, 0) => {
                            self.has_passenger_spawn_binding(vehicle)
                                && spawns.point(source.spawn_point_index).is_some_and(|point| {
                                    point.active
                                        && matches!(
                                            point.kind,
                                            crate::spawn_catalog::SpawnKind::VehiclePassenger
                                        )
                                })
                        }
                        (7, 1, 1) => spawns.point(source.spawn_point_index).is_some_and(|point| {
                            point.active
                                && matches!(
                                    point.kind,
                                    crate::spawn_catalog::SpawnKind::Unsupported
                                )
                        }),
                        _ => false,
                    }
            } else {
                match (
                    source.vehicle_index < 4,
                    source.spawn_type,
                    source.user_class,
                ) {
                    (true, 3, 0) => {
                        source.team == 0
                            && source.authored_active
                            && self.is_existing_apc_source(vehicle)
                            && spawns.point(source.spawn_point_index).is_some_and(|point| {
                                point.active
                                    && matches!(
                                        point.kind,
                                        crate::spawn_catalog::SpawnKind::VehiclePassenger
                                    )
                            })
                    }
                    (false, 7 | 9, 1) => true,
                    _ => false,
                }
            };
            if !source_kind_valid
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
        if domination
            && (source_points
                .values()
                .filter(|source| source.spawn_type == 3)
                .count()
                != 8
                || source_points.keys().copied().ne(0..16)
                || source_points
                    .values()
                    .zip(source_points.values().skip(1))
                    .any(|(before, after)| before.spawn_point_index >= after.spawn_point_index))
        {
            return Err("invalid_domination_ground_vehicle_load_order");
        }
        let mut drivers = BTreeSet::new();
        let driver_spawn_type = if domination { 3 } else { 9 };
        for driver in &self.driver_seats {
            let vehicle = self
                .vehicle(driver.vehicle_index)
                .ok_or("unknown_ground_driver_vehicle")?;
            if !drivers.insert(driver.vehicle_index)
                || driver.seat >= 16
                || (domination && driver.seat != 0)
                || !driver.maximum_speed_mps.is_finite()
                || driver.maximum_speed_mps <= 0.
                || vehicle.passenger_seats.contains(&driver.seat)
                || vehicle.maximum_mount_distance_m.is_none()
                || vehicle.maximum_mount_position_age_ms.is_none()
                || vehicle.exit_policy().is_none()
                || source_points
                    .get(&driver.vehicle_index)
                    .is_none_or(|s| s.spawn_type != driver_spawn_type)
            {
                return Err("invalid_ground_driver_policy");
            }
        }
        let mut ids = BTreeSet::new();
        let mut points = BTreeSet::new();
        for v in &self.vehicles {
            v.validate(drivers.contains(&v.index))?;
            let point = spawns
                .point(v.spawn_point_index)
                .ok_or("unknown_ground_vehicle_spawn")?;
            let faction = if v.team == 0 {
                policy.attacking_faction
            } else {
                policy.defending_faction
            };
            let source = source_points.get(&v.index);
            if domination_drivers {
                let invalid_seats = if drivers.contains(&v.index) {
                    v.passenger_seats.iter().copied().ne(2..=9) || v.maximum_occupancy != Some(8)
                } else {
                    !v.passenger_seats.is_empty()
                };
                if invalid_seats {
                    return Err("invalid_domination_ground_driver_passenger_policy");
                }
            }
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
        if let Some(visibility) = &self.seat_visibility {
            self.validate_seat_visibility(visibility)?;
        }
        Ok(())
    }
    fn validate_seat_visibility(&self, visibility: &SeatVisibility) -> Result<(), &'static str> {
        let valid_hash =
            |hash: &str| hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit());
        if !matches!(
            self.source.creation_scope.as_deref(),
            Some("native_vehicle_source_points" | "native_domination_vehicle_source_points")
        ) || !valid_hash(&visibility.archive_sha256)
            || !valid_hash(&visibility.registry_member_sha256)
        {
            return Err("invalid_ground_seat_visibility_source");
        }
        let expected: BTreeSet<_> = self
            .vehicles
            .iter()
            .flat_map(|vehicle| {
                vehicle
                    .passenger_seats
                    .iter()
                    .copied()
                    .chain(self.driver_seat(vehicle.index).map(|driver| driver.seat))
                    .map(|seat| (vehicle.index, seat))
            })
            .collect();
        let mut vehicles = BTreeSet::new();
        let mut seats = BTreeSet::new();
        for row in &visibility.vehicles {
            if !vehicles.insert(row.vehicle_index)
                || !valid_hash(&row.properties_member_sha256)
                || row.seats.is_empty()
                || row
                    .seats
                    .iter()
                    .any(|seat| !seats.insert((row.vehicle_index, seat.seat)))
            {
                return Err("invalid_ground_seat_visibility_inventory");
            }
        }
        if expected.is_empty() || seats != expected {
            return Err("invalid_ground_seat_visibility_inventory");
        }
        Ok(())
    }

    pub(crate) fn has_qualified_mount_visibility(&self) -> bool {
        // When present, the validated inventory covers every admitted seat.
        self.seat_visibility.is_some()
            || (self.driver_seats.is_empty()
                && self
                    .vehicles
                    .iter()
                    .all(|vehicle| vehicle.passenger_seats.is_empty()))
    }

    pub(crate) fn seat_occupant_visible(&self, vehicle_index: u16, seat: u8) -> Option<bool> {
        if seat == 1 && self.turret_policy(vehicle_index).is_some() {
            return Some(false);
        }
        self.seat_visibility
            .as_ref()?
            .vehicles
            .iter()
            .find(|row| row.vehicle_index == vehicle_index)?
            .seats
            .iter()
            .find(|row| row.seat == seat)
            .map(|row| row.occupant_visible)
    }

    pub(crate) fn driver_seat(&self, index: u16) -> Option<DriverSeat> {
        self.driver_seats
            .iter()
            .copied()
            .find(|seat| seat.vehicle_index == index)
    }

    pub(crate) fn vehicle_source(&self, index: u16) -> Option<&VehicleSource> {
        self.vehicle_sources
            .iter()
            .find(|source| source.vehicle_index == index)
    }
    fn is_existing_apc_source(&self, creation: &Creation) -> bool {
        creation.index < 4
            && creation.team == 0
            && creation.initial_state == 2
            && self.has_passenger_spawn_binding(creation)
    }

    fn has_passenger_spawn_binding(&self, creation: &Creation) -> bool {
        self.passenger_spawns.iter().any(|binding| {
            binding.vehicle_index == creation.index
                && binding.spawn_point_index == creation.spawn_point_index
                && binding.spawn_point_token == creation.spawn_point_token
        })
    }
    pub(crate) fn source_spawn_type(&self, creation: &Creation) -> Option<u8> {
        self.vehicle_source(creation.index)
            .map(|source| source.spawn_type)
            // A legacy source catalog may qualify the exact APC point through
            // its validated passenger binding without a vehicle_sources row.
            .or_else(|| self.is_existing_apc_source(creation).then_some(3))
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
    pub(crate) fn ground_driver_enabled(&self) -> bool {
        self.ground_vehicles
            .as_ref()
            .is_some_and(|catalog| !catalog.driver_seats.is_empty())
    }

    pub(crate) fn ground_driver_seat(&self, index: u16) -> Option<DriverSeat> {
        self.ground_vehicles.as_ref()?.driver_seat(index)
    }

    pub(crate) fn ground_driver_seat_index(&self, actor: &crate::Active) -> Option<u16> {
        let seat = actor.ground_seat?;
        (self.ground_driver_seat(seat.vehicle_index)?.seat == seat.seat)
            .then_some(seat.vehicle_index)
    }

    pub(crate) fn active_ground_drivers(&self) -> Vec<(u16, u16)> {
        self.peers
            .values()
            .chain(self.active.iter())
            .filter_map(|actor| {
                let index = self.ground_driver_seat_index(actor)?;
                self.ground_driver_authority(actor, index)
                    .then_some((index, actor.reservation.player))
            })
            .collect()
    }

    pub(crate) fn queue_ground_driver_revocation(
        &mut self,
        index: u16,
        now: std::time::Duration,
    ) -> Result<(), &'static str> {
        let packet = encode_owner_field(
            self.wire_profile,
            now.as_millis() as u32,
            index,
            SERVER_OWNER,
        )?;
        for recipient in self
            .peers
            .values_mut()
            .chain(self.active.iter_mut())
            .filter(|actor| actor.initial_objects.is_some())
        {
            recipient.queue_shared(packet.clone());
        }
        Ok(())
    }

    pub(crate) fn ground_driver_authority(&self, actor: &crate::Active, index: u16) -> bool {
        actor.life.is_alive()
            && actor.entered_game
            && actor.ready_for_updates
            && self.ground_driver_seat_index(actor) == Some(index)
            && self.ground_vehicle_is_operational(index)
    }

    pub(crate) fn ground_driver_owner(&self, index: u16) -> Option<u16> {
        self.peers
            .values()
            .chain(self.active.iter())
            .find(|actor| self.ground_driver_authority(actor, index))
            .map(|actor| actor.reservation.player)
    }

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
        let packet = encode_list(sequence, timestamp_ms, &vehicles, |vehicle, bits| {
            vehicle.validate_runtime()?;
            let current = runtime
                .get(vehicle.index)
                .ok_or("ground_vehicle_runtime_required")?;
            vehicle.encode_fields_with_damage(
                bits,
                current.damage_wire(),
                self.ground_driver_owner(vehicle.index)
                    .unwrap_or(SERVER_OWNER),
                current.driver_controls,
                self.ground_turret_snapshot(vehicle.index),
            )
        })?;
        if packet.len() > 1397 {
            return Err("ground_creation_packet_exceeds_budget");
        }
        Ok(packet)
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
    fn validate(&self, driver_bound: bool) -> Result<(), &'static str> {
        self.validate_common()?;
        if !matches!(self.initial_state, 0 | 2)
            || (self.initial_state == 0
                && (self.index < 4 || (!driver_bound && !self.passenger_seats.is_empty())))
            || self.health_quarter_points == 0
        {
            return Err("invalid_ground_vehicle_creation");
        }
        Ok(())
    }
    fn validate_runtime(&self) -> Result<(), &'static str> {
        self.validate_common()?;
        if !matches!(self.initial_state, 0 | 2 | 3 | 4 | 5 | 6 | 7 | 8) {
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
        self.validate(false)?;
        self.encode_fields_with_damage(
            bits,
            self.damage(u32::MAX),
            SERVER_OWNER,
            Default::default(),
            None,
        )
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
        owner: u16,
        controls: crate::acquisition_ground_runtime::DriverControls,
        turret: Option<TurretUpdate>,
    ) -> Result<(), &'static str> {
        let fields = [0, 1, 3, 4, 5, 6, 7, 18, 19, 20, 25, 26]
            .into_iter()
            .fold(0, |mask, field| mask | (1 << field));
        let fields = fields
            | (u32::from(controls.brake.is_some()) << 27)
            | (u32::from(controls.throttle.is_some()) << 28)
            | (u32::from(controls.smoke_screen.is_some()) << 29);
        self.encode_masked_fields(bits, damage, fields, owner, controls, turret)
    }
    fn encode_masked_fields(
        &self,
        bits: &mut BitWriter,
        damage: VehicleDamage,
        mask: u32,
        owner: u16,
        controls: crate::acquisition_ground_runtime::DriverControls,
        turret: Option<TurretUpdate>,
    ) -> Result<(), &'static str> {
        bits.write(1, 1)?;
        bits.write(self.index.into(), 16)?;
        for field in 0..30 {
            if field != 1
                && let Some(turret) = turret
                && turret.write_field(field, bits)?
            {
                continue;
            }
            let present = mask & (1 << field) != 0;
            bits.write(u32::from(present), 1)?;
            if !present {
                continue;
            }
            match field {
                0 => bits.write(1, 1)?,
                1 => bits.write(owner.into(), 16)?,
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
                27 => bits.write(u32::from(controls.brake.ok_or("missing_ground_brake")?), 1)?,
                28 => write_signed_fixed(
                    bits,
                    controls.throttle.ok_or("missing_ground_throttle")?,
                    9,
                )?,
                29 => bits.write(
                    u32::from(controls.smoke_screen.ok_or("missing_ground_smoke")?),
                    1,
                )?,
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
    let allowed = (1 << 1) | (1 << 5) | (1 << 6) | (1 << 18) | (1 << 25) | (1 << 26);
    if changes.is_empty() || changes.len() > 32 {
        return Err("invalid_ground_vehicle_delta");
    }
    let mut masks = std::collections::BTreeMap::new();
    let mut vehicles = Vec::with_capacity(changes.len());
    for &(index, mask) in changes {
        if mask == 0
            || mask & !allowed != 0
            || (mask & (1 << 6) != 0 && catalog.turret_policy(index).is_none())
            || (mask & (1 << 1) != 0
                && !catalog
                    .driver_seats
                    .iter()
                    .any(|driver| driver.vehicle_index == index))
        {
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
            current.damage_wire(),
            masks[&vehicle.index],
            SERVER_OWNER,
            Default::default(),
            None,
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
