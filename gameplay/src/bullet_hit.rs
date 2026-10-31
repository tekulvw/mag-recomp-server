//! Native NetMessageHit020f after SCERT/plugin framing is removed.
//!
//! (Demarshal), and NetMessageHit DWARF DIE0348204c. The attacker member
//! exists in memory but is deliberately absent from this fixed39-byte wire.
//! Bind the attacker to the admitted session; decoding is not hit validation.

pub const MESSAGE_ID: u16 = 0x020f;
pub const WIRE_LEN: usize = 39;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NetMessageHit {
    pub gear_index: u8,
    pub fire_mode: u8,
    pub hit_location: u8,
    /// Native forwards this as damage userdata. It is not a proven shot nonce.
    pub client_data: u32,
    pub defender: u32,
    pub hit_origin: [f32; 3],
    pub hit_dest: [f32; 3],
    /// Preserve the received integer for diagnostics; use the clamped fraction
    /// below to match native Demarshal's explicit maximum of1000.
    pub penetration_milli: u16,
}

impl NetMessageHit {
    pub fn decode(bytes: &[u8]) -> Result<Self, &'static str> {
        if bytes.len() != WIRE_LEN {
            return Err("invalid_hit_length");
        }
        if u16::from_be_bytes([bytes[0], bytes[1]]) != MESSAGE_ID {
            return Err("invalid_hit_message_type");
        }
        // All offsets are bounded by the exact length check above.
        let u32_at = |offset| {
            u32::from_be_bytes([
                bytes[offset],
                bytes[offset + 1],
                bytes[offset + 2],
                bytes[offset + 3],
            ])
        };
        let vector_at = |offset| {
            [
                f32::from_bits(u32_at(offset)),
                f32::from_bits(u32_at(offset + 4)),
                f32::from_bits(u32_at(offset + 8)),
            ]
        };
        let hit_origin = vector_at(13);
        let hit_dest = vector_at(25);
        // Explicit server input bound; native Buffer decoding alone does not
        // establish finiteness, geometry, distance, visibility or ownership.
        if hit_origin
            .iter()
            .chain(hit_dest.iter())
            .any(|v| !v.is_finite())
        {
            return Err("nonfinite_hit_coordinates");
        }
        Ok(Self {
            gear_index: bytes[2],
            fire_mode: bytes[3],
            hit_location: bytes[4],
            client_data: u32_at(5),
            defender: u32_at(9),
            hit_origin,
            hit_dest,
            penetration_milli: u16::from_be_bytes([bytes[37], bytes[38]]),
        })
    }

    pub fn penetration_fraction(&self) -> f32 {
        self.penetration_milli.min(1000) as f32 / 1000.0
    }
}
