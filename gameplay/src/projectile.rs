//! Native grenade projectile replication. Evidence: heal-grenade-armor and spawn-wire.
//! Types16..18 reconcile with the client's already predicted projectiles; their
//! Object::Position and Object::Rotation fields have49 and44 bits respectively.
use crate::item_catalog::{self, ItemCatalog};
use crate::replication::{BitReader, BitWriter, CharacterPosition, ObjectUpdate, UpdateHeader};
use serde::Serialize;
/// Age allowance is local admission policy, separate from authored lifetime.
pub const PROJECTILE_AGE_GRACE_MS: u64 = 2000;
pub const PROJECTILE_TYPE_MASK: u32 = (1 << 16) | (1 << 17) | (1 << 18);
#[derive(Clone, Copy, Debug)]
pub struct ProjectileCreation {
    pub slot: u8,
    pub object_index: u16,
    pub owner_index: u16,
    pub ammo_token: u32,
    pub position: CharacterPosition,
    pub rotation_xyzw: [f32; 4],
    pub shooter_id: u32,
}
fn position_encode(position: CharacterPosition, bits: &mut BitWriter) -> Result<(), &'static str> {
    for (value, signed) in [(position.x, false), (position.y, true), (position.z, false)] {
        if !value.is_finite() || value.abs() >= 2048.0 || (!signed && value < 0.0) {
            return Err("projectile_position_out_of_range");
        }
        if signed {
            bits.write(u32::from(value < 0.0), 1)?;
        }
        let magnitude = value.abs();
        let integer = magnitude as u32;
        bits.write(integer, 11)?;
        bits.write(((magnitude - integer as f32) * 32.0) as u32, 5)?;
    }
    Ok(())
}
fn normalize(rotation: [f32; 4]) -> Result<[f32; 4], &'static str> {
    if rotation.iter().any(|v| !v.is_finite()) {
        return Err("invalid_projectile_rotation");
    }
    let length = rotation.iter().map(|v| v * v).sum::<f32>().sqrt();
    if !length.is_finite() || length < 1e-6 {
        return Err("invalid_projectile_rotation");
    }
    Ok(rotation.map(|v| v / length))
}
fn rotation_encode(rotation: [f32; 4], bits: &mut BitWriter) -> Result<(), &'static str> {
    for value in normalize(rotation)? {
        bits.write(u32::from(value < 0.0), 1)?;
        let magnitude = value.abs();
        let integer = magnitude as u32;
        bits.write(integer, 2)?;
        bits.write(((magnitude - integer as f32) * 256.0) as u32, 8)?;
    }
    Ok(())
}
impl ProjectileCreation {
    pub fn encode(&self, header: UpdateHeader) -> Result<Vec<u8>, &'static str> {
        self.encode_with_profile(header, crate::wire_profile::WireProfile::prototype())
    }
    pub fn encode_with_profile(
        &self,
        header: UpdateHeader,
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        if self.slot >= 3 || header.object_type_mask != 1u32 << (16 + self.slot) {
            return Err("invalid_projectile_type");
        }
        let mut bits = BitWriter::with_profile(profile);
        header.encode(&mut bits)?;
        bits.write(1, 1)?;
        bits.write(self.object_index.into(), 16)?;
        for field in 0..8 {
            bits.write(u32::from([0, 1, 4, 5, 6, 7].contains(&field)), 1)?;
            match field {
                0 => bits.write(1, 1)?,
                1 => bits.write(self.owner_index.into(), 16)?,
                // Constructor-default updateRate/state stay absent until changed.
                4 => bits.write(self.ammo_token, 32)?,
                5 => position_encode(self.position, &mut bits)?,
                6 => rotation_encode(self.rotation_xyzw, &mut bits)?,
                7 => bits.write(self.shooter_id, 32)?,
                _ => (),
            }
        }
        bits.write(0, 1)?;
        Ok(bits.finish())
    }
}
#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct ProjectileOwnerFields {
    pub state: Option<u8>,
    pub position: Option<CharacterPosition>,
    pub rotation_xyzw: Option<[f32; 4]>,
}
#[derive(Clone, Copy, Debug, Serialize)]
pub struct ProjectileOwnerUpdate {
    pub slot: u8,
    pub object_index: u16,
    pub fields: ProjectileOwnerFields,
}
impl ProjectileOwnerFields {
    pub(crate) fn encode_fields(&self, bits: &mut BitWriter) -> Result<(), &'static str> {
        if self.state.is_some_and(|state| state > 7) {
            return Err("invalid_projectile_state");
        }
        for field in 0..8 {
            let present = match field {
                3 => self.state.is_some(),
                5 => self.position.is_some(),
                6 => self.rotation_xyzw.is_some(),
                _ => false,
            };
            bits.write(u32::from(present), 1)?;
            match field {
                3 if present => bits.write(self.state.unwrap().into(), 8)?,
                5 if present => position_encode(self.position.unwrap(), bits)?,
                6 if present => rotation_encode(self.rotation_xyzw.unwrap(), bits)?,
                _ => (),
            }
        }
        Ok(())
    }
}
/// Decode one object's eight interleaved flags after its list marker and index.
/// Do not mutate ownership/lifecycle until the aggregate packet is fully decoded.
pub(crate) fn decode_owner_fields(
    bits: &mut BitReader<'_>,
) -> Result<ProjectileOwnerFields, &'static str> {
    let mut result = ProjectileOwnerFields::default();
    for field in 0..8 {
        if bits.read(1)? == 0 {
            continue;
        }
        match field {
            3 => {
                let state = bits.read(8)? as u8;
                if state > 7 {
                    return Err("invalid_projectile_state");
                }
                result.state = Some(state);
            }
            5 => {
                let x = bits.read(16)? as f32 / 32.0;
                let negative = bits.read(1)? != 0;
                let y = bits.read(16)? as f32 / 32.0;
                let z = bits.read(16)? as f32 / 32.0;
                result.position = Some(CharacterPosition {
                    x,
                    y: if negative { -y } else { y },
                    z,
                });
            }
            6 => {
                let mut rotation = [0.0; 4];
                for value in &mut rotation {
                    let negative = bits.read(1)? != 0;
                    let magnitude = bits.read(2)? as f32 + bits.read(8)? as f32 / 256.0;
                    *value = if negative { -magnitude } else { magnitude };
                }
                result.rotation_xyzw = Some(normalize(rotation)?);
            }
            _ => return Err("unsupported_client_projectile_field"),
        }
    }
    Ok(result)
}
/// Standalone projectile-body decoder. Mixed Character/projectile bodies use
/// the same entry codec in the aggregate owner-update decoder.
pub fn decode_projectile_updates(
    update: &ObjectUpdate<'_>,
) -> Result<Vec<ProjectileOwnerUpdate>, &'static str> {
    decode_projectile_updates_with_profile(update, crate::wire_profile::WireProfile::prototype())
}
pub fn decode_projectile_updates_with_profile(
    update: &ObjectUpdate<'_>,
    profile: crate::wire_profile::WireProfile,
) -> Result<Vec<ProjectileOwnerUpdate>, &'static str> {
    update.validate_profile(profile)?;
    if update.header.object_type_mask & !PROJECTILE_TYPE_MASK != 0 {
        return Err("unsupported_client_object_type");
    }
    let mut bits = BitReader::with_profile(update.wire, profile);
    bits.position = profile.header_bits();
    let mut entries = Vec::new();
    for slot in 0..3 {
        if update.header.object_type_mask & (1 << (16 + slot)) == 0 {
            continue;
        }
        while bits.read(1)? != 0 {
            let object_index = bits.read(16)? as u16;
            let fields = decode_owner_fields(&mut bits)?;
            entries.push(ProjectileOwnerUpdate {
                slot,
                object_index,
                fields,
            });
        }
    }
    let remaining = update.wire.len() * 8 - bits.position;
    if remaining >= 8 || bits.read(remaining)? != 0 {
        return Err("trailing_client_update_bits");
    }
    Ok(entries)
}

