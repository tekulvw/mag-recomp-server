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
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    spawn_catalog_sha256: String,
    native_elf_sha256: String,
    native_binding_sha256: String,
    vehicle_properties_sha256: String,
}
/// Explicit qualified external bindings. No default vehicles or native IDs.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    format: String,
    schema_version: u32,
    source: Source,
    vehicles: Vec<Creation>,
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
            if !ids.insert(v.index)
                || !points.insert(v.spawn_point_index)
                || point.token != v.spawn_point_token
                || !point.active
                || v.faction != faction
            {
                return Err("invalid_ground_vehicle_spawn_binding");
            }
        }
        Ok(())
    }
    pub(crate) fn batch_count(&self) -> usize {
        self.vehicles.len().div_ceil(16)
    }
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
        self.ground_vehicles = Some(catalog);
        Ok(self)
    }
}
impl Creation {
    pub fn object_id(&self) -> u32 {
        0x01000000 | u32::from(self.index)
    }
    fn validate(&self) -> Result<(), &'static str> {
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
            || !(1..4096).contains(&self.health_quarter_points)
            || self.rotation.iter().any(|v| !v.is_finite())
            || (norm - 1.).abs() > 0.002
        {
            return Err("invalid_ground_vehicle_creation");
        }
        self.position.validate()
    }
    fn encode_fields(&self, bits: &mut BitWriter) -> Result<(), &'static str> {
        self.validate()?;
        bits.write(1, 1)?;
        bits.write(self.index.into(), 16)?;
        for field in 0..30 {
            bits.write(
                u32::from([0, 1, 3, 4, 5, 6, 7, 18, 19, 20, 25, 26].contains(&field)),
                1,
            )?;
            match field {
                0 => bits.write(1, 1)?,
                1 => bits.write(0x105, 16)?,
                3 => bits.write(self.vehicle_type_token, 32)?,
                4 => bits.write(u32::from((self.modifier << 2) | self.faction), 8)?,
                5 => bits.write(2, 8)?,
                6 | 7 => bits.write(u16::MAX.into(), 16)?,
                18 => VehicleDamage {
                    last_location_and_type: 0,
                    health_quarter_points: self.health_quarter_points,
                    conditions: 0,
                    last_attacker: u32::MAX,
                    last_source_object: u32::MAX,
                    last_user_data: 0,
                }
                .encode(bits)?,
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
        vehicle.encode_fields(&mut bits)?;
    }
    bits.write(0, 1)?;
    Ok(bits.finish())
}

#[cfg(test)]
#[path = "ground_vehicle_tests.rs"]
mod tests;
