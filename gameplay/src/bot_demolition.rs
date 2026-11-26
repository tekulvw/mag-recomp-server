//! Population bots attack the other side's demolition structures.
//!
//! A demolition structure is a structural objective with a demolition rule:
//! the side that does not own it arms a charge in its trigger, the charge
//! detonates after its fuse and completes (destroys) the structure, and the
//! owners may disarm the charge or later repair the wreck. What a destroyed
//! structure changes is authored data: it can close the owners' forward
//! spawn, open a spawn of the other side, switch command abilities, or only
//! swap the structure's own geometry (gates, barricades, bridges).
//!
//! The behaviour is specified in docs/bot-demolition.md; this is an original
//! design for this server:
//!
//! - Each squad keeps at most one *demolition team*: up to `team_size` of its
//!   bots sent to one enemy structure instead of the squad's lane objective.
//!   A side keeps at most `teams_per_side` teams, and after one of its teams
//!   ends it waits `rest_ms` before it sends another, so structures fall over
//!   the course of a match rather than all at once.
//! - A squad without a team picks the structure with the best score, the
//!   value of what destroying it changes less `distance_weight` for every
//!   metre between it and the squad's lane objective. Only structures within
//!   `reach_m` of the lane that the squad's side may arm now (not armed,
//!   destroyed or held by a human), with a positive score, and taken by fewer
//!   than `squads_per_target` other squads are candidates.
//! - Members are the squad's nearest available bots. A member keeps its place
//!   for its life; empty places are refilled. Bots with an armed-objective or
//!   repair duty, a supply task, or that hold their lane objective's action
//!   are not available.
//! - The team's nearest member becomes the structure's operator through the
//!   ordinary objective operator selection and arms the charge with the same
//!   request a player sends; the others take attack roles around it.
//! - Once the charge is armed the structure is no longer a candidate: the team
//!   ends and the armed-objective duties (`bot_armed_objectives`) guard it. A
//!   team that has not armed its structure within `give_up_ms` gives it up and
//!   the squad does not pick it again for `retry_ms`.
//!
//! Selection is O(squads x structures + bots) distance comparisons per tick.
use super::*;
use std::collections::{BTreeMap, BTreeSet};

/// Score of what destroying a structure changes, before the distance cost.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Values {
    /// Closes a forward spawn some of the owners' lives started at.
    pub spawn: f64,
    /// Closes a forward spawn of the owners' squads that no current life uses.
    pub idle_spawn: f64,
    /// Switches command abilities or opens a spawn of the attacking side.
    pub support: f64,
    /// Closes one of the owners' vehicle sources.
    pub vehicle_spawn: f64,
    /// Changes only the structure itself (gates, barricades, bridges).
    pub other: f64,
}
impl Default for Values {
    fn default() -> Self {
        Self {
            spawn: 100.,
            idle_spawn: 40.,
            support: 60.,
            vehicle_spawn: 30.,
            other: 20.,
        }
    }
}

/// Population-file policy (`demolition`). Omitted fields use these defaults.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Policy {
    pub enabled: bool,
    /// Match game types (`spawn_catalog` game type numbers) it runs in.
    pub game_types: Vec<u8>,
    /// Bots of one squad sent to its structure.
    pub team_size: usize,
    /// Squads that may send a team to the same structure.
    pub squads_per_target: usize,
    /// Teams one side keeps at once.
    pub teams_per_side: usize,
    /// After one of its teams ends, a side waits this long before it
    /// sends another.
    pub rest_ms: u64,
    /// Only structures this close (horizontally) to the squad's lane objective.
    pub reach_m: f64,
    /// Score lost per metre between the structure and the lane objective.
    pub distance_weight: f64,
    pub values: Values,
    /// A team that has not armed its structure by then gives it up.
    pub give_up_ms: u64,
    /// How long a squad leaves a structure it gave up.
    pub retry_ms: u64,
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            enabled: true,
            game_types: vec![2],
            team_size: 1,
            squads_per_target: 1,
            teams_per_side: 1,
            rest_ms: 90_000,
            reach_m: 150.,
            distance_weight: 0.5,
            values: Values::default(),
            give_up_ms: 90_000,
            retry_ms: 60_000,
        }
    }
}
impl Policy {
    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        let value = |v: f64| v.is_finite() && (0. ..=1000.).contains(&v);
        if self.game_types.len() > 8
            || self.game_types.iter().any(|&t| t > 7)
            || self.team_size > 8
            || !(1..=8).contains(&self.squads_per_target)
            || !(1..=32).contains(&self.teams_per_side)
            || self.rest_ms > 600_000
            || !self.reach_m.is_finite()
            || !(1. ..=2048.).contains(&self.reach_m)
            || !self.distance_weight.is_finite()
            || !(0. ..=100.).contains(&self.distance_weight)
            || ![
                self.values.spawn,
                self.values.idle_spawn,
                self.values.support,
                self.values.vehicle_spawn,
                self.values.other,
            ]
            .into_iter()
            .all(value)
            || !(1000..=600_000).contains(&self.give_up_ms)
            || self.retry_ms > 600_000
        {
            return Err("invalid_bot_demolition_policy");
        }
        Ok(())
    }
}

