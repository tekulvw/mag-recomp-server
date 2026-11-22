use crate::{
    replication::{BitReader, BitWriter, CharacterPosition, ObjectUpdate, UpdateHeader},
    wire_profile::WireProfile,
};

pub const CAPACITY: u16 = 32;
const FAMILY_MASK: u32 = 1 << 5;
const SERVER_OWNER: u32 = 0x0105;

#[derive(Clone, Copy, Debug)]
pub struct VolumeCreation {
    pub object_index: u16,
    pub position: CharacterPosition,
}

#[derive(Clone, Copy, Debug)]
pub struct VolumeRemoval {
    pub object_index: u16,
}

impl VolumeCreation {
    pub fn encode_with_profile(
        &self,
        sequence: u32,
        timestamp_ms: u32,
        profile: WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        let mut bits = begin_row(self.object_index, sequence, timestamp_ms, profile)?;
        bits.write(1, 1)?; // Active value.
        bits.write(1, 1)?; // Owner present.
        bits.write(SERVER_OWNER, 16)?;
        bits.write(1, 1)?; // Position present.
        self.position.encode(&mut bits)?;
        bits.write(0, 1)?; // List end.
        Ok(bits.finish())
    }
}

impl VolumeRemoval {
    pub fn encode_with_profile(
        &self,
        sequence: u32,
        timestamp_ms: u32,
        profile: WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        let mut bits = begin_row(self.object_index, sequence, timestamp_ms, profile)?;
        // Inactive, absent owner, absent position, list end.
        bits.write(0, 4)?;
        Ok(bits.finish())
    }
}

fn begin_row(
    object_index: u16,
    sequence: u32,
    timestamp_ms: u32,
    profile: WireProfile,
) -> Result<BitWriter, &'static str> {
    if profile.application_id() != 20231 {
        return Err("smoke_volume_requires_retail_profile");
    }
    if object_index >= CAPACITY {
        return Err("smoke_volume_index_out_of_range");
    }
    let mut bits = BitWriter::with_profile(profile);
    UpdateHeader {
        sequence,
        nak_sequence: 0,
        client_index: 0,
        object_type_mask: FAMILY_MASK,
        timestamp_ms,
    }
    .encode(&mut bits)?;
    bits.write(1, 1)?; // Row present.
    bits.write(u32::from(object_index), 16)?;
    bits.write(1, 1)?; // Active present.
    Ok(bits)
}

/// Recognize a standalone, canonical removal queued by this encoder.
pub fn is_removal(packet: &[u8], profile: WireProfile) -> Option<u16> {
    const BODY_BITS: usize = 1 + 16 + 5;
    let packet_bits = profile.header_bits() + BODY_BITS;
    if profile.application_id() != 20231 || packet.len() != packet_bits.div_ceil(8) {
        return None;
    }
    let header = ObjectUpdate::decode_with_profile(packet, profile)
        .ok()?
        .header;
    if header.nak_sequence != 0
        || header.client_index != 0
        || header.object_type_mask != FAMILY_MASK
    {
        return None;
    }
    let mut bits = BitReader::with_profile(packet, profile);
    bits.position = profile.header_bits();
    let has_row = bits.read(1).ok()? == 1;
    let index = bits.read(16).ok()? as u16;
    // Active presence = 1, all remaining field/list/padding bits = 0.
    let padding = packet.len() * 8 - packet_bits;
    let canonical_tail = bits.read(5 + padding).ok()? == 1 << (4 + padding);
    (has_row && index < CAPACITY && canonical_tail).then_some(index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire_profile::WireProfileCatalog;

    fn retail() -> WireProfile {
        WireProfileCatalog::embedded().profile(20231).unwrap()
    }

    // Decode independently of both the production reader and writer.
    fn wire_bits(packet: &[u8]) -> String {
        packet.iter().map(|byte| format!("{byte:08b}")).collect()
    }

    fn creation(object_index: u16) -> VolumeCreation {
        VolumeCreation {
            object_index,
            position: CharacterPosition {
                x: 2.25,
                y: -3.5,
                z: 4.75,
            },
        }
    }

    fn expected_header() -> String {
        format!(
            "{:016b}{:032b}{:032b}{:016b}{:020b}{:032b}",
            8, 17, 0, 0, 0x20, 91
        )
    }

    #[test]
    fn smoke_volume_creation_is_one_complete_retail_family_five_row() {
        let packet = creation(31).encode_with_profile(17, 91, retail()).unwrap();
        let expected = format!(
            "{}1{:016b}111{:016b}1{:016b}1{:016b}{:016b}0",
            expected_header(),
            31,
            0x105,
            72,
            112,
            152
        );
        assert_eq!(expected.len(), 235);
        assert_eq!(wire_bits(&packet), expected + "00000");
        assert_eq!(is_removal(&packet, retail()), None);
    }

    #[test]
    fn smoke_volume_removal_recognizes_only_exact_canonical_packets() {
        let packet = VolumeRemoval { object_index: 0 }
            .encode_with_profile(17, 91, retail())
            .unwrap();
        let expected = format!("{}1{:016b}10000", expected_header(), 0);
        assert_eq!(expected.len(), 170);
        assert_eq!(wire_bits(&packet), expected + "000000");
        assert_eq!(is_removal(&packet, retail()), Some(0));
        assert_eq!(is_removal(&packet, WireProfile::prototype()), None);
        assert_eq!(is_removal(&packet[..21], retail()), None);
        let mut excess = packet.clone();
        excess.push(0);
        assert_eq!(is_removal(&excess, retail()), None);
        for offset in [15, 48, 80, 94, 148, 159, 165, 166, 167, 168, 169, 175] {
            let mut corrupt = packet.clone();
            corrupt[offset / 8] ^= 1 << (7 - offset % 8);
            assert_eq!(is_removal(&corrupt, retail()), None, "bit {offset}");
        }
    }

    #[test]
    fn smoke_volume_encoders_reject_outside_profile_pool_or_position() {
        for (index, profile) in [(32, retail()), (0, WireProfile::prototype())] {
            assert!(
                creation(index)
                    .encode_with_profile(17, 91, profile)
                    .is_err()
            );
            assert!(
                VolumeRemoval {
                    object_index: index
                }
                .encode_with_profile(17, 91, profile)
                .is_err()
            );
        }
        let mut invalid = creation(0);
        invalid.position.z = f32::NAN;
        assert!(invalid.encode_with_profile(17, 91, retail()).is_err());
    }
}
