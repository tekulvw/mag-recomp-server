//! Attacking bots keep out of the blast of a charge they armed.
//!
//! An armed demolition objective (charge planted, fuse running) explodes
//! with its authored instantaneous blast when the fuse ends. The blast is
//! centred on the trigger the charge was planted at, so the hot zone is the
//! objective's position widened by its farthest trigger, the authored outer
//! blast radius and `margin_m`. While the objective is armed:
//!
//! - Every living bot of the attacking side (the side that is not the
//!   objective's owner) inside the zone takes a guard duty on it whose ring
//!   lies outside the zone, so it walks out before the fuse ends. The planter
//!   is one of them: its operator action ends when the charge is armed.
//! - The attackers' guard ring (`bot_armed_objectives`) is pushed outside the
//!   zone, and a guard only keeps its current spot when it already stands
//!   outside the zone.
//! - An attacker's route that would enter a zone it does not start in is
//!   refused; the bot plans again later, by when the charge has gone off or
//!   been disarmed.
//!
//! Defenders are unaffected: they may stay to defend or disarm the charge.
//! The behaviour is specified in docs/bot-armed-objectives.md; this is an
//! original design for this server.
use super::*;
use std::collections::BTreeMap;

/// Population-file policy (`blast`). Omitted fields use these defaults.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Policy {
    pub enabled: bool,
    /// Distance kept beyond the authored blast radius.
    pub margin_m: f64,
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            enabled: true,
            margin_m: 4.,
        }
    }
}
impl Policy {
    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        if !self.margin_m.is_finite() || !(0. ..=20.).contains(&self.margin_m) {
            return Err("invalid_bot_blast_policy");
        }
        Ok(())
    }
}

/// The ground an armed objective's blast reaches, plus the margin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Zone {
    pub objective: u16,
    pub center: [f64; 3],
    pub radius: f64,
    /// The side that armed it, which the zone keeps out.
    pub attackers: u8,
}
impl Zone {
    pub(crate) fn contains(&self, p: [f64; 3]) -> bool {
        (p[0] - self.center[0]).hypot(p[2] - self.center[2]) < self.radius
    }
    /// Whether the horizontal segment from `a` to `b` passes inside.
    pub(crate) fn crossed_by(&self, a: [f64; 3], b: [f64; 3]) -> bool {
        let (dx, dz) = (b[0] - a[0], b[2] - a[2]);
        let length = dx * dx + dz * dz;
        let t = if length > 0. {
            (((self.center[0] - a[0]) * dx + (self.center[2] - a[2]) * dz) / length).clamp(0., 1.)
        } else {
            0.
        };
        self.contains([a[0] + dx * t, 0., a[2] + dz * t])
    }
    /// Whether a route along `points` from a start outside the zone enters it.
    pub(crate) fn entered_by(&self, points: &[[f64; 3]]) -> bool {
        points.first().is_some_and(|p| !self.contains(*p))
            && points.windows(2).any(|w| self.crossed_by(w[0], w[1]))
    }
}

/// Zones keyed by objective.
pub(crate) type Zones = BTreeMap<u16, Zone>;

/// Duty slots given to evacuating bots, apart from the armed duty book's.
const EVACUATION_SLOT_BASE: u8 = 64;

impl<W: Write> Engine<W> {
    fn bot_blast_policy(&self) -> Option<Policy> {
        self.bot_registry
            .config
            .as_ref()
            .map(|c| c.blast.clone())
            .filter(|p| p.enabled)
    }

    /// The authored outer blast radius of a structural objective, from the
    /// mode runtime that owns its explosion.
    fn structural_blast_radius(&self, index: u16) -> Option<f64> {
        let properties = self
            .domination_gameplay
            .as_ref()
            .and_then(|world| world.state.structural.blast_properties(index).ok())
            .or_else(|| {
                self.acquisition_gameplay
                    .as_ref()
                    .and_then(|runtime| runtime.structural_blast_properties(index))
            })?;
        let radius = properties
            .full_damage_radius
            .max(properties.half_damage_radius);
        (radius.is_finite() && radius > 0.).then_some(f64::from(radius))
    }

