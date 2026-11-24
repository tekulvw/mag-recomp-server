//! One timed reserve refill for a bot idling at a qualified static provider.
use crate::{
    bot_registry::WorldBot,
    bot_resupply_catalog::{Catalog, ProviderKey},
    development_target::BotPhase,
    *,
};
use std::collections::{BTreeMap, BTreeSet};

#[path = "bot_resupply_approach.rs"]
mod approach;
pub(crate) use approach::Seek;

#[derive(Clone, Copy)]
pub(crate) struct PlanningReservation {
    id: u64,
    scope: Scope,
    policy_revision: u64,
    allocation: u32,
    ammo: AmmoBinding,
    damage_revision: u64,
    origin: [f64; 3],
    started: Duration,
    deadline: Duration,
    window: Duration,
}

impl PlanningReservation {
    fn interrupted(
        self,
        bot: &WorldBot,
        scope: Option<Scope>,
        revision: Option<u64>,
        operators: &BTreeSet<(u16, u32)>,
        now: Duration,
    ) -> Option<&'static str> {
        if Some(self.scope) != scope
            || Some(self.policy_revision) != revision
            || self.allocation != bot.allocation_generation
        {
            Some("planning_scope_changed")
        } else if now >= self.deadline {
            Some("planning_window_expired")
        } else if !has_ammo_binding(bot, None, self.ammo) {
            Some("planning_equipment_changed")
        } else if self.damage_revision != bot.damage_revision {
            Some("combat_interrupt")
        } else if !planning_origin(bot, operators).is_some_and(|origin| {
            origin
                .iter()
                .zip(self.origin)
                .all(|(a, b)| (*a - b).abs() <= 1e-6)
        }) {
            Some("planning_lost_idle_ownership")
        } else {
            None
        }
    }
}

#[derive(Default)]
pub(super) struct State {
    provider: Option<(Catalog, Binding)>,
    revision: u64,
    approach: Option<Box<approach::Approach>>,
    approach_revision: u64,
}

struct Binding {
    application: u32,
    map: String,
    mode: i8,
    spawns: Option<String>,
    collision: String,
}

impl Binding {
    fn capture<W: Write>(engine: &Engine<W>) -> Option<Self> {
        let spawn = engine.spawn_catalog.as_ref()?;
        let world = engine.collision_world.as_ref()?;
        Some(Self {
            application: engine.wire_profile.application_id(),
            map: spawn.map_name().into(),
            mode: spawn.match_profile().game_type,
            spawns: spawn.source_sha256().map(str::to_owned),
            collision: world.source_sha256().into(),
        })
    }

