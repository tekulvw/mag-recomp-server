//! Domination attackers fight for their points as one force.
//!
//! Specified in docs/bot-assault.md. This is an original policy for this
//! server's bots, not a decoded behaviour: retail has no bots. What it reads
//! is retail data: a point's secure-and-hold states and the controller rules
//! that advance a stage only while every point of it is secured at once.
//!
//! - **Points.** Attacking bots of a squad whose objective is a
//!   secure-and-hold point stand on a ring inside or at the edge of the point
//!   (`hold_radius_m`), whatever its state, instead of the general attack
//!   roles that kept most of a squad 18-34 m back. While the point is in its
//!   completion window, the attackers' armed-objective guards use the same
//!   ring. The operator still walks into a trigger and holds the action.
//! - **Stages.** A stage advances only while all of its points are secured
//!   at once. While some point of a stage is secured (completed, or being
//!   retaken) and another is still open, only the first `stage_hold_squads`
//!   attacking squads of the secured point (by squad index) stay to hold it.
//!   The others reinforce the open point with the fewest squads (ties: lower
//!   objective index). A point the defenders take back sends its
//!   reinforcements home.
//! - **Defences.** While a point's defences stand, `breach_reserve_squads`
//!   of its attacking squads stay on the point instead of all of them
//!   breaking the defences first (bot_breach).
//! - **Waves.** A squad whose bots are all far from its open point gathers at
//!   a rally place `rally_back_m` before it (towards the squad's home spawn)
//!   until `rally_share` of its living bots are there, or `rally_wait_ms`
//!   after the first got there, then pushes in together. A push lasts at least `rally_wait_ms`;
//!   the squad gathers again only once it has no living bot within
//!   `rally_back_m + rally_radius_m` of the point.
//!
//! Each bot tick is O(groups x squads) for the stages and O(squads x bots)
//! for the rallies.
use super::*;
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Policy {
    /// `false` keeps the general attack roles, every squad on its own lane
    /// point and no rally, as before.
    pub enabled: bool,
    /// Match game types it runs in (2 is Domination).
    pub game_types: Vec<u8>,
    /// Inner and outer radius of the attackers' ring around their point.
    pub hold_radius_m: [f64; 2],
    /// Attacking squads that stay on a secured point while its stage is open.
    pub stage_hold_squads: u16,
    /// Attacking squads of a point that stay on it while the others break its
    /// defences (raises `breach.attacker_reserve_squads` in these matches).
    pub breach_reserve_squads: u16,
    /// Distance of the rally place from the point; `0` turns rallies off.
    pub rally_back_m: f64,
    /// Bots within this distance of the rally place have gathered.
    pub rally_radius_m: f64,
    /// Share of the squad's living bots that must gather before the push.
    pub rally_share: f64,
    /// Longest a squad waits at its rally place.
    pub rally_wait_ms: u64,
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            enabled: true,
            game_types: vec![2],
            hold_radius_m: [2.5, 7.],
            stage_hold_squads: 1,
            breach_reserve_squads: 1,
            rally_back_m: 35.,
            rally_radius_m: 12.,
            rally_share: 0.6,
            rally_wait_ms: 20_000,
        }
    }
}
impl Policy {
    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        let [inner, outer] = self.hold_radius_m;
        if self.game_types.len() > 8
            || self.game_types.iter().any(|&t| t > 7)
            || !(inner.is_finite() && outer.is_finite())
            || !(0.5..=20.).contains(&inner)
            || !(inner..=30.).contains(&outer)
            || self.stage_hold_squads > 16
            || self.breach_reserve_squads > 16
            || !self.rally_back_m.is_finite()
            || !(self.rally_back_m == 0. || (10. ..=80.).contains(&self.rally_back_m))
            || !self.rally_radius_m.is_finite()
            || !(2. ..=40.).contains(&self.rally_radius_m)
            || !self.rally_share.is_finite()
            || !(0. ..=1.).contains(&self.rally_share)
            || self.rally_wait_ms > 120_000
        {
            return Err("invalid_bot_assault_policy");
        }
        Ok(())
    }
}

/// Whether the attackers hold or are taking a secure-and-hold point: from
/// the start of their hold until the defenders have taken it back.
pub(crate) fn secured(state: replication::ObjectiveState) -> bool {
    use replication::ObjectiveState::*;
    matches!(state, Completed | Retaking | Retaken | CancelRetaken)
}

