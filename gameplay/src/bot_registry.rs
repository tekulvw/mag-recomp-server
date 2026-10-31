//! Opt-in, world-owned population. Human transport slots and bot occupancy are separate.
use super::*;
pub use crate::bot_objectives::ObjectivePolicy;
use development_bot_movement::{DropAdmission, DropPassage, DropPhase};
use development_target::{BotPhase, BotRuntime, Target};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BotSlot {
    pub team: u8,
    #[serde(default)]
    pub squad: Option<u16>,
    #[serde(default)]
    pub source_spawn: Option<u16>,
    pub position: replication::CharacterPosition,
    #[serde(default)]
    pub goal: Option<replication::CharacterPosition>,
    #[serde(default)]
    pub sprint: bool,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BotPopulation {
    pub format: String,
    pub map_name: String,
    pub slots: Vec<BotSlot>,
    #[serde(default)]
    pub autonomous: bool,
    #[serde(default)]
    pub objective_policy: Option<ObjectivePolicy>,
    #[serde(default)]
    pub firearms: crate::bot_firearms::Policy,
    #[serde(default)]
    pub healing: crate::bot_healing::Policy,
}
impl BotPopulation {
    pub fn load(path: impl AsRef<std::path::Path>) -> Result<Self, &'static str> {
        use std::io::Read;
        let mut bytes = Vec::new();
        std::fs::File::open(path)
            .map_err(|_| "bot_population_unavailable")?
            .take(1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "bot_population_unavailable")?;
        if bytes.len() > 1024 * 1024 {
            return Err("bot_population_file_limit");
        }
        let config: Self = serde_json::from_slice(&bytes).map_err(|_| "invalid_bot_population")?;
        config.validate()?;
        Ok(config)
    }
    fn validate(&self) -> Result<(), &'static str> {
        if self.format != "mag_bot_population_v1"
            || self.map_name.is_empty()
            || self.slots.len() > 256
            || self.slots.is_empty()
        {
            return Err("invalid_bot_population");
        }
        self.firearms.validate()?;
        self.healing.validate()?;
        if let Some(policy) = &self.objective_policy {
            policy.validate()?;
        }
        for (i, slot) in self.slots.iter().enumerate() {
            if slot.squad.is_some() != slot.source_spawn.is_some() {
                return Err("bot_spawn_binding_incomplete");
            }
            if let Some(squad) = slot.squad
                && replication::SquadAddress::from_index(squad)?.team() != slot.team
            {
                return Err("bot_spawn_binding_team_mismatch");
            }
            if slot.team > 1 {
                return Err("invalid_bot_team");
            }
            slot.position.validate()?;
            if let Some(goal) = slot.goal {
                goal.validate()?;
            }
            for other in &self.slots[..i] {
                if distance(slot.position, other.position) < 0.71 {
                    return Err("bot_spawn_slots_overlap");
                }
            }
        }
        Ok(())
    }
}
#[derive(Default)]
pub(crate) struct BotRegistry {
    pub(super) config: Option<BotPopulation>,
    total: usize,
    pub(super) bots: BTreeMap<u16, WorldBot>,
    generations: BTreeMap<u16, u32>,
    known: BTreeMap<String, BTreeSet<u16>>,
    reservations: BTreeMap<u16, (crate::actor_collision::Bounds, Duration)>,
    last_wait_receipt: Option<Duration>,
    collision_evidence: Option<String>,
    pub(super) director: crate::bot_objectives::Director,
    drop_queues: BTreeMap<u32, PassageQueue>,
    pub(super) failed_passages: BTreeMap<u32, Duration>,
    pub(super) traffic_cursor: u16,
    pub(super) firearm_cursor: u16,
    pub(super) delay_book: crate::bot_traffic_costs::Book,
}
pub(super) struct WorldBot {
    name_index: u16,
    pub(super) target: Target,
    pub(super) slot: usize,
    pub(super) role_slot: u8,
    pub(super) traffic_yield: Option<crate::bot_traffic_flow::Yield>,
    pub(super) traffic_yield_cursor: usize,
    pub(super) traffic_progress: Option<crate::bot_traffic_flow::Progress>,
    pub(super) holding_access: Option<crate::bot_traffic_flow::HoldingAccess>,
    pub(super) delay_episode: Option<crate::bot_traffic_costs::Episode>,
    pub(super) reconsider: Option<crate::bot_traffic_costs::Reconsideration>,
    pub(super) reconsider_at: Duration,
    pub(super) reconsider_history: BTreeMap<u32, Duration>,
    pub(super) drop_waiting: Option<u32>,
    pub(super) drop_committed: bool,
    pub(super) drop_permit: Option<u32>,
    cleared_drop_routes: BTreeMap<u32, (u32, Duration)>,
    pub(super) drop_queue_passage: Option<DropPassage>,
    pub(super) drop_holding_goal: Option<replication::CharacterPosition>,
    pub(super) drop_holding_attempt: usize,
    pub(super) holding_retry_at: Duration,
    pub(super) drop_resume: bool,
    pub(super) drop_egress: bool,
    pub(super) drop_retreat: bool,
    pub(super) drop_retreat_preentry: bool,
    pub(super) drop_reroute: bool,
    pub(super) drop_resume_failure_at: Option<Duration>,
    pub(super) drop_queue_approach: Vec<[[f64; 3]; 2]>,
    pub(super) drop_approach_ready: bool,
    pub(super) drop_queue_since: Option<Duration>,
    pub(super) health: u16,
    pub(super) damage: replication::CharacterDamage,
    pub(super) yaw: u16,
    pub(super) firearm: crate::bot_firearms::State,
    pub(super) healing: crate::bot_healing::State,
    pub(super) move_attempted: bool,
    respawn_wait: Option<Duration>,
    last_move_receipt: Option<Duration>,
    omitted_healthy_steps: u32,
    last_move_since: Option<Duration>,
    last_navigation_status: Option<String>,
    pub(super) task: crate::bot_objectives::Task,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PassageLife {
    id: u16,
    generation: u32,
}
struct PassageOwner {
    life: PassageLife,
    passage: DropPassage,
    last_position: replication::CharacterPosition,
    progress_at: Duration,
    entered: bool,
    cert_started: bool,
}
#[derive(Default)]
struct PassageQueue {
    owner: Option<PassageOwner>,
    waiting: std::collections::VecDeque<PassageLife>,
    probe_cursor: usize,
}
fn intersects(a: crate::actor_collision::Bounds, b: crate::actor_collision::Bounds) -> bool {
    (0..3).all(|i| a.min[i] <= b.max[i] && a.max[i] >= b.min[i])
}
fn blocked_preentry_can_yield(
    runtime: BotRuntime,
    protected: crate::actor_collision::Bounds,
) -> bool {
    runtime.physics_motion.is_some_and(|m| {
        let body = crate::actor_collision::Bounds::body(m.position, Default::default());
        let horizontally_clear = body.max[0] < protected.min[0]
            || body.min[0] > protected.max[0]
            || body.max[2] < protected.min[2]
            || body.min[2] > protected.max[2];
        !runtime.navmesh_state.on_player
            && m.blocked
            && !m.grounded
            && m.vertical_velocity == 0.
            && m.slide_velocity == [0., 0.]
            && horizontally_clear
    })
}
#[derive(Clone, Copy, Debug)]
pub struct BotCombatSnapshot {
    pub object_index: u16,
    pub generation: u32,
    pub position: replication::CharacterPosition,
    pub health_quarter_points: u16,
    pub team: u8,
    pub yaw: Option<u16>,
}
fn point(p: replication::CharacterPosition) -> [f64; 3] {
    [p.x.into(), p.y.into(), p.z.into()]
}
fn position(p: [f64; 3]) -> replication::CharacterPosition {
    replication::CharacterPosition {
        x: p[0] as f32,
        y: p[1] as f32,
        z: p[2] as f32,
    }
}
fn distance(a: replication::CharacterPosition, b: replication::CharacterPosition) -> f64 {
    let a = point(a);
    let b = point(b);
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}
fn initial_damage() -> replication::CharacterDamage {
    replication::CharacterDamage {
        last_location_and_type: 0,
        health_quarter_points: 400,
        conditions: 0,
        last_attacker: u32::MAX,
        last_source_object: u32::MAX,
        last_user_data: 0,
    }
}
impl BotRegistry {
    pub(super) fn traffic_protected(&self, exclude: u16) -> Vec<crate::actor_collision::Bounds> {
        self.drop_queues
            .values()
            .filter_map(|q| q.owner.as_ref())
            .filter(|o| o.life.id != exclude)
            .map(|o| o.passage.protected_bounds)
            .collect()
    }
    pub(super) fn traffic_lanes(&self, exclude: u16) -> Vec<Vec<[[f64; 3]; 2]>> {
        self.traffic_lanes_inner(exclude, true)
    }
    pub(super) fn traffic_lanes_for_yield(&self, exclude: u16) -> Vec<Vec<[[f64; 3]; 2]>> {
        self.traffic_lanes_inner(exclude, false)
    }
    fn traffic_lanes_inner(&self, exclude: u16, own_yield: bool) -> Vec<Vec<[[f64; 3]; 2]>> {
        self.drop_queues
            .values()
            .filter_map(|q| {
                q.owner.as_ref().map(|o| o.life.id).or_else(|| {
                    q.waiting
                        .iter()
                        .find(|life| self.bots[&life.id].drop_approach_ready)
                        .map(|life| life.id)
                })
            })
            .filter(|id| *id != exclude)
            .filter_map(|id| self.bots.get(&id))
            .map(|b| b.drop_queue_approach.clone())
            .chain(
                self.bots
                    .iter()
                    .filter(|(id, _)| own_yield || **id != exclude)
                    .filter_map(|(_, b)| b.traffic_yield.as_ref())
                    .filter(|y| y.requester != exclude && !y.resume)
                    .map(|y| y.lane.clone()),
            )
            .chain(
                self.bots
                    .values()
                    .filter_map(|b| b.holding_access.as_ref())
                    .filter(|a| a.requester != exclude)
                    .map(|a| a.lane.clone()),
            )
            .collect()
    }
    pub(super) fn reserved_passages(&self, query: u16, buffer: f64) -> Vec<(u16, u32)> {
        let owners: BTreeMap<_, _> = self
            .drop_queues
            .iter()
            .filter_map(|(&passage, q)| q.owner.as_ref().map(|o| (o.life.id, passage)))
            .collect();
        self.bots
            .iter()
            .filter(|&(id, b)| *id != query && b.health > 0 && !b.drop_reroute)
            .filter_map(|(&id, b)| {
                owners
                    .get(&id)
                    .copied()
                    .or(b.drop_waiting)
                    .or_else(|| b.planned_drop(buffer).map(|p| p.id))
                    .map(|p| (id, p))
            })
            .collect()
    }
    pub(super) fn owner_remaining(
        &self,
        passage: u32,
        query: u16,
        now: Duration,
        service: f64,
        maximum: f64,
    ) -> Option<f64> {
        let owner = self.drop_queues.get(&passage)?.owner.as_ref()?;
        if owner.life.id == query {
            return None;
        }
        let b = self.bots.get(&owner.life.id)?;
        if b.health == 0 || b.target.runtime.unwrap().generation != owner.life.generation {
            return None;
        }
        let elapsed = b
            .delay_episode
            .as_ref()
            .and_then(|e| e.granted)
            .map_or(0., |at| now.saturating_sub(at).as_secs_f64());
        let stalled = now.saturating_sub(owner.progress_at).as_secs_f64();
        Some((service - elapsed).max(stalled).clamp(0., maximum))
    }
    pub(crate) fn members(&self, squad: u16) -> impl Iterator<Item = u16> + '_ {
        self.bots
            .iter()
            .filter(move |(_, b)| b.target.squad.squad_index() == squad)
            .map(|(&id, _)| id)
    }
    pub(crate) fn enabled(&self) -> bool {
        self.config.is_some()
    }
}
impl WorldBot {
    pub(super) fn bind_delay_episode(&mut self, now: Duration, buffer: f64) {
        self.delay_episode = self.target.next_registry_drop(buffer).map(|p| {
            crate::bot_traffic_costs::Episode::new(
                crate::bot_traffic_costs::Key {
                    generation: self.target.runtime.unwrap().generation,
                    objective: self.task.objective,
                    role_slot: self.role_slot,
                    passage: p.id,
                },
                now,
            )
        });
    }
    pub(super) fn idle_queued_waiter(&self, arrival: f64) -> bool {
        self.health > 0
            && self.drop_waiting.is_some()
            && self.drop_permit.is_none()
            && !self.drop_committed
            && self.target.runtime.is_some_and(|r| {
                matches!(r.phase, BotPhase::Complete)
                    && !r.navmesh_state.on_player
                    && r.physics_motion.is_some_and(|m| m.grounded)
                    && self
                        .drop_holding_goal
                        .is_some_and(|g| distance(g, r.position) <= arrival)
            })
    }
    pub(super) fn can_reconsider(&self, buffer: f64, arrival: f64) -> bool {
        let Some(r) = self.target.runtime else {
            return false;
        };
        let Some(p) = self.planned_drop(buffer).or(self.drop_queue_passage) else {
            return false;
        };
        self.health > 0
            && self.drop_permit.is_none()
            && !self.drop_committed
            && self.traffic_yield.is_none()
            && !self.drop_resume
            && !self.drop_reroute
            && !self.drop_retreat
            && !self.drop_egress
            && !self.drop_retreat_preentry
            && !self.task.recovering
            && self.task.role_goal.is_some()
            && !r.navmesh_state.on_player
            && !r.navmesh_state.navigation_aborted
            && r.physics_motion.is_some_and(|m| m.grounded)
            && p.phase == DropPhase::Approaching
            && !intersects(
                crate::actor_collision::Bounds::body(r.physical_position(), Default::default()),
                p.protected_bounds,
            )
            && self.drop_holding_goal.is_none_or(|g| {
                self.drop_approach_ready
                    && matches!(r.phase, BotPhase::Complete)
                    && distance(g, r.position) <= arrival
            })
    }
    pub(super) fn planned_drop(&self, entry_buffer_m: f64) -> Option<DropPassage> {
        let runtime = self.target.runtime?;
        let route = self.target.registry_route()?;
        route
            .waypoint_kinds
            .iter()
            .enumerate()
            .filter_map(|(i, kind)| (*kind == navmesh::WaypointKind::DropStart).then_some(i))
            .find_map(|i| {
                let id = route.waypoint_drop_ids[i]?;
                if self.cleared_drop_routes.get(&id) == Some(&(runtime.generation, runtime.since)) {
                    return None;
                }
                self.target
                    .registry_drop_status(id, entry_buffer_m)
                    .filter(|p| p.phase != DropPhase::Cleared)
            })
    }
    fn snapshot(
        &self,
        now: Duration,
        profile: wire_profile::WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        let runtime = self.target.runtime.ok_or("bot_runtime_missing")?;
        let physical = replication::CharacterRespawn {
            sequence: 1,
            timestamp_ms: now.as_millis() as u32,
            object_index: self.target.object_index,
            state: replication::CharacterState {
                state: if self.health == 0 { 7 } else { 1 },
                stance: 0,
                user_data: [0; 3],
            },
            position: runtime.position,
            pitch: self.firearm.packed_pitch(),
            yaw: self.yaw,
            damage: self.damage,
        };
        let mut loadout = self.target.loadout.clone();
        loadout.primary = self.firearm.item(loadout.primary);
        dummy::character_snapshot(
            self.target.object_index,
            1,
            now.as_millis() as u32,
            &loadout,
            self.target.faction,
            profile,
            self.target
                .display_name
                .as_deref()
                .ok_or("bot_name_index")?,
            self.target.squad,
            Some(&physical),
        )
    }
}
impl<W: Write> Engine<W> {
    pub fn with_bot_population(
        mut self,
        mut config: BotPopulation,
        total: usize,
    ) -> Result<Self, &'static str> {
        if config.objective_policy.is_none() {
            config.objective_policy = Some(ObjectivePolicy::bundled());
        }
        config.validate()?;
        if self.development_dummy_enabled() {
            return Err("bot_population_conflicts_with_legacy_dummy");
        }
        if total > self.bot_names.len() {
            return Err("bot_names_insufficient_capacity");
        }
        if !(2..=256).contains(&total)
            || total > self.admission_capacity_for(20231)
            || total > self.match_profile().team_capacity * 2
        {
            return Err("bot_population_capacity");
        }
        if config.map_name != self.map_name() {
            return Err("bot_population_map_profile_mismatch");
        }
        if config.autonomous
            && total > 32
            && config
                .slots
                .iter()
                .any(|s| s.squad.is_none() || s.source_spawn.is_none())
        {
            return Err("bot_population_full_match_spawn_binding_required");
        }
        for team in 0..2 {
            if config.slots.iter().filter(|s| s.team == team).count() < total.div_ceil(2) {
                return Err("bot_population_insufficient_slots");
            }
        }
        if let Some(mesh) = &self.navmesh_world
            && (config.autonomous || config.slots.iter().any(|s| s.goal.is_some()))
            && mesh
                .artifact()
                .drop_links
                .iter()
                .any(|link| (link.speed - retail_movement::WALK_FORWARD_MPS).abs() > 1e-9)
        {
            return Err("bot_navmesh_drop_speed_profile");
        }
        for faction in [
            self.match_profile().attacking_faction,
            self.match_profile().defending_faction,
        ] {
            let index = self
                .item_catalog
                .bootstrap_default_inventory_index(faction)
                .ok_or("missing_retail_target_loadout")?;
            self.item_catalog
                .bootstrap_loadout(faction, index)
                .ok_or("missing_retail_target_loadout")?;
        }
        let terrain =
            bot_physics::WorldTerrain::new(self.collision_world.as_ref(), self.map_name())
                .map_err(|e| e.code())?;
        let body = bot_physics::Body::default();
        if config.slots.iter().any(|s| s.squad.is_some())
            && config.slots.iter().any(|s| s.squad.is_none())
        {
            return Err("bot_population_mixed_spawn_binding");
        }
        if config.slots.iter().all(|s| s.squad.is_some())
            && total == self.match_profile().team_capacity * 2
        {
            for squad in &self
                .spawn_catalog
                .as_ref()
                .ok_or("spawn_catalog_unavailable")?
                .map_pool()
                .squads
            {
                if config
                    .slots
                    .iter()
                    .filter(|s| s.squad == Some(squad.index))
                    .count()
                    < self.match_profile().squad_capacity
                {
                    return Err("bot_population_insufficient_squad_slots");
                }
            }
        }
        for slot in &mut config.slots {
            if let (Some(squad), Some(source)) = (slot.squad, slot.source_spawn) {
                let catalog = self
                    .spawn_catalog
                    .as_ref()
                    .ok_or("spawn_catalog_unavailable")?;
                if !catalog
                    .available_for_squad(squad)
                    .is_some_and(|available| available.contains(&source))
                {
                    return Err("bot_spawn_binding_not_authored_for_squad");
                }
                let anchor = catalog
                    .point(source)
                    .filter(|s| {
                        s.active
                            && !matches!(
                                s.kind,
                                spawn_catalog::SpawnKind::Unsupported
                                    | spawn_catalog::SpawnKind::BunkerPassenger
                            )
                    })
                    .ok_or("bot_spawn_binding_inactive")?;
                if (slot.position.x - anchor.position.x).hypot(slot.position.z - anchor.position.z)
                    > 8.
                {
                    return Err("bot_spawn_binding_offset_limit");
                }
            }
            terrain
                .require_horizontal_coverage(point(slot.position), point(slot.position), body)
                .map_err(|e| e.code())?;
            let motion = terrain
                .start_motion(point(slot.position), body)
                .map_err(|e| e.code())?;
            let motion = terrain
                .advance(motion, [0.; 3], 1., body)
                .map_err(|e| e.code())?;
            if !motion.grounded || motion.blocked {
                return Err("bot_population_unsupported_spawn");
            }
            slot.position = position(motion.position);
            if slot.goal.is_some() || config.autonomous {
                self.navmesh_world
                    .as_ref()
                    .ok_or("bot_navmesh_unavailable")?
                    .validate_coverage(
                        self.collision_world
                            .as_ref()
                            .ok_or("bot_navmesh_geometry_unavailable")?,
                    )
                    .map_err(|e| e.0)?;
            }
        }
        config.validate()?;
        self.bot_registry = BotRegistry {
            config: Some(config),
            total,
            ..Default::default()
        };
        Ok(self)
    }
    pub(crate) fn registry_bot_loadout(&self, id: u16) -> Option<&replication::CharacterLoadout> {
        self.bot_registry.bots.get(&id).map(|b| &b.target.loadout)
    }
    pub(crate) fn is_registry_bot(&self, id: u16) -> bool {
        self.bot_registry.bots.contains_key(&id)
    }
    /// Current authoritative human tracking and living bots only. Recent swept
    /// bot bounds reserve published paths, so later bots cannot cross them.
    pub(super) fn actor_grid(
        &self,
        now: Duration,
    ) -> Result<crate::actor_collision::ActorGrid, &'static str> {
        use crate::actor_collision::{ActorGrid, Bounds};
        let mut grid = ActorGrid::default();
        let body = bot_physics::Body::default();
        for a in self.active.iter().chain(self.peers.values()).filter(|a| {
            a.ready_for_updates
                && a.entered_game
                && a.initial_objects.is_some()
                && a.spawn_delta.is_some()
                && a.life.is_alive()
                && a.mounted_seat.is_none()
                && now.saturating_sub(a.last_event) < IDLE_TIMEOUT
        }) {
            if let Some(sample) = a.tracking.sample {
                grid.insert(
                    a.reservation.player,
                    Bounds::body(point(sample.position), body),
                )?;
            }
        }
        for bot in self
            .bot_combat_snapshots()
            .into_iter()
            .filter(|b| b.health_quarter_points > 0)
        {
            let physical = self
                .bot_registry
                .bots
                .get(&bot.object_index)
                .and_then(|b| b.target.runtime)
                .or_else(|| {
                    self.active
                        .iter()
                        .chain(self.peers.values())
                        .filter_map(|a| a.development_target.as_ref())
                        .find(|t| t.object_index == bot.object_index)
                        .and_then(|t| t.runtime)
                })
                .map_or_else(|| point(bot.position), BotRuntime::physical_position);
            let mut bounds = Bounds::body(physical, body);
            if let Some((reserved, until)) = self.bot_registry.reservations.get(&bot.object_index)
                && *until > now
            {
                bounds = bounds.union(*reserved);
            }
            grid.insert_reserved(bot.object_index, bounds)?;
        }
        Ok(grid)
    }
    pub fn bot_combat_snapshots(&self) -> Vec<BotCombatSnapshot> {
        let mut result: Vec<_> = self
            .bot_registry
            .bots
            .iter()
            .map(|(&object_index, b)| {
                let r = b.target.runtime.unwrap();
                BotCombatSnapshot {
                    object_index,
                    generation: r.generation,
                    position: r.position,
                    health_quarter_points: b.health,
                    team: b.target.squad.team(),
                    yaw: Some(b.yaw),
                }
            })
            .collect();
        for a in self.active.iter().chain(self.peers.values()) {
            if a.dummy_created
                && let Some(t) = &a.development_target
                && let Some(r) = t.runtime
            {
                result.push(BotCombatSnapshot {
                    object_index: t.object_index,
                    generation: r.generation,
                    position: r.position,
                    health_quarter_points: a.dummy_health.unwrap_or(0),
                    team: t.squad.team(),
                    yaw: None,
                });
            }
        }
        result
    }
    /// External damage admission publishes once; planned events use commit_bot_damage
    /// after their audit record and provide their own already-staged publication.
    pub fn apply_bot_damage(
        &mut self,
        id: u16,
        generation: u32,
        damage: replication::CharacterDamage,
        now: Duration,
    ) -> Result<(), &'static str> {
        let packet = replication::CharacterDamageUpdate {
            sequence: 2,
            timestamp_ms: now.as_millis() as u32,
            object_index: id,
            damage,
            forced_state: (damage.health_quarter_points == 0).then_some(
                replication::CharacterState {
                    state: 7,
                    stance: 0,
                    user_data: [0; 3],
                },
            ),
        }
        .encode_with_profile(self.wire_profile)?;
        self.commit_bot_damage(id, generation, damage, now)?;
        self.broadcast_bot_packet(packet);
        Ok(())
    }
    pub(crate) fn commit_bot_damage(
        &mut self,
        id: u16,
        generation: u32,
        damage: replication::CharacterDamage,
        now: Duration,
    ) -> Result<(), &'static str> {
        if let Some(bot) = self.bot_registry.bots.get_mut(&id) {
            let mut runtime = bot.target.runtime.ok_or("bot_runtime_missing")?;
            if runtime.generation != generation
                || bot.health == 0
                || damage.health_quarter_points > bot.health
            {
                return Err("stale_bot_damage");
            }
            bot.health = damage.health_quarter_points;
            bot.damage = damage;
            if bot.health == 0 {
                bot.firearm = Default::default();
                bot.healing = Default::default();
                runtime.phase = BotPhase::Dead;
                runtime.since = now;
                runtime.last_update = now;
                runtime.physics_motion = None;
                bot.task = Default::default();
                bot.delay_episode = None;
                bot.reconsider = None;
                bot.reconsider_history.clear();
                bot.traffic_yield = None;
                bot.traffic_yield_cursor = 0;
                bot.traffic_progress = None;
                bot.holding_access = None;
                bot.drop_queue_passage = None;
                bot.drop_queue_since = None;
                bot.drop_holding_goal = None;
                bot.drop_resume = false;
                bot.drop_egress = false;
                bot.drop_retreat = false;
                bot.drop_retreat_preentry = false;
                bot.drop_reroute = false;
                bot.drop_resume_failure_at = None;
                bot.drop_queue_approach.clear();
                bot.holding_retry_at = Duration::ZERO;
                bot.drop_approach_ready = false;
                bot.drop_holding_attempt = 0;
                self.bot_registry.reservations.remove(&id);
                bot.target.pending_move = None;
            }
            bot.target.runtime = Some(runtime);
            return Ok(());
        }
        for a in self.active.iter_mut().chain(self.peers.values_mut()) {
            if let Some(t) = a.development_target.as_mut()
                && t.object_index == id
                && let Some(mut r) = t.runtime
            {
                if r.generation != generation
                    || !a
                        .dummy_health
                        .is_some_and(|hp| hp > 0 && damage.health_quarter_points <= hp)
                {
                    return Err("stale_bot_damage");
                }
                a.dummy_health = Some(damage.health_quarter_points);
                if damage.health_quarter_points == 0 {
                    r.phase = BotPhase::Dead;
                    r.since = now;
                    r.last_update = now;
                    r.physics_motion = None;
                    t.pending_move = None;
                }
                t.runtime = Some(r);
                return Ok(());
            }
        }
        Err("bot_not_found")
    }
    pub(super) fn broadcast_bot_packet(&mut self, packet: Vec<u8>) {
        for recipient in self
            .peers
            .values_mut()
            .chain(self.active.iter_mut())
            .filter(|a| a.initial_objects.is_some())
        {
            recipient.queue_shared(packet.clone());
        }
    }
    pub(super) fn retire_bot_for_human(
        &mut self,
        index: u16,
        now: Duration,
        wall: u64,
    ) -> Result<(), &'static str> {
        if let Some(bot) = self.bot_registry.bots.get(&index) {
            let r = bot.target.runtime.unwrap();
            self.release_retiring_bot_objectives(
                objective_control::ObjectiveActor {
                    player: index,
                    team: bot.target.squad.team(),
                    generation: r.generation,
                    position: r.position,
                },
                now,
                wall,
            )?;
        }
        if self.bot_registry.bots.remove(&index).is_some() {
            self.bot_registry.reservations.remove(&index);
            let packet = replication::multiplayer::CharacterRemoval {
                sequence: 1,
                timestamp_ms: now.as_millis() as u32,
                player: index,
            }
            .encode_with_profile(self.wire_profile)?;
            self.broadcast_bot_packet(packet);
            for known in self.bot_registry.known.values_mut() {
                known.remove(&index);
            }
            self.queue_roster(now)?;
        }
        Ok(())
    }
    pub(crate) fn tick_bots(
        &mut self,
        now: Duration,
        wall: u64,
    ) -> Result<Vec<OutboundMessage>, &'static str> {
        if self.match_terminal.is_some() {
            for bot in self.bot_registry.bots.values_mut() {
                bot.task.operator = false;
            }
            return Ok(Vec::new());
        }
        self.tick_legacy_bot(now, wall)?;
        if !self.bot_registry.enabled() || self.wire_profile.application_id() != 20231 {
            return Ok(Vec::new());
        }
        if !self
            .peers
            .values()
            .chain(self.active.iter())
            .any(|a| a.ready_for_updates && a.initial_objects.is_some())
        {
            return Ok(Vec::new());
        }
        // Start the first population only after a real human reaches on foot.
        // Thereafter its world-owned lives/routes continue through human death.
        if self.bot_registry.bots.is_empty()
            && !self.peers.values().chain(self.active.iter()).any(|a| {
                a.entered_game
                    && a.ready_for_updates
                    && a.life.is_alive()
                    && a.mounted_seat.is_none()
            })
        {
            return Ok(Vec::new());
        }
        if let Some(world) = &self.collision_world
            && self.bot_registry.collision_evidence.as_deref() != Some(world.source_sha256())
        {
            let fingerprint = world.source_sha256().to_owned();
            self.record(json!({"op":"bot_collision_world","collision_world_sha256":fingerprint,"collision_metadata":world.metadata(),"collision_stats":world.stats()}),wall)?;
            self.bot_registry.collision_evidence = Some(fingerprint);
        }
        self.bot_registry
            .reservations
            .retain(|_, (_, until)| *until > now);
        let mut actors = self.actor_grid(now)?;
        self.reconcile_bot_population(now, wall, &mut actors)?;
        // New recipients get one coherent creation+inventory+current-life snapshot per bot.
        let recipients: Vec<_> = self
            .peers
            .values()
            .chain(self.active.iter())
            .filter(|a| a.ready_for_updates && a.initial_objects.is_some())
            .map(|a| a.reservation.id.clone())
            .collect();
        self.bot_registry
            .known
            .retain(|id, _| recipients.contains(id));
        for id in recipients {
            let known = self.bot_registry.known.entry(id.clone()).or_default();
            let mut packets = Vec::new();
            for (&index, bot) in &self.bot_registry.bots {
                if !known.contains(&index) {
                    packets.push(bot.snapshot(now, self.wire_profile)?);
                    known.insert(index);
                }
            }
            let recipient = self
                .active
                .as_mut()
                .filter(|a| a.reservation.id == id)
                .or_else(|| self.peers.get_mut(&id))
                .unwrap();
            for packet in packets {
                recipient.queue_shared(packet);
            }
        }
        self.queue_roster(now)?;
        self.tick_drop_admissions(now, wall)?;
        self.tick_bot_objectives(now, wall, &actors)?;
        let mut packets = Vec::new();
        let mut receipts = Vec::new();
        let map = self.map_name().to_string();
        let respawn_delays = self.match_profile().spawn_wave_ms;
        for (&id, bot) in &mut self.bot_registry.bots {
            let runtime = bot.target.runtime.unwrap();
            if bot.health == 0 {
                let team = usize::from(bot.target.squad.team());
                // Match the world-owned team schedule. Compute
                // from this life's death time so a late tick or occupied spawn
                // cannot roll a due bot forward to another wave.
                let waiting = match self.team_waves[team] {
                    Some(clock) => now < clock.next_after(runtime.since)?,
                    None => {
                        now.saturating_sub(runtime.since)
                            < Duration::from_millis(respawn_delays[team].into())
                    }
                };
                if waiting {
                    continue;
                }
                if actors.occupied(id, point(bot.target.position), bot_physics::Body::default())? {
                    if bot
                        .respawn_wait
                        .is_none_or(|last| now.saturating_sub(last) >= Duration::from_secs(1))
                    {
                        receipts.push(json!({"op":"bot_respawn_waiting","object_index":id,"generation":runtime.generation,"reason":"player_body_occupied"}));
                        bot.respawn_wait = Some(now);
                    }
                    continue;
                }
                bot.respawn_wait = None;
                let mut runtime = BotRuntime::spawned(now, bot.target.position);
                runtime.generation = self.bot_registry.generations[&id]
                    .checked_add(1)
                    .ok_or("bot_generation_exhausted")?;
                self.bot_registry.generations.insert(id, runtime.generation);
                let terrain = bot_physics::WorldTerrain::new(self.collision_world.as_ref(), &map)
                    .map_err(|e| e.code())?;
                runtime.physics_motion = Some(
                    terrain
                        .start_motion(runtime.physical_position(), bot_physics::Body::default())
                        .map_err(|e| e.code())?,
                );
                runtime.phase = BotPhase::Complete;
                bot.target.runtime = Some(runtime);
                bot.health = 400;
                bot.firearm = Default::default();
                bot.healing = Default::default();
                bot.damage = initial_damage();
                bot.move_attempted = false;
                bot.task = Default::default();
                bot.last_move_receipt = None;
                bot.omitted_healthy_steps = 0;
                bot.last_move_since = None;
                bot.last_navigation_status = None;
                actors.insert_reserved(
                    id,
                    crate::actor_collision::Bounds::body(
                        bot.target.runtime.unwrap().physical_position(),
                        bot_physics::Body::default(),
                    ),
                )?;
                packets.push(dummy::respawn_for(
                    id,
                    2,
                    now.as_millis() as u32,
                    bot.target.position,
                    &bot.target.loadout,
                    self.wire_profile,
                )?);
                receipts.push(
                    json!({"op":"bot_respawn","object_index":id,"generation":runtime.generation}),
                );
            }
            if !self.bot_registry.config.as_ref().unwrap().autonomous && !bot.move_attempted {
                bot.move_attempted = true;
                let slot = &self.bot_registry.config.as_ref().unwrap().slots[bot.slot];
                if let Some(goal) = slot.goal {
                    let movement = BotMove {
                        mode: BotMoveMode::Navmesh,
                        destination: goal,
                        duration_ms: 120000,
                        update_ms: 250,
                        yaw: None,
                        sprint: slot.sprint,
                    };
                    let result = bot.target.begin_registry_move(
                        self.navmesh_world
                            .clone()
                            .ok_or("bot_navmesh_unavailable")?,
                        &map,
                        movement,
                        now,
                        bot.health,
                        self.fall_damage_rules.as_ref(),
                    );
                    if let Err(reason) = result {
                        receipts.push(json!({"op":"bot_move_stopped","object_index":id,"generation":bot.target.runtime.unwrap().generation,"reason":reason}));
                    }
                }
            }
            if matches!(bot.target.runtime.unwrap().phase, BotPhase::Relocating) {
                if let Some(mut step) = bot.target.movement_step_with_drop_admission(
                    1,
                    now,
                    self.wire_profile,
                    self.collision_world.as_ref(),
                    &map,
                    bot.health,
                    self.fall_damage_rules.as_ref(),
                    Some(crate::actor_collision::Query {
                        grid: &actors,
                        actor: id,
                    }),
                    if self.bot_registry.config.as_ref().unwrap().autonomous {
                        DropAdmission::Permit {
                            drop_id: bot.drop_permit,
                            entry_buffer_m: self
                                .bot_registry
                                .config
                                .as_ref()
                                .unwrap()
                                .objective_policy
                                .as_ref()
                                .unwrap()
                                .drop_entry_buffer_m,
                        }
                    } else {
                        DropAdmission::Unrestricted
                    },
                )? {
                    step.receipt["movement_purpose"] =
                        json!(match step.receipt["navigation"]["planner"].as_str() {
                            Some("physics_validated_aborted_drop_egress") => "aborted_drop_egress",
                            Some("physics_validated_aborted_drop_retreat") =>
                                "aborted_drop_retreat",
                            Some("physics_validated_precommit_passage_retreat") =>
                                "precommit_passage_retreat",
                            _ if bot.target.registry_ground_recovery() => "mesh_recovery",
                            _ if bot.traffic_yield.is_some() => "traffic_yield",
                            _ if bot.drop_holding_goal.is_some() => "drop_holding",
                            _ => "objective_or_fixed_goal",
                        });
                    if let Some(hp) = step.health {
                        bot.health = hp;
                        bot.damage.health_quarter_points = hp;
                    }
                    if let Some(yaw) = step.receipt["facing_yaw"].as_u64() {
                        bot.yaw = yaw as u16;
                    }
                    bot.target.runtime = Some(step.runtime);
                    if matches!(step.runtime.phase, BotPhase::Dead) {
                        bot.target.pending_move = None;
                    }
                    if bot.health > 0 {
                        let bounds = step.swept_bounds.unwrap_or_else(|| {
                            crate::actor_collision::Bounds::body(
                                step.runtime.physical_position(),
                                bot_physics::Body::default(),
                            )
                        });
                        self.bot_registry
                            .reservations
                            .insert(id, (bounds, now + Duration::from_millis(250)));
                        actors.insert_reserved(id, bounds)?;
                    } else {
                        actors.remove(id);
                        self.bot_registry.reservations.remove(&id);
                    }
                    packets.push(step.packet);
                    let status = step.receipt["navigation"]["status"].as_str();
                    let transition = status != bot.last_navigation_status.as_deref()
                        || bot.last_move_since != Some(runtime.since);
                    bot.last_navigation_status = status.map(str::to_owned);
                    bot.last_move_since = Some(runtime.since);
                    let has_events = step.receipt["ground_events"]
                        .as_array()
                        .is_some_and(|v| !v.is_empty())
                        || step.receipt["navigation"]["certified_drop_events"]
                            .as_array()
                            .is_some_and(|v| !v.is_empty())
                        || step.receipt["navigation"]["landing_hits"]
                            .as_array()
                            .is_some_and(|v| !v.is_empty());
                    let routine = step.receipt["phase"] == "move"
                        && step.health.is_none()
                        && !transition
                        && !has_events
                        && step.receipt["physics_error"].is_null()
                        && step.receipt["navigation_error"].is_null();
                    if routine
                        && bot
                            .last_move_receipt
                            .is_some_and(|last| now.saturating_sub(last) < Duration::from_secs(1))
                    {
                        bot.omitted_healthy_steps = bot.omitted_healthy_steps.saturating_add(1);
                    } else {
                        let mut receipt = step.receipt;
                        receipt["routine_audit"] = json!(routine);
                        receipt["healthy_steps_omitted"] = json!(bot.omitted_healthy_steps);
                        receipt["routine_audit_period_ms"] = json!(1000);
                        bot.omitted_healthy_steps = 0;
                        bot.last_move_receipt = Some(now);
                        receipts.push(receipt);
                    }
                }
            } else if bot.health > 0
                && let Some(step) = {
                    let result = bot.target.idle_registry_step(
                        now,
                        self.wire_profile,
                        self.collision_world.as_ref(),
                        &map,
                        bot.health,
                        self.fall_damage_rules.as_ref(),
                        crate::actor_collision::Query {
                            grid: &actors,
                            actor: id,
                        },
                    );
                    #[cfg(test)]
                    if let Ok(Some(step)) = &result
                        && let Some(after) = step.runtime.physics_motion
                        && !after.grounded
                        && after.blocked
                        && after.vertical_velocity == 0.
                        && after.slide_velocity == [0.; 2]
                        && bot
                            .target
                            .runtime
                            .unwrap()
                            .physics_motion
                            .is_some_and(|before| {
                                !before.grounded
                                    && before.blocked
                                    && before.vertical_velocity == 0.
                                    && before.slide_velocity == [0.; 2]
                                    && before.position == after.position
                            })
                        && std::env::var("MAG_BOT_IDLE_STALL_TRACE_IDS").is_ok_and(|ids| {
                            ids.split(',').any(|v| v.parse::<u16>().ok() == Some(id))
                        })
                        && let Ok(dir) = std::env::var("MAG_BOT_IDLE_STALL_TRACE_DIR")
                    {
                        let path = format!("{dir}/idle-stalled-{id}.json");
                        let runtime = bot.target.runtime.unwrap();
                        let entries:Vec<_>=actors.diagnostic_entries().into_iter().map(|(id,bounds,allow_overlap_escape)|json!({"id":id,"min":bounds.min,"max":bounds.max,"allow_overlap_escape":allow_overlap_escape})).collect();
                        let temp = format!("{path}.next");
                        std::fs::write(&temp,serde_json::to_vec_pretty(&json!({"actor":id,"timestamp_ms":now.as_millis()as u64,"last_update_ms":runtime.last_update.as_millis()as u64,"generation":runtime.generation,"motion":runtime.physics_motion,"after_motion":after,"on_player":runtime.navmesh_state.on_player,"actors":entries})).unwrap()).unwrap();
                        std::fs::rename(temp, path).unwrap();
                    }
                    #[cfg(test)]
                    if let Err(reason) = &result
                        && let Ok(dir) = std::env::var("MAG_BOT_REPLAY_LOG_DIR")
                    {
                        let runtime = bot.target.runtime.unwrap();
                        let entries:Vec<_>=actors.diagnostic_entries().into_iter().map(|(id,bounds,allow_overlap_escape)|json!({"id":id,"min":bounds.min,"max":bounds.max,"allow_overlap_escape":allow_overlap_escape})).collect();
                        let evidence = json!({"actor":id,"timestamp_ms":now.as_millis()as u64,"last_update_ms":runtime.last_update.as_millis()as u64,"motion":runtime.physics_motion,"error":reason,"actors":entries});
                        std::fs::write(
                            format!("{dir}/idle-failure-input.json"),
                            serde_json::to_vec_pretty(&evidence).unwrap(),
                        )
                        .unwrap();
                    }
                    result
                }?
            {
                bot.target.runtime = Some(step.runtime);
                if let Some(hp) = step.health {
                    bot.health = hp;
                    bot.damage.health_quarter_points = hp;
                }
                if bot.health > 0 {
                    actors.insert_reserved(
                        id,
                        crate::actor_collision::Bounds::body(
                            step.runtime.physical_position(),
                            bot_physics::Body::default(),
                        ),
                    )?;
                } else {
                    bot.target.pending_move = None;
                    actors.remove(id);
                    self.bot_registry.reservations.remove(&id);
                }
                if !step.packet.is_empty() {
                    packets.push(step.packet);
                    receipts.push(step.receipt);
                }
            }
        }
        for receipt in receipts {
            self.record(receipt, wall)?;
        }
        for packet in packets {
            self.broadcast_bot_packet(packet);
        }
        Ok(Vec::new())
    }
    /// Fair admission to shared certified passages. Ordinary role routes remain
    /// intact; the follower holds supported current poses outside entry until granted.
    fn tick_drop_admissions(&mut self, now: Duration, wall: u64) -> Result<(), &'static str> {
        if !self
            .bot_registry
            .config
            .as_ref()
            .is_some_and(|c| c.autonomous)
        {
            return Ok(());
        }
        let policy = self
            .bot_registry
            .config
            .as_ref()
            .unwrap()
            .objective_policy
            .as_ref()
            .unwrap()
            .clone();
        let mut receipts = Vec::new();
        self.bot_registry
            .failed_passages
            .retain(|_, until| *until > now);
        let owned: BTreeSet<_> = self
            .bot_registry
            .drop_queues
            .values()
            .filter_map(|q| q.owner.as_ref().map(|o| o.life.id))
            .collect();
        let mesh = self.navmesh_world.clone();
        for b in self.bot_registry.bots.values_mut() {
            b.drop_waiting = None;
            b.drop_committed = false;
            b.drop_permit = None;
            if b.health == 0 {
                b.delay_episode = None;
                b.reconsider = None;
                b.reconsider_history.clear();
                b.traffic_yield = None;
                b.traffic_yield_cursor = 0;
                b.traffic_progress = None;
                b.holding_access = None;
                b.drop_queue_passage = None;
                b.drop_queue_since = None;
                b.drop_holding_goal = None;
                b.drop_resume = false;
                b.drop_egress = false;
                b.drop_retreat = false;
                b.drop_retreat_preentry = false;
                b.drop_reroute = false;
                b.drop_resume_failure_at = None;
                b.drop_queue_approach.clear();
                b.holding_retry_at = Duration::ZERO;
                b.drop_approach_ready = false;
                continue;
            }
            let r = b.target.runtime.unwrap();
            if let Some(p) = b
                .target
                .registry_aborted_drop_retreat(policy.drop_entry_buffer_m)
                && r.navmesh_state.navigation_aborted
                && p.phase != DropPhase::Retreated
            {
                b.drop_retreat = true;
                b.drop_retreat_preentry = false;
                self.bot_registry
                    .failed_passages
                    .entry(p.id)
                    .or_insert(now + Duration::from_millis(policy.drop_failure_cooldown_ms));
            }
            if let Some(p) = b
                .target
                .registry_grounded_passage_retreat(policy.drop_entry_buffer_m)
            {
                if p.phase == DropPhase::Retreated && !owned.contains(&b.target.object_index) {
                    let actual_component = mesh
                        .as_ref()
                        .and_then(|m| m.component_id(r.physical_position()).ok());
                    let launch_component =
                        mesh.as_ref().and_then(|m| m.component_id(p.launch).ok());
                    receipts.push(json!({"op":"bot_drop_admission","timestamp_ms":now.as_millis()as u64,"drop_id":p.id,"object_index":b.target.object_index,"generation":r.generation,"action":"released","reason":"preentry_retreat_clear","actual_position":r.position,"actual_component":actual_component,"launch_component":launch_component,"left_spawn_component":actual_component.is_some() && actual_component!=launch_component}));
                    b.drop_reroute = true;
                    b.drop_retreat = false;
                    b.drop_retreat_preentry = false;
                    b.drop_resume = false;
                    b.drop_holding_goal = None;
                } else if r.navmesh_state.navigation_aborted {
                    b.drop_retreat = true;
                    b.drop_retreat_preentry = true;
                }
            }
            if let Some(p) = b.drop_queue_passage
                && !b.drop_retreat
                && !b.drop_reroute
                && p.phase == DropPhase::Approaching
                && (r.navmesh_state.navigation_aborted
                    || (!owned.contains(&b.target.object_index)
                        && matches!(r.phase, BotPhase::Complete)))
                && r.physics_motion.is_some_and(|m| m.grounded)
                && !r.navmesh_state.on_player
                && intersects(
                    crate::actor_collision::Bounds::body(r.physical_position(), Default::default()),
                    p.protected_bounds,
                )
                && b.target
                    .registry_drop_status(p.id, policy.drop_entry_buffer_m)
                    .is_some_and(|s| s.phase == DropPhase::Approaching)
            {
                // A body already inside a gate cannot take a holding route whose
                // entire envelope must avoid that gate. Physically retreat first.
                b.drop_retreat = true;
                b.drop_retreat_preentry = true;
            }
        }
        let passages: BTreeMap<_, _> = self
            .bot_registry
            .bots
            .iter()
            .filter(|(_, b)| b.health > 0 && !b.drop_reroute)
            .filter_map(|(&id, b)| {
                b.planned_drop(policy.drop_entry_buffer_m)
                    .or(b.drop_queue_passage)
                    .map(|p| (id, p))
            })
            .collect();
        for (&id, b) in &mut self.bot_registry.bots {
            let r = b.target.runtime.unwrap();
            let key = passages.get(&id).map(|p| crate::bot_traffic_costs::Key {
                generation: r.generation,
                objective: b.task.objective,
                role_slot: b.role_slot,
                passage: p.id,
            });
            if b.delay_episode.as_ref().is_some_and(|e| Some(e.key) != key) {
                b.delay_episode = None;
                b.reconsider = None;
                b.reconsider_history.clear();
            }
            if b.delay_episode.is_none()
                && !owned.contains(&id)
                && let Some(key) = key
            {
                b.delay_episode = Some(crate::bot_traffic_costs::Episode::new(key, now));
            }
        }
        let actors = self.actor_grid(now)?;
        let mut traffic_budget = policy.drop_traffic_query_budget;
        let mut yield_requests = BTreeSet::new();
        let mut queues = std::mem::take(&mut self.bot_registry.drop_queues);
        for passage in passages.values() {
            queues.entry(passage.id).or_default();
        }
        let mut active_access: BTreeMap<_, _> = queues
            .iter()
            .filter_map(|(&id, q)| {
                q.owner.as_ref().map(|o| {
                    (
                        id,
                        (
                            o.passage.protected_bounds,
                            if o.cert_started {
                                Vec::new()
                            } else {
                                self.bot_registry.bots[&o.life.id]
                                    .drop_queue_approach
                                    .clone()
                            },
                        ),
                    )
                })
            })
            .collect();
        // Give every passage a bounded work share, even when an earlier queue
        // has an expensive blocked approach. Older tickets win crossing access.
        let queue_budget = policy.drop_traffic_query_budget / queues.len().max(1);
        let mut queue_order: Vec<_> = queues.keys().copied().collect();
        queue_order.sort_by_key(|id| {
            (
                queues[id]
                    .waiting
                    .iter()
                    .filter_map(|life| {
                        self.bot_registry
                            .bots
                            .get(&life.id)
                            .and_then(|b| b.drop_queue_since)
                    })
                    .min()
                    .unwrap_or(now),
                *id,
            )
        });
        for drop_id in queue_order {
            let q = queues.get_mut(&drop_id).unwrap();
            q.waiting.retain(|life| {
                self.bot_registry.bots.get(&life.id).is_some_and(|b| {
                    b.health > 0 && b.target.runtime.unwrap().generation == life.generation
                }) && passages.get(&life.id).is_some_and(|p| p.id == drop_id)
            });
            let mut rotate = None;
            let mut released_clear = None;
            if let Some(owner) = q.owner.as_mut() {
                let b = self.bot_registry.bots.get_mut(&owner.life.id).filter(|b| {
                    b.health > 0 && b.target.runtime.unwrap().generation == owner.life.generation
                });
                let mut release = None;
                if let Some(b) = b {
                    let r = b.target.runtime.unwrap();
                    let status = b
                        .target
                        .registry_drop_status(drop_id, policy.drop_entry_buffer_m);
                    if let Some(p) = status {
                        owner.passage = p;
                    }
                    let inside = intersects(
                        crate::actor_collision::Bounds::body(
                            r.physical_position(),
                            Default::default(),
                        ),
                        owner.passage.protected_bounds,
                    );
                    owner.cert_started |= matches!(
                        owner.passage.phase,
                        DropPhase::Traversing | DropPhase::Landed
                    );
                    if let Some(e) = b.delay_episode.as_mut() {
                        if owner.cert_started {
                            e.entered.get_or_insert(now);
                        }
                        if owner.cert_started
                            && r.physics_motion.is_some_and(|m| {
                                m.grounded
                                    && m.position[1]
                                        <= owner.passage.landing[1]
                                            + bot_physics::Body::default().max_step
                            })
                            && !r.navmesh_state.on_player
                        {
                            e.landed.get_or_insert(now);
                        }
                    }
                    owner.entered |= inside
                        || matches!(
                            owner.passage.phase,
                            DropPhase::Traversing | DropPhase::Landed
                        );
                    if distance(owner.last_position, r.position) >= 0.5 {
                        owner.last_position = r.position;
                        owner.progress_at = now;
                    }
                    let safe_outside = r.physics_motion.is_some_and(|m| m.grounded)
                        && !r.navmesh_state.on_player
                        && !inside;
                    // An unentered contact-stopped body outside the entire XZ
                    // passage column cannot be certified flight. Yield its permit,
                    // retain its ticket and let normal idle gravity settle it.
                    let blocked_outside =
                        blocked_preentry_can_yield(r, owner.passage.protected_bounds);
                    let clear_distance = distance(r.position, position(owner.passage.landing));
                    if status.is_some_and(|p| p.phase == DropPhase::Retreated) && safe_outside {
                        release = Some(if b.drop_retreat_preentry {
                            "preentry_retreat_clear"
                        } else {
                            "aborted_retreat_clear"
                        });
                        released_clear = Some(owner.life.id);
                    } else if !owner.cert_started && b.drop_reroute && safe_outside {
                        release = Some("reroute_before_entry");
                    } else if owner.cert_started
                        && r.physics_motion.is_some_and(|m| {
                            m.position[1]
                                <= owner.passage.landing[1] + bot_physics::Body::default().max_step
                        })
                        && safe_outside
                        && clear_distance >= policy.drop_clearance_m
                        && (status.is_none() || owner.passage.phase == DropPhase::Cleared)
                    {
                        release = Some("cleared");
                        released_clear = Some(owner.life.id);
                    } else if !owner.cert_started
                        && safe_outside
                        && !passages
                            .get(&owner.life.id)
                            .is_some_and(|p| p.id == drop_id)
                    {
                        release = Some("route_changed_before_entry");
                    } else if !owner.cert_started
                        && (safe_outside || blocked_outside)
                        && now.saturating_sub(owner.progress_at)
                            >= Duration::from_millis(policy.drop_preentry_stall_ms)
                    {
                        rotate = Some(owner.life);
                        release = Some(if blocked_outside {
                            "stalled_before_entry_blocked_contact"
                        } else {
                            "stalled_before_entry"
                        });
                    }
                } else {
                    release = Some("life_removed_noncollidable");
                }
                if let Some(reason) = release {
                    let recovery =
                        matches!(reason, "aborted_retreat_clear" | "preentry_retreat_clear");
                    let r = self
                        .bot_registry
                        .bots
                        .get(&owner.life.id)
                        .and_then(|b| b.target.runtime);
                    let actual_component = r.and_then(|r| {
                        mesh.as_ref()
                            .and_then(|m| m.component_id(r.physical_position()).ok())
                    });
                    let launch_component = mesh
                        .as_ref()
                        .and_then(|m| m.component_id(owner.passage.launch).ok());
                    if let Some(b) = self.bot_registry.bots.get_mut(&owner.life.id)
                        && let Some(e) = b.delay_episode.take()
                        && reason == "cleared"
                    {
                        let key = crate::bot_traffic_costs::Key {
                            generation: b.target.runtime.unwrap().generation,
                            objective: b.task.objective,
                            role_slot: b.role_slot,
                            passage: drop_id,
                        };
                        let p = b.target.runtime.unwrap().physical_position();
                        let free_departure = (p[0] - owner.passage.landing[0])
                            .hypot(p[2] - owner.passage.landing[2])
                            / retail_movement::WALK_FORWARD_MPS;
                        if let Some(sample) = e.sample(key, now, free_departure) {
                            self.bot_registry.delay_book.record(
                                drop_id,
                                now,
                                sample,
                                policy.drop_congestion_headway_s,
                                &policy.drop_delay,
                            );
                            self.bot_registry
                                .delay_book
                                .approaches
                                .insert(drop_id, e.approach.clone());
                            receipts.push(json!({"op":"bot_drop_delay_sample","timestamp_ms":now.as_millis()as u64,"object_index":owner.life.id,"generation":owner.life.generation,"drop_id":drop_id,"sample":sample,"assigned_ms":e.assigned.as_millis()as u64,"queued_ms":e.queued.map(|t|t.as_millis()as u64),"granted_ms":e.granted.map(|t|t.as_millis()as u64),"entered_ms":e.entered.map(|t|t.as_millis()as u64),"landed_ms":e.landed.map(|t|t.as_millis()as u64),"free_approach_s":e.free_approach_s,"free_traversal_s":e.free_traversal_s,"free_departure_s":free_departure,"estimate":self.bot_registry.delay_book.estimate(drop_id,now,policy.drop_congestion_headway_s,&policy.drop_delay)}));
                        }
                    }
                    receipts.push(json!({"op":"bot_drop_admission","timestamp_ms":now.as_millis()as u64,"drop_id":drop_id,"object_index":owner.life.id,"generation":owner.life.generation,"action":"released","reason":reason,"actual_position":r.map(|r|r.position),"actual_component":actual_component,"launch_component":launch_component,"left_spawn_component":actual_component.is_some() && actual_component!=launch_component}));
                    if let Some(b) = self.bot_registry.bots.get_mut(&owner.life.id)
                        && (reason == "cleared"
                            || recovery
                            || reason == "life_removed_noncollidable")
                    {
                        if reason == "cleared" || recovery {
                            let r = b.target.runtime.unwrap();
                            b.cleared_drop_routes
                                .insert(drop_id, (r.generation, r.since));
                        }
                        b.drop_queue_passage = recovery.then_some(owner.passage);
                        b.drop_queue_since = None;
                        b.drop_holding_goal = None;
                        b.drop_resume = false;
                        b.drop_retreat = false;
                        b.drop_egress = false;
                        if recovery {
                            b.drop_reroute = true;
                            self.bot_registry.failed_passages.insert(
                                drop_id,
                                now + Duration::from_millis(policy.drop_failure_cooldown_ms),
                            );
                        }
                        b.drop_retreat_preentry = false;
                    }
                    q.owner = None;
                    active_access.remove(&drop_id);
                }
            }
            q.waiting.retain(|life| {
                let Some(b) = self.bot_registry.bots.get_mut(&life.id) else { return false; };
                if b.drop_reroute { return false; }
                let r = b.target.runtime.unwrap();
                let failed = self.bot_registry.failed_passages.get(&drop_id).is_some_and(|until|*until>now);
                if failed && !b.drop_committed && r.physics_motion.is_some_and(|m|m.grounded) && !r.navmesh_state.on_player
                    && b.drop_queue_passage.is_some_and(|p| !intersects(crate::actor_collision::Bounds::body(r.physical_position(),Default::default()),p.protected_bounds)) {
                    b.drop_reroute=true; b.drop_resume=false;
                    receipts.push(json!({"op":"bot_drop_admission","timestamp_ms":now.as_millis()as u64,"drop_id":drop_id,"object_index":life.id,"generation":life.generation,"action":"reroute_requested","reason":"failed_passage_cooldown"}));
                    return false;
                }
                true
            });
            let mut ready: Vec<_> = passages
                .iter()
                .filter(|&(id, p)| {
                    p.id == drop_id
                        && released_clear != Some(*id)
                        && !self.bot_registry.bots[id].drop_reroute
                        && !self.bot_registry.bots[id].drop_retreat_preentry
                        && p.phase == DropPhase::Approaching
                        && p.route_distance_to_launch <= policy.drop_queue_distance_m
                        && self.bot_registry.bots[id]
                            .target
                            .runtime
                            .unwrap()
                            .physics_motion
                            .is_some_and(|m| m.grounded)
                        && !self.bot_registry.bots[id]
                            .target
                            .runtime
                            .unwrap()
                            .navmesh_state
                            .on_player
                })
                .map(|(&id, p)| (id, *p))
                .collect();
            ready.sort_by(|a, b| {
                a.1.route_distance_to_launch
                    .total_cmp(&b.1.route_distance_to_launch)
                    .then(a.0.cmp(&b.0))
            });
            for (id, passage) in ready {
                let life = PassageLife {
                    id,
                    generation: self.bot_registry.bots[&id]
                        .target
                        .runtime
                        .unwrap()
                        .generation,
                };
                if q.owner.as_ref().is_some_and(|o| o.life == life)
                    || q.waiting.contains(&life)
                    || rotate == Some(life)
                {
                    continue;
                }
                q.waiting.push_back(life);
                let b = self.bot_registry.bots.get_mut(&id).unwrap();
                b.drop_queue_passage = Some(passage);
                b.drop_queue_since.get_or_insert(now);
                if let Some(e) = b.delay_episode.as_mut() {
                    e.queued.get_or_insert(now);
                }
                b.drop_queue_approach.clear();
                b.holding_retry_at = Duration::ZERO;
                b.drop_approach_ready = false;
                if let Some(route) = b.target.registry_route()
                    && let Some(end) = route
                        .waypoint_drop_ids
                        .iter()
                        .position(|d| *d == Some(drop_id))
                {
                    let mut remaining = policy.drop_entry_lane_m;
                    for i in (1..=end).rev() {
                        let to = route.waypoints[i];
                        let mut from = route.waypoints[i - 1];
                        let length = ((from[0] - to[0]).powi(2) + (from[2] - to[2]).powi(2)).sqrt();
                        if length > remaining {
                            let f = remaining / length;
                            from =
                                std::array::from_fn(|axis| to[axis] + (from[axis] - to[axis]) * f);
                        }
                        b.drop_queue_approach.push([from, to]);
                        remaining -= length;
                        if remaining <= 0. {
                            break;
                        }
                    }
                }
                // Freeze the original approach while bounded holding work is pending.
                // Route intent/cursor remain available; no pose or Motion is changed.
                b.target.runtime.as_mut().unwrap().phase = BotPhase::Complete;
                receipts.push(json!({"op":"bot_drop_admission","timestamp_ms":now.as_millis()as u64,"drop_id":drop_id,"object_index":id,"generation":life.generation,"action":"queued","queue_length":q.waiting.len(),"launch":passage.launch,"distance_to_launch":passage.distance_to_launch}));
            }
            if let Some(life) = rotate {
                self.bot_registry
                    .bots
                    .get_mut(&life.id)
                    .unwrap()
                    .target
                    .runtime
                    .as_mut()
                    .unwrap()
                    .phase = BotPhase::Complete;
                q.waiting.push_back(life);
            }
            if q.owner.is_none() {
                // A fresh run cannot normally find an already traversing actor,
                // but never revoke in-flight physics if admission starts mid-step.
                let flying = passages
                    .iter()
                    .find(|(id, p)| {
                        p.id == drop_id
                            && released_clear != Some(**id)
                            && matches!(p.phase, DropPhase::Traversing | DropPhase::Landed)
                    })
                    .map(|(&id, _)| PassageLife {
                        id,
                        generation: self.bot_registry.bots[&id]
                            .target
                            .runtime
                            .unwrap()
                            .generation,
                    });
                let probes = policy.candidate_checks_per_tick.min(q.waiting.len());
                let candidate_budget = queue_budget / probes.max(1);
                let mut indices: Vec<_> = (0..probes)
                    .map(|offset| (q.probe_cursor + offset) % q.waiting.len())
                    .collect();
                q.probe_cursor = (q.probe_cursor + probes) % q.waiting.len().max(1);
                // Within each fair probe window preserve original ticket age.
                indices.sort_unstable();
                let staged = indices.into_iter().find(|&index| {
                    let life = &q.waiting[index];
                    let mut traffic_budget = candidate_budget;
                    let b = &self.bot_registry.bots[&life.id];
                    let ready = b.drop_approach_ready
                        && b.target.runtime.is_some_and(|r| {
                            matches!(r.phase, BotPhase::Complete)
                                && !r.navmesh_state.navigation_aborted
                                && r.physics_motion.is_some_and(|m| m.grounded)
                                && !r.navmesh_state.on_player
                                && b.drop_queue_passage.is_some_and(|p| {
                                    !intersects(
                                        crate::actor_collision::Bounds::body(
                                            r.physical_position(),
                                            Default::default(),
                                        ),
                                        p.protected_bounds,
                                    )
                                })
                                && b.drop_holding_goal
                                    .is_some_and(|g| distance(g, r.position) <= 0.6)
                        });
                    if !ready {
                        return false;
                    }
                    if active_access.iter().any(|(&other, (bounds, lane))| {
                        other != drop_id
                            && (intersects(*bounds, b.drop_queue_passage.unwrap().protected_bounds)
                                || crate::bot_traffic_flow::lanes_conflict(
                                    &b.drop_queue_approach,
                                    lane,
                                    &mut traffic_budget,
                                ))
                    }) {
                        return false;
                    }
                    match crate::bot_traffic_flow::approach_blocker(
                        &actors,
                        life.id,
                        &b.drop_queue_approach,
                        &mut traffic_budget,
                    ) {
                        Ok(Some(blocker)) => {
                            yield_requests.insert(blocker);
                            false
                        }
                        Ok(None) => true,
                        Err(_) => false,
                    }
                });
                if let Some(life) =
                    flying.or_else(|| staged.and_then(|index| q.waiting.remove(index)))
                {
                    q.waiting.retain(|v| *v != life);
                    let b = self.bot_registry.bots.get_mut(&life.id).unwrap();
                    let r = b.target.runtime.unwrap();
                    let p = passages[&life.id];
                    b.reconsider = None;
                    if let Some(e) = b.delay_episode.as_mut() {
                        let free_approach = b
                            .drop_queue_approach
                            .iter()
                            .map(|s| (s[1][0] - s[0][0]).hypot(s[1][2] - s[0][2]))
                            .sum::<f64>()
                            / retail_movement::WALK_FORWARD_MPS;
                        let free_traversal = (p.launch[0] - p.landing[0])
                            .hypot(p.launch[2] - p.landing[2])
                            / retail_movement::WALK_FORWARD_MPS;
                        e.grant(now, free_approach, free_traversal);
                        e.approach = b.drop_queue_approach.clone();
                    }
                    let entered = matches!(p.phase, DropPhase::Traversing | DropPhase::Landed)
                        || intersects(
                            crate::actor_collision::Bounds::body(
                                r.physical_position(),
                                Default::default(),
                            ),
                            p.protected_bounds,
                        );
                    active_access.insert(
                        drop_id,
                        (
                            p.protected_bounds,
                            if matches!(p.phase, DropPhase::Traversing | DropPhase::Landed) {
                                Vec::new()
                            } else {
                                b.drop_queue_approach.clone()
                            },
                        ),
                    );
                    q.owner = Some(PassageOwner {
                        life,
                        passage: p,
                        last_position: r.position,
                        progress_at: now,
                        entered,
                        cert_started: matches!(p.phase, DropPhase::Traversing | DropPhase::Landed),
                    });
                    receipts.push(json!({"op":"bot_drop_admission","timestamp_ms":now.as_millis()as u64,"drop_id":drop_id,"object_index":life.id,"generation":life.generation,"action":"granted","queue_length":q.waiting.len()}));
                }
            }
            if let Some(owner) = &q.owner
                && let Some(b) = self.bot_registry.bots.get_mut(&owner.life.id)
            {
                b.drop_permit = Some(drop_id);
                b.drop_committed = owner.entered;
                b.drop_egress = b
                    .target
                    .registry_aborted_drop(policy.drop_entry_buffer_m)
                    .is_some();
                if b.drop_holding_goal.is_some()
                    && !owner.entered
                    && !b.drop_retreat
                    && !b.drop_retreat_preentry
                {
                    b.drop_resume = true;
                }
            }
            for life in &q.waiting {
                if let Some(b) = self.bot_registry.bots.get_mut(&life.id) {
                    b.drop_waiting = Some(drop_id);
                }
            }
        }
        // Refresh the owner's actual remaining route, including the approach
        // from its holding pad. Other queues must keep this physical lane open.
        for q in queues.values() {
            if let Some(owner) = &q.owner {
                let b = self.bot_registry.bots.get_mut(&owner.life.id).unwrap();
                if let Some(route) = b.target.registry_route() {
                    let r = b.target.runtime.unwrap();
                    if let Some(approach) = crate::bot_traffic_flow::approach_prefix(
                        r.physical_position(),
                        route,
                        r.navigation_waypoint,
                        owner.passage.id,
                    ) {
                        b.drop_queue_approach = approach;
                    }
                }
                if !owner.cert_started
                    && let Ok(Some(blocker)) = crate::bot_traffic_flow::approach_blocker(
                        &actors,
                        owner.life.id,
                        &b.drop_queue_approach,
                        &mut traffic_budget,
                    )
                {
                    yield_requests.insert(blocker);
                }
            }
        }
        // Only queued, supported nonowners can yield. Humans and committed
        // certificates remain authoritative obstacles. Reuse bounded holding
        // planning and normal collision movement; never rewrite the actual pose.
        let owner_ids: BTreeSet<_> = queues
            .values()
            .filter_map(|q| q.owner.as_ref().map(|o| o.life.id))
            .collect();
        for id in yield_requests {
            if owner_ids.contains(&id) {
                continue;
            }
            if let Some(b) = self.bot_registry.bots.get_mut(&id)
                && b.drop_waiting.is_some()
                && b.target.runtime.is_some_and(|r| {
                    r.physics_motion.is_some_and(|m| m.grounded) && !r.navmesh_state.on_player
                })
                && b.drop_holding_goal.is_some_and(|goal| {
                    distance(goal, b.target.runtime.unwrap().position) <= policy.arrival_m
                })
            {
                b.drop_holding_goal = None;
                b.drop_approach_ready = false;
                b.target.runtime.as_mut().unwrap().phase = BotPhase::Complete;
                receipts.push(json!({"op":"bot_traffic_yield","timestamp_ms":now.as_millis()as u64,"object_index":id,"drop_id":b.drop_waiting,"action":"yield_requested","reason":"blocks_approach"}));
            }
        }
        queues.retain(|_, q| q.owner.is_some() || !q.waiting.is_empty());
        self.bot_registry.drop_queues = queues;
        self.tick_local_traffic(now, &policy, &actors, &mut receipts)?;
        for receipt in receipts {
            self.record(receipt, wall)?;
        }
        Ok(())
    }
    fn tick_legacy_bot(&mut self, now: Duration, wall: u64) -> Result<(), &'static str> {
        // Compatibility admin target: identity/life gates remain unchanged, only the clock moves.
        let map = self.map_name().to_owned();
        let mut actors = self.actor_grid(now)?;
        let mut packets = Vec::new();
        let mut receipts = Vec::new();
        for actor in self.active.iter_mut().chain(self.peers.values_mut()) {
            if !actor.dummy_created || !actor.ready_for_updates || !actor.entered_game {
                continue;
            }
            let Some(target) = actor.development_target.as_mut() else {
                continue;
            };
            if !matches!(actor.life,CharacterLife::Alive{generation,..} if generation==target.generation)
            {
                continue;
            }
            if let Some(step) = target.script_step_with_actors(
                2,
                now,
                self.wire_profile,
                self.collision_world.as_ref(),
                &map,
                actor.dummy_health.unwrap_or(0),
                self.fall_damage_rules.as_ref(),
                Some(crate::actor_collision::Query {
                    grid: &actors,
                    actor: target.object_index,
                }),
            )? {
                if let Some(hp) = step.health {
                    actor.dummy_health = Some(hp);
                }
                target.runtime = Some(step.runtime);
                if matches!(step.runtime.phase, BotPhase::Dead) {
                    target.pending_move = None;
                }
                if actor.dummy_health.is_some_and(|hp| hp > 0) {
                    let bounds = step.swept_bounds.unwrap_or_else(|| {
                        crate::actor_collision::Bounds::body(
                            step.runtime.physical_position(),
                            bot_physics::Body::default(),
                        )
                    });
                    actors.insert_reserved(target.object_index, bounds)?;
                    self.bot_registry
                        .reservations
                        .insert(target.object_index, (bounds, now + Duration::from_secs(1)));
                } else {
                    actors.remove(target.object_index);
                }
                packets.push(step.packet);
                receipts.push(step.receipt);
            }
        }
        for receipt in receipts {
            self.record(receipt, wall)?;
        }
        for packet in packets {
            self.broadcast_bot_packet(packet);
        }
        Ok(())
    }
    fn reconcile_bot_population(
        &mut self,
        now: Duration,
        wall: u64,
        actors: &mut crate::actor_collision::ActorGrid,
    ) -> Result<(), &'static str> {
        let humans: Vec<_> = self
            .active
            .iter()
            .chain(self.peers.values())
            .map(|a| {
                (
                    a.reservation.player,
                    a.team(),
                    a.tracking.sample.map(|s| s.position),
                )
            })
            .collect();
        let desired = self.bot_registry.total.saturating_sub(humans.len());
        let human_ids: BTreeSet<_> = humans.iter().map(|h| h.0).collect();
        let collisions: Vec<_> = self
            .bot_registry
            .bots
            .keys()
            .copied()
            .filter(|id| human_ids.contains(id))
            .collect();
        for id in collisions {
            self.retire_bot_for_human(id, now, wall)?;
        }
        while self.bot_registry.bots.len() > desired {
            let counts = |team| {
                humans.iter().filter(|h| h.1 == team).count()
                    + self
                        .bot_registry
                        .bots
                        .values()
                        .filter(|b| b.target.squad.team() == team)
                        .count()
            };
            let team = if counts(0) > counts(1) { 0 } else { 1 };
            let id = self
                .bot_registry
                .bots
                .iter()
                .rev()
                .find(|(_, b)| b.target.squad.team() == team)
                .or_else(|| self.bot_registry.bots.iter().next_back())
                .map(|(&id, _)| id)
                .unwrap();
            self.retire_bot_for_human(id, now, wall)?;
        }
        // A forced ID replacement may remove the other side's bot. Retire an
        // overfull-side bot, then refill the underfull side; never evict humans.
        for team in 0..2 {
            loop {
                let count = humans.iter().filter(|h| h.1 == team).count()
                    + self
                        .bot_registry
                        .bots
                        .values()
                        .filter(|b| b.target.squad.team() == team)
                        .count();
                if count <= self.bot_registry.total.div_ceil(2) {
                    break;
                }
                let Some(id) = self
                    .bot_registry
                    .bots
                    .iter()
                    .rev()
                    .find(|(_, b)| b.target.squad.team() == team)
                    .map(|(&id, _)| id)
                else {
                    break;
                };
                self.retire_bot_for_human(id, now, wall)?;
            }
        }
        // Human reservations own squad space before their initial snapshot is
        // published. A late admission can replace a bot in a different squad;
        // retire any newly overfull squad and refill the actual vacancies below.
        let squads: Vec<_> = self
            .spawn_catalog
            .as_ref()
            .ok_or("spawn_catalog_unavailable")?
            .map_pool()
            .squads
            .iter()
            .map(|s| s.index)
            .collect();
        for squad in squads {
            while self.squad_reservation_count(squad) + self.bot_registry.members(squad).count()
                > self.match_profile().squad_capacity
            {
                let Some(id) = self.bot_registry.members(squad).max() else {
                    break;
                };
                self.retire_bot_for_human(id, now, wall)?;
            }
        }
        let config = self.bot_registry.config.as_ref().unwrap().clone();
        while self.bot_registry.bots.len() < desired {
            let counts = |team| {
                humans.iter().filter(|h| h.1 == team).count()
                    + self
                        .bot_registry
                        .bots
                        .values()
                        .filter(|b| b.target.squad.team() == team)
                        .count()
            };
            let team = if counts(0) <= counts(1) { 0 } else { 1 };
            let used: BTreeSet<_> = self.bot_registry.bots.values().map(|b| b.slot).collect();
            let mut squads: Vec<_> = self
                .spawn_catalog
                .as_ref()
                .ok_or("spawn_catalog_unavailable")?
                .map_pool()
                .squads
                .iter()
                .filter(|s| s.team == team)
                .map(|s| {
                    (
                        self.squad_reservation_count(s.index)
                            + self.bot_registry.members(s.index).count(),
                        s.index,
                    )
                })
                .filter(|(count, _)| *count < self.match_profile().squad_capacity)
                .collect();
            squads.sort_unstable();
            let selected = squads.into_iter().find_map(|(_, squad)| {
                config
                    .slots
                    .iter()
                    .enumerate()
                    .find(|(i, s)| {
                        s.team == team
                            && s.squad.is_none_or(|bound| bound == squad)
                            && !used.contains(i)
                            && !actors
                                .occupied(u16::MAX, point(s.position), bot_physics::Body::default())
                                .unwrap_or(true)
                    })
                    .map(|(i, s)| (squad, i, s))
            });
            let Some((squad, slot_index, slot)) = selected else {
                if self
                    .bot_registry
                    .last_wait_receipt
                    .is_none_or(|last| now.saturating_sub(last) >= Duration::from_secs(1))
                {
                    self.record(json!({"op":"bot_population_waiting","reason":"spawn_slots_occupied","team":team,"desired_bots":desired,"actual_bots":self.bot_registry.bots.len()}),wall)?;
                    self.bot_registry.last_wait_receipt = Some(now);
                }
                break;
            };
            let Some(index) = (0..self.admission_capacity() as u16)
                .rev()
                .find(|i| !human_ids.contains(i) && !self.bot_registry.bots.contains_key(i))
            else {
                return Err("bot_population_no_free_character_index");
            };
            let squad = replication::SquadAddress::from_index(squad)?;
            let used_roles: BTreeSet<_> = self
                .bot_registry
                .bots
                .values()
                .filter(|b| b.target.squad == squad)
                .map(|b| b.role_slot)
                .collect();
            let role_slot = (0..8)
                .find(|role| !used_roles.contains(role))
                .ok_or("bot_population_no_free_role")?;
            let faction = if team == 0 {
                self.match_profile().attacking_faction
            } else {
                self.match_profile().defending_faction
            };
            let inventory = self
                .item_catalog
                .bootstrap_default_inventory_index(faction)
                .ok_or("missing_retail_target_loadout")?;
            let loadout = self
                .item_catalog
                .bootstrap_loadout(faction, inventory)
                .ok_or("missing_retail_target_loadout")?;
            let generation = self
                .bot_registry
                .generations
                .get(&index)
                .copied()
                .unwrap_or(0)
                .checked_add(1)
                .ok_or("bot_generation_exhausted")?;
            let mut runtime = BotRuntime::spawned(now, slot.position);
            runtime.generation = generation;
            runtime.phase = BotPhase::Complete;
            let terrain =
                bot_physics::WorldTerrain::new(self.collision_world.as_ref(), self.map_name())
                    .map_err(|e| e.code())?;
            runtime.physics_motion = Some(
                terrain
                    .start_motion(runtime.physical_position(), bot_physics::Body::default())
                    .map_err(|e| e.code())?,
            );
            let used_names: BTreeSet<_> = self
                .bot_registry
                .bots
                .values()
                .map(|b| b.name_index)
                .collect();
            let name_index = (0..self.bot_names.len() as u16)
                .find(|i| !used_names.contains(i))
                .ok_or("bot_name_index_exhausted")?;
            let display_name = self
                .bot_names
                .name_for(name_index)
                .ok_or("bot_name_index")?
                .to_owned();
            // A side containing only bots still owns a shared schedule. Adopt
            // the same authored first-participant policy as human confirmation,
            // while preserving both initial bot admission and an existing phase.
            let new_clock = if self.team_waves[usize::from(team)].is_none() {
                let policy = self.match_profile();
                Some(TeamWaveClock {
                    first_deadline: now
                        .checked_add(Duration::from_millis(u64::from(
                            policy.initial_spawn_delay_ms[usize::from(team)],
                        )))
                        .ok_or("wave_clock_overflow")?,
                    period: Duration::from_millis(u64::from(
                        policy.spawn_wave_ms[usize::from(team)],
                    )),
                })
            } else {
                None
            };
            self.record(json!({"op":"bot_population_spawn","name":display_name,"name_index":name_index,"object_index":index,"generation":generation,"team":team,"squad_index":squad.squad_index(),"source_spawn":slot.source_spawn,"slot":slot_index,"position":slot.position}),wall)?;
            if let Some(clock) = new_clock {
                self.team_waves[usize::from(team)] = Some(clock);
            }
            self.bot_registry.generations.insert(index, generation);
            self.bot_registry.bots.insert(
                index,
                WorldBot {
                    name_index,
                    target: Target {
                        object_index: index,
                        display_name: Some(display_name),
                        generation: 0,
                        position: slot.position,
                        faction,
                        squad,
                        roster_published: true,
                        loadout,
                        script: None,
                        runtime: Some(runtime),
                        pending_move: None,
                        move_commands: Default::default(),
                    },
                    slot: slot_index,
                    role_slot,
                    traffic_yield: None,
                    traffic_yield_cursor: 0,
                    traffic_progress: None,
                    holding_access: None,
                    delay_episode: None,
                    reconsider: None,
                    reconsider_at: Duration::ZERO,
                    reconsider_history: BTreeMap::new(),
                    drop_waiting: None,
                    drop_committed: false,
                    drop_permit: None,
                    cleared_drop_routes: BTreeMap::new(),
                    drop_queue_passage: None,
                    drop_holding_goal: None,
                    drop_holding_attempt: 0,
                    holding_retry_at: Duration::ZERO,
                    drop_resume: false,
                    drop_egress: false,
                    drop_retreat: false,
                    drop_retreat_preentry: false,
                    drop_reroute: false,
                    drop_resume_failure_at: None,
                    drop_queue_approach: Vec::new(),
                    drop_approach_ready: false,
                    drop_queue_since: None,
                    health: 400,
                    damage: initial_damage(),
                    yaw: 0,
                    firearm: Default::default(),
                    healing: Default::default(),
                    move_attempted: false,
                    respawn_wait: None,
                    last_move_receipt: None,
                    omitted_healthy_steps: 0,
                    last_move_since: None,
                    last_navigation_status: None,
                    task: Default::default(),
                },
            );
            actors.insert_reserved(
                index,
                crate::actor_collision::Bounds::body(
                    runtime.physical_position(),
                    bot_physics::Body::default(),
                ),
            )?;
        }
        Ok(())
    }
    pub(super) fn clear_bot_world(&mut self) {
        self.bot_registry.bots.clear();
        self.bot_registry.known.clear();
        self.bot_registry.reservations.clear();
        self.bot_registry.director = Default::default();
        self.bot_registry.drop_queues.clear();
        self.bot_registry.delay_book = Default::default();
        self.bot_registry.collision_evidence = None;
        self.bot_registry.firearm_cursor = 0;
    }
}

