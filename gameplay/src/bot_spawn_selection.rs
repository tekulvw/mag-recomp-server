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
//! Defending squads (whose lane objective their own team owns) instead rank
//! every point they may use now, including an operational hospital of their
//! side and a natively routed field hospital, by navmesh route length to the
//! lane objective; points that do not route there are skipped, near-equal
//! points are spread per bot, and the population slot is the last fallback.
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
/// Defenders spread over points whose route to the lane objective is within
/// this distance of the shortest one.
const FORWARD_SPREAD_M: f64 = 20.0;
/// A lane route ends within this distance of the objective centre.
const LANE_GOAL_RADIUS_M: f64 = 5.0;
const HOSPITAL_RING_RADIUS_STEPS: i32 = 1;
/// Field hospitals have no exit data; bots stand within this many 1 m rings
/// of the authored point, on ground the navmesh routes to the lane from.
const FIELD_HOSPITAL_RING_RADIUS_STEPS: i32 = 6;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Arrival {
    PopulationSlot,
    AuthoredPoint,
    ParachuteLanding,
    /// Outside an operational Domination hospital, at its authored exits.
    HospitalExit,
    /// On foot beside a standing Acquisition field hospital the squad is
    /// natively routed to.
    FieldHospital,
}
impl Arrival {
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::PopulationSlot => "population_slot",
            Self::AuthoredPoint => "authored_point",
            Self::ParachuteLanding => "parachute_landing",
            Self::HospitalExit => "hospital_exit",
            Self::FieldHospital => "field_hospital",
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

/// A defender spawn point proven to route to the squad's lane objective.
#[derive(Clone, Debug)]
pub(crate) struct LaneSpawn {
    /// Placements in the routed navmesh component; empty for the bot's own
    /// population slot, which always arrives at its slot.
    qualified: QualifiedPoint,
    route_m: f64,
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

    /// The squad's lane objective and its centre when the squad's team owns
    /// it, i.e. the squad's bots defend it now. Before the objective world
    /// assigns lanes (a population placed before the match starts) the
    /// squad's authored initial assignment stands in.
    pub(crate) fn bot_defended_lane(&self, squad: u16) -> Option<(u16, [f64; 3])> {
        let team = replication::SquadAddress::from_index(squad).ok()?.team();
        let lane = self.bot_squad_lane(squad).or_else(|| {
            self.initial_objective_index(squad)
                .ok()
                .filter(|&index| index != u16::MAX)
        })?;
        let (center, owner) = self.bot_objective_center(lane)?;
        (owner == team).then(|| (lane, center.map(f64::from)))
    }

    /// An operational Domination hospital of the squad's own side that the
    /// squad may spawn at now (its point is linked and active).
    fn bot_hospital_point(&self, squad: u16, index: u16) -> bool {
        let Some(catalog) = &self.spawn_catalog else {
            return false;
        };
        let Ok(team) = replication::SquadAddress::from_index(squad).map(|s| s.team()) else {
            return false;
        };
        let faction = if team == 0 {
            self.match_profile().attacking_faction
        } else {
            self.match_profile().defending_faction
        };
        catalog
            .available_for_squad(squad)
            .is_some_and(|points| points.contains(&index))
            && catalog.point_active(index)
            && catalog
                .point(index)
                .is_some_and(|p| matches!(p.kind, spawn_catalog::SpawnKind::BunkerPassenger))
            && self
                .bunker_world
                .as_ref()
                .and_then(|world| world.domination_bunker_at_spawn(index))
                .is_some_and(|h| h.operational && h.team == team && h.faction == faction)
    }

