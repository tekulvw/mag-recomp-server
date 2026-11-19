//! Concrete Acquisition objective transactions over owned Engine state.
use crate::{
    acquisition_atomic::{EnteredState, Entry},
    acquisition_character_effects::NativeState,
    acquisition_control::Catalog,
    acquisition_gameplay::{Effect, Gameplay},
    acquisition_round::{Round, Rules, ScoreIntent},
    acquisition_spawn_effects::Clearance,
    acquisition_volume_effects::VolumeRequest,
    acquisition_world::Settings,
    dynamic_collision::QueryBox,
    objective_control::ObjectiveActor,
    objective_range::TriggerBox,
    replication::{ObjectiveControlDelta, ObjectiveState},
    *,
};
use std::{collections::BTreeMap, sync::Arc};

pub struct Setup {
    pub controller: Catalog,
    pub state: Gameplay,
    pub rules: Rules,
    pub filter_rows: [u32; 32],
    pub physics: Settings,
    pub volumes: BTreeMap<u32, QueryBox>,
    pub initial_active: Vec<u32>,
    pub arrival_triggers: BTreeMap<u32, TriggerBox>,
    pub arrival_static: Option<ground_vehicles::arrival_static::ArrivalStatic>,
    pub ground_capabilities: Vec<(u16, acquisition_ground_runtime::Capabilities)>,
    pub ground_damage: Option<acquisition_ground_damage::Policy>,
    pub vehicle_reinforcement_intervals_ms: [u32; 2],
}

impl Setup {
    /// Every capture chain must link a distinct, drivable Retail Escort.
    /// Check this before advertising or starting a source gameplay profile.
    pub fn validate_ground_drivers(
        &self,
        vehicles: &ground_vehicles::Catalog,
    ) -> Result<(), &'static str> {
        let mut seen = std::collections::BTreeSet::new();
        for chain in self.controller.chains() {
            let fsp = self
                .state
                .objective(chain.capture_token)
                .and_then(|capture| capture.fsp)
                .ok_or("acquisition_escort_driver_source_mismatch")?;
            let index = fsp.vehicle.index;
            if !seen.insert(index)
                || vehicles.vehicle_source(index).is_none_or(|source| {
                    source.spawn_type != 9
                        || source.spawn_point_index != fsp.source.spawn_point_index
                        || source.spawn_point_token != fsp.source.spawn_point_token
                        || source.spawn_point_token != chain.vehicle_spawn_token
                })
            {
                return Err("acquisition_escort_driver_source_mismatch");
            }
            if !vehicles
                .driver_seat(index)
                .is_some_and(|driver| driver.seat == 0)
            {
                return Err("acquisition_escort_driver_policy_required");
            }
        }
        Ok(())
    }

    pub fn validate_ground_hulk_templates(
        &self,
        catalog: &dynamic_collision::Catalog,
    ) -> Result<(), &'static str> {
        for (_, capabilities) in &self.ground_capabilities {
            if capabilities.hulk_template.as_ref().is_some_and(|template| {
                catalog.template_kind(template)
                    != Some(acquisition_volume_damage::TargetKind::GroundVehicle)
            }) {
                return Err("acquisition_ground_hulk_template_missing");
            }
        }
        Ok(())
    }
}

#[derive(Clone)]
pub(crate) struct Runtime {
    pub(crate) setup: Arc<Setup>,
    pub(crate) state: Gameplay,
    started: bool,
    pub(crate) last_ground_update: Option<Duration>,
    pub(crate) vehicle_waves: Option<acquisition_vehicle_waves::VehicleWaves>,
}
impl Runtime {
    pub(crate) fn from_setup(setup: Arc<Setup>) -> Self {
        Self {
            state: setup.state.clone(),
            setup,
            started: false,
            last_ground_update: None,
            vehicle_waves: None,
        }
    }
    pub(crate) fn reset(&mut self) {
        self.state = self.setup.state.clone();
        self.started = false;
        self.last_ground_update = None;
        self.vehicle_waves = None;
    }
}

