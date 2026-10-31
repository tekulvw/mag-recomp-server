//! Native MSB-first object replication. Wire evidence: NEXT-REPLICATION.md.
//! Object data immediately follows the selected build's header (147/148 bits).
#[path = "replication_multiplayer.rs"]
pub mod multiplayer;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UpdateHeader {
    pub sequence: u32,
    pub nak_sequence: u32,
    /// The native client-send path supplies zero; ownership is checked against authenticated transport and body object IDs.
    pub client_index: u16,
    pub object_type_mask: u32,
    pub timestamp_ms: u32,
}
#[derive(Clone, Debug)]
pub struct ObjectUpdate<'a> {
    pub header: UpdateHeader,
    /// Complete native packet used by typed owner-field decoding and evidence capture.
    pub wire: &'a [u8],
}
pub(crate) struct BitReader<'a> {
    pub profile: crate::wire_profile::WireProfile,
    bytes: &'a [u8],
    pub position: usize,
}
impl<'a> BitReader<'a> {
    pub fn with_profile(bytes: &'a [u8], profile: crate::wire_profile::WireProfile) -> Self {
        Self {
            bytes,
            position: 0,
            profile,
        }
    }
    pub fn read(&mut self, width: usize) -> Result<u32, &'static str> {
        if width > 32
            || self
                .position
                .checked_add(width)
                .is_none_or(|end| end > self.bytes.len() * 8)
        {
            return Err("truncated_replication_bits");
        }
        let mut value = 0;
        for _ in 0..width {
            value = (value << 1)
                | ((self.bytes[self.position / 8] >> (7 - self.position % 8)) & 1) as u32;
            self.position += 1;
        }
        Ok(value)
    }
}
pub(crate) struct BitWriter {
    pub profile: crate::wire_profile::WireProfile,
    bytes: Vec<u8>,
    position: usize,
}
impl BitWriter {
    pub fn with_profile(profile: crate::wire_profile::WireProfile) -> Self {
        Self {
            profile,
            bytes: Vec::new(),
            position: 0,
        }
    }
    pub fn write(&mut self, value: u32, width: usize) -> Result<(), &'static str> {
        if width > 32 || (width < 32 && value >= 1u32 << width) {
            return Err("replication_value_out_of_range");
        }
        if self.position + width > 1397 * 8 {
            return Err("replication_packet_too_long");
        }
        for shift in (0..width).rev() {
            if self.position.is_multiple_of(8) {
                self.bytes.push(0);
            }
            self.bytes[self.position / 8] |=
                (((value >> shift) & 1) as u8) << (7 - self.position % 8);
            self.position += 1;
        }
        Ok(())
    }
    pub fn finish(self) -> Vec<u8> {
        self.bytes
    }
}
impl<'a> ObjectUpdate<'a> {
    /// The public header/wire shape is retained for existing packet builders.
    /// Body decoders verify this pair under their explicit context before using
    /// a body offset. In particular, legacy prototype wrappers reject a header
    /// decoded as retail rather than silently reinterpreting its body.
    pub(crate) fn validate_profile(
        &self,
        profile: crate::wire_profile::WireProfile,
    ) -> Result<(), &'static str> {
        if Self::decode_with_profile(self.wire, profile)?.header != self.header {
            return Err("wire_profile_mismatch");
        }
        Ok(())
    }
    pub fn decode(wire: &'a [u8]) -> Result<Self, &'static str> {
        Self::decode_with_profile(wire, crate::wire_profile::WireProfile::prototype())
    }
    pub fn decode_with_profile(
        wire: &'a [u8],
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Self, &'static str> {
        let mut bits = BitReader::with_profile(wire, profile);
        if bits.read(16)? != u32::from(profile.wire_message_type(7)) {
            return Err("wrong_replication_message");
        }
        let header = UpdateHeader {
            sequence: bits.read(32)?,
            nak_sequence: bits.read(32)?,
            client_index: bits.read(16)? as u16,
            object_type_mask: bits.read(profile.object_type_bits())?,
            timestamp_ms: bits.read(32)?,
        };
        if header.object_type_mask == 0 && wire.len() != 19 {
            return Err("trailing_empty_update_bytes");
        }
        Ok(Self { header, wire })
    }
    pub fn encode_empty(&self) -> Result<Vec<u8>, &'static str> {
        self.encode_empty_with_profile(crate::wire_profile::WireProfile::prototype())
    }
    pub fn encode_empty_with_profile(
        &self,
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        if self.header.object_type_mask != 0 {
            return Err("nonempty_update_requires_object_fields");
        }
        let mut bits = BitWriter::with_profile(profile);
        self.header.encode(&mut bits)?;
        Ok(bits.finish())
    }
}
impl UpdateHeader {
    pub(crate) fn encode(&self, bits: &mut BitWriter) -> Result<(), &'static str> {
        bits.write(bits.profile.wire_message_type(7).into(), 16)?;
        bits.write(self.sequence, 32)?;
        bits.write(self.nak_sequence, 32)?;
        bits.write(self.client_index.into(), 16)?;
        bits.write(self.object_type_mask, bits.profile.object_type_bits())?;
        bits.write(self.timestamp_ms, 32)
    }
}

/// Native MakeSquadID / PlatoonProperties::GetObjectCreationID use fixed
/// side, platoon and squad slots. A defender's first Alpha squad is16, not0.
/// Evidence: OBJECTIVE-REPLICATION.md (0x825640, 0x3ee424, 0x5efc2c).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SquadAddress {
    team: u8,
    platoon: u8,
    squad: u8,
}
impl SquadAddress {
    pub fn new(team: u8, platoon: u8, squad: u8) -> Result<Self, &'static str> {
        if team > 1 || platoon >= 4 || squad >= 4 {
            return Err("invalid_squad_address");
        }
        Ok(Self {
            team,
            platoon,
            squad,
        })
    }
    pub fn from_index(index: u16) -> Result<Self, &'static str> {
        if index >= 32 {
            return Err("invalid_squad_index");
        }
        Self::new(
            (index / 16) as u8,
            ((index % 16) / 4) as u8,
            (index % 4) as u8,
        )
    }
    pub fn squad_index(self) -> u16 {
        u16::from(self.team) * 16 + u16::from(self.platoon) * 4 + u16::from(self.squad)
    }
    pub fn platoon_index(self) -> u16 {
        u16::from(self.team) * 4 + u16::from(self.platoon)
    }
    pub fn team(self) -> u8 {
        self.team
    }
}