#[cfg(test)]
mod identity_tests {
    use super::*;
    use crate::resources::tests::{retail_items, wire_profiles};
    fn engine(prefix: &str) -> Engine<Vec<u8>> {
        let mut proof = json!({"verified_binding":{"validated":true,"positions_compared":667,"blank_positions_compared":25,"blank_zero_empirically_verified":true}});
        for name in [
            "original_retail_elf_sha256",
            "snapshot_sha256",
            "catalog_sha256",
            "rules_sha256",
            "owner_attestation_sha256",
            "gear_slots_sha256",
            "comparator_sha256",
        ] {
            proof[name] = json!("a".repeat(64));
        }
        let mut kit = vec![None; 22];
        kit[..3].fill(Some(2));
        let rows:Vec<_>=(0..5).map(|i|json!({"update_id":1,"items":if i<3 {kit.clone()} else {vec![None;22]},"primary_ammo":if i<3 {7}else{0}})).collect();
        let bootstrap=crate::bootstrap_loadouts::BootstrapLoadouts::from_json(&json!({"schema_version":1,"application_id":20231,"provenance":proof,
            "factions":(0..3).map(|f|json!({"faction":f,"class_ref":"CharacterClassInfo0","loadouts":rows})).collect::<Vec<_>>()}).to_string()).unwrap();
        let items = std::sync::Arc::try_unwrap(retail_items())
            .ok()
            .unwrap()
            .with_bootstrap_loadouts(bootstrap);
        let names=crate::bot_names::Catalog::from_json(&serde_json::to_vec(&json!({"entries":[{"name":format!("{prefix} Alpha")},{"name":format!("{prefix} Bravo")}]})).unwrap()).unwrap();
        let spawns:spawn_catalog::SpawnCatalog=serde_json::from_value(json!({
            "format":"mag_spawn_catalog","schema_version":1,"source":"SYNTHETIC identity test",
            "active":{"map_name":"synthetic_arena","squad_index":0,"default_spawn":0,"available":[0]},
            "match_profile":{"has_bunkers":false,"has_objectives":false},
            "maps":[{"name":"synthetic_arena","owner_faction":2,"spawn_count":1,
                "locations":[{"index":0,"token":1,"kind":"on_foot","position":{"x":2.,"y":0.,"z":2.}}],
                "squads":[{"index":0,"team":0,"platoon":0,"squad":0,"faction":null,"available":[0],"default_spawn":0},
                {"index":16,"team":1,"platoon":0,"squad":0,"faction":2,"available":[0],"default_spawn":0}]}]})).unwrap();
        let resources = EngineResources::for_application(
            std::sync::Arc::new(items),
            spawns,
            None,
            wire_profiles(),
            20231,
        )
        .unwrap()
        .with_bot_names(names);
        let mut engine = Engine::from_resources(Vec::new(), 100_000, resources);
        engine.maximum_players = 2;
        let world = json!({"format":"mag_collision_world","schema_version":1,"coordinate_space":"world",
            "metadata":{"map_name":"synthetic_arena","build":"synthetic","coverage":"provisional","provenance":["Synthetic identity test floor"],"unknowns":["Not a game map"],
            "physics_regions":[{"min":[-10.,-5.,-10.],"max":[10.,8.,10.],"ground_verified":true,"obstacles_verified":true,"provenance":["Synthetic floor"]}]},
            "instances":[{"id":"floor","vertices_xyz":[[-10.,0.,-10.],[10.,0.,-10.],[10.,0.,10.],[-10.,0.,10.]],"triangles":[[0,2,1],[0,3,2]]}]});
        let path = std::env::temp_dir().join(format!(
            "mag-synthetic-identity-floor-{}-{prefix}.json",
            std::process::id()
        ));
        std::fs::write(&path, serde_json::to_vec(&world).unwrap()).unwrap();
        engine = engine.with_collision_world(
            collision::CollisionWorld::load(&path, collision::CollisionLimits::default()).unwrap(),
        );
        std::fs::remove_file(path).unwrap();
        let config:BotPopulation=serde_json::from_value(json!({"format":"mag_bot_population_v1","map_name":"synthetic_arena","firearms":{"enabled":false},"slots":[
            {"team":0,"position":{"x":2.,"y":0.,"z":2.}}, {"team":1,"position":{"x":6.,"y":0.,"z":2.}}]})).unwrap();
        engine.with_bot_population(config, 2).unwrap()
    }
    #[test]
    fn external_identities_reach_real_allocation_audit_and_client_snapshots() {
        let mut a = engine("Synthetic First");
        let mut b = engine("Synthetic Second");
        for (engine, prefix) in [(&mut a, "Synthetic First"), (&mut b, "Synthetic Second")] {
            engine
                .reconcile_bot_population(Duration::ZERO, 1, &mut Default::default())
                .unwrap();
            assert_eq!(engine.bot_registry.bots.len(), 2);
            let names: std::collections::BTreeSet<_> = engine
                .bot_registry
                .bots
                .values()
                .map(|b| b.target.display_name.as_deref().unwrap().to_owned())
                .collect();
            assert_eq!(
                names,
                std::collections::BTreeSet::from([
                    format!("{prefix} Alpha"),
                    format!("{prefix} Bravo")
                ])
            );
            for bot in engine.bot_registry.bots.values() {
                let snapshot = bot.snapshot(Duration::ZERO, engine.wire_profile).unwrap();
                let expected = format!(
                    "{prefix} {}",
                    if bot.name_index == 0 {
                        "Alpha"
                    } else {
                        "Bravo"
                    }
                );
                let runtime = bot.target.runtime.unwrap();
                let physical = replication::CharacterRespawn {
                    sequence: 1,
                    timestamp_ms: 0,
                    object_index: bot.target.object_index,
                    state: replication::CharacterState {
                        state: 1,
                        stance: 0,
                        user_data: [0; 3],
                    },
                    position: runtime.position,
                    pitch: bot.firearm.packed_pitch(),
                    yaw: bot.yaw,
                    damage: bot.damage,
                };
                let mut loadout = bot.target.loadout.clone();
                loadout.primary = bot.firearm.item(loadout.primary);
                let expected_packet = dummy::character_snapshot(
                    bot.target.object_index,
                    1,
                    0,
                    &loadout,
                    bot.target.faction,
                    engine.wire_profile,
                    &expected,
                    bot.target.squad,
                    Some(&physical),
                )
                .unwrap();
                assert_eq!(
                    snapshot, expected_packet,
                    "selected external identity must reach the native character snapshot"
                );
            }
            let records: Vec<serde_json::Value> = String::from_utf8(engine.writer.clone())
                .unwrap()
                .lines()
                .map(|l| serde_json::from_str(l).unwrap())
                .filter(|d: &serde_json::Value| d["op"] == "bot_population_spawn")
                .collect();
            assert_eq!(records.len(), 2);
            assert!(
                records
                    .iter()
                    .all(|r| names.contains(r["name"].as_str().unwrap()))
            );
        }
    }
}