    /// The shared squad spawn a human member of the squad leads with, once it
    /// differs from the squad's authored default.
    fn bot_led_squad_spawn(&self, squad: u16) -> Option<u16> {
        let catalog = self.spawn_catalog.as_ref()?;
        let current = self
            .peers
            .values()
            .chain(self.active.iter())
            .filter(|a| a.squad_catalog.squad_index() == squad)
            .min_by_key(|a| a.reservation.player)?
            .squad_spawn;
        let current = catalog.locally_selectable(squad, current);
        let authored = catalog
            .selected_squad(squad)
            .map(|binding| catalog.locally_selectable(squad, binding.default_spawn));
        (Some(current) != authored).then_some(current)
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
        if self.navmesh_world.is_some()
            && let Some(lane) = self.bot_defended_lane(squad)
        {
            for (index, positions, arrival) in
                self.bot_defender_spawn_order(actor, squad, home, bound.position, lane, receipts)
            {
                if index == home {
                    if let LifeSpawnChoice::Spawn(spawn) = slot_choice(actors)? {
                        return Ok(LifeSpawnChoice::Spawn(spawn));
                    }
                    continue;
                }
                if let Some(position) = self.free_bot_placement(actor, &positions, actors)? {
                    return Ok(LifeSpawnChoice::Spawn(LifeSpawn {
                        position,
                        spawn_point: Some(index),
                        arrival,
                    }));
                }
            }
            return slot_choice(actors);
        }
        for index in self.bot_spawn_order(squad, home) {
            if index == home {
                return slot_choice(actors);
            }
            let Ok(qualified) = self.qualify_bot_spawn(index, home, bound.position, receipts)
            else {
                continue;
            };
            if let Some(position) = self.free_bot_placement(actor, &qualified.positions, actors)? {
                return Ok(LifeSpawnChoice::Spawn(LifeSpawn {
                    position,
                    spawn_point: Some(index),
                    arrival: qualified.arrival,
                }));
            }
        }
        slot_choice(actors)
    }

    /// The first placement no body occupies and the static world still
    /// supports; a later blocker skips the spot.
    fn free_bot_placement(
        &self,
        actor: u16,
        positions: &[replication::CharacterPosition],
        actors: &crate::actor_collision::ActorGrid,
    ) -> Result<Option<replication::CharacterPosition>, &'static str> {
        if positions.is_empty() {
            return Ok(None);
        }
        let terrain =
            bot_physics::WorldTerrain::new(self.collision_world.as_ref(), self.map_name())
                .map_err(|e| e.code())?;
        let movement_body = self.bot_movement_body();
        for &position in positions {
            if !actors.occupied(actor, point(position), bot_physics::Body::default())?
                && terrain.start_motion(point(position), movement_body).is_ok()
            {
                return Ok(Some(position));
            }
        }
        Ok(None)
    }

    /// Defender policy: every point the squad may use now (including an
    /// operational hospital of its side) that routes to the lane objective,
    /// shortest route first. Points within `FORWARD_SPREAD_M` of the shortest
    /// are rotated per bot so a squad spreads over equally forward points. A
    /// human member's led squad spawn goes first when it routes. The bot's own
    /// population slot is always last as the fallback.
    fn bot_defender_spawn_order(
        &mut self,
        actor: u16,
        squad: u16,
        home: u16,
        home_position: replication::CharacterPosition,
        lane: (u16, [f64; 3]),
        receipts: &mut Vec<Value>,
    ) -> Vec<(u16, Vec<replication::CharacterPosition>, Arrival)> {
        let mut points = self.bot_spawn_order(squad, home);
        if let Some(available) = self
            .spawn_catalog
            .as_ref()
            .and_then(|c| c.available_for_squad(squad))
            .map(<[u16]>::to_vec)
        {
            for index in available {
                if !points.contains(&index) && self.bot_hospital_point(squad, index) {
                    points.push(index);
                }
            }
        }
        if let Some(index) = self
            .spawn_catalog
            .as_ref()
            .and_then(|c| c.native_unselectable_default(squad))
            && !points.contains(&index)
        {
            points.push(index);
        }
        let mut ranked = Vec::new();
        for index in points {
            if let Ok(spawn) = self.lane_spawn(index, home, home_position, lane, receipts) {
                ranked.push((spawn.route_m, index, spawn.qualified));
            }
        }
        ranked.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        if let Some(&(best, _, _)) = ranked.first() {
            let near = ranked
                .iter()
                .take_while(|r| r.0 <= best + FORWARD_SPREAD_M)
                .count();
            ranked[..near].rotate_left(usize::from(actor) % near);
        }
        if let Some(led) = self.bot_led_squad_spawn(squad)
            && let Some(at) = ranked.iter().position(|r| r.1 == led)
        {
            let led = ranked.remove(at);
            ranked.insert(0, led);
        }
        let mut order: Vec<_> = ranked
            .into_iter()
            .map(|(_, index, qualified)| (index, qualified.positions, qualified.arrival))
            .collect();
        if !order.iter().any(|entry| entry.0 == home) {
            order.push((home, Vec::new(), Arrival::PopulationSlot));
        }
        order
    }