/// NetDataCharacterInfo is one field in the 22-field MAG character object.
/// Team is a single side bit; faction is a separate byte.
#[derive(Clone, Debug)]
pub struct CharacterInfo {
    pub team: u8,
    pub faction: u8,
    pub character_class: u8,
    pub name: String,
    pub client_index: u16,
    pub squad_index: u16,
    pub account_id: u32,
    pub np_identity: [u8; 36],
}
impl CharacterInfo {
    fn encode(&self, bits: &mut BitWriter) -> Result<(), &'static str> {
        bits.write(self.team.into(), 1)?;
        bits.write(self.faction.into(), 8)?;
        bits.write(self.character_class.into(), 8)?;
        let name_length = u16::try_from(self.name.len()).map_err(|_| "character_name_too_long")?;
        bits.write(name_length.into(), 16)?;
        for byte in self.name.as_bytes() {
            bits.write((*byte).into(), 8)?;
        }
        bits.write(self.client_index.into(), 16)?;
        bits.write(self.squad_index.into(), 16)?;
        bits.write(self.account_id, 32)?;
        for byte in self.np_identity {
            bits.write(byte.into(), 8)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct MagCharacterInfo {
    pub nominations: u8,
    pub level: u8,
    pub leadership_level: u8,
    pub skill_specialty: u8,
    pub head_id: u8,
    pub voice_id: u8,
}
impl MagCharacterInfo {
    fn encode(&self, bits: &mut BitWriter) -> Result<(), &'static str> {
        for byte in [
            self.nominations,
            self.level,
            self.leadership_level,
            self.skill_specialty,
            self.head_id,
            self.voice_id,
        ] {
            bits.write(byte.into(), 8)?;
        }
        if let Some(extension) = bits.profile.mag_info_extension() {
            bits.write(extension.member_0a.into(), 8)?;
            bits.write(extension.member_0b.into(), 8)?;
        }
        Ok(())
    }
}
#[derive(Clone, Debug)]
pub struct CharacterCreation {
    pub object_index: u16,
    pub owner_index: u16,
    pub info: CharacterInfo,
    pub mag_info: MagCharacterInfo,
}
#[derive(Clone, Debug)]
pub struct SquadCreation {
    pub object_index: u16,
    pub owner_index: u16,
    pub members: [u16; 8],
    pub leader_index: u16,
}
/// A bounded initial snapshot, not a general-purpose replacement for native simulation.
#[derive(Clone, Debug)]
pub struct PlayerAndSquad {
    pub sequence: u32,
    pub timestamp_ms: u32,
    pub player: CharacterCreation,
    pub squad: SquadCreation,
}
impl CharacterCreation {
    fn encode(
        &self,
        bits: &mut BitWriter,
        inventory: Option<&CharacterLoadout>,
    ) -> Result<(), &'static str> {
        self.encode_current(bits, inventory, None)
    }
    fn encode_current(
        &self,
        bits: &mut BitWriter,
        inventory: Option<&CharacterLoadout>,
        physical: Option<&CharacterRespawn>,
    ) -> Result<(), &'static str> {
        bits.write(1, 1)?; // type3 object present
        bits.write(self.object_index.into(), 16)?;
        for field in 0..22 {
            match field {
                0 => {
                    bits.write(1, 1)?;
                    bits.write(1, 1)?;
                } // active
                1 => {
                    bits.write(1, 1)?;
                    bits.write(self.owner_index.into(), 16)?;
                }
                3 => {
                    bits.write(1, 1)?;
                    self.info.encode(bits)?;
                }
                4 if physical.is_some() => {
                    bits.write(1, 1)?;
                    physical.unwrap().state.encode(bits)?;
                }
                5 if physical.is_some() => {
                    bits.write(1, 1)?;
                    physical.unwrap().position.encode(bits)?;
                }
                6 if physical.is_some() => {
                    bits.write(1, 1)?;
                    bits.write(physical.unwrap().pitch.into(), 8)?;
                }
                7 if physical.is_some() => {
                    bits.write(1, 1)?;
                    bits.write(physical.unwrap().yaw.into(), 10)?;
                }
                8 if physical.is_some() => {
                    bits.write(1, 1)?;
                    physical.unwrap().damage.encode(bits)?;
                }
                9 if physical.is_some_and(|p| p.state.state != 7) => {
                    bits.write(1, 1)?;
                } // coherent current-life placement
                11 if physical.is_some() && inventory.is_some() => {
                    bits.write(1, 1)?;
                    inventory.unwrap().primary.encode(bits)?;
                }
                13 if physical.is_some() && inventory.is_some() => {
                    bits.write(1, 1)?;
                    bits.write(u32::from(inventory.unwrap().primary_equipped), 1)?;
                }
                15 if inventory.is_some() => {
                    bits.write(1, 1)?;
                    for item in &inventory.unwrap().inventories {
                        item.encode(bits)?;
                    }
                }
                17 if inventory.is_some() => {
                    bits.write(1, 1)?;
                    inventory.unwrap().handle.encode(bits)?;
                }
                18 => {
                    bits.write(1, 1)?;
                    self.mag_info.encode(bits)?;
                }
                _ => bits.write(0, 1)?, // unchanged constructor value; no untraced payload
            }
        }
        bits.write(0, 1) // end type3 object list
    }
}
impl SquadCreation {
    fn encode(&self, bits: &mut BitWriter) -> Result<(), &'static str> {
        self.encode_object(bits)?;
        bits.write(0, 1)
    }
    /// Native field10 is 0xffff while vacant (`SimSquad::RemoveLeader`), else a
    /// member's object index (`SimSquad::SetLeader`).
    pub(crate) fn leader_is_vacant_or_member(&self) -> bool {
        self.leader_index == u16::MAX || self.members.contains(&self.leader_index)
    }
    /// One Squad object, leaving the shared type8 list open for the next squad.
    fn encode_object(&self, bits: &mut BitWriter) -> Result<(), &'static str> {
        bits.write(1, 1)?; // type8 object present
        bits.write(self.object_index.into(), 16)?;
        for field in 0..26 {
            match field {
                0 => {
                    bits.write(1, 1)?;
                    bits.write(1, 1)?;
                }
                1 => {
                    bits.write(1, 1)?;
                    bits.write(self.owner_index.into(), 16)?;
                }
                2..=9 => {
                    bits.write(1, 1)?;
                    bits.write(self.members[field - 2].into(), 16)?;
                }
                10 => {
                    bits.write(1, 1)?;
                    bits.write(self.leader_index.into(), 16)?;
                }
                _ => bits.write(0, 1)?,
            }
        }
        Ok(())
    }
}
/// Native NetObjectGameState::GameState_t (wire u16). Phase is local policy;
/// selecting Playing does not implement objectives, spawn waves or simulation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u16)]
pub enum MatchPhase {
    PreGame = 1,
    Playing = 2,
    EndCountdown = 3,
    PostGame = 4,
}
#[derive(Clone, Debug)]
pub struct MatchStateCreation {
    pub object_index: u16,
    pub owner_index: u16,
    pub phase: MatchPhase,
}
impl MatchStateCreation {
    fn encode(&self, bits: &mut BitWriter) -> Result<(), &'static str> {
        if self.object_index == u16::MAX {
            return Err("invalid_match_state_index");
        }
        bits.write(1, 1)?;
        bits.write(self.object_index.into(), 16)?;
        // Native GameState registers exactly ten fields. Omitted spawn data
        // and global flags retain constructor zero; no spawn points invented.
        for field in 0..10 {
            match field {
                0 => {
                    bits.write(1, 1)?;
                    bits.write(1, 1)?;
                }
                1 => {
                    bits.write(1, 1)?;
                    bits.write(self.owner_index.into(), 16)?;
                }
                4 => {
                    bits.write(1, 1)?;
                    bits.write(self.phase as u32, 16)?;
                }
                _ => bits.write(0, 1)?,
            }
        }
        bits.write(0, 1)
    }
}
/// Native GameState player spawn deadline, in synchronized game milliseconds.
/// Fields5/7 address teams0/1; vehicle deadlines are distinct fields6/8.
#[derive(Clone, Copy, Debug)]
pub struct TeamSpawnDeadline {
    pub sequence: u32,
    pub timestamp_ms: u32,
    pub object_index: u16,
    pub team: u8,
    pub deadline_ms: u32,
}
impl TeamSpawnDeadline {
    pub fn encode(&self) -> Result<Vec<u8>, &'static str> {
        self.encode_with_profile(crate::wire_profile::WireProfile::prototype())
    }
    pub fn encode_with_profile(
        &self,
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        if self.team > 1 || self.object_index == u16::MAX {
            return Err("invalid_team_spawn_deadline");
        }
        let mut bits = BitWriter::with_profile(profile);
        UpdateHeader {
            sequence: self.sequence,
            nak_sequence: 0,
            client_index: 0,
            object_type_mask: 1 << 7,
            timestamp_ms: self.timestamp_ms,
        }
        .encode(&mut bits)?;
        bits.write(1, 1)?;
        bits.write(self.object_index.into(), 16)?;
        for field in 0..10 {
            let present = field == 5 + 2 * self.team;
            bits.write(u32::from(present), 1)?;
            if present {
                bits.write(self.deadline_ms, 32)?;
            }
        }
        bits.write(0, 1)?;
        Ok(bits.finish())
    }
}

