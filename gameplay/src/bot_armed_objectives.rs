//! Armed-objective duties for population bots.
//!
//! An objective is *armed* while the attackers' action on it is complete but
//! still reversible by the defenders: a planted demolition charge whose fuse is
//! running (`Activated`, or `Deactivating` while someone defuses it), or a
//! Sabotage capture point inside its completion window that the owner team
//! may still deactivate. Only objectives the defenders can actually reverse
//! count; a charge that cannot be defused needs no response.
//!
//! While an objective is armed, a bounded number of bots from each team take
//! an objective-level duty that replaces their squad's normal destination:
//!
//! - Defenders **converge**: they move to a tight ring around the objective.
//!   The ordinary operator selection then picks the nearest converging (or
//!   already assigned) defender to perform the defuse/deactivate action.
//! - Attackers **guard**: they spread on a wider ring around it and engage
//!   anyone who comes near. While a defender is disarming it, guards prefer
//!   that defender as their target. They do not walk in to hunt it: on foot a
//!   moving bot cannot fire, and the disarm hold is only a few seconds long.
//!
//! Defenders clear before they disarm: a new defender operator is chosen for
//! an armed objective only while no living attacker is near it
//! (`contest_radius_m`), or once at most `late_disarm_ms` of its window or
//! fuse remains. A disarm already under way is kept.
//!
//! A bot that already stands inside its duty's outer radius first tries to
//! keep its current spot, so a guard already covering the objective keeps
//! firing instead of walking to a ring place.
//!
//! Membership is decided per objective and per team, nearest squads first
//! (squads are ranked by their closest available member), then nearest bots,
//! ties broken by squad and object index. A bot keeps its duty for the rest of
//! its life or until the objective stops being armed, so a role never flips
//! while the bot walks. Empty places are refilled from available bots in
//! range. When the objective is disarmed, completes or the match ends, every
//! duty is dropped and the bots return to their squad's normal objective.
//!
//! Duties are ordinary objective goals: routes are planned by the shared
//! bounded objective planner, so combat interrupts (perception holds) pause
//! and resume them like any other objective walk. The selection itself is
//! O(armed objectives x bots) distance comparisons per tick.
use super::*;
use std::collections::{BTreeMap, BTreeSet};

/// Population-tunable limits; a missing `armed` object uses these defaults.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArmedPolicy {
    pub(crate) enabled: bool,
    /// Most defenders that converge on one armed objective.
    pub(crate) defender_cap: usize,
    /// Most attackers that guard one armed objective.
    pub(crate) attacker_cap: usize,
    /// Only bots this close (straight line) to the objective may take a duty.
    pub(crate) reach_m: f64,
    /// Inner and outer radius of the converging defenders' ring.
    pub(crate) converge_radius_m: [f64; 2],
    /// Inner and outer radius of the guarding attackers' ring.
    pub(crate) guard_radius_m: [f64; 2],
    /// Living characters this close to an armed objective count as holding it.
    pub(crate) contest_radius_m: f64,
    /// With this little time left a defender disarms even when outnumbered.
    pub(crate) late_disarm_ms: u32,
}
impl Default for ArmedPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            defender_cap: 8,
            attacker_cap: 8,
            reach_m: 160.,
            converge_radius_m: [3., 7.],
            guard_radius_m: [8., 14.],
            contest_radius_m: 15.,
            late_disarm_ms: 8000,
        }
    }
}
impl ArmedPolicy {
    pub(crate) fn validate(&self) -> bool {
        let ring = |r: [f64; 2]| {
            r.iter().all(|v| v.is_finite()) && 1. <= r[0] && r[0] <= r[1] && r[1] <= 40.
        };
        self.defender_cap <= 32
            && self.attacker_cap <= 32
            && self.reach_m.is_finite()
            && (10. ..=1000.).contains(&self.reach_m)
            && ring(self.converge_radius_m)
            && ring(self.guard_radius_m)
            && self.contest_radius_m.is_finite()
            && (1. ..=60.).contains(&self.contest_radius_m)
            && self.late_disarm_ms <= 120_000
    }
}