    /// Cached per (point, lane objective, population slot); geometry and
    /// navmesh are static.
    fn lane_spawn(
        &mut self,
        index: u16,
        home: u16,
        home_position: replication::CharacterPosition,
        (objective, goal): (u16, [f64; 3]),
        receipts: &mut Vec<Value>,
    ) -> Result<LaneSpawn, &'static str> {
        let slot = index == home;
        let key = (index, objective, slot);
        if let Some(cached) = self.bot_registry.lane_spawns.get(&key) {
            return cached.clone();
        }
        let result = self.lane_spawn_uncached(index, slot.then_some(home_position), goal);
        match &result {
            Ok(spawn) => receipts.push(json!({"op":"bot_lane_spawn_qualified",
                "spawn_point_index":index,"objective":objective,"population_slot":slot,
                "arrival":spawn.qualified.arrival.code(),
                "placements":spawn.qualified.positions.len(),"route_m":spawn.route_m})),
            Err(reason) => receipts.push(json!({"op":"bot_lane_spawn_unusable",
                "spawn_point_index":index,"objective":objective,"population_slot":slot,
                "reason":reason})),
        }
        self.bot_registry.lane_spawns.insert(key, result.clone());
        result
    }

    fn lane_spawn_uncached(
        &self,
        index: u16,
        slot: Option<replication::CharacterPosition>,
        goal: [f64; 3],
    ) -> Result<LaneSpawn, &'static str> {
        let mesh = self
            .navmesh_world
            .clone()
            .ok_or("bot_navmesh_unavailable")?;
        let (candidates, arrival) = match slot {
            Some(position) => (vec![position], Arrival::PopulationSlot),
            None => self.bot_spawn_candidates(index)?,
        };
        // A complete route from a placement to the objective proves bots can
        // leave the point and reach their lane from it.
        let mut routed = None;
        let mut attempts = 0;
        let mut last = "bot_spawn_navmesh_uncovered";
        for &candidate in &candidates {
            let Ok(component) = mesh.component_id(point(candidate)) else {
                continue;
            };
            if attempts == ROUTE_ATTEMPTS {
                break;
            }
            attempts += 1;
            match self.bot_route(&mesh, point(candidate), goal, LANE_GOAL_RADIUS_M) {
                Ok(route) if route.status == navmesh::Status::Complete => {
                    routed = Some((component, route.length_m));
                    break;
                }
                Ok(_) => last = "bot_lane_route_incomplete",
                Err(reason) => last = reason,
            }
        }
        let (component, route_m) = routed.ok_or(last)?;
        let positions = if slot.is_some() {
            Vec::new()
        } else {
            separated(&mesh, candidates, component)
        };
        Ok(LaneSpawn {
            qualified: QualifiedPoint { positions, arrival },
            route_m,
        })
    }

    fn bot_route(
        &self,
        mesh: &navmesh::NavMesh,
        from: [f64; 3],
        to: [f64; 3],
        radius: f64,
    ) -> Result<navmesh::Route, &'static str> {
        if let Some(rules) = &self.fall_damage_rules {
            mesh.route_with_drops(from, to, radius, rules, 400)
                .map_err(|e| e.0)
        } else if mesh.artifact().drop_links.is_empty() {
            mesh.route(from, to, radius).map_err(|e| e.0)
        } else {
            Err("bot_navmesh_fall_rules_required")
        }
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

    /// Grounded, collision-covered placements for a point before routing:
    /// the authored position and a ring around it, a parachute point's
    /// landings, or a hospital's authored exits and the ring cells further
    /// from the hospital than each exit.
    fn bot_spawn_candidates(
        &self,
        index: u16,
    ) -> Result<(Vec<replication::CharacterPosition>, Arrival), &'static str> {
        let location = *self
            .spawn_catalog
            .as_ref()
            .and_then(|catalog| catalog.point(index))
            .ok_or("unknown_spawn_point")?;
        let world = self.collision_world.as_ref();
        let map = self.map_name();
        let body = self.bot_movement_body();
        let ground = |raw, up, down| {
            crate::bot_objectives::supported_interval(world, map, raw, up, down, body)
        };
        let offset = |anchor: replication::CharacterPosition, [dx, dz]: [f64; 2]| {
            replication::CharacterPosition {
                x: (f64::from(anchor.x) + dx) as f32,
                y: anchor.y,
                z: (f64::from(anchor.z) + dz) as f32,
            }
        };
        let mut last = "bot_spawn_unsupported";
        let mut candidates = Vec::new();
        match location.kind {
            spawn_catalog::SpawnKind::BunkerPassenger => {
                let hospital = self
                    .bunker_world
                    .as_ref()
                    .and_then(|world| world.domination_bunker_at_spawn(index))
                    .ok_or("bot_spawn_hospital_unbound")?;
                let origin = hospital.position;
                let away = |p: replication::CharacterPosition| {
                    f64::from(p.x - origin.x).hypot(f64::from(p.z - origin.z))
                };
                let exits: Vec<_> = hospital.exit_policy.candidate_positions().collect();
                for raw in exits {
                    match ground(raw, 0.5, AUTHORED_GROUND_SEARCH_M) {
                        Ok(p) => candidates.push(p),
                        Err(reason) => last = reason,
                    }
                }
                let exits = candidates.clone();
                for exit in exits {
                    for cell in ring(HOSPITAL_RING_RADIUS_STEPS) {
                        let raw = offset(exit, cell);
                        if away(raw) > away(exit)
                            && let Ok(p) = ground(raw, 1.5, AUTHORED_GROUND_SEARCH_M)
                        {
                            candidates.push(p);
                        }
                    }
                }
                if candidates.is_empty() {
                    return Err(last);
                }
                return Ok((candidates, Arrival::HospitalExit));
            }
            spawn_catalog::SpawnKind::Unsupported => {
                // Only reached for a natively routed field hospital (see
                // `native_unselectable_default`); its structure may cover the
                // authored point, so no grounded anchor is required.
                for cell in std::iter::once([0., 0.]).chain(ring(FIELD_HOSPITAL_RING_RADIUS_STEPS))
                {
                    if let Ok(p) = ground(
                        offset(location.position, cell),
                        1.5,
                        AUTHORED_GROUND_SEARCH_M,
                    ) {
                        candidates.push(p);
                    }
                }
                if candidates.is_empty() {
                    return Err(last);
                }
                return Ok((candidates, Arrival::FieldHospital));
            }
            spawn_catalog::SpawnKind::Parachute => {
                for ordinal in 0..self.match_profile().squad_capacity.min(8) {
                    match ground(
                        location.parachute_position(ordinal)?,
                        0.5,
                        PARACHUTE_GROUND_SEARCH_M,
                    ) {
                        Ok(p) => candidates.push(p),
                        Err(reason) => last = reason,
                    }
                }
            }
            _ => match ground(location.position, 0.5, AUTHORED_GROUND_SEARCH_M) {
                Ok(p) => candidates.push(p),
                Err(reason) => last = reason,
            },
        }
        let parachute = matches!(location.kind, spawn_catalog::SpawnKind::Parachute);
        let anchor = *candidates.first().ok_or(last)?;
        let steps = if parachute {
            PARACHUTE_RING_RADIUS_STEPS
        } else {
            RING_RADIUS_STEPS
        };
        for cell in ring(steps) {
            if let Ok(p) = ground(offset(anchor, cell), 1.5, AUTHORED_GROUND_SEARCH_M) {
                candidates.push(p);
            }
        }
        Ok((
            candidates,
            if parachute {
                Arrival::ParachuteLanding
            } else {
                Arrival::AuthoredPoint
            },
        ))
    }

    fn qualify_bot_spawn_uncached(
        &self,
        index: u16,
        home_position: replication::CharacterPosition,
    ) -> Result<QualifiedPoint, &'static str> {
        let mesh = self
            .navmesh_world
            .clone()
            .ok_or("bot_navmesh_unavailable")?;
        let (candidates, arrival) = self.bot_spawn_candidates(index)?;
        // One complete route from the population slot proves the landing
        // area belongs to the walking network bots reach from their spawn.
        // (Spawns are often left through one-way drops, so the reverse
        // direction is not required.)
        let mut routed = None;
        let mut attempts = 0;
        let mut last = "bot_spawn_navmesh_uncovered";
        for &candidate in &candidates {
            let Ok(component) = mesh.component_id(point(candidate)) else {
                continue;
            };
            if attempts == ROUTE_ATTEMPTS {
                break;
            }
            attempts += 1;
            match self.bot_route(&mesh, point(home_position), point(candidate), 0.5) {
                Ok(route) if route.status == navmesh::Status::Complete => {
                    routed = Some(component);
                    break;
                }
                Ok(_) => last = "bot_spawn_navmesh_disconnected",
                Err(reason) => last = reason,
            }
        }
        let component = routed.ok_or(last)?;
        Ok(QualifiedPoint {
            positions: separated(&mesh, candidates, component),
            arrival,
        })
    }
}

/// Candidates in one navmesh component, at least `MIN_SEPARATION_M` apart.
fn separated(
    mesh: &navmesh::NavMesh,
    candidates: Vec<replication::CharacterPosition>,
    component: i32,
) -> Vec<replication::CharacterPosition> {
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
    positions
}

#[cfg(test)]
#[path = "bot_spawn_selection_tests.rs"]
mod tests;