    /// The hot zones of every armed demolition objective with an authored blast.
    pub(crate) fn armed_blast_zones(&self) -> Zones {
        use replication::ObjectiveState::{Activated, Deactivating};
        let mut zones = Zones::new();
        let Some(policy) = self.bot_blast_policy() else {
            return zones;
        };
        let (Some(catalog), Some(world)) = (&self.objective_catalog, &self.objective_world) else {
            return zones;
        };
        if !self.match_started || self.match_terminal.is_some() {
            return zones;
        }
        for object in &catalog.objects {
            let Some(rule) = &object.demolition else {
                continue;
            };
            if !matches!(
                world.state(object.object_index),
                Some(Activated | Deactivating)
            ) {
                continue;
            }
            let Some(blast) = self.structural_blast_radius(object.object_index) else {
                continue;
            };
            let center = rule.position.map(f64::from);
            let reach = rule
                .trigger_volumes
                .iter()
                .map(|v| {
                    (f64::from(v.centre[0]) - center[0]).hypot(f64::from(v.centre[2]) - center[2])
                })
                .fold(0., f64::max);
            zones.insert(
                object.object_index,
                Zone {
                    objective: object.object_index,
                    center,
                    radius: reach + blast + policy.margin_m,
                    attackers: 1 - rule.owner_team.min(1),
                },
            );
        }
        zones
    }

    /// Attackers inside a zone without a duty on it get one there whose
    /// ring lies outside, so they leave before the fuse ends.
    pub(crate) fn plan_blast_evacuations(
        &self,
        zones: &Zones,
        duties: &mut BTreeMap<u16, crate::bot_armed_objectives::Assignment>,
        now: Duration,
        receipts: &mut Vec<Value>,
    ) {
        for (&id, bot) in &self.bot_registry.bots {
            let Some(r) = bot.target.runtime.filter(|_| bot.health > 0) else {
                continue;
            };
            let team = bot.target.squad.team();
            let here = point(r.position);
            let Some(zone) = zones
                .values()
                .find(|z| z.attackers == team && z.contains(here))
            else {
                continue;
            };
            if duties
                .get(&id)
                .is_some_and(|d| d.objective == zone.objective)
            {
                continue;
            }
            duties.insert(
                id,
                crate::bot_armed_objectives::Assignment {
                    objective: zone.objective,
                    duty: crate::bot_armed_objectives::Duty::Guard,
                    slot: EVACUATION_SLOT_BASE + (id % 64) as u8,
                },
            );
            receipts.push(
                json!({"op":"bot_blast_evacuation","timestamp_ms":now.as_millis() as u64,
                "object_index":id,"generation":r.generation,"objective":zone.objective,
                "radius_m":zone.radius,"position":r.position}),
            );
        }
    }

    /// The armed policy for one objective's duties: the attackers' guard ring
    /// sits outside the objective's hot zone.
    pub(crate) fn armed_policy_outside_blast(
        zones: &Zones,
        objective: u16,
        policy: &crate::bot_armed_objectives::ArmedPolicy,
    ) -> crate::bot_armed_objectives::ArmedPolicy {
        let mut policy = policy.clone();
        if let Some(zone) = zones.get(&objective) {
            let [inner, outer] = policy.guard_radius_m;
            let pushed = inner.max(zone.radius + 0.5);
            policy.guard_radius_m = [pushed, outer.max(pushed + (outer - inner))];
        }
        policy
    }
}

fn point(p: replication::CharacterPosition) -> [f64; 3] {
    [p.x.into(), p.y.into(), p.z.into()]
}

#[cfg(test)]
#[path = "bot_blast_tests.rs"]
mod tests;
