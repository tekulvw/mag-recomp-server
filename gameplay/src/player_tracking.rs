//! Restricted native owner-field decoder and client-reported position tracking.
//! Authority and sequence evidence: PLAYER-POSITION.md. No movement physics.
use crate::replication::{
    BitReader, CharacterPosition, CharacterState, InventoryItem, ObjectUpdate,
};
use serde::Serialize;
#[derive(Clone, Debug, Default)]
pub struct ClientCharacterFields {
    pub projectiles: Vec<crate::projectile::ProjectileOwnerUpdate>,
    pub state: Option<CharacterState>,
    pub position: Option<CharacterPosition>,
    pub pitch: Option<u8>,
    pub yaw: Option<u16>,
    pub primary: Option<InventoryItem>,
    pub secondary: Option<InventoryItem>,
    pub primary_equipped: Option<bool>,
}
pub(crate) fn inventory(bits: &mut BitReader<'_>) -> Result<InventoryItem, &'static str> {
    Ok(InventoryItem {
        inventory_update_id: bits.read(8)? as u8,
        gear_slot: bits.read(5)? as u8,
        ammo_pool: bits.read(2)? as u8,
        ammo_count: bits.read(8)? as u8,
        reloading: bits.read(1)? != 0,
    })
}
impl ClientCharacterFields {
    pub fn decode(update: &ObjectUpdate<'_>) -> Result<Self, &'static str> {
        Self::decode_for_owner(update, 0)
    }
    pub fn decode_for_owner(update: &ObjectUpdate<'_>, owner: u16) -> Result<Self, &'static str> {
        Self::decode_for_owner_with_profile(
            update,
            owner,
            crate::wire_profile::WireProfile::prototype(),
        )
    }
    pub fn decode_for_owner_with_profile(
        update: &ObjectUpdate<'_>,
        owner: u16,
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Self, &'static str> {
        update.validate_profile(profile)?;
        let mut fields = Self::default();
        let mut bits = BitReader::with_profile(update.wire, profile);
        bits.position = bits.profile.header_bits();
        let mask = update.header.object_type_mask;
        if mask & !(8 | crate::projectile::PROJECTILE_TYPE_MASK) != 0 {
            return Err("unsupported_client_object_type");
        }
        // Native lists are ordered by increasing object type, with each object's
        // presence bits interleaved with its data. Decode the entire body before
        // exposing either the Character prefix or any projectile report.
        for object_type in [3, 16, 17, 18] {
            if mask & (1 << object_type) == 0 || bits.read(1)? == 0 {
                continue;
            }
            if object_type == 3 {
                fields = Self::decode_present_object(&mut bits, owner)?;
            } else {
                if bits.read(16)? != u32::from(owner) {
                    return Err("unowned_projectile_update");
                }
                fields
                    .projectiles
                    .push(crate::projectile::ProjectileOwnerUpdate {
                        slot: object_type - 16,
                        object_index: owner,
                        fields: crate::projectile::decode_owner_fields(&mut bits)?,
                    });
                if bits.read(1)? != 0 {
                    return Err("multiple_client_projectile_updates");
                }
            }
        }
        let remaining = update.wire.len() * 8 - bits.position;
        if remaining >= 8 || bits.read(remaining)? != 0 {
            return Err("trailing_client_update_bits");
        }
        Ok(fields)
    }
    /// Decode a present Character3 object. The shared body parser consumes the
    /// list-presence bit, preserving one codec for pure and mixed reports.
    pub(crate) fn decode_present_object(
        bits: &mut BitReader<'_>,
        owner: u16,
    ) -> Result<Self, &'static str> {
        if bits.read(16)? != u32::from(owner) {
            return Err("unowned_character_update");
        }
        let mut fields = Self::default();
        for field in 0..22 {
            if bits.read(1)? == 0 {
                continue;
            }
            match field {
                4 => {
                    let packed = bits.read(8)? as u8;
                    fields.state = Some(CharacterState {
                        state: packed & 63,
                        stance: packed >> 6,
                        user_data: [
                            bits.read(8)? as u8,
                            bits.read(8)? as u8,
                            bits.read(8)? as u8,
                        ],
                    });
                }
                5 => {
                    let x = bits.read(16)? as f32 / 32.0;
                    let negative = bits.read(1)? != 0;
                    let y = bits.read(16)? as f32 / 32.0;
                    let z = bits.read(16)? as f32 / 32.0;
                    fields.position = Some(CharacterPosition {
                        x,
                        y: if negative { -y } else { y },
                        z,
                    });
                }
                6 => fields.pitch = Some(bits.read(8)? as u8),
                7 => fields.yaw = Some(bits.read(10)? as u16),
                11 => fields.primary = Some(inventory(bits)?),
                12 => fields.secondary = Some(inventory(bits)?),
                13 => fields.primary_equipped = Some(bits.read(1)? != 0),
                _ => return Err("unsupported_client_character_field"),
            }
        }
        if bits.read(1)? != 0 {
            return Err("multiple_client_character_updates");
        }
        Ok(fields)
    }
}
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PositionOrigin {
    ServerSpawn,
    ServerExit,
    ClientReported,
}
#[derive(Clone, Copy, Debug, Serialize)]
pub struct PositionSample {
    pub position: CharacterPosition,
    pub origin: PositionOrigin,
    pub received_at_ms: u64,
    pub client_sequence: Option<u32>,
}
#[derive(Default)]
pub struct PlayerTracking {
    pub sample: Option<PositionSample>,
    pub accepted_position_sequence: u32,
}
impl PlayerTracking {
    pub fn server_position(
        &mut self,
        position: CharacterPosition,
        origin: PositionOrigin,
        received_at_ms: u64,
    ) {
        self.sample = Some(PositionSample {
            position,
            origin,
            received_at_ms,
            client_sequence: None,
        });
    }
    pub fn accepts(&self, sequence: u32) -> bool {
        sequence > self.accepted_position_sequence
    }
    pub fn accept_client(
        &mut self,
        position: CharacterPosition,
        sequence: u32,
        received_at_ms: u64,
    ) {
        self.accepted_position_sequence = sequence;
        self.sample = Some(PositionSample {
            position,
            origin: PositionOrigin::ClientReported,
            received_at_ms,
            client_sequence: Some(sequence),
        });
    }
}
