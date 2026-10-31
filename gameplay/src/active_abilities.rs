//! Original retail command codecs, authored availability and cooldown bookkeeping.
//!
//! No strike, response, delayed execution or ability replication is implemented.
//! The caller must require effect authority after preflight. Cooldowns are recorded
//! only after an authoritative effect has actually been accepted, never on receipt
//! of an untrusted request. See RETAIL-ACTIVE-ABILITIES-NATIVE.md.

use serde::{Deserialize, Serialize};
use std::{io, path::Path};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub enum Ability {
    None,
    ClusterBomb,
    PrecisionStrike,
    Sensor,
    GasBombardment,
    StrafingRun,
    GuidedArtillery,
}

impl Ability {
    pub fn code(self) -> u16 {
        match self {
            Self::None => 0,
            Self::ClusterBomb => 1,
            Self::PrecisionStrike => 2,
            Self::Sensor => 3,
            Self::GasBombardment => 4,
            Self::StrafingRun => 5,
            Self::GuidedArtillery => 6,
        }
    }

    pub fn from_code(code: u16) -> Result<Self, &'static str> {
        match code {
            0 => Ok(Self::None),
            1 => Ok(Self::ClusterBomb),
            2 => Ok(Self::PrecisionStrike),
            3 => Ok(Self::Sensor),
            4 => Ok(Self::GasBombardment),
            5 => Ok(Self::StrafingRun),
            6 => Ok(Self::GuidedArtillery),
            _ => Err("unknown_command_ability"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CommandRequest {
    pub squad_index: u16,
    pub ability: Ability,
    pub position: [f32; 3],
    /// Retained as the native u32 field. Never used as a trusted server clock.
    pub request_time: u32,
    pub target_id: u32,
}

impl CommandRequest {
    pub fn parse(raw: &[u8]) -> Result<Self, &'static str> {
        if raw.len() != 26 || raw[..2] != [0x02, 0x48] {
            return Err("invalid_command_ability_message");
        }
        let value = Self {
            squad_index: u16::from_be_bytes([raw[2], raw[3]]),
            ability: Ability::from_code(u16::from_be_bytes([raw[4], raw[5]]))?,
            position: std::array::from_fn(|i| {
                f32::from_be_bytes(raw[6 + i * 4..10 + i * 4].try_into().unwrap())
            }),
            request_time: u32::from_be_bytes(raw[18..22].try_into().unwrap()),
            target_id: u32::from_be_bytes(raw[22..26].try_into().unwrap()),
        };
        if !value.position.iter().all(|v| v.is_finite()) {
            return Err("invalid_command_ability_position");
        }
        Ok(value)
    }

    pub fn to_bytes(self) -> Result<[u8; 26], &'static str> {
        if !self.position.iter().all(|v| v.is_finite()) {
            return Err("invalid_command_ability_position");
        }
        let mut raw = [0; 26];
        raw[..2].copy_from_slice(&[0x02, 0x48]);
        raw[2..4].copy_from_slice(&self.squad_index.to_be_bytes());
        raw[4..6].copy_from_slice(&self.ability.code().to_be_bytes());
        for (i, value) in self.position.iter().enumerate() {
            raw[6 + i * 4..10 + i * 4].copy_from_slice(&value.to_be_bytes());
        }
        raw[18..22].copy_from_slice(&self.request_time.to_be_bytes());
        raw[22..26].copy_from_slice(&self.target_id.to_be_bytes());
        Ok(raw)
    }
}

/// Original retail 0249 has only one u16 body field. Strategic enum meaning and
/// execution are intentionally unresolved; retain the code for diagnostics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StrategicRequest {
    pub ability_code: u16,
}