/// One armed objective, as read from the objective world this tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Armed {
    pub index: u16,
    pub center: [f64; 3],
    pub defender_team: u8,
}

/// One live bot, as seen by the duty selection.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Candidate {
    pub id: u16,
    pub generation: u32,
    pub team: u8,
    pub squad: u16,
    pub position: [f64; 3],
    /// May take a new duty (alive and not busy with supply).
    pub available: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Duty {
    Converge,
    Guard,
}
impl Duty {
    pub(crate) fn ring(self, policy: &ArmedPolicy) -> [f64; 2] {
        match self {
            Self::Converge => policy.converge_radius_m,
            Self::Guard => policy.guard_radius_m,
        }
    }
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::Converge => "armed_converge",
            Self::Guard => "armed_guard",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Assignment {
    pub objective: u16,
    pub duty: Duty,
    /// Stable place index on the duty's ring, unique per objective and team.
    pub slot: u8,
}

/// A membership edge, reported once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Change {
    pub bot: u16,
    pub objective: u16,
    pub defending: bool,
    pub slot: u8,
    pub joined: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Member {
    generation: u32,
    defending: bool,
    slot: u8,
}

/// Sticky duty membership across ticks.
#[derive(Default)]
pub(crate) struct Book {
    episodes: BTreeMap<u16, BTreeMap<u16, Member>>,
}

fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    a.iter()
        .zip(b)
        .map(|(a, b)| (a - b).powi(2))
        .sum::<f64>()
        .sqrt()
}

impl Book {
    pub(crate) fn is_empty(&self) -> bool {
        self.episodes.is_empty()
    }

    /// Update membership for this tick and return each member's duty.
    pub(crate) fn plan(
        &mut self,
        armed: &[Armed],
        bots: &[Candidate],
        policy: &ArmedPolicy,
        changes: &mut Vec<Change>,
    ) -> BTreeMap<u16, Assignment> {
        let live: BTreeMap<u16, &Candidate> = bots.iter().map(|b| (b.id, b)).collect();
        let current: BTreeMap<u16, &Armed> = armed.iter().map(|a| (a.index, a)).collect();
        // Episodes end with the armed state; members end with their life.
        let ended: Vec<u16> = self
            .episodes
            .keys()
            .copied()
            .filter(|index| !current.contains_key(index) || !policy.enabled)
            .collect();
        for index in ended {
            for (bot, m) in self.episodes.remove(&index).unwrap() {
                changes.push(Change {
                    bot,
                    objective: index,
                    defending: m.defending,
                    slot: m.slot,
                    joined: false,
                });
            }
        }
        if !policy.enabled {
            return BTreeMap::new();
        }
        for (&index, members) in &mut self.episodes {
            let defender_team = current[&index].defender_team;
            members.retain(|&bot, m| {
                let keep = live.get(&bot).is_some_and(|b| {
                    b.generation == m.generation && (b.team == defender_team) == m.defending
                });
                if !keep {
                    changes.push(Change {
                        bot,
                        objective: index,
                        defending: m.defending,
                        slot: m.slot,
                        joined: false,
                    });
                }
                keep
            });
        }
        let mut taken: BTreeSet<u16> = self
            .episodes
            .values()
            .flat_map(|m| m.keys().copied())
            .collect();
        let mut sorted: Vec<&Armed> = armed.iter().collect();
        sorted.sort_by_key(|a| a.index);
        for objective in sorted {
            let members = self.episodes.entry(objective.index).or_default();
            for defending in [true, false] {
                let cap = if defending {
                    policy.defender_cap
                } else {
                    policy.attacker_cap
                };
                let mut count = members
                    .values()
                    .filter(|m| m.defending == defending)
                    .count();
                if count >= cap {
                    continue;
                }
                let mut pool: Vec<(f64, u16, u16)> = bots
                    .iter()
                    .filter(|b| {
                        b.available
                            && b.team <= 1
                            && (b.team == objective.defender_team) == defending
                            && !taken.contains(&b.id)
                    })
                    .map(|b| (distance(b.position, objective.center), b.squad, b.id))
                    .filter(|(d, _, _)| d.is_finite() && *d <= policy.reach_m)
                    .collect();
                if pool.is_empty() {
                    continue;
                }
                // Squads in order of their closest member, then members by distance.
                let mut squad_rank: BTreeMap<u16, f64> = BTreeMap::new();
                for &(d, squad, _) in &pool {
                    let best = squad_rank.entry(squad).or_insert(d);
                    if d < *best {
                        *best = d;
                    }
                }
                pool.sort_by(|a, b| {
                    squad_rank[&a.1]
                        .total_cmp(&squad_rank[&b.1])
                        .then(a.1.cmp(&b.1))
                        .then(a.0.total_cmp(&b.0))
                        .then(a.2.cmp(&b.2))
                });
                for (_, _, bot) in pool {
                    if count >= cap {
                        break;
                    }
                    let used: BTreeSet<u8> = members
                        .values()
                        .filter(|m| m.defending == defending)
                        .map(|m| m.slot)
                        .collect();
                    let slot = (0..=u8::MAX).find(|s| !used.contains(s)).unwrap();
                    members.insert(
                        bot,
                        Member {
                            generation: live[&bot].generation,
                            defending,
                            slot,
                        },
                    );
                    changes.push(Change {
                        bot,
                        objective: objective.index,
                        defending,
                        slot,
                        joined: true,
                    });
                    taken.insert(bot);
                    count += 1;
                }
            }
        }
        self.episodes.retain(|_, m| !m.is_empty());
        let mut duties = BTreeMap::new();
        for (&index, members) in &self.episodes {
            for (&bot, m) in members {
                let duty = if m.defending {
                    Duty::Converge
                } else {
                    Duty::Guard
                };
                duties.insert(
                    bot,
                    Assignment {
                        objective: index,
                        duty,
                        slot: m.slot,
                    },
                );
            }
        }
        duties
    }
}

