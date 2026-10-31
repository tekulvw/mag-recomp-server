//! Private lab command; retail equipment eligibility is explicit and replication-only.
use super::*;

pub(super) struct Target {
    pub object_index: u16,
    pub display_name: Option<&'static str>,
    pub generation: u32,
    pub position: replication::CharacterPosition,
    pub faction: u8,
    pub squad: replication::SquadAddress,
    pub roster_published: bool,
    pub loadout: replication::CharacterLoadout,
    pub script: Option<BotScript>,
    pub runtime: Option<BotRuntime>,
    pub pending_move: Option<development_bot_movement::PendingMove>,
    pub move_commands: std::collections::BTreeMap<String, BotMove>,
}
impl<W: Write> Engine<W> {
    pub(super) fn development_target_replication_enabled(&self, active: &Active) -> bool {
        self.match_terminal.is_none() && (self.development_dummy_enabled()
            || active.development_target.as_ref().is_some_and(|target| {
                matches!(active.life, CharacterLife::Alive { generation, .. } if generation == target.generation)
                    && active.mounted_seat.is_none()
                    && active.entered_game
                    && active.ready_for_updates
            }))
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn queue_development_target(
        &mut self,
        reservation_id: &str,
        world_id: u32,
        player_id: u16,
        spawn_id: String,
        expected_generation: u32,
        position: replication::CharacterPosition,
        bot_script: Option<BotScript>,
        wall_secs: u64,
    ) -> Result<Vec<OutboundMessage>, &'static str> {
        if !self.development_control {
            return Err("development_control_disabled");
        }
        if self.match_terminal.is_some() {
            return Err("match_finished");
        }
        if self.maximum_players != 1 {
            return Err("dummy_slot_conflicts_with_multiplayer");
        }
        if self.wire_profile.application_id() != 20231 {
            return Err("target_requires_retail_profile");
        }
        if player_id == dummy::OBJECT_INDEX {
            return Err("dummy_slot_conflicts_with_owner");
        }
        position.validate()?;
        if let Some(script) = &bot_script {
            script.validate(position)?;
        }
        let active = self.active.as_ref().ok_or("no_reservation")?;
        if active.reservation.id != reservation_id
            || active.reservation.world != world_id
            || active.reservation.player != player_id
        {
            return Err("identity_mismatch");
        }
        let CharacterLife::Alive { generation, .. } = active.life else {
            return Err("target_requires_living_character");
        };
        if generation != expected_generation {
            return Err("target_life_mismatch");
        }
        if active.mounted_seat.is_some() || !active.entered_game || !active.ready_for_updates {
            return Err("target_requires_ready_on_foot_character");
        }
        if let Some(previous) = active.target_commands.get(&spawn_id) {
            return if *previous == (generation, position, bot_script.clone()) {
                Ok(Vec::new())
            } else {
                Err("target_spawn_id_conflict")
            };
        }
        if active
            .development_target
            .as_ref()
            .is_some_and(|t| t.script.is_some())
            || (bot_script.is_some() && active.development_target.is_some())
        {
            return Err("bot_target_already_exists");
        }
        if active.target_commands.len() >= 64 {
            return Err("target_command_limit");
        }
        let history_budget = bot_script.as_ref().map_or(2, |script| {
            script.publication_budget() + BOT_HISTORY_HEADROOM
        });
        if active.replication_history.len() + history_budget >= MAX_REPLICATION_HISTORY {
            return Err("replication_history_limit");
        }
        let human_index = active
            .current_inventory_handle
            .ok_or("target_requires_equipped_inventory")?
            .inventory_index;
        let character = &active.reservation.character;
        let (faction, index, squad) = if bot_script.is_some() {
            let team = 1 - active.team();
            let squad = replication::SquadAddress::new(team, 0, 0)?;
            if self.squad_reservation_count(squad.squad_index()) != 0 {
                return Err("bot_squad_occupied");
            }
            if !self
                .squad_creations(None)
                .iter()
                .any(|s| s.object_index == squad.squad_index())
            {
                return Err("bot_requires_initialized_opposite_squad");
            }
            let profile = self.match_profile();
            let faction = if team == 0 {
                profile.attacking_faction
            } else {
                profile.defending_faction
            };
            let index = self
                .item_catalog
                .bootstrap_default_inventory_index(faction)
                .ok_or("missing_retail_target_loadout")?;
            (faction, index, squad)
        } else {
            (
                character.faction,
                human_index,
                replication::SquadAddress::new(0, 0, 0)?,
            )
        };
        let loadout = if bot_script.is_some() {
            self.item_catalog.bootstrap_loadout(faction, index)
        } else {
            character.replication_loadout(&self.item_catalog, index, self.wire_profile)
        }
        .ok_or("missing_retail_target_loadout")?;
        // Validate typed packets before recording or mutating state.
        if bot_script.is_some() {
            dummy::character_with_loadout(
                2,
                0,
                &loadout,
                faction,
                self.wire_profile,
                "BOT Mercer",
                squad,
            )?;
        } else {
            dummy::creation_with_loadout(2, 0, &loadout, faction, self.wire_profile)?;
        }
        dummy::respawn_with_loadout(3, 0, position, &loadout, self.wire_profile)?;
        self.record(json!({"op":"development_target_queued","reservation_id":reservation_id,
            "world_id":world_id,"player_id":player_id,"spawn_id":spawn_id,"life_generation":generation,
            "position":position,"team":squad.team(),"squad_index":squad.squad_index(),"faction":faction,"inventory_index":index,
            "equipment_source":"verified_retail_bootstrap","combat_authority":self.item_catalog.retail_combat().is_ok(),
            "name":if bot_script.is_some(){"BOT Mercer"}else{"Target"},"bot_script":bot_script,"bot_object_id":dummy::OBJECT_ID,"bot_owner_index":dummy::SERVER_OWNER,
            "placement_policy":"explicit_lab_route_not_ground_validated"}),wall_secs)?;
        let active = self.active.as_mut().unwrap();
        active
            .target_commands
            .insert(spawn_id, (generation, position, bot_script.clone()));
        active.development_target = Some(Target {
            object_index: dummy::OBJECT_INDEX,
            display_name: None,
            generation,
            position,
            faction,
            squad,
            roster_published: false,
            loadout,
            script: bot_script,
            runtime: None,
            pending_move: None,
            move_commands: Default::default(),
        });
        active.dummy_health = None;
        Ok(Vec::new())
    }
}