impl PlayerAndSquad {
    pub fn encode(&self) -> Result<Vec<u8>, &'static str> {
        self.encode_with_profile(crate::wire_profile::WireProfile::prototype())
    }
    pub fn encode_with_profile(
        &self,
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        self.encode_objects_with_profile(None, None, profile)
    }
    /// Creation inventory without changing an already running match state.
    pub fn encode_with_inventory(
        &self,
        inventory: &CharacterLoadout,
    ) -> Result<Vec<u8>, &'static str> {
        self.encode_with_inventory_with_profile(
            inventory,
            crate::wire_profile::WireProfile::prototype(),
        )
    }
    pub fn encode_with_inventory_with_profile(
        &self,
        inventory: &CharacterLoadout,
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        inventory.validate()?;
        self.encode_objects_with_profile(None, Some(inventory), profile)
    }
    pub fn encode_with_match_state(
        &self,
        state: &MatchStateCreation,
    ) -> Result<Vec<u8>, &'static str> {
        self.encode_with_match_state_with_profile(
            state,
            crate::wire_profile::WireProfile::prototype(),
        )
    }
    pub fn encode_with_match_state_with_profile(
        &self,
        state: &MatchStateCreation,
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        self.encode_objects_with_profile(Some(state), None, profile)
    }
    /// Native MagSimCharacter::Initialize provides inventory array/current handle
    /// before physical respawn. Equipment and health/state remain spawn fields.
    pub fn encode_with_match_state_and_inventory(
        &self,
        state: &MatchStateCreation,
        inventory: Option<&CharacterLoadout>,
    ) -> Result<Vec<u8>, &'static str> {
        self.encode_with_match_state_and_inventory_with_profile(
            state,
            inventory,
            crate::wire_profile::WireProfile::prototype(),
        )
    }
    pub fn encode_with_match_state_and_inventory_with_profile(
        &self,
        state: &MatchStateCreation,
        inventory: Option<&CharacterLoadout>,
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        if let Some(inventory) = inventory {
            inventory.validate()?;
        }
        self.encode_objects_with_profile(Some(state), inventory, profile)
    }
    /// Bootstrap every configured squad, including empty squads, in one type8 list.
    /// The player's assigned squad must match the same authoritative roster supplied here.
    pub fn encode_with_match_state_inventory_and_squads_with_profile(
        &self,
        state: &MatchStateCreation,
        inventory: Option<&CharacterLoadout>,
        squads: &[SquadCreation],
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        if let Some(inventory) = inventory {
            inventory.validate()?;
        }
        if squads.is_empty() || squads.len() > 32 {
            return Err("invalid_bootstrap_squads");
        }
        let mut indices = std::collections::BTreeSet::new();
        let mut members = std::collections::BTreeSet::new();
        let mut assigned = false;
        for squad in squads {
            SquadAddress::from_index(squad.object_index)?;
            if !indices.insert(squad.object_index) {
                return Err("duplicate_bootstrap_squad");
            }
            if !squad.leader_is_vacant_or_member() {
                return Err("inconsistent_squad_leader");
            }
            for member in squad.members.iter().copied().filter(|id| *id != u16::MAX) {
                if !members.insert(member) {
                    return Err("duplicate_squad_member");
                }
            }
            if squad.object_index == self.squad.object_index {
                if squad.owner_index != self.squad.owner_index
                    || squad.members != self.squad.members
                    || squad.leader_index != self.squad.leader_index
                {
                    return Err("inconsistent_player_squad_links");
                }
                assigned = true;
            }
        }
        if !assigned {
            return Err("missing_assigned_squad");
        }
        self.encode_objects_and_squads_with_profile(Some(state), inventory, squads, profile)
    }
    fn encode_objects_with_profile(
        &self,
        state: Option<&MatchStateCreation>,
        inventory: Option<&CharacterLoadout>,
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        self.encode_objects_and_squads_with_profile(
            state,
            inventory,
            std::slice::from_ref(&self.squad),
            profile,
        )
    }
    fn encode_objects_and_squads_with_profile(
        &self,
        state: Option<&MatchStateCreation>,
        inventory: Option<&CharacterLoadout>,
        squads: &[SquadCreation],
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        let squad_address = SquadAddress::from_index(self.squad.object_index)?;
        if squad_address.team() != self.player.info.team
            || self.player.info.squad_index != self.squad.object_index
            || self.player.owner_index != self.player.info.client_index
            || !self.squad.members.contains(&self.player.object_index)
            || !self.squad.leader_is_vacant_or_member()
            || self.player.object_index == u16::MAX
            || self.squad.object_index == u16::MAX
        {
            return Err("inconsistent_player_squad_links");
        }
        let mut bits = BitWriter::with_profile(profile);
        UpdateHeader {
            sequence: self.sequence,
            nak_sequence: 0,
            client_index: 0,
            object_type_mask: (1 << 3) | (1 << 8) | if state.is_some() { 1 << 7 } else { 0 },
            timestamp_ms: self.timestamp_ms,
        }
        .encode(&mut bits)?;
        self.player.encode(&mut bits, inventory)?;
        if let Some(state) = state {
            state.encode(&mut bits)?;
        }
        if squads.len() == 1 {
            squads[0].encode(&mut bits)?;
        } else {
            for squad in squads {
                squad.encode_object(&mut bits)?;
            }
            bits.write(0, 1)?;
        }
        Ok(bits.finish())
    }
}

/// Native Object::Position: unsigned X/Z and signed Y, 11 integer + 5 fraction bits.
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct CharacterPosition {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}
impl CharacterPosition {
    pub fn validate(&self) -> Result<(), &'static str> {
        for (value, signed) in [(self.x, false), (self.y, true), (self.z, false)] {
            if !value.is_finite() || value.abs() >= 2048.0 || (!signed && value < 0.0) {
                return Err("spawn_position_out_of_range");
            }
        }
        Ok(())
    }
    fn encode(&self, bits: &mut BitWriter) -> Result<(), &'static str> {
        self.validate()?;
        for (value, signed) in [(self.x, false), (self.y, true), (self.z, false)] {
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
}
#[derive(Clone, Copy, Debug)]
pub struct CharacterState {
    pub state: u8,
    pub stance: u8,
    pub user_data: [u8; 3],
}
impl CharacterState {
    fn encode(&self, bits: &mut BitWriter) -> Result<(), &'static str> {
        if self.state >= 64 || self.stance >= 4 {
            return Err("character_state_out_of_range");
        }
        bits.write(u32::from((self.stance << 6) | self.state), 8)?;
        for byte in self.user_data {
            bits.write(byte.into(), 8)?;
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug)]
pub struct CharacterDamage {
    pub last_location_and_type: u8,
    pub health_quarter_points: u16,
    pub conditions: u8,
    pub last_attacker: u32,
    pub last_source_object: u32,
    pub last_user_data: u32,
}
impl CharacterDamage {
    fn encode(&self, bits: &mut BitWriter) -> Result<(), &'static str> {
        if self.health_quarter_points >= 4096 || self.conditions >= 16 {
            return Err("character_damage_out_of_range");
        }
        bits.write(self.last_location_and_type.into(), 8)?;
        bits.write(
            u32::from(self.health_quarter_points) | (u32::from(self.conditions) << 12),
            16,
        )?;
        bits.write(self.last_attacker, 32)?;
        bits.write(self.last_source_object, 32)?;
        bits.write(self.last_user_data, 32)
    }
}
/// Native Character field18 publishes the complete MagCharacterInfo on level change.
#[derive(Clone, Debug)]
pub struct CharacterLevelUpdate {
    pub sequence: u32,
    pub timestamp_ms: u32,
    pub object_index: u16,
    pub mag_info: MagCharacterInfo,
}
impl CharacterLevelUpdate {
    pub fn encode(&self) -> Result<Vec<u8>, &'static str> {
        self.encode_with_profile(crate::wire_profile::WireProfile::prototype())
    }
    pub fn encode_with_profile(
        &self,
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        if self.object_index == u16::MAX
            || self.sequence == 0
            || !(1..=40).contains(&self.mag_info.level)
        {
            return Err("invalid_level_delta");
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
        bits.write(self.object_index.into(), 16)?;
        for field in 0..22 {
            bits.write(u32::from(field == 18), 1)?;
            if field == 18 {
                self.mag_info.encode(&mut bits)?;
            }
        }
        bits.write(0, 1)?;
        Ok(bits.finish())
    }
}