    fn matches<W: Write>(&self, engine: &Engine<W>) -> bool {
        engine.wire_profile.application_id() == self.application
            && engine.spawn_catalog.as_ref().is_some_and(|spawn| {
                spawn.map_name() == self.map
                    && spawn.match_profile().game_type == self.mode
                    && spawn.source_sha256() == self.spawns.as_deref()
            })
            && engine.collision_world.as_ref().is_some_and(|world| {
                world.metadata().map_name == self.map && world.source_sha256() == self.collision
            })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct AmmoBinding {
    pub(super) equipment: bot_firearms::EquipmentKey,
    pub(super) ammo_pool: u8,
    pub(super) reserve_cap: u32,
}

#[derive(Clone, Copy, Default)]
pub(super) enum Threat {
    #[default]
    Deferred,
    Quiet,
    Enemy,
}

#[derive(Clone, Copy)]
pub(super) struct Observation {
    pub(super) ammo: Option<AmmoBinding>,
    pub(super) threat: Threat,
    pub(super) damage_revision: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Scope {
    world: u32,
    round: u64,
    catalog_revision: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Session {
    scope: Scope,
    allocation: u32,
    ammo: AmmoBinding,
    provider: ProviderKey,
    origin: [f64; 3],
    damage_revision: u64,
    due: Duration,
}

impl Session {
    pub(super) fn due(self, now: Duration) -> bool {
        now >= self.due
    }
}

enum Plan {
    Start(Session),
    Cancel(Session, &'static str),
    Pulse(Session),
}

#[derive(Clone, Copy)]
struct Frame<'a> {
    bot: &'a WorldBot,
    catalog: &'a Catalog,
    scope: Scope,
    origin: [f64; 3],
    observed: Option<Observation>,
}

impl Plan {
    fn prepare(
        bot: &WorldBot,
        frame: Option<Frame<'_>>,
        now: Duration,
        selected_provider: Option<ProviderKey>,
    ) -> Result<Option<Self>, &'static str> {
        let before = bot.resupply;
        let Some(frame) = frame else {
            return Ok(before.map(|session| Self::Cancel(session, "invalid_service_scope_or_body")));
        };
        if let Some(session) = before {
            let cancel = |reason| Ok(Some(Self::Cancel(session, reason)));
            if session.scope != frame.scope || session.allocation != bot.allocation_generation {
                return cancel("changed_service_allocation");
            }
            if !frame.has_ammo_binding(session.ammo) {
                return cancel("changed_service_equipment");
            }
            if !frame
                .catalog
                .provider(session.provider)
                .is_some_and(|p| p.contains(frame.origin))
            {
                return cancel("left_service_provider");
            }
            // Use the movement publisher's physical displacement threshold.
            if frame
                .origin
                .iter()
                .zip(session.origin)
                .any(|(a, b)| (*a - b).abs() > 1e-6)
            {
                return cancel("moved_during_service");
            }
            if bot.damage_revision != session.damage_revision
                || frame
                    .observed
                    .is_some_and(|seen| matches!(seen.threat, Threat::Enemy))
            {
                return cancel("combat_interrupt");
            }
            if !session.due(now)
                || !frame
                    .observed
                    .is_some_and(|seen| matches!(seen.threat, Threat::Quiet))
            {
                return Ok(None);
            }
            return Ok(Some(Self::Pulse(session)));
        }
        let Some(ammo) = frame.start_ammo() else {
            return Ok(None);
        };
        let Some(provider) = selected_provider.or_else(|| frame.catalog.select(frame.origin))
        else {
            return Ok(None);
        };
        let source = frame
            .catalog
            .provider(provider)
            .expect("selected current provider");
        let interval = Duration::try_from_secs_f32(source.tick_rate_seconds())
            .map_err(|_| "invalid_bot_service_interval")?;
        let due = now
            .checked_add(interval)
            .ok_or("bot_service_clock_overflow")?;
        Ok(Some(Self::Start(Session {
            scope: frame.scope,
            allocation: bot.allocation_generation,
            ammo,
            provider,
            origin: frame.origin,
            damage_revision: bot.damage_revision,
            due,
        })))
    }
}

impl Frame<'_> {
    fn start_ammo(&self) -> Option<AmmoBinding> {
        self.observed
            .filter(|seen| {
                matches!(seen.threat, Threat::Quiet)
                    && self.bot.damage_revision == seen.damage_revision
            })
            .and_then(|seen| seen.ammo)
            .filter(|ammo| self.has_ammo_binding(*ammo))
    }

    fn has_ammo_binding(&self, binding: AmmoBinding) -> bool {
        has_ammo_binding(self.bot, self.observed, binding)
    }
}

fn has_ammo_binding(bot: &WorldBot, observed: Option<Observation>, binding: AmmoBinding) -> bool {
    binding.reserve_cap > 0
        && bot.firearm.is_resupply_dry()
        && bot.firearm.equipment_matches(binding.equipment)
        && bot_firearms::current_equipment_key(bot) == Some(binding.equipment)
        && bot.target.loadout.primary.ammo_pool == binding.ammo_pool
        && observed.is_none_or(|seen| seen.ammo == Some(binding))
}

fn planning_origin(bot: &WorldBot, operators: &BTreeSet<(u16, u32)>) -> Option<[f64; 3]> {
    let runtime = bot.target.runtime?;
    let motion = runtime.physics_motion?;
    (matches!(runtime.phase, BotPhase::Complete)
        && bot_firearms::safe(bot)
        && motion.vertical_velocity == 0.
        && motion.position.iter().all(|p| p.is_finite())
        && bot.holding_access.is_none()
        && !operators.contains(&(bot.target.object_index, runtime.generation)))
    .then_some(motion.position)
}

impl<W: Write> Engine<W> {
    /// Enable one-pulse bot service for the qualified direct retail worker.
    pub fn with_bot_resupply_catalog(mut self, catalog: Catalog) -> Result<Self, &'static str> {
        self.configure_bot_resupply_catalog(catalog)?;
        Ok(self)
    }

    /// Configure an already constructed worker without copying its engine state.
    pub fn configure_bot_resupply_catalog(&mut self, catalog: Catalog) -> Result<(), &'static str> {
        if !self.bot_registry.enabled()
            || !self
                .bot_registry
                .config
                .as_ref()
                .is_some_and(|config| config.firearms.enabled)
            || self.match_profiles.is_some()
            || self.item_catalog.retail_combat().is_err()
        {
            return Err("bot_resupply_requires_retail_firearm_population");
        }
        let binding = Binding::capture(self).ok_or("bot_resupply_requires_spawn_and_collision")?;
        let spawn = binding
            .spawns
            .as_deref()
            .ok_or("bot_resupply_requires_spawn_fingerprint")?;
        if !binding.matches(self) {
            return Err("bot_resupply_catalog_binding_mismatch");
        }
        catalog.validate_binding(
            binding.application,
            &binding.map,
            binding.mode,
            spawn,
            &binding.collision,
        )?;
        let revision = self
            .bot_resupply
            .revision
            .checked_add(1)
            .ok_or("bot_resupply_catalog_revision_exhausted")?;
        self.bot_resupply.provider = Some((catalog, binding));
        self.bot_resupply.revision = revision;
        self.bot_resupply.approach = None;
        Ok(())
    }

