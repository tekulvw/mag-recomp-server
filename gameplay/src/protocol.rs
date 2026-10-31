//! Raw MAG plugin messages after SCERT/SDK framing has been removed.
//! Source: docs/protocol/dme-probe/CLIENT-FRAMING.md; no Medius70-byte header.

mod death_notification;
pub use death_notification::{HeadRegionHit, RetailDeathNotification, VictimWasMounted};

#[derive(Clone)]
pub struct ClaimReservation<'a> {
    pub np_identity: &'a [u8],
    pub clan: &'a [u8],
    pub local_ip: u32,
    pub public_key: &'a [u8],
    pub private_key: &'a [u8],
    pub symmetric_key: &'a [u8],
}
/// Retail NetMessageProtocolInfo (type 0x0002): BE u32 protocol then BE u32 build.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProtocolInfo {
    pub protocol: u32,
    pub build: u32,
}
impl ProtocolInfo {
    /// The retail client accepts exactly protocol 1740 (0x6CC); a mismatch fails its connect transaction.
    pub const RETAIL_PROTOCOL: u32 = 0x6cc;
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(10);
        bytes.extend_from_slice(&2u16.to_be_bytes());
        bytes.extend_from_slice(&self.protocol.to_be_bytes());
        bytes.extend_from_slice(&self.build.to_be_bytes());
        bytes
    }
}
/// Native TimeSinceAppStartInMilliSeconds uses wrapping u32 milliseconds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimeRequest {
    pub client_time_ms: u32,
}
#[derive(Debug)]
pub struct TimeResponse {
    pub server_time_ms: u32,
    pub client_time_ms: u32,
}
impl TimeResponse {
    pub fn encode(&self) -> Vec<u8> {
        self.encode_with_profile(crate::wire_profile::WireProfile::prototype())
    }
    pub fn encode_with_profile(&self, profile: crate::wire_profile::WireProfile) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(10);
        bytes.extend_from_slice(&profile.wire_message_type(0x000b).to_be_bytes());
        bytes.extend_from_slice(&self.server_time_ms.to_be_bytes());
        bytes.extend_from_slice(&self.client_time_ms.to_be_bytes());
        bytes
    }
}
/// Byte-aligned native NetClientGameInfoMessage, without a Medius header.
#[derive(Clone, Debug)]
pub struct GameInfo {
    pub client_index: u16,
    pub level_name: String,
    pub start_time_ms: u32,
    pub attacking_faction: u8,
    pub defending_faction: u8,
    pub maximum_points: [i16; 2],
    pub current_points: [i16; 2],
    pub game_type: i8,
}
impl GameInfo {
    pub fn encode(&self) -> Result<Vec<u8>, &'static str> {
        self.encode_with_profile(crate::wire_profile::WireProfile::prototype())
    }
    pub fn encode_with_profile(
        &self,
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        // Stay below the native SDK's unfragmented plugin boundary.
        if self.level_name.len() > 1397 - 21 {
            return Err("game_info_too_long");
        }
        let mut bytes = Vec::with_capacity(21 + self.level_name.len());
        bytes.extend_from_slice(&profile.wire_message_type(0x0004).to_be_bytes());
        bytes.extend_from_slice(&self.client_index.to_be_bytes());
        bytes.extend_from_slice(&(self.level_name.len() as u16).to_be_bytes());
        bytes.extend_from_slice(self.level_name.as_bytes());
        bytes.extend_from_slice(&self.start_time_ms.to_be_bytes());
        bytes.extend_from_slice(&[self.attacking_faction, self.defending_faction]);
        for score in self.maximum_points.iter().chain(self.current_points.iter()) {
            bytes.extend_from_slice(&score.to_be_bytes());
        }
        bytes.push(self.game_type as u8);
        Ok(bytes)
    }
}
#[derive(Clone, Debug)]
pub struct ClientReady {
    pub ready: bool,
}
/// Native NetMessageSelectLoadout serializes its integer member as signed char.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SelectLoadout {
    pub index: i8,
}
impl SelectLoadout {
    /// Native SetDesiredInventoryIndex stores only the low three bits.
    /// Inventory validity/equipping is deferred until respawn by the native server.
    pub fn desired_inventory_index(&self) -> u8 {
        (self.index as u8) & 7
    }
}
/// NetClientImpl::EnterGame sends only its native message type0006.
#[derive(Clone, Debug)]
pub struct EnterGame;
/// Native selection is a u16 identifier, not a position or map asset hash.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CharacterSpawnSelection {
    pub spawn_point_index: u16,
}
/// NetMessageSquadSpawnSelection: native receiver uses the point only.
/// The UI leaves alt_insertion_index uninitialized on this path; retain it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SquadSpawnSelection {
    pub spawn_point_index: u16,
    pub alt_insertion_index: u8,
}
/// Native confirmation carries no body; physical respawn follows through replication.
#[derive(Clone, Debug)]
pub struct ConfirmRespawn;
/// Native NetMessageCancelRespawn0213 has no body.
#[derive(Clone, Debug)]
pub struct CancelRespawn;
/// Native m_result is a packed boolean, not the Flash NetResult enum.
/// The client converts true to Flash success0 and false to error1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CancelRespawnResult {
    pub success: bool,
}
impl CancelRespawnResult {
    pub fn encode(self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(3);
        bytes.extend_from_slice(&0x0214u16.to_be_bytes());
        // One MSB-first bit with seven zero padding bits (17 meaningful bits).
        bytes.push(u8::from(self.success) << 7);
        bytes
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PingRequest {
    pub client_time_ms: u32,
}
#[derive(Debug)]
pub struct PingResponse {
    pub client_time_ms: u32,
}
impl PingResponse {
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(6);
        bytes.extend_from_slice(&0x0209u16.to_be_bytes());
        bytes.extend_from_slice(&self.client_time_ms.to_be_bytes());
        bytes
    }
}
/// Native ScoreChangeReason values; reason7 is a sentinel, never a wire reason.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ScoreChangeReason {
    LightReinforcement = 0,
    MediumReinforcement = 1,
    HeavyReinforcement = 2,
    PrimaryObjective = 3,
    TimedBleed = 4,
    Initial = 5,
    Death = 6,
}
/// NetMessageScoreChange0207 carries an absolute score, not a decrement.
#[derive(Clone, Copy, Debug)]
pub struct ScoreChange {
    pub team: u8,
    pub points: i16,
    pub reason: ScoreChangeReason,
}
impl ScoreChange {
    pub fn encode(&self) -> Result<Vec<u8>, &'static str> {
        self.encode_with_profile(crate::wire_profile::WireProfile::prototype())
    }
    pub fn encode_with_profile(
        &self,
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        let mut bits = crate::replication::BitWriter::with_profile(profile);
        bits.write(0x0207, 16)?;
        bits.write(self.team.into(), 1)?;
        bits.write(self.points as u16 as u32, 16)?;
        bits.write(self.reason as u32, 3)?;
        Ok(bits.finish())
    }
}
/// Native NetMessageStatUpdate0204: low16 object index, byte count and
/// byte stat/absolute value entries using the selected build's value encoding.
/// Stat2 is cumulative match XP; other native stat values share this codec.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StatValue {
    pub stat: u8,
    pub value: i32,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatUpdate {
    pub character_index: u16,
    pub stats: Vec<StatValue>,
}
impl StatUpdate {
    pub fn encode(&self) -> Result<Vec<u8>, &'static str> {
        self.encode_with_profile(crate::wire_profile::WireProfile::prototype())
    }
    pub fn encode_with_profile(
        &self,
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        if self.character_index == u16::MAX || self.stats.is_empty() || self.stats.len() > 255 {
            return Err("invalid_stat_update");
        }
        let mut seen = std::collections::BTreeSet::new();
        let mut wire = vec![2, 4];
        wire.extend(self.character_index.to_be_bytes());
        wire.push(self.stats.len() as u8);
        for entry in &self.stats {
            if !seen.insert(entry.stat) {
                return Err("duplicate_stat_update");
            };
            wire.push(entry.stat);
            match profile.stat_encoding() {
                crate::wire_profile::StatEncoding::FixedBeI32 => {
                    wire.extend(entry.value.to_be_bytes())
                }
                crate::wire_profile::StatEncoding::UnsignedContinuationU32 => {
                    let mut value = entry.value as u32;
                    loop {
                        let byte = (value & 127) as u8;
                        value >>= 7;
                        wire.push(byte | if value == 0 { 0 } else { 128 });
                        if value == 0 {
                            break;
                        }
                    }
                }
            }
        }
        Ok(wire)
    }
}

