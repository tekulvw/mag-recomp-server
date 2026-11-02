//! Target-only resuscitate request and the retail accepted-revive notification.
//! Source ownership and restore admission belong to the current connection's
//! gameplay handler; neither codec grants authority or changes actor state.

use super::{HeadRegionHit, Reader, RetailDeathNotification, VictimWasMounted};

/// Native 0x021c carries only a patient object handle. The healer is derived
/// from the server's connection-owned actor, never from this packet body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Resuscitate {
    pub patient: u32,
}

impl Resuscitate {
    pub const WIRE_TYPE: u16 = 0x021c;
    pub const ENCODED_LEN: usize = 6;

    pub fn decode(bytes: &[u8]) -> Result<Self, &'static str> {
        let mut reader = Reader { bytes, pos: 0 };
        if reader.u16()? != Self::WIRE_TYPE {
            return Err("invalid_resuscitate_message_type");
        }
        let request = Self {
            patient: reader.u32()?,
        };
        reader.finish()?;
        Ok(request)
    }
}

/// Presentation of an already accepted revive, using actual participant pool
/// indices. This carries no health/state transition and performs no routing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RetailResuscitationNotification {
    pub healer: u16,
    pub patient: u16,
}

impl RetailResuscitationNotification {
    /// Original accepted-resuscitate sender uses source type11, index−1 and
    /// constructor-false head/vehicle bits, independently of prior death flags.
    pub const fn death_notification(self) -> RetailDeathNotification {
        RetailDeathNotification {
            attacker: Some(self.healer),
            defender: self.patient,
            damage_source_type: 11,
            damage_source_index: -1,
            head_region_hit: HeadRegionHit::NonHead,
            victim_was_mounted: VictimWasMounted::NoAssociatedVehicle,
        }
    }

    pub fn encode(self) -> Result<[u8; RetailDeathNotification::ENCODED_LEN], &'static str> {
        self.death_notification().encode()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resuscitate_preserves_only_the_patient_full_object_handle() {
        assert_eq!(
            Resuscitate::decode(&[0x02, 0x1c, 0x03, 0x80, 0x12, 0x34]),
            Ok(Resuscitate {
                patient: 0x0380_1234
            })
        );
        // Handle validity is gameplay admission, not a codec default/rewrite.
        assert_eq!(
            Resuscitate::decode(&[0x02, 0x1c, 0xff, 0xff, 0xff, 0xff]),
            Ok(Resuscitate { patient: u32::MAX })
        );
    }

    #[test]
    fn resuscitate_refuses_wrong_id_truncation_and_added_healer_body() {
        let packet = [0x02, 0x1c, 0x03, 0x80, 0x12, 0x34];
        for length in 0..packet.len() {
            assert_eq!(
                Resuscitate::decode(&packet[..length]),
                Err("truncated_game_message")
            );
        }
        assert_eq!(
            Resuscitate::decode(&[0x02, 0x1b, 0x03, 0x80, 0x12, 0x34]),
            Err("invalid_resuscitate_message_type")
        );
        assert_eq!(
            Resuscitate::decode(&[0x02, 0x1c, 0x03, 0x80, 0x12, 0x34, 0x03, 0x80, 0x56, 0x78]),
            Err("trailing_game_message_bytes")
        );
    }

    #[test]
    fn accepted_revive_encodes_literal_retail_notice_with_no_death_flags() {
        assert_eq!(
            RetailResuscitationNotification {
                healer: 0x1234,
                patient: 0x5678
            }
            .encode(),
            Ok([
                0x01, 0xff, 0x12, 0x34, 0x56, 0x78, 0x0b, 0xff, 0xff, 0xff, 0xff, 0x00
            ])
        );
    }

    #[test]
    fn accepted_revive_requires_present_participant_indices() {
        assert_eq!(
            RetailResuscitationNotification {
                healer: u16::MAX,
                patient: 1
            }
            .encode(),
            Err("invalid_death_attacker")
        );
        assert_eq!(
            RetailResuscitationNotification {
                healer: 1,
                patient: u16::MAX
            }
            .encode(),
            Err("invalid_death_defender")
        );
    }
}