/// Whether the attackers still have to win a point of an open stage: it is
/// open, being taken, or inside its completion window.
pub(crate) fn contested(state: replication::ObjectiveState) -> bool {
    use replication::ObjectiveState::*;
    matches!(state, Deactivated | Activating | Activated | Deactivating)
}

/// One attacking squad on a point of a stage this tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OnPoint {
    pub squad: u16,
    pub point: u16,
}

/// Squads that leave their secured point for an open point of its stage, as
/// (squad, point it reinforces).
pub(crate) fn reinforcements(
    groups: &[Vec<u16>],
    state: impl Fn(u16) -> Option<replication::ObjectiveState>,
    squads: &[OnPoint],
    hold_squads: u16,
) -> BTreeMap<u16, u16> {
    let mut moved = BTreeMap::new();
    for group in groups {
        let open: Vec<u16> = group
            .iter()
            .copied()
            .filter(|&p| state(p).is_some_and(contested))
            .collect();
        if open.is_empty() {
            continue;
        }
        let mut load: BTreeMap<u16, usize> = open
            .iter()
            .map(|&p| (p, squads.iter().filter(|s| s.point == p).count()))
            .collect();
        for &held in group.iter().filter(|&&p| state(p).is_some_and(secured)) {
            let mut on: Vec<u16> = squads
                .iter()
                .filter(|s| s.point == held)
                .map(|s| s.squad)
                .collect();
            on.sort_unstable();
            for squad in on.into_iter().skip(usize::from(hold_squads)) {
                let (&target, _) = load
                    .iter()
                    .min_by(|a, b| a.1.cmp(b.1).then(a.0.cmp(b.0)))
                    .unwrap();
                *load.get_mut(&target).unwrap() += 1;
                moved.insert(squad, target);
            }
        }
    }
    moved
}

/// Deterministic place on the attackers' ring around a point. Places spread
/// by the golden angle over the squad's slot and its rank on the point; each
/// failed candidate rotates the place and alternates the radius.
pub(crate) fn ring_point(
    center: [f64; 3],
    ring: [f64; 2],
    slot: usize,
    attempt: usize,
) -> [f64; 3] {
    const GOLDEN: f64 = 2.399_963_229_728_653;
    let attempt = attempt % 64;
    let angle = slot as f64 * GOLDEN + attempt as f64 * std::f64::consts::FRAC_PI_8;
    let blend = ((slot + attempt) % 3) as f64 / 2.;
    let radius = ring[0] + (ring[1] - ring[0]) * blend;
    [
        center[0] + angle.cos() * radius,
        center[1] + 0.5,
        center[2] + angle.sin() * radius,
    ]
}

/// The rally place: `back_m` from the point towards the squad's home.
pub(crate) fn rally_point(center: [f64; 3], home: [f64; 3], back_m: f64) -> [f64; 3] {
    let (dx, dz) = (home[0] - center[0], home[2] - center[2]);
    let len = dx.hypot(dz);
    let (x, z) = if len > 0.01 {
        (dx / len, dz / len)
    } else {
        (1., 0.)
    };
    let back = back_m.min(len);
    [center[0] + x * back, center[1] + 0.5, center[2] + z * back]
}

fn flat(a: [f64; 3], b: [f64; 3]) -> f64 {
    (a[0] - b[0]).hypot(a[2] - b[2])
}

/// A squad's wave state on one open point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Wave {
    /// Gathering at the rally place; `since` is when its first bot got
    /// there.
    Rally { point: u16, since: Option<Duration> },
    /// Pushing into the point since then.
    Push { point: u16, since: Duration },
}

/// What a squad's living bots look like for its wave this tick.
#[derive(Clone, Copy, Debug)]
pub(crate) struct WaveInput {
    pub squad: u16,
    /// The open point the squad attacks, when it attacks one.
    pub point: Option<u16>,
    pub center: [f64; 3],
    pub rally: [f64; 3],
}