/// Character damage and optional server-forced state. Native dead characters
/// publish CS_DEATH7 alongside HP0; field8 alone does not leave the active state.
#[derive(Clone, Copy, Debug)]
pub struct CharacterDamageUpdate {
    pub sequence: u32,
    pub timestamp_ms: u32,
    pub object_index: u16,
    pub damage: CharacterDamage,
    pub forced_state: Option<CharacterState>,
}
impl CharacterDamageUpdate {
    pub fn encode(&self) -> Result<Vec<u8>, &'static str> {
        self.encode_with_profile(crate::wire_profile::WireProfile::prototype())
    }
    pub fn encode_with_profile(
        &self,
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        if self.object_index == u16::MAX || self.sequence <= 1 {
            return Err("invalid_damage_delta");
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
        bits.write(self.object_index.into(), 16)?;
        for field in 0..22 {
            bits.write(
                u32::from(field == 8 || (field == 4 && self.forced_state.is_some())),
                1,
            )?;
            if field == 4
                && let Some(state) = &self.forced_state
            {
                state.encode(&mut bits)?;
            }
            if field == 8 {
                self.damage.encode(&mut bits)?;
            }
        }
        bits.write(0, 1)?;
        Ok(bits.finish())
    }
}
/// Native NetInventory: update byte followed by 22 presence/global item IDs.
/// The selected build supplies their width; equipment keeps gear-slot indices.
#[derive(Clone, Debug)]
pub struct CharacterInventory {
    pub update_id: u8,
    pub slots: [Option<u16>; 22],
}
impl CharacterInventory {
    fn encode(&self, bits: &mut BitWriter) -> Result<(), &'static str> {
        bits.write(self.update_id.into(), 8)?;
        for item in self.slots {
            bits.write(u32::from(item.is_some()), 1)?;
            if let Some(id) = item {
                bits.write(id.into(), bits.profile.inventory_item_bits())?;
            }
        }
        Ok(())
    }
}
/// This index selects a gear slot in the current inventory, not a global item ID.
#[derive(Clone, Copy, Debug)]
pub struct InventoryItem {
    pub inventory_update_id: u8,
    pub gear_slot: u8,
    pub ammo_pool: u8,
    pub ammo_count: u8,
    pub reloading: bool,
}
impl InventoryItem {
    pub(crate) fn encode(&self, bits: &mut BitWriter) -> Result<(), &'static str> {
        bits.write(self.inventory_update_id.into(), 8)?;
        bits.write(self.gear_slot.into(), 5)?;
        bits.write(self.ammo_pool.into(), 2)?;
        bits.write(self.ammo_count.into(), 8)?;
        bits.write(u32::from(self.reloading), 1)
    }
}
#[derive(Clone, Copy, Debug)]
pub struct InventoryHandle {
    pub update_id: u8,
    pub inventory_index: u8,
}
impl InventoryHandle {
    fn encode(&self, bits: &mut BitWriter) -> Result<(), &'static str> {
        bits.write(self.update_id.into(), 8)?;
        bits.write(self.inventory_index.into(), 8)
    }
}
#[derive(Clone, Debug)]
pub struct CharacterLoadout {
    pub inventories: [CharacterInventory; 5],
    pub handle: InventoryHandle,
    pub primary: InventoryItem,
    pub primary_equipped: bool,
}
impl CharacterLoadout {
    fn validate(&self) -> Result<(), &'static str> {
        let inventory = self
            .inventories
            .get(usize::from(self.handle.inventory_index))
            .ok_or("unsupported_inventory_index")?;
        // Native GetCurrentInventory substitutes handle.updateID into the selected
        // inventory; the item consumer compares against that handle version.
        if self.primary.inventory_update_id != self.handle.update_id
            || inventory.slots[0].is_none()
            || inventory
                .slots
                .get(usize::from(self.primary.gear_slot))
                .copied()
                .flatten()
                .is_none()
        {
            return Err("inconsistent_loadout_equipment");
        }
        Ok(())
    }
}
/// Three authored Valor defaults; wire slots 3/4 are empty. No scavenge inventory.
/// ItemListData placeholders are retained when resolving global item ordinals.
pub fn valor_loadout(index: u8) -> Option<CharacterLoadout> {
    crate::item_catalog::bundled().ok()?.valor_loadout(index)
}

/// Delta to an existing Character, with all six respawn-dependent field versions equal.
/// Field9 is NullFieldData: its presence bit is the trigger and has no payload.
#[derive(Clone, Debug)]
pub struct CharacterRespawn {
    pub sequence: u32,
    pub timestamp_ms: u32,
    pub object_index: u16,
    pub state: CharacterState,
    pub position: CharacterPosition,
    pub pitch: u8,
    pub yaw: u16,
    pub damage: CharacterDamage,
}
impl CharacterRespawn {
    /// Respawn at the caller's validated catalog position; no map default is embedded.
    pub fn at_position(timestamp_ms: u32, position: CharacterPosition) -> Self {
        Self {
            sequence: 2,
            timestamp_ms,
            object_index: 0,
            state: CharacterState {
                state: 1,
                stance: 0,
                user_data: [0; 3],
            },
            position,
            pitch: 127,
            yaw: 0, // Facing is explicit local policy pending native transform conversion.
            damage: CharacterDamage {
                last_location_and_type: 0,
                health_quarter_points: 400,
                conditions: 0,
                last_attacker: u32::MAX,
                last_source_object: u32::MAX,
                last_user_data: 0,
            },
        }
    }
    pub fn encode(&self) -> Result<Vec<u8>, &'static str> {
        self.encode_with_profile(crate::wire_profile::WireProfile::prototype())
    }
    pub fn encode_with_profile(
        &self,
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        self.encode_equipped_with_profile(None, None, profile)
    }
    /// Inventory is consumed before equipment by the client, after the whole
    /// object has decoded, so later field15/17 can accompany earlier field11.
    pub fn encode_with_loadout(&self, loadout: &CharacterLoadout) -> Result<Vec<u8>, &'static str> {
        self.encode_with_loadout_with_profile(
            loadout,
            crate::wire_profile::WireProfile::prototype(),
        )
    }
    pub fn encode_with_loadout_with_profile(
        &self,
        loadout: &CharacterLoadout,
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        loadout.validate()?;
        self.encode_equipped_with_profile(Some(loadout), None, profile)
    }
    /// Native Respawn sets mounted state14 and field9 together, choosing the
    /// last available seat (8) in an empty Alpha bunker.
    pub fn encode_in_alpha(
        &self,
        loadout: Option<&CharacterLoadout>,
        client_index: u16,
    ) -> Result<Vec<u8>, &'static str> {
        self.encode_in_alpha_with_profile(
            loadout,
            client_index,
            crate::wire_profile::WireProfile::prototype(),
        )
    }
    pub fn encode_in_alpha_with_profile(
        &self,
        loadout: Option<&CharacterLoadout>,
        client_index: u16,
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        if client_index >= 0x100
            || self.state.state != 14
            || self.state.user_data[..2] != [13, 0]
            || !(1..=8).contains(&self.state.user_data[2])
        {
            return Err("invalid_bunker_respawn");
        }
        if let Some(kit) = loadout {
            kit.validate()?;
        }
        self.encode_in_alpha_with_ownership_with_profile(
            loadout,
            client_index,
            AlphaVehicleOwnership {
                owner: client_index,
                turret_owner: u16::MAX,
            },
            profile,
        )
    }
    pub fn encode_in_alpha_with_ownership(
        &self,
        loadout: Option<&CharacterLoadout>,
        client_index: u16,
        ownership: AlphaVehicleOwnership,
    ) -> Result<Vec<u8>, &'static str> {
        self.encode_in_alpha_with_ownership_with_profile(
            loadout,
            client_index,
            ownership,
            crate::wire_profile::WireProfile::prototype(),
        )
    }
    pub fn encode_in_alpha_with_ownership_with_profile(
        &self,
        loadout: Option<&CharacterLoadout>,
        client_index: u16,
        ownership: AlphaVehicleOwnership,
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        if client_index >= 0x100
            || self.state.state != 14
            || self.state.user_data[..2] != [13, 0]
            || !(1..=8).contains(&self.state.user_data[2])
        {
            return Err("invalid_bunker_respawn");
        }
        if let Some(kit) = loadout {
            kit.validate()?;
        }
        self.encode_equipped_with_profile(loadout, Some(ownership), profile)
    }
    fn encode_equipped_with_profile(
        &self,
        loadout: Option<&CharacterLoadout>,
        bunker_owner: Option<AlphaVehicleOwnership>,
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        if self.object_index == u16::MAX || self.sequence <= 1 {
            return Err("invalid_respawn_delta");
        }
        let mut bits = BitWriter::with_profile(profile);
        UpdateHeader {
            sequence: self.sequence,
            nak_sequence: 0,
            client_index: 0,
            object_type_mask: (1 << 3) | if bunker_owner.is_some() { 1 << 13 } else { 0 },
            timestamp_ms: self.timestamp_ms,
        }
        .encode(&mut bits)?;
        bits.write(1, 1)?;
        bits.write(self.object_index.into(), 16)?;
        for field in 0..22 {
            bits.write(
                u32::from(
                    (4..=9).contains(&field)
                        || (loadout.is_some() && [11, 13, 15, 17].contains(&field)),
                ),
                1,
            )?;
            match field {
                4 => self.state.encode(&mut bits)?,
                5 => self.position.encode(&mut bits)?,
                6 => bits.write(self.pitch.into(), 8)?,
                7 => bits.write(self.yaw.into(), 10)?,
                8 => self.damage.encode(&mut bits)?,
                11 => {
                    if let Some(l) = loadout {
                        l.primary.encode(&mut bits)?;
                    }
                }
                13 => {
                    if let Some(l) = loadout {
                        bits.write(u32::from(l.primary_equipped), 1)?;
                    }
                }
                15 => {
                    if let Some(l) = loadout {
                        for inventory in &l.inventories {
                            inventory.encode(&mut bits)?;
                        }
                    }
                }
                17 => {
                    if let Some(l) = loadout {
                        l.handle.encode(&mut bits)?;
                    }
                }
                _ => (),
            }
        }
        bits.write(0, 1)?;
        if let Some(ownership) = bunker_owner {
            ownership.encode(&mut bits)?;
        }
        Ok(bits.finish())
    }
}