/// Local admission results. The request has no independent nonce: an identical
/// active/completed request remains a duplicate until its configured slot expiry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum LaunchDisposition {
    Created { generation: u32 },
    Duplicate { generation: u32 },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectileTerminalReason {
    ReportedDetonation,
    LifetimeExpired,
}
#[derive(Clone, Copy, Debug, Serialize)]
pub struct ProjectileTerminal {
    pub item_id: u16,
    pub slot: u8,
    pub generation: u32,
    pub position: Option<CharacterPosition>,
    pub reason: ProjectileTerminalReason,
}
#[derive(Clone, Debug)]
pub struct ProjectileFlight {
    pub item_id: u16,
    pub generation: u32,
    pub life_generation: u32,
    pub launched_at_ms: u64,
    pub expiry_after_ms: u64,
    pub fuse_ms: u64,
    pub accepted_owner_sequence: u32,
    pub last_position: Option<CharacterPosition>,
    pub terminal_reported: bool,
    pub completed: bool,
    fingerprint: Vec<u8>,
}
#[derive(Clone, Default)]
pub struct ProjectileFlights {
    slots: [Option<ProjectileFlight>; 3],
    next_generation: u32,
}
impl ProjectileFlights {
    pub fn get(&self, slot: u8) -> Option<&ProjectileFlight> {
        self.slots.get(usize::from(slot)).and_then(Option::as_ref)
    }
    /// Admit launch metadata only. Caller validates owned shooter, kit and ammo,
    /// emits the typed creation, and consumes ammo only on Created.
    pub fn admit(
        &mut self,
        slot: u8,
        life: u32,
        now: u64,
        floor: u32,
        fingerprint: &[u8],
    ) -> Result<LaunchDisposition, &'static str> {
        self.admit_with_catalog(
            slot,
            life,
            now,
            floor,
            fingerprint,
            item_catalog::bundled()?,
        )
    }
    pub fn admit_with_catalog(
        &mut self,
        slot: u8,
        life: u32,
        now: u64,
        floor: u32,
        fingerprint: &[u8],
        catalog: &ItemCatalog,
    ) -> Result<LaunchDisposition, &'static str> {
        self.admit_item_with_catalog(
            slot,
            life,
            now,
            floor,
            fingerprint,
            catalog.default_grenade_item()?,
            catalog,
        )
    }
    #[allow(clippy::too_many_arguments)]
    pub fn admit_item_with_catalog(
        &mut self,
        slot: u8,
        life: u32,
        now: u64,
        floor: u32,
        fingerprint: &[u8],
        item_id: u16,
        catalog: &ItemCatalog,
    ) -> Result<LaunchDisposition, &'static str> {
        let parameters = catalog.grenade(item_id)?;
        let expiry_after_ms = parameters
            .authored_lifetime_ms
            .checked_add(catalog.grenade_expiry_grace_ms())
            .ok_or("invalid_projectile_lifetime")?;
        if slot >= 3 || fingerprint.len() != 57 {
            return Err("invalid_projectile_launch");
        }
        if let Some(previous) = &self.slots[usize::from(slot)]
            && previous.life_generation == life
            && now.saturating_sub(previous.launched_at_ms) < previous.expiry_after_ms
        {
            if previous.fingerprint == fingerprint && previous.item_id == item_id {
                return Ok(LaunchDisposition::Duplicate {
                    generation: previous.generation,
                });
            }
            if !previous.completed {
                return Err("projectile_slot_in_use");
            }
        }
        let generation = self
            .next_generation
            .checked_add(1)
            .ok_or("projectile_generation_exhausted")?;
        self.next_generation = generation;
        self.slots[usize::from(slot)] = Some(ProjectileFlight {
            item_id,
            generation,
            life_generation: life,
            launched_at_ms: now,
            expiry_after_ms,
            fuse_ms: parameters.fuse_ms,
            accepted_owner_sequence: floor,
            last_position: None,
            terminal_reported: false,
            completed: false,
            fingerprint: fingerprint.to_vec(),
        });
        Ok(LaunchDisposition::Created { generation })
    }
    /// Owner data remains client-reported physics; caller checks the authenticated
    /// index and validates the whole packet before calling this method.
    pub fn accept_owner(
        &mut self,
        slot: u8,
        life: u32,
        seq: u32,
        now: u64,
        fields: ProjectileOwnerFields,
    ) -> Result<bool, &'static str> {
        let flight = self
            .slots
            .get_mut(usize::from(slot))
            .and_then(Option::as_mut)
            .ok_or("unadmitted_projectile_update")?;
        if flight.life_generation != life {
            return Err("stale_projectile_life");
        }
        if now < flight.launched_at_ms || now - flight.launched_at_ms >= flight.expiry_after_ms {
            return Err("expired_projectile_update");
        }
        if flight.completed || seq <= flight.accepted_owner_sequence {
            return Ok(false);
        }
        flight.accepted_owner_sequence = seq;
        if let Some(position) = fields.position {
            flight.last_position = Some(position);
        }
        if matches!(fields.state, Some(2 | 4)) {
            flight.terminal_reported = true;
        }
        Ok(true)
    }
    /// The authored fuse requires a reported collision-resolved terminal state.
    /// Authored lifetime plus the local age allowance expires an unreported
    /// projectile without fabricating a grounded explosion position.
    pub fn take_due(&mut self, now: u64, life: u32) -> Vec<ProjectileTerminal> {
        let mut events = Vec::new();
        for (slot, entry) in self.slots.iter_mut().enumerate() {
            let Some(flight) = entry else {
                continue;
            };
            if flight.life_generation != life {
                *entry = None;
                continue;
            }
            if flight.completed {
                continue;
            }
            let age = now.saturating_sub(flight.launched_at_ms);
            let reason = if flight.terminal_reported && age >= flight.fuse_ms {
                Some(ProjectileTerminalReason::ReportedDetonation)
            } else if age >= flight.expiry_after_ms {
                Some(ProjectileTerminalReason::LifetimeExpired)
            } else {
                None
            };
            if let Some(reason) = reason {
                flight.completed = true;
                events.push(ProjectileTerminal {
                    item_id: flight.item_id,
                    slot: slot as u8,
                    generation: flight.generation,
                    position: if reason == ProjectileTerminalReason::ReportedDetonation {
                        flight.last_position
                    } else {
                        None
                    },
                    reason,
                });
            }
        }
        events
    }
}
