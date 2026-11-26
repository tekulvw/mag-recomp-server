use super::*;
use crate::{
    actor_collision::Bounds, bot_resupply_approach_policy::Policy,
    development_bot_movement::protected_drop_bounds,
};
use std::sync::Arc;

pub(super) struct Approach {
    policy: Policy,
    mesh: Arc<navmesh::NavMesh>,
    attempts: BTreeMap<u16, Retry>,
    cursor: u16,
}

struct Retry {
    life: (u32, u32),
    next: usize,
    at: Duration,
    try_drop: bool,
    /// Where the last seek started, and how many seeks in a row started
    /// there: a bot whose seeks keep stopping before it moves (a blocked
    /// route) waits longer before each further one.
    origin: [f64; 3],
    repeats: u32,
}

/// Seek attempts in a row from within this distance count as no progress.
const SEEK_PROGRESS_M: f64 = 1.0;
/// Longest wait between seeks that keep starting from the same spot.
const SEEK_BACKOFF_CAP: Duration = Duration::from_secs(60);

/// When the next seek may start after one from `origin` now, and its repeat
/// count. The ordinary retry interval applies while the bot makes progress;
/// each further seek from the same spot doubles it, up to a minute.
fn seek_backoff(
    previous: Option<(&[f64; 3], u32)>,
    origin: [f64; 3],
    now: Duration,
    retry: Duration,
) -> (Duration, u32) {
    let repeats = previous
        .filter(|(at, _)| (at[0] - origin[0]).hypot(at[2] - origin[2]) <= SEEK_PROGRESS_M)
        .map_or(0, |(_, repeats)| repeats.saturating_add(1));
    let wait = retry
        .saturating_mul(1u32 << repeats.min(8))
        .min(SEEK_BACKOFF_CAP.max(retry));
    (now.saturating_add(wait), repeats)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Passage {
    Pending(u32),
    Cleared(u32),
    Aborted,
}

#[derive(Clone)]
pub(crate) struct Seek {
    scope: Scope,
    policy_revision: u64,
    allocation: u32,
    ammo: AmmoBinding,
    damage_revision: u64,
    pub(super) provider: ProviderKey,
    pub(super) movement_id: String,
    projected_goal: [f64; 3],
    previous_goal: Option<replication::CharacterPosition>,
    deadline: Duration,
    passage: Option<Passage>,
    cancellation: Option<&'static str>,
}

impl Seek {
    pub(crate) fn parent_id(&self) -> &str {
        &self.movement_id
    }

    pub(crate) fn allowed_drop_id(&self) -> Option<u32> {
        match self.passage {
            Some(Passage::Pending(id)) => Some(id),
            _ => None,
        }
    }

    pub(crate) fn passage_resume_goal(&self) -> [f64; 3] {
        self.projected_goal
    }

    pub(crate) fn cancellation_pending(&self) -> bool {
        self.cancellation.is_some()
    }

    pub(crate) fn passage_released(&mut self, id: u32, cleared: bool) {
        if self.allowed_drop_id() == Some(id) {
            self.passage = Some(if cleared {
                Passage::Cleared(id)
            } else {
                Passage::Aborted
            });
        }
    }

    #[cfg(test)]
    pub(crate) fn mark_test_passage_pending(&mut self, id: u32) {
        self.passage = Some(Passage::Pending(id));
    }

    pub(super) fn latch_cancellation(&mut self, reason: &'static str) {
        self.cancellation.get_or_insert(reason);
    }

    pub(super) fn drop_interrupt_reason(
        &self,
        bot: &WorldBot,
        scope: Option<Scope>,
        revision: Option<u64>,
        observed: Option<Observation>,
        now: Duration,
    ) -> Option<&'static str> {
        if revision != Some(self.policy_revision) || scope != Some(self.scope) || !self.owns(bot) {
            Some("lost_seek_scope_or_movement")
        } else if now >= self.deadline {
            Some("seek_expired")
        } else if !has_ammo_binding(bot, observed, self.ammo) {
            Some("changed_seek_equipment")
        } else if bot.damage_revision != self.damage_revision
            || observed.is_some_and(|seen| matches!(seen.threat, Threat::Enemy))
        {
            Some("combat_interrupt")
        } else {
            None
        }
    }

    pub(crate) fn owns(&self, bot: &WorldBot) -> bool {
        bot.allocation_generation == self.allocation
            && bot
                .target
                .runtime
                .is_some_and(|r| r.generation == self.ammo.equipment.0)
            && bot.target.owns_supply_parent(&self.movement_id)
    }

    pub(super) fn step(
        &self,
        bot: &WorldBot,
        scope: Option<Scope>,
        observed: Option<Observation>,
        frame: Option<&Frame<'_>>,
        revision: Option<u64>,
        now: Duration,
    ) -> Disposition {
        if self.cancellation_pending() {
            return Disposition::FinishCancellation;
        }
        if let Some(passage) = self.passage {
            if let Some(reason) = self.drop_interrupt_reason(bot, scope, revision, observed, now) {
                return Disposition::Cancel(reason);
            }
            return match passage {
                Passage::Pending(_) => Disposition::Wait,
                Passage::Cleared(id) if frame.is_some() => Disposition::ContinueAfterPassage(id),
                Passage::Cleared(_) => Disposition::Wait,
                Passage::Aborted => Disposition::Cancel("seek_passage_aborted"),
            };
        }
        let Some(frame) = frame.filter(|frame| {
            revision == Some(self.policy_revision)
                && self.scope == frame.scope
                && self.owns(frame.bot)
        }) else {
            return Disposition::Cancel("lost_seek_scope_or_movement");
        };
        if now >= self.deadline {
            return Disposition::Cancel("seek_expired");
        }
        if !frame.has_ammo_binding(self.ammo) {
            return Disposition::Cancel("changed_seek_equipment");
        }
        if frame.bot.damage_revision != self.damage_revision
            || frame
                .observed
                .is_some_and(|seen| matches!(seen.threat, Threat::Enemy))
        {
            return Disposition::Cancel("combat_interrupt");
        }
        let runtime = frame.bot.target.runtime.unwrap();
        if matches!(runtime.phase, BotPhase::Relocating) {
            return Disposition::Wait;
        }
        if runtime.navmesh_state.navigation_aborted
            || !frame
                .catalog
                .provider(self.provider)
                .is_some_and(|p| p.contains(frame.origin))
        {
            return Disposition::Cancel("seek_stopped_outside_provider");
        }
        if frame.start_ammo().is_some() {
            Disposition::Arrive
        } else {
            Disposition::Wait
        }
    }
}