/// What completing a structure changes, as the mode runtime reads it from
/// its authored data. Teams are the spawn points' teams.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Hint {
    pub closes_spawn_of: Option<u8>,
    /// The squad spawn point it closes.
    pub spawn_point: Option<u16>,
    pub closes_vehicle_spawn_of: Option<u8>,
    pub opens_spawn_of: Option<u8>,
    pub toggles_abilities: bool,
}
impl Hint {
    /// Whether destroying the structure changes only its own geometry.
    pub(crate) fn changes_only_geometry(&self) -> bool {
        self.closes_spawn_of.is_none()
            && self.closes_vehicle_spawn_of.is_none()
            && self.opens_spawn_of.is_none()
            && !self.toggles_abilities
    }

    /// The best value class that applies when `attacker` destroys a
    /// structure `owner` owns. `in_use` says whether a current life of the
    /// owners started at the spawn it closes.
    pub(crate) fn value(&self, values: &Values, owner: u8, attacker: u8, in_use: bool) -> f64 {
        let mut value = values.other;
        if self.closes_vehicle_spawn_of == Some(owner) {
            value = value.max(values.vehicle_spawn);
        }
        if self.toggles_abilities || self.opens_spawn_of == Some(attacker) {
            value = value.max(values.support);
        }
        if self.closes_spawn_of == Some(owner) {
            value = value.max(if in_use {
                values.spawn
            } else {
                values.idle_spawn
            });
        }
        value
    }
}

/// One structure the attacking side may arm now.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Target {
    pub index: u16,
    pub center: [f64; 3],
    pub owner: u8,
    /// Value for the side that does not own it.
    pub value: f64,
}

/// One squad with living bots, and where its lane objective is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Squad {
    pub squad: u16,
    pub team: u8,
    pub lane: Option<[f64; 3]>,
}

/// One live bot, as seen by the team selection.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Candidate {
    pub id: u16,
    pub generation: u32,
    pub squad: u16,
    pub position: [f64; 3],
    /// May join a team now.
    pub available: bool,
}

/// A membership or team edge, reported once.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Change {
    Picked {
        squad: u16,
        target: u16,
        score: f64,
    },
    Ended {
        squad: u16,
        target: u16,
        reason: &'static str,
    },
    Joined {
        squad: u16,
        target: u16,
        bot: u16,
    },
    Left {
        squad: u16,
        target: u16,
        bot: u16,
    },
}

#[derive(Clone, Debug)]
struct Team {
    team: u8,
    target: u16,
    since: Duration,
    members: BTreeMap<u16, u32>,
}

/// Sticky demolition teams across ticks.
#[derive(Default)]
pub(crate) struct Book {
    teams: BTreeMap<u16, Team>,
    refused: BTreeMap<(u16, u16), Duration>,
    /// When each side may send its next team.
    resting: BTreeMap<u8, Duration>,
}

fn flat(a: [f64; 3], b: [f64; 3]) -> f64 {
    (a[0] - b[0]).hypot(a[2] - b[2])
}

impl Book {
    pub(crate) fn clear(&mut self, changes: &mut Vec<Change>) {
        for (squad, team) in std::mem::take(&mut self.teams) {
            for bot in team.members.keys() {
                changes.push(Change::Left {
                    squad,
                    target: team.target,
                    bot: *bot,
                });
            }
            changes.push(Change::Ended {
                squad,
                target: team.target,
                reason: "disabled",
            });
        }
        self.refused.clear();
        self.resting.clear();
    }

