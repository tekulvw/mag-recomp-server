//! World-owned phase-aware tactical routes and bounded physical objective operators.
use super::*;
use development_target::BotPhase;
use std::collections::BTreeMap;
use std::time::Instant;

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DestinationRole {
    pub name: String,
    pub back_m: f64,
    pub side_m: f64,
}
fn drop_policy_default(name: &str) -> serde_json::Value {
    static DEFAULT: std::sync::OnceLock<serde_json::Value> = std::sync::OnceLock::new();
    DEFAULT.get_or_init(|| {
        serde_json::from_str(include_str!("../data/synthetic-bot-objective-policy.json"))
            .expect("bundled policy JSON")
    })[name]
        .clone()
}
fn default_interactions() -> bool {
    true
}
fn default_failure_cooldown() -> u64 {
    drop_policy_default("drop_failure_cooldown_ms")
        .as_u64()
        .unwrap()
}
fn default_congestion_headway() -> f64 {
    drop_policy_default("drop_congestion_headway_s")
        .as_f64()
        .unwrap()
}
fn default_drop_delay() -> crate::bot_traffic_costs::Policy {
    serde_json::from_value(drop_policy_default("drop_delay")).expect("bundled drop-delay policy")
}
fn default_traffic_query_budget() -> usize {
    drop_policy_default("drop_traffic_query_budget")
        .as_u64()
        .unwrap() as usize
}
fn default_entry_buffer() -> f64 {
    drop_policy_default("drop_entry_buffer_m").as_f64().unwrap()
}
fn default_queue_distance() -> f64 {
    drop_policy_default("drop_queue_distance_m")
        .as_f64()
        .unwrap()
}
fn default_clearance() -> f64 {
    drop_policy_default("drop_clearance_m").as_f64().unwrap()
}
fn default_preentry_stall() -> u64 {
    drop_policy_default("drop_preentry_stall_ms")
        .as_u64()
        .unwrap()
}
fn default_entry_lane() -> f64 {
    drop_policy_default("drop_entry_lane_m").as_f64().unwrap()
}
fn default_egress_grid() -> f64 {
    drop_policy_default("drop_egress_grid_m").as_f64().unwrap()
}
fn default_holding_grid() -> f64 {
    drop_policy_default("drop_holding_grid_m").as_f64().unwrap()
}
fn default_holding_candidates() -> usize {
    drop_policy_default("drop_holding_candidate_limit")
        .as_u64()
        .unwrap() as usize
}
fn default_holding_stall() -> u64 {
    drop_policy_default("drop_holding_stall_ms")
        .as_u64()
        .unwrap()
}
/// Disk-backed local policy; a population may override the complete policy object.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectivePolicy {
    #[serde(default = "default_interactions")]
    pub(super) interactions_enabled: bool,
    #[serde(default = "default_failure_cooldown")]
    pub(super) drop_failure_cooldown_ms: u64,
    #[serde(default = "default_drop_delay")]
    pub(super) drop_delay: crate::bot_traffic_costs::Policy,
    #[serde(default = "default_traffic_query_budget")]
    pub(super) drop_traffic_query_budget: usize,
    #[serde(default = "default_congestion_headway")]
    pub(super) drop_congestion_headway_s: f64,
    #[serde(default = "default_entry_lane")]
    pub(super) drop_entry_lane_m: f64,
    #[serde(default = "default_egress_grid")]
    pub(super) drop_egress_grid_m: f64,
    #[serde(default = "default_holding_grid")]
    pub(super) drop_holding_grid_m: f64,
    #[serde(default = "default_holding_candidates")]
    pub(super) drop_holding_candidate_limit: usize,
    #[serde(default = "default_holding_stall")]
    pub(super) drop_holding_stall_ms: u64,
    #[serde(default = "default_entry_buffer")]
    pub(super) drop_entry_buffer_m: f64,
    #[serde(default = "default_queue_distance")]
    pub(super) drop_queue_distance_m: f64,
    #[serde(default = "default_clearance")]
    pub(super) drop_clearance_m: f64,
    #[serde(default = "default_preentry_stall")]
    pub(super) drop_preentry_stall_ms: u64,
    planning_slice_us: u64,
    pub(super) candidate_checks_per_tick: usize,
    pub(super) route_queries_per_tick: usize,
    fresh_route_share: usize,
    retry_ms: Vec<u64>,
    stall_ms: u64,
    reservation_spacing_m: f64,
    pub(super) arrival_m: f64,
    recovery_m: f64,
    ground_search_m: f64,
    cache_limit: usize,
    candidate_offsets_m: Vec<[f64; 2]>,
    paired_squad_lateral_m: [f64; 4],
    reinforced_squad_lateral_m: [f64; 4],
    attack_roles: Vec<DestinationRole>,
    defend_roles: Vec<DestinationRole>,
}
impl ObjectivePolicy {
    pub fn bundled() -> Self {
        serde_json::from_str(include_str!("../data/synthetic-bot-objective-policy.json"))
            .expect("validated bundled objective policy")
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        if !self.drop_delay.valid()
            || self.drop_delay.maximum_s < self.drop_congestion_headway_s
            || !(5000..=120000).contains(&self.drop_failure_cooldown_ms)
            || !(256..=65536).contains(&self.drop_traffic_query_budget)
            || !(1. ..=12.).contains(&self.drop_congestion_headway_s)
            || !(3. ..=12.).contains(&self.drop_entry_lane_m)
            || !(0.25..=1.).contains(&self.drop_egress_grid_m)
            || !(1.5..=4.).contains(&self.drop_holding_grid_m)
            || !(16..=256).contains(&self.drop_holding_candidate_limit)
            || !(5000..=30000).contains(&self.drop_holding_stall_ms)
            || !(0.5..=5.).contains(&self.drop_entry_buffer_m)
            || !(4. ..=32.).contains(&self.drop_queue_distance_m)
            || !(0.7..=4.).contains(&self.drop_clearance_m)
            || !(1000..=30000).contains(&self.drop_preentry_stall_ms)
            || !(100..=10000).contains(&self.planning_slice_us)
            || !(1..=32).contains(&self.candidate_checks_per_tick)
            || !(1..=4).contains(&self.route_queries_per_tick)
            || !(1..=8).contains(&self.fresh_route_share)
            || self.retry_ms.is_empty()
            || self.retry_ms.len() > 16
            || self.retry_ms.iter().any(|v| !(50..=30000).contains(v))
            || !(1000..=30000).contains(&self.stall_ms)
            || !(1.5..=8.).contains(&self.reservation_spacing_m)
            || !(0.1..=1.).contains(&self.arrival_m)
            || !(0.5..=4.).contains(&self.recovery_m)
            || !(0.5..=16.).contains(&self.ground_search_m)
            || !(128..=32768).contains(&self.cache_limit)
            || self
                .paired_squad_lateral_m
                .iter()
                .chain(self.reinforced_squad_lateral_m.iter())
                .any(|v| !v.is_finite() || v.abs() > 48.)
            || self.candidate_offsets_m.is_empty()
            || self.candidate_offsets_m.len() > 64
            || self
                .candidate_offsets_m
                .iter()
                .flatten()
                .any(|v| !v.is_finite() || v.abs() > 16.)
            || [&self.attack_roles, &self.defend_roles]
                .into_iter()
                .any(|roles| {
                    roles.len() != 8
                        || roles.iter().any(|r| {
                            r.name.is_empty()
                                || r.name.len() > 32
                                || !r.back_m.is_finite()
                                || !(0. ..=64.).contains(&r.back_m)
                                || !r.side_m.is_finite()
                                || r.side_m.abs() > 64.
                        })
                })
        {
            return Err("invalid_bot_objective_policy");
        }
        Ok(())
    }
}
#[derive(Default)]
struct DropCostGeometry {
    mesh: std::sync::Weak<navmesh::NavMesh>,
    at: Option<Duration>,
    revision: u64,
    entry_buffer: f64,
    signature: Option<u64>,
    bounds: BTreeMap<u32, crate::actor_collision::Bounds>,
    related: std::sync::Arc<crate::bot_traffic_costs::Relations>,
    rebuild_us: u64,
    quotes: u32,
}
#[derive(Default)]
pub(super) struct Director {
    failed_operators: BTreeMap<(u16, u16, u32), Duration>,
    cost_geometry: std::cell::RefCell<DropCostGeometry>,
    fresh_cursor: u16,
    retry_cursor: u16,
    fresh_streak: usize,
    cache_world: Option<String>,
    cache: BTreeMap<CandidateKey, Result<replication::CharacterPosition, &'static str>>,
    holding_cache: BTreeMap<(u32, [u32; 3]), Result<replication::CharacterPosition, &'static str>>,
}
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct CandidateKey {
    objective: u16,
    squad: u16,
    role: u8,
    candidate: usize,
    raw: [u32; 3],
}
#[derive(Default)]
pub(super) struct Task {
    pub(super) operator: bool,
    pub(super) objective: Option<u16>,
    pub(super) goal: Option<replication::CharacterPosition>,
    pub(super) role_goal: Option<replication::CharacterPosition>,
    generation: u32,
    pub(super) recovering: bool,
    continuing: bool,
    routed: bool,
    wide: bool,
    unavailable_reported: bool,
    progress_distance: Option<(usize, f64)>,
    progress_at: Duration,
    retry_at: Duration,
    attempts: u32,
    candidate_attempts: usize,
}
impl Task {
    fn finish_segment(&mut self, now: Duration) {
        self.goal = None;
        self.retry_at = now;
    }
}
fn eligible(state: replication::ObjectiveState) -> bool {
    use replication::ObjectiveState::*;
    !matches!(state, Inactive | Completed | Locked)
}
/// Initial authored lanes; after one prerequisite completes, reinforce the other.
/// C remains unavailable through its real five-second delayed activation.
fn assignment(
    catalog: &objective_catalog::ObjectiveCatalog,
    world: &objective_control::ObjectiveWorld,
    squad: u16,
) -> Option<u16> {
    if let Some(progression) = &catalog.progression {
        if world
            .state(progression.delayed_objective)
            .is_some_and(eligible)
        {
            return Some(progression.delayed_objective);
        }
        let live: Vec<_> = progression
            .prerequisites
            .iter()
            .copied()
            .filter(|id| world.state(*id).is_some_and(eligible))
            .collect();
        if live.len() == 1 {
            return live.first().copied();
        }
        if live.is_empty() {
            return None;
        }
    }
    catalog
        .dynamic_assignment(squad, |id| world.state(id))
        .ok()
        .filter(|id| *id != u16::MAX)
}
/// Supported shared actions only. In particular A/B defenders have no capture action.
pub(super) fn operator_action(
    object: &objective_catalog::Objective,
    state: replication::ObjectiveState,
    team: u8,
) -> Option<(protocol::ObjectiveInteractionKind, [f32; 3], f32)> {
    use protocol::ObjectiveInteractionKind::{Activate, Deactivate};
    use replication::ObjectiveState::{Activated, Activating, Deactivated, Deactivating};
    if let Some(r) = &object.demolition {
        let kind = if team != r.owner_team && matches!(state, Deactivated | Activating) {
            Activate
        } else if team == r.owner_team && r.can_defuse && matches!(state, Activated | Deactivating)
        {
            Deactivate
        } else {
            return None;
        };
        Some((kind, r.position, r.maximum_distance_m))
    } else {
        let r = object.interaction.as_ref()?;
        (team != r.owner_team && matches!(state, Deactivated | Activating)).then_some((
            Activate,
            r.position,
            r.maximum_distance_m,
        ))
    }
}
fn operator_safe(bot: &crate::bot_registry::WorldBot) -> bool {
    let r = bot.target.runtime.unwrap();
    bot.health > 0
        && r.physics_motion.is_some_and(|m| m.grounded)
        && !r.navmesh_state.on_player
        && !bot.drop_committed
        && bot.drop_waiting.is_none()
        && bot.drop_queue_passage.is_none()
        && bot.drop_permit.is_none()
        && bot.drop_holding_goal.is_none()
        && !bot.drop_resume
        && !bot.drop_egress
        && !bot.drop_retreat
        && !bot.drop_reroute
        && bot.traffic_yield.is_none()
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
    point(a)
        .iter()
        .zip(point(b))
        .map(|(a, b)| (a - b).powi(2))
        .sum::<f64>()
        .sqrt()
}
fn role_candidate(
    center: [f32; 3],
    anchor: replication::CharacterPosition,
    role: &DestinationRole,
    offset: [f64; 2],
) -> replication::CharacterPosition {
    let dx = f64::from(anchor.x - center[0]);
    let dz = f64::from(anchor.z - center[2]);
    let len = dx.hypot(dz);
    let (x, z) = if len > 0.01 {
        (dx / len, dz / len)
    } else {
        (1., 0.)
    };
    let back = role.back_m + offset[0];
    let side = role.side_m + offset[1];
    position([
        f64::from(center[0]) + x * back - z * side,
        f64::from(center[1]) + 0.5,
        f64::from(center[2]) + z * back + x * side,
    ])
}
fn supported(
    world: Option<&collision::CollisionWorld>,
    map: &str,
    raw: replication::CharacterPosition,
    policy: &ObjectivePolicy,
) -> Result<replication::CharacterPosition, &'static str> {
    supported_interval(
        world,
        map,
        raw,
        policy.ground_search_m,
        policy.ground_search_m,
    )
}
fn supported_operator(
    world: Option<&collision::CollisionWorld>,
    map: &str,
    raw: replication::CharacterPosition,
    center: [f32; 3],
    radius: f32,
) -> Result<replication::CharacterPosition, &'static str> {
    // Restrict support discovery to this trigger's vertical sphere slice.
    // A generic +8m ray can hit a roof above the action instead of its floor.
    let horizontal = f64::from(raw.x - center[0]).powi(2) + f64::from(raw.z - center[2]).powi(2);
    let vertical = (f64::from(radius).powi(2) - horizontal).max(0.).sqrt();
    let up = (f64::from(center[1]) + vertical - f64::from(raw.y)).max(0.);
    let down = (f64::from(raw.y) - f64::from(center[1]) + vertical).max(0.);
    supported_interval(world, map, raw, up, down)
}
fn supported_interval(
    world: Option<&collision::CollisionWorld>,
    map: &str,
    raw: replication::CharacterPosition,
    up: f64,
    down: f64,
) -> Result<replication::CharacterPosition, &'static str> {
    let terrain = bot_physics::WorldTerrain::new(world, map).map_err(|e| e.code())?;
    let ground = terrain
        .sample_ground(point(raw), up, down)
        .map_err(|e| e.code())?
        .ok_or("bot_objective_unsupported")?;
    let p = [
        ground.position[0],
        ground.position[1] + 0.5,
        ground.position[2],
    ];
    let body = bot_physics::Body::default();
    terrain
        .require_horizontal_coverage(p, p, body)
        .map_err(|e| e.code())?;
    let m = terrain
        .advance(
            terrain.start_motion(p, body).map_err(|e| e.code())?,
            [0.; 3],
            1.,
            body,
        )
        .map_err(|e| e.code())?;
    if !m.grounded || m.blocked {
        return Err("bot_objective_unsupported");
    }
    Ok(position(m.position))
}
impl Director {
    fn candidate_support(
        &mut self,
        key: CandidateKey,
        limit: usize,
        sample: impl FnOnce() -> Result<replication::CharacterPosition, &'static str>,
    ) -> (Result<replication::CharacterPosition, &'static str>, bool) {
        if let Some(value) = self.cache.get(&key) {
            return (*value, true);
        }
        let value = sample();
        if self.cache.len() >= limit {
            self.cache.pop_first();
        }
        self.cache.insert(key, value);
        (value, false)
    }
    fn choose_ready(
        &mut self,
        fresh: &[u16],
        retry: &[u16],
        policy: &ObjectivePolicy,
    ) -> Option<u16> {
        // Only eligible work participates. A backing-off/unreachable actor is
        // not a global barrier to physical recovery or optional reconsideration.
        self.choose(fresh, if fresh.is_empty() { retry } else { &[] }, policy)
    }
    fn choose(&mut self, fresh: &[u16], retry: &[u16], policy: &ObjectivePolicy) -> Option<u16> {
        let use_fresh =
            !fresh.is_empty() && (retry.is_empty() || self.fresh_streak < policy.fresh_route_share);
        let (ids, cursor) = if use_fresh {
            self.fresh_streak += 1;
            (fresh, &mut self.fresh_cursor)
        } else {
            self.fresh_streak = 0;
            (retry, &mut self.retry_cursor)
        };
        let id = ids
            .iter()
            .copied()
            .find(|id| *id > *cursor)
            .or_else(|| ids.first().copied())?;
        *cursor = id;
        Some(id)
    }
}
fn holding_offset(index: usize, grid: f64) -> [f64; 2] {
    ring_offset(index, grid, 2)
}
fn ring_offset(mut index: usize, grid: f64, first: i32) -> [f64; 2] {
    for radius in first..=32i32 {
        let mut ring = Vec::new();
        for x in -radius..=radius {
            for z in -radius..=radius {
                if x.abs().max(z.abs()) == radius {
                    ring.push([f64::from(x) * grid, f64::from(z) * grid]);
                }
            }
        }
        if index < ring.len() {
            return ring[index];
        }
        index -= ring.len();
    }
    [48., 48.]
}
fn boxes_intersect(a: crate::actor_collision::Bounds, b: crate::actor_collision::Bounds) -> bool {
    (0..3).all(|i| a.min[i] <= b.max[i] && a.max[i] >= b.min[i])
}
impl<W: Write> Engine<W> {
    pub(super) fn drop_cost_quotes(
        &self,
        id: u16,
        now: Duration,
        policy: &ObjectivePolicy,
    ) -> BTreeMap<u32, crate::bot_traffic_costs::Cost> {
        let reservations = self
            .bot_registry
            .reserved_passages(id, policy.drop_entry_buffer_m);
        let Some(mesh) = &self.navmesh_world else {
            return BTreeMap::new();
        };
        let (related, bounds) = {
            use std::hash::{Hash, Hasher};
            let mut fingerprint = std::collections::hash_map::DefaultHasher::new();
            for (&actor, b) in &self.bot_registry.bots {
                if b.health > 0
                    && let Some(p) = b.drop_queue_passage
                {
                    actor.hash(&mut fingerprint);
                    p.id.hash(&mut fingerprint);
                    b.drop_queue_approach.len().hash(&mut fingerprint);
                    for coordinate in b.drop_queue_approach.iter().flatten().flatten() {
                        coordinate.to_bits().hash(&mut fingerprint);
                    }
                }
            }
            let signature = fingerprint.finish();
            let mut cache = self.bot_registry.director.cost_geometry.borrow_mut();
            if cache.entry_buffer != policy.drop_entry_buffer_m
                || !cache
                    .mesh
                    .upgrade()
                    .is_some_and(|old| std::sync::Arc::ptr_eq(&old, mesh))
            {
                cache.mesh = std::sync::Arc::downgrade(mesh);
                cache.entry_buffer = policy.drop_entry_buffer_m;
                cache.at = None;
                cache.signature = None;
                cache.bounds = mesh
                    .artifact()
                    .drop_links
                    .iter()
                    .map(|link| {
                        (
                            link.id,
                            development_bot_movement::protected_drop_bounds(
                                link,
                                policy.drop_entry_buffer_m,
                            ),
                        )
                    })
                    .collect();
            }
            if cache.at != Some(now) {
                cache.at = Some(now);
                cache.rebuild_us = 0;
                cache.quotes = 0;
            }
            if cache.signature != Some(signature)
                || cache.revision != self.bot_registry.delay_book.revision
            {
                let began = Instant::now();
                let geometry: Vec<_> = mesh
                    .artifact()
                    .drop_links
                    .iter()
                    .map(|link| {
                        let mut approaches: Vec<_> = self
                            .bot_registry
                            .delay_book
                            .approaches
                            .get(&link.id)
                            .cloned()
                            .into_iter()
                            .collect();
                        approaches.extend(
                            self.bot_registry
                                .bots
                                .values()
                                .filter(|b| {
                                    b.health > 0
                                        && b.drop_queue_passage.is_some_and(|p| p.id == link.id)
                                        && !b.drop_queue_approach.is_empty()
                                })
                                .map(|b| b.drop_queue_approach.clone()),
                        );
                        crate::bot_traffic_costs::Passage {
                            id: link.id,
                            protected: cache.bounds[&link.id],
                            approaches,
                            slots: 0,
                            owner_remaining: None,
                            estimate: self.bot_registry.delay_book.estimate(
                                link.id,
                                now,
                                policy.drop_congestion_headway_s,
                                &policy.drop_delay,
                            ),
                            cooldown_s: 0.,
                        }
                    })
                    .collect();
                let mut budget = policy.drop_traffic_query_budget;
                cache.related = std::sync::Arc::new(crate::bot_traffic_costs::relations(
                    &geometry,
                    &mut budget,
                ));
                cache.signature = Some(signature);
                cache.revision = self.bot_registry.delay_book.revision;
                cache.rebuild_us += began.elapsed().as_micros() as u64;
            }
            cache.quotes += 1;
            (cache.related.clone(), cache.bounds.clone())
        };
        // Reservations, owner progress and decaying estimates stay current for
        // every actor, including routes assigned earlier in the same batch.
        let passages: Vec<_> = mesh
            .artifact()
            .drop_links
            .iter()
            .map(|link| {
                let estimate = self.bot_registry.delay_book.estimate(
                    link.id,
                    now,
                    policy.drop_congestion_headway_s,
                    &policy.drop_delay,
                );
                crate::bot_traffic_costs::Passage {
                    id: link.id,
                    protected: bounds[&link.id],
                    approaches: Vec::new(),
                    slots: reservations.iter().filter(|(_, p)| *p == link.id).count(),
                    owner_remaining: self.bot_registry.owner_remaining(
                        link.id,
                        id,
                        now,
                        estimate.service_s,
                        policy.drop_delay.maximum_s,
                    ),
                    estimate,
                    cooldown_s: self
                        .bot_registry
                        .failed_passages
                        .get(&link.id)
                        .map_or(0., |until| until.saturating_sub(now).as_secs_f64()),
                }
            })
            .collect();
        crate::bot_traffic_costs::costs_with_relations(&passages, &related)
    }
    fn drop_route_costs(
        &self,
        id: u16,
        now: Duration,
        policy: &ObjectivePolicy,
    ) -> (BTreeMap<u32, f64>, BTreeMap<u32, f64>) {
        let quotes = self.drop_cost_quotes(id, now, policy);
        let demand = quotes
            .iter()
            .map(|(&id, q)| (id, q.queued_service_s))
            .collect();
        let costs = quotes
            .iter()
            .map(|(&id, q)| {
                (
                    id,
                    (q.total_s * retail_movement::WALK_FORWARD_MPS).min(1024.),
                )
            })
            .collect();
        (demand, costs)
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn plan_drop_reconsideration(
        &mut self,
        id: u16,
        now: Duration,
        policy: &ObjectivePolicy,
        started: &Instant,
        queries: &mut usize,
        receipts: &mut Vec<Value>,
    ) -> Result<(), &'static str> {
        let b = &self.bot_registry.bots[&id];
        if !b.can_reconsider(policy.drop_entry_buffer_m, policy.arrival_m) {
            self.bot_registry.bots.get_mut(&id).unwrap().reconsider = None;
            return Ok(());
        }
        let passage = b
            .planned_drop(policy.drop_entry_buffer_m)
            .or(b.drop_queue_passage)
            .unwrap();
        let goal = b.task.role_goal.unwrap();
        let r = b.target.runtime.unwrap();
        let key = crate::bot_traffic_costs::Key {
            generation: r.generation,
            objective: b.task.objective,
            role_slot: b.role_slot,
            passage: passage.id,
        };
        let origin = r.physical_position();
        let quotes = self.drop_cost_quotes(id, now, policy);
        let costs: BTreeMap<_, _> = quotes
            .iter()
            .map(|(&id, c)| {
                (
                    id,
                    (c.total_s * retail_movement::WALK_FORWARD_MPS).min(1024.),
                )
            })
            .collect();
        if costs.get(&passage.id).copied().unwrap_or(0.)
            < policy.drop_delay.minimum_gain_s * retail_movement::WALK_FORWARD_MPS
        {
            let b = self.bot_registry.bots.get_mut(&id).unwrap();
            b.reconsider = None;
            b.reconsider_at =
                crate::bot_traffic_costs::next_reconsideration(now, id, &policy.drop_delay);
            return Ok(());
        }
        if started.elapsed().as_micros() >= u128::from(policy.planning_slice_us) {
            return Ok(());
        }
        let valid_quote = b.reconsider.as_ref().is_some_and(|q| {
            q.key == key
                && q.goal == goal
                && (0..3)
                    .map(|axis| (q.origin[axis] - origin[axis]).powi(2))
                    .sum::<f64>()
                    <= 0.25f64.powi(2)
        });
        let mut baseline = valid_quote.then(|| b.reconsider.as_ref().unwrap().baseline.clone());
        if baseline.is_none()
            && let Some(route) = b
                .target
                .registry_route()
                .filter(|route| route.waypoint_drop_ids.contains(&Some(passage.id)))
        {
            // This is already a real role route. Trim its consumed prefix rather
            // than spending another route query merely to price the current plan.
            let mut current = route.clone();
            let cursor = r.navigation_waypoint.min(route.waypoints.len());
            current.waypoints = std::iter::once(origin)
                .chain(route.waypoints[cursor..].iter().copied())
                .collect();
            current.waypoint_kinds = std::iter::once(navmesh::WaypointKind::Walk)
                .chain(route.waypoint_kinds[cursor..].iter().copied())
                .collect();
            current.waypoint_drop_ids = std::iter::once(None)
                .chain(route.waypoint_drop_ids[cursor..].iter().copied())
                .collect();
            baseline = Some(current);
        }
        // A holding route no longer contains the original role path. Quote its
        // required exit in one quota tick, then query alternatives on a later tick.
        let baseline_query = baseline.is_none();
        let sprint = self.bot_registry.config.as_ref().unwrap().slots[b.slot].sprint;
        let map = self.map_name().to_owned();
        let b = self.bot_registry.bots.get_mut(&id).unwrap();
        b.reconsider = None;
        let old_pending = b.target.pending_move.take();
        let old_runtime = b.target.runtime;
        *queries += 1;
        let result = b.target.begin_registry_move_with_drop_costs(
            self.navmesh_world.clone().unwrap(),
            &map,
            BotMove {
                mode: BotMoveMode::Navmesh,
                destination: goal,
                duration_ms: 120000,
                update_ms: 250,
                yaw: None,
                sprint,
            },
            now,
            b.health,
            self.fall_damage_rules.as_ref(),
            Some(0.3),
            &costs,
            baseline_query.then_some(passage.id),
        );
        b.reconsider_at =
            crate::bot_traffic_costs::next_reconsideration(now, id, &policy.drop_delay);
        if let Err(reason) = result {
            b.target.pending_move = old_pending;
            b.target.runtime = old_runtime;
            receipts.push(json!({"op":"bot_drop_reconsider","timestamp_ms":now.as_millis()as u64,"object_index":id,"drop_id":passage.id,"action":"query_failed","reason":reason}));
            return Ok(());
        }
        if baseline_query {
            let route = b.target.registry_route().unwrap().clone();
            b.target.pending_move = old_pending;
            b.target.runtime = old_runtime;
            b.reconsider = Some(crate::bot_traffic_costs::Reconsideration {
                key,
                goal,
                origin,
                baseline: route,
            });
            b.reconsider_at = now;
            receipts.push(json!({"op":"bot_drop_reconsider","timestamp_ms":now.as_millis()as u64,"object_index":id,"drop_id":passage.id,"action":"baseline_quoted"}));
            return Ok(());
        }
        let baseline = baseline.unwrap();
        let candidate = b.target.registry_route().unwrap();
        let old_s = crate::bot_traffic_costs::route_seconds(&baseline, 0, origin, &costs);
        let new_s = crate::bot_traffic_costs::route_seconds(candidate, 0, origin, &costs);
        let old_hp = baseline.predicted_remaining_health;
        let new_hp = candidate.predicted_remaining_health;
        let next = b
            .target
            .next_registry_drop(policy.drop_entry_buffer_m)
            .map(|p| p.id);
        let recent_until = next
            .and_then(|id| b.reconsider_history.get(&id))
            .copied()
            .filter(|until| *until > now);
        let accepted = recent_until.is_none()
            && crate::bot_traffic_costs::worthwhile(
                old_s,
                new_s,
                old_hp,
                new_hp,
                passage.id,
                next,
                &policy.drop_delay,
            );
        if accepted {
            b.reconsider_history.retain(|_, until| *until > now);
            if b.reconsider_history.len() >= policy.drop_delay.reversal_history_limit
                && let Some(oldest) = b
                    .reconsider_history
                    .iter()
                    .min_by_key(|(_, until)| **until)
                    .map(|(&id, _)| id)
            {
                b.reconsider_history.remove(&oldest);
            }
            b.reconsider_history.insert(
                passage.id,
                now + Duration::from_millis(policy.drop_delay.reversal_ms),
            );
            // Relinquish the old uncommitted ticket. The admission pass removes
            // that life from its obsolete queue before the next physical step.
            b.drop_queue_passage = None;
            b.drop_queue_since = None;
            b.drop_waiting = None;
            b.drop_holding_goal = None;
            b.drop_queue_approach.clear();
            b.drop_approach_ready = false;
            b.drop_resume = false;
            b.drop_reroute = false;
            b.drop_resume_failure_at = None;
            b.delay_episode = None;
            b.task.goal = Some(b.target.registry_move_destination().unwrap_or(goal));
            b.task.recovering = false;
            b.task.progress_at = now;
            b.task.progress_distance = b.target.registry_route_progress();
            b.task.retry_at = now + Duration::from_millis(policy.stall_ms);
            b.bind_delay_episode(now, policy.drop_entry_buffer_m);
        } else {
            b.target.pending_move = old_pending;
            b.target.runtime = old_runtime;
            if let Some(until) = recent_until {
                b.reconsider_at = b.reconsider_at.max(until);
            }
        }
        receipts.push(json!({"op":"bot_drop_reconsider","timestamp_ms":now.as_millis()as u64,"object_index":id,"generation":r.generation,"drop_id":passage.id,"candidate_drop_id":next,"action":if accepted{"switched"}else{"retained"},"old_expected_s":old_s,"candidate_expected_s":new_s,"expected_gain_s":old_s-new_s,"recent_route_blocked":recent_until.is_some(),"minimum_gain_s":policy.drop_delay.minimum_gain_s,"minimum_gain_ratio":policy.drop_delay.minimum_gain_ratio,"old_remaining_hp":old_hp,"candidate_remaining_hp":new_hp,"forecast":quotes}));
        Ok(())
    }
    /// Actual stalled contacts create bounded local access requests. Supported
    /// enroute bots use stable physical-stall priority; humans and passage
    /// ownership remain untouched.
    pub(super) fn tick_local_traffic(
        &mut self,
        now: Duration,
        policy: &ObjectivePolicy,
        actors: &crate::actor_collision::ActorGrid,
        receipts: &mut Vec<Value>,
    ) -> Result<(), &'static str> {
        for b in self.bot_registry.bots.values_mut() {
            let r = b.target.runtime.unwrap();
            if b.health == 0 {
                b.traffic_progress = None;
                b.holding_access = None;
                continue;
            }
            if b.traffic_progress.as_ref().is_none_or(|p| {
                p.generation != r.generation
                    || p.objective != b.task.objective
                    || p.role_slot != b.role_slot
                    || distance(p.position, r.position) >= 0.5
            }) {
                b.traffic_progress = Some(crate::bot_traffic_flow::Progress {
                    generation: r.generation,
                    objective: b.task.objective,
                    role_slot: b.role_slot,
                    position: r.position,
                    at: now,
                });
            }
        }
        let expired_access: Vec<_> = self
            .bot_registry
            .bots
            .iter()
            .filter_map(|(&id, b)| {
                let a = b.holding_access.as_ref()?;
                let obsolete = self
                    .bot_registry
                    .bots
                    .get(&a.requester)
                    .is_none_or(|source| {
                        let r = source.target.runtime.unwrap();
                        source.health == 0
                            || r.generation != a.generation
                            || r.since != a.source_since
                            || !matches!(r.phase, BotPhase::Relocating)
                            || source.drop_waiting.is_none()
                            || !crate::bot_traffic_flow::in_lane(
                                r.physical_position(),
                                &a.lane,
                                policy.reservation_spacing_m,
                            )
                    });
                (obsolete || now >= a.until).then_some(id)
            })
            .collect();
        for id in expired_access {
            self.bot_registry.bots.get_mut(&id).unwrap().holding_access = None;
        }
        let releases: Vec<_> = self
            .bot_registry
            .bots
            .iter()
            .filter_map(|(&id, b)| {
                let y = b.traffic_yield.as_ref()?;
                if y.resume {
                    return None;
                }
                let clear =
                    self.bot_registry
                        .bots
                        .get(&y.requester)
                        .is_none_or(|source| {
                            let r = source.target.runtime.unwrap();
                            source.health == 0
                                || r.generation != y.generation
                                || source.task.objective != y.objective
                                || source.role_slot != y.role_slot
                                || !crate::bot_traffic_flow::in_lane(
                                    r.physical_position(),
                                    &y.lane,
                                    policy.reservation_spacing_m,
                                )
                                || (!source.task.recovering
                                    && source.task.role_goal.is_some_and(|g| {
                                        distance(g, r.position) <= policy.arrival_m
                                    }))
                        });
                (clear
                    || now.saturating_sub(y.started)
                        >= Duration::from_millis(policy.drop_holding_stall_ms))
                .then_some(id)
            })
            .collect();
        for id in releases {
            self.bot_registry
                .bots
                .get_mut(&id)
                .unwrap()
                .traffic_yield
                .as_mut()
                .unwrap()
                .resume = true;
        }
        let mut active_requesters: std::collections::BTreeSet<_> = self
            .bot_registry
            .bots
            .values()
            .filter_map(|b| b.traffic_yield.as_ref().map(|y| y.requester))
            .collect();
        let mut sources: Vec<_> = self
            .bot_registry
            .bots
            .iter()
            .filter_map(|(&id, b)| {
                let r = b.target.runtime.unwrap();
                (b.health > 0
                    && b.traffic_yield.is_none()
                    && (b.drop_waiting.is_none() || b.drop_holding_goal.is_some())
                    && matches!(r.phase, BotPhase::Relocating)
                    && !r.navmesh_state.on_player
                    && r.physics_motion.is_some_and(|m| m.grounded)
                    && now.saturating_sub(b.traffic_progress.as_ref().map_or(now, |p| p.at))
                        >= Duration::from_millis(policy.stall_ms))
                .then_some(id)
            })
            .collect();
        sources.sort_by_key(|id| (*id <= self.bot_registry.traffic_cursor, *id));
        let limit = policy.candidate_checks_per_tick.min(sources.len());
        for id in sources.into_iter().take(limit) {
            self.bot_registry.traffic_cursor = id;
            let source = &self.bot_registry.bots[&id];
            // The source list predates this loop's requests. A newly selected
            // yielder must not issue a reciprocal request from that snapshot.
            if source.traffic_yield.is_some() {
                continue;
            }
            let source_priority = (source.traffic_progress.as_ref().unwrap().at, id);
            let r = source.target.runtime.unwrap();
            let Some(route) = source.target.registry_route() else {
                continue;
            };
            let lane = crate::bot_traffic_flow::local_prefix(
                r.physical_position(),
                route,
                r.navigation_waypoint,
                policy.drop_entry_lane_m,
            );
            let requester_generation = r.generation;
            let requester_objective = source.task.objective;
            let requester_role = source.role_slot;
            let source_since = r.since;
            let source_queued_at = source.drop_queue_since;
            let source_in_admission = self.bot_registry.traffic_lanes(id).iter().any(|lane| {
                crate::bot_traffic_flow::in_lane(
                    r.physical_position(),
                    lane,
                    policy.reservation_spacing_m,
                )
            });
            let mut budget = policy.drop_traffic_query_budget / limit.max(1);
            let Ok(Some(blocker)) =
                crate::bot_traffic_flow::approach_blocker(actors, id, &lane, &mut budget)
            else {
                continue;
            };
            let Some(b) = self.bot_registry.bots.get_mut(&blocker) else {
                continue;
            };
            let r = b.target.runtime.unwrap();
            if source_queued_at.is_some()
                && b.idle_queued_waiter(policy.arrival_m)
                && (source_queued_at <= b.drop_queue_since || source_in_admission)
            {
                b.holding_access = Some(crate::bot_traffic_flow::HoldingAccess {
                    requester: id,
                    generation: requester_generation,
                    source_since,
                    lane,
                    until: now + Duration::from_millis(policy.drop_holding_stall_ms),
                });
                b.drop_holding_goal = None;
                b.drop_approach_ready = false;
                b.reconsider = None;
                b.target.runtime.as_mut().unwrap().phase = BotPhase::Complete;
                receipts.push(json!({"op":"bot_traffic_yield","timestamp_ms":now.as_millis()as u64,"object_index":blocker,"requester":id,"action":"queued_access_restage","reason":"queued_waiter_blocks_holding_access","source_queue_since_ms":source_queued_at.map(|t|t.as_millis()as u64),"blocker_queue_since_ms":b.drop_queue_since.map(|t|t.as_millis()as u64),"clears_admission":source_in_admission}));
                continue;
            }
            if b.health == 0
                || b.traffic_yield.is_some()
                || b.drop_waiting.is_some()
                || b.drop_queue_passage.is_some()
                || b.drop_committed
                || b.drop_permit.is_some()
                || active_requesters.contains(&blocker)
                || r.navmesh_state.on_player
                || !r.physics_motion.is_some_and(|m| m.grounded)
            {
                continue;
            }
            let idle = matches!(r.phase, BotPhase::Complete)
                && !b.task.recovering
                && b.task
                    .goal
                    .is_some_and(|g| distance(g, r.position) <= policy.arrival_m);
            let stalled_enroute = b.task.role_goal.is_some()
                && b.task.goal.is_some()
                && b.traffic_progress.as_ref().is_some_and(|p| {
                    now.saturating_sub(p.at) >= Duration::from_millis(policy.stall_ms)
                        && source_priority < (p.at, blocker)
                })
                && !self
                    .navmesh_world
                    .as_ref()
                    .unwrap()
                    .artifact()
                    .drop_links
                    .iter()
                    .any(|link| {
                        boxes_intersect(
                            crate::actor_collision::Bounds::body(
                                r.physical_position(),
                                Default::default(),
                            ),
                            development_bot_movement::protected_drop_bounds(
                                link,
                                policy.drop_entry_buffer_m,
                            ),
                        )
                    });
            if !idle && !stalled_enroute {
                continue;
            }
            if !idle {
                // Freeze only the selected supported ground job. Actual Motion,
                // HP, stamina and real role remain unchanged until safe staging.
                b.target.pending_move = None;
                b.target.runtime.as_mut().unwrap().phase = BotPhase::Complete;
                b.reconsider = None;
            }
            b.traffic_yield = Some(crate::bot_traffic_flow::Yield {
                requester: id,
                generation: requester_generation,
                objective: requester_objective,
                role_slot: requester_role,
                lane,
                started: now,
                goal: None,
                attempt: 0,
                retry_at: now,
                resume: false,
            });
            active_requesters.insert(id);
            receipts.push(json!({"op":"bot_traffic_yield","timestamp_ms":now.as_millis()as u64,"object_index":blocker,"requester":id,"action":"requested","reason":if idle{"idle_role_blocks_local_access"}else{"stalled_enroute_blocks_local_access"},"physical_stall_since_ms":b.traffic_progress.as_ref().map(|p|p.at.as_millis()as u64),"requester_stall_since_ms":source_priority.0.as_millis()as u64}));
        }
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn plan_traffic_yield(
        &mut self,
        id: u16,
        now: Duration,
        policy: &ObjectivePolicy,
        actors: &crate::actor_collision::ActorGrid,
        started: &Instant,
        queries: &mut usize,
        receipts: &mut Vec<Value>,
    ) -> Result<(), &'static str> {
        let b = &self.bot_registry.bots[&id];
        let y = b.traffic_yield.as_ref().unwrap();
        let resume = y.resume;
        let requester = y.requester;
        let goal = if resume {
            b.task
                .role_goal
                .or(b.task.goal)
                .ok_or("traffic_yield_role_invalidated")
        } else {
            let p = b.target.runtime.unwrap().physical_position();
            // Search nearby body-clearance pockets first. Keep a bounded
            // cursor across failed episodes instead of retesting the same
            // prefix whenever the request expires.
            let index = b.traffic_yield_cursor % policy.drop_holding_candidate_limit;
            let near = policy.drop_holding_candidate_limit / 2;
            let offset = if index < near {
                ring_offset(index, crate::bot_physics::Body::default().radius, 2)
            } else {
                holding_offset(index - near, policy.drop_holding_grid_m)
            };
            let raw = position([p[0] + offset[0], p[1] + 0.5, p[2] + offset[1]]);
            let key = CandidateKey {
                objective: b.task.objective.unwrap_or(u16::MAX),
                squad: b.target.squad.squad_index(),
                role: b.role_slot,
                candidate: index,
                raw: [raw.x.to_bits(), raw.y.to_bits(), raw.z.to_bits()],
            };
            let map = self.map_name().to_owned();
            self.bot_registry
                .director
                .candidate_support(key, policy.cache_limit, || {
                    supported(self.collision_world.as_ref(), &map, raw, policy)
                })
                .0
        };
        let already_home = goal.as_ref().is_ok_and(|g| {
            distance(
                *g,
                self.bot_registry.bots[&id].target.runtime.unwrap().position,
            ) <= policy.arrival_m
        });
        if resume && (goal.is_err() || already_home) {
            let b = self.bot_registry.bots.get_mut(&id).unwrap();
            b.traffic_yield = None;
            b.target.pending_move = None;
            b.target.runtime.as_mut().unwrap().phase = BotPhase::Complete;
            b.task.retry_at = now;
            receipts.push(json!({"op":"bot_traffic_yield","timestamp_ms":now.as_millis()as u64,"object_index":id,"requester":requester,"action":"released","reason":if already_home{"already_at_role"}else{"role_invalidated"}}));
            return Ok(());
        }
        let result = (|| {
            let goal = goal?;
            if !resume {
                if self.bot_registry.bots[&id]
                    .task
                    .goal
                    .is_some_and(|original| {
                        distance(original, goal)
                            < policy
                                .arrival_m
                                .max(2. * crate::bot_physics::Body::default().radius)
                    })
                {
                    return Err("traffic_yield_no_displacement");
                }
                // A temporary pad must clear the requester's continuation,
                // not merely sit beyond its short protected prefix. Use the
                // physical upright boxes, without widening the collider.
                let continuation = self
                    .bot_registry
                    .bots
                    .get(&requester)
                    .and_then(|source| {
                        let r = source.target.runtime.unwrap();
                        source.target.registry_route().map(|route| {
                            crate::bot_traffic_flow::local_prefix(
                                r.physical_position(),
                                route,
                                r.navigation_waypoint,
                                3. * policy.drop_entry_lane_m,
                            )
                        })
                    })
                    .unwrap_or_else(|| {
                        self.bot_registry.bots[&id]
                            .traffic_yield
                            .as_ref()
                            .unwrap()
                            .lane
                            .clone()
                    });
                let mut pad = crate::actor_collision::ActorGrid::default();
                pad.insert_reserved(
                    id,
                    crate::actor_collision::Bounds::body(point(goal), Default::default()),
                )?;
                let mut budget = policy.drop_traffic_query_budget;
                if crate::bot_traffic_flow::approach_blocker(
                    &pad,
                    requester,
                    &continuation,
                    &mut budget,
                )?
                .is_some()
                {
                    return Err("traffic_yield_blocks_continuation");
                }
                if self
                    .bot_registry
                    .traffic_lanes_for_yield(id)
                    .iter()
                    .any(|lane| {
                        crate::bot_traffic_flow::in_lane(
                            point(goal),
                            lane,
                            policy.reservation_spacing_m,
                        )
                    })
                    || self
                        .navmesh_world
                        .as_ref()
                        .unwrap()
                        .artifact()
                        .drop_links
                        .iter()
                        .any(|link| {
                            boxes_intersect(
                                crate::actor_collision::Bounds::body(
                                    point(goal),
                                    Default::default(),
                                ),
                                development_bot_movement::protected_drop_bounds(
                                    link,
                                    policy.drop_entry_buffer_m,
                                ),
                            )
                        })
                {
                    return Err("traffic_yield_protected");
                }
                if actors.occupied(id, point(goal), Default::default())?
                    || self.bot_registry.bots.iter().any(|(&other, b)| {
                        other != id
                            && b.health > 0
                            && b.task
                                .goal
                                .into_iter()
                                .chain(b.drop_holding_goal)
                                .chain(b.traffic_yield.as_ref().and_then(|y| y.goal))
                                .any(|g| {
                                    boxes_intersect(
                                        crate::actor_collision::Bounds::body(
                                            point(g),
                                            Default::default(),
                                        ),
                                        crate::actor_collision::Bounds::body(
                                            point(goal),
                                            Default::default(),
                                        ),
                                    )
                                })
                    })
                {
                    return Err("traffic_yield_occupied");
                }
            }
            if started.elapsed().as_micros() >= u128::from(policy.planning_slice_us) {
                return Err("bot_planning_slice_yield");
            }
            *queries += 1;
            let map = self.map_name().to_owned();
            let b = self.bot_registry.bots.get_mut(&id).unwrap();
            let old_pending = b.target.pending_move.take();
            let old_runtime = b.target.runtime;
            let result = b.target.begin_registry_move_with_drop_costs(
                self.navmesh_world.clone().unwrap(),
                &map,
                BotMove {
                    mode: BotMoveMode::Navmesh,
                    destination: goal,
                    duration_ms: 120000,
                    update_ms: 250,
                    yaw: None,
                    sprint: false,
                },
                now,
                b.health,
                self.fall_damage_rules.as_ref(),
                Some(0.3),
                &BTreeMap::new(),
                None,
            );
            let rejection = result.err().or_else(|| {
                if resume {
                    return None;
                }
                let Some(route) = b.target.registry_route() else {
                    return Some("traffic_yield_route_missing");
                };
                if !route.waypoint_drop_ids.iter().all(Option::is_none) {
                    return Some("traffic_yield_route_requires_drop");
                }
                let local_limit = 2. * policy.drop_entry_lane_m;
                if route.length_m > local_limit {
                    return Some("traffic_yield_route_too_long");
                }
                let lane = crate::bot_traffic_flow::local_prefix(
                    b.target.runtime.unwrap().physical_position(),
                    route,
                    0,
                    local_limit,
                );
                let mut budget = policy.drop_traffic_query_budget;
                match crate::bot_traffic_flow::approach_blocker(actors, id, &lane, &mut budget) {
                    Ok(None) => None,
                    Ok(Some(_)) => Some("traffic_yield_route_occupied"),
                    Err(e) => Some(e),
                }
            });
            if let Some(reason) = rejection {
                b.target.pending_move = old_pending;
                b.target.runtime = old_runtime;
                return Err(reason);
            }
            if resume {
                b.task.goal = Some(b.target.registry_move_destination().unwrap_or(goal));
                b.task.recovering = false;
                b.task.progress_at = now;
                b.task.progress_distance = b.target.registry_route_progress();
                b.traffic_yield = None;
            } else {
                b.traffic_yield.as_mut().unwrap().goal = Some(goal);
            }
            Ok(goal)
        })();
        if resume && result.is_err() && !matches!(result, Err("bot_planning_slice_yield")) {
            // The role remains the goal. A failed return cannot occupy the
            // shared planner indefinitely; use ordinary supported recovery.
            let b = self.bot_registry.bots.get_mut(&id).unwrap();
            b.traffic_yield = None;
            b.target.pending_move = None;
            b.target.runtime.as_mut().unwrap().phase = BotPhase::Complete;
            b.task.retry_at = now;
            b.task.progress_at = now.saturating_sub(Duration::from_millis(policy.stall_ms));
        }
        if !resume && !matches!(result, Err("bot_planning_slice_yield")) {
            let b = self.bot_registry.bots.get_mut(&id).unwrap();
            b.traffic_yield_cursor =
                (b.traffic_yield_cursor + 1) % policy.drop_holding_candidate_limit;
        }
        if !matches!(result, Err("bot_planning_slice_yield"))
            && let Some(y) = self
                .bot_registry
                .bots
                .get_mut(&id)
                .unwrap()
                .traffic_yield
                .as_mut()
        {
            y.attempt += 1;
            y.retry_at = now + Duration::from_millis(policy.retry_ms[0]);
            if y.attempt >= policy.drop_holding_candidate_limit {
                y.resume = true;
            }
        }
        receipts.push(json!({"op":"bot_traffic_yield","timestamp_ms":now.as_millis()as u64,"object_index":id,"requester":requester,"action":if resume{"resume"}else{"stage"},"result":result}));
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn plan_drop_holding(
        &mut self,
        id: u16,
        now: Duration,
        policy: &ObjectivePolicy,
        actors: &crate::actor_collision::ActorGrid,
        started: &Instant,
        queries: &mut usize,
        receipts: &mut Vec<Value>,
    ) -> Result<(), &'static str> {
        let b = &self.bot_registry.bots[&id];
        let Some(passage) = b.drop_queue_passage else {
            return Ok(());
        };
        let egress = b.drop_egress;
        let retreat = b.drop_retreat;
        let preentry_retreat = b.drop_retreat_preentry;
        let reroute = b.drop_reroute;
        let physical_exit = egress || retreat;
        let prepare = b.drop_waiting.is_some()
            && !b.drop_approach_ready
            && b.drop_holding_goal.is_some_and(|goal| {
                distance(goal, b.target.runtime.unwrap().position) <= policy.arrival_m
            })
            && matches!(b.target.runtime.unwrap().phase, BotPhase::Complete);
        let resume = b.drop_resume || physical_exit || reroute || prepare;
        let local_rejoin = !resume
            && b.target.runtime.unwrap().navmesh_state.navigation_aborted
            && self.navmesh_world.as_ref().is_some_and(|mesh| {
                mesh.component_id(b.target.runtime.unwrap().physical_position())
                    .is_err()
            });
        let lanes = self.bot_registry.traffic_lanes(id);
        let protected = self.bot_registry.traffic_protected(id);
        if resume
            && !physical_exit
            && !prepare
            && b.task.goal.is_none()
            && b.task.role_goal.is_none()
        {
            let b = self.bot_registry.bots.get_mut(&id).unwrap();
            b.drop_resume = false;
            b.drop_holding_goal = None;
            if reroute {
                b.drop_reroute = false;
                b.drop_queue_passage = None;
                b.drop_queue_since = None;
            }
            b.task.retry_at = now;
            return Ok(());
        }
        let attempt = b.drop_holding_attempt;
        let role_sprint = self.bot_registry.config.as_ref().unwrap().slots[b.slot].sprint;
        let map = self.map_name().to_owned();
        let goal = if prepare {
            Ok(position(passage.launch))
        } else if resume && !physical_exit {
            b.task
                .role_goal
                .or(b.task.goal)
                .ok_or("bot_drop_resume_goal_missing")
        } else {
            let origin = if physical_exit || local_rejoin {
                point(b.target.runtime.unwrap().position)
            } else {
                passage.launch
            };
            let grid = if egress {
                policy.drop_egress_grid_m
            } else {
                policy.drop_holding_grid_m
            };
            let offset = if local_rejoin {
                let angle = (attempt % 8) as f64 * std::f64::consts::FRAC_PI_4;
                let radius = policy.recovery_m * (1. + ((attempt / 8) % 2) as f64);
                [angle.cos() * radius, angle.sin() * radius]
            } else if retreat {
                // Cover the known useful4m ring first, with full angular resolution.
                let angles = policy.drop_holding_candidate_limit;
                let radii = [4., 2., 6., 1., 3., 5., 7., 8.];
                let i = attempt % (angles * radii.len());
                let angle = (i % angles) as f64 * std::f64::consts::TAU / angles as f64;
                let radius = radii[i / angles];
                [angle.cos() * radius, angle.sin() * radius]
            } else {
                holding_offset(attempt % policy.drop_holding_candidate_limit, grid)
            };
            let raw = position([
                origin[0] + offset[0],
                origin[1] + 0.5,
                origin[2] + offset[1],
            ]);
            let key = (
                passage.id,
                [raw.x.to_bits(), raw.y.to_bits(), raw.z.to_bits()],
            );
            if let Some(g) = self.bot_registry.director.holding_cache.get(&key) {
                *g
            } else {
                let g = supported(self.collision_world.as_ref(), &map, raw, policy);
                if self.bot_registry.director.holding_cache.len() >= policy.cache_limit {
                    self.bot_registry.director.holding_cache.pop_first();
                }
                self.bot_registry.director.holding_cache.insert(key, g);
                g
            }
        };
        let result = (|| {
            let goal = goal?;
            let body = bot_physics::Body::default();
            if physical_exit
                && (distance(goal, position(passage.landing)) < policy.drop_clearance_m
                    || boxes_intersect(
                        crate::actor_collision::Bounds::body(point(goal), body),
                        passage.protected_bounds,
                    ))
            {
                return Err("egress_goal_not_clear");
            }
            if (!resume || physical_exit)
                && (protected.iter().any(|bounds| {
                    boxes_intersect(
                        crate::actor_collision::Bounds::body(point(goal), body),
                        *bounds,
                    )
                }) || lanes.iter().any(|lane| {
                    crate::bot_traffic_flow::in_lane(
                        point(goal),
                        lane,
                        policy.reservation_spacing_m,
                    )
                }))
            {
                return Err("holding_approach");
            }
            if !resume {
                if boxes_intersect(
                    crate::actor_collision::Bounds::body(point(goal), body),
                    passage.protected_bounds,
                ) {
                    return Err("holding_protected");
                }
                if lanes.iter().any(|lane| {
                    crate::bot_traffic_flow::in_lane(
                        point(goal),
                        lane,
                        policy.reservation_spacing_m,
                    )
                }) {
                    return Err("holding_approach");
                }
                if self.bot_registry.bots.iter().any(|(&other, b)| {
                    other != id
                        && b.drop_holding_goal
                            .into_iter()
                            .chain(b.task.goal)
                            .any(|g| distance(g, goal) < policy.reservation_spacing_m)
                }) || actors.occupied(id, point(goal), body)?
                {
                    return Err("holding_occupied");
                }
            }
            if started.elapsed().as_micros() >= u128::from(policy.planning_slice_us) {
                return Err("bot_planning_slice_yield");
            }
            *queries += 1;
            // Admission reserves a specific physical certificate. A staged
            // actor must resume through that passage rather than recomputing
            // the static shortest exit and silently discarding its ticket.
            let reroute_costs = if reroute {
                self.drop_route_costs(id, now, policy).1
            } else {
                BTreeMap::new()
            };
            let bot = self.bot_registry.bots.get_mut(&id).unwrap();
            // The egress proof reads the failed pending route to identify the
            // committed certificate. It mutates the target only after success.
            let old_pending = if physical_exit {
                None
            } else {
                bot.target.pending_move.take()
            };
            let old_runtime = bot.target.runtime;
            let result = if retreat {
                let world = self
                    .collision_world
                    .as_ref()
                    .ok_or("bot_collision_world_unavailable")?;
                let mesh = self
                    .navmesh_world
                    .clone()
                    .ok_or("bot_navmesh_unavailable")?;
                let movement = BotMove {
                    mode: BotMoveMode::Navmesh,
                    destination: goal,
                    duration_ms: 5000,
                    update_ms: 250,
                    yaw: None,
                    sprint: false,
                };
                let rules = self
                    .fall_damage_rules
                    .as_ref()
                    .ok_or("bot_fall_rules_unavailable")?;
                let query = Some(crate::actor_collision::Query {
                    grid: actors,
                    actor: id,
                });
                if preentry_retreat {
                    bot.target.begin_registry_grounded_passage_retreat(
                        world,
                        mesh,
                        &map,
                        movement,
                        now,
                        policy.drop_entry_buffer_m,
                        bot.health,
                        rules,
                        query,
                        passage.id,
                    )
                } else {
                    bot.target.begin_registry_aborted_drop_retreat(
                        world,
                        mesh,
                        &map,
                        movement,
                        now,
                        policy.drop_entry_buffer_m,
                        bot.health,
                        rules,
                        query,
                    )
                }
            } else if egress {
                bot.target.begin_registry_aborted_drop_egress(
                    self.collision_world
                        .as_ref()
                        .ok_or("bot_collision_world_unavailable")?,
                    self.navmesh_world
                        .clone()
                        .ok_or("bot_navmesh_unavailable")?,
                    &map,
                    BotMove {
                        mode: BotMoveMode::Navmesh,
                        destination: goal,
                        duration_ms: 5000,
                        update_ms: 250,
                        yaw: None,
                        sprint: false,
                    },
                    now,
                    policy.drop_entry_buffer_m,
                    bot.health,
                    self.fall_damage_rules
                        .as_ref()
                        .ok_or("bot_fall_rules_unavailable")?,
                    Some(crate::actor_collision::Query {
                        grid: actors,
                        actor: id,
                    }),
                )
            } else if local_rejoin {
                bot.target.begin_registry_ground_recovery(
                    self.collision_world
                        .as_ref()
                        .ok_or("bot_collision_world_unavailable")?,
                    self.navmesh_world
                        .clone()
                        .ok_or("bot_navmesh_unavailable")?,
                    &map,
                    BotMove {
                        mode: BotMoveMode::Navigation,
                        destination: goal,
                        duration_ms: 5000,
                        update_ms: 250,
                        yaw: None,
                        sprint: false,
                    },
                    now,
                    2. * policy.recovery_m,
                    policy.drop_entry_buffer_m,
                    crate::actor_collision::Query {
                        grid: actors,
                        actor: id,
                    },
                    policy.drop_traffic_query_budget,
                )
            } else {
                bot.target.begin_registry_move_with_drop_costs(
                    self.navmesh_world
                        .clone()
                        .ok_or("bot_navmesh_unavailable")?,
                    &map,
                    BotMove {
                        mode: BotMoveMode::Navmesh,
                        destination: goal,
                        duration_ms: 120000,
                        update_ms: 250,
                        yaw: None,
                        sprint: resume && role_sprint,
                    },
                    now,
                    bot.health,
                    self.fall_damage_rules.as_ref(),
                    Some(0.3),
                    &reroute_costs,
                    (resume && !reroute && !prepare).then_some(passage.id),
                )
            };
            let safe = result.is_ok()
                && (physical_exit
                    || local_rejoin
                    || reroute
                    || (prepare
                        && bot
                            .target
                            .registry_route()
                            .is_some_and(|r| r.waypoint_drop_ids.iter().all(Option::is_none)))
                    || (resume
                        && !physical_exit
                        && bot
                            .target
                            .next_registry_drop(policy.drop_entry_buffer_m)
                            .is_some_and(|p| p.id == passage.id))
                    || bot.target.registry_route().is_some_and(|r| {
                        r.waypoint_drop_ids.iter().all(Option::is_none)
                            && (egress
                                || r.waypoints.windows(2).all(|w| {
                                    !boxes_intersect(
                                        crate::actor_collision::Bounds::body(w[0], body).union(
                                            crate::actor_collision::Bounds::body(w[1], body),
                                        ),
                                        passage.protected_bounds,
                                    )
                                }))
                    }));
            if !safe {
                if !physical_exit {
                    bot.target.pending_move = old_pending;
                }
                bot.target.runtime = old_runtime;
                return Err(result.err().unwrap_or("holding_route_crosses_passage"));
            }
            if prepare {
                let r = bot.target.runtime.unwrap();
                let approach = bot.target.registry_route().map(|route| {
                    let mut from = r.physical_position();
                    route
                        .waypoints
                        .iter()
                        .map(|&to| {
                            let segment = [from, to];
                            from = to;
                            segment
                        })
                        .collect()
                });
                bot.target.pending_move = old_pending;
                bot.target.runtime = old_runtime;
                bot.drop_queue_approach = approach.ok_or("bot_drop_approach_missing")?;
                bot.drop_approach_ready = true;
                bot.drop_resume_failure_at = None;
                return Ok(goal);
            }
            if resume {
                if !physical_exit {
                    bot.task.goal = Some(bot.target.registry_move_destination().unwrap_or(goal));
                    bot.task.recovering = false;
                }
                bot.drop_holding_goal = None;
                bot.drop_resume = false;
                bot.drop_egress = false;
                bot.drop_retreat = false;
                bot.drop_resume_failure_at = None;
                if reroute {
                    bot.drop_reroute = false;
                    bot.drop_queue_passage = None;
                    bot.drop_queue_since = None;
                    bot.bind_delay_episode(now, policy.drop_entry_buffer_m);
                }
                bot.task.progress_at = now;
                bot.task.progress_distance = bot.target.registry_route_progress();
            } else {
                bot.drop_holding_goal = Some(goal);
                bot.holding_retry_at = now;
                bot.drop_approach_ready = false;
            }
            Ok(goal)
        })();
        if !resume && result.is_err() && !matches!(result, Err("bot_planning_slice_yield")) {
            self.bot_registry
                .bots
                .get_mut(&id)
                .unwrap()
                .holding_retry_at = now + Duration::from_millis(policy.retry_ms[0]);
        }
        if !matches!(result, Err("bot_planning_slice_yield")) && (!resume || physical_exit) {
            self.bot_registry
                .bots
                .get_mut(&id)
                .unwrap()
                .drop_holding_attempt += 1;
        }
        if prepare && result.is_err() && !matches!(result, Err("bot_planning_slice_yield")) {
            let b = self.bot_registry.bots.get_mut(&id).unwrap();
            let since = *b.drop_resume_failure_at.get_or_insert(now);
            if now.saturating_sub(since) >= Duration::from_millis(policy.drop_preentry_stall_ms) {
                b.drop_holding_goal = None;
                b.drop_approach_ready = false;
                b.drop_holding_attempt += 1;
                b.drop_resume_failure_at = None;
            }
        }
        if resume
            && !physical_exit
            && !reroute
            && !prepare
            && result.is_err()
            && !matches!(result, Err("bot_planning_slice_yield"))
        {
            let b = self.bot_registry.bots.get_mut(&id).unwrap();
            let since = *b.drop_resume_failure_at.get_or_insert(now);
            let r = b.target.runtime.unwrap();
            if now.saturating_sub(since) >= Duration::from_millis(policy.drop_preentry_stall_ms)
                && !b.drop_committed
                && r.physics_motion.is_some_and(|m| m.grounded)
                && !r.navmesh_state.on_player
                && !boxes_intersect(
                    crate::actor_collision::Bounds::body(point(r.position), Default::default()),
                    passage.protected_bounds,
                )
            {
                b.drop_reroute = true;
                b.drop_resume = false;
                self.bot_registry.failed_passages.insert(
                    passage.id,
                    now + Duration::from_millis(policy.drop_failure_cooldown_ms),
                );
            }
        }
        receipts.push(json!({"op":"bot_drop_holding","timestamp_ms":now.as_millis()as u64,"object_index":id,"drop_id":passage.id,"action":if prepare{"prepare"}else if retreat{if preentry_retreat{"preentry_retreat"}else{"abort_retreat"}}else if egress{"egress"}else if reroute{"reroute"}else if resume{"resume"}else{"stage"},"result":result,"selected_next_drop":self.bot_registry.bots[&id].target.next_registry_drop(policy.drop_entry_buffer_m).map(|p|p.id)}));
        Ok(())
    }
    pub(super) fn tick_bot_objectives(
        &mut self,
        now: Duration,
        wall: u64,
        actors: &crate::actor_collision::ActorGrid,
    ) -> Result<(), &'static str> {
        let Some(config) = self.bot_registry.config.as_ref().filter(|c| c.autonomous) else {
            return Ok(());
        };
        let policy = config
            .objective_policy
            .as_ref()
            .expect("population policy initialized")
            .clone();
        let started = Instant::now();
        let world = self
            .collision_world
            .as_ref()
            .map(|w| w.source_sha256().to_owned());
        if self.bot_registry.director.cache_world != world {
            self.bot_registry.director.cache.clear();
            self.bot_registry.director.holding_cache.clear();
            self.bot_registry.director.cache_world = world;
        }
        let mut receipts = Vec::new();
        // Resolve once per authored squad, rather than making each bot choose lanes.
        let assigned: BTreeMap<_, _> = self
            .bot_registry
            .bots
            .values()
            .map(|b| b.target.squad.squad_index())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .map(|squad| {
                let objective = self
                    .objective_catalog
                    .as_ref()
                    .zip(self.objective_world.as_ref())
                    .and_then(|(c, w)| assignment(c, w, squad));
                (squad, objective)
            })
            .collect();
        let mut operators = BTreeMap::new();
        if policy.interactions_enabled
            && self.match_started
            && self.match_terminal.is_none()
            && let Some((catalog, world)) = self
                .objective_catalog
                .as_ref()
                .zip(self.objective_world.as_ref())
        {
            self.bot_registry
                .director
                .failed_operators
                .retain(|_, until| *until > now);
            for object in &catalog.objects {
                let index = object.object_index;
                let owner = world.interaction_owner(index);
                // An authenticated human owns their entire accepted hold.
                if owner.is_some_and(|(id, _, _)| !self.bot_registry.bots.contains_key(&id)) {
                    continue;
                }
                let state = world.state(index).unwrap();
                let mut choices = Vec::new();
                for (&id, bot) in &self.bot_registry.bots {
                    let r = bot.target.runtime.unwrap();
                    if assigned[&bot.target.squad.squad_index()] != Some(index)
                        || !operator_safe(bot)
                    {
                        continue;
                    }
                    let Some((_, center, _)) =
                        operator_action(object, state, bot.target.squad.team())
                    else {
                        continue;
                    };
                    let key = (index, id, r.generation);
                    if bot.task.operator
                        && bot.task.goal.is_none()
                        && bot.task.candidate_attempts >= 24
                    {
                        self.bot_registry.director.failed_operators.insert(
                            key,
                            now + Duration::from_millis(policy.drop_failure_cooldown_ms),
                        );
                    }
                    if self
                        .bot_registry
                        .director
                        .failed_operators
                        .contains_key(&key)
                    {
                        continue;
                    }
                    let stable = owner.is_some_and(|(p, g, _)| p == id && g == r.generation)
                        || (bot.task.operator
                            && bot.task.objective == Some(index)
                            && bot.task.generation == r.generation);
                    choices.push((
                        !stable,
                        distance(r.position, position(center.map(f64::from))),
                        id,
                    ));
                }
                choices.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)).then(a.2.cmp(&b.2)));
                if let Some((_, _, id)) = choices.first() {
                    operators.insert(index, *id);
                }
            }
        }
        for (&id, bot) in &mut self.bot_registry.bots {
            let r = bot.target.runtime.unwrap();
            if bot.health == 0 {
                bot.task = Default::default();
                continue;
            }
            let t = &mut bot.task;
            if t.generation != r.generation {
                *t = Task {
                    generation: r.generation,
                    ..Default::default()
                };
            }
            let target = assigned[&bot.target.squad.squad_index()];
            let wide = target.is_some_and(|objective| {
                assigned
                    .iter()
                    .filter(|(s, o)| {
                        **s / 16 == bot.target.squad.squad_index() / 16 && **o == Some(objective)
                    })
                    .count()
                    > 2
            });
            let operator = target.is_some_and(|index| operators.get(&index) == Some(&id));
            if t.objective != target || t.wide != wide || t.operator != operator {
                if t.operator != operator {
                    receipts.push(json!({"op":"bot_objective_operator","object_index":id,"generation":r.generation,"objective":target,"selected":operator,"timestamp_ms":now.as_millis()as u64}));
                }
                t.operator = operator;
                t.goal = None;
                t.role_goal = None;
                t.continuing = false;
                bot.delay_episode = None;
                bot.reconsider = None;
                bot.reconsider_history.clear();
                bot.reconsider_at =
                    crate::bot_traffic_costs::next_reconsideration(now, id, &policy.drop_delay);
                bot.drop_approach_ready = false;
                t.wide = wide;
                t.objective = target;
                t.routed = false;
                t.retry_at = now;
                t.candidate_attempts = 0;
                t.attempts = 0;
                t.unavailable_reported = false;
            }
            if let Some(y) = bot.traffic_yield.as_mut() {
                if t.goal.is_none() && t.role_goal.is_none() {
                    y.resume = true;
                }
                t.progress_at = now;
                continue;
            }
            if let Some((waypoint, remaining)) = bot.target.registry_route_progress()
                && t.progress_distance.is_none_or(|(old, d)| {
                    waypoint > old || (waypoint == old && remaining + 0.5 <= d)
                })
            {
                t.progress_distance = Some((waypoint, remaining));
                t.progress_at = now;
            }
            let mut arrived = t
                .goal
                .is_some_and(|g| distance(g, r.position) <= policy.arrival_m);
            if arrived && t.continuing && !bot.drop_committed && bot.drop_waiting.is_none() {
                receipts.push(json!({"op":"bot_objective_segment_complete","object_index":id,"objective":t.objective,"position":r.position,"final_goal":t.role_goal,"timestamp_ms":now.as_millis()as u64}));
                t.finish_segment(now);
                arrived = false;
            }
            if arrived && t.recovering {
                t.goal = None;
                t.recovering = false;
                t.retry_at = now;
                arrived = false;
            }
            let grounded =
                r.physics_motion.is_some_and(|m| m.grounded) && !r.navmesh_state.on_player;
            if !grounded {
                continue;
            }
            // A committed passage keeps its physical route even if objective
            // intent changes while departure is still grounded or landing uncleared.
            if bot.drop_committed && !bot.drop_egress {
                t.progress_at = now;
                continue;
            }
            if t.goal.is_none()
                && bot.drop_holding_goal.is_none()
                && bot.drop_queue_passage.is_none()
                && bot.drop_permit.is_none()
                && matches!(r.phase, BotPhase::Relocating)
            {
                bot.target.pending_move = None;
                bot.target.runtime.as_mut().unwrap().phase = BotPhase::Complete;
            }
            if target.is_none() && !t.unavailable_reported {
                t.unavailable_reported = true;
                receipts.push(json!({"op":"bot_objective_waiting","timestamp_ms":now.as_millis()as u64,"object_index":id,"assigned_objective":null,"reason":"bot_objective_unavailable","planning_query":false}));
            }
            if bot.drop_waiting.is_some() {
                t.progress_at = now;
                t.retry_at = now + Duration::from_millis(policy.stall_ms);
            }
            if arrived {
                continue;
            }
        }
        let mut candidates = 0;
        let mut queries = 0;
        let mut cache_hits = 0;
        let mut selected_ids = std::collections::BTreeSet::new();
        while candidates < policy.candidate_checks_per_tick
            && queries < policy.route_queries_per_tick
            && started.elapsed().as_micros() < u128::from(policy.planning_slice_us)
        {
            let mut fresh = Vec::new();
            let mut retry = Vec::new();
            for (&id, b) in &self.bot_registry.bots {
                let r = b.target.runtime.unwrap();
                let t = &b.task;
                let active_retreat = b
                    .target
                    .registry_aborted_drop_retreat(policy.drop_entry_buffer_m)
                    .or_else(|| {
                        b.target
                            .registry_grounded_passage_retreat(policy.drop_entry_buffer_m)
                    });
                if active_retreat
                    .is_some_and(|p| p.phase == development_bot_movement::DropPhase::Retreating)
                    && matches!(r.phase, BotPhase::Relocating)
                    && !r.navmesh_state.navigation_aborted
                {
                    continue;
                }
                let traffic_job = b.traffic_yield.as_ref().is_some_and(|y| {
                    now >= y.retry_at
                        && (y.resume
                            || y.goal.is_none()
                            || r.navmesh_state.navigation_aborted
                            || (matches!(r.phase, BotPhase::Relocating)
                                && now.saturating_sub(r.since)
                                    > Duration::from_millis(policy.drop_holding_stall_ms)))
                });
                if traffic_job
                    && !selected_ids.contains(&id)
                    && b.health > 0
                    && r.physics_motion.is_some_and(|m| m.grounded)
                    && !r.navmesh_state.on_player
                {
                    retry.push(id);
                    continue;
                }
                if b.traffic_yield.is_some() {
                    continue;
                }
                let holding_job = b.drop_waiting.is_some()
                    && (b.drop_holding_goal.is_none()
                        || (!b.drop_approach_ready
                            && matches!(r.phase, BotPhase::Complete)
                            && b.drop_holding_goal
                                .is_some_and(|g| distance(g, r.position) <= policy.arrival_m))
                        || r.navmesh_state.navigation_aborted
                        || (b
                            .drop_holding_goal
                            .is_some_and(|g| distance(g, r.position) > policy.arrival_m)
                            && (!matches!(r.phase, BotPhase::Relocating)
                                || now.saturating_sub(r.since)
                                    > Duration::from_millis(policy.drop_holding_stall_ms))))
                    || b.drop_resume
                    || b.drop_egress
                    || b.drop_retreat
                    || b.drop_reroute;
                if holding_job
                    && (now >= b.holding_retry_at
                        || b.drop_resume
                        || b.drop_egress
                        || b.drop_retreat
                        || b.drop_reroute)
                    && !selected_ids.contains(&id)
                    && b.health > 0
                    && r.physics_motion.is_some_and(|m| m.grounded)
                    && !r.navmesh_state.on_player
                {
                    retry.push(id);
                    continue;
                }
                if !selected_ids.contains(&id)
                    && now >= b.reconsider_at
                    && b.can_reconsider(policy.drop_entry_buffer_m, policy.arrival_m)
                {
                    retry.push(id);
                    continue;
                }
                if selected_ids.contains(&id)
                    || b.drop_waiting.is_some()
                    || b.drop_committed
                    || b.health == 0
                    || t.objective.is_none()
                    || now < t.retry_at
                    || !r.physics_motion.is_some_and(|m| m.grounded)
                    || r.navmesh_state.on_player
                {
                    continue;
                }
                if t.goal
                    .is_some_and(|g| distance(g, r.position) <= policy.arrival_m)
                {
                    continue;
                }
                if t.goal.is_some()
                    && now.saturating_sub(t.progress_at) < Duration::from_millis(policy.stall_ms)
                    && matches!(r.phase, BotPhase::Relocating)
                {
                    continue;
                }
                if t.routed {
                    retry.push(id);
                } else {
                    fresh.push(id);
                }
            }
            let Some(id) = self
                .bot_registry
                .director
                .choose_ready(&fresh, &retry, &policy)
            else {
                break;
            };
            selected_ids.insert(id);
            candidates += 1;
            if self.bot_registry.bots[&id].traffic_yield.is_some() {
                self.plan_traffic_yield(
                    id,
                    now,
                    &policy,
                    actors,
                    &started,
                    &mut queries,
                    &mut receipts,
                )?;
                continue;
            }
            if now >= self.bot_registry.bots[&id].reconsider_at
                && self.bot_registry.bots[&id]
                    .can_reconsider(policy.drop_entry_buffer_m, policy.arrival_m)
            {
                self.plan_drop_reconsideration(
                    id,
                    now,
                    &policy,
                    &started,
                    &mut queries,
                    &mut receipts,
                )?;
                continue;
            }
            if self.bot_registry.bots[&id].drop_waiting.is_some()
                || self.bot_registry.bots[&id].drop_resume
                || self.bot_registry.bots[&id].drop_egress
                || self.bot_registry.bots[&id].drop_retreat
                || self.bot_registry.bots[&id].drop_reroute
            {
                self.plan_drop_holding(
                    id,
                    now,
                    &policy,
                    actors,
                    &started,
                    &mut queries,
                    &mut receipts,
                )?;
                continue;
            }
            let bot = &self.bot_registry.bots[&id];
            let objective = bot.task.objective.unwrap();
            let squad = bot.target.squad.squad_index();
            let Some((center, owner)) = self
                .objective_catalog
                .as_ref()
                .and_then(|c| c.objects.iter().find(|o| o.object_index == objective))
                .and_then(|o| {
                    o.interaction
                        .as_ref()
                        .map(|r| (r.position, r.owner_team))
                        .or_else(|| o.demolition.as_ref().map(|r| (r.position, r.owner_team)))
                })
            else {
                continue;
            };
            let defending = bot.target.squad.team() == owner;
            let operator = bot.task.operator;
            let operator_rule = operator.then(|| {
                let object = self
                    .objective_catalog
                    .as_ref()
                    .unwrap()
                    .objects
                    .iter()
                    .find(|o| o.object_index == objective)
                    .unwrap();
                operator_action(
                    object,
                    self.objective_world
                        .as_ref()
                        .unwrap()
                        .state(objective)
                        .unwrap(),
                    bot.target.squad.team(),
                )
                .unwrap()
            });
            let operator_role = DestinationRole {
                name: "operator".into(),
                back_m: 0.,
                side_m: 0.,
            };
            let role = if operator {
                &operator_role
            } else if defending {
                &policy.defend_roles[usize::from(bot.role_slot)]
            } else {
                &policy.attack_roles[usize::from(bot.role_slot)]
            };
            let anchor = self
                .spawn_catalog
                .as_ref()
                .and_then(|c| c.default_for_squad(squad).and_then(|s| c.point(s)))
                .map_or(bot.target.position, |s| s.position);
            let attempt = bot.task.attempts;
            let recovering = bot.task.goal.is_some() && attempt % 2 == 1;
            let candidate_attempt = bot.task.candidate_attempts;
            let index = candidate_attempt % policy.candidate_offsets_m.len();
            let sector = if bot.task.wide {
                policy.reinforced_squad_lateral_m[usize::from(squad % 4)]
            } else {
                policy.paired_squad_lateral_m[usize::from(squad % 4)]
            };
            let mut offset = policy.candidate_offsets_m[index];
            offset[1] += sector;
            let mut raw = if let Some((_, center, radius)) = operator_rule {
                // Generic concentric approaches; grounding is checked again against the
                // actual 3D trigger radius before accepting the physical route.
                let angle = (candidate_attempt % 8) as f64 * std::f64::consts::FRAC_PI_4;
                let radial = f64::from(radius) * [0.55, 0.25, 0.8][candidate_attempt / 8 % 3];
                position([
                    f64::from(center[0]) + radial * angle.cos(),
                    f64::from(center[1]) + 0.5,
                    f64::from(center[2]) + radial * angle.sin(),
                ])
            } else {
                role_candidate(center, anchor, role, offset)
            };
            if bot.task.continuing
                && bot.task.goal.is_none()
                && let Some(final_goal) = bot.task.role_goal
            {
                raw = final_goal;
            }
            if recovering {
                let p = bot.target.runtime.unwrap().position;
                let a = f64::from((u32::from(id) + attempt / 2) % 8) * std::f64::consts::FRAC_PI_4;
                let offmesh = bot.target.runtime.unwrap().navmesh_state.navigation_aborted
                    && self.navmesh_world.as_ref().is_some_and(|mesh| {
                        mesh.component_id(bot.target.runtime.unwrap().physical_position())
                            .is_err()
                    });
                let radius = policy.recovery_m
                    * if offmesh {
                        1. + f64::from((attempt / 16) % 2)
                    } else {
                        1.
                    };
                raw = replication::CharacterPosition {
                    x: p.x + (a.cos() * radius) as f32,
                    y: p.y + 0.5,
                    z: p.z + (a.sin() * radius) as f32,
                };
            }
            let key = CandidateKey {
                objective,
                squad,
                role: bot.role_slot,
                candidate: index,
                raw: [raw.x.to_bits(), raw.y.to_bits(), raw.z.to_bits()],
            };
            let map = self.map_name().to_string();
            // Support is static for this exact full-coordinate key, including
            // recovery candidates. Retain it before the soft-slice guard so
            // one expensive preflight cannot restart forever on every tick.
            let (static_goal, cached) =
                self.bot_registry
                    .director
                    .candidate_support(key, policy.cache_limit, || {
                        if !recovering && let Some((_, center, radius)) = operator_rule {
                            supported_operator(
                                self.collision_world.as_ref(),
                                &map,
                                raw,
                                center,
                                radius,
                            )
                        } else {
                            supported(self.collision_world.as_ref(), &map, raw, &policy)
                        }
                    });
            cache_hits += usize::from(cached);
            let reservations: Vec<_> = self
                .bot_registry
                .bots
                .iter()
                .filter(|(other, b)| **other != id && b.health > 0)
                .flat_map(|(_, b)| {
                    b.task
                        .goal
                        .into_iter()
                        .chain(b.drop_holding_goal)
                        .chain(b.traffic_yield.as_ref().and_then(|y| y.goal))
                })
                .collect();
            let (demand, drop_costs) = self.drop_route_costs(id, now, &policy);
            let t = &mut self.bot_registry.bots.get_mut(&id).unwrap().task;
            t.attempts = t.attempts.saturating_add(1);
            if !recovering {
                t.candidate_attempts = t.candidate_attempts.saturating_add(1);
            }
            t.retry_at = now
                + Duration::from_millis(
                    policy.retry_ms[(t.attempts as usize / policy.candidate_offsets_m.len())
                        .min(policy.retry_ms.len() - 1)],
                );
            let mut query_us = 0;
            let result = (|| {
                let goal = static_goal?;
                if !recovering
                    && let Some((_, center, radius)) = operator_rule
                    && distance(goal, position(center.map(f64::from))) > f64::from(radius) * 0.95
                {
                    return Err("bot_operator_approach_out_of_range");
                }
                if self.navmesh_world.as_ref().is_some_and(|mesh| {
                    mesh.artifact().drop_links.iter().any(|link| {
                        let body = bot_physics::Body::default();
                        let mut protected = crate::actor_collision::Bounds::body(link.launch, body);
                        for sample in &link.trajectory {
                            protected = protected
                                .union(crate::actor_collision::Bounds::body(sample.position, body));
                        }
                        protected = protected.union(crate::actor_collision::Bounds {
                            min: [
                                link.launch[0] - policy.drop_entry_buffer_m,
                                link.launch[1],
                                link.launch[2] - policy.drop_entry_buffer_m,
                            ],
                            max: [
                                link.launch[0] + policy.drop_entry_buffer_m,
                                link.launch[1] + body.height,
                                link.launch[2] + policy.drop_entry_buffer_m,
                            ],
                        });
                        let target = crate::actor_collision::Bounds::body(point(goal), body);
                        (0..3).all(|axis| {
                            target.min[axis] <= protected.max[axis]
                                && target.max[axis] >= protected.min[axis]
                        })
                    })
                }) {
                    return Err("bot_objective_drop_passage_reserved");
                }
                if reservations
                    .iter()
                    .any(|p| distance(*p, goal) < policy.reservation_spacing_m)
                    || actors.occupied(id, point(goal), Default::default())?
                {
                    return Err("bot_objective_occupied");
                }
                // Preflight work and a full route query consume separate bounded quotas.
                if started.elapsed().as_micros() >= u128::from(policy.planning_slice_us) {
                    return Err("bot_planning_slice_yield");
                }
                queries += 1;
                let query_started = Instant::now();
                let bot = self.bot_registry.bots.get_mut(&id).unwrap();
                let mesh = self
                    .navmesh_world
                    .clone()
                    .ok_or("bot_navmesh_unavailable")?;
                let local_rejoin = recovering
                    && bot.target.runtime.unwrap().navmesh_state.navigation_aborted
                    && mesh
                        .component_id(bot.target.runtime.unwrap().physical_position())
                        .is_err();
                let movement = BotMove {
                    mode: if local_rejoin {
                        BotMoveMode::Navigation
                    } else {
                        BotMoveMode::Navmesh
                    },
                    destination: goal,
                    duration_ms: if local_rejoin { 5000 } else { 120000 },
                    update_ms: 250,
                    yaw: None,
                    sprint: !local_rejoin
                        && self.bot_registry.config.as_ref().unwrap().slots[bot.slot].sprint,
                };
                let result = if local_rejoin {
                    bot.target.begin_registry_ground_recovery(
                        self.collision_world
                            .as_ref()
                            .ok_or("bot_collision_world_unavailable")?,
                        mesh,
                        &map,
                        movement,
                        now,
                        2. * policy.recovery_m,
                        policy.drop_entry_buffer_m,
                        crate::actor_collision::Query {
                            grid: actors,
                            actor: id,
                        },
                        policy.drop_traffic_query_budget,
                    )
                } else {
                    bot.target.begin_registry_move_with_drop_costs(
                        mesh,
                        &map,
                        movement,
                        now,
                        bot.health,
                        self.fall_damage_rules.as_ref(),
                        Some(0.3),
                        &drop_costs,
                        None,
                    )
                };
                query_us = query_started.elapsed().as_micros() as u64;
                result?;
                let segment_goal = bot.target.registry_move_destination().unwrap_or(goal);
                let t = &mut bot.task;
                t.goal = Some(segment_goal);
                t.continuing = !recovering && distance(segment_goal, goal) > policy.arrival_m;
                if !recovering {
                    if t.role_goal != Some(goal) {
                        bot.traffic_yield_cursor = 0;
                        bot.reconsider_history.clear();
                    }
                    t.role_goal = Some(goal);
                }
                t.recovering = recovering;
                bot.delay_episode = None;
                bot.reconsider = None;
                bot.reconsider_at =
                    crate::bot_traffic_costs::next_reconsideration(now, id, &policy.drop_delay);
                t.routed = true;
                t.progress_at = now;
                t.progress_distance = bot.target.registry_route_progress();
                t.retry_at = now + Duration::from_millis(policy.stall_ms);
                bot.move_attempted = true;
                bot.bind_delay_episode(now, policy.drop_entry_buffer_m);
                Ok(goal)
            })();
            if matches!(result, Err("bot_planning_slice_yield")) {
                let t = &mut self.bot_registry.bots.get_mut(&id).unwrap().task;
                t.attempts = attempt;
                t.candidate_attempts = candidate_attempt;
                t.retry_at = now;
                receipts.push(
                    json!({"op":"bot_planning_yield","object_index":id,"candidate_retained":true}),
                );
                break;
            }
            receipts.push(match result {
                Ok(goal)=>json!({"op":"bot_objective_task","timestamp_ms":now.as_millis()as u64,"object_index":id,"squad_index":squad,"objective":objective,"destination_role":role.name,"role_slot":self.bot_registry.bots[&id].role_slot,"reinforced_allocation":self.bot_registry.bots[&id].task.wide,"recovering":recovering,"segment_goal":self.bot_registry.bots[&id].task.goal,"continuing":self.bot_registry.bots[&id].task.continuing,"role":if defending{"defend"}else{"attack"},"goal":goal,"attempt":attempt,"planning_query_us":query_us,"drop_cost_equivalent_m":drop_costs,"drop_reserved_demand_s":demand,"drop_headway_s":policy.drop_congestion_headway_s,"selected_next_drop":self.bot_registry.bots[&id].target.next_registry_drop(policy.drop_entry_buffer_m).map(|p|p.id)}),
                Err(reason)=>json!({"op":"bot_objective_waiting","timestamp_ms":now.as_millis()as u64,"object_index":id,"squad_index":squad,"assigned_objective":objective,"destination_role":role.name,"reason":reason,"attempt":attempt,"planning_query":query_us>0,"planning_query_us":query_us}),
            });
        }
        if candidates > 0 {
            let geometry = {
                let cache = self.bot_registry.director.cost_geometry.borrow();
                json!({"rebuild_us":if cache.at==Some(now){cache.rebuild_us}else{0},"quotes":if cache.at==Some(now){cache.quotes}else{0}})
            };
            receipts.push(json!({"op":"bot_planning_slice","timestamp_ms":now.as_millis()as u64,"candidate_checks":candidates,"route_queries":queries,"cached_candidates":cache_hits,"cost_geometry":geometry,"elapsed_us":started.elapsed().as_micros()as u64,"soft_budget_us":policy.planning_slice_us}));
        }
        for receipt in receipts {
            self.record(receipt, wall)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod public_synthetic_policy_tests {
    #[test]
    fn reaching_an_intermediate_leg_preserves_the_final_objective() {
        let final_goal = super::position([300., 20., 300.]);
        let mut task = super::Task {
            objective: Some(4),
            goal: Some(super::position([100., 20., 100.])),
            role_goal: Some(final_goal),
            continuing: true,
            retry_at: std::time::Duration::from_secs(50),
            ..Default::default()
        };
        task.finish_segment(std::time::Duration::from_secs(12));
        assert!(task.goal.is_none());
        assert_eq!(task.objective, Some(4));
        assert_eq!(task.role_goal, Some(final_goal));
        assert!(task.continuing);
        assert_eq!(task.retry_at, std::time::Duration::from_secs(12));
    }
    #[test]
    fn bundled_synthetic_policy_satisfies_bounds() {
        let policy = super::ObjectivePolicy::bundled();
        assert_eq!(policy.validate(), Ok(()));
        assert!(!policy.interactions_enabled);
    }
}
