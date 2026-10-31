//! Neutral authored spawn data loaded from disk; no native resources or compiled coordinates.
use crate::replication::CharacterPosition;
use serde::Deserialize;
use std::{
    collections::BTreeSet,
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
};
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpawnKind {
    Insertion,
    Alternate,
    BunkerPassenger,
    OnFoot,
    Unsupported,
}
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpawnLocation {
    pub index: u16,
    pub token: u32,
    pub kind: SpawnKind,
    #[serde(default = "default_active")]
    pub active: bool,
    pub position: CharacterPosition,
}
fn default_active() -> bool {
    true
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MapSpawns {
    pub name: String,
    pub spawn_count: u8,
    pub locations: Vec<SpawnLocation>,
    #[serde(default)]
    pub owner_faction: Option<u8>,
    /// Verified initial-load pool order is distinct from live observation of every point.
    #[serde(default)]
    pub runtime_indices_verified: bool,
    #[serde(default)]
    pub runtime_indices_basis: Option<String>,
    #[serde(default)]
    pub squads: Vec<SquadSpawnBinding>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SquadSpawnBinding {
    pub index: u16,
    pub team: u8,
    pub platoon: u8,
    pub squad: u8,
    /// Defending squads use the map owner; attacking faction is match dependent.
    pub faction: Option<u8>,
    pub available: Vec<u16>,
    pub default_spawn: u16,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ActiveSpawns {
    map_name: String,
    squad_index: u16,
    default_spawn: u16,
    available: Vec<u16>,
}
/// Match presentation and world-feature policy for the selected map.
/// Absent fields keep the original local Alyeska Domination policy.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MatchProfileFile {
    #[serde(default)]
    game_type: Option<i8>,
    #[serde(default)]
    auto_assign_teams: bool,
    #[serde(default)]
    team_capacity: Option<usize>,
    #[serde(default)]
    squad_capacity: Option<usize>,
    #[serde(default)]
    spawn_wave_ms: Option<[u32; 2]>,
    #[serde(default)]
    initial_spawn_delay_ms: Option<[u32; 2]>,
    #[serde(default)]
    attacking_faction: Option<u8>,
    #[serde(default)]
    maximum_points: Option<[i16; 2]>,
    #[serde(default)]
    has_bunkers: Option<bool>,
    #[serde(default)]
    has_objectives: Option<bool>,
}
/// Resolved disk-backed policy; match side is independent from faction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MatchProfile {
    pub game_type: i8,
    pub auto_assign_teams: bool,
    pub team_capacity: usize,
    pub squad_capacity: usize,
    pub spawn_wave_ms: [u32; 2],
    pub initial_spawn_delay_ms: [u32; 2],
    pub attacking_faction: u8,
    pub defending_faction: u8,
    pub maximum_points: [i16; 2],
    pub has_bunkers: bool,
    pub has_objectives: bool,
}
impl Default for MatchProfile {
    /// The original single-player Alyeska Domination policy.
    fn default() -> Self {
        Self {
            game_type: 2,
            auto_assign_teams: false,
            team_capacity: 8,
            squad_capacity: 8,
            spawn_wave_ms: [10_000; 2],
            initial_spawn_delay_ms: [10_000; 2],
            attacking_faction: 1,
            defending_faction: 0,
            maximum_points: [1200, 1000],
            has_bunkers: true,
            has_objectives: true,
        }
    }
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpawnCatalog {
    format: String,
    schema_version: u32,
    source: String,
    active: ActiveSpawns,
    #[serde(default)]
    match_profile: Option<MatchProfileFile>,
    maps: Vec<MapSpawns>,
}
impl SpawnCatalog {
    pub fn load(path: impl AsRef<Path>) -> io::Result<Self> {
        let mut bytes = Vec::new();
        fs::File::open(path)?
            .take(1_048_577)
            .read_to_end(&mut bytes)?;
        if bytes.len() > 1_048_576 {
            return Err(io::Error::other("spawn_catalog_too_large"));
        }
        let value: Self = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
        value.validate().map_err(io::Error::other)?;
        Ok(value)
    }
    fn validate(&self) -> Result<(), &'static str> {
        if self.format != "mag_spawn_catalog"
            || self.schema_version != 1
            || self.source.is_empty()
            || self.source.len() > 8192
            || self.maps.is_empty()
            || self.maps.len() > 32
        {
            return Err("invalid_spawn_catalog");
        }
        let mut names = BTreeSet::new();
        for map in &self.maps {
            if map.name.is_empty()
                || map.name.len() > 64
                || !names.insert(map.name.as_str())
                || map.spawn_count == 0
                || map.spawn_count > 128
                || map.locations.len() != usize::from(map.spawn_count)
            {
                return Err("invalid_spawn_map");
            }
            if map.owner_faction.is_some_and(|f| f > 2)
                || map.squads.len() > 32
                || (!map.squads.is_empty() && map.owner_faction.is_none())
            {
                return Err("invalid_spawn_faction");
            }
            if map
                .runtime_indices_basis
                .as_ref()
                .is_some_and(|b| b.is_empty() || b.len() > 128)
                || (map.runtime_indices_verified && map.runtime_indices_basis.is_none())
            {
                return Err("missing_spawn_pool_identity_basis");
            }
            let mut squad_ids = BTreeSet::new();
            for squad in &map.squads {
                let address =
                    crate::replication::SquadAddress::new(squad.team, squad.platoon, squad.squad)?;
                if address.squad_index() != squad.index
                    || !squad_ids.insert(squad.index)
                    || squad.faction.is_some_and(|f| f > 2)
                    || (squad.team == 1 && squad.faction != map.owner_faction)
                    || (squad.team == 0 && squad.faction.is_some())
                {
                    return Err("invalid_squad_spawn_binding");
                }
                Self::validate_links(map, &squad.available, squad.default_spawn)?;
            }
            let mut ids = BTreeSet::new();
            let mut tokens = BTreeSet::new();
            for p in &map.locations {
                if p.index >= u16::from(map.spawn_count)
                    || !ids.insert(p.index)
                    || p.token == 0
                    || !tokens.insert(p.token)
                {
                    return Err("invalid_spawn_identity");
                }
                if [p.position.x, p.position.y, p.position.z]
                    .into_iter()
                    .any(|v| !v.is_finite() || v.abs() >= 2048.0)
                    || p.position.x < 0.0
                    || p.position.z < 0.0
                {
                    return Err("invalid_spawn_position");
                }
            }
        }
        if let Some(profile) = &self.match_profile
            && (profile.attacking_faction.is_some_and(|f| f > 2)
                || profile.game_type.is_some_and(|t| !(0..=4).contains(&t))
                || profile
                    .maximum_points
                    .is_some_and(|p| p.iter().any(|v| *v < 0)))
        {
            return Err("invalid_match_profile");
        }
        crate::replication::SquadAddress::from_index(self.active.squad_index)?;
        let map = self
            .maps
            .iter()
            .find(|m| m.name == self.active.map_name)
            .ok_or("missing_active_spawn_map")?;
        Self::validate_links(map, &self.active.available, self.active.default_spawn)?;
        if !map.squads.is_empty() {
            let squad = map
                .squads
                .iter()
                .find(|s| s.index == self.active.squad_index)
                .ok_or("missing_active_squad_binding")?;
            if squad.available != self.active.available
                || squad.default_spawn != self.active.default_spawn
            {
                return Err("inconsistent_active_squad_binding");
            }
        }
        let profile = self.match_profile();
        if profile.team_capacity == 0
            || profile.team_capacity > crate::MAX_MATCH_PLAYERS / 2
            || profile.squad_capacity == 0
            || profile.squad_capacity > 8
            || profile
                .spawn_wave_ms
                .iter()
                .chain(profile.initial_spawn_delay_ms.iter())
                .any(|ms| !(1..=120_000).contains(ms))
        {
            return Err("invalid_match_admission_policy");
        }
        if profile.auto_assign_teams {
            if profile.has_bunkers {
                return Err("automatic_bunker_assignment_unsupported");
            }
            for team in 0..2 {
                let slots =
                    map.squads.iter().filter(|s| s.team == team).count() * profile.squad_capacity;
                if slots < profile.team_capacity {
                    return Err("insufficient_configured_team_squads");
                }
            }
        }
        Ok(())
    }
    fn validate_links(
        map: &MapSpawns,
        available: &[u16],
        default: u16,
    ) -> Result<(), &'static str> {
        if available.is_empty() || available.len() > 8 {
            return Err("invalid_spawn_availability");
        }
        let mut ids = BTreeSet::new();
        for id in available {
            let p = map
                .locations
                .iter()
                .find(|p| p.index == *id)
                .ok_or("missing_spawn_link")?;
            if !ids.insert(*id) || !p.active || matches!(p.kind, SpawnKind::Unsupported) {
                return Err("unsupported_spawn_link");
            }
        }
        if !ids.contains(&default) {
            return Err("unavailable_default_spawn");
        }
        Ok(())
    }
    /// Resolved match policy for the active map.
    pub fn match_profile(&self) -> MatchProfile {
        let default = MatchProfile::default();
        let file = self.match_profile.clone().unwrap_or_default();
        let defending = self
            .map_pool()
            .owner_faction
            .unwrap_or(default.defending_faction);
        let attacking =
            file.attacking_faction
                .unwrap_or(if default.attacking_faction == defending {
                    // Pick a distinct opposing faction so the local defender stays team 1.
                    (defending + 1) % 3
                } else {
                    default.attacking_faction
                });
        MatchProfile {
            game_type: file.game_type.unwrap_or(default.game_type),
            auto_assign_teams: file.auto_assign_teams,
            team_capacity: file.team_capacity.unwrap_or(default.team_capacity),
            squad_capacity: file.squad_capacity.unwrap_or(default.squad_capacity),
            spawn_wave_ms: file.spawn_wave_ms.unwrap_or(default.spawn_wave_ms),
            initial_spawn_delay_ms: file
                .initial_spawn_delay_ms
                .unwrap_or(file.spawn_wave_ms.unwrap_or(default.spawn_wave_ms)),
            attacking_faction: attacking,
            defending_faction: defending,
            maximum_points: file.maximum_points.unwrap_or(default.maximum_points),
            has_bunkers: file.has_bunkers.unwrap_or(default.has_bunkers),
            has_objectives: file.has_objectives.unwrap_or(default.has_objectives),
        }
    }
    /// Selected map's entire pool, including points unavailable to the selected squad.
    pub fn map_pool(&self) -> &MapSpawns {
        self.maps
            .iter()
            .find(|m| m.name == self.active.map_name)
            .unwrap()
    }
    pub fn point(&self, index: u16) -> Option<&SpawnLocation> {
        self.map_pool().locations.iter().find(|p| p.index == index)
    }
    pub fn selected_squad(&self, index: u16) -> Option<&SquadSpawnBinding> {
        self.map_pool().squads.iter().find(|s| s.index == index)
    }
    pub fn available_for_squad(&self, index: u16) -> Option<&[u16]> {
        self.selected_squad(index).map(|s| s.available.as_slice())
    }
    pub fn default_for_squad(&self, index: u16) -> Option<u16> {
        self.selected_squad(index).map(|s| s.default_spawn)
    }
    /// Select an authored squad without changing or re-reading the source file.
    /// This infrequent copy is retained per participant, never made per tick.
    pub fn for_squad(&self, index: u16) -> Result<Self, &'static str> {
        let binding = self.selected_squad(index).ok_or("unknown_squad")?;
        let mut selected = self.clone();
        selected.active.squad_index = binding.index;
        selected.active.available = binding.available.clone();
        selected.active.default_spawn = binding.default_spawn;
        selected.validate()?;
        Ok(selected)
    }
    /// Runtime leader selection is constrained to the authored squad's choices.
    /// It is separate from the immutable file's default binding.
    pub(crate) fn set_runtime_default(&mut self, index: u16) -> Result<(), &'static str> {
        if self.alpha(index).is_none() {
            return Err("unavailable_squad_spawn");
        }
        self.active.default_spawn = index;
        Ok(())
    }
    pub fn alpha(&self, index: u16) -> Option<&SpawnLocation> {
        self.active
            .available
            .contains(&index)
            .then(|| {
                self.maps
                    .iter()
                    .find(|m| m.name == self.active.map_name)?
                    .locations
                    .iter()
                    .find(|p| p.index == index)
            })
            .flatten()
    }
    pub fn default_spawn(&self) -> u16 {
        self.active.default_spawn
    }
    pub fn spawn_count(&self) -> u8 {
        self.maps
            .iter()
            .find(|m| m.name == self.active.map_name)
            .unwrap()
            .spawn_count
    }
    pub fn squad_index(&self) -> u16 {
        self.active.squad_index
    }
    pub fn map_name(&self) -> &str {
        &self.active.map_name
    }
    pub fn alternate_position(&self) -> Option<CharacterPosition> {
        self.maps
            .iter()
            .find(|m| m.name == self.active.map_name)?
            .locations
            .iter()
            .find(|p| {
                self.active.available.contains(&p.index) && matches!(p.kind, SpawnKind::Alternate)
            })
            .map(|p| p.position)
    }
}
/// Development/library fixture path; executable startup requires an explicit disk path.
pub fn default_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data/prototype-spawns.json")
}
pub fn alpha(index: u16) -> Option<SpawnLocation> {
    SpawnCatalog::load(default_path())
        .ok()?
        .alpha(index)
        .copied()
}