    /// Update the teams for this tick and return each member's structure.
    pub(crate) fn plan(
        &mut self,
        now: Duration,
        targets: &[Target],
        squads: &[Squad],
        bots: &[Candidate],
        policy: &Policy,
        changes: &mut Vec<Change>,
    ) -> BTreeMap<u16, u16> {
        self.refused.retain(|_, until| *until > now);
        self.resting.retain(|_, until| *until > now);
        let current: BTreeMap<u16, &Target> = targets.iter().map(|t| (t.index, t)).collect();
        let live: BTreeMap<u16, &Squad> = squads.iter().map(|s| (s.squad, s)).collect();
        let bot_of: BTreeMap<u16, &Candidate> = bots.iter().map(|b| (b.id, b)).collect();
        // Teams end with their structure's availability, their squad, or
        // their patience.
        let mut ended = Vec::new();
        for (&squad, team) in &self.teams {
            let reason = if !current.contains_key(&team.target) {
                Some("unavailable")
            } else if live.get(&squad).is_none_or(|s| s.lane.is_none()) {
                Some("squad_unavailable")
            } else if now.saturating_sub(team.since) >= Duration::from_millis(policy.give_up_ms) {
                Some("gave_up")
            } else {
                None
            };
            if let Some(reason) = reason {
                ended.push((squad, reason));
            }
        }
        for (squad, reason) in ended {
            let team = self.teams.remove(&squad).unwrap();
            self.resting
                .insert(team.team, now + Duration::from_millis(policy.rest_ms));
            if reason == "gave_up" {
                self.refused.insert(
                    (squad, team.target),
                    now + Duration::from_millis(policy.retry_ms),
                );
            }
            for bot in team.members.keys() {
                changes.push(Change::Left {
                    squad,
                    target: team.target,
                    bot: *bot,
                });
            }
            changes.push(Change::Ended {
                squad,
                target: team.target,
                reason,
            });
        }
        // New teams, squads in index order.
        for squad in squads {
            if self.teams.contains_key(&squad.squad)
                || self.resting.contains_key(&squad.team)
                || self.teams.values().filter(|t| t.team == squad.team).count()
                    >= policy.teams_per_side
            {
                continue;
            }
            let Some(lane) = squad.lane else {
                continue;
            };
            if !bots.iter().any(|b| b.squad == squad.squad && b.available) {
                continue;
            }
            let best = targets
                .iter()
                .filter(|t| t.owner != squad.team)
                .filter(|t| !self.refused.contains_key(&(squad.squad, t.index)))
                .filter(|t| {
                    self.teams.values().filter(|x| x.target == t.index).count()
                        < policy.squads_per_target
                })
                .filter_map(|t| {
                    let d = flat(lane, t.center);
                    let score = t.value - policy.distance_weight * d;
                    (d <= policy.reach_m && score > 0.).then_some((score, t.index))
                })
                .max_by(|a, b| a.0.total_cmp(&b.0).then(b.1.cmp(&a.1)));
            if let Some((score, target)) = best {
                changes.push(Change::Picked {
                    squad: squad.squad,
                    target,
                    score,
                });
                self.teams.insert(
                    squad.squad,
                    Team {
                        team: squad.team,
                        target,
                        since: now,
                        members: BTreeMap::new(),
                    },
                );
            }
        }
        // Members keep their place for their life; empty places are refilled
        // by the squad's nearest available bots.
        let mut taken: BTreeSet<u16> = BTreeSet::new();
        for (&squad, team) in &mut self.teams {
            let center = current[&team.target].center;
            let gone: Vec<u16> = team
                .members
                .iter()
                .filter(|(id, generation)| {
                    bot_of
                        .get(id)
                        .is_none_or(|b| b.generation != **generation || !b.available)
                })
                .map(|(id, _)| *id)
                .collect();
            for bot in gone {
                team.members.remove(&bot);
                changes.push(Change::Left {
                    squad,
                    target: team.target,
                    bot,
                });
            }
            taken.extend(team.members.keys().copied());
            let mut free: Vec<(f64, u16, u32)> = bots
                .iter()
                .filter(|b| b.squad == squad && b.available && !taken.contains(&b.id))
                .map(|b| (flat(b.position, center), b.id, b.generation))
                .collect();
            free.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            for (_, bot, generation) in free {
                if team.members.len() >= policy.team_size {
                    break;
                }
                team.members.insert(bot, generation);
                taken.insert(bot);
                changes.push(Change::Joined {
                    squad,
                    target: team.target,
                    bot,
                });
            }
        }
        self.teams
            .values()
            .flat_map(|team| team.members.keys().map(|bot| (*bot, team.target)))
            .collect()
    }
}

impl<W: Write> Engine<W> {
    fn bot_demolition_policy(&self) -> Option<Policy> {
        let config = self.bot_registry.config.as_ref()?;
        let interactions = config
            .objective_policy
            .as_ref()
            .is_some_and(|p| p.interactions_enabled);
        let policy = &config.demolition;
        (interactions
            && policy.enabled
            && policy
                .game_types
                .iter()
                .any(|&t| i16::from(t) == i16::from(self.match_profile().game_type)))
        .then(|| policy.clone())
    }

    /// What completing each structure changes, from the mode runtime.
    fn bot_structure_hints(&self) -> BTreeMap<u16, Hint> {
        self.domination_gameplay
            .as_ref()
            .map(|world| world.runtime.structure_hints())
            .unwrap_or_default()
    }