pub(super) enum Disposition {
    Wait,
    Arrive,
    Cancel(&'static str),
    FinishCancellation,
    ContinueAfterPassage(u32),
}

pub(super) struct Request {
    pub(super) id: u16,
    pub(super) scope: Scope,
    pub(super) ammo: AmmoBinding,
    pub(super) origin: [f64; 3],
    pub(super) planning: Option<PlanningReservation>,
}

impl<W: Write> Engine<W> {
    pub fn configure_bot_resupply_approach_policy(
        &mut self,
        policy: Policy,
    ) -> Result<(), &'static str> {
        let (catalog, binding) = self
            .bot_resupply
            .provider
            .as_ref()
            .ok_or("bot_resupply_approach_requires_provider_catalog")?;
        let mesh = self
            .navmesh_world
            .as_ref()
            .ok_or("bot_resupply_approach_requires_navmesh")?;
        let world = self
            .collision_world
            .as_ref()
            .ok_or("bot_resupply_approach_requires_collision_world")?;
        if !binding.matches(self) {
            return Err("bot_resupply_approach_world_binding_mismatch");
        }
        policy.validate_binding(catalog, mesh, world)?;
        let population = self.bot_registry.config.as_ref();
        policy.validate_drop_context(
            population.is_some_and(|config| config.autonomous),
            population.is_some_and(|config| config.objective_policy.is_some()),
            self.fall_damage_rules.is_some(),
        )?;
        self.install_resupply_approach(policy)
    }