/// Deterministic place on the duty's ring. Slots spread by the golden angle;
/// each failed candidate rotates the place and alternates the radius, so a
/// blocked or unreachable spot is replaced by a nearby one.
pub(crate) fn duty_point(
    center: [f64; 3],
    objective: u16,
    assignment: Assignment,
    policy: &ArmedPolicy,
    attempt: usize,
) -> [f64; 3] {
    const GOLDEN: f64 = 2.399_963_229_728_653;
    let ring = assignment.duty.ring(policy);
    // Teams start half a turn apart so their rings interleave.
    let side = if assignment.duty == Duty::Converge {
        0.
    } else {
        std::f64::consts::PI
    };
    let attempt = attempt % 64;
    let angle = f64::from(objective) * 0.61
        + side
        + f64::from(assignment.slot) * GOLDEN
        + attempt as f64 * std::f64::consts::FRAC_PI_4 * 0.5;
    let blend = ((usize::from(assignment.slot) + attempt) % 3) as f64 / 2.;
    let radius = ring[0] + (ring[1] - ring[0]) * blend;
    [
        center[0] + angle.cos() * radius,
        center[1] + 0.5,
        center[2] + angle.sin() * radius,
    ]
}

impl<W: Write> Engine<W> {
    /// Armed objectives the defenders can still reverse, in object order.
    pub(crate) fn armed_objectives(&self) -> Vec<Armed> {
        use replication::ObjectiveState::{Activated, Deactivating};
        let (Some(catalog), Some(world)) = (&self.objective_catalog, &self.objective_world) else {
            return Vec::new();
        };
        if !self.match_started || self.match_terminal.is_some() {
            return Vec::new();
        }
        catalog
            .objects
            .iter()
            .filter_map(|object| {
                let state = world.state(object.object_index)?;
                if !matches!(state, Activated | Deactivating) {
                    return None;
                }
                let (position, owner) = if let Some(rule) = &object.demolition {
                    rule.can_defuse
                        .then_some((rule.position, rule.owner_team))?
                } else {
                    let rule = object.interaction.as_ref()?;
                    world
                        .defender_can_deactivate(object.object_index)
                        .then_some((rule.position, rule.owner_team))?
                };
                Some(Armed {
                    index: object.object_index,
                    center: position.map(f64::from),
                    defender_team: owner,
                })
            })
            .collect()
    }

