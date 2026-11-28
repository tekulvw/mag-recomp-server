//! Disk-backed Domination controller callbacks and ordered state/score/ability requests.
//! Objective and platoon tokens are authored identities. Callers own native slot resolution,
//! transition dispatch, delayed invocation scheduling and the actual gameplay effects.

use crate::acquisition_score_rate::Request;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

const MAX_BYTES: usize = 1_048_576;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Ord, PartialOrd)]
#[serde(rename_all = "snake_case")]
pub enum Event {
    Completed,
    Deactivated,
    Repaired,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Completed,
    Deactivated,
    Locked,
    Other,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Caller {
    Original,
    None,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Ability {
    ClusterBomb,
    GasBombardment,
    GuidedArtillery,
    Sensor,
    PrecisionStrike,
    StrafingRun,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Effect {
    SetState {
        objective_token: u32,
        state: State,
        caller: Caller,
    },
    ScoreRate(Request),
    /// A zero platoon token preserves an authored empty parameter.
    Ability {
        platoon_token: u32,
        ability: Ability,
        enabled: bool,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Transition {
    pub objective_token: u32,
    pub event: Event,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Invocation {
    pub binding_token: u32,
    pub entry_index: usize,
    pub delay_ms: u32,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Condition {
    objective_token: u32,
    state: State,
    equal: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Command {
    SetState {
        objective_token: u32,
        state: State,
        caller: Caller,
    },
    ScoreRate {
        frequency_ms: u32,
        amount: u32,
        team: u8,
        objectives: Vec<u32>,
    },
    Ability {
        platoon_token: u32,
        ability: Ability,
        enabled: bool,
    },
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Rule {
    conditions: Vec<Condition>,
    commands: Vec<Command>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    delay_ms: u32,
    rules: Vec<Rule>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Binding {
    binding_token: u32,
    entries: Vec<Entry>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Hook {
    objective_token: u32,
    event: Event,
    binding_token: u32,
    resolved: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MapFile {
    name: String,
    level_archive_sha256: String,
    level_clv_sha256: String,
    bindings: Vec<Binding>,
    hooks: Vec<Hook>,
}

#[derive(Clone, Debug)]
pub struct Map {
    pub name: String,
    pub level_archive_sha256: String,
    pub level_clv_sha256: String,
    bindings: BTreeMap<u32, Vec<Entry>>,
    hooks: BTreeMap<(u32, Event), Hook>,
    objective_tokens: BTreeSet<u32>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CatalogFile {
    format: String,
    schema_version: u8,
    game_archive_sha256: String,
    metadata_sha256: String,
    script_sha256: Vec<String>,
    maps: Vec<MapFile>,
}

#[derive(Clone, Debug)]
pub struct Catalog {
    pub game_archive_sha256: String,
    pub metadata_sha256: String,
    maps: BTreeMap<String, Map>,
}

fn is_sha256(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

impl Catalog {
    pub fn from_json(raw: &[u8]) -> Result<Self, String> {
        if raw.len() > MAX_BYTES {
            return Err("Domination controller catalog exceeds size limit".into());
        }
        let file: CatalogFile = serde_json::from_slice(raw).map_err(|err| err.to_string())?;
        if file.format != "mag_domination_control"
            || file.schema_version != 1
            || !is_sha256(&file.game_archive_sha256)
            || !is_sha256(&file.metadata_sha256)
            || file.script_sha256.is_empty()
            || file.script_sha256.len() > 16
            || !file.script_sha256.iter().all(|s| is_sha256(s))
            || file.maps.is_empty()
            || file.maps.len() > 3
        {
            return Err("unsupported Domination controller catalog".into());
        }
        let mut maps = BTreeMap::new();
        for file in file.maps {
            let map = Map::from_file(file)?;
            if maps.insert(map.name.clone(), map).is_some() {
                return Err("duplicate Domination controller map".into());
            }
        }
        Ok(Self {
            game_archive_sha256: file.game_archive_sha256,
            metadata_sha256: file.metadata_sha256,
            maps,
        })
    }

    pub fn map(&self, name: &str) -> Option<&Map> {
        self.maps.get(name)
    }
}

impl Map {
    fn from_file(file: MapFile) -> Result<Self, &'static str> {
        if file.name.is_empty()
            || !is_sha256(&file.level_archive_sha256)
            || !is_sha256(&file.level_clv_sha256)
            || file.bindings.is_empty()
            || file.bindings.len() > 128
            || file.hooks.len() > 512
        {
            return Err("invalid Domination map source or counts");
        }

        let mut bindings = BTreeMap::new();
        let mut objective_tokens = BTreeSet::new();
        for binding in file.bindings {
            if binding.binding_token == 0 || binding.entries.is_empty() || binding.entries.len() > 8
            {
                return Err("invalid Domination controller binding");
            }
            for (index, entry) in binding.entries.iter().enumerate() {
                if entry.delay_ms > 60_000
                    || (entry.delay_ms == 0 && index != 0)
                    || entry.rules.is_empty()
                    || entry.rules.len() > 8
                {
                    return Err("invalid Domination controller entry");
                }
                for rule in &entry.rules {
                    if rule.conditions.len() > 8
                        || rule.commands.is_empty()
                        || rule.commands.len() > 16
                    {
                        return Err("invalid Domination controller rule");
                    }
                    objective_tokens.extend(rule.conditions.iter().map(|c| c.objective_token));
                    for command in &rule.commands {
                        match command {
                            Command::SetState {
                                objective_token,
                                state,
                                ..
                            } => {
                                if *state == State::Other {
                                    return Err("invalid Domination controller output state");
                                }
                                objective_tokens.insert(*objective_token);
                            }
                            Command::ScoreRate {
                                frequency_ms,
                                amount,
                                team,
                                objectives,
                            } => {
                                if *team > 1
                                    || *frequency_ms > 60_000
                                    || objectives.len() > 32
                                    || objectives.iter().collect::<BTreeSet<_>>().len()
                                        != objectives.len()
                                    || amount.checked_mul(objectives.len().max(1) as u32).is_none()
                                {
                                    return Err("invalid Domination score rate");
                                }
                                objective_tokens.extend(objectives);
                            }
                            Command::Ability { .. } => {}
                        }
                    }
                }
            }
            if bindings
                .insert(binding.binding_token, binding.entries)
                .is_some()
            {
                return Err("duplicate Domination controller binding");
            }
        }

        let mut hooks = BTreeMap::new();
        let mut referenced = BTreeSet::new();
        for hook in file.hooks {
            if hook.binding_token == 0
                || hook.resolved != bindings.contains_key(&hook.binding_token)
            {
                return Err("invalid Domination objective hook target");
            }
            if hook.resolved {
                referenced.insert(hook.binding_token);
            }
            objective_tokens.insert(hook.objective_token);
            if hooks
                .insert((hook.objective_token, hook.event), hook)
                .is_some()
            {
                return Err("duplicate Domination objective hook");
            }
        }
        if referenced.len() != bindings.len() || objective_tokens.contains(&0) {
            return Err("invalid Domination controller/objective token coverage");
        }

        Ok(Self {
            name: file.name,
            level_archive_sha256: file.level_archive_sha256,
            level_clv_sha256: file.level_clv_sha256,
            bindings,
            hooks,
            objective_tokens,
        })
    }

    pub fn objective_tokens(&self) -> impl Iterator<Item = u32> + '_ {
        self.objective_tokens.iter().copied()
    }

    pub fn invocations(
        &self,
        objective_token: u32,
        event: Event,
    ) -> impl Iterator<Item = Invocation> + '_ {
        self.hooks
            .get(&(objective_token, event))
            .filter(|hook| hook.resolved)
            .into_iter()
            .flat_map(|hook| {
                self.bindings[&hook.binding_token].iter().enumerate().map(
                    move |(entry_index, entry)| Invocation {
                        binding_token: hook.binding_token,
                        entry_index,
                        delay_ms: entry.delay_ms,
                    },
                )
            })
    }

    /// Whether any authored rule run when the objective reaches `event`
    /// enables or disables a command ability, whatever its conditions.
    pub(crate) fn toggles_abilities(&self, objective_token: u32, event: Event) -> bool {
        self.hooks
            .get(&(objective_token, event))
            .and_then(|hook| self.bindings.get(&hook.binding_token))
            .into_iter()
            .flatten()
            .flat_map(|entry| &entry.rules)
            .flat_map(|rule| &rule.commands)
            .any(|command| matches!(command, Command::Ability { .. }))
    }

    /// Whether the event's hook switches some command ability on (anti-air
    /// opening the other side's air support), not only off.
    pub(crate) fn enables_abilities(&self, objective_token: u32, event: Event) -> bool {
        self.hooks
            .get(&(objective_token, event))
            .and_then(|hook| self.bindings.get(&hook.binding_token))
            .into_iter()
            .flatten()
            .flat_map(|entry| &entry.rules)
            .flat_map(|rule| &rule.commands)
            .any(|command| matches!(command, Command::Ability { enabled: true, .. }))
    }

    /// Objectives the authored stage rules advance together: each group is a
    /// rule that requires every member to be completed at once and then locks
    /// them all (in Domination, the points of one stage of a platoon). Groups
    /// come in binding order and members in condition order, without repeats.
    pub(crate) fn stage_groups(&self) -> Vec<Vec<u32>> {
        let mut groups: Vec<Vec<u32>> = Vec::new();
        for rule in self
            .bindings
            .values()
            .flatten()
            .flat_map(|entry| &entry.rules)
        {
            let mut members: Vec<u32> = Vec::new();
            for condition in &rule.conditions {
                if condition.equal
                    && condition.state == State::Completed
                    && !members.contains(&condition.objective_token)
                {
                    members.push(condition.objective_token);
                }
            }
            let locks_all = members.iter().all(|token| {
                rule.commands.iter().any(|command| {
                    matches!(command, Command::SetState { objective_token, state: State::Locked, .. }
                        if objective_token == token)
                })
            });
            if members.len() >= 2 && locks_all && !groups.contains(&members) {
                groups.push(members);
            }
        }
        groups
    }

    pub fn unresolved_hook(&self, objective_token: u32, event: Event) -> Option<u32> {
        self.hooks
            .get(&(objective_token, event))
            .filter(|hook| !hook.resolved)
            .map(|hook| hook.binding_token)
    }

    pub fn effects(
        &self,
        invocation: Invocation,
        mut state: impl FnMut(u32) -> Option<State>,
    ) -> Result<Vec<Effect>, &'static str> {
        let entry = self
            .bindings
            .get(&invocation.binding_token)
            .and_then(|entries| entries.get(invocation.entry_index))
            .filter(|entry| entry.delay_ms == invocation.delay_ms)
            .ok_or("unknown_domination_invocation")?;
        let mut effects = Vec::new();
        // SetState requests a desired state. Reads continue to observe current state
        // until the objective state machine enters that state.
        let mut current = |token| state(token).ok_or("missing_domination_objective_state");
        for rule in &entry.rules {
            let mut admitted = true;
            for condition in &rule.conditions {
                if (current(condition.objective_token)? == condition.state) != condition.equal {
                    admitted = false;
                    break;
                }
            }
            if !admitted {
                continue;
            }
            for command in &rule.commands {
                let effect = match command {
                    Command::SetState {
                        objective_token,
                        state,
                        caller,
                    } => Effect::SetState {
                        objective_token: *objective_token,
                        state: *state,
                        caller: *caller,
                    },
                    Command::ScoreRate {
                        frequency_ms,
                        amount,
                        team,
                        objectives,
                    } => {
                        let mut count = 0;
                        for token in objectives {
                            count += u32::from(current(*token)? == State::Completed);
                        }
                        Effect::ScoreRate(Request {
                            frequency_ms: *frequency_ms,
                            amount: *amount * if objectives.is_empty() { 1 } else { count },
                            team: *team,
                        })
                    }
                    Command::Ability {
                        platoon_token,
                        ability,
                        enabled,
                    } => Effect::Ability {
                        platoon_token: *platoon_token,
                        ability: *ability,
                        enabled: *enabled,
                    },
                };
                effects.push(effect);
            }
        }
        Ok(effects)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn synthetic_catalog() -> Value {
        let hash = "a".repeat(64);
        json!({
            "format": "mag_domination_control",
            "schema_version": 1,
            "game_archive_sha256": hash,
            "metadata_sha256": hash,
            "script_sha256": [hash],
            "maps": [{
                "name": "synthetic",
                "level_archive_sha256": hash,
                "level_clv_sha256": hash,
                "bindings": [
                    {"binding_token": 51, "entries": [{"delay_ms": 0, "rules": [
                        {"conditions": [
                            {"objective_token": 1, "state": "completed", "equal": true},
                            {"objective_token": 2, "state": "completed", "equal": true}
                        ], "commands": [
                            {"kind": "set_state", "objective_token": 1, "state": "locked", "caller": "original"},
                            {"kind": "set_state", "objective_token": 2, "state": "locked", "caller": "original"},
                            {"kind": "set_state", "objective_token": 3, "state": "completed", "caller": "none"}
                        ]},
                        {"conditions": [], "commands": [
                            {"kind": "score_rate", "frequency_ms": 6000, "amount": 7, "team": 1, "objectives": [1, 5]}
                        ]}
                    ]}]},
                    {"binding_token": 52, "entries": [{"delay_ms": 4000, "rules": [
                        {"conditions": [], "commands": [
                            {"kind": "set_state", "objective_token": 4, "state": "deactivated", "caller": "original"}
                        ]}
                    ]}]},
                    {"binding_token": 53, "entries": [{"delay_ms": 0, "rules": [
                        {"conditions": [{"objective_token": 6, "state": "completed", "equal": true}], "commands": [
                            {"kind": "ability", "platoon_token": 0, "ability": "sensor", "enabled": true}
                        ]},
                        {"conditions": [{"objective_token": 6, "state": "completed", "equal": false}], "commands": [
                            {"kind": "ability", "platoon_token": 0, "ability": "sensor", "enabled": false}
                        ]}
                    ]}]}
                ],
                "hooks": [
                    {"objective_token": 1, "event": "completed", "binding_token": 51, "resolved": true},
                    {"objective_token": 3, "event": "completed", "binding_token": 52, "resolved": true},
                    {"objective_token": 3, "event": "deactivated", "binding_token": 99, "resolved": false},
                    {"objective_token": 6, "event": "repaired", "binding_token": 53, "resolved": true}
                ]
            }]
        })
    }

    #[test]
    fn stage_groups_are_the_rules_that_lock_points_completed_together() {
        let catalog = Catalog::from_json(&synthetic_catalog().to_string().into_bytes()).unwrap();
        let map = catalog.map("synthetic").unwrap();
        // Binding 51 locks 1 and 2 once both are completed. The ability rule
        // tests one objective and locks nothing, so it is not a stage.
        assert_eq!(map.stage_groups(), vec![vec![1, 2]]);

        let mut raw = synthetic_catalog();
        let rule = &mut raw["maps"][0]["bindings"][0]["entries"][0]["rules"][0];
        // A rule that requires both but locks only one is not a stage either.
        rule["commands"].as_array_mut().unwrap().remove(1);
        let catalog = Catalog::from_json(&raw.to_string().into_bytes()).unwrap();
        assert!(catalog.map("synthetic").unwrap().stage_groups().is_empty());
    }

    #[test]
    fn ability_hooks_report_whether_they_switch_an_ability_on() {
        let catalog = Catalog::from_json(&synthetic_catalog().to_string().into_bytes()).unwrap();
        let map = catalog.map("synthetic").unwrap();
        // Objective 6's repair hook switches the sensor on in one branch.
        assert!(map.toggles_abilities(6, Event::Repaired));
        assert!(map.enables_abilities(6, Event::Repaired));
        assert!(!map.toggles_abilities(1, Event::Completed));
        assert!(!map.enables_abilities(1, Event::Completed));
        assert!(!map.enables_abilities(6, Event::Completed));
    }

    #[test]
    fn resolves_ordered_effects_with_delay_and_source_tokens() {
        let catalog = Catalog::from_json(&synthetic_catalog().to_string().into_bytes()).unwrap();
        let map = catalog.map("synthetic").unwrap();
        assert_eq!(
            map.objective_tokens().collect::<Vec<_>>(),
            [1, 2, 3, 4, 5, 6]
        );

        let invocation = map.invocations(1, Event::Completed).next().unwrap();
        let effects = map
            .effects(invocation, |token| {
                Some(if token <= 4 {
                    State::Completed
                } else {
                    State::Other
                })
            })
            .unwrap();
        assert_eq!(
            effects,
            vec![
                Effect::SetState {
                    objective_token: 1,
                    state: State::Locked,
                    caller: Caller::Original
                },
                Effect::SetState {
                    objective_token: 2,
                    state: State::Locked,
                    caller: Caller::Original
                },
                Effect::SetState {
                    objective_token: 3,
                    state: State::Completed,
                    caller: Caller::None
                },
                Effect::ScoreRate(Request {
                    frequency_ms: 6000,
                    amount: 7,
                    team: 1
                })
            ]
        );

        let invocation = map.invocations(3, Event::Completed).next().unwrap();
        assert_eq!(invocation.delay_ms, 4000);
        assert_eq!(
            map.effects(invocation, |_| None).unwrap(),
            [Effect::SetState {
                objective_token: 4,
                state: State::Deactivated,
                caller: Caller::Original
            }]
        );
        assert_eq!(map.unresolved_hook(3, Event::Deactivated), Some(99));
        assert!(map.invocations(3, Event::Deactivated).next().is_none());
    }

    #[test]
    fn preserves_zero_score_rate_and_empty_ability_parameter() {
        let catalog = Catalog::from_json(&synthetic_catalog().to_string().into_bytes()).unwrap();
        let map = catalog.map("synthetic").unwrap();
        let invocation = map.invocations(1, Event::Completed).next().unwrap();
        assert_eq!(
            map.effects(invocation, |_| Some(State::Deactivated))
                .unwrap(),
            [Effect::ScoreRate(Request {
                frequency_ms: 6000,
                amount: 0,
                team: 1
            })]
        );
        assert_eq!(
            map.effects(invocation, |_| None),
            Err("missing_domination_objective_state")
        );

        let invocation = map.invocations(6, Event::Repaired).next().unwrap();
        assert_eq!(
            map.effects(invocation, |_| Some(State::Deactivated))
                .unwrap(),
            [Effect::Ability {
                platoon_token: 0,
                ability: Ability::Sensor,
                enabled: false
            }]
        );
    }

    #[test]
    fn rejects_mismatched_binding_and_invalid_score_rate() {
        let mut raw = synthetic_catalog();
        raw["maps"][0]["hooks"][0]["binding_token"] = json!(88);
        assert!(Catalog::from_json(&raw.to_string().into_bytes()).is_err());

        let mut raw = synthetic_catalog();
        raw["maps"][0]["bindings"][0]["entries"][0]["rules"][1]["commands"][0]["team"] = json!(2);
        assert!(Catalog::from_json(&raw.to_string().into_bytes()).is_err());
    }
}
