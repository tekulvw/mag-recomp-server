//! Ordered ground updates inside the owned Acquisition callback.
use super::*;
use crate::{
    acquisition_character_effects::{self as character, NativeState},
    acquisition_gameplay_engine::Runtime,
    acquisition_ground_damage::Policy,
    acquisition_ground_runtime::{self as ground, Effect, Key},
    acquisition_vehicle_waves::VehicleWaves,
    acquisition_volume_damage::TargetKind,
    domination_structural_effects::InstantaneousBlast,
    domination_structural_world::{AreaDamageState, BlastCollision, BlastSource},
    dynamic_collision::QueryLimits,
    dynamic_collision::{Geometry, Pose, geometry},
    item_catalog::TargetDamageParameters,
};
use std::collections::BTreeMap;

struct Effects<'a> {
    now: Duration,
    native: &'a mut BTreeMap<u32, NativeState>,
    changes: &'a mut BTreeMap<u16, u32>,
    audit: Vec<Value>,
    messages: Vec<OutboundMessage>,
    helo_doses: Vec<crate::domination_helo_damage::RadialDose>,
}

pub(crate) struct GroundFrame<'a> {
    pub now: Duration,
    pub dt: f32,
    pub waves: &'a mut VehicleWaves,
    pub policy: Option<&'a Policy>,
    pub filter_row: u32,
    pub limits: QueryLimits,
    pub domination_blast: Option<(BlastCollision<'a>, f32)>,
}

#[derive(Default)]
pub(crate) struct GroundUpdate {
    pub audit: Vec<Value>,
    pub messages: Vec<OutboundMessage>,
    pub forced_fsp: Vec<(u32, u32, u8)>,
    pub helo_doses: Vec<crate::domination_helo_damage::RadialDose>,
}