    pub(in crate::bot_resupply) fn install_resupply_approach(
        &mut self,
        policy: Policy,
    ) -> Result<(), &'static str> {
        self.bot_resupply.approach_revision = self
            .bot_resupply
            .approach_revision
            .checked_add(1)
            .ok_or("bot_resupply_approach_revision_exhausted")?;
        self.bot_resupply.approach = Some(Box::new(Approach {
            policy,
            mesh: self
                .navmesh_world
                .as_ref()
                .expect("validated approach mesh")
                .clone(),
            attempts: BTreeMap::new(),
            cursor: 0,
        }));
        Ok(())
    }

    pub(super) fn resupply_approach_revision(&self) -> Option<u64> {
        self.bot_resupply
            .approach
            .as_ref()
            .filter(|approach| {
                self.navmesh_world
                    .as_ref()
                    .is_some_and(|mesh| Arc::ptr_eq(mesh, &approach.mesh))
            })
            .map(|_| self.bot_resupply.approach_revision)
    }

    pub(super) fn finish_resupply_seek(&mut self, id: u16, now: Duration) {
        let bot = self.bot_registry.bots.get_mut(&id).unwrap();
        let seek = bot.resupply_seek.take().unwrap();
        if !seek.owns(bot) || !bot.target.stop_owned_supply_move(&seek.movement_id) {
            return;
        }
        let config = self.bot_registry.config.as_ref();
        if config.is_some_and(|config| config.autonomous) {
            bot.task.resume_after_supply(now);
        } else if seek.previous_goal.is_some()
            && config
                .and_then(|config| config.slots.get(bot.slot))
                .and_then(|slot| slot.goal)
                == seek.previous_goal
        {
            bot.move_attempted = false;
        }
    }

    pub(super) fn resupply_query_budget(&self) -> usize {
        self.bot_resupply
            .approach
            .as_ref()
            .map_or(0, |approach| approach.policy.query_budget_per_tick())
    }

    pub(super) fn resupply_planning_context(&self) -> Option<(u64, u32, Duration)> {
        let revision = self.resupply_approach_revision()?;
        let policy = &self.bot_resupply.approach.as_ref()?.policy;
        let population = self.bot_registry.config.as_ref()?;
        if !population.autonomous {
            return None;
        }
        Some((
            revision,
            policy.allowed_drop_id()?,
            policy.planning_window()?,
        ))
    }

    pub(super) fn resupply_passage_obligation(&self, id: u16, seek: &Seek) -> bool {
        seek.allowed_drop_id().is_some_and(|drop_id| {
            self.bot_registry
                .supply_passage_obligation(id, seek.ammo.equipment.0, drop_id)
        })
    }

    pub(crate) fn prepare_resupply_drop_resume(
        &self,
        id: u16,
        actors: &crate::actor_collision::ActorGrid,
        now: Duration,
    ) -> Result<crate::development_bot_movement::PreparedRegistryMove, &'static str> {
        let drop_id = self
            .bot_registry
            .bots
            .get(&id)
            .and_then(|bot| bot.resupply_seek.as_ref())
            .and_then(Seek::allowed_drop_id)
            .ok_or("bot_resupply_seek_passage_mismatch")?;
        self.prepare_resupply_route(id, actors, now, Some(drop_id))
    }

    fn prepare_resupply_route(
        &self,
        id: u16,
        actors: &crate::actor_collision::ActorGrid,
        now: Duration,
        drop_id: Option<u32>,
    ) -> Result<crate::development_bot_movement::PreparedRegistryMove, &'static str> {
        let bot = &self.bot_registry.bots[&id];
        let seek = bot
            .resupply_seek
            .as_ref()
            .filter(|seek| {
                seek.owns(bot)
                    && !seek.cancellation_pending()
                    && seek.deadline > now
                    && self.resupply_approach_revision() == Some(seek.policy_revision)
                    && self
                        .bot_resupply
                        .provider
                        .as_ref()
                        .is_some_and(|(_, binding)| binding.matches(self))
                    && has_ammo_binding(bot, None, seek.ammo)
                    && seek.damage_revision == bot.damage_revision
            })
            .ok_or("bot_resupply_seek_route_scope_changed")?;
        let Approach { policy, mesh, .. } = self.bot_resupply.approach.as_deref().unwrap();
        let origin = bot.target.runtime.unwrap().physical_position();
        let goal = seek.passage_resume_goal();
        let radius = policy.goal_radius_m();
        let route = if let Some(drop_id) = drop_id {
            mesh.route_with_drop_costs(
                origin,
                goal,
                radius,
                self.fall_damage_rules
                    .as_ref()
                    .ok_or("bot_fall_rules_unavailable")?,
                bot.health,
                &BTreeMap::new(),
                Some(drop_id),
            )
        } else {
            mesh.route(origin, goal, radius)
        }
        .map_err(|error| error.0)?;
        if route.length_m > policy.max_route_length_m()
            || !self.supply_route_clear(mesh, &route, id, seek.provider, actors, drop_id)
        {
            return Err("bot_resupply_seek_unsafe_route");
        }
        bot.target.prepare_registry_supply_route(
            mesh.clone(),
            route,
            goal,
            seek.parent_id().to_owned(),
            policy.max_duration().as_millis() as u32,
            now,
            drop_id,
        )
    }

    pub(super) fn continue_resupply_after_passage(
        &mut self,
        id: u16,
        cleared_drop: u32,
        now: Duration,
        wall: u64,
    ) -> Result<bool, &'static str> {
        let Ok(actors) = self.actor_grid(now) else {
            return Ok(false);
        };
        let Ok(plan) = self.prepare_resupply_route(id, &actors, now, None) else {
            return Ok(false);
        };
        let bot = &self.bot_registry.bots[&id];
        let seek = bot.resupply_seek.as_ref().unwrap();
        self.record(
            json!({
                "op":"bot_resupply_seek_continued","object_index":id,
                "movement_id":seek.parent_id(),"drop_id":cleared_drop,
                "origin":bot.target.runtime.unwrap().physical_position(),
                "projected_goal":seek.projected_goal,"timestamp_ms":now.as_millis(),
            }),
            wall,
        )?;
        let bot = self.bot_registry.bots.get_mut(&id).unwrap();
        plan.install(&mut bot.target);
        bot.resupply_seek.as_mut().unwrap().passage = None;
        Ok(true)
    }
}