    /// Whether a selected profile can install its bot service without
    /// exhausting a revision after the selection is recorded.
    pub(crate) fn bot_resupply_revisions_available(&self) -> bool {
        self.bot_resupply.revision < u64::MAX && self.bot_resupply.approach_revision < u64::MAX
    }

    /// Replace world-bound bot service with the newly selected profile's inputs.
    /// Startup already validated them against this exact world; a profile
    /// without them leaves bot service disabled rather than reusing another
    /// world's providers.
    pub(crate) fn install_profile_bot_resupply(
        &mut self,
        catalog: Option<Catalog>,
        approach: Option<crate::bot_resupply_approach_policy::Policy>,
    ) {
        self.bot_resupply.provider = None;
        self.bot_resupply.approach = None;
        let Some(catalog) = catalog else {
            return;
        };
        let Some(binding) = Binding::capture(self) else {
            return;
        };
        self.bot_resupply.provider = Some((catalog, binding));
        self.bot_resupply.revision = self.bot_resupply.revision.saturating_add(1);
        if let Some(policy) = approach
            && self.navmesh_world.is_some()
        {
            // Revisions were checked before selection was recorded.
            let _ = self.install_resupply_approach(policy);
        }
    }

    #[cfg(test)]
    pub(super) fn install_synthetic_bot_resupply(&mut self, catalog: Catalog) {
        let binding = Binding::capture(self).unwrap();
        self.bot_resupply.provider = Some((catalog, binding));
        self.bot_resupply.revision += 1;
    }
    #[cfg(test)]
    pub(super) fn install_synthetic_resupply_approach(
        &mut self,
        policy: crate::bot_resupply_approach_policy::Policy,
    ) {
        self.install_resupply_approach(policy).unwrap();
    }

    pub(super) fn bot_resupply_enabled(&self) -> bool {
        self.bot_resupply.provider.is_some()
    }

