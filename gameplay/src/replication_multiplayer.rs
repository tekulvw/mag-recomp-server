//! Typed shared-match deltas using the already traced native registered fields.
use super::*;
use crate::player_tracking::ClientCharacterFields;

/// A recipient owns its reliable sequence space. Preserve the known encoded
/// body verbatim while marshalling its typed server header for that recipient.
pub fn for_recipient(wire: &[u8], sequence: u32) -> Result<Vec<u8>, &'static str> {
    for_recipient_with_profile(
        wire,
        sequence,
        crate::wire_profile::WireProfile::prototype(),
    )
}
pub fn for_recipient_with_profile(
    wire: &[u8],
    sequence: u32,
    profile: crate::wire_profile::WireProfile,
) -> Result<Vec<u8>, &'static str> {
    if wire.len() > 1397 || sequence == 0 {
        return Err("invalid_shared_packet");
    }
    let update = ObjectUpdate::decode_with_profile(wire, profile)?;
    if update.header.client_index != 0 || update.header.nak_sequence != 0 {
        return Err("not_server_replication");
    }
    let mut bits = BitWriter::with_profile(profile);
    UpdateHeader {
        sequence,
        ..update.header
    }
    .encode(&mut bits)?;
    let mut source = BitReader::with_profile(wire, profile);
    source.position = profile.header_bits();
    while source.position < wire.len() * 8 {
        bits.write(source.read(1)?, 1)?;
    }
    Ok(bits.finish())
}
pub struct RemoteCharacterSnapshot<'a> {
    pub sequence: u32,
    pub timestamp_ms: u32,
    pub character: &'a CharacterCreation,
    pub inventory: Option<&'a CharacterLoadout>,
    pub physical: Option<&'a CharacterRespawn>,
}
impl RemoteCharacterSnapshot<'_> {
    pub fn encode(&self) -> Result<Vec<u8>, &'static str> {
        self.encode_with_profile(crate::wire_profile::WireProfile::prototype())
    }
    pub fn encode_with_profile(
        &self,
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        let mut bits = BitWriter::with_profile(profile);
        UpdateHeader {
            sequence: self.sequence,
            nak_sequence: 0,
            client_index: 0,
            object_type_mask: 8,
            timestamp_ms: self.timestamp_ms,
        }
        .encode(&mut bits)?;
        self.character
            .encode_current(&mut bits, self.inventory, self.physical)?;
        Ok(bits.finish())
    }
}
/// Replace the typed CharacterInfo relationship after an accepted squad change.
/// No physical, life, inventory or match-clock fields are republished.
pub struct CharacterSquadAssignment<'a> {
    pub sequence: u32,
    pub timestamp_ms: u32,
    pub player: u16,
    pub info: &'a CharacterInfo,
}
impl CharacterSquadAssignment<'_> {
    pub fn encode_with_profile(
        &self,
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        if self.sequence == 0
            || self.player == u16::MAX
            || self.info.client_index != self.player
            || SquadAddress::from_index(self.info.squad_index)?.team() != self.info.team
        {
            return Err("inconsistent_character_squad_assignment");
        }
        let mut bits = BitWriter::with_profile(profile);
        UpdateHeader {
            sequence: self.sequence,
            nak_sequence: 0,
            client_index: 0,
            object_type_mask: 1 << 3,
            timestamp_ms: self.timestamp_ms,
        }
        .encode(&mut bits)?;
        bits.write(1, 1)?;
        bits.write(self.player.into(), 16)?;
        for field in 0..22 {
            bits.write(u32::from(field == 3), 1)?;
            if field == 3 {
                self.info.encode(&mut bits)?;
            }
        }
        bits.write(0, 1)?;
        Ok(bits.finish())
    }
}
pub struct CharacterOwnerDelta<'a> {
    pub sequence: u32,
    pub timestamp_ms: u32,
    pub player: u16,
    pub fields: &'a ClientCharacterFields,
}
impl CharacterOwnerDelta<'_> {
    pub fn encode(&self) -> Result<Vec<u8>, &'static str> {
        self.encode_with_profile(crate::wire_profile::WireProfile::prototype())
    }
    pub fn encode_with_profile(
        &self,
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        self.encode_with_damage(profile, None)
    }
    /// Server physical movement may publish landing damage in the same sequence.
    pub fn encode_with_damage(
        &self,
        profile: crate::wire_profile::WireProfile,
        damage: Option<CharacterDamage>,
    ) -> Result<Vec<u8>, &'static str> {
        let mut b = BitWriter::with_profile(profile);
        UpdateHeader {
            sequence: self.sequence,
            nak_sequence: 0,
            client_index: 0,
            object_type_mask: 8,
            timestamp_ms: self.timestamp_ms,
        }
        .encode(&mut b)?;
        b.write(1, 1)?;
        b.write(self.player.into(), 16)?;
        for field in 0..22 {
            let f = self.fields;
            let present = match field {
                4 => f.state.is_some(),
                5 => f.position.is_some(),
                6 => f.pitch.is_some(),
                7 => f.yaw.is_some(),
                8 => damage.is_some(),
                11 => f.primary.is_some(),
                12 => f.secondary.is_some(),
                13 => f.primary_equipped.is_some(),
                _ => false,
            };
            b.write(u32::from(present), 1)?;
            if present {
                match field {
                    4 => f.state.unwrap().encode(&mut b)?,
                    5 => f.position.unwrap().encode(&mut b)?,
                    6 => b.write(f.pitch.unwrap().into(), 8)?,
                    7 => b.write(f.yaw.unwrap().into(), 10)?,
                    8 => damage.unwrap().encode(&mut b)?,
                    11 => f.primary.unwrap().encode(&mut b)?,
                    12 => f.secondary.unwrap().encode(&mut b)?,
                    13 => b.write(u32::from(f.primary_equipped.unwrap()), 1)?,
                    _ => unreachable!(),
                }
            }
        }
        b.write(0, 1)?;
        Ok(b.finish())
    }
}
pub struct SquadRoster {
    pub sequence: u32,
    pub timestamp_ms: u32,
    pub squad: u16,
    pub members: [u16; 8],
    /// Field10: 0xffff or one of `members`.
    pub leader: u16,
}
impl SquadRoster {
    pub fn encode(&self) -> Result<Vec<u8>, &'static str> {
        self.encode_with_profile(crate::wire_profile::WireProfile::prototype())
    }
    pub fn encode_with_profile(
        &self,
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        SquadAddress::from_index(self.squad)?;
        if self.leader != u16::MAX && !self.members.contains(&self.leader) {
            return Err("inconsistent_squad_leader");
        }
        let mut b = BitWriter::with_profile(profile);
        UpdateHeader {
            sequence: self.sequence,
            nak_sequence: 0,
            client_index: 0,
            object_type_mask: 1 << 8,
            timestamp_ms: self.timestamp_ms,
        }
        .encode(&mut b)?;
        b.write(1, 1)?;
        b.write(self.squad.into(), 16)?;
        for field in 0..26 {
            let present = (2..=10).contains(&field);
            b.write(u32::from(present), 1)?;
            if present {
                b.write(
                    if field == 10 {
                        self.leader
                    } else {
                        self.members[field - 2]
                    }
                    .into(),
                    16,
                )?;
            }
        }
        b.write(0, 1)?;
        Ok(b.finish())
    }
}
pub struct CharacterRemoval {
    pub sequence: u32,
    pub timestamp_ms: u32,
    pub player: u16,
}
impl CharacterRemoval {
    pub fn encode(&self) -> Result<Vec<u8>, &'static str> {
        self.encode_with_profile(crate::wire_profile::WireProfile::prototype())
    }
    pub fn encode_with_profile(
        &self,
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        let mut b = BitWriter::with_profile(profile);
        UpdateHeader {
            sequence: self.sequence,
            nak_sequence: 0,
            client_index: 0,
            object_type_mask: 8,
            timestamp_ms: self.timestamp_ms,
        }
        .encode(&mut b)?;
        b.write(1, 1)?;
        b.write(self.player.into(), 16)?;
        for field in 0..22 {
            b.write(u32::from(field == 0), 1)?;
            if field == 0 {
                b.write(0, 1)?;
            }
        }
        b.write(0, 1)?;
        Ok(b.finish())
    }
}
pub struct MatchPhaseUpdate {
    pub sequence: u32,
    pub timestamp_ms: u32,
    pub phase: MatchPhase,
}
impl MatchPhaseUpdate {
    pub fn encode(&self) -> Result<Vec<u8>, &'static str> {
        self.encode_with_profile(crate::wire_profile::WireProfile::prototype())
    }
    pub fn encode_with_profile(
        &self,
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        let mut b = BitWriter::with_profile(profile);
        UpdateHeader {
            sequence: self.sequence,
            nak_sequence: 0,
            client_index: 0,
            object_type_mask: 1 << 7,
            timestamp_ms: self.timestamp_ms,
        }
        .encode(&mut b)?;
        b.write(1, 1)?;
        b.write(0, 16)?;
        for field in 0..10 {
            b.write(u32::from(field == 4), 1)?;
            if field == 4 {
                b.write(self.phase as u32, 16)?;
            }
        }
        b.write(0, 1)?;
        Ok(b.finish())
    }
}