/// Advance every squad's wave. Returns the squads whose wave changed, with
/// their new state (`None`: the squad no longer attacks an open point).
pub(crate) fn advance_waves(
    waves: &mut BTreeMap<u16, Wave>,
    squads: &[WaveInput],
    bots: &BTreeMap<u16, Vec<[f64; 3]>>,
    policy: &Policy,
    now: Duration,
) -> Vec<(u16, Option<Wave>)> {
    let mut changes = Vec::new();
    let present: BTreeSet<u16> = squads.iter().map(|s| s.squad).collect();
    let gone: Vec<u16> = waves
        .keys()
        .copied()
        .filter(|s| !present.contains(s))
        .collect();
    for squad in gone {
        waves.remove(&squad);
        changes.push((squad, None));
    }
    for s in squads {
        let Some(point) = s.point.filter(|_| policy.rally_back_m > 0.) else {
            if waves.remove(&s.squad).is_some() {
                changes.push((s.squad, None));
            }
            continue;
        };
        let alive = bots.get(&s.squad).map_or(&[][..], Vec::as_slice);
        if alive.is_empty()
            && waves.get(&s.squad).is_some_and(|w| match *w {
                Wave::Rally { point: p, .. } | Wave::Push { point: p, .. } => p == point,
            })
        {
            // Nobody to move: the squad keeps its wave until its next life.
            continue;
        }
        let forward = alive
            .iter()
            .any(|&p| flat(p, s.center) < policy.rally_back_m);
        // A push keeps going while a bot is anywhere up to the far side of
        // the rally place, so a squad leaving its rally place does not
        // flip straight back to gathering.
        let engaged = alive
            .iter()
            .any(|&p| flat(p, s.center) <= policy.rally_back_m + policy.rally_radius_m);
        let wait = Duration::from_millis(policy.rally_wait_ms);
        let gathered = alive
            .iter()
            .filter(|&&p| {
                flat(p, s.rally) <= policy.rally_radius_m || flat(p, s.center) < policy.rally_back_m
            })
            .count();
        let ready = !alive.is_empty() && gathered as f64 >= policy.rally_share * alive.len() as f64;
        let next = match waves.get(&s.squad).copied() {
            Some(Wave::Rally { point: p, since }) if p == point => {
                // The wait starts with the first bot at the rally place, so
                // a squad still walking up does not push in one by one.
                let since = since.or((gathered > 0).then_some(now));
                if ready || since.is_some_and(|t| now.saturating_sub(t) >= wait) {
                    Wave::Push { point, since: now }
                } else {
                    Wave::Rally { point, since }
                }
            }
            // A push lasts at least the rally wait, then while a bot is
            // engaged. A squad with a bot forward pushes at once; one with
            // none there, on a new point or after its push is spent,
            // gathers first.
            Some(Wave::Push { point: p, since })
                if p == point && (engaged || now.saturating_sub(since) < wait) =>
            {
                Wave::Push { point, since }
            }
            _ if forward => Wave::Push { point, since: now },
            _ => Wave::Rally {
                point,
                since: (gathered > 0).then_some(now),
            },
        };
        let previous = waves.insert(s.squad, next);
        // Only a new state or point is a change; a rally's first arrival is not.
        let same = match (previous, next) {
            (Some(Wave::Rally { point: a, .. }), Wave::Rally { point: b, .. }) => a == b,
            (previous, next) => previous == Some(next),
        };
        if !same {
            changes.push((s.squad, Some(next)));
        }
    }
    changes
}

impl<W: Write> Engine<W> {
    /// The policy, while it runs in this match.
    pub(crate) fn bot_assault_policy(&self) -> Option<Policy> {
        let config = self.bot_registry.config.as_ref()?;
        let interactions = config
            .objective_policy
            .as_ref()
            .is_some_and(|p| p.interactions_enabled);
        let policy = &config.assault;
        (interactions
            && policy.enabled
            && self.domination_gameplay.is_some()
            && self.match_started
            && self.match_terminal.is_none()
            && policy
                .game_types
                .iter()
                .any(|&t| i16::from(t) == i16::from(self.match_profile().game_type)))
        .then(|| policy.clone())
    }

    /// The owner team of a secure-and-hold point, when `objective` is one.
    pub(crate) fn assault_point_owner(&self, objective: u16) -> Option<u8> {
        let object = self
            .objective_catalog
            .as_ref()?
            .objects
            .iter()
            .find(|o| o.object_index == objective)?;
        (object.native_class == Some(objective_catalog::NativeClass::SecureHold))
            .then_some(object.interaction.as_ref()?.owner_team)
    }