// Leave room for ordinary owner lifecycle and objective/clock updates.
pub(super) const BOT_HISTORY_HEADROOM: usize = 64;

/// Bounded private representation proof, not navigation or combat AI.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BotScript {
    pub offset: [f32; 3],
    pub spawn_hold_ms: u32,
    pub travel_ms: u32,
    pub dead_ms: u32,
    pub update_ms: u32,
    pub respawn_count: u8,
    pub outbound_yaw: u16,
    pub return_yaw: u16,
}
impl BotScript {
    pub(crate) fn validate(
        &self,
        anchor: replication::CharacterPosition,
    ) -> Result<(), &'static str> {
        if self.offset.iter().any(|v| !v.is_finite() || v.abs() > 8.0)
            || self.offset.iter().all(|v| *v == 0.0)
            || self.offset[1] != 0.0
            || self.spawn_hold_ms > 60000
            || !(1000..=60000).contains(&self.travel_ms)
            || !(1000..=60000).contains(&self.dead_ms)
            || !(100..=1000).contains(&self.update_ms)
            || !(1..=3).contains(&self.respawn_count)
            || self.outbound_yaw > 1023
            || self.return_yaw > 1023
        {
            return Err("invalid_bot_script");
        }
        // Lab scripts preserve arbitrary supplied facing, so use the minimum
        // normal directional walking cap and never infer sprint from timing.
        let distance = f64::from(self.offset[0]).hypot(f64::from(self.offset[2]));
        if distance / (f64::from(self.travel_ms) / 1000.) > retail_movement::WALK_BACKWARD_MPS {
            return Err("bot_script_player_speed_limit");
        }
        if self.publication_budget() + BOT_HISTORY_HEADROOM >= MAX_REPLICATION_HISTORY {
            return Err("bot_script_history_budget");
        }
        self.position(anchor, 1.0).validate()
    }
    fn publication_budget(&self) -> usize {
        // At most one movement publication per update interval, then death+respawn.
        3 + usize::from(self.respawn_count)
            * ((self.travel_ms as usize * 2).div_ceil(self.update_ms as usize) + 2)
    }
    fn position(
        &self,
        anchor: replication::CharacterPosition,
        fraction: f32,
    ) -> replication::CharacterPosition {
        replication::CharacterPosition {
            x: anchor.x + self.offset[0] * fraction,
            y: anchor.y,
            z: anchor.z + self.offset[2] * fraction,
        }
    }
}
#[derive(Clone, Copy)]
pub(super) enum BotPhase {
    Moving,
    Dead,
    Complete,
    Relocating,
}
#[derive(Clone, Copy)]
pub(super) struct BotRuntime {
    pub generation: u32,
    pub(super) phase: BotPhase,
    pub(super) since: Duration,
    pub(super) last_update: Duration,
    pub(super) position: replication::CharacterPosition,
    pub(super) physics_motion: Option<bot_physics::Motion>,
    pub(super) navigation_waypoint: usize,
    pub(super) navmesh_state: development_bot_movement::NavmeshRuntime,
}
impl BotRuntime {
    /// Collision and recovery use the authoritative integrator pose, never its
    /// f32 replication projection. Wire-only/scripted actors retain a fallback.
    pub(super) fn physical_position(self) -> [f64; 3] {
        self.physics_motion.map_or(
            [
                f64::from(self.position.x),
                f64::from(self.position.y),
                f64::from(self.position.z),
            ],
            |m| m.position,
        )
    }
    pub fn spawned(now: Duration, position: replication::CharacterPosition) -> Self {
        Self {
            generation: 1,
            physics_motion: None,
            navigation_waypoint: 0,
            navmesh_state: Default::default(),
            phase: BotPhase::Moving,
            since: now,
            last_update: now,
            position,
        }
    }
}
pub(super) struct BotStep {
    pub packet: Vec<u8>,
    pub runtime: BotRuntime,
    /// Spawn/death or a newly observed physical landing may change HP; ordinary poses preserve it.
    pub health: Option<u16>,
    pub receipt: Value,
    pub swept_bounds: Option<crate::actor_collision::Bounds>,
}
impl Target {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn script_step_with_actors(
        &self,
        sequence: u32,
        now: Duration,
        profile: wire_profile::WireProfile,
        collision_world: Option<&collision::CollisionWorld>,
        map_name: &str,
        health: u16,
        fall_rules: Option<&fall_damage::Rules>,
        actors: Option<crate::actor_collision::Query<'_>>,
    ) -> Result<Option<BotStep>, &'static str> {
        if self
            .runtime
            .is_some_and(|r| matches!(r.phase, BotPhase::Relocating))
        {
            return self.movement_step_with_actors(
                sequence,
                now,
                profile,
                collision_world,
                map_name,
                health,
                fall_rules,
                actors,
            );
        }
        let (Some(script), Some(mut runtime)) = (&self.script, self.runtime) else {
            return Ok(None);
        };
        if matches!(runtime.phase, BotPhase::Complete)
            || now.saturating_sub(runtime.last_update)
                < Duration::from_millis(script.update_ms.into())
        {
            return Ok(None);
        }
        let elapsed = now.saturating_sub(runtime.since).as_millis() as u64;
        let timestamp_ms = now.as_millis() as u32;
        let (packet, health, phase, position) = match runtime.phase {
            BotPhase::Moving => {
                if elapsed < u64::from(script.spawn_hold_ms) {
                    return Ok(None);
                }
                let movement = elapsed - u64::from(script.spawn_hold_ms);
                if movement >= u64::from(script.travel_ms) * 2 {
                    runtime.phase = BotPhase::Dead;
                    runtime.since = now;
                    let packet = replication::CharacterDamageUpdate {
                        sequence,
                        timestamp_ms,
                        object_index: self.object_index,
                        damage: replication::CharacterDamage {
                            last_location_and_type: 0,
                            health_quarter_points: 0,
                            conditions: 0,
                            last_attacker: u32::MAX,
                            last_source_object: u32::MAX,
                            last_user_data: 0,
                        },
                        forced_state: Some(replication::CharacterState {
                            state: 7,
                            stance: 0,
                            user_data: [0; 3],
                        }),
                    }
                    .encode_with_profile(profile)?;
                    (packet, Some(0), "death", runtime.position)
                } else {
                    let leg = movement as f32 / script.travel_ms as f32;
                    let fraction = if leg <= 1.0 { leg } else { 2.0 - leg };
                    let mut position = script.position(self.position, fraction);
                    let dx = f64::from(position.x - runtime.position.x);
                    let dz = f64::from(position.z - runtime.position.z);
                    let d = dx.hypot(dz);
                    let cap = retail_movement::WALK_BACKWARD_MPS
                        * now.saturating_sub(runtime.last_update).as_secs_f64();
                    if d > cap && d > 0. {
                        position.x = runtime.position.x + (dx / d * cap) as f32;
                        position.z = runtime.position.z + (dz / d * cap) as f32;
                    }
                    if let Some(query) = actors {
                        let from = [
                            runtime.position.x.into(),
                            runtime.position.y.into(),
                            runtime.position.z.into(),
                        ];
                        let to = [position.x.into(), position.y.into(), position.z.into()];
                        if query
                            .grid
                            .sweep(
                                query.actor,
                                from,
                                to,
                                bot_physics::Body::default(),
                                &mut 256,
                            )?
                            .is_some()
                        {
                            position = runtime.position;
                        }
                    }
                    // Traced Character fields4-7; ordinary standing state plus interpolated pose.
                    // Animation acceptance is a native observation, not inferred from this codec.
                    let fields = player_tracking::ClientCharacterFields {
                        state: Some(replication::CharacterState {
                            state: 1,
                            stance: 0,
                            user_data: [0; 3],
                        }),
                        position: Some(position),
                        pitch: Some(127),
                        yaw: Some(if movement < u64::from(script.travel_ms) {
                            script.outbound_yaw
                        } else {
                            script.return_yaw
                        }),
                        ..Default::default()
                    };
                    let packet = replication::multiplayer::CharacterOwnerDelta {
                        sequence,
                        timestamp_ms,
                        player: self.object_index,
                        fields: &fields,
                    }
                    .encode_with_profile(profile)?;
                    (packet, None, "move", position)
                }
            }
            BotPhase::Dead => {
                if elapsed < u64::from(script.dead_ms) {
                    return Ok(None);
                }
                if let Some(query) = actors
                    && query.grid.occupied(
                        query.actor,
                        [
                            self.position.x.into(),
                            self.position.y.into(),
                            self.position.z.into(),
                        ],
                        bot_physics::Body::default(),
                    )?
                {
                    return Ok(None);
                }
                runtime.generation = runtime
                    .generation
                    .checked_add(1)
                    .ok_or("bot_generation_exhausted")?;
                runtime.phase = if runtime.generation > u32::from(script.respawn_count) {
                    BotPhase::Complete
                } else {
                    BotPhase::Moving
                };
                runtime.since = now;
                let packet = dummy::respawn_for(
                    self.object_index,
                    sequence,
                    timestamp_ms,
                    self.position,
                    &self.loadout,
                    profile,
                )?;
                (
                    packet,
                    Some(dummy::INITIAL_HEALTH_QUARTER_POINTS),
                    "respawn",
                    self.position,
                )
            }
            BotPhase::Complete | BotPhase::Relocating => unreachable!(),
        };
        let route_leg = if phase == "move" {
            Some(
                if elapsed - u64::from(script.spawn_hold_ms) < u64::from(script.travel_ms) {
                    "out"
                } else {
                    "back"
                },
            )
        } else {
            None
        };
        runtime.last_update = now;
        runtime.position = position;
        Ok(Some(BotStep {
            packet,
            runtime,
            health,
            swept_bounds: Some(crate::actor_collision::Bounds::body(
                [
                    runtime.position.x.into(),
                    runtime.position.y.into(),
                    runtime.position.z.into(),
                ],
                bot_physics::Body::default(),
            )),
            receipt: json!({"object_id":(0x0380_0000 | u32::from(self.object_index)),"name":"BOT Mercer","team":self.squad.team(),"squad_index":self.squad.squad_index(),"faction":self.faction,"owner_life_generation":self.generation,"owner_index":dummy::SERVER_OWNER,"generation":runtime.generation,"phase":phase,"route_leg":route_leg,"position":position,"sequence":sequence,"timestamp_ms":timestamp_ms,"combat_policy":"ordinary_client_reported_bullets_when_catalog_installed"}),
        }))
    }
}
