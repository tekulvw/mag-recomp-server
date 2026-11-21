//! One timed reserve refill for a bot idling at a qualified static provider.
use crate::{
    bot_registry::WorldBot,
    bot_resupply_catalog::{Catalog, ProviderKey},
    development_target::BotPhase,
    *,
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
pub(super) struct State {
    provider: Option<(Catalog, Binding)>,
    revision: u64,
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
        let Some(ammo) = frame
            .observed
            .filter(|seen| {
                matches!(seen.threat, Threat::Quiet) && bot.damage_revision == seen.damage_revision
            })
            .and_then(|seen| seen.ammo)
            .filter(|ammo| frame.has_ammo_binding(*ammo))
        else {
            return Ok(None);
        };
        let Some(provider) = frame.catalog.select(frame.origin) else {
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
    fn has_ammo_binding(&self, binding: AmmoBinding) -> bool {
        binding.reserve_cap > 0
            && self.bot.firearm.is_resupply_dry()
            && self.bot.firearm.equipment_matches(binding.equipment)
            && bot_firearms::current_equipment_key(self.bot) == Some(binding.equipment)
            && self.bot.target.loadout.primary.ammo_pool == binding.ammo_pool
            && self.observed.is_none_or(|seen| seen.ammo == Some(binding))
    }
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
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn install_synthetic_bot_resupply(&mut self, catalog: Catalog) {
        let binding = Binding::capture(self).unwrap();
        self.bot_resupply.provider = Some((catalog, binding));
        self.bot_resupply.revision += 1;
    }

    pub(super) fn bot_resupply_enabled(&self) -> bool {
        self.bot_resupply.provider.is_some()
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
        let scope = (self.match_started
            && self.match_terminal.is_none()
            && self.round_combat_allowed(now)
            && self.item_catalog.retail_combat().is_ok()
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
        });
        let operators: BTreeSet<_> = self
            .objective_catalog
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
            .collect();
        let ids: Vec<_> = self
            .bot_registry
            .bots
            .iter()
            .filter(|(id, bot)| bot.resupply.is_some() || observed.contains_key(id))
            .map(|(&id, _)| id)
            .collect();
        for id in ids {
            let bot = &self.bot_registry.bots[&id];
            let frame = scope.and_then(|scope| {
                let (catalog, binding) = self.bot_resupply.provider.as_ref()?;
                let runtime = bot.target.runtime?;
                let motion = runtime.physics_motion?;
                if !binding.matches(self)
                    || !matches!(runtime.phase, BotPhase::Complete)
                    || !bot_firearms::safe(bot)
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
                    observed: observed.get(&id).copied(),
                })
            });
            let Some(plan) = Plan::prepare(bot, frame, now)? else {
                continue;
            };
            let (session, event, reason) = match plan {
                Plan::Start(session) => (session, "bot_resupply_started", None),
                Plan::Cancel(session, reason) => (session, "bot_resupply_cancelled", Some(reason)),
                Plan::Pulse(session) => (session, "bot_resupply_pulse", None),
            };
            self.record(json!({
                "op":event,"reason":reason,"object_index":id,"world_id":session.scope.world,
                "round":session.scope.round,"allocation_generation":session.allocation,
                "life_generation":session.ammo.equipment.0,"weapon_item":session.ammo.equipment.4,
                "source_offset":session.provider.source_offset,"name_token":session.provider.name_token,
                "timestamp_ms":now.as_millis(),"due_ms":session.due.as_millis(),
                "reserve_before":bot.firearm.reserve,
                "reserve_after":if matches!(plan, Plan::Pulse(_)) { session.ammo.reserve_cap } else { bot.firearm.reserve },
                "magazine":bot.firearm.magazine,"ammo_pool":session.ammo.ammo_pool,"origin":session.origin,
            }), wall)?;
            let bot = self
                .bot_registry
                .bots
                .get_mut(&id)
                .expect("planned current bot");
            match plan {
                Plan::Start(session) => bot.resupply = Some(session),
                Plan::Cancel(_, _) => bot.resupply = None,
                Plan::Pulse(session) => {
                    bot.firearm.reserve = session.ammo.reserve_cap;
                    bot.resupply = None;
                }
            }
        }
        Ok(())
    }
}