/// Native ObjectiveState_t; wire u8. Availability is distinct from capture or damage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ObjectiveState {
    Inactive = 0,
    Deactivated = 1,
    Activating = 2,
    Activated = 3,
    Deactivating = 4,
    Repairing = 5,
    Repaired = 6,
    Completed = 7,
    Failed = 8,
    Locked = 9,
    Retaking = 10,
    Retaken = 11,
    CancelRetaken = 12,
}
/// NetFieldStateActor markup: u8 state, u32 change time, u16 actor; no padding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObjectiveStateActor {
    pub state: ObjectiveState,
    pub change_time_ms: u32,
    pub actor_index: u16,
}
impl ObjectiveStateActor {
    fn encode(self, bits: &mut BitWriter) -> Result<(), &'static str> {
        bits.write(self.state as u32, 8)?;
        bits.write(self.change_time_ms, 32)?;
        bits.write(self.actor_index.into(), 16)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObjectiveStateDelta {
    pub object_index: u16,
    pub state_actor: ObjectiveStateActor,
}
/// Timed objective state: native fields2,3,4,6. Active/owner stay map-owned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObjectiveControlDelta {
    pub object_index: u16,
    pub state_actor: ObjectiveStateActor,
    pub expiration_time_ms: u32,
    pub deactivation_time_ms: u32,
    pub health_percent: u8,
}
#[derive(Clone, Debug)]
pub struct ObjectiveControlSnapshot {
    pub sequence: u32,
    pub timestamp_ms: u32,
    pub objectives: Vec<ObjectiveControlDelta>,
}
impl ObjectiveControlSnapshot {
    pub fn encode_with_profile(
        &self,
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        if self.objectives.is_empty() {
            return Err("empty_objective_snapshot");
        }
        let mut bits = BitWriter::with_profile(profile);
        UpdateHeader {
            sequence: self.sequence,
            nak_sequence: 0,
            client_index: 0,
            object_type_mask: 1 << 12,
            timestamp_ms: self.timestamp_ms,
        }
        .encode(&mut bits)?;
        let mut previous = None;
        for objective in &self.objectives {
            if objective.object_index >= 128
                || previous.is_some_and(|index| objective.object_index <= index)
                || objective.health_percent > 100
            {
                return Err("invalid_objective_control_delta");
            }
            previous = Some(objective.object_index);
            bits.write(1, 1)?;
            bits.write(objective.object_index.into(), 16)?;
            for field in 0..11 {
                bits.write(u32::from(matches!(field, 2 | 3 | 4 | 6)), 1)?;
                match field {
                    2 => objective.state_actor.encode(&mut bits)?,
                    3 => bits.write(objective.expiration_time_ms, 32)?,
                    4 => bits.write(objective.deactivation_time_ms, 32)?,
                    6 => bits.write(objective.health_percent.into(), 8)?,
                    _ => {}
                }
            }
        }
        bits.write(0, 1)?;
        Ok(bits.finish())
    }
}
/// Squad field12 holds a16-bit objective pool index, not a packed entity ID.
#[derive(Clone, Copy, Debug)]
pub struct SquadObjectiveAssignment {
    pub sequence: u32,
    pub timestamp_ms: u32,
    pub squad_index: u16,
    pub objective_index: u16,
}
impl SquadObjectiveAssignment {
    pub fn encode(&self) -> Result<Vec<u8>, &'static str> {
        self.encode_with_profile(crate::wire_profile::WireProfile::prototype())
    }
    pub fn encode_with_profile(
        &self,
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        SquadAddress::from_index(self.squad_index)?;
        if self.sequence <= 1 {
            return Err("invalid_squad_assignment_delta");
        }
        let mut bits = BitWriter::with_profile(profile);
        UpdateHeader {
            sequence: self.sequence,
            nak_sequence: 0,
            client_index: 0,
            object_type_mask: 1 << 8,
            timestamp_ms: self.timestamp_ms,
        }
        .encode(&mut bits)?;
        bits.write(1, 1)?;
        bits.write(self.squad_index.into(), 16)?;
        for field in 0..26 {
            bits.write(u32::from(field == 12), 1)?;
            if field == 12 {
                bits.write(self.objective_index.into(), 16)?;
            }
        }
        bits.write(0, 1)?;
        Ok(bits.finish())
    }
}
/// State changes for already map-created type12 objects. The default encoder
/// changes only field2; optional sequence playback adds fields9/10 together.
/// Active/owner are omitted, preserving map entities.
#[derive(Clone, Debug)]
pub struct ObjectiveStateSnapshot {
    pub sequence: u32,
    pub timestamp_ms: u32,
    pub objectives: Vec<ObjectiveStateDelta>,
}
/// Native signed sequence index and synchronized start time, fields9/10.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObjectiveSequenceDelta {
    pub object_index: u16,
    pub sequence_index: i32,
    pub start_time_ms: u32,
}
impl ObjectiveStateSnapshot {
    pub fn encode(&self) -> Result<Vec<u8>, &'static str> {
        self.encode_with_profile(crate::wire_profile::WireProfile::prototype())
    }
    pub fn encode_with_profile(
        &self,
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        self.encode_with_sequences_with_profile(&[], profile)
    }
    /// Native SetSequenceIndex publishes fields9/10 together. -1 stops playback;
    /// -2 means leave unchanged and should be omitted rather than serialized.
    pub fn encode_with_sequences(
        &self,
        sequences: &[ObjectiveSequenceDelta],
    ) -> Result<Vec<u8>, &'static str> {
        self.encode_with_sequences_with_profile(
            sequences,
            crate::wire_profile::WireProfile::prototype(),
        )
    }
    pub fn encode_with_sequences_with_profile(
        &self,
        sequences: &[ObjectiveSequenceDelta],
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        if sequences
            .windows(2)
            .any(|pair| pair[0].object_index >= pair[1].object_index)
            || sequences.iter().any(|sequence| {
                sequence.sequence_index < -1
                    || !self
                        .objectives
                        .iter()
                        .any(|objective| objective.object_index == sequence.object_index)
            })
        {
            return Err("invalid_objective_sequence_links");
        }
        if self.objectives.is_empty() {
            return Err("empty_objective_snapshot");
        }
        let mut bits = BitWriter::with_profile(profile);
        UpdateHeader {
            sequence: self.sequence,
            nak_sequence: 0,
            client_index: 0,
            object_type_mask: 1 << 12,
            timestamp_ms: self.timestamp_ms,
        }
        .encode(&mut bits)?;
        let mut previous = None;
        for objective in &self.objectives {
            // This prototype's objective pool has128 slots. Require unique,
            // ascending indices, matching the native object traversal.
            if objective.object_index >= 128
                || previous.is_some_and(|p| objective.object_index <= p)
            {
                return Err("invalid_objective_index_order");
            }
            previous = Some(objective.object_index);
            bits.write(1, 1)?;
            bits.write(objective.object_index.into(), 16)?;
            let sequence = sequences
                .iter()
                .find(|sequence| sequence.object_index == objective.object_index);
            for field in 0..11 {
                bits.write(
                    u32::from(field == 2 || (sequence.is_some() && (field == 9 || field == 10))),
                    1,
                )?;
                if field == 2 {
                    objective.state_actor.encode(&mut bits)?;
                } else if let Some(sequence) = sequence {
                    if field == 9 {
                        bits.write(sequence.sequence_index as u32, 32)?;
                    } else if field == 10 {
                        bits.write(sequence.start_time_ms, 32)?;
                    }
                }
            }
        }
        bits.write(0, 1)?;
        Ok(bits.finish())
    }
}