impl<W: Write> Engine<W> {
    pub(super) fn plan_resupply_seeks(
        &mut self,
        now: Duration,
        wall: u64,
        mut requests: Vec<Request>,
        credits: usize,
    ) -> Result<(), &'static str> {
        let Some(revision) = self
            .resupply_approach_revision()
            .filter(|_| !requests.is_empty())
        else {
            return Ok(());
        };
        let Ok(actors) = self.actor_grid(now) else {
            return Ok(());
        };
        let approach = self.bot_resupply.approach.as_mut().unwrap();
        let policy = &approach.policy;
        let mesh = approach.mesh.clone();
        let radius = policy.goal_radius_m();
        let allowed_drop_id = policy.allowed_drop_id();
        let max_length = policy.max_route_length_m();
        let duration = policy.max_duration();
        let retry_at = now
            .checked_add(policy.retry())
            .ok_or("bot_resupply_seek_clock_overflow")?;
        let (catalog, _) = self.bot_resupply.provider.as_ref().unwrap();
        approach.attempts.retain(|id, attempt| {
            self.bot_registry.bots.get(id).is_some_and(|bot| {
                bot.allocation_generation == attempt.life.0
                    && bot
                        .target
                        .runtime
                        .is_some_and(|r| r.generation == attempt.life.1)
            })
        });
        requests.sort_unstable_by_key(|request| request.id.wrapping_sub(approach.cursor));
        let mut queries = Vec::new();
        for request in requests {
            if queries.len() >= credits {
                break;
            }
            if allowed_drop_id.is_some() && request.planning.is_none() {
                continue;
            }
            let bot = &self.bot_registry.bots[&request.id];
            if approach
                .attempts
                .get(&request.id)
                .is_some_and(|attempt| now < attempt.at)
            {
                continue;
            }
            let mut anchors: Vec<_> = policy
                .providers()
                .flat_map(|key| {
                    policy
                        .candidates(key)
                        .iter()
                        .map(move |anchor| (key, *anchor))
                })
                .filter_map(|(key, anchor)| {
                    let distance =
                        (anchor[0] - request.origin[0]).hypot(anchor[2] - request.origin[2]);
                    (distance <= max_length
                        && catalog.provider(key).is_some_and(|p| p.contains(anchor)))
                    .then_some((distance, key, anchor))
                })
                .collect();
            if anchors.is_empty() {
                continue;
            }
            anchors.sort_by(|a, b| a.0.total_cmp(&b.0));
            let ordinal = approach
                .attempts
                .get(&request.id)
                .map_or(0, |attempt| attempt.next);
            let try_drop = allowed_drop_id.is_some()
                && approach
                    .attempts
                    .get(&request.id)
                    .is_some_and(|attempt| attempt.try_drop);
            let (_, key, anchor) = anchors[ordinal % anchors.len()];
            let (at, repeats) = seek_backoff(
                approach
                    .attempts
                    .get(&request.id)
                    .map(|attempt| (&attempt.origin, attempt.repeats)),
                request.origin,
                now,
                policy.retry(),
            );
            approach.attempts.insert(
                request.id,
                Retry {
                    life: (bot.allocation_generation, request.ammo.equipment.0),
                    next: if allowed_drop_id.is_some() && !try_drop {
                        ordinal
                    } else {
                        ordinal.wrapping_add(1)
                    },
                    at: at.max(retry_at),
                    try_drop: allowed_drop_id.is_some() && !try_drop,
                    origin: request.origin,
                    repeats,
                },
            );
            approach.cursor = request.id.wrapping_add(1);
            queries.push((request, key, anchor, allowed_drop_id.filter(|_| try_drop)));
        }

