//! Population bots follow their squad's FRAGO and lane.
//!
//! Specified in docs/bot-squad-orders.md. This is an original design for this
//! server, not a decoded behaviour: retail has no bots, and its authored
//! automatic FRAGO lists are empty, so only a human squad leader's tag ever
//! names a FRAGO objective there.
//!
//! - Each squad works on one objective. A squad with a human leader whose
//!   tag names an objective that is still open for the squad's team works on
//!   that objective, inside or outside its lane. Otherwise the squad works on
//!   its lane objective, the mode runtime's next objective in the squad's
//!   authored list. A squad whose lane is exhausted helps the nearest
//!   objective another squad of its team works on.
//! - An objective is open for a team while the team has an action there
//!   (arm, capture, secure, defuse, deactivate or retake) or while it is
//!   neither inactive, completed nor locked (for example the team's own point
//!   to defend, or a charge that is already armed).
//! - A squad without a human leader keeps its tag equal to the objective it
//!   works on, so human members see the order its bots follow. When the
//!   objective completes and the squad moves on, the tag moves with it. A
//!   human who becomes the squad's leader inherits that tag; the server keeps
//!   moving it until the leader sets or clears a tag of their own.
//! - Duties (armed objectives, blast evacuation, repair, demolition teams,
//!   resupply) still interrupt this objective for the bots that take them.
//!
//! Selection is O(squads^2) distance comparisons per tick.
use super::*;
use std::collections::BTreeMap;

const OBJECTIVE_FAMILY: u32 = 0x0c80_0000;

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Policy {
    /// Bots work on their human-led squad's FRAGO objective before the lane,
    /// and on the nearest objective of their team once their lane is exhausted.
    /// `false` keeps the lane only, as before.
    pub follow_orders: bool,
    /// Squads without a human leader tag the objective their bots work on.
    pub automatic_orders: bool,
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            follow_orders: true,
            automatic_orders: true,
        }
    }
}

/// The objective index a full tag names, when it names an objective.
pub(crate) fn tagged_objective(tag: u32) -> Option<u16> {
    (tag & 0xffff_0000 == OBJECTIVE_FAMILY && tag as u16 != u16::MAX).then_some(tag as u16)
}