/// Effective client vehicle damage uses the shared NetDataMAGDamageModel markup.
/// Character registers it first (0x420b68), including source; duplicate vehicle
/// registration (0x41a19c) is skipped by0xfdb9e8. Live decoding proves120 bits.
#[derive(Clone, Copy, Debug)]
pub struct VehicleDamage {
    pub last_location_and_type: u8,
    pub health_quarter_points: u16,
    pub conditions: u8,
    pub last_attacker: u32,
    pub last_source_object: u32,
    pub last_user_data: u32,
}
impl VehicleDamage {
    pub(crate) fn encode(&self, bits: &mut BitWriter) -> Result<(), &'static str> {
        if self.health_quarter_points >= 4096 || self.conditions >= 16 {
            return Err("vehicle_damage_out_of_range");
        }
        bits.write(self.last_location_and_type.into(), 8)?;
        bits.write(
            u32::from(self.health_quarter_points) | (u32::from(self.conditions) << 12),
            16,
        )?;
        bits.write(self.last_attacker, 32)?;
        bits.write(self.last_source_object, 32)?;
        bits.write(self.last_user_data, 32)
    }
}
#[derive(Clone, Copy, Debug)]
#[repr(u8)]
pub enum VehicleState {
    Operational = 2,
}
/// One type13 vehicle, placed by its map-created SpawnPoint reference. Its
/// 25-field schema has no position/quaternion fields (unlike ground vehicles).
#[derive(Clone, Debug)]
pub struct DefensiveEmplacementCreation {
    pub sequence: u32,
    pub timestamp_ms: u32,
    pub object_index: u16,
    pub owner_index: u16,
    pub vehicle_type_token: u32,
    pub faction: u8,
    pub modifier: u8,
    pub state: VehicleState,
    pub turret_owners: [u16; 2],
    pub damage: VehicleDamage,
    pub team: u8,
    pub spawn_point_index: u16,
}
impl DefensiveEmplacementCreation {
    pub fn encode(&self) -> Result<Vec<u8>, &'static str> {
        self.encode_with_profile(crate::wire_profile::WireProfile::prototype())
    }
    pub fn encode_with_profile(
        &self,
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        Self::encode_list_with_profile(
            self.sequence,
            self.timestamp_ms,
            std::slice::from_ref(self),
            profile,
        )
    }
    /// A single native object-type list, terminated after its final object.
    pub fn encode_list(
        sequence: u32,
        timestamp_ms: u32,
        vehicles: &[Self],
    ) -> Result<Vec<u8>, &'static str> {
        Self::encode_list_with_profile(
            sequence,
            timestamp_ms,
            vehicles,
            crate::wire_profile::WireProfile::prototype(),
        )
    }
    pub fn encode_list_with_profile(
        sequence: u32,
        timestamp_ms: u32,
        vehicles: &[Self],
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        if sequence <= 1 {
            return Err("invalid_emplacement_creation");
        }
        let mut bits = BitWriter::with_profile(profile);
        UpdateHeader {
            sequence,
            nak_sequence: 0,
            client_index: 0,
            object_type_mask: 1 << 13,
            timestamp_ms,
        }
        .encode(&mut bits)?;
        Self::encode_objects(&mut bits, vehicles)?;
        Ok(bits.finish())
    }
    fn encode_objects(bits: &mut BitWriter, vehicles: &[Self]) -> Result<(), &'static str> {
        if vehicles.len() > 16 {
            return Err("too_many_emplacement_creations");
        }
        let mut ids = std::collections::BTreeSet::new();
        let mut spawn_ids = std::collections::BTreeSet::new();
        for vehicle in vehicles {
            if !ids.insert(vehicle.object_index) || !spawn_ids.insert(vehicle.spawn_point_index) {
                return Err("duplicate_emplacement_creation");
            }
            vehicle.encode_fields(bits)?;
        }
        bits.write(0, 1)
    }
    fn encode_fields(&self, bits: &mut BitWriter) -> Result<(), &'static str> {
        if self.object_index >= 16
            || self.faction > 2
            || self.modifier >= 64
            || self.team > 1
            || self.spawn_point_index >= 128
        {
            return Err("invalid_emplacement_creation");
        }
        bits.write(1, 1)?;
        bits.write(self.object_index.into(), 16)?;
        for field in 0..25 {
            bits.write(
                u32::from([0, 1, 3, 4, 5, 6, 7, 18, 19, 20].contains(&field)),
                1,
            )?;
            match field {
                0 => bits.write(1, 1)?,
                1 => bits.write(self.owner_index.into(), 16)?,
                3 => bits.write(self.vehicle_type_token, 32)?,
                4 => bits.write(u32::from((self.modifier << 2) | self.faction), 8)?,
                5 => bits.write(self.state as u32, 8)?,
                6 => bits.write(self.turret_owners[0].into(), 16)?,
                7 => bits.write(self.turret_owners[1].into(), 16)?,
                18 => self.damage.encode(bits)?,
                19 => bits.write(self.team.into(), 8)?,
                20 => bits.write(self.spawn_point_index.into(), 16)?,
                _ => (),
            }
        }
        Ok(())
    }
}

/// Native seat transition: Character state correction plus emplacement ownership.
/// No active, respawn, damage or inventory fields are resent.
#[derive(Clone, Copy, Debug)]
pub enum AlphaSeatAction {
    Mount {
        seat: u8,
        client_index: u16,
    },
    Exit {
        yaw: u16,
        position: CharacterPosition,
    },
}
#[derive(Clone, Copy, Debug)]
pub struct AlphaSeatTransition {
    pub sequence: u32,
    pub timestamp_ms: u32,
    pub action: AlphaSeatAction,
}
impl AlphaSeatTransition {
    pub fn encode(&self) -> Result<Vec<u8>, &'static str> {
        self.encode_with_profile(crate::wire_profile::WireProfile::prototype())
    }
    pub fn encode_with_profile(
        &self,
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        self.encode_for_player_with_profile(0, profile)
    }
    pub fn encode_for_player(&self, player: u16) -> Result<Vec<u8>, &'static str> {
        self.encode_for_player_with_profile(player, crate::wire_profile::WireProfile::prototype())
    }
    pub fn encode_for_player_with_profile(
        &self,
        player: u16,
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        self.encode_for_player_with_ownership_with_profile(player, None, profile)
    }
    pub fn encode_for_player_with_ownership(
        &self,
        player: u16,
        ownership: Option<AlphaVehicleOwnership>,
    ) -> Result<Vec<u8>, &'static str> {
        self.encode_for_player_with_ownership_with_profile(
            player,
            ownership,
            crate::wire_profile::WireProfile::prototype(),
        )
    }
    pub fn encode_for_player_with_ownership_with_profile(
        &self,
        player: u16,
        ownership: Option<AlphaVehicleOwnership>,
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        if self.sequence < 5 {
            return Err("invalid_seat_sequence");
        }
        let (state, owner, turret_owner, exit) = match self.action {
            AlphaSeatAction::Mount { seat, client_index } => {
                if seat > 8 || client_index >= 0x100 {
                    return Err("invalid_alpha_seat");
                }
                (
                    CharacterState {
                        state: 14,
                        stance: 0,
                        user_data: [13, 0, seat],
                    },
                    client_index,
                    if seat == 0 { client_index } else { u16::MAX },
                    None,
                )
            }
            AlphaSeatAction::Exit { yaw, position } => {
                if yaw >= 1024 {
                    return Err("invalid_exit_yaw");
                }
                (
                    CharacterState {
                        state: 1,
                        stance: 0,
                        user_data: [0; 3],
                    },
                    0x105,
                    u16::MAX,
                    Some((yaw, position)),
                )
            }
        };
        let mut bits = BitWriter::with_profile(profile);
        UpdateHeader {
            sequence: self.sequence,
            nak_sequence: 0,
            client_index: 0,
            object_type_mask: (1 << 3) | (1 << 13),
            timestamp_ms: self.timestamp_ms,
        }
        .encode(&mut bits)?;
        bits.write(1, 1)?;
        bits.write(player.into(), 16)?;
        for field in 0..22 {
            bits.write(
                u32::from(field == 4 || (exit.is_some() && [5, 7].contains(&field))),
                1,
            )?;
            match field {
                4 => state.encode(&mut bits)?,
                5 => {
                    if let Some((_, position)) = exit {
                        position.encode(&mut bits)?;
                    }
                }
                7 => {
                    if let Some((yaw, _)) = exit {
                        bits.write(yaw.into(), 10)?;
                    }
                }
                _ => (),
            }
        }
        bits.write(0, 1)?; // End Character list.
        ownership
            .unwrap_or(AlphaVehicleOwnership {
                owner,
                turret_owner,
            })
            .encode(&mut bits)?;
        Ok(bits.finish())
    }
}