impl<W: Write> Engine<W> {
    /// Explicit replacement gameplay configuration, separate from the native
    /// pre-MissionStart identity catalog. No services or game process are used.
    pub fn with_acquisition_gameplay(mut self, setup: Setup) -> Result<Self, &'static str> {
        if self.active.is_some()
            || !self.peers.is_empty()
            || self.wire_profile.application_id() != 20231
            || self.match_profile().game_type != 1
        {
            return Err("acquisition_gameplay_requires_idle_retail_world");
        }
        setup.controller.validate_for(
            self.spawn_catalog
                .as_ref()
                .ok_or("spawn_catalog_unavailable")?,
        )?;
        let ground = self
            .ground_vehicles
            .as_ref()
            .ok_or("ground_vehicle_catalog_required")?;
        setup.validate_ground_drivers(ground)?;
        let catalog = self
            .objective_catalog
            .as_ref()
            .ok_or("objective_catalog_unavailable")?;
        if catalog.source().capture_qualification != "native_source_gameplay"
            || setup.rules.starting_points != self.match_profile().maximum_points
            || setup.rules.starting_points.iter().any(|v| *v <= 0)
            || !(0..=100).contains(&setup.rules.major_percent)
            || !(1..=60_000).contains(&setup.rules.end_delay_ms)
        {
            return Err("acquisition_gameplay_configuration_mismatch");
        }
        for (token, objective) in setup.state.objectives() {
            if catalog.binding(token)
                != Some((
                    objective.replication.object_index,
                    objective.state.current as u8,
                ))
            {
                return Err("acquisition_gameplay_objective_mismatch");
            }
        }
        let empty = dynamic_collision::Snapshot::capture(
            acquisition_atomic::WorldKey {
                world_id: 1,
                incarnation: 1,
            },
            0,
            &[],
            vec![],
            0,
        )?;
        for chain in setup.controller.chains() {
            let query = setup
                .volumes
                .get(&chain.volume_token)
                .ok_or("acquisition_volume_missing")?;
            empty.penetrations(*query, setup.physics.query_limits)?;
            if !setup.arrival_triggers.contains_key(&chain.arrival_token) {
                return Err("acquisition_arrival_trigger_missing");
            }
        }
        for &token in &setup.initial_active {
            if setup.state.objective(token).is_none() {
                return Err("acquisition_startup_objective_missing");
            }
        }
        if let Some(policy) = &setup.ground_damage {
            policy.validate()?;
            let runtime = self
                .ground_runtime
                .as_ref()
                .ok_or("ground_vehicle_runtime_required")?;
            for vehicle in runtime.vehicles() {
                let creation = ground
                    .vehicle(vehicle.key.index)
                    .ok_or("unknown_ground_vehicle")?;
                policy.validate_vehicle(creation)?;
            }
        }
        acquisition_gameplay_config::configure_ground(
            self.ground_runtime
                .as_mut()
                .ok_or("ground_vehicle_runtime_required")?,
            &setup.ground_capabilities,
        )?;
        self.acquisition_round = Some(Round::new(setup.rules));
        let state = setup.state.clone();
        self.acquisition_gameplay = Some(Runtime {
            setup: Arc::new(setup),
            state,
            started: false,
            last_ground_update: None,
            vehicle_waves: None,
        });
        Ok(self)
    }

    /// The admitted replacement runtime has no temporary invulnerability
    /// producer. These values describe this owned world, not a native capture.
    /// Mounted status comes from current seats, never a default false flag.
    fn owned_acquisition_characters(&self) -> BTreeMap<u32, NativeState> {
        let mut states = BTreeMap::new();
        for actor in self.active.iter().chain(self.peers.values()) {
            states.insert(
                0x0380_0000 | u32::from(actor.reservation.player),
                NativeState {
                    invulnerability: Some(0),
                    associated_vehicle: Some(if actor.is_mounted() {
                        protocol::VictimWasMounted::AssociatedVehicle
                    } else {
                        protocol::VictimWasMounted::NoAssociatedVehicle
                    }),
                },
            );
        }
        for &id in self.bot_registry.bots.keys() {
            states.insert(
                0x0380_0000 | u32::from(id),
                NativeState {
                    invulnerability: Some(0),
                    associated_vehicle: Some(protocol::VictimWasMounted::NoAssociatedVehicle),
                },
            );
        }
        states
    }

    pub(crate) fn tick_acquisition_objectives(
        &mut self,
        now: Duration,
        wall: u64,
    ) -> Result<Vec<OutboundMessage>, &'static str> {
        let mut runtime = self
            .acquisition_gameplay
            .clone()
            .ok_or("acquisition_gameplay_unavailable")?;
        if !self.match_started || self.match_terminal.is_some() {
            return Ok(vec![]);
        }
        if !self
            .active
            .iter()
            .chain(self.peers.values())
            .any(|a| a.entered_game && a.ready_for_updates && a.initial_objects.is_some())
        {
            return Ok(vec![]);
        }
        let setup = Arc::clone(&runtime.setup);
        let mut world = self
            .objective_world
            .clone()
            .ok_or("objective_world_unavailable")?;
        let mut round = self
            .acquisition_round
            .clone()
            .ok_or("acquisition_round_unavailable")?;
        let before_ground = self
            .ground_runtime
            .clone()
            .ok_or("ground_vehicle_runtime_required")?;
        let mut ground = before_ground.clone();
        let prior_owners: BTreeMap<_, _> = world
            .snapshot(1, now.as_millis() as u32)
            .objectives
            .iter()
            .filter_map(|d| {
                world
                    .interaction_owner(d.object_index)
                    .map(|owner| (d.object_index, owner))
            })
            .collect();
        let mut actors: BTreeMap<_, _> = self
            .active
            .iter()
            .chain(self.peers.values())
            .filter_map(|a| {
                let CharacterLife::Alive { generation, .. } = a.life else {
                    return None;
                };
                if !a.entered_game || !a.ready_for_updates || a.is_mounted() {
                    return None;
                }
                Some((
                    a.reservation.player,
                    ObjectiveActor {
                        player: a.reservation.player,
                        team: a.team(),
                        generation,
                        position: a.tracking.sample.as_ref()?.position,
                    },
                ))
            })
            .collect();
        for (&id, bot) in &self.bot_registry.bots {
            if let Some(r) = bot.target.runtime
                && bot.health > 0
            {
                actors.insert(
                    id,
                    ObjectiveActor {
                        player: id,
                        team: bot.target.squad.team(),
                        generation: r.generation,
                        position: r.position,
                    },
                );
            }
        }
        let mut changed: BTreeMap<u16, ObjectiveControlDelta> = world
            .tick(now, &actors)
            .into_iter()
            .map(|d| (d.object_index, d))
            .collect();
        let mut entry_actors = BTreeMap::new();
        for (token, object) in runtime.state.objectives().collect::<Vec<_>>() {
            if let Some(delta) = changed.get(&object.replication.object_index) {
                entry_actors.insert(token, delta.state_actor.actor_index);
            }
        }
        let observations: Vec<_> = runtime
            .state
            .objectives()
            .filter_map(|(token, object)| {
                changed
                    .get(&object.replication.object_index)
                    .map(|&delta| (token, delta))
            })
            .collect();
        for (token, delta) in observations {
            if matches!(
                delta.state_actor.state,
                ObjectiveState::Completed | ObjectiveState::Deactivated
            ) {
                runtime
                    .state
                    .request_state(token, delta.state_actor.state)?;
            } else {
                runtime
                    .state
                    .request_state(token, delta.state_actor.state)?;
                runtime.state.commit_state(
                    token,
                    delta.state_actor.state,
                    delta.state_actor.actor_index,
                    delta.state_actor.change_time_ms,
                )?;
            }
        }
        let starting = !runtime.started;
        if starting {
            for &token in &setup.initial_active {
                runtime
                    .state
                    .request_state(token, ObjectiveState::Deactivated)?;
            }
            runtime.started = true;
        }
        // Resolve the current squad spawn each time; the initial catalog alone
        // is not a claim that a later selected spawn still resolves.
        for index in (0..2).flat_map(|team| (0..8).map(move |slot| team * 16 + slot)) {
            if let Some(squad) = runtime.state.squad_mut(index) {
                squad.spawn_resolved = self.spawn_catalog.as_ref().is_some_and(|c| {
                    c.default_for_squad(index)
                        .and_then(|p| c.point(p))
                        .is_some()
                });
            }
        }
        let mut ground_changes = BTreeMap::new();
        let mut transition_packets = Vec::new();
        let mut native = self.owned_acquisition_characters();
        let characters = self.prepare_acquisition_character_callback(&native, now, |engine, native| {
            let mut audit = Vec::new();
            let mut messages = Vec::new();
            let (rows, output) = engine.stage_acquisition_vehicle_waves(&mut runtime, &mut ground, now, native, &mut ground_changes)?;
            audit.extend(rows);
            messages.extend(output);
            let (rows, output) = engine.stage_acquisition_ground_updates(&mut runtime, &mut ground, now, native, &mut ground_changes, &mut entry_actors)?;
            audit.extend(rows);
            messages.extend(output);
            let (rows, output) = engine.stage_acquisition_deliveries(&mut runtime, &mut ground, now, &mut ground_changes, &mut entry_actors)?;
            audit.extend(rows);
            messages.extend(output);
            *native = engine.owned_acquisition_characters();
            for _ in 0..256 {
                let next = runtime.state.pending_transitions().min_by_key(|(token, _)| runtime.state.objective(*token).unwrap().replication.object_index);
                let Some((token, target)) = next else {
                    for (&index, &(player, generation, trigger)) in &prior_owners {
                        if !changed.get(&index).is_some_and(|d| d.state_actor.state != ObjectiveState::Activating) {
                            continue;
                        }
                        if let Some(actor) = engine.active.iter().chain(engine.peers.values()).find(|a| {
                            a.reservation.player == player && matches!(a.life, CharacterLife::Alive { generation: current, .. } if current == generation)
                        }) {
                            messages.push(events::objective_hold_exit(actor, index, player, trigger));
                        }
                    }
                    return Ok((audit, messages));
                };
                let state = objective_catalog::native_state(target as u8).ok_or("unsupported_acquisition_target_state")?;
                let object = runtime.state.objective(token).ok_or("unknown_acquisition_gameplay_objective")?;
                let attributed = if state == ObjectiveState::Completed { object.current_attacker } else { object.current_defender };
                let actor_index = entry_actors.remove(&token).unwrap_or({
                    if attributed & 0xffff_0000 == 0x0380_0000 { attributed as u16 } else { u16::MAX }
                });
                let callback = setup.controller.objective_tokens().any(|t| t == token);
                let delta = if callback && matches!(state, ObjectiveState::Completed | ObjectiveState::Deactivated) {
                    let entry = Entry { objective_token: token, state: if state == ObjectiveState::Completed { EnteredState::Completed } else { EnteredState::Deactivated }, actor_index, timestamp_ms: now.as_millis() as u32 };
                    let mut rates = round.rates_mut().clone();
                    let publication = runtime.state.apply_entry(&setup.controller, &entry, &mut ground, &mut rates, |ground, effect| {
                        let previous = engine.ground_runtime.replace(ground.clone()).ok_or("ground_vehicle_runtime_required")?;
                        let result = match effect {
                            Effect::Blockers(source) => engine.stage_respawn_clearance(Clearance { source, physics: setup.physics, filter_rows: setup.filter_rows, now }, native, &mut ground_changes),
                            Effect::Volume { target, arguments, damage_type_symbol, damage_source_symbol } => {
                                acquisition_damage_metadata::Metadata::from_symbols(damage_type_symbol, damage_source_symbol)?;
                                engine.stage_acquisition_volume(VolumeRequest {
                                    query: *setup.volumes.get(&target).ok_or("acquisition_volume_missing")?,
                                    arguments: acquisition_volume_damage::Arguments { player_damage: arguments[0] as f32, ground_vehicle_damage: arguments[1] as f32, mine_damage: arguments[2] as f32, apply_to_team: arguments[3] },
                                    row6: setup.filter_rows[6], limits: setup.physics.query_limits, now,
                                }, native, &mut ground_changes)
                            }
                            Effect::ScoreDeduction { team, amount } => {
                                round.apply_score(ScoreIntent { team, delta: amount.wrapping_neg() })?;
                                Ok((vec![], vec![]))
                            }
                        };
                        *ground = engine.ground_runtime.replace(previous).unwrap();
                        let (rows, output) = result?;
                        audit.extend(rows);
                        messages.extend(output);
                        Ok(())
                    })?;
                    *round.rates_mut() = rates;
                    for event in &publication.lifecycle {
                        let mask = match event.effect { acquisition_ground_runtime::Effect::ForcePosition(_) => 1 << 25, acquisition_ground_runtime::Effect::ForceRotation(_) => 1 << 26, acquisition_ground_runtime::Effect::DamageReset => 1 << 18, acquisition_ground_runtime::Effect::StateChanged(_) => 1 << 5, _ => 0 };
                        if mask != 0 { *ground_changes.entry(event.vehicle.index).or_default() |= mask; }
                    }
                    audit.push(json!({"objective":token,"current":state as u8,"commands":publication.plan.commands,"calls":publication.plan.unbound_calls}));
                    publication.objective
                } else {
                    runtime.state.commit_state(token, state, actor_index, now.as_millis() as u32)?
                };
                transition_packets.push(replication::ObjectiveControlSnapshot {
                    sequence: 1,
                    timestamp_ms: now.as_millis() as u32,
                    objectives: vec![delta],
                }.encode_with_profile(engine.wire_profile)?);
                world.install_acquisition_delta(delta)?;
                changed.insert(delta.object_index, delta);
            }
            Err("acquisition_transition_limit")
        });
        self.ground_runtime = Some(before_ground);
        let characters = characters?;
        if changed.is_empty()
            && !starting
            && ground_changes.is_empty()
            && characters.audit().is_empty()
        {
            // A scheduling cursor carries no gameplay result. Keep it fresh
            // during idle ticks so new critical damage never ages retroactively.
            if let Some(live) = &mut self.acquisition_gameplay {
                live.last_ground_update = Some(now);
            }
            return Ok(vec![]);
        }
        let packet = if changed.is_empty() {
            None
        } else {
            Some(
                replication::ObjectiveControlSnapshot {
                    sequence: 1,
                    timestamp_ms: now.as_millis() as u32,
                    objectives: changed.values().copied().collect(),
                }
                .encode_with_profile(self.wire_profile)?,
            )
        };
        let ground_packet = self.encode_ground_changes(&ground, &ground_changes, now)?;
        self.check_acquisition_character_effects(&characters, &native)?;
        self.record(json!({"op":"acquisition_objective_tick","timestamp_ms":now.as_millis() as u64,"startup":starting,"controller_sha256":setup.controller.fingerprint(),"effects":characters.audit(),"messages":characters.messages(),"scores":round.points(),"objective_updates":transition_packets.iter().map(|p| protocol::encode_hex(p)).collect::<Vec<_>>(),"objectives":changed.values().map(|d|json!({"index":d.object_index,"state":d.state_actor.state as u8})).collect::<Vec<_>>()}), wall)?;
        let messages = self.install_character_effects_after_record(characters, &mut native);
        self.ground_runtime = Some(ground);
        self.objective_world = Some(world);
        self.acquisition_round = Some(round);
        self.acquisition_gameplay = Some(runtime);
        for packet in transition_packets {
            self.broadcast_bot_packet(packet);
        }
        if let Some(packet) = packet {
            self.broadcast_bot_packet(packet);
        }
        if let Some(packet) = ground_packet {
            self.broadcast_bot_packet(packet);
        }
        for actor in self.active.iter_mut().chain(self.peers.values_mut()) {
            actor.objective_assigned = false;
            actor.objective_needs_clear = true;
        }
        Ok(messages)
    }
}
