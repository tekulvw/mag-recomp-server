//! Ordered objective command planning from external Acquisition controller data.
//! This module does not execute scripts, apply commands or infer native events.
use crate::spawn_catalog::SpawnCatalog;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::{self, Read},
    path::Path,
};

const MAX_BYTES: usize = 65_536;

/// Explicit current-state classification, independent of native wire enums.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ObservedState {
    Completed,
    Deactivated,
    Other,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ObjectiveOperation {
    Lock,
    Enable,
    Disable,
}

/// Intent only. Tokens are not native pool IDs. Supplied damage-volume arguments
/// and symbols are retained without inferring damage or enabled/disabled meaning.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    Objective {
        target: u32,
        operation: ObjectiveOperation,
    },
    DamageVolume {
        target: u32,
        arguments: [i32; 4],
        damage_type_symbol: String,
        damage_source_symbol: String,
    },
}

/// Retained source calls that this planner cannot apply.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct UnboundCall {
    pub symbol: String,
    pub arguments: Vec<i32>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct Plan {
    pub commands: Vec<Command>,
    pub unbound_calls: Vec<UnboundCall>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    map_name: String,
    spawn_catalog_sha256: String,
    game_archive_sha256: String,
    script_sha256: String,
    graph_proof_sha256: String,
}
#[derive(Clone, Copy, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
enum Stage {
    A,
    B,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Chain {
    platoon: u8,
    stage: Stage,
    capture_token: u32,
    arrival_token: u32,
    vehicle_spawn_token: u32,
    volume_token: u32,
}
#[derive(Clone, Copy, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
enum PredicateState {
    Completed,
    Deactivated,
}
impl PredicateState {
    fn observed(self) -> ObservedState {
        match self {
            Self::Completed => ObservedState::Completed,
            Self::Deactivated => ObservedState::Deactivated,
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Condition {
    objective_token: u32,
    state: PredicateState,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Branch {
    condition: Condition,
    commands: Vec<Command>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Group {
    branches: Vec<Branch>,
}

/// Validated map-bound graph. Fingerprints identify external evidence; validation
/// checks its joins and bounds rather than independently certifying its semantics.
pub struct Catalog {
    definition: Definition,
    fingerprint: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Definition {
    format: String,
    schema_version: u32,
    controller_token: u32,
    source: Source,
    chains: Vec<Chain>,
    groups: Vec<Group>,
    unbound_calls: Vec<UnboundCall>,
}
impl Catalog {
    pub fn load(path: impl AsRef<Path>, spawns: &SpawnCatalog) -> io::Result<Self> {
        let mut text = String::new();
        File::open(path)?
            .take(MAX_BYTES as u64 + 1)
            .read_to_string(&mut text)?;
        Self::from_json(&text, spawns)
    }
    pub fn from_json(text: &str, spawns: &SpawnCatalog) -> io::Result<Self> {
        if text.len() > MAX_BYTES {
            return Err(io::Error::other("acquisition_controller_too_large"));
        }
        let definition: Definition = serde_json::from_str(text).map_err(io::Error::other)?;
        definition.validate(spawns).map_err(io::Error::other)?;
        Ok(Self {
            definition,
            fingerprint: format!("{:x}", Sha256::digest(text.as_bytes())),
        })
    }
    /// Exact bytes loaded by this catalog, for durable callback provenance.
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }
    pub fn objective_tokens(&self) -> impl Iterator<Item = u32> + '_ {
        self.definition
            .chains
            .iter()
            .flat_map(|c| [c.capture_token, c.arrival_token])
    }
    /// Evaluate all ordered groups against an explicit complete current-state
    /// snapshot. First matching branch wins in each group. Repeating a snapshot
    /// repeats its intents without changing authoritative state. Applying them
    /// needs a qualified native adapter and the engine's record-before-commit
    /// path; callback scheduling and command effects are not inferred here.
    pub fn plan(&self, states: &BTreeMap<u32, ObservedState>) -> Result<Plan, &'static str> {
        if self
            .objective_tokens()
            .any(|token| !states.contains_key(&token))
        {
            return Err("incomplete_acquisition_observations");
        }
        let mut commands = Vec::new();
        for group in &self.definition.groups {
            if let Some(branch) = group.branches.iter().find(|branch| {
                states.get(&branch.condition.objective_token)
                    == Some(&branch.condition.state.observed())
            }) {
                commands.extend(branch.commands.iter().cloned());
            }
        }
        Ok(Plan {
            commands,
            unbound_calls: self.definition.unbound_calls.clone(),
        })
    }
}
impl Definition {
    fn validate(&self, spawns: &SpawnCatalog) -> Result<(), &'static str> {
        if self.format != "mag_acquisition_control"
            || self.schema_version != 1
            || self.chains.len() != 4
            || self.groups.len() != 4
            || spawns.match_profile().game_type != 1
            || self.source.map_name != spawns.map_pool().name
            || Some(self.source.spawn_catalog_sha256.as_str()) != spawns.source_sha256()
            || [
                &self.source.spawn_catalog_sha256,
                &self.source.game_archive_sha256,
                &self.source.script_sha256,
                &self.source.graph_proof_sha256,
            ]
            .into_iter()
            .any(|v| v.len() != 64 || !v.bytes().all(|c| c.is_ascii_hexdigit()))
        {
            return Err("invalid_acquisition_controller_source");
        }
        let mut identities = BTreeSet::from([self.controller_token]);
        let mut addresses = BTreeSet::new();
        let mut objectives = BTreeSet::new();
        let mut volumes = BTreeSet::new();
        for chain in &self.chains {
            if chain.platoon > 1
                || !addresses.insert((chain.platoon, chain.stage))
                || [
                    chain.capture_token,
                    chain.arrival_token,
                    chain.vehicle_spawn_token,
                    chain.volume_token,
                ]
                .into_iter()
                .any(|token| token == 0 || !identities.insert(token))
                || !spawns
                    .map_pool()
                    .locations
                    .iter()
                    .any(|p| p.token == chain.vehicle_spawn_token)
            {
                return Err("invalid_acquisition_controller_chain");
            }
            objectives.extend([chain.capture_token, chain.arrival_token]);
            volumes.insert(chain.volume_token);
        }
        if self.controller_token == 0 {
            return Err("invalid_acquisition_controller_chain");
        }
        for group in &self.groups {
            let mut predicates = BTreeSet::new();
            if group.branches.is_empty() || group.branches.len() > 3 {
                return Err("invalid_acquisition_controller_group");
            }
            for branch in &group.branches {
                let condition = &branch.condition;
                if !objectives.contains(&condition.objective_token)
                    || !predicates.insert((condition.objective_token, condition.state))
                    || branch.commands.is_empty()
                    || branch.commands.len() > 8
                {
                    return Err("invalid_acquisition_controller_branch");
                }
                for command in &branch.commands {
                    let valid = match command {
                        Command::Objective { target, .. } => objectives.contains(target),
                        Command::DamageVolume {
                            target,
                            damage_type_symbol,
                            damage_source_symbol,
                            ..
                        } => {
                            volumes.contains(target)
                                && symbol(damage_type_symbol)
                                && symbol(damage_source_symbol)
                        }
                    };
                    if !valid {
                        return Err("invalid_acquisition_controller_command");
                    }
                }
            }
        }
        if self.unbound_calls.len() > 8
            || self
                .unbound_calls
                .iter()
                .any(|call| !symbol(&call.symbol) || call.arguments.len() > 8)
        {
            return Err("invalid_acquisition_unbound_call");
        }
        Ok(())
    }
}
fn symbol(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.')
}