    /// The armed-objective rings for `objective`: the attackers' guards of a
    /// point in its completion window stand on the point's ring.
    pub(crate) fn assault_armed_policy(
        &self,
        objective: u16,
        armed: &crate::bot_armed_objectives::ArmedPolicy,
        assault: Option<&Policy>,
    ) -> crate::bot_armed_objectives::ArmedPolicy {
        let mut armed = armed.clone();
        if let Some(policy) = assault
            && self.assault_point_owner(objective).is_some()
        {
            armed.guard_radius_m = policy.hold_radius_m;
        }
        armed
    }

    /// Squads that leave their secured point for an open point of its stage.
    pub(crate) fn assault_reinforcements(
        &self,
        squads: &[crate::bot_squad_orders::SquadInput],
        chosen: &BTreeMap<u16, crate::bot_squad_orders::Choice>,
    ) -> BTreeMap<u16, u16> {
        let (Some(policy), Some(world), Some(gameplay)) = (
            self.bot_assault_policy(),
            self.objective_world.as_ref(),
            self.domination_gameplay.as_ref(),
        ) else {
            return BTreeMap::new();
        };
        let groups = gameplay.runtime.stage_groups();
        if groups.is_empty() {
            return BTreeMap::new();
        }
        let on: Vec<OnPoint> = squads
            .iter()
            .filter_map(|s| {
                let (point, source) = chosen.get(&s.squad).copied().flatten()?;
                (source == crate::bot_squad_orders::Source::Lane
                    && self
                        .assault_point_owner(point)
                        .is_some_and(|owner| owner != s.team))
                .then_some(OnPoint {
                    squad: s.squad,
                    point,
                })
            })
            .collect();
        reinforcements(
            &groups,
            |point| world.state(point),
            &on,
            policy.stage_hold_squads,
        )
    }

    /// Advance the attacking squads' waves on their open points and return
    /// the squads gathering at their rally place. `members` holds the
    /// positions of each squad's living bots that work on its objective.
    pub(crate) fn plan_assault_waves(
        &mut self,
        policy: Option<&Policy>,
        assigned: &BTreeMap<u16, Option<u16>>,
        members: &BTreeMap<u16, Vec<[f64; 3]>>,
        now: Duration,
        receipts: &mut Vec<Value>,
    ) -> BTreeSet<u16> {
        let squads: Vec<WaveInput> = match policy {
            None => Vec::new(),
            Some(policy) => assigned
                .iter()
                .filter_map(|(&squad, &objective)| {
                    let team = replication::SquadAddress::from_index(squad).ok()?.team();
                    let objective = objective?;
                    let owner = self.assault_point_owner(objective)?;
                    let (center, _) = self.bot_objective_center(objective)?;
                    let center = center.map(f64::from);
                    let open = self.objective_world.as_ref()?.state(objective)
                        == Some(replication::ObjectiveState::Deactivated);
                    let home = self
                        .spawn_catalog
                        .as_ref()
                        .and_then(|c| c.default_for_squad(squad).and_then(|s| c.point(s)))
                        .map_or(center, |s| {
                            [
                                s.position.x.into(),
                                s.position.y.into(),
                                s.position.z.into(),
                            ]
                        });
                    (team != owner).then(|| WaveInput {
                        squad,
                        point: open.then_some(objective),
                        center,
                        rally: rally_point(center, home, policy.rally_back_m),
                    })
                })
                .collect(),
        };
        let default = Policy::default();
        let changes = advance_waves(
            &mut self.bot_registry.director.waves,
            &squads,
            members,
            policy.unwrap_or(&default),
            now,
        );
        for (squad, wave) in changes {
            let (state, point) = match wave {
                Some(Wave::Rally { point, .. }) => ("rally", Some(point)),
                Some(Wave::Push { point, .. }) => ("push", Some(point)),
                None => ("none", None),
            };
            receipts.push(
                json!({"op":"bot_assault_wave","timestamp_ms":now.as_millis() as u64,
                "squad_index":squad,"state":state,"objective":point,
                "living":members.get(&squad).map_or(0, Vec::len)}),
            );
        }
        self.bot_registry
            .director
            .waves
            .iter()
            .filter(|(_, w)| matches!(w, Wave::Rally { .. }))
            .map(|(&s, _)| s)
            .collect()
    }
}

#[cfg(test)]
#[path = "bot_assault_tests.rs"]
mod tests;