pub(crate) fn objective_tag(objective: Option<u16>) -> u32 {
    objective.map_or(u32::MAX, |index| OBJECTIVE_FAMILY | u32::from(index))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Source {
    /// The squad leader's tag.
    Order,
    /// The squad's lane objective.
    Lane,
    /// The nearest objective of another squad of the team (lane exhausted).
    Nearest,
}
impl Source {
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::Order => "frago",
            Self::Lane => "lane",
            Self::Nearest => "nearest_lane",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct SquadInput {
    pub squad: u16,
    pub team: u8,
    /// The tag belongs to a human leader (not one the server keeps).
    pub human_order: bool,
    /// Effective tag, FFFFFFFF when none.
    pub tag: u32,
    pub lane: Option<u16>,
    /// Mean position of the squad's living bots.
    pub position: Option<[f64; 3]>,
}

pub(crate) type Choice = Option<(u16, Source)>;

/// The objective each squad works on. `open(objective, team)` says whether the
/// objective is still open for the team; `center` is its place, when bots can
/// reach it.
pub(crate) fn choose(
    squads: &[SquadInput],
    follow_orders: bool,
    open: impl Fn(u16, u8) -> bool,
    center: impl Fn(u16) -> Option<[f64; 3]>,
) -> BTreeMap<u16, Choice> {
    let mut chosen: BTreeMap<u16, Choice> = squads
        .iter()
        .map(|s| {
            let order = tagged_objective(s.tag)
                .filter(|_| follow_orders && s.human_order)
                .filter(|&index| open(index, s.team) && center(index).is_some());
            let choice = order
                .map(|index| (index, Source::Order))
                .or_else(|| s.lane.map(|index| (index, Source::Lane)));
            (s.squad, choice)
        })
        .collect();
    if !follow_orders {
        return chosen;
    }
    // Squads with an exhausted lane help the nearest objective another squad
    // of their team works on now (ties: lower objective index).
    let working: Vec<(u8, u16)> = squads
        .iter()
        .filter_map(|s| Some((s.team, chosen[&s.squad]?.0)))
        .collect();
    let idle: Vec<&SquadInput> = squads
        .iter()
        .filter(|s| chosen[&s.squad].is_none())
        .collect();
    for s in idle {
        let mut best: Option<(f64, u16)> = None;
        for &(_, index) in working.iter().filter(|(team, _)| *team == s.team) {
            let Some(at) = center(index) else { continue };
            let d = s
                .position
                .map_or(0., |p| (p[0] - at[0]).hypot(p[2] - at[2]));
            if best.is_none_or(|(bd, bi)| d < bd || (d == bd && index < bi)) {
                best = Some((d, index));
            }
        }
        if let Some((_, index)) = best {
            chosen.insert(s.squad, Some((index, Source::Nearest)));
        }
    }
    chosen
}

/// The tag a squad without a human leader should carry, when it differs from
/// its current one.
pub(crate) fn automatic_tag(current: u32, choice: Choice) -> Option<u32> {
    let wanted = objective_tag(choice.map(|(index, _)| index));
    (wanted != current).then_some(wanted)
}

/// Tags this server set per squad, so a later human leader's own tag is told
/// apart from the one they inherited.
#[derive(Default)]
pub(crate) struct Book {
    automatic: BTreeMap<u16, u32>,
    /// The objective each bot squad worked on at the last bot tick.
    current: BTreeMap<u16, Option<u16>>,
    /// The last tag the mode refused per squad, not asked for again.
    refused: BTreeMap<u16, u32>,
}
impl Book {
    /// Whether the squad's current tag belongs to a human leader.
    pub(crate) fn human_order(&mut self, squad: u16, led: bool, manual: u32) -> bool {
        if !led {
            return false;
        }
        if self.automatic.get(&squad) == Some(&manual) {
            return false;
        }
        // The leader replaced or cleared the inherited tag: it is theirs now.
        self.automatic.remove(&squad);
        true
    }
    pub(crate) fn set(&mut self, squad: u16, tag: u32) {
        self.automatic.insert(squad, tag);
        self.refused.remove(&squad);
    }
    pub(crate) fn refuse(&mut self, squad: u16, tag: u32) {
        self.refused.insert(squad, tag);
    }
    pub(crate) fn refused(&self, squad: u16, tag: u32) -> bool {
        self.refused.get(&squad) == Some(&tag)
    }
}

impl<W: Write> Engine<W> {
    /// Whether a team still has something to do at an objective.
    pub(crate) fn objective_open_for(&self, objective: u16, team: u8) -> bool {
        let Some((catalog, world)) = self
            .objective_catalog
            .as_ref()
            .zip(self.objective_world.as_ref())
        else {
            return false;
        };
        let Some(object) = catalog.objects.iter().find(|o| o.object_index == objective) else {
            return false;
        };
        let Some(state) = world.state(objective) else {
            return false;
        };
        crate::bot_objectives::eligible(state)
            || crate::bot_objectives::operator_action(object, state, team, world).is_some()
    }

    /// (effective tag, manual tag) of a squad whose mode keeps squad orders.
    pub(crate) fn squad_order_tags(&self, squad: u16) -> Option<(u32, u32)> {
        if let Some(runtime) = &self.acquisition_gameplay {
            let s = runtime.state.squad(squad)?;
            let effective = if s.manual_tag != u32::MAX {
                s.manual_tag
            } else {
                s.assignment.tagged_object
            };
            return Some((effective, s.manual_tag));
        }
        if let Some(world) = &self.domination_gameplay {
            let slot = world.state.assignments.get(usize::from(squad))?.as_ref()?;
            let effective = if slot.manual_tag != u32::MAX {
                slot.manual_tag
            } else {
                slot.assignment.tagged_object
            };
            return Some((effective, slot.manual_tag));
        }
        if self.sabotage_squad_assignments() {
            self.objective_catalog
                .as_ref()?
                .initial_assignment(squad)
                .ok()?;
            let tag = self.squad_orders.as_ref()?.state.manual_tag(squad);
            return Some((tag, tag));
        }
        None
    }

    /// The objective each bot squad works on now, keeping automatic tags of
    /// squads without a human leader in step (recorded before they apply).
    pub(super) fn bot_squad_objectives(
        &mut self,
        now: Duration,
        wall: u64,
    ) -> Result<BTreeMap<u16, Option<u16>>, &'static str> {
        let policy = self
            .bot_registry
            .config
            .as_ref()
            .map(|c| c.squad_orders.clone())
            .unwrap_or_default();
        let mut members: BTreeMap<u16, (u8, [f64; 3], usize)> = BTreeMap::new();
        for bot in self.bot_registry.bots.values() {
            let squad = bot.target.squad.squad_index();
            let entry = members
                .entry(squad)
                .or_insert((bot.target.squad.team(), [0.; 3], 0));
            if let Some(r) = bot.target.runtime.filter(|_| bot.health > 0) {
                entry.1[0] += f64::from(r.position.x);
                entry.1[1] += f64::from(r.position.y);
                entry.1[2] += f64::from(r.position.z);
                entry.2 += 1;
            }
        }
        self.refresh_breach(now, wall)?;
        let playing = self.match_started && self.match_terminal.is_none();
        let mut inputs = Vec::with_capacity(members.len());
        let mut manual = BTreeMap::new();
        for (&squad, &(team, sum, alive)) in &members {
            let tags = self.squad_order_tags(squad);
            let led = self.squad_has_held_leader(squad);
            let human_order = tags.is_some_and(|(_, m)| {
                self.bot_registry
                    .director
                    .squad_orders
                    .human_order(squad, led, m)
            });
            if let Some((_, m)) = tags {
                manual.insert(squad, (m, led));
            }
            inputs.push(SquadInput {
                squad,
                team,
                human_order,
                tag: tags.map_or(u32::MAX, |(effective, _)| effective),
                lane: self.bot_squad_lane(squad),
                position: (alive > 0).then(|| sum.map(|v| v / alive as f64)),
            });
        }
        let chosen = choose(
            &inputs,
            policy.follow_orders,
            |index, team| self.objective_open_for(index, team),
            |index| {
                self.bot_objective_center(index)
                    .map(|(c, _)| c.map(f64::from))
            },
        );
        // Squads without a human order keep their tag on what their bots do.
        let tags: Vec<(u16, u32, u32, Choice)> =
            if policy.automatic_orders && playing && self.squad_orders.is_some() {
                inputs
                    .iter()
                    .filter(|s| !s.human_order)
                    .filter_map(|s| {
                        let (current, _) = *manual.get(&s.squad)?;
                        let choice = chosen[&s.squad];
                        automatic_tag(current, choice)
                            .filter(|&tag| {
                                !self
                                    .bot_registry
                                    .director
                                    .squad_orders
                                    .refused(s.squad, tag)
                            })
                            .map(|tag| (s.squad, current, tag, choice))
                    })
                    .collect()
            } else {
                Vec::new()
            };
        if !tags.is_empty() {
            let requested: Vec<(u16, u32)> = tags.iter().map(|&(s, _, t, _)| (s, t)).collect();
            let (candidate, applied) = self.plan_automatic_squad_tags(&requested)?;
            self.record(
                json!({"op":"bot_squad_orders","timestamp_ms":now.as_millis() as u64,
                    "squads":tags.iter().map(|&(squad, previous, tag, choice)| json!({
                        "squad_index":squad,"previous_tag":previous,"tag":tag,
                        "objective":choice.map(|(index, _)| index),
                        "source":choice.map(|(_, source)| source.code()),
                        "applied":applied.get(&squad).is_some_and(|r| r.is_ok()),
                        "error":applied.get(&squad).and_then(|r| r.err()),
                    })).collect::<Vec<_>>()}),
                wall,
            )?;
            if let Some(candidate) = candidate {
                self.install_squad_order_candidate(candidate);
            }
            for (squad, tag) in requested {
                let book = &mut self.bot_registry.director.squad_orders;
                if applied.get(&squad).is_some_and(|r| r.is_ok()) {
                    // A led squad still following the server's tag stays automatic.
                    book.set(squad, tag);
                } else {
                    book.refuse(squad, tag);
                }
            }
        }
        let current: BTreeMap<u16, Option<u16>> = chosen
            .into_iter()
            .map(|(squad, choice)| (squad, choice.map(|(index, _)| index)))
            .collect();
        self.bot_registry.director.squad_orders.current = current.clone();
        Ok(current)
    }

    /// The objective a squad works on now: its human leader's FRAGO, else its
    /// lane objective (in Acquisition the lane defence it breaks first), else
    /// the nearest objective of its team, as chosen at the last bot tick.
    /// Before a squad's first bot tick, or for a squad without bots, its lane.
    /// Bot spawn ranking reads it (its change lands separately).
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn bot_squad_current_objective(&self, squad: u16) -> Option<u16> {
        match self.bot_registry.director.squad_orders.current.get(&squad) {
            Some(&current) => current,
            None => self.bot_squad_lane(squad),
        }
    }
}

#[cfg(test)]
#[path = "bot_squad_orders_tests.rs"]
mod tests;
