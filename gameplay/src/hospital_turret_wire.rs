//! Common retail Vehicle turret 0 fields and the family 13 row wrapper.
//! Seat, grant and equipment authority belong to the caller.
use crate::player_tracking;
use crate::replication::{BitReader, BitWriter, InventoryItem, UpdateHeader};
use crate::wire_profile::WireProfile;

const TYPE_MASK: u32 = 1 << 13;
const FIELD_COUNT: usize = 25;
const ITEM_ID_BITS: usize = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Yaw(u16);

impl Yaw {
    const BITS: usize = 11;

    pub(crate) fn new(packed: u16) -> Result<Self, &'static str> {
        if packed >= 1 << Self::BITS {
            return Err("invalid_hospital_turret_yaw_bits");
        }
        Ok(Self(packed))
    }

    pub(crate) fn packed(self) -> u16 {
        self.0
    }

    pub(crate) fn radians(self) -> f32 {
        signed_fixed(self.0, 1 << 10, 128.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Pitch(u16);

impl Pitch {
    const BITS: usize = 10;

    pub(crate) fn new(packed: u16) -> Result<Self, &'static str> {
        if packed >= 1 << Self::BITS {
            return Err("invalid_hospital_turret_pitch_bits");
        }
        Ok(Self(packed))
    }

    pub(crate) fn packed(self) -> u16 {
        self.0
    }

    #[cfg(test)]
    pub(crate) fn radians(self) -> f32 {
        signed_fixed(self.0, 1 << 9, 256.0)
    }
}

fn signed_fixed(packed: u16, sign_bit: u16, scale: f32) -> f32 {
    let magnitude = f32::from(packed & (sign_bit - 1)) / scale;
    if packed & sign_bit == 0 {
        magnitude
    } else {
        -magnitude
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct InventoryDefinition {
    pub version: u8,
    pub items: [Option<u16>; 3],
}

impl InventoryDefinition {
    pub(crate) fn encode(self, bits: &mut BitWriter) -> Result<(), &'static str> {
        validate_profile(bits.profile)?;
        bits.write(self.version.into(), 8)?;
        for item in self.items {
            write_optional(bits, item, |bits, id| bits.write(id.into(), ITEM_ID_BITS))?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Fields {
    pub yaw: Option<Yaw>,
    pub pitch: Option<Pitch>,
    pub primary: Option<InventoryItem>,
    pub secondary: Option<InventoryItem>,
    pub overheat_info: Option<u8>,
}

impl Fields {
    pub(crate) fn has_changes(self) -> bool {
        self.yaw.is_some()
            || self.pitch.is_some()
            || self.primary.is_some()
            || self.secondary.is_some()
            || self.overheat_info.is_some()
    }

    pub(crate) fn read_present(
        &mut self,
        field: usize,
        bits: &mut BitReader<'_>,
    ) -> Result<bool, &'static str> {
        match field {
            8 => self.yaw = Some(Yaw::new(bits.read(Yaw::BITS)? as u16)?),
            9 => self.pitch = Some(Pitch::new(bits.read(Pitch::BITS)? as u16)?),
            11 => self.primary = Some(player_tracking::inventory(bits)?),
            12 => self.secondary = Some(player_tracking::inventory(bits)?),
            21 => self.overheat_info = Some(bits.read(8)? as u8),
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// Write presence and value for a common turret field owned by the caller.
    pub(crate) fn write_field(
        self,
        field: usize,
        definition: Option<InventoryDefinition>,
        bits: &mut BitWriter,
    ) -> Result<bool, &'static str> {
        match field {
            8 => write_optional(bits, self.yaw, |bits, yaw| {
                bits.write(yaw.packed().into(), Yaw::BITS)
            })?,
            9 => write_optional(bits, self.pitch, |bits, pitch| {
                bits.write(pitch.packed().into(), Pitch::BITS)
            })?,
            10 => write_optional(bits, definition, |bits, definition| definition.encode(bits))?,
            11 => write_optional(bits, self.primary, |bits, item| item.encode(bits))?,
            12 => write_optional(bits, self.secondary, |bits, item| item.encode(bits))?,
            21 => write_optional(bits, self.overheat_info, |bits, info| {
                bits.write(info.into(), 8)
            })?,
            _ => return Ok(false),
        }
        Ok(true)
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ClientUpdate {
    pub index: u16,
    pub fields: Fields,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ServerUpdate {
    pub index: u16,
    pub definition: Option<InventoryDefinition>,
    pub fields: Fields,
}

/// Consume the family 13 list, including its terminator. The enclosing owner
/// decoder handles other selected families and final packet padding.
pub(crate) fn decode_objects(bits: &mut BitReader<'_>) -> Result<Vec<ClientUpdate>, &'static str> {
    validate_profile(bits.profile)?;
    let mut rows = Vec::new();
    let mut seen = 0;
    while bits.read(1)? != 0 {
        let index = bits.read(16)? as u16;
        claim_index(index, &mut seen)?;
        let mut fields = Fields::default();
        for field in 0..FIELD_COUNT {
            if bits.read(1)? != 0 && !fields.read_present(field, bits)? {
                return Err("unsupported_client_turret_field");
            }
        }
        rows.push(ClientUpdate { index, fields });
    }
    Ok(rows)
}

/// The full sixteen-row list is at most 351 bytes, including the header.
pub(crate) fn encode_updates(
    sequence: u32,
    timestamp_ms: u32,
    rows: &[ServerUpdate],
    profile: WireProfile,
) -> Result<Vec<Vec<u8>>, &'static str> {
    validate_profile(profile)?;
    if sequence == 0 {
        return Err("invalid_hospital_turret_sequence");
    }
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let mut bits = BitWriter::with_profile(profile);
    UpdateHeader {
        sequence,
        nak_sequence: 0,
        client_index: 0,
        object_type_mask: TYPE_MASK,
        timestamp_ms,
    }
    .encode(&mut bits)?;

    let mut seen = 0;
    for row in rows {
        claim_index(row.index, &mut seen)?;
        bits.write(1, 1)?;
        bits.write(row.index.into(), 16)?;
        for field in 0..FIELD_COUNT {
            if !row.fields.write_field(field, row.definition, &mut bits)? {
                bits.write(0, 1)?;
            }
        }
    }
    bits.write(0, 1)?;
    Ok(vec![bits.finish()])
}

fn validate_profile(profile: WireProfile) -> Result<(), &'static str> {
    if profile.application_id() != 20231
        || profile.object_type_bits() != 20
        || profile.inventory_item_bits() != ITEM_ID_BITS
    {
        return Err("unsupported_hospital_turret_wire_profile");
    }
    Ok(())
}

fn claim_index(index: u16, seen: &mut u16) -> Result<(), &'static str> {
    let bit = 1u16
        .checked_shl(index.into())
        .ok_or("invalid_hospital_turret_index")?;
    if *seen & bit != 0 {
        return Err("duplicate_hospital_turret_index");
    }
    *seen |= bit;
    Ok(())
}

fn write_optional<T>(
    bits: &mut BitWriter,
    value: Option<T>,
    write: impl FnOnce(&mut BitWriter, T) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    bits.write(u32::from(value.is_some()), 1)?;
    if let Some(value) = value {
        write(bits, value)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "hospital_turret_wire_tests.rs"]
mod tests;
