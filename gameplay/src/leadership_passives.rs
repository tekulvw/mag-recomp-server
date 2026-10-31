//! Retail inherited leadership levels. This module evaluates authoritative match
//! roles; account leadership progression is deliberately absent. Native consumer
//! evidence and integration limits: docs/protocol/retail-join/LEADERSHIP-PASSIVES.md.
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    path::Path,
};

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum ModifierKind {
    #[serde(rename = "kCharModReloadTime")]
    ReloadTime,
    #[serde(rename = "kCharModChemicalDamageTaken")]
    ChemicalDamageTaken,
    #[serde(rename = "kCharModMovementSpeed")]
    MovementSpeed,
    #[serde(rename = "kCharModIncapacitatedTime")]
    IncapacitatedTime,
    #[serde(rename = "kCharModExplosiveDamageTaken")]
    ExplosiveDamageTaken,
    #[serde(rename = "kCharModAcousticDetectionRange")]
    AcousticDetectionRange,
    #[serde(rename = "kCharModAutoRepair")]
    AutoRepair,
    #[serde(rename = "kCharModDemolitionTime")]
    DemolitionTime,
    #[serde(rename = "kCharModDamageTaken")]
    DamageTaken,
    #[serde(rename = "kCharModDetectionSpeed")]
    DetectionSpeed,
    #[serde(rename = "kCharModAutoHeal")]
    AutoHeal,
}
impl ModifierKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReloadTime => "kCharModReloadTime",
            Self::ChemicalDamageTaken => "kCharModChemicalDamageTaken",
            Self::MovementSpeed => "kCharModMovementSpeed",
            Self::IncapacitatedTime => "kCharModIncapacitatedTime",
            Self::ExplosiveDamageTaken => "kCharModExplosiveDamageTaken",
            Self::AcousticDetectionRange => "kCharModAcousticDetectionRange",
            Self::AutoRepair => "kCharModAutoRepair",
            Self::DemolitionTime => "kCharModDemolitionTime",
            Self::DamageTaken => "kCharModDamageTaken",
            Self::DetectionSpeed => "kCharModDetectionSpeed",
            Self::AutoHeal => "kCharModAutoHeal",
        }
    }
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthoredModifier {
    kind: ModifierKind,
    value: f32,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AbilityLevel {
    level: u8,
    name: String,
    modifiers: Vec<AuthoredModifier>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    member: String,
    member_sha256: String,
    archive_sha256: String,
    extractor: String,
    projector: String,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LeadershipCatalog {
    schema_version: u32,
    application_id: u32,
    source: Source,
    max_separation_range: f32,
    refresh_hold_ms: u64,
    removal_grace_ms: u64,
    levels: Vec<AbilityLevel>,
}
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
impl LeadershipCatalog {
    pub fn load(path: impl AsRef<Path>) -> io::Result<Self> {
        Self::from_json(&std::fs::read_to_string(path)?)
    }
    pub fn from_json(raw: &str) -> io::Result<Self> {
        let value: Self = serde_json::from_str(raw).map_err(|e| invalid(&e.to_string()))?;
        if value.schema_version != 1
            || value.application_id != 20231
            || !value.max_separation_range.is_finite()
            || value.max_separation_range <= 0.
            || value.refresh_hold_ms == 0
            || value.removal_grace_ms == 0
            || value
                .refresh_hold_ms
                .checked_add(value.removal_grace_ms)
                .is_none()
            || value.levels.len() != 4
        {
            return Err(invalid("invalid retail leadership configuration"));
        }
        if value.source.member != "scripts/character_leadership_abilities.zpr"
            || value.source.extractor.is_empty()
            || value.source.projector.is_empty()
            || [&value.source.member_sha256, &value.source.archive_sha256]
                .iter()
                .any(|v| v.len() != 64 || !v.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            return Err(invalid("invalid leadership source identity"));
        }
        for (i, row) in value.levels.iter().enumerate() {
            let mut seen = BTreeSet::new();
            if usize::from(row.level) != i
                || row.name.is_empty()
                || (i == 0) != row.modifiers.is_empty()
                || row
                    .modifiers
                    .iter()
                    .any(|m| !m.value.is_finite() || !seen.insert(m.kind))
            {
                return Err(invalid("invalid leadership level/modifier"));
            }
        }
        Ok(value)
    }
    pub fn max_separation_range(&self) -> f32 {
        self.max_separation_range
    }
    /// Native ChangeModifiers adds every intermediate level in ascending order.
    pub fn modifiers(&self, level: u8) -> Result<PassiveModifiers, &'static str> {
        if level > 3 {
            return Err("unsupported_leadership_ability_level");
        }
        let mut values = BTreeMap::new();
        for row in self.levels.iter().take(usize::from(level) + 1).skip(1) {
            for m in &row.modifiers {
                let value = values.entry(m.kind).or_insert(0.0_f32);
                *value += m.value;
                if !value.is_finite() {
                    return Err("nonfinite_leadership_modifier");
                }
            }
        }
        Ok(PassiveModifiers { values })
    }
}
#[derive(Clone, Debug, Default)]
pub struct PassiveModifiers {
    values: BTreeMap<ModifierKind, f32>,
}
impl PassiveModifiers {
    pub fn value(&self, kind: ModifierKind) -> f32 {
        self.values.get(&kind).copied().unwrap_or(0.)
    }
    pub fn iter(&self) -> impl Iterator<Item = (ModifierKind, f32)> + '_ {
        self.values.iter().map(|(k, v)| (*k, *v))
    }
    /// Add to the recipient's resolved owned skill slot, never multiply tier auras.
    pub fn with_owned(&self, kind: ModifierKind, owned: f32) -> Result<f32, &'static str> {
        let value = owned + self.value(kind);
        if !owned.is_finite() || !value.is_finite() {
            Err("nonfinite_combined_modifier")
        } else {
            Ok(value)
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HeldLeadershipRole {
    #[default]
    None,
    SquadLeader,
    PlatoonLeader,
    OfficerInCharge,
}
impl HeldLeadershipRole {
    pub fn ability_level(self) -> u8 {
        match self {
            Self::None => 0,
            Self::SquadLeader => 1,
            Self::PlatoonLeader => 2,
            Self::OfficerInCharge => 3,
        }
    }
    pub fn from_native_role(role: u8) -> Result<Self, &'static str> {
        match role {
            0 => Ok(Self::None),
            1 => Ok(Self::SquadLeader),
            2 => Ok(Self::PlatoonLeader),
            4 => Ok(Self::OfficerInCharge),
            _ => Err("unsupported_held_leadership_role"),
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct PassiveParticipant {
    pub character_id: u32,
    pub team: u8,
    pub squad: u16,
    pub position: [f32; 3],
    pub held_role: HeldLeadershipRole,
    /// The exact native SimCharacter state byte, not account progression.
    pub state_code: u8,
    pub damage_model_dead: bool,
    pub locally_dead: bool,
    pub damage_conditions: u8,
    /// Retained for explicit integration context. The native provider has no mounted gate.
    pub mounted: bool,
}
impl PassiveParticipant {
    pub fn can_provide(self) -> bool {
        self.held_role != HeldLeadershipRole::None
            && self.state_code != 0
            && !self.damage_model_dead
            && (self.locally_dead || self.damage_conditions & 2 == 0)
    }
    fn identity(self) -> (u32, u8, u16) {
        (self.character_id, self.team, self.squad)
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct PassiveState {
    identity: Option<(u32, u8, u16)>,
    level: u8,
    expires_ms: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PassiveChange {
    pub old_level: u8,
    pub new_level: u8,
}
impl PassiveChange {
    pub fn changed(self) -> bool {
        self.old_level != self.new_level
    }
}
impl PassiveState {
    pub fn level(&self) -> u8 {
        self.level
    }
    pub fn clear(&mut self) {
        *self = Self::default();
    }
    /// Refresh once from an authoritative, complete same-world roster. Self is
    /// evaluated first. Monotonic server milliseconds must be supplied; caller
    /// controls tick cadence. This performs a full refresh, unlike native's
    /// amortized one-self/one-pair sweep. No client request grants a held role.
    pub fn refresh(
        &mut self,
        catalog: &LeadershipCatalog,
        recipient: &PassiveParticipant,
        providers: &[PassiveParticipant],
        now_ms: u64,
        squad_range_modifier: f32,
    ) -> Result<PassiveChange, &'static str> {
        if !squad_range_modifier.is_finite() || recipient.position.iter().any(|v| !v.is_finite()) {
            return Err("invalid_leadership_context");
        }
        let range = catalog
            .max_separation_range
            .mul_add(squad_range_modifier, catalog.max_separation_range);
        if !range.is_finite() || range <= 0. {
            return Err("invalid_leadership_range");
        }
        if providers
            .iter()
            .any(|p| p.position.iter().any(|v| !v.is_finite()))
        {
            return Err("invalid_leadership_provider_position");
        }
        let old_level = self.level;
        if self.identity != Some(recipient.identity()) {
            self.clear();
            self.identity = Some(recipient.identity());
        }
        for source in std::iter::once(recipient).chain(
            providers
                .iter()
                .filter(|p| p.character_id != recipient.character_id),
        ) {
            if source.team != recipient.team
                || source.squad != recipient.squad
                || !source.can_provide()
            {
                continue;
            }
            let delta =
                std::array::from_fn::<_, 3, _>(|i| source.position[i] - recipient.position[i]);
            let distance = (delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2]).sqrt();
            if distance >= range {
                continue;
            }
            let candidate = source.held_role.ability_level();
            if self.level < candidate || self.expires_ms <= now_ms {
                self.level = candidate;
                self.expires_ms = now_ms.saturating_add(catalog.refresh_hold_ms);
            } else if self.level == candidate {
                self.expires_ms = now_ms.saturating_add(catalog.refresh_hold_ms);
            }
        }
        if self.level > 0 && self.expires_ms.saturating_add(catalog.removal_grace_ms) <= now_ms {
            self.level = 0;
            self.expires_ms = 0;
        }
        Ok(PassiveChange {
            old_level,
            new_level: self.level,
        })
    }
}
