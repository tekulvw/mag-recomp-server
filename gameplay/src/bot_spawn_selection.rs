//! Bot lives follow their squad's currently active spawn set.
//!
//! Objective progression (Sabotage forward insertions), Domination spawn
//! effects and Acquisition spawn changes all move a squad's runtime spawn
//! choices in the world's spawn catalog. Humans respawn at their squad's
//! current point; this module applies the same rules to population bots.
//!
//! Policy, evaluated whenever a bot life starts (population allocation and
//! every respawn wave), deterministic for identical world state:
//!
//! 1. The squad's current spawn (a human member's shared squad spawn, else the
//!    runtime default) once it differs from the squad's authored default.
//!    While it is still the authored default the bot keeps its population
//!    source spawn, so match starts are unchanged.
//! 2. Other points the squad gained at runtime, most recently added first.
//! 3. The bot's population source spawn.
//! 4. The squad's remaining authored points in authored order.
//!
//! Only points the squad may use now, that are active and whose arrival a
//! server-simulated body can take (insertion, alternate, on-foot, parachute)
//! are considered; seat, vehicle, helicopter and unsupported points are not.
//! The source spawn always uses the bot's own population slot, waiting while a
//! body occupies it exactly as before. Any other point uses a cached list of
//! grounded, collision-covered placements whose navmesh component a complete
//! route from the bot's population slot reaches; the first placement no
//! body occupies wins, else the next point is tried. A point without such a
//! placement is skipped and reported once with its reason. When nothing else
//! qualifies the population slot is the fallback.
//!
//! Parachute points: the server has no airborne descent model for bot bodies.
//! Bots arrive landed on the first supporting surface below each authored drop
//! line position (the same line human parachutists are spaced along), then on
//! a small ring around the first landing.
use super::*;