    pub(crate) fn tick_resupply_planning(
        &mut self,
        now: Duration,
        wall: u64,
    ) -> Result<(), &'static str> {
        let context = self.resupply_planning_context();
        if context.is_none()
            && !self
                .bot_registry
                .bots
                .values()
                .any(|bot| bot.resupply_planning.is_some())
        {
            return Ok(());
        }
        let scope = self.resupply_scope(now);
        let operators = self.resupply_operators();
        let ids: Vec<_> = self
            .bot_registry
            .bots
            .iter()
            .filter_map(|(&id, bot)| {
                (bot.resupply_planning.is_some()
                    || (!bot.has_supply_action()
                        && now >= bot.resupply_planning_retry_at
                        && bot.firearm.is_resupply_dry()))
                .then_some(id)
            })
            .collect();
        for id in ids {
            let bot = &self.bot_registry.bots[&id];
            if let Some(planning) = bot.resupply_planning {
                let revision = context.map(|(revision, _, _)| revision);
                if let Some(reason) = planning.interrupted(bot, scope, revision, &operators, now) {
                    self.cancel_resupply_planning(id, planning, reason, now, wall)?;
                }
                continue;
            }
            let Some((policy_revision, allowed_drop_id, window)) = context else {
                continue;
            };
            let Some(scope) = scope else { continue };
            let Some(origin) = planning_origin(bot, &operators) else {
                continue;
            };
            let Some(ammo) = bot_firearms::resupply_planning_ammo(&self.item_catalog, bot) else {
                continue;
            };
            let planning = PlanningReservation {
                id: self
                    .sequence
                    .checked_add(1)
                    .ok_or("bot_resupply_planning_sequence_exhausted")?,
                scope,
                policy_revision,
                allocation: bot.allocation_generation,
                ammo,
                damage_revision: bot.damage_revision,
                origin,
                started: now,
                deadline: now
                    .checked_add(window)
                    .ok_or("bot_resupply_planning_clock_overflow")?,
                window,
            };
            self.record(json!({
                "op":"bot_resupply_planning_started","planning_id":planning.id,"object_index":id,
                "world_id":scope.world,"round":scope.round,"allocation_generation":planning.allocation,
                "life_generation":ammo.equipment.0,"origin":origin,"allowed_drop_id":allowed_drop_id,
                "timestamp_ms":now.as_millis(),"deadline_ms":planning.deadline.as_millis(),
            }), wall)?;
            self.bot_registry
                .bots
                .get_mut(&id)
                .unwrap()
                .resupply_planning = Some(planning);
        }
        Ok(())
    }

    fn cancel_resupply_planning(
        &mut self,
        id: u16,
        planning: PlanningReservation,
        reason: &'static str,
        now: Duration,
        wall: u64,
    ) -> Result<(), &'static str> {
        let retry_at = now
            .checked_add(planning.window)
            .ok_or("bot_resupply_planning_clock_overflow")?;
        self.record(
            json!({
                "op":"bot_resupply_planning_cancelled","planning_id":planning.id,"object_index":id,
                "reason":reason,"timestamp_ms":now.as_millis(),"retry_at_ms":retry_at.as_millis(),
            }),
            wall,
        )?;
        let bot = self.bot_registry.bots.get_mut(&id).unwrap();
        bot.resupply_planning = None;
        bot.resupply_planning_retry_at = retry_at;
        Ok(())
    }

    fn resupply_scope(&self, now: Duration) -> Option<Scope> {
        (self.match_started
            && self.match_terminal.is_none()
            && self.round_combat_allowed(now)
            && self.item_catalog.retail_combat().is_ok()
            && self
                .bot_resupply
                .provider
                .as_ref()
                .is_some_and(|(_, binding)| binding.matches(self))
            && self
                .bot_registry
                .config
                .as_ref()
                .is_some_and(|config| config.firearms.enabled))
        .then(|| {
            self.active
                .iter()
                .chain(self.peers.values())
                .find(|actor| actor.ready_for_updates && actor.initial_objects.is_some())
        })
        .flatten()
        .map(|actor| Scope {
            world: actor.reservation.world,
            round: self.round_number,
            catalog_revision: self.bot_resupply.revision,
        })
    }

    fn cancel_resupply_seek(
        &mut self,
        id: u16,
        reason: &'static str,
        now: Duration,
        wall: u64,
    ) -> Result<(), &'static str> {
        let seek = self.bot_registry.bots[&id].resupply_seek.as_ref().unwrap();
        let defer = self.resupply_passage_obligation(id, seek);
        self.record(
            json!({
                "op":"bot_resupply_seek_cancelled","object_index":id,
                "movement_id":seek.parent_id(),"source_offset":seek.provider.source_offset,
                "name_token":seek.provider.name_token,"reason":reason,
                "awaiting_passage_release":defer,"timestamp_ms":now.as_millis(),
            }),
            wall,
        )?;
        if defer {
            self.bot_registry
                .bots
                .get_mut(&id)
                .unwrap()
                .resupply_seek
                .as_mut()
                .unwrap()
                .latch_cancellation(reason);
        } else {
            self.finish_resupply_seek(id, now);
        }
        Ok(())
    }

    /// Known interruptions must be latched before that tick can grant or enter a passage.
    pub(crate) fn tick_resupply_drop_interruptions(
        &mut self,
        now: Duration,
        wall: u64,
    ) -> Result<(), &'static str> {
        let scope = self.resupply_scope(now);
        let revision = self.resupply_approach_revision();
        let interruptions: Vec<_> = self
            .bot_registry
            .bots
            .iter()
            .filter_map(|(&id, bot)| {
                let seek = bot.resupply_seek.as_ref()?;
                seek.allowed_drop_id()?;
                if seek.cancellation_pending() {
                    return None;
                }
                seek.drop_interrupt_reason(bot, scope, revision, None, now)
                    .map(|reason| (id, reason))
            })
            .collect();
        for (id, reason) in interruptions {
            self.cancel_resupply_seek(id, reason, now, wall)?;
        }
        Ok(())
    }

    /// Runs after all normal firearm commits for this pass. A later shooter in
    /// that pass can still interrupt a due session before its refill is applied.
    pub(super) fn tick_bot_resupply(
        &mut self,
        now: Duration,
        wall: u64,
        observed: &BTreeMap<u16, Observation>,
    ) -> Result<(), &'static str> {
        if !self.bot_resupply_enabled() {
            return Ok(());
        }
        let scope = self.resupply_scope(now);
        let operators = self.resupply_operators();
        let ids: Vec<_> = self
            .bot_registry
            .bots
            .iter()
            .filter(|(id, bot)| bot.has_supply_action() || observed.contains_key(id))
            .map(|(&id, _)| id)
            .collect();
        let approach_revision = self.resupply_approach_revision();
        let mut credits = self.resupply_query_budget();
        let mut outside = Vec::new();
        for id in ids {
            let bot = &self.bot_registry.bots[&id];
            let observation = observed.get(&id).copied();
            let planning = bot.resupply_planning;
            if let Some(planning) = planning {
                let revision = self
                    .resupply_planning_context()
                    .map(|(revision, _, _)| revision);
                let reason = planning
                    .interrupted(bot, scope, revision, &operators, now)
                    .or_else(|| {
                        observation.and_then(|seen| {
                            if matches!(seen.threat, Threat::Enemy) {
                                Some("combat_interrupt")
                            } else if seen.ammo != Some(planning.ammo) {
                                Some("planning_equipment_changed")
                            } else {
                                None
                            }
                        })
                    });
                if let Some(reason) = reason {
                    self.cancel_resupply_planning(id, planning, reason, now, wall)?;
                    continue;
                }
            }
            let frame = scope.and_then(|scope| {
                let (catalog, binding) = self.bot_resupply.provider.as_ref()?;
                let runtime = bot.target.runtime?;
                let motion = runtime.physics_motion?;
                if !binding.matches(self)
                    || !(matches!(runtime.phase, BotPhase::Complete)
                        || bot
                            .resupply_seek
                            .as_ref()
                            .is_some_and(|seek| seek.owns(bot)))
                    || !bot_firearms::safe_for_supply_observation(bot)
                    || motion.vertical_velocity != 0.
                    || bot.holding_access.is_some()
                    || !motion.position.iter().all(|p| p.is_finite())
                    || operators.contains(&(id, runtime.generation))
                {
                    return None;
                }
                Some(Frame {
                    bot,
                    catalog,
                    scope,
                    origin: motion.position,
                    observed: observation,
                })
            });
            let mut arrived = None;
            if let Some(seek) = &bot.resupply_seek {
                match seek.step(
                    bot,
                    scope,
                    observation,
                    frame.as_ref(),
                    approach_revision,
                    now,
                ) {
                    approach::Disposition::Wait => continue,
                    approach::Disposition::Cancel(reason) => {
                        self.cancel_resupply_seek(id, reason, now, wall)?;
                        continue;
                    }
                    approach::Disposition::FinishCancellation => {
                        if !self.resupply_passage_obligation(id, seek) {
                            self.finish_resupply_seek(id, now);
                        }
                        continue;
                    }
                    approach::Disposition::ContinueAfterPassage(drop_id) => {
                        if credits > 0 {
                            credits -= 1;
                            if !self.continue_resupply_after_passage(id, drop_id, now, wall)? {
                                self.cancel_resupply_seek(
                                    id,
                                    "seek_continuation_unavailable",
                                    now,
                                    wall,
                                )?;
                            }
                        }
                        continue;
                    }
                    approach::Disposition::Arrive => arrived = Some(seek),
                }
            }
            let Some(plan) = Plan::prepare(bot, frame, now, arrived.map(|seek| seek.provider))?
            else {
                if approach_revision.is_some()
                    && (!bot.has_supply_action() || planning.is_some())
                    && let Some(frame) = frame
                    && let Some(ammo) = frame.start_ammo()
                {
                    outside.push(approach::Request {
                        id,
                        scope: frame.scope,
                        ammo,
                        origin: frame.origin,
                        planning,
                    });
                }
                continue;
            };
            let (session, event, reason) = match plan {
                Plan::Start(session) => (session, "bot_resupply_started", None),
                Plan::Cancel(session, reason) => (session, "bot_resupply_cancelled", Some(reason)),
                Plan::Pulse(session) => (session, "bot_resupply_pulse", None),
            };
            let service_record = json!({
                "op":event,"reason":reason,"object_index":id,"world_id":session.scope.world,
                "round":session.scope.round,"allocation_generation":session.allocation,
                "life_generation":session.ammo.equipment.0,"weapon_item":session.ammo.equipment.4,
                "source_offset":session.provider.source_offset,"name_token":session.provider.name_token,
                "timestamp_ms":now.as_millis(),"due_ms":session.due.as_millis(),
                "reserve_before":bot.firearm.reserve,
                "reserve_after":if matches!(plan, Plan::Pulse(_)) { session.ammo.reserve_cap } else { bot.firearm.reserve },
                "magazine":bot.firearm.magazine,"ammo_pool":session.ammo.ammo_pool,"origin":session.origin,
                "seek_movement_id":arrived.map(|seek| &seek.movement_id),
                "planning_id":planning.map(|plan| plan.id),
            });
            let completed_seek = arrived.is_some();
            self.record(service_record, wall)?;
            if completed_seek {
                self.finish_resupply_seek(id, now);
            }
            let bot = self
                .bot_registry
                .bots
                .get_mut(&id)
                .expect("planned current bot");
            if planning.is_some() {
                bot.resupply_planning = None;
            }
            match plan {
                Plan::Start(session) => bot.resupply = Some(session),
                Plan::Cancel(_, _) => bot.resupply = None,
                Plan::Pulse(session) => {
                    bot.firearm.reserve = session.ammo.reserve_cap;
                    bot.resupply = None;
                }
            }
        }
        self.plan_resupply_seeks(now, wall, outside, credits)
    }

    fn resupply_operators(&self) -> BTreeSet<(u16, u32)> {
        self.objective_catalog
            .as_ref()
            .zip(self.objective_world.as_ref())
            .into_iter()
            .flat_map(|(catalog, world)| {
                catalog
                    .objects
                    .iter()
                    .filter_map(|object| world.interaction_owner(object.object_index))
            })
            .map(|(player, generation, _)| (player, generation))
            .collect()
    }
}
