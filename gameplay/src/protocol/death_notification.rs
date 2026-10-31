//! Original-retail NetMessageDeath application payload (0x01ff).
//!
//! The caller supplies authoritative identities, damage source and pre-death
//! flags. This codec does not resolve actors, derive body mappings, publish a
//! notification, change character state, or add SCERT/SDK framing.

/// The terminal hit's compact body record has armor mapping index zero (head).
/// Callers must resolve that mapping from their actual combat catalog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeadRegionHit {
    NonHead,
    Head,
}

/// Whether the victim had a valid associated vehicle before its death commit.
/// A mounted-state byte alone does not establish a valid associated vehicle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VictimWasMounted {
    NoAssociatedVehicle,
    AssociatedVehicle,
}

/// Retail-only server death/kill notification; no prototype encoder is implied.
///
/// An absent attacker uses the native 0xffff sentinel. Present attacker and
/// defender indices must name actual characters; this codec checks only their
/// wire sentinel, leaving actor/life/recipient validation to the caller.
/// No field or flag has a default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RetailDeathNotification {
    pub attacker: Option<u16>,
    pub defender: u16,
    pub damage_source_type: u8,
    pub damage_source_index: i32,
    pub head_region_hit: HeadRegionHit,
    pub victim_was_mounted: VictimWasMounted,
}

impl RetailDeathNotification {
    pub const WIRE_TYPE: u16 = 0x01ff;
    pub const MEANINGFUL_BITS: usize = 90;
    pub const ENCODED_LEN: usize = 12;

    /// Encode the application message only: ID16 + body74 + zero tail6.
    ///
    /// This does not authorize terminal-death publication or infer either flag.
    pub fn encode(&self) -> Result<[u8; Self::ENCODED_LEN], &'static str> {
        if self.defender == u16::MAX {
            return Err("invalid_death_defender");
        }
        if self.attacker == Some(u16::MAX) {
            return Err("invalid_death_attacker");
        }
        // Original retail Marshal at 0x4543ac writes byte-aligned fields before
        // its two MSB-first Boolean bits. The zeroed native send buffer and
        // SendToPeer at 0x15ce5c..0x15ced0 supply ceil(90/8) bytes and a zero tail.
        let mut wire = [0; Self::ENCODED_LEN];
        wire[..2].copy_from_slice(&Self::WIRE_TYPE.to_be_bytes());
        wire[2..4].copy_from_slice(&self.attacker.unwrap_or(u16::MAX).to_be_bytes());
        wire[4..6].copy_from_slice(&self.defender.to_be_bytes());
        wire[6] = self.damage_source_type;
        wire[7..11].copy_from_slice(&self.damage_source_index.to_be_bytes());
        wire[11] = match self.head_region_hit {
            HeadRegionHit::NonHead => 0,
            HeadRegionHit::Head => 0x80,
        } | match self.victim_was_mounted {
            VictimWasMounted::NoAssociatedVehicle => 0,
            VictimWasMounted::AssociatedVehicle => 0x40,
        };
        Ok(wire)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn original_retail_literal_bytes_cover_all_flags_and_signed_source_bits() {
        // Independently laid out from original Marshal widths/order:
        // ID16, attacker16, defender16, sourceType8, sourceIndex32, flag1, flag1.
        // -0x01234567 is 0xfedcba99 in the native signed32 representation.
        let cases = [
            (
                HeadRegionHit::NonHead,
                VictimWasMounted::NoAssociatedVehicle,
                [
                    0x01, 0xff, 0x01, 0x23, 0x45, 0x67, 0x89, 0xfe, 0xdc, 0xba, 0x99, 0x00,
                ],
            ),
            (
                HeadRegionHit::NonHead,
                VictimWasMounted::AssociatedVehicle,
                [
                    0x01, 0xff, 0x01, 0x23, 0x45, 0x67, 0x89, 0xfe, 0xdc, 0xba, 0x99, 0x40,
                ],
            ),
            (
                HeadRegionHit::Head,
                VictimWasMounted::NoAssociatedVehicle,
                [
                    0x01, 0xff, 0x01, 0x23, 0x45, 0x67, 0x89, 0xfe, 0xdc, 0xba, 0x99, 0x80,
                ],
            ),
            (
                HeadRegionHit::Head,
                VictimWasMounted::AssociatedVehicle,
                [
                    0x01, 0xff, 0x01, 0x23, 0x45, 0x67, 0x89, 0xfe, 0xdc, 0xba, 0x99, 0xc0,
                ],
            ),
        ];
        for (head_region_hit, victim_was_mounted, expected) in cases {
            let actual = RetailDeathNotification {
                attacker: Some(0x0123),
                defender: 0x4567,
                damage_source_type: 0x89,
                damage_source_index: -0x01234567,
                head_region_hit,
                victim_was_mounted,
            }
            .encode()
            .unwrap();
            assert_eq!(actual, expected);
            assert_eq!(actual.len(), 12);
            assert_eq!(actual[11] & 0x3f, 0, "six unused tail bits must stay zero");
        }
    }

    #[test]
    fn absent_attacker_and_native_signed_minus_one_have_literal_wire_values() {
        let actual = RetailDeathNotification {
            attacker: None,
            defender: 42,
            damage_source_type: 1,
            damage_source_index: -1,
            head_region_hit: HeadRegionHit::NonHead,
            victim_was_mounted: VictimWasMounted::NoAssociatedVehicle,
        }
        .encode()
        .unwrap();
        assert_eq!(
            actual,
            [
                0x01, 0xff, 0xff, 0xff, 0x00, 0x2a, 0x01, 0xff, 0xff, 0xff, 0xff, 0x00
            ]
        );
    }

    #[test]
    fn present_character_indices_cannot_use_the_absent_sentinel() {
        let mut message = RetailDeathNotification {
            attacker: None,
            defender: u16::MAX,
            damage_source_type: 1,
            damage_source_index: 0,
            head_region_hit: HeadRegionHit::NonHead,
            victim_was_mounted: VictimWasMounted::NoAssociatedVehicle,
        };
        assert_eq!(message.encode(), Err("invalid_death_defender"));
        message.defender = 0;
        message.attacker = Some(u16::MAX);
        assert_eq!(message.encode(), Err("invalid_death_attacker"));
    }
}
