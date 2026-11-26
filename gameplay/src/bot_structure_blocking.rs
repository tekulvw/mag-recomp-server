//! Bots do not plan routes through standing gates and barricades.
//!
//! The navmesh is baked from static world collision. The geometry a
//! demolition structure's sequences place (a gate's leaves, a barricade's
//! blocks) is not part of it, so a route can lead straight through a closed
//! gate. While such a structure stands (any state but destroyed or being
//! repaired from destroyed), its placed geometry blocks bot routes: a route
//! whose path would put a standing bot body into it is refused, and the bot
//! plans again later. Once the structure is destroyed the debris its sequence
//! places instead does not count, and the passage is open; a repair closes it
//! again.
//!
//! Only structures whose destruction changes nothing but their own geometry
//! count (the `other` class of `bot_demolition`); emplacements whose
//! sequences also place objective props at their triggers keep today's
//! behaviour. A route that starts inside the geometry may leave it.
//!
//! The behaviour is specified in docs/bot-demolition.md; this is an original
//! design for this server.
use super::*;

/// Sample spacing along a route.
const STEP_M: f64 = 0.5;
/// The standing body tested at each sample: above a step, up to head height.
const BODY_HALF: [f64; 3] = [0.3, 0.6, 0.3];
const BODY_LIFT_M: f64 = 1.05;

/// Population-file policy (`structure_blocking`).
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Policy {
    pub enabled: bool,
}
impl Default for Policy {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// A standing structure that blocks routes, and its world bounds.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Blocker {
    pub objective: u16,
    pub bounds: crate::actor_collision::Bounds,
}

/// Whether a route along `points` puts a standing body into geometry that
/// `hit(objective, centre, half)` reports, after first leaving any geometry
/// it starts in.
pub(crate) fn route_blocked(
    points: &[[f64; 3]],
    blockers: &[Blocker],
    mut hit: impl FnMut(u16, [f64; 3], [f64; 3]) -> Result<bool, &'static str>,
) -> Result<Option<u16>, &'static str> {
    let mut leaving = true;
    for leg in points.windows(2) {
        let (a, b) = (leg[0], leg[1]);
        let length = (0..3).map(|i| (b[i] - a[i]).powi(2)).sum::<f64>().sqrt();
        let steps = (length / STEP_M).ceil().max(1.) as usize;
        for step in 0..=steps {
            let t = step as f64 / steps as f64;
            let centre = [
                a[0] + (b[0] - a[0]) * t,
                a[1] + (b[1] - a[1]) * t + BODY_LIFT_M,
                a[2] + (b[2] - a[2]) * t,
            ];
            let mut inside = None;
            for blocker in blockers {
                if (0..3).any(|i| {
                    blocker.bounds.max[i] < centre[i] - BODY_HALF[i]
                        || blocker.bounds.min[i] > centre[i] + BODY_HALF[i]
                }) {
                    continue;
                }
                if hit(blocker.objective, centre, BODY_HALF)? {
                    inside = Some(blocker.objective);
                    break;
                }
            }
            match inside {
                Some(objective) if !leaving => return Ok(Some(objective)),
                Some(_) => {}
                None => leaving = false,
            }
        }
    }
    Ok(None)
}

impl<W: Write> Engine<W> {
    /// Standing structures whose placed geometry blocks bot routes.
    pub(crate) fn structure_blockers(&self) -> Vec<Blocker> {
        use replication::ObjectiveState::{Completed, Repairing};
        if !self
            .bot_registry
            .config
            .as_ref()
            .is_some_and(|c| c.structure_blocking.enabled)
        {
            return Vec::new();
        }
        let (Some(catalog), Some(world), Some(gameplay)) = (
            &self.objective_catalog,
            &self.objective_world,
            &self.domination_gameplay,
        ) else {
            return Vec::new();
        };
        let hints = gameplay.runtime.structure_hints();
        catalog
            .objects
            .iter()
            .filter(|o| o.demolition.is_some())
            .filter(|o| {
                hints
                    .get(&o.object_index)
                    .is_none_or(crate::bot_demolition::Hint::changes_only_geometry)
            })
            .filter(|o| {
                world
                    .state(o.object_index)
                    .is_some_and(|state| !matches!(state, Completed | Repairing))
            })
            .filter_map(|o| {
                Some(Blocker {
                    objective: o.object_index,
                    bounds: gameplay.state.collisions.objective_bounds(o.object_index)?,
                })
            })
            .collect()
    }
}

/// The standing structure a route along `points` would walk into, given the
/// current placed geometry.
pub(crate) fn route_structure_block(
    collisions: Option<&crate::domination_structural_world::SequenceCollisionState>,
    points: &[[f64; 3]],
    blockers: &[Blocker],
) -> Result<Option<u16>, &'static str> {
    let Some(collisions) = collisions else {
        return Ok(None);
    };
    let mut budget = 1 << 20;
    route_blocked(points, blockers, |objective, centre, half| {
        collisions.objective_box_hit(objective, centre, half, &mut budget)
    })
}

#[cfg(test)]
#[path = "bot_structure_blocking_tests.rs"]
mod tests;