/// Native0210 launch request: scalar IDs after the10-bit power are unaligned.
/// This describes a projectile launch, not an explosion or a damage claim.
#[derive(Clone, Copy, Debug)]
pub struct FireGrenade {
    pub position: [f32; 3],
    pub rotation_xyzw: [f32; 4],
    pub initial_velocity: [f32; 3],
    pub ammo_index: u8,
    pub throw_power: f32,
    pub target_id: u32,
    pub shooter_id: u32,
    pub shooter_client_index: u16,
    pub projectile_index: u16,
}
impl FireGrenade {
    fn decode(
        bytes: &[u8],
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Self, &'static str> {
        if bytes.len() != 57 {
            return Err("invalid_grenade_message_length");
        }
        let mut bits = crate::replication::BitReader::with_profile(bytes, profile);
        if bits.read(16)? != 0x0210 {
            return Err("wrong_grenade_message");
        }
        fn vector<const N: usize>(
            bits: &mut crate::replication::BitReader<'_>,
        ) -> Result<[f32; N], &'static str> {
            let mut values = [0.0; N];
            for value in &mut values {
                *value = f32::from_bits(bits.read(32)?);
                if !value.is_finite() {
                    return Err("nonfinite_grenade_vector");
                }
            }
            Ok(values)
        }
        let position = vector(&mut bits)?;
        let rotation_xyzw = vector(&mut bits)?;
        let initial_velocity = vector(&mut bits)?;
        let ammo_index = bits.read(8)? as u8;
        let power = bits.read(10)?;
        if power > 511 {
            return Err("grenade_power_out_of_range");
        }
        let request = Self {
            position,
            rotation_xyzw,
            initial_velocity,
            ammo_index,
            throw_power: power as f32 / 511.0,
            target_id: bits.read(32)?,
            shooter_id: bits.read(32)?,
            shooter_client_index: bits.read(16)? as u16,
            projectile_index: bits.read(16)? as u16,
        };
        if bits.read(6)? != 0 {
            return Err("nonzero_grenade_padding");
        }
        Ok(request)
    }
}
/// Native NetMessageHeal021b: target handles followed by the equipped gear slot.
#[derive(Clone, Debug)]
pub struct Heal {
    pub target_ids: Vec<u32>,
    pub gear_slot: u8,
}
/// Native NetMessageKillSelf0215: the full owned Character handle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KillSelf {
    pub target_character_id: u32,
}
/// Native NetMessageMountSeat0216: three full-width big-endian IDs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MountSeat {
    pub vehicle_id: u32,
    pub seat_id: u32,
    pub character_id: u32,
}
/// Native NetMessageDisMountSeat0217 adds the client's camera yaw word.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DismountSeat {
    pub vehicle_id: u32,
    pub seat_id: u32,
    pub character_id: u32,
    pub camera_yaw: u32,
}
impl DismountSeat {
    /// Native degrees0..360 -> f32 radians ->10-bit yaw. The native fallback
    /// uses vehicle rotation; this local probe uses its on-foot spawn yaw0.
    pub fn packed_yaw(&self) -> u16 {
        if self.camera_yaw > 360 {
            return 0;
        }
        let radians = self.camera_yaw as f32 * 0.017_453_292_f32;
        ((radians / std::f32::consts::TAU * 1024.0) as u16) & 0x3ff
    }
}
/// Native NetMessageRepair serializes only the target; actor/tool/amount come
/// from the authenticated character and current inventory (Marshal0014a848).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Repair {
    pub target_object_id: u32,
}
/// NetMessageObjectiveMessage03EA interaction operation (DEV ctor00435C08).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ObjectiveInteractionKind {
    Activate = 0,
    Deactivate = 1,
    Ack = 2,
    Nak = 3,
    Release = 4,
    ActivatingOnExit = 5,
    Destroy = 6,
    LookAt = 7,
    LookAway = 8,
}
impl TryFrom<u8> for ObjectiveInteractionKind {
    type Error = &'static str;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Activate),
            1 => Ok(Self::Deactivate),
            2 => Ok(Self::Ack),
            3 => Ok(Self::Nak),
            4 => Ok(Self::Release),
            5 => Ok(Self::ActivatingOnExit),
            6 => Ok(Self::Destroy),
            7 => Ok(Self::LookAt),
            8 => Ok(Self::LookAway),
            _ => Err("invalid_objective_interaction_kind"),
        }
    }
}
/// ObjectInteraction target followed by subtype, actor and trigger (Marshal00434370).
/// Message ID03EA is unchanged in retail; actor ownership is checked by the session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObjectiveInteraction {
    pub target_object_id: u32,
    pub kind: ObjectiveInteractionKind,
    pub actor_index: u16,
    pub trigger_index: u8,
}
impl ObjectiveInteraction {
    pub fn decode(bytes: &[u8]) -> Result<Self, &'static str> {
        let mut r = Reader { bytes, pos: 0 };
        if r.u16()? != 0x03ea {
            return Err("invalid_objective_interaction_message_type");
        }
        let message = Self {
            target_object_id: r.u32()?,
            kind: ObjectiveInteractionKind::try_from(r.take(1)?[0])?,
            actor_index: r.u16()?,
            trigger_index: r.take(1)?[0],
        };
        r.finish()?;
        Ok(message)
    }

    pub fn encode(self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(10);
        bytes.extend_from_slice(&0x03eau16.to_be_bytes());
        bytes.extend_from_slice(&self.target_object_id.to_be_bytes());
        bytes.push(self.kind as u8);
        bytes.extend_from_slice(&self.actor_index.to_be_bytes());
        bytes.push(self.trigger_index);
        bytes
    }
}
/// Native global Squad pool index, not a faction-local ordinal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChangePlayerSquad {
    pub squad_index: u8,
}
/// Retail NetMessageChangePlayerNominations 0x0243: one u8 of squad/platoon/OIC
/// bits (1/2/4). The server sends no result message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChangePlayerNominations {
    pub nominations: u8,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChangePlayerSquadResult {
    pub success: bool,
}
impl ChangePlayerSquadResult {
    pub fn encode(self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(3);
        bytes.extend_from_slice(&0x0203u16.to_be_bytes());
        bytes.push(u8::from(self.success) << 7);
        bytes
    }
}
#[derive(Clone)]
pub enum GameMessage<'a> {
    ClaimReservation(ClaimReservation<'a>),
    /// Retail NetMessageProtocolInfo: the client's reply to the server's handshake.
    ProtocolInfo(ProtocolInfo),
    PluginGreeting,
    TimeRequest(TimeRequest),
    ClientReady(ClientReady),
    SelectLoadout(SelectLoadout),
    EnterGame(EnterGame),
    CharacterSpawnSelection(CharacterSpawnSelection),
    SquadSpawnSelection(SquadSpawnSelection),
    ChangePlayerSquad(ChangePlayerSquad),
    ChangePlayerNominations(ChangePlayerNominations),
    ConfirmRespawn(ConfirmRespawn),
    CancelRespawn(CancelRespawn),
    Hit(crate::bullet_hit::NetMessageHit),
    CommandAbility(crate::active_abilities::CommandRequest),
    StrategicAbility(crate::active_abilities::StrategicRequest),
    FireGrenade(FireGrenade),
    Heal(Heal),
    Repair(Repair),
    ObjectiveInteraction(ObjectiveInteraction),
    KillSelf(KillSelf),
    MountSeat(MountSeat),
    DismountSeat(DismountSeat),
    PingRequest(PingRequest),
    ObjectUpdate(crate::replication::ObjectUpdate<'a>),
    Unknown {
        message_type: u16,
        body: &'a [u8],
    },
}
struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], &'static str> {
        let end = self.pos.checked_add(n).ok_or("length_overflow")?;
        let slice = self
            .bytes
            .get(self.pos..end)
            .ok_or("truncated_game_message")?;
        self.pos = end;
        Ok(slice)
    }
    fn u16(&mut self) -> Result<u16, &'static str> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }
    fn u32(&mut self) -> Result<u32, &'static str> {
        let b = self.take(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn finish(&self) -> Result<(), &'static str> {
        if self.pos == self.bytes.len() {
            Ok(())
        } else {
            Err("trailing_game_message_bytes")
        }
    }
    fn string(&mut self) -> Result<&'a [u8], &'static str> {
        let len = self.u16()? as usize;
        self.take(len)
    }
}
impl<'a> GameMessage<'a> {
    pub fn decode(bytes: &'a [u8]) -> Result<Self, &'static str> {
        Self::decode_with_profile(bytes, crate::wire_profile::WireProfile::prototype())
    }
    pub fn decode_with_profile(
        bytes: &'a [u8],
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Self, &'static str> {
        let mut r = Reader { bytes, pos: 0 };
        let wire_type = r.u16()?;
        if profile.requires_protocol_info() && wire_type == 0x0002 {
            let protocol = r.u32()?;
            let build = r.u32()?;
            r.finish()?;
            return Ok(Self::ProtocolInfo(ProtocolInfo { protocol, build }));
        }
        let Some(message_type) = profile.prototype_message_type(wire_type) else {
            return Ok(Self::Unknown {
                message_type: wire_type,
                body: &bytes[r.pos..],
            });
        };
        match message_type {
            0x0248 => Ok(Self::CommandAbility(
                crate::active_abilities::CommandRequest::parse(bytes)?,
            )),
            0x0249 => Ok(Self::StrategicAbility(
                crate::active_abilities::StrategicRequest::parse(bytes)?,
            )),
            0x0204 => Err("server_only_stat_update"),
            0x0003 => {
                let np_identity = r.take(36)?;
                let clan = r.string()?;
                let ip = r.take(4)?;
                let local_ip = u32::from_be_bytes([ip[0], ip[1], ip[2], ip[3]]);
                let public_key = r.string()?;
                let private_key = r.string()?;
                let symmetric_key = r.string()?;
                if r.pos != bytes.len() {
                    return Err("trailing_game_message_bytes");
                }
                Ok(Self::ClaimReservation(ClaimReservation {
                    np_identity,
                    clan,
                    local_ip,
                    public_key,
                    private_key,
                    symmetric_key,
                }))
            }
            0x0007 => Ok(Self::ObjectUpdate(
                crate::replication::ObjectUpdate::decode_with_profile(bytes, profile)?,
            )),
            0x000a => {
                let client_time_ms = r.u32()?;
                r.finish()?;
                Ok(Self::TimeRequest(TimeRequest { client_time_ms }))
            }
            0x0005 => {
                let ready = r.take(1)?[0] & 0x80 != 0;
                r.finish()?;
                Ok(Self::ClientReady(ClientReady { ready }))
            }
            0x0006 => {
                r.finish()?;
                Ok(Self::EnterGame(EnterGame))
            }
            0x020f => Ok(Self::Hit(crate::bullet_hit::NetMessageHit::decode(bytes)?)),
            0x0210 => Ok(Self::FireGrenade(FireGrenade::decode(bytes, profile)?)),
            0x021b => {
                if bytes.len() < 5 {
                    return Err("invalid_heal_message_length");
                }
                let count = u16::from_be_bytes([bytes[2], bytes[3]]) as usize;
                if count > 64 || bytes.len() != 5 + 4 * count {
                    return Err("invalid_heal_message_length");
                }
                let target_ids = bytes[4..4 + count * 4]
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|&b| u32::from_be_bytes(b))
                    .collect();
                Ok(Self::Heal(Heal {
                    target_ids,
                    gear_slot: bytes[4 + count * 4],
                }))
            }
            0x03ea => Ok(Self::ObjectiveInteraction(ObjectiveInteraction::decode(
                bytes,
            )?)),
            0x021d => {
                let request = Repair {
                    target_object_id: r.u32()?,
                };
                r.finish()?;
                Ok(Self::Repair(request))
            }
            0x0215 => {
                let request = KillSelf {
                    target_character_id: r.u32()?,
                };
                r.finish()?;
                Ok(Self::KillSelf(request))
            }
            0x0216 => {
                let request = MountSeat {
                    vehicle_id: r.u32()?,
                    seat_id: r.u32()?,
                    character_id: r.u32()?,
                };
                r.finish()?;
                Ok(Self::MountSeat(request))
            }
            0x0217 => {
                let request = DismountSeat {
                    vehicle_id: r.u32()?,
                    seat_id: r.u32()?,
                    character_id: r.u32()?,
                    camera_yaw: r.u32()?,
                };
                r.finish()?;
                Ok(Self::DismountSeat(request))
            }
            0x0202 => {
                let request = ChangePlayerSquad {
                    squad_index: r.take(1)?[0],
                };
                r.finish()?;
                Ok(Self::ChangePlayerSquad(request))
            }
            0x0243 => {
                let request = ChangePlayerNominations {
                    nominations: r.take(1)?[0],
                };
                r.finish()?;
                Ok(Self::ChangePlayerNominations(request))
            }
            0x0218 => {
                let request = SquadSpawnSelection {
                    spawn_point_index: r.u16()?,
                    alt_insertion_index: r.take(1)?[0],
                };
                r.finish()?;
                Ok(Self::SquadSpawnSelection(request))
            }
            0x0219 => {
                let spawn_point_index = r.u16()?;
                r.finish()?;
                Ok(Self::CharacterSpawnSelection(CharacterSpawnSelection {
                    spawn_point_index,
                }))
            }
            0x0213 => {
                r.finish()?;
                Ok(Self::CancelRespawn(CancelRespawn))
            }
            0x0211 => {
                r.finish()?;
                Ok(Self::ConfirmRespawn(ConfirmRespawn))
            }
            0x0208 => {
                let client_time_ms = r.u32()?;
                r.finish()?;
                Ok(Self::PingRequest(PingRequest { client_time_ms }))
            }
            0x0252 => {
                let index = r.take(1)?[0] as i8;
                r.finish()?;
                Ok(Self::SelectLoadout(SelectLoadout { index }))
            }
            0x0010 => {
                if r.pos != bytes.len() {
                    return Err("trailing_game_message_bytes");
                }
                Ok(Self::PluginGreeting)
            }
            _ => Ok(Self::Unknown {
                message_type,
                body: &bytes[r.pos..],
            }),
        }
    }
}
/// Caller has already validated the hex representation and payload bound.
pub(crate) fn decode_hex(hex: &str) -> Vec<u8> {
    fn digit(b: u8) -> u8 {
        match b {
            b'0'..=b'9' => b - b'0',
            b'a'..=b'f' => b - b'a' + 10,
            b'A'..=b'F' => b - b'A' + 10,
            _ => unreachable!("validated hex"),
        }
    }
    hex.as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| digit(b[0]) * 16 + digit(b[1]))
        .collect()
}