        for (request, key, anchor, drop_id) in queries {
            let bot = &self.bot_registry.bots[&request.id];
            let route = if let Some(drop_id) = drop_id {
                let Some(rules) = self.fall_damage_rules.as_ref() else {
                    continue;
                };
                mesh.route_with_drop_costs(
                    request.origin,
                    anchor,
                    radius,
                    rules,
                    bot.health,
                    &BTreeMap::new(),
                    Some(drop_id),
                )
            } else {
                mesh.route(request.origin, anchor, radius)
            };
            let Ok(route) = route else {
                continue;
            };
            if route.length_m > max_length
                || !self.supply_route_clear(&mesh, &route, request.id, key, &actors, drop_id)
            {
                continue;
            }
            let generation = request.ammo.equipment.0;
            let deadline = request
                .planning
                .map_or(now, |planning| planning.started)
                .checked_add(duration)
                .ok_or("bot_resupply_seek_clock_overflow")?;
            let endpoint = route.projected_goal;
            let length = route.length_m;
            let serial = self
                .sequence
                .checked_add(1)
                .ok_or("bot_resupply_seek_sequence_exhausted")?;
            let movement_id = format!("population-supply-{}-{generation}-{serial}", request.id);
            let move_plan = if drop_id.is_some() {
                bot.target.prepare_registry_supply_route(
                    mesh.clone(),
                    route,
                    anchor,
                    movement_id.clone(),
                    duration.as_millis() as u32,
                    now,
                    drop_id,
                )
            } else {
                bot.target.prepare_registry_supply_walk(
                    mesh.clone(),
                    route,
                    anchor,
                    movement_id.clone(),
                    duration.as_millis() as u32,
                    now,
                )
            };
            let Ok(move_plan) = move_plan else {
                continue;
            };
            let previous_goal = self
                .bot_registry
                .config
                .as_ref()
                .filter(|c| !c.autonomous)
                .and_then(|config| config.slots.get(bot.slot))
                .and_then(|slot| slot.goal)
                .filter(|_| bot.move_attempted);
            let seek = Seek {
                scope: request.scope,
                policy_revision: revision,
                allocation: bot.allocation_generation,
                ammo: request.ammo,
                damage_revision: bot.damage_revision,
                provider: key,
                movement_id,
                projected_goal: endpoint,
                deadline,
                previous_goal,
                passage: drop_id.map(Passage::Pending),
                cancellation: None,
            };
            self.record(
                json!({
                    "op":"bot_resupply_seek_started","object_index":request.id,
                    "world_id":request.scope.world,"round":request.scope.round,
                    "allocation_generation":seek.allocation,"life_generation":generation,
                    "source_offset":key.source_offset,"name_token":key.name_token,
                    "movement_id":seek.movement_id,"origin":request.origin,"requested_goal":anchor,
                    "projected_goal":endpoint,"route_length_m":length,
                    "allowed_drop_id":drop_id,
                    "planning_id":request.planning.map(|planning| planning.id),
                    "timestamp_ms":now.as_millis() as u64,"deadline_ms":deadline.as_millis() as u64,
                }),
                wall,
            )?;
            let bot = self.bot_registry.bots.get_mut(&request.id).unwrap();
            move_plan.install(&mut bot.target);
            bot.resupply_seek = Some(seek);
            bot.resupply_planning = None;
        }
        Ok(())
    }

    fn supply_route_clear(
        &self,
        mesh: &navmesh::NavMesh,
        route: &navmesh::Route,
        id: u16,
        key: ProviderKey,
        actors: &crate::actor_collision::ActorGrid,
        own_drop: Option<u32>,
    ) -> bool {
        let (catalog, _) = self.bot_resupply.provider.as_ref().unwrap();
        let Some(provider) = catalog.provider(key) else {
            return false;
        };
        let endpoint = route.projected_goal;
        if route.status != navmesh::Status::Complete
            || route.waypoints.last() != Some(&endpoint)
            || !provider.contains(endpoint)
        {
            return false;
        }
        let body = self.bot_movement_body();
        let Ok(terrain) =
            bot_physics::WorldTerrain::new(self.collision_world.as_ref(), self.map_name())
        else {
            return false;
        };
        let Ok(supported) = terrain
            .start_motion(endpoint, body)
            .and_then(|motion| terrain.advance(motion, [0.; 3], 0.25, body))
        else {
            return false;
        };
        if !supported.grounded
            || supported.blocked
            || !provider.contains(supported.position)
            || !matches!(actors.occupied(id, supported.position, body), Ok(false))
        {
            return false;
        }
        let buffer = self
            .bot_registry
            .config
            .as_ref()
            .and_then(|c| c.objective_policy.as_ref())
            .map_or(0., |policy| policy.drop_entry_buffer_m);
        let drops = mesh
            .artifact()
            .drop_links
            .iter()
            .filter(|link| Some(link.id) != own_drop)
            .map(|link| protected_drop_bounds(link, buffer));
        let reserved = self
            .bot_registry
            .traffic_lanes(id)
            .into_iter()
            .flatten()
            .map(|[a, b]| Bounds::body(a, body).union(Bounds::body(b, body)));
        let others = self
            .bot_registry
            .bots
            .values()
            .filter(|bot| bot.target.object_index != id)
            .filter_map(|bot| bot.resupply_seek.as_ref())
            .map(|seek| Bounds::body(seek.projected_goal, body));
        !drops.chain(reserved).any(|bounds| {
            route.waypoints.windows(2).any(|pair| {
                overlaps(
                    bounds,
                    Bounds::body(pair[0], body).union(Bounds::body(pair[1], body)),
                )
            })
        }) && !others
            .into_iter()
            .any(|bounds| overlaps(bounds, Bounds::body(endpoint, body)))
    }
}