const RING_GRID_M: f64 = 1.0;
const RING_RADIUS_STEPS: i32 = 3;
const PARACHUTE_RING_RADIUS_STEPS: i32 = 2;
const PARACHUTE_GROUND_SEARCH_M: f64 = 400.0;
const AUTHORED_GROUND_SEARCH_M: f64 = 3.0;
const ROUTE_ATTEMPTS: usize = 4;
const MIN_SEPARATION_M: f64 = 0.71;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Arrival {
    PopulationSlot,
    AuthoredPoint,
    ParachuteLanding,
}
impl Arrival {
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::PopulationSlot => "population_slot",
            Self::AuthoredPoint => "authored_point",
            Self::ParachuteLanding => "parachute_landing",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct LifeSpawn {
    pub position: replication::CharacterPosition,
    /// The point used; `None` for a population slot without a source binding.
    pub spawn_point: Option<u16>,
    pub arrival: Arrival,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum LifeSpawnChoice {
    Spawn(LifeSpawn),
    /// The population slot is the choice and a body occupies it.
    Wait,
}

#[derive(Clone, Debug)]
pub(crate) struct QualifiedPoint {
    positions: Vec<replication::CharacterPosition>,
    arrival: Arrival,
}

fn point(p: replication::CharacterPosition) -> [f64; 3] {
    [f64::from(p.x), f64::from(p.y), f64::from(p.z)]
}

fn ring(radius_steps: i32) -> impl Iterator<Item = [f64; 2]> {
    (1..=radius_steps).flat_map(|radius| {
        (-radius..=radius).flat_map(move |x| {
            (-radius..=radius)
                .filter(move |z| x.abs().max(z.abs()) == radius)
                .map(move |z| [f64::from(x) * RING_GRID_M, f64::from(z) * RING_GRID_M])
        })
    })
}

impl<W: Write> Engine<W> {
    /// Fixed-region populations keep their bound slots; every other
    /// population with spawn-bound slots follows the active spawn set.
    fn bot_population_follows_active_spawns(&self) -> bool {
        self.bot_registry
            .config
            .as_ref()
            .is_some_and(|c| c.regional_team.is_none() && c.regional_collision.is_none())
    }

    /// Ordered usable points for one bot life (policy steps 1-4).
    pub(crate) fn bot_spawn_order(&self, squad: u16, home: u16) -> Vec<u16> {
        let Some(catalog) = &self.spawn_catalog else {
            return Vec::new();
        };
        let authored_default = catalog
            .selected_squad(squad)
            .map(|binding| catalog.locally_selectable(squad, binding.default_spawn));
        let preferred = self
            .squad_spawn_for(squad)
            .map(|current| catalog.locally_selectable(squad, current))
            .filter(|&current| Some(current) != authored_default)
            .unwrap_or(home);
        let mut order = vec![preferred];
        order.extend(catalog.runtime_added_points(squad));
        order.push(home);
        if let Some(binding) = catalog.selected_squad(squad) {
            order.extend(binding.available.iter().copied());
        }
        let mut seen = std::collections::BTreeSet::new();
        order.retain(|&index| seen.insert(index));
        order.retain(|&index| catalog.bot_arrival_point(squad, index).is_some());
        order
    }

    /// Where a bot life in `squad`, bound to population `slot`, starts now.
    /// `actor` is the bot's own object index (excluded from occupancy).
    pub(crate) fn choose_bot_life_spawn(
        &mut self,
        actor: u16,
        squad: u16,
        slot: usize,
        actors: &crate::actor_collision::ActorGrid,
        receipts: &mut Vec<Value>,
    ) -> Result<LifeSpawnChoice, &'static str> {
        let bound = self
            .bot_registry
            .config
            .as_ref()
            .ok_or("bot_population_unavailable")?
            .slots
            .get(slot)
            .cloned()
            .ok_or("bot_population_slot")?;
        let body = bot_physics::Body::default();
        let slot_choice = |actors: &crate::actor_collision::ActorGrid| {
            Ok::<_, &'static str>(if actors.occupied(actor, point(bound.position), body)? {
                LifeSpawnChoice::Wait
            } else {
                LifeSpawnChoice::Spawn(LifeSpawn {
                    position: bound.position,
                    spawn_point: bound.source_spawn,
                    arrival: Arrival::PopulationSlot,
                })
            })
        };
        let Some(home) = bound
            .source_spawn
            .filter(|_| self.bot_population_follows_active_spawns())
        else {
            return slot_choice(actors);
        };
        for index in self.bot_spawn_order(squad, home) {
            if index == home {
                return slot_choice(actors);
            }
            let Ok(qualified) = self.qualify_bot_spawn(index, home, bound.position, receipts)
            else {
                continue;
            };
            // Re-check the static world at use; a later blocker skips the spot.
            let terrain =
                bot_physics::WorldTerrain::new(self.collision_world.as_ref(), self.map_name())
                    .map_err(|e| e.code())?;
            let movement_body = self.bot_movement_body();
            for &position in &qualified.positions {
                if !actors.occupied(actor, point(position), body)?
                    && terrain.start_motion(point(position), movement_body).is_ok()
                {
                    return Ok(LifeSpawnChoice::Spawn(LifeSpawn {
                        position,
                        spawn_point: Some(index),
                        arrival: qualified.arrival,
                    }));
                }
            }
        }
        slot_choice(actors)
    }

    /// Cached per (point, source spawn); geometry and navmesh are static.
    fn qualify_bot_spawn(
        &mut self,
        index: u16,
        home: u16,
        home_position: replication::CharacterPosition,
        receipts: &mut Vec<Value>,
    ) -> Result<QualifiedPoint, &'static str> {
        if let Some(cached) = self.bot_registry.spawn_qualification.get(&(index, home)) {
            return cached.clone();
        }
        let result = self.qualify_bot_spawn_uncached(index, home_position);
        match &result {
            Ok(qualified) => receipts.push(json!({"op":"bot_spawn_point_qualified",
                "spawn_point_index":index,"source_spawn":home,
                "arrival":qualified.arrival.code(),"placements":qualified.positions.len()})),
            Err(reason) => receipts.push(json!({"op":"bot_spawn_point_unusable",
                "spawn_point_index":index,"source_spawn":home,"reason":reason})),
        }
        self.bot_registry
            .spawn_qualification
            .insert((index, home), result.clone());
        result
    }

    fn qualify_bot_spawn_uncached(
        &self,
        index: u16,
        home_position: replication::CharacterPosition,
    ) -> Result<QualifiedPoint, &'static str> {
        let location = *self
            .spawn_catalog
            .as_ref()
            .and_then(|catalog| catalog.point(index))
            .ok_or("unknown_spawn_point")?;
        let mesh = self
            .navmesh_world
            .clone()
            .ok_or("bot_navmesh_unavailable")?;
        let world = self.collision_world.as_ref();
        let map = self.map_name();
        let body = self.bot_movement_body();
        let parachute = matches!(location.kind, spawn_catalog::SpawnKind::Parachute);
        let (raws, down, ring_steps) = if parachute {
            (
                (0..self.match_profile().squad_capacity.min(8))
                    .map(|ordinal| location.parachute_position(ordinal))
                    .collect::<Result<Vec<_>, _>>()?,
                PARACHUTE_GROUND_SEARCH_M,
                PARACHUTE_RING_RADIUS_STEPS,
            )
        } else {
            (
                vec![location.position],
                AUTHORED_GROUND_SEARCH_M,
                RING_RADIUS_STEPS,
            )
        };
        let mut last = "bot_spawn_unsupported";
        let mut candidates = Vec::new();
        for raw in raws {
            match crate::bot_objectives::supported_interval(world, map, raw, 0.5, down, body) {
                Ok(p) => candidates.push(p),
                Err(reason) => last = reason,
            }
        }
        let anchor = *candidates.first().ok_or(last)?;
        for [dx, dz] in ring(ring_steps) {
            let raw = replication::CharacterPosition {
                x: (f64::from(anchor.x) + dx) as f32,
                y: anchor.y,
                z: (f64::from(anchor.z) + dz) as f32,
            };
            if let Ok(p) = crate::bot_objectives::supported_interval(
                world,
                map,
                raw,
                1.5,
                AUTHORED_GROUND_SEARCH_M,
                body,
            ) {
                candidates.push(p);
            }
        }
        // One complete route from the population slot proves the landing
        // area belongs to the walking network bots reach from their spawn.
        // (Spawns are often left through one-way drops, so the reverse
        // direction is not required.)
        let mut routed = None;
        let mut attempts = 0;
        last = "bot_spawn_navmesh_uncovered";
        for &candidate in &candidates {
            let Ok(component) = mesh.component_id(point(candidate)) else {
                continue;
            };
            if attempts == ROUTE_ATTEMPTS {
                break;
            }
            attempts += 1;
            let route = if let Some(rules) = &self.fall_damage_rules {
                mesh.route_with_drops(point(home_position), point(candidate), 0.5, rules, 400)
            } else if mesh.artifact().drop_links.is_empty() {
                mesh.route(point(home_position), point(candidate), 0.5)
            } else {
                return Err("bot_navmesh_fall_rules_required");
            };
            match route {
                Ok(route) if route.status == navmesh::Status::Complete => {
                    routed = Some(component);
                    break;
                }
                Ok(_) => last = "bot_spawn_navmesh_disconnected",
                Err(error) => last = error.0,
            }
        }
        let component = routed.ok_or(last)?;
        let mut positions: Vec<replication::CharacterPosition> = Vec::new();
        for candidate in candidates {
            if mesh.component_id(point(candidate)).ok() == Some(component)
                && positions.iter().all(|other| {
                    (other.x - candidate.x).hypot(other.z - candidate.z) >= MIN_SEPARATION_M as f32
                        || (other.y - candidate.y).abs() > 2.0
                })
            {
                positions.push(candidate);
            }
        }
        Ok(QualifiedPoint {
            positions,
            arrival: if parachute {
                Arrival::ParachuteLanding
            } else {
                Arrival::AuthoredPoint
            },
        })
    }
}

#[cfg(test)]
#[path = "bot_spawn_selection_tests.rs"]
mod tests;