/// Hex is only the private bridge representation; native serializers own the wire layout.
pub(crate) fn encode_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut hex = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        hex.push(DIGITS[(b >> 4) as usize] as char);
        hex.push(DIGITS[(b & 15) as usize] as char);
    }
    hex
}

#[cfg(test)]
mod objective_interaction_tests {
    use super::*;

    #[test]
    fn objective_interaction_captured_retail_packet_is_known() {
        let packet = decode_hex("03ea0c80000000000000");
        let profile = crate::wire_profile::WireProfileCatalog::embedded()
            .profile(20231)
            .unwrap();
        let GameMessage::ObjectiveInteraction(message) =
            GameMessage::decode_with_profile(&packet, profile).unwrap()
        else {
            panic!("captured retail ObjectiveMessage03EA must decode as a named message");
        };
        assert_eq!(
            message,
            ObjectiveInteraction {
                target_object_id: 0x0c800000,
                kind: ObjectiveInteractionKind::Activate,
                actor_index: 0,
                trigger_index: 0,
            }
        );
        assert_eq!(message.encode(), packet);
    }

    #[test]
    fn objective_interaction_exact_ack_release_and_nak_fixtures() {
        let ack = ObjectiveInteraction {
            target_object_id: 0x0c800000,
            kind: ObjectiveInteractionKind::Ack,
            actor_index: 0,
            trigger_index: 0,
        };
        let ack_bytes = decode_hex("03ea0c80000002000000");
        assert_eq!(ack.encode(), ack_bytes);
        assert_eq!(ObjectiveInteraction::decode(&ack_bytes), Ok(ack));

        let release_bytes = decode_hex("03ea0c80000004123456");
        let release = ObjectiveInteraction {
            kind: ObjectiveInteractionKind::Release,
            actor_index: 0x1234,
            trigger_index: 0x56,
            ..ack
        };
        assert_eq!(ObjectiveInteraction::decode(&release_bytes), Ok(release));
        assert_eq!(release.encode(), release_bytes);
        let nak = ObjectiveInteraction {
            kind: ObjectiveInteractionKind::Nak,
            actor_index: 0x1234,
            trigger_index: 0xff,
            ..ack
        };
        assert_eq!(nak.encode(), decode_hex("03ea0c800000031234ff"));
    }