fn overlaps(a: Bounds, b: Bounds) -> bool {
    (0..3).all(|axis| a.min[axis] <= b.max[axis] && b.min[axis] <= a.max[axis])
}

#[cfg(test)]
mod seek_backoff_tests {
    use super::*;

    /// A bot whose supply walks keep stopping where they started (a blocked
    /// route) waits twice as long before each further seek, up to a minute;
    /// moving on resets the wait.
    #[test]
    fn repeated_seeks_from_one_spot_back_off_and_progress_resets_them() {
        let retry = Duration::from_millis(2500);
        let here = [10., 0., 10.];
        let mut now = Duration::from_secs(100);
        let (mut at, mut repeats) = seek_backoff(None, here, now, retry);
        assert_eq!((at, repeats), (now + retry, 0));
        let mut waits = Vec::new();
        for _ in 0..8 {
            now = at;
            // Stopped half a metre on: still no progress.
            let origin = [10.5, 0., 10.];
            (at, repeats) = seek_backoff(Some((&here, repeats)), origin, now, retry);
            waits.push((at - now).as_millis());
        }
        assert_eq!(
            waits,
            [5000, 10000, 20000, 40000, 60000, 60000, 60000, 60000]
        );
        // A seek from somewhere else is ordinary again.
        let (next, reset) = seek_backoff(Some((&here, repeats)), [20., 0., 10.], now, retry);
        assert_eq!((next - now, reset), (retry, 0));
    }
}