/// Shared stationary-emplacement authority; passengers use Character seat state.
#[derive(Clone, Copy, Debug)]
pub struct AlphaVehicleOwnership {
    pub owner: u16,
    pub turret_owner: u16,
}
impl AlphaVehicleOwnership {
    pub fn encode_update(&self, sequence: u32, timestamp_ms: u32) -> Result<Vec<u8>, &'static str> {
        self.encode_update_with_profile(
            sequence,
            timestamp_ms,
            crate::wire_profile::WireProfile::prototype(),
        )
    }
    pub fn encode_update_with_profile(
        &self,
        sequence: u32,
        timestamp_ms: u32,
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        let mut bits = BitWriter::with_profile(profile);
        UpdateHeader {
            sequence,
            nak_sequence: 0,
            client_index: 0,
            object_type_mask: 1 << 13,
            timestamp_ms,
        }
        .encode(&mut bits)?;
        self.encode(&mut bits)?;
        Ok(bits.finish())
    }
    fn encode(&self, bits: &mut BitWriter) -> Result<(), &'static str> {
        bits.write(1, 1)?;
        bits.write(0, 16)?; // Existing Alpha emplacement0.
        for field in 0..25 {
            bits.write(u32::from([1, 6].contains(&field)), 1)?;
            match field {
                1 => bits.write(self.owner.into(), 16)?,
                6 => bits.write(self.turret_owner.into(), 16)?,
                _ => (),
            }
        }
        bits.write(0, 1)?;
        Ok(())
    }
}

/// Native GameState fields2/3: a byte count and 128 logical flags in index order.
struct SpawnAvailability;
impl SpawnAvailability {
    fn encode(
        bits: &mut BitWriter,
        catalog: &crate::spawn_catalog::SpawnCatalog,
        all_supported: bool,
    ) -> Result<(), &'static str> {
        bits.write(1, 1)?;
        bits.write(0, 16)?;
        for field in 0..10 {
            bits.write(u32::from(field == 2 || field == 3), 1)?;
            if field == 2 {
                bits.write(catalog.spawn_count().into(), 8)?;
            }
            if field == 3 {
                for index in 0..128 {
                    let usable = if all_supported {
                        catalog.point(index).is_some_and(|point| {
                            point.active
                                && !matches!(
                                    point.kind,
                                    crate::spawn_catalog::SpawnKind::Unsupported
                                )
                        })
                    } else {
                        catalog.alpha(index).is_some()
                    };
                    bits.write(u32::from(usable), 1)?;
                }
            }
        }
        bits.write(0, 1)
    }
}
#[derive(Clone, Copy, Debug)]
pub struct SquadSpawnAssignment {
    pub sequence: u32,
    pub timestamp_ms: u32,
    pub spawn_point_index: u16,
}
impl SquadSpawnAssignment {
    fn encode_object_for_squad(
        &self,
        bits: &mut BitWriter,
        squad: u16,
    ) -> Result<(), &'static str> {
        SquadAddress::from_index(squad)?;
        if self.spawn_point_index >= 128 {
            return Err("invalid_alpha_spawn");
        }
        bits.write(1, 1)?;
        bits.write(squad.into(), 16)?;
        for field in 0..26 {
            bits.write(u32::from(field == 11), 1)?;
            if field == 11 {
                bits.write(self.spawn_point_index.into(), 16)?;
            }
        }
        bits.write(0, 1)
    }
    pub fn encode(&self) -> Result<Vec<u8>, &'static str> {
        self.encode_with_profile(crate::wire_profile::WireProfile::prototype())
    }
    pub fn encode_with_profile(
        &self,
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        self.encode_with_character_with_profile(false, profile)
    }
    pub fn encode_with_character(&self, follow: bool) -> Result<Vec<u8>, &'static str> {
        self.encode_with_character_with_profile(
            follow,
            crate::wire_profile::WireProfile::prototype(),
        )
    }
    pub fn encode_with_character_with_profile(
        &self,
        follow: bool,
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        self.encode_with_character_for_player_with_profile(follow, 0, profile)
    }
    pub fn encode_with_character_for_player(
        &self,
        follow: bool,
        player: u16,
    ) -> Result<Vec<u8>, &'static str> {
        self.encode_with_character_for_player_with_profile(
            follow,
            player,
            crate::wire_profile::WireProfile::prototype(),
        )
    }
    pub fn encode_with_character_for_player_with_profile(
        &self,
        follow: bool,
        player: u16,
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        let catalog =
            crate::spawn_catalog::SpawnCatalog::load(crate::spawn_catalog::default_path())
                .map_err(|_| "spawn_catalog_unavailable")?;
        self.encode_with_character_for_player_and_catalog_with_profile(
            follow, player, &catalog, profile,
        )
    }
    pub fn encode_with_character_for_player_and_catalog(
        &self,
        follow: bool,
        player: u16,
        catalog: &crate::spawn_catalog::SpawnCatalog,
    ) -> Result<Vec<u8>, &'static str> {
        self.encode_with_character_for_player_and_catalog_with_profile(
            follow,
            player,
            catalog,
            crate::wire_profile::WireProfile::prototype(),
        )
    }
    pub fn encode_with_character_for_player_and_catalog_with_profile(
        &self,
        follow: bool,
        player: u16,
        catalog: &crate::spawn_catalog::SpawnCatalog,
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        if catalog.alpha(self.spawn_point_index).is_none() {
            return Err("invalid_alpha_spawn");
        }
        if self.sequence <= 1 {
            return Err("invalid_squad_spawn_sequence");
        }
        let mut bits = BitWriter::with_profile(profile);
        UpdateHeader {
            sequence: self.sequence,
            nak_sequence: 0,
            client_index: 0,
            object_type_mask: (1 << 8) | if follow { 1 << 3 } else { 0 },
            timestamp_ms: self.timestamp_ms,
        }
        .encode(&mut bits)?;
        if follow {
            CharacterSpawnAssignment {
                sequence: self.sequence,
                timestamp_ms: self.timestamp_ms,
                spawn_point_index: self.spawn_point_index,
            }
            .encode_object_for_player(&mut bits, player)?;
        }
        self.encode_object_for_squad(&mut bits, catalog.squad_index())?;
        Ok(bits.finish())
    }
}
/// Publish usable points, selected forward location, and its linked vehicle before
/// the pre-spawn UI. Immutable history replays this setup without reallocating it.
#[derive(Clone, Copy, Debug)]
pub struct AlphaSpawnSetup {
    pub sequence: u32,
    pub timestamp_ms: u32,
}
impl AlphaSpawnSetup {
    pub fn encode_with_bunkers(
        &self,
        player: u16,
        occupant: Option<(u16, u8)>,
        catalog: &crate::spawn_catalog::SpawnCatalog,
        bunkers: &[DefensiveEmplacementCreation],
    ) -> Result<Vec<u8>, &'static str> {
        self.encode_with_bunkers_with_profile(
            player,
            occupant,
            catalog,
            bunkers,
            crate::wire_profile::WireProfile::prototype(),
        )
    }
    pub fn encode_with_bunkers_with_profile(
        &self,
        player: u16,
        occupant: Option<(u16, u8)>,
        catalog: &crate::spawn_catalog::SpawnCatalog,
        bunkers: &[DefensiveEmplacementCreation],
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        self.encode_setup_with_profile(player, occupant, catalog, bunkers, true, profile)
    }
    fn encode_setup_with_profile(
        &self,
        player: u16,
        occupant: Option<(u16, u8)>,
        catalog: &crate::spawn_catalog::SpawnCatalog,
        bunkers: &[DefensiveEmplacementCreation],
        all_supported: bool,
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        if self.sequence <= 1 {
            return Err("invalid_alpha_setup_sequence");
        }
        let mut bits = BitWriter::with_profile(profile);
        UpdateHeader {
            sequence: self.sequence,
            nak_sequence: 0,
            client_index: 0,
            object_type_mask: (1 << 3)
                | (1 << 7)
                | (1 << 8)
                | if bunkers.is_empty() { 0 } else { 1 << 13 },
            timestamp_ms: self.timestamp_ms,
        }
        .encode(&mut bits)?;
        CharacterSpawnAssignment {
            sequence: self.sequence,
            timestamp_ms: self.timestamp_ms,
            spawn_point_index: catalog.default_spawn(),
        }
        .encode_object_for_player(&mut bits, player)?;
        SpawnAvailability::encode(&mut bits, catalog, all_supported)?;
        SquadSpawnAssignment {
            sequence: self.sequence,
            timestamp_ms: self.timestamp_ms,
            spawn_point_index: catalog.default_spawn(),
        }
        .encode_object_for_squad(&mut bits, catalog.squad_index())?;
        let mut vehicles = bunkers.to_vec();
        for vehicle in &mut vehicles {
            if all_supported {
                let point = catalog
                    .point(vehicle.spawn_point_index)
                    .ok_or("missing_bunker_spawn")?;
                if !matches!(point.kind, crate::spawn_catalog::SpawnKind::BunkerPassenger)
                    || !point.active
                    || Some(vehicle.faction) != catalog.map_pool().owner_faction
                    || vehicle.team != 1
                {
                    return Err("invalid_bunker_map_binding");
                }
            }
            if vehicle.spawn_point_index == catalog.default_spawn()
                && let Some((owner, seat)) = occupant
            {
                if ![0, 8].contains(&seat) {
                    return Err("invalid_bunker_seat");
                }
                vehicle.owner_index = owner;
                if seat == 0 {
                    vehicle.turret_owners[0] = owner;
                }
            }
        }
        if !vehicles.is_empty() {
            DefensiveEmplacementCreation::encode_objects(&mut bits, &vehicles)?;
        }
        Ok(bits.finish())
    }
}