    /// Structures a side may arm now, in object order.
    pub(crate) fn demolition_targets(&self, values: &Values) -> Vec<Target> {
        use replication::ObjectiveState::{Activating, Deactivated, Repaired};
        let (Some(catalog), Some(world)) = (&self.objective_catalog, &self.objective_world) else {
            return Vec::new();
        };
        let hints = self.bot_structure_hints();
        // Spawn points a current life of each side started at.
        let mut used: BTreeSet<(u8, u16)> = self
            .bot_registry
            .bots
            .values()
            .filter(|b| b.health > 0)
            .filter_map(|b| Some((b.target.squad.team(), b.life_spawn?)))
            .collect();
        used.extend(
            self.peers
                .values()
                .chain(self.active.iter())
                .filter(|a| a.life.is_alive())
                .map(|a| (a.team(), a.squad_spawn)),
        );
        catalog
            .objects
            .iter()
            .filter_map(|object| {
                let rule = object.demolition.as_ref()?;
                let index = object.object_index;
                if !matches!(world.state(index)?, Deactivated | Activating | Repaired) {
                    return None;
                }
                // A human's hold is theirs.
                if world
                    .interaction_owner(index)
                    .is_some_and(|(id, _, _)| !self.bot_registry.bots.contains_key(&id))
                {
                    return None;
                }
                let owner = rule.owner_team;
                let hint = hints.get(&index).copied().unwrap_or_default();
                let in_use = hint
                    .spawn_point
                    .is_some_and(|point| used.contains(&(owner, point)));
                let value = hint.value(values, owner, 1 - owner.min(1), in_use);
                Some(Target {
                    index,
                    center: rule.position.map(f64::from),
                    owner,
                    value,
                })
            })
            .collect()
    }

    /// Demolition team membership for this tick (see module documentation).
    /// `busy` bots have another duty or hold their lane's action.
    pub(crate) fn plan_demolition_duties(
        &mut self,
        now: Duration,
        busy: &BTreeSet<u16>,
        receipts: &mut Vec<Value>,
    ) -> BTreeMap<u16, u16> {
        let mut changes = Vec::new();
        let policy = self
            .bot_demolition_policy()
            .filter(|_| self.match_started && self.match_terminal.is_none());
        let duties = if let Some(policy) = policy {
            let targets = self.demolition_targets(&policy.values);
            let bots: Vec<Candidate> = self
                .bot_registry
                .bots
                .iter()
                .filter(|(_, b)| b.health > 0)
                .filter_map(|(&id, b)| {
                    let r = b.target.runtime?;
                    Some(Candidate {
                        id,
                        generation: r.generation,
                        squad: b.target.squad.squad_index(),
                        position: point(r.position),
                        available: !busy.contains(&id)
                            && !matches!(r.phase, development_target::BotPhase::Dead)
                            && !b.has_supply_action(),
                    })
                })
                .collect();
            let squads: Vec<Squad> = self
                .bot_registry
                .bots
                .values()
                .map(|b| (b.target.squad.squad_index(), b.target.squad.team()))
                .collect::<BTreeSet<_>>()
                .into_iter()
                .map(|(squad, team)| Squad {
                    squad,
                    team,
                    lane: self
                        .bot_squad_lane(squad)
                        .and_then(|lane| self.bot_objective_center(lane))
                        .map(|(center, _)| center.map(f64::from)),
                })
                .collect();
            self.bot_registry.director.demolition.plan(
                now,
                &targets,
                &squads,
                &bots,
                &policy,
                &mut changes,
            )
        } else {
            self.bot_registry.director.demolition.clear(&mut changes);
            BTreeMap::new()
        };
        let at = now.as_millis() as u64;
        for change in changes {
            receipts.push(match change {
                Change::Picked {
                    squad,
                    target,
                    score,
                } => json!({"op":"bot_demolition_team","timestamp_ms":at,"squad_index":squad,
                    "objective":target,"picked":true,"score":score}),
                Change::Ended {
                    squad,
                    target,
                    reason,
                } => json!({"op":"bot_demolition_team","timestamp_ms":at,"squad_index":squad,
                    "objective":target,"picked":false,"reason":reason}),
                Change::Joined { squad, target, bot } => json!({"op":"bot_demolition_duty",
                    "timestamp_ms":at,"squad_index":squad,"objective":target,"object_index":bot,"joined":true}),
                Change::Left { squad, target, bot } => json!({"op":"bot_demolition_duty",
                    "timestamp_ms":at,"squad_index":squad,"objective":target,"object_index":bot,"joined":false}),
            });
        }
        duties
    }
}

fn point(p: replication::CharacterPosition) -> [f64; 3] {
    [p.x.into(), p.y.into(), p.z.into()]
}

#[cfg(test)]
#[path = "bot_demolition_tests.rs"]
mod tests;