impl StrategicRequest {
    pub fn parse(raw: &[u8]) -> Result<Self, &'static str> {
        if raw.len() != 4 || raw[..2] != [0x02, 0x49] {
            return Err("invalid_strategic_ability_message");
        }
        Ok(Self {
            ability_code: u16::from_be_bytes([raw[2], raw[3]]),
        })
    }

    pub fn to_bytes(self) -> [u8; 4] {
        let value = self.ability_code.to_be_bytes();
        [0x02, 0x49, value[0], value[1]]
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Attacker,
    Defender,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandRole {
    SquadLeader,
    PlatoonLeader,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Authority {
    pub effects: bool,
    pub target_validation: bool,
    pub ability_replication: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mode {
    pub players: u16,
    pub side: Side,
    pub squad: Vec<Ability>,
    pub platoon: Vec<Ability>,
    pub strategic: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandProperty {
    pub ability: Ability,
    pub source_member: String,
    pub source_sha256: String,
    pub cooldown_ms: u64,
    /// Authored delay only, not proof of an impact/activation schedule.
    pub delay_ms: u64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StrategicProperty {
    pub ability: String,
    pub source_member: String,
    pub source_sha256: String,
    pub cooldown_ms: u64,
    pub duration_ms: u64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rules {
    pub schema_version: u32,
    pub application_id: u32,
    pub rules_sha256: String,
    pub source_archive_sha256: String,
    pub global_source_member: String,
    pub global_source_sha256: String,
    pub command_global_cooldown_ms: u64,
    pub strategic_global_cooldown_ms: u64,
    pub authority: Authority,
    pub modes: Vec<Mode>,
    pub commands: Vec<CommandProperty>,
    pub strategic: Vec<StrategicProperty>,
}

fn digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|v| v.is_ascii_hexdigit())
}
fn duration(value: u64) -> bool {
    value > 0 && value < u32::MAX as u64
}
fn unique<T: PartialEq>(values: &[T]) -> bool {
    values
        .iter()
        .enumerate()
        .all(|(i, value)| !values[..i].contains(value))
}

impl Rules {
    pub fn load(path: impl AsRef<Path>) -> io::Result<Self> {
        let result: Self = serde_json::from_reader(std::fs::File::open(path)?)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        result
            .validate()
            .map_err(|reason| io::Error::new(io::ErrorKind::InvalidData, reason))?;
        Ok(result)
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        if self.schema_version != 1
            || self.application_id != 20231
            || !digest(&self.rules_sha256)
            || !digest(&self.source_archive_sha256)
            || !digest(&self.global_source_sha256)
            || self.global_source_member.is_empty()
            || !duration(self.command_global_cooldown_ms)
            || !duration(self.strategic_global_cooldown_ms)
            || self.authority.effects
            || self.authority.target_validation
            || self.authority.ability_replication
            || self.commands.len() != 6
            || self.modes.len() != 6
        {
            return Err("invalid_retail_ability_rules");
        }
        let commands: Vec<_> = self.commands.iter().map(|p| p.ability).collect();
        if !unique(&commands)
            || commands.contains(&Ability::None)
            || self.commands.iter().any(|p| {
                !duration(p.cooldown_ms)
                    || !duration(p.delay_ms)
                    || !digest(&p.source_sha256)
                    || p.source_member.is_empty()
            })
        {
            return Err("invalid_retail_command_properties");
        }
        let modes: Vec<_> = self.modes.iter().map(|m| (m.players, m.side)).collect();
        if !unique(&modes)
            || self.modes.iter().any(|m| {
                ![64, 128, 256].contains(&m.players)
                    || !unique(&m.squad)
                    || !unique(&m.platoon)
                    || !unique(&m.strategic)
                    || m.squad
                        .iter()
                        .chain(&m.platoon)
                        .any(|a| !commands.contains(a))
                    || m.strategic
                        .iter()
                        .any(|name| !self.strategic.iter().any(|p| &p.ability == name))
            })
        {
            return Err("invalid_retail_ability_modes");
        }
        let strategic: Vec<_> = self.strategic.iter().map(|p| &p.ability).collect();
        if !unique(&strategic)
            || self.strategic.iter().any(|p| {
                p.ability.is_empty()
                    || !duration(p.cooldown_ms)
                    || !duration(p.duration_ms)
                    || !digest(&p.source_sha256)
                    || p.source_member.is_empty()
            })
        {
            return Err("invalid_retail_strategic_properties");
        }
        Ok(())
    }

    pub fn property(&self, ability: Ability) -> Option<&CommandProperty> {
        self.commands.iter().find(|p| p.ability == ability)
    }

    /// Select by authored mode capacity, never current connected population.
    pub fn allowed(&self, players: u16, side: Side, role: CommandRole) -> Option<&[Ability]> {
        self.modes
            .iter()
            .find(|m| m.players == players && m.side == side)
            .map(|mode| match role {
                CommandRole::SquadLeader => mode.squad.as_slice(),
                CommandRole::PlatoonLeader => mode.platoon.as_slice(),
            })
    }

    /// Local admission checks. Does not establish native target/asset validity or
    /// consume a cooldown. Context must come from the owned authoritative roster.
    pub fn preflight(
        &self,
        request: &CommandRequest,
        context: &CommandContext,
    ) -> Result<(), Denial> {
        if !request.position.iter().all(|v| v.is_finite()) {
            return Err(Denial::InvalidPosition);
        }
        if !context.alive {
            return Err(Denial::NotAlive);
        }
        if context.owned_squad != request.squad_index
            || context.owned_platoon != context.requested_platoon
        {
            return Err(Denial::WrongScope);
        }
        let role = context.role.ok_or(Denial::LeaderRequired)?;
        if !self
            .allowed(context.players, context.side, role)
            .is_some_and(|a| a.contains(&request.ability))
        {
            return Err(Denial::UnavailableInMode);
        }
        Ok(())
    }

    /// Remains denied until effects, target predicates and peer state are proved
    /// and implemented. Editing an authority bit alone cannot enable this module.
    pub fn require_effect_authority(&self) -> Result<(), Denial> {
        Err(Denial::EffectAuthorityPending)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct CommandContext {
    pub owned_squad: u16,
    pub owned_platoon: u16,
    pub requested_platoon: u16,
    pub alive: bool,
    pub role: Option<CommandRole>,
    pub players: u16,
    pub side: Side,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Denial {
    InvalidPosition,
    NotAlive,
    WrongScope,
    LeaderRequired,
    UnavailableInMode,
    EffectAuthorityPending,
    DuplicateActivation,
    ClockMovedBackwards,
    Cooldown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    Squad(u16),
    Platoon,
}

#[derive(Clone, Debug)]
struct ScopeCooldowns {
    scope: Scope,
    last_by_ability: [Option<u64>; 6],
}

/// One ledger per authoritative team/platoon/round. Bounded by the roster supplied
/// at construction, with six fixed entries per scope and no request history.
/// Retain it across leader replacement, disconnect, death and reconnect.
/// This bookkeeping does not create, accept, schedule or cancel an effect.
#[derive(Clone, Debug)]
pub struct Cooldowns {
    scopes: Vec<ScopeCooldowns>,
    last_global: Option<u64>,
    last_activation_ordinal: Option<u64>,
}

impl Cooldowns {
    pub fn new(squad_ids: &[u16]) -> Result<Self, Denial> {
        if !unique(squad_ids) {
            return Err(Denial::WrongScope);
        }
        let scopes = std::iter::once(Scope::Platoon)
            .chain(squad_ids.iter().copied().map(Scope::Squad))
            .map(|scope| ScopeCooldowns {
                scope,
                last_by_ability: [None; 6],
            })
            .collect();
        Ok(Self {
            scopes,
            last_global: None,
            last_activation_ordinal: None,
        })
    }

    pub fn remaining_ms(
        &self,
        rules: &Rules,
        now: u64,
        scope: Scope,
        ability: Ability,
    ) -> Result<u64, Denial> {
        let slot = self
            .scopes
            .iter()
            .find(|entry| entry.scope == scope)
            .ok_or(Denial::WrongScope)?;
        let property = rules.property(ability).ok_or(Denial::UnavailableInMode)?;
        if self.last_global.is_some_and(|last| now < last) {
            return Err(Denial::ClockMovedBackwards);
        }
        let remaining = |last: Option<u64>, duration: u64| {
            last.map_or(0, |last| duration.saturating_sub(now.saturating_sub(last)))
        };
        Ok(
            remaining(self.last_global, rules.command_global_cooldown_ms).max(remaining(
                slot.last_by_ability[ability.code() as usize - 1],
                property.cooldown_ms,
            )),
        )
    }

    /// Only after successful authoritative activation. Ordinal is issued by the
    /// server monotonically within this round, never request_time or client data.
    /// Duplicate/older receipts cannot reset a cooldown, even after it expires.
    pub fn record_activation(
        &mut self,
        rules: &Rules,
        ordinal: u64,
        now: u64,
        scope: Scope,
        ability: Ability,
    ) -> Result<(), Denial> {
        if self
            .last_activation_ordinal
            .is_some_and(|last| ordinal <= last)
        {
            return Err(Denial::DuplicateActivation);
        }
        if self.remaining_ms(rules, now, scope, ability)? != 0 {
            return Err(Denial::Cooldown);
        }
        let slot = self
            .scopes
            .iter_mut()
            .find(|entry| entry.scope == scope)
            .ok_or(Denial::WrongScope)?;
        slot.last_by_ability[ability.code() as usize - 1] = Some(now);
        self.last_global = Some(now);
        self.last_activation_ordinal = Some(ordinal);
        Ok(())
    }
}