/// Character field14 drives CameraPolicySpawn's selected-point preview.
/// Native ranged_int<0,127> uses seven bits, unlike Squad's u16 field11.
#[derive(Clone, Copy, Debug)]
pub struct CharacterSpawnAssignment {
    pub sequence: u32,
    pub timestamp_ms: u32,
    pub spawn_point_index: u16,
}
impl CharacterSpawnAssignment {
    fn encode_object_for_player(
        &self,
        bits: &mut BitWriter,
        player: u16,
    ) -> Result<(), &'static str> {
        if self.spawn_point_index >= 128 {
            return Err("invalid_alpha_spawn");
        }
        bits.write(1, 1)?;
        bits.write(player.into(), 16)?;
        for field in 0..22 {
            bits.write(u32::from(field == 14), 1)?;
            if field == 14 {
                bits.write(self.spawn_point_index.into(), 7)?;
            }
        }
        bits.write(0, 1)
    }
    pub fn encode(&self) -> Result<Vec<u8>, &'static str> {
        self.encode_with_profile(crate::wire_profile::WireProfile::prototype())
    }
    pub fn encode_with_profile(
        &self,
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        self.encode_for_player_with_profile(0, profile)
    }
    pub fn encode_for_player(&self, player: u16) -> Result<Vec<u8>, &'static str> {
        self.encode_for_player_with_profile(player, crate::wire_profile::WireProfile::prototype())
    }
    pub fn encode_for_player_with_profile(
        &self,
        player: u16,
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        let catalog =
            crate::spawn_catalog::SpawnCatalog::load(crate::spawn_catalog::default_path())
                .map_err(|_| "spawn_catalog_unavailable")?;
        self.encode_for_player_with_catalog_with_profile(player, &catalog, profile)
    }
    pub fn encode_for_player_with_catalog(
        &self,
        player: u16,
        catalog: &crate::spawn_catalog::SpawnCatalog,
    ) -> Result<Vec<u8>, &'static str> {
        self.encode_for_player_with_catalog_with_profile(
            player,
            catalog,
            crate::wire_profile::WireProfile::prototype(),
        )
    }
    pub fn encode_for_player_with_catalog_with_profile(
        &self,
        player: u16,
        catalog: &crate::spawn_catalog::SpawnCatalog,
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        if catalog.alpha(self.spawn_point_index).is_none() {
            return Err("invalid_alpha_spawn");
        }
        if self.sequence <= 1 {
            return Err("invalid_character_spawn_sequence");
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
        self.encode_object_for_player(&mut bits, player)?;
        Ok(bits.finish())
    }
}

#[cfg(test)]
mod objective_sequence_tests {
    use super::*;

    // SYNTHETIC IDs, states and timing values authored from codec constraints.
    fn snapshot() -> ObjectiveStateSnapshot {
        ObjectiveStateSnapshot {
            sequence: 37,
            timestamp_ms: 900,
            objectives: vec![
                ObjectiveStateDelta {
                    object_index: 61,
                    state_actor: ObjectiveStateActor {
                        state: ObjectiveState::Inactive,
                        change_time_ms: 700,
                        actor_index: 13,
                    },
                },
                ObjectiveStateDelta {
                    object_index: 117,
                    state_actor: ObjectiveStateActor {
                        state: ObjectiveState::Completed,
                        change_time_ms: 800,
                        actor_index: u16::MAX,
                    },
                },
            ],
        }
    }

    fn assert_fields(sequences: &[ObjectiveSequenceDelta]) {
        let packet = snapshot();
        let profiles = crate::wire_profile::WireProfileCatalog::embedded();
        for id in [20230, 20231] {
            let profile = profiles.profile(id).unwrap();
            let bytes = packet
                .encode_with_sequences_with_profile(sequences, profile)
                .unwrap();
            let header = ObjectUpdate::decode_with_profile(&bytes, profile)
                .unwrap()
                .header;
            assert_eq!(header.sequence, packet.sequence);
            assert_eq!(header.timestamp_ms, packet.timestamp_ms);
            assert_eq!(header.object_type_mask, 1 << 12);
            let mut bits = BitReader::with_profile(&bytes, profile);
            bits.position = profile.header_bits();
            for objective in &packet.objectives {
                assert_eq!(bits.read(1).unwrap(), 1);
                assert_eq!(bits.read(16).unwrap(), u32::from(objective.object_index));
                let sequence = sequences
                    .iter()
                    .find(|value| value.object_index == objective.object_index);
                for field in 0..11 {
                    let present = bits.read(1).unwrap();
                    match field {
                        2 => {
                            assert_eq!(present, 1);
                            assert_eq!(bits.read(8).unwrap(), objective.state_actor.state as u32);
                            assert_eq!(
                                bits.read(32).unwrap(),
                                objective.state_actor.change_time_ms
                            );
                            assert_eq!(
                                bits.read(16).unwrap(),
                                u32::from(objective.state_actor.actor_index)
                            );
                        }
                        9 | 10 if sequence.is_some() => {
                            assert_eq!(present, 1);
                            let sequence = sequence.unwrap();
                            assert_eq!(
                                bits.read(32).unwrap(),
                                if field == 9 {
                                    sequence.sequence_index as u32
                                } else {
                                    sequence.start_time_ms
                                }
                            );
                        }
                        _ => assert_eq!(present, 0),
                    }
                }
            }
            assert_eq!(bits.read(1).unwrap(), 0);
            assert!(bytes.len() * 8 - bits.position < 8);
        }
    }

    #[test]
    fn synthetic_objectives_preserve_state_without_implicit_sequences() {
        assert_fields(&[]);
        assert_eq!(
            snapshot().encode().unwrap(),
            snapshot().encode_with_sequences(&[]).unwrap()
        );
    }

    #[test]
    fn synthetic_objectives_preserve_caller_sequences_and_stop_sentinel() {
        assert_fields(&[
            ObjectiveSequenceDelta {
                object_index: 61,
                sequence_index: 7,
                start_time_ms: 500,
            },
            ObjectiveSequenceDelta {
                object_index: 117,
                sequence_index: -1,
                start_time_ms: 600,
            },
        ]);
    }

    #[test]
    fn synthetic_objectives_reject_invalid_links_and_indices() {
        let packet = snapshot();
        let sequence = ObjectiveSequenceDelta {
            object_index: 61,
            sequence_index: 7,
            start_time_ms: 500,
        };
        for sequences in [
            vec![ObjectiveSequenceDelta {
                object_index: 62,
                ..sequence
            }],
            vec![ObjectiveSequenceDelta {
                sequence_index: -2,
                ..sequence
            }],
            vec![sequence, sequence],
            vec![
                ObjectiveSequenceDelta {
                    object_index: 117,
                    ..sequence
                },
                sequence,
            ],
        ] {
            assert_eq!(
                packet.encode_with_sequences(&sequences),
                Err("invalid_objective_sequence_links")
            );
        }
        let mut packet = snapshot();
        packet.objectives.reverse();
        assert_eq!(packet.encode(), Err("invalid_objective_index_order"));
        packet.objectives = snapshot().objectives;
        packet.objectives[1].object_index = 128;
        assert_eq!(packet.encode(), Err("invalid_objective_index_order"));
        packet.objectives.clear();
        assert_eq!(packet.encode(), Err("empty_objective_snapshot"));
    }
}