    /// Whether a new defender may start disarming `index` now (module docs).
    /// Objectives that are not armed are always open.
    pub(crate) fn disarm_open(&self, index: u16, now: Duration, policy: &ArmedPolicy) -> bool {
        if !policy.enabled {
            return true;
        }
        let Some(armed) = self
            .armed_objectives()
            .into_iter()
            .find(|a| a.index == index)
        else {
            return true;
        };
        let world = self.objective_world.as_ref().unwrap();
        if world
            .expiration_ms(index)
            .is_some_and(|end| end.saturating_sub(now.as_millis() as u32) <= policy.late_disarm_ms)
        {
            return true;
        }
        let near = |p: [f64; 3]| {
            (p[0] - armed.center[0]).hypot(p[2] - armed.center[2]) <= policy.contest_radius_m
        };
        let attacker = |team: u8| team <= 1 && team != armed.defender_team;
        let bots = self.bot_registry.bots.values().any(|b| {
            b.health > 0
                && attacker(b.target.squad.team())
                && b.target.runtime.is_some_and(|r| near(point(r.position)))
        });
        let humans = self.peers.values().chain(self.active.iter()).any(|a| {
            a.life.is_alive()
                && attacker(a.team())
                && a.tracking.sample.is_some_and(|s| near(point(s.position)))
        });
        !bots && !humans
    }

    /// The enemy currently disarming the objective this bot guards, if any.
    pub(crate) fn armed_priority_target(&self, bot: &crate::bot_registry::WorldBot) -> Option<u16> {
        let duty = bot.task.armed?;
        let world = self.objective_world.as_ref()?;
        // Only a disarm hold: the owner of any other hold is a teammate.
        (duty.duty == Duty::Guard
            && world.state(duty.objective) == Some(replication::ObjectiveState::Deactivating))
        .then(|| world.interaction_owner(duty.objective))
        .flatten()
        .map(|(player, _, _)| player)
    }

    /// Duty selection for this tick (see module documentation).
    pub(crate) fn plan_armed_duties(
        &mut self,
        policy: &ArmedPolicy,
        now: Duration,
        receipts: &mut Vec<Value>,
    ) -> BTreeMap<u16, Assignment> {
        let armed = if policy.enabled {
            self.armed_objectives()
        } else {
            Vec::new()
        };
        if armed.is_empty() && self.bot_registry.director.armed.is_empty() {
            return BTreeMap::new();
        }
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
                    team: b.target.squad.team(),
                    squad: b.target.squad.squad_index(),
                    position: point(r.position),
                    available: !matches!(r.phase, development_target::BotPhase::Dead)
                        && !b.has_supply_action(),
                })
            })
            .collect();
        let mut changes = Vec::new();
        let duties = self
            .bot_registry
            .director
            .armed
            .plan(&armed, &bots, policy, &mut changes);
        for c in changes {
            receipts.push(
                json!({"op":"bot_armed_objective_duty","timestamp_ms":now.as_millis() as u64,
                "object_index":c.bot,"objective":c.objective,"joined":c.joined,
                "role":if c.defending {"defend"} else {"attack"},"slot":c.slot,
                "duty":duties.get(&c.bot).map(|d| d.duty.code())}),
            );
        }
        duties
    }
}

fn point(p: replication::CharacterPosition) -> [f64; 3] {
    [p.x.into(), p.y.into(), p.z.into()]
}

#[cfg(test)]
#[path = "bot_armed_objectives_tests.rs"]
mod tests;