    #[test]
    fn objective_interaction_rejects_truncation_trailing_and_bad_kind() {
        let packet = decode_hex("03ea0c80000000000000");
        let profile = crate::wire_profile::WireProfileCatalog::embedded()
            .profile(20231)
            .unwrap();
        for length in 0..packet.len() {
            assert!(ObjectiveInteraction::decode(&packet[..length]).is_err());
            assert!(GameMessage::decode_with_profile(&packet[..length], profile).is_err());
        }
        let mut trailing = packet.clone();
        trailing.push(0);
        assert_eq!(
            ObjectiveInteraction::decode(&trailing),
            Err("trailing_game_message_bytes")
        );
        assert!(GameMessage::decode_with_profile(&trailing, profile).is_err());
        let mut bad_kind = packet.clone();
        bad_kind[6] = 9;
        assert_eq!(
            ObjectiveInteraction::decode(&bad_kind),
            Err("invalid_objective_interaction_kind")
        );
        assert!(GameMessage::decode_with_profile(&bad_kind, profile).is_err());
        let mut wrong_type = packet;
        wrong_type[1] = 0xeb;
        assert_eq!(
            ObjectiveInteraction::decode(&wrong_type),
            Err("invalid_objective_interaction_message_type")
        );
    }
}

/// Original retail NetMessageEndOfGame01FE: 1-bit team, 1-bit major flag,
/// absolute u32 deadline, padded to a whole byte.
#[derive(Clone, Copy, Debug)]
pub struct EndOfGame {
    pub winning_team: u8,
    pub major_victory: bool,
    pub end_time_ms: u32,
}
impl EndOfGame {
    pub fn encode_with_profile(
        &self,
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        if self.winning_team > 1 {
            return Err("invalid_winning_team");
        }
        let mut b = crate::replication::BitWriter::with_profile(profile);
        b.write(0x01fe, 16)?;
        b.write(self.winning_team.into(), 1)?;
        b.write(u32::from(self.major_victory), 1)?;
        b.write(self.end_time_ms, 32)?;
        Ok(b.finish())
    }
}
/// Original NetMessageCharacterStatsComplete0205 has no payload.
#[derive(Clone, Copy, Debug)]
pub struct CharacterStatsComplete;
impl CharacterStatsComplete {
    pub fn encode(self) -> Vec<u8> {
        0x0205_u16.to_be_bytes().to_vec()
    }
}