impl<W: Write> Engine<W> {
    pub(crate) fn stage_acquisition_ground_updates(
        &mut self,
        runtime: &mut Runtime,
        ground: &mut ground::Runtime,
        now: Duration,
        native: &mut BTreeMap<u32, NativeState>,
        changes: &mut BTreeMap<u16, u32>,
        _entry_actors: &mut BTreeMap<u32, u16>,
    ) -> Result<(Vec<Value>, Vec<OutboundMessage>), &'static str> {
        let dt = runtime
            .last_ground_update
            .map_or(Ok(Duration::ZERO), |last| {
                now.checked_sub(last)
                    .ok_or("acquisition_ground_update_time_regression")
            })?
            .as_secs_f32();
        let update = self.stage_ground_updates(
            GroundFrame {
                now,
                dt,
                waves: runtime
                    .vehicle_waves
                    .as_mut()
                    .ok_or("acquisition_vehicle_waves_unavailable")?,
                policy: runtime.setup.ground_damage.as_ref(),
                filter_row: runtime.setup.filter_rows[6],
                limits: runtime.setup.physics.query_limits,
                domination_blast: None,
            },
            ground,
            native,
            changes,
        )?;
        for (objective, attacker, team) in update.forced_fsp {
            runtime
                .state
                .request_forced_fsp_deactivation(objective, attacker, team)?;
        }
        runtime.last_ground_update = Some(now);
        Ok((update.audit, update.messages))
    }

    pub(crate) fn stage_ground_updates(
        &mut self,
        mut frame: GroundFrame<'_>,
        ground: &mut ground::Runtime,
        native: &mut BTreeMap<u32, NativeState>,
        changes: &mut BTreeMap<u16, u32>,
    ) -> Result<GroundUpdate, &'static str> {
        let now = frame.now;
        let dt = frame.dt;
        let mut forced_fsp = Vec::new();
        let mut output = Effects {
            now,
            native,
            changes,
            audit: Vec::new(),
            messages: Vec::new(),
            helo_doses: Vec::new(),
        };
        let keys: Vec<_> = ground.vehicles().map(|vehicle| vehicle.key).collect();
        for key in keys {
            let before = ground
                .get(key.index)
                .ok_or("stale_acquisition_ground_candidate")?;
            let timer_running = before.inert_seconds > 0. || before.is_dying();
            let team = self
                .ground_vehicles
                .as_ref()
                .and_then(|catalog| catalog.vehicle(key.index))
                .ok_or("unknown_ground_vehicle")?
                .team;
            let next_spawn = u64::from(frame.waves.next_timestamp(team)?);
            let mut events = if before.is_dying() {
                frame
                    .policy
                    .ok_or("ground_damage_policy_required")?
                    .advance_critical(ground, key, dt)?
            } else {
                Vec::new()
            };
            events.extend(ground.update(key, dt, next_spawn)?);
            if events.is_empty() {
                if timer_running && dt > 0. {
                    output.audit.push(
                        json!({"op":"acquisition_ground_timer","vehicle":key.object_id(),"dt":dt}),
                    );
                }
                continue;
            }
            let previous = self
                .ground_runtime
                .take()
                .ok_or("ground_vehicle_runtime_required")?;
            self.ground_runtime = Some(ground.clone());
            let result = self.consume_acquisition_ground_events(
                &mut frame,
                &events,
                &mut output,
                &mut forced_fsp,
            );
            *ground = self
                .ground_runtime
                .replace(previous)
                .ok_or("ground_vehicle_runtime_required")?;
            result?;
        }
        Ok(GroundUpdate {
            audit: output.audit,
            messages: output.messages,
            forced_fsp,
            helo_doses: output.helo_doses,
        })
    }

    fn consume_acquisition_ground_events(
        &mut self,
        frame: &mut GroundFrame<'_>,
        events: &[ground::LifecycleEvent],
        output: &mut Effects<'_>,
        forced_fsp: &mut Vec<(u32, u32, u8)>,
    ) -> Result<(), &'static str> {
        for event in events {
            let key = event.vehicle;
            output.audit.push(
                json!({"op":"acquisition_ground_effect","vehicle":key.object_id(),
                "effect": format!("{:?}", event.effect)}),
            );
            match event.effect {
                Effect::StateChanged(state) => {
                    let mask = output.changes.entry(key.index).or_default();
                    *mask |= 1 << 5;
                    if matches!(state, 3 | 4 | 5 | 7)
                        && self.ground_turret_policy(key.index).is_some()
                    {
                        *mask |= 1 << 6;
                    }
                    if matches!(state, 4 | 5 | 7) && self.ground_driver_seat(key.index).is_some() {
                        *mask |= 1 << 1;
                    }
                }
                Effect::DamageChanged { .. } => {
                    *output.changes.entry(key.index).or_default() |= 1 << 18
                }
                Effect::KillOccupants { attacker, position } => {
                    let targets: Vec<_> = self
                        .active
                        .iter()
                        .chain(self.peers.values())
                        .filter(|actor| {
                            actor
                                .ground_seat
                                .is_some_and(|seat| seat.vehicle_index == key.index)
                        })
                        .filter_map(|actor| {
                            actor
                                .life
                                .damage_health()
                                .map(|(generation, _)| character::Target {
                                    character_id: 0x0380_0000 | u32::from(actor.reservation.player),
                                    generation,
                                    world: actor.reservation.world,
                                })
                        })
                        .collect();
                    for target in targets {
                        self.apply_ground_character_effect(
                            target,
                            character::Request::VehicleDeathOccupant {
                                last_attacker: attacker,
                                vehicle_vector: [position.x, position.y, position.z],
                            },
                            output,
                        )?;
                    }
                }
                Effect::DestructionExplosion(destruction) => {
                    if frame.domination_blast.is_some() {
                        self.stage_domination_ground_explosion(frame, key, destruction, output)?;
                    } else {
                        self.stage_acquisition_destruction_explosion(
                            frame,
                            key,
                            destruction,
                            output,
                        )?;
                    }
                }
                Effect::RequestFspDeactivation {
                    objective,
                    attacker,
                    forced: true,
                } => {
                    let world = self
                        .active
                        .iter()
                        .chain(self.peers.values())
                        .next()
                        .ok_or("acquisition_ground_world_missing")?
                        .reservation
                        .world;
                    let team = self.character_team_in_world(attacker, world).unwrap_or(0);
                    forced_fsp.push((objective, attacker, team));
                }
                Effect::DismountOccupants => {
                    let vehicle = self
                        .current_ground_vehicle(key.index)
                        .ok_or("unknown_ground_vehicle")?;
                    self.stage_acquisition_ground_ejection(&vehicle, output.now)?;
                    for actor in self
                        .active
                        .iter()
                        .chain(self.peers.values())
                        .filter(|actor| !actor.is_mounted())
                    {
                        let id = 0x0380_0000 | u32::from(actor.reservation.player);
                        if let Some(state) = output.native.get_mut(&id) {
                            state.associated_vehicle =
                                Some(protocol::VictimWasMounted::NoAssociatedVehicle);
                        }
                    }
                }
                Effect::QueueRespawn => {
                    let team = self
                        .ground_vehicles
                        .as_ref()
                        .and_then(|catalog| catalog.vehicle(key.index))
                        .ok_or("unknown_ground_vehicle")?
                        .team;
                    frame.waves.queue(team, key)?;
                    let ground = self
                        .ground_runtime
                        .as_mut()
                        .ok_or("ground_vehicle_runtime_required")?;
                    for queued in ground.admit_queued_respawn(key)? {
                        output.audit.push(json!({"op":"acquisition_ground_effect",
                            "vehicle":key.object_id(),"effect":format!("{:?}", queued.effect)}));
                    }
                    *output.changes.entry(key.index).or_default() |= 1 << 5;
                }
                Effect::VehicleDestroyed { .. }
                | Effect::RemovePhysics
                | Effect::AddPhysics
                | Effect::ActivateHulk { .. }
                | Effect::DeactivateHulk
                | Effect::SpawnDelaySet(_) => {}
                _ => return Err("unexpected_acquisition_ground_update_effect"),
            }
        }
        Ok(())
    }

    fn apply_ground_character_effect(
        &mut self,
        target: character::Target,
        request: character::Request,
        output: &mut Effects<'_>,
    ) -> Result<(), &'static str> {
        let state = output
            .native
            .get_mut(&target.character_id)
            .ok_or("acquisition_character_native_state_missing")?;
        let plan = self.plan_acquisition_character_effect(target, request, *state, output.now)?;
        output.audit.push(plan.audit().clone());
        output
            .messages
            .extend(self.commit_acquisition_character_effect(plan, state)?);
        Ok(())
    }

    fn stage_domination_ground_explosion(
        &mut self,
        frame: &GroundFrame<'_>,
        key: Key,
        blast: ground::Destruction,
        output: &mut Effects<'_>,
    ) -> Result<(), &'static str> {
        let (collision, material_penetration) = frame
            .domination_blast
            .ok_or("domination_ground_blast_context_required")?;
        let source = self
            .ground_runtime
            .as_ref()
            .and_then(|r| r.get(key.index))
            .filter(|vehicle| vehicle.key == key)
            .ok_or("stale_domination_ground_candidate")?;
        let position = source.pose.position;
        let work = self.collect_domination_blast(
            BlastSource::Ground {
                body_id: key.object_id(),
            },
            source.last_attacker,
            InstantaneousBlast {
                origins: vec![[position.x, position.y, position.z]],
                inner_radius: blast.half_radius,
                outer_radius: blast.full_radius,
                damage: blast.damage,
                material_penetration,
            },
            collision,
        )?;
        let result = self.stage_domination_damage_with_ground(
            &mut AreaDamageState::default(),
            output.native,
            frame.now,
            std::slice::from_ref(&work),
            frame.policy,
        )?;
        output.helo_doses.extend(result.helo_doses);
        output.audit.extend(result.audit);
        output.messages.extend(result.messages);
        for index in result.changed_ground {
            *output.changes.entry(index).or_default() |= 1 << 18;
        }
        Ok(())
    }

    fn stage_acquisition_destruction_explosion(
        &mut self,
        frame: &GroundFrame<'_>,
        key: Key,
        blast: ground::Destruction,
        output: &mut Effects<'_>,
    ) -> Result<(), &'static str> {
        if blast.damage == 0. || blast.full_radius == 0. {
            return Ok(());
        }
        if blast.half_radius <= 0. || blast.half_radius >= blast.full_radius {
            return Err("invalid_acquisition_ground_destruction_radii");
        }
        let scene = self
            .collision_world
            .as_ref()
            .filter(|scene| {
                scene.stats().triangles != 0
                    && self
                        .spawn_catalog
                        .as_ref()
                        .is_some_and(|spawns| spawns.map_name() == scene.metadata().map_name)
            })
            .ok_or("acquisition_destruction_collision_world_required")?
            .clone();
        let source = self
            .ground_runtime
            .as_ref()
            .and_then(|ground| ground.get(key.index))
            .filter(|vehicle| vehicle.key == key)
            .ok_or("stale_acquisition_ground_candidate")?;
        let attacker = source.last_attacker;
        let origin = [
            source.pose.position.x,
            source.pose.position.y,
            source.pose.position.z,
        ];
        let source_id = u64::from(key.object_id());
        let snapshot = self.dynamic_collision_snapshot(frame.filter_row)?;
        let world = snapshot.context().0.world_id;
        let radius = Geometry::Sphere {
            radius: f64::from(blast.full_radius),
        };
        let limits = frame.limits;
        let mut budget = limits.max_support_points;
        let mut los_budget = 1_000_000;
        let attacker_team = self.character_team_in_world(attacker, world);
        for body in snapshot.bodies() {
            if body.target.key.id == source_id
                || frame.filter_row & (1 << body.layer) == 0
                || !geometry::penetrates(
                    &radius,
                    Pose::at(origin.map(f64::from)),
                    &body.shape,
                    body.pose,
                    limits,
                    &mut budget,
                )?
            {
                continue;
            }
            let id = u32::try_from(body.target.key.id)
                .map_err(|_| "invalid_acquisition_explosion_target")?;
            let character_target = character::Target {
                character_id: id,
                generation: body.target.key.generation,
                world,
            };
            let ground_target = Key {
                index: id as u16,
                generation: body.target.key.generation,
            };
            let point = match body.target.kind {
                TargetKind::Character => {
                    let actor = self
                        .active
                        .iter()
                        .chain(self.peers.values())
                        .find(|a| a.reservation.player == id as u16);
                    // The qualified ground seats of all supported models protect occupants from this radius callback.
                    if actor.is_some_and(|a| {
                        a.ground_seat.is_some() || a.life.damage_health().is_none()
                    }) {
                        continue;
                    }
                    let height = actor.map_or(2., |a| {
                        a.owner_fields.state.map_or(2., |state| {
                            [2., 0.8, 0.3, 1.8]
                                .get(usize::from(state.stance))
                                .copied()
                                .unwrap_or(1.)
                        })
                    });
                    let mut point = self.acquisition_character_position(character_target)?;
                    point[1] += height;
                    point
                }
                TargetKind::GroundVehicle => {
                    let Some(vehicle) = self
                        .ground_runtime
                        .as_ref()
                        .and_then(|r| r.get(ground_target.index))
                        .filter(|v| {
                            v.key == ground_target && !v.is_dead() && !matches!(v.state, 4 | 5)
                        })
                    else {
                        continue;
                    };
                    [
                        vehicle.pose.position.x,
                        vehicle.pose.position.y,
                        vehicle.pose.position.z,
                    ]
                }
                _ => return Err("unsupported_acquisition_explosion_target"),
            };
            let direction: [f32; 3] = std::array::from_fn(|axis| point[axis] - origin[axis]);
            let distance = direction.iter().map(|v| v * v).sum::<f32>().sqrt();
            let scale = destruction_scale(blast, distance);
            if scale <= 0. {
                continue;
            }
            // Replacement hard LOS over the authored compiled map. Runtime
            // material penetration and dynamic occluders are separate fidelity work.
            if distance > 0.
                && scene
                    .raycast_segment_bounded(
                        origin.map(f64::from),
                        point.map(f64::from),
                        &mut los_budget,
                    )
                    .map_err(|_| "acquisition_destruction_static_los_failed")?
                    .nearest
                    .is_some_and(|hit| hit.fraction < 1.)
            {
                output
                    .audit
                    .push(json!({"op":"acquisition_destruction_static_los_blocked","target":id}));
                continue;
            }
            match body.target.kind {
                TargetKind::Character => {
                    self.apply_ground_character_effect(
                        character_target,
                        character::Request::VehicleExplosion {
                            raw_hp: blast.damage * scale,
                            from_direction: if distance > 0. {
                                direction.map(|v| v / distance)
                            } else {
                                [0.; 3]
                            },
                            attacker,
                        },
                        output,
                    )?;
                }
                TargetKind::GroundVehicle => {
                    let creation = self
                        .current_ground_vehicle(ground_target.index)
                        .ok_or("unknown_ground_vehicle")?;
                    let hp = frame
                        .policy
                        .ok_or("ground_damage_policy_required")?
                        .blast_amount(
                            &creation,
                            TargetDamageParameters {
                                amount: blast.damage,
                                armor_pierce: 0.,
                                armor_redux: 0.,
                            },
                            scale,
                            attacker_team == Some(body.target.team),
                        )?;
                    let changes = self
                        .ground_runtime
                        .as_mut()
                        .ok_or("ground_vehicle_runtime_required")?
                        .apply_explosion_damage(ground_target, hp, attacker)?;
                    if !changes.is_empty() {
                        *output.changes.entry(ground_target.index).or_default() |= 1 << 18;
                        output.audit.push(json!({"op":"acquisition_destruction_ground_blast","target":id,"hp":hp,"root":body.root}));
                    }
                }
                _ => return Err("unsupported_acquisition_explosion_target"),
            }
        }
        Ok(())
    }
}

fn destruction_scale(blast: ground::Destruction, distance: f32) -> f32 {
    if distance <= blast.half_radius {
        (-0.5_f32).mul_add(distance / blast.half_radius, 1.)
    } else if distance < blast.full_radius {
        (-0.5_f32).mul_add(
            (distance - blast.half_radius) / (blast.full_radius - blast.half_radius),
            0.5,
        )
    } else {
        0.
    }
}
