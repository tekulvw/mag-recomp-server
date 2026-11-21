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
}

impl Seek {
    pub(crate) fn owns(&self, bot: &WorldBot) -> bool {
        bot.allocation_generation == self.allocation
            && bot
                .target
                .runtime
                .is_some_and(|r| r.generation == self.ammo.equipment.0)
            && bot.target.owns_registry_move(&self.movement_id)
    }

    pub(super) fn step(
        &self,
        frame: Option<&Frame<'_>>,
        revision: Option<u64>,
        now: Duration,
    ) -> Disposition {
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
}

pub(super) struct Request {
    pub(super) id: u16,
    pub(super) scope: Scope,
    pub(super) ammo: AmmoBinding,
    pub(super) origin: [f64; 3],
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
        if !seek.owns(bot) || !bot.target.stop_owned_supply_walk(&seek.movement_id) {
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
}

impl<W: Write> Engine<W> {
    pub(super) fn plan_resupply_seeks(
        &mut self,
        now: Duration,
        wall: u64,
        mut requests: Vec<Request>,
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
        let max_length = policy.max_route_length_m();
        let duration = policy.max_duration();
        let deadline = now
            .checked_add(duration)
            .ok_or("bot_resupply_seek_clock_overflow")?;
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
            if queries.len() >= policy.query_budget_per_tick() {
                break;
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
            let (_, key, anchor) = anchors[ordinal % anchors.len()];
            approach.attempts.insert(
                request.id,
                Retry {
                    life: (bot.allocation_generation, request.ammo.equipment.0),
                    next: ordinal.wrapping_add(1),
                    at: retry_at,
                },
            );
            approach.cursor = request.id.wrapping_add(1);
            queries.push((request, key, anchor));
        }

        for (request, key, anchor) in queries {
            let Ok(route) = mesh.route(request.origin, anchor, radius) else {
                continue;
            };
            if route.length_m > max_length
                || !self.supply_walk_clear(&mesh, &route, request.id, key, &actors)
            {
                continue;
            }
            let bot = &self.bot_registry.bots[&request.id];
            let generation = request.ammo.equipment.0;
            let endpoint = route.projected_goal;
            let length = route.length_m;
            let serial = self
                .sequence
                .checked_add(1)
                .ok_or("bot_resupply_seek_sequence_exhausted")?;
            let movement_id = format!("population-supply-{}-{generation}-{serial}", request.id);
            let Ok(move_plan) = bot.target.prepare_registry_supply_walk(
                mesh.clone(),
                route,
                anchor,
                movement_id.clone(),
                duration.as_millis() as u32,
                now,
            ) else {
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
            };
            self.record(
                json!({
                    "op":"bot_resupply_seek_started","object_index":request.id,
                    "world_id":request.scope.world,"round":request.scope.round,
                    "allocation_generation":seek.allocation,"life_generation":generation,
                    "source_offset":key.source_offset,"name_token":key.name_token,
                    "movement_id":seek.movement_id,"origin":request.origin,"requested_goal":anchor,
                    "projected_goal":endpoint,"route_length_m":length,
                    "timestamp_ms":now.as_millis() as u64,"deadline_ms":deadline.as_millis() as u64,
                }),
                wall,
            )?;
            let bot = self.bot_registry.bots.get_mut(&request.id).unwrap();
            move_plan.install(&mut bot.target);
            bot.resupply_seek = Some(seek);
        }
        Ok(())
    }

    fn supply_walk_clear(
        &self,
        mesh: &navmesh::NavMesh,
        route: &navmesh::Route,
        id: u16,
        key: ProviderKey,
        actors: &crate::actor_collision::ActorGrid,
    ) -> bool {
        let (catalog, _) = self.bot_resupply.provider.as_ref().unwrap();
        let provider = catalog.provider(key).unwrap();
        let endpoint = route.projected_goal;
        if route.status != navmesh::Status::Complete
            || route.waypoints.last() != Some(&endpoint)
            || !provider.contains(endpoint)
        {
            return false;
        }
        let body = bot_physics::Body::default();
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
