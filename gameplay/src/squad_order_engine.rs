//! Authenticated manual squad orders and retained reliable publication.
use crate::{
    squad_orders::{Rules, State, Waypoint},
    *,
};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Snapshot {
    tagged_object: u32,
    waypoint: Option<Waypoint>,
}
#[derive(Clone)]
pub(crate) struct Runtime {
    pub(crate) rules: Rules,
    pub(crate) state: State,
    published: BTreeMap<String, BTreeMap<u16, Snapshot>>,
}
impl Runtime {
    pub(crate) fn new(rules: Rules) -> Self {
        Self {
            rules,
            state: State::default(),
            published: BTreeMap::new(),
        }
    }
    pub(crate) fn reset(&mut self) {
        self.state = State::default();
        self.published.clear();
    }
}
pub(crate) struct Candidate {
    runtime: Runtime,
    acquisition: Option<acquisition_gameplay_engine::Runtime>,
    domination: Option<domination_gameplay_provider::World>,
}

impl<W: Write> Engine<W> {
    pub fn with_squad_orders(mut self, rules: Rules) -> Result<Self, &'static str> {
        if self.active.is_some() || !self.peers.is_empty() {
            return Err("squad_orders_require_idle_worker");
        }
        if !self
            .leadership
            .as_ref()
            .is_some_and(|runtime| runtime.supports_match_profiles())
        {
            return Err("squad_orders_require_native_leadership");
        }
        self.squad_orders = Some(Runtime::new(rules));
        Ok(self)
    }
    /// Sabotage has no source gameplay provider; the order runtime owns its tag.
    pub(crate) fn sabotage_squad_assignments(&self) -> bool {
        self.match_profile().game_type == 0
            && self.acquisition_gameplay.is_none()
            && self.domination_gameplay.is_none()
            && self.objective_catalog.is_some()
    }
    fn squad_order_authority(&self, actor: &Active) -> Result<u16, &'static str> {
        if !self.match_started || self.match_terminal.is_some() {
            return Err("match_not_playing");
        }
        if !matches!(self.match_profile().game_type, 1 | 2) && !self.sabotage_squad_assignments() {
            return Err("squad_orders_mode_unavailable");
        }
        if !actor.entered_game || !actor.ready_for_updates || actor.initial_objects.is_none() {
            return Err("squad_order_actor_not_created");
        }
        let squad = actor.squad_catalog.squad_index();
        if !self.held_squad_order_leader(actor) {
            return Err("squad_order_requires_held_squad_leader");
        }
        Ok(squad)
    }
    fn squad_order_candidate(&self) -> Result<Candidate, &'static str> {
        Ok(Candidate {
            runtime: self
                .squad_orders
                .clone()
                .ok_or("squad_orders_unconfigured")?,
            acquisition: self.acquisition_gameplay.clone(),
            domination: self.domination_gameplay.clone(),
        })
    }
    fn set_candidate_tag(
        &self,
        candidate: &mut Candidate,
        squad: u16,
        target: u32,
    ) -> Result<(), &'static str> {
        if let Some(runtime) = &mut candidate.acquisition {
            runtime.state.set_manual_tag(squad, target)?;
        } else if let Some(world) = &mut candidate.domination {
            world.set_manual_tag(
                squad,
                target,
                self.objective_world
                    .as_ref()
                    .ok_or("objective_world_required")?,
                self.spawn_catalog
                    .as_ref()
                    .ok_or("spawn_catalog_required")?,
                match self.domination_round.as_ref().map(|round| round.phase()) {
                    Some(acquisition_round::Phase::Ending { .. }) => {
                        replication::MatchPhase::EndCountdown
                    }
                    Some(acquisition_round::Phase::Finished) => replication::MatchPhase::PostGame,
                    _ if self.match_started => replication::MatchPhase::Playing,
                    _ => replication::MatchPhase::PreGame,
                },
            )?;
        } else if self.sabotage_squad_assignments() {
            self.objective_catalog
                .as_ref()
                .ok_or("retail_objective_catalog_required")?
                .initial_assignment(squad)?;
            candidate.runtime.state.set_manual_tag(squad, target)?;
        } else {
            return Err("manual_tag_requires_gameplay_provider");
        }
        Ok(())
    }
    pub(crate) fn plan_squad_tag(
        &self,
        actor: &Active,
        target: u32,
        now: u32,
    ) -> Result<Candidate, &'static str> {
        let squad = self.squad_order_authority(actor)?;
        let mut candidate = self.squad_order_candidate()?;
        if target != u32::MAX {
            if !candidate
                .runtime
                .state
                .can_set(squad, now, &candidate.runtime.rules)
            {
                return Err("squad_order_cooldown");
            }
            if !self.squad_order_target_exists(target) {
                return Err("squad_order_target_unavailable");
            }
        }
        self.set_candidate_tag(&mut candidate, squad, target)?;
        if target != u32::MAX {
            candidate.runtime.state.stamp_set(squad, now);
        }
        Ok(candidate)
    }
    pub(crate) fn plan_squad_waypoint(
        &self,
        actor: &Active,
        position: replication::CharacterPosition,
        clear: bool,
        now: u32,
    ) -> Result<Candidate, &'static str> {
        let squad = self.squad_order_authority(actor)?;
        let mut candidate = self.squad_order_candidate()?;
        if clear {
            candidate.runtime.state.clear_waypoint(squad);
        } else {
            position.validate()?;
            if !candidate
                .runtime
                .state
                .can_set(squad, now, &candidate.runtime.rules)
            {
                return Err("squad_order_cooldown");
            }
            candidate.runtime.state.set_waypoint(squad, position, now);
        }
        Ok(candidate)
    }
    pub(crate) fn plan_removed_squad_orders(
        &self,
        squads: &[u16],
    ) -> Result<Option<Candidate>, &'static str> {
        if self.squad_orders.is_none() || squads.is_empty() {
            return Ok(None);
        }
        let mut candidate = self.squad_order_candidate()?;
        for &squad in squads {
            candidate.runtime.state.clear_waypoint(squad);
            if candidate
                .acquisition
                .as_ref()
                .is_some_and(|r| r.state.squad(squad).is_some())
                || candidate
                    .domination
                    .as_ref()
                    .is_some_and(|w| w.squad_assignment(squad).is_some())
            {
                self.set_candidate_tag(&mut candidate, squad, u32::MAX)?;
            }
            // Only Sabotage stores a tag here; other modes keep it in their provider.
            // Like waypoint cleanup, an index outside the order table is ignored.
            let _ = candidate.runtime.state.set_manual_tag(squad, u32::MAX);
        }
        Ok(Some(candidate))
    }
    pub(crate) fn install_squad_order_candidate(&mut self, candidate: Candidate) {
        self.squad_orders = Some(candidate.runtime);
        self.acquisition_gameplay = candidate.acquisition;
        self.domination_gameplay = candidate.domination;
    }
    fn squad_order_target_exists(&self, target: u32) -> bool {
        let index = target as u16;
        match target & 0xffff_0000 {
            0x0c80_0000 => {
                self.objective_catalog.as_ref().is_some_and(|catalog| {
                    catalog
                        .objects
                        .iter()
                        .any(|object| object.object_index == index)
                }) && self
                    .objective_world
                    .as_ref()
                    .is_some_and(|world| world.state(index).is_some())
            }
            0x0100_0000 => self.ground_runtime.as_ref().is_some_and(|runtime| {
                runtime
                    .vehicles()
                    .any(|vehicle| vehicle.key.object_id() == target)
            }),
            0x0280_0000 => self
                .domination_gameplay
                .as_ref()
                .and_then(|world| world.state.helo.as_ref())
                .is_some_and(|fleet| {
                    fleet
                        .carriers
                        .iter()
                        .any(|carrier| carrier.source.object_index == index)
                }),
            0x0d80_0000 => self
                .bunker_world
                .as_ref()
                .is_some_and(|world| world.domination_bunker(index).is_some()),
            _ => false,
        }
    }
    fn squad_order_snapshot(&self, squad: u16, state: &State) -> Snapshot {
        let tag = self
            .acquisition_gameplay
            .as_ref()
            .and_then(|r| r.state.squad(squad).map(|s| s.assignment))
            .or_else(|| {
                self.domination_gameplay
                    .as_ref()
                    .and_then(|w| w.squad_assignment(squad))
            })
            .map_or_else(|| state.manual_tag(squad), |a| a.tagged_object);
        Snapshot {
            tagged_object: tag,
            waypoint: state.waypoint(squad),
        }
    }
    pub(crate) fn squad_roster_orders(&self, squad: u16) -> Option<(u32, bool)> {
        let runtime = self.squad_orders.as_ref()?;
        let snapshot = self.squad_order_snapshot(squad, &runtime.state);
        Some((
            snapshot.tagged_object,
            snapshot.waypoint.is_some_and(|waypoint| waypoint.active),
        ))
    }
    pub(crate) fn queue_squad_orders(
        &mut self,
        now: Duration,
        wall: u64,
    ) -> Result<(), &'static str> {
        let Some(mut next) = self.squad_orders.clone() else {
            return Ok(());
        };
        let mut publications = Vec::new();
        let actors: Vec<_> = self.active.iter().chain(self.peers.values()).collect();
        next.published
            .retain(|id, _| actors.iter().any(|actor| actor.reservation.id == *id));
        for actor in &actors {
            if !actor.ready_for_updates || actor.initial_objects.is_none() {
                continue;
            }
            let mut baseline = next
                .published
                .get(&actor.reservation.id)
                .cloned()
                .unwrap_or_default();
            let mut packets = Vec::new();
            let catalog = &actor.squad_catalog;
            let selected = catalog.squad_index();
            for squad in 0..32 {
                let registered = if catalog.map_pool().squads.is_empty() {
                    squad == selected
                } else {
                    catalog.selected_squad(squad).is_some_and(|binding| {
                        catalog.match_profile().auto_assign_teams
                            || (u16::from(binding.team) == selected / 16
                                && u16::from(binding.platoon) == selected % 16 / 4)
                    })
                };
                if !registered {
                    continue;
                }
                let snapshot = self.squad_order_snapshot(squad, &next.state);
                if baseline.get(&squad) == Some(&snapshot) {
                    continue;
                }
                let previous = baseline.get(&squad);
                let waypoint_changed = previous.is_none_or(|old| old.waypoint != snapshot.waypoint);
                packets.push(
                    replication::SquadOrders {
                        sequence: 2,
                        timestamp_ms: now.as_millis() as u32,
                        squad_index: squad,
                        tagged_object: previous
                            .is_none_or(|old| old.tagged_object != snapshot.tagged_object)
                            .then_some(snapshot.tagged_object),
                        waypoint_position: snapshot
                            .waypoint
                            .filter(|waypoint| {
                                previous
                                    .and_then(|old| old.waypoint)
                                    .is_none_or(|old| old.position != waypoint.position)
                            })
                            .map(|waypoint| waypoint.position),
                        waypoint_active: waypoint_changed
                            .then_some(snapshot.waypoint.is_some_and(|w| w.active)),
                    }
                    .encode_with_profile(self.wire_profile)?,
                );
                baseline.insert(squad, snapshot);
            }
            if packets.is_empty()
                || actor.ordinary_shared_len() + packets.len() > multiplayer::MAX_SHARED_QUEUE
            {
                continue;
            }
            next.published
                .insert(actor.reservation.id.clone(), baseline);
            publications.push((actor.reservation.id.clone(), packets));
        }
        if publications.is_empty() {
            return Ok(());
        }
        self.record(json!({"op":"squad_orders_published","timestamp_ms":now.as_millis(),
            "recipients":publications.iter().map(|(id,packets)|json!({"reservation_id":id,
                "packets":packets.iter().map(|packet|protocol::encode_hex(packet)).collect::<Vec<_>>()})).collect::<Vec<_>>()}),wall)?;
        self.squad_orders = Some(next);
        for (id, packets) in publications {
            if let Some(actor) = self
                .active
                .iter_mut()
                .chain(self.peers.values_mut())
                .find(|a| a.reservation.id == id)
            {
                for packet in packets {
                    actor.queue_shared(packet);
                }
            }
        }
        Ok(())
    }
}
