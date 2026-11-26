//! Authoritative staged Helo movement and rider disembark publication.

use crate::*;

use domination_helo::{Carrier, DisembarkGround, FlightPose, FlightState, FlightTick, HeloUpdate};

/// Dismount qualification needs the live vehicle frame. The path's tangent
/// alone cannot establish the lateral axis of a banked vehicle.
#[derive(Clone, Copy, PartialEq)]
pub(crate) struct BodyBasis {
    pub(crate) forward: [f32; 3],
    pub(crate) right: [f32; 3],
}

impl BodyBasis {
    pub(crate) fn packed_yaw(self) -> u16 {
        let rad = self.forward[0]
            .atan2(self.forward[2])
            .rem_euclid(std::f32::consts::TAU);
        ((rad / std::f32::consts::TAU * 1024.0) as u16) & 1023
    }

    pub(crate) fn place(
        self,
        position: replication::CharacterPosition,
    ) -> Result<dynamic_collision::Pose, &'static str> {
        let [rx, ry, rz] = self.right.map(f64::from);
        let [fx, fy, fz] = self.forward.map(f64::from);
        let up = [fy * rz - fz * ry, fz * rx - fx * rz, fx * ry - fy * rx];
        let pose = dynamic_collision::Pose {
            translation: [position.x, position.y, position.z].map(f64::from),
            rotation: [[rx, up[0], fx], [ry, up[1], fy], [rz, up[2], fz]],
        };
        pose.validate()?;
        Ok(pose)
    }
}

pub(crate) struct HeloFrame<'a> {
    pub(crate) dt: f32,
    pub(crate) row6: u32,
    pub(crate) now: Duration,
    pub(crate) collisions: &'a mut domination_structural_world::SequenceCollisionState,
    pub(crate) assets: &'a domination_structural_world::HeloCollisionAssets,
    pub(crate) random: &'a mut domination_helo::FlightRandom,
}

pub(crate) struct ManualHeloExit {
    pub(crate) fleet: domination_helo::FleetRuntime,
    pub(crate) transition: Option<(vehicle_exit::BunkerExit, Vec<u8>)>,
}

pub(crate) struct Exit {
    player: u16,
    generation: u32,
    state: replication::CharacterState,
    pose: vehicle_exit::BunkerExit,
    packet: Vec<u8>,
}

impl<W: Write> Engine<W> {
    pub(crate) fn prepare_manual_helo_exit(
        &self,
        actor: &Active,
        request: protocol::DismountSeat,
        sequence: u32,
        now: Duration,
    ) -> Result<ManualHeloExit, &'static str> {
        let seat = actor.helo_seat.ok_or("domination_helo_seat_not_owned")?;
        let player = actor.reservation.player;
        if request.vehicle_id & 0xff7f_ffff != (0x0200_0000 | u32::from(seat.object_index))
            || request.seat_id != u32::from(seat.seat)
            || request.character_id != (0x0380_0000 | u32::from(player))
            || !actor.life.is_alive()
            || !actor.entered_game
            || !actor.ready_for_updates
        {
            return Err("domination_helo_exit_not_authorized");
        }
        let world = self
            .domination_gameplay
            .as_ref()
            .ok_or("domination_helo_world_required")?;
        let mut fleet = world
            .state
            .helo
            .clone()
            .ok_or("domination_helo_world_required")?;
        let carrier = fleet
            .carriers
            .get_mut(usize::from(seat.object_index))
            .ok_or("domination_helo_carrier_missing")?;
        let basis = carrier
            .live_basis
            .ok_or("domination_helo_live_basis_required")?;
        let pose = carrier
            .current_pose
            .ok_or("domination_helo_position_pending")?;
        // Native class-5 dismount reads current body origin and axes.
        // Candidate geometry can refuse an early airborne dismount.
        let dynamic =
            self.owned_dynamic_collision_snapshot(world.state.filter_rows[6], Some(player))?;
        let outcome = domination_helo::settle_disembark_with_fallback(
            pose.position,
            basis.forward,
            basis.right,
            || Some(fleet.random.disembark_offset()),
            |candidate| self.qualify_helo_exit(candidate, &dynamic, &[], &world.state.collisions),
        );
        let position = match outcome {
            Ok(position) => position,
            Err("domination_helo_disembark_unresolved") => {
                return Ok(ManualHeloExit {
                    fleet,
                    transition: None,
                });
            }
            Err(error) => return Err(error),
        };
        carrier.motion.dismount_passenger(seat, player)?;
        let exit = vehicle_exit::BunkerExit {
            position,
            packed_yaw: if request.camera_yaw <= 360 {
                request.packed_yaw()
            } else {
                basis.packed_yaw()
            },
            candidate_index: 0,
        };
        let packet = ground_seats::transition(
            sequence,
            now.as_millis() as u32,
            player,
            replication::CharacterState {
                state: 1,
                stance: 0,
                user_data: [0; 3],
            },
            Some(exit),
            None,
            self.wire_profile,
        )?;
        Ok(ManualHeloExit {
            fleet,
            transition: Some((exit, packet)),
        })
    }

    fn qualify_helo_exit(
        &self,
        candidate: replication::CharacterPosition,
        dynamic: &dynamic_collision::Snapshot,
        reserved: &[replication::CharacterPosition],
        collisions: &domination_structural_world::SequenceCollisionState,
    ) -> Result<Option<replication::CharacterPosition>, &'static str> {
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
            .ok_or("domination_helo_static_world_required")?;
        let ground = DisembarkGround::new(candidate)?;
        let (start, end) = ground.ray();
        let mut budget = 1_000_000;
        let Some(hit) = scene
            .raycast_segment_bounded(start, end, &mut budget)
            .map_err(|_| "domination_helo_floor_query_failed")?
            .nearest
        else {
            return Ok(None);
        };
        let Some((feet, _)) = ground.settled(hit.position[1] as f32) else {
            return Ok(None);
        };
        let feet = correction_ack::quantized(feet)?;
        let (x, y, z) = (f64::from(feet.x), f64::from(feet.y), f64::from(feet.z));
        let Some(floor) = scene
            .raycast_segment_bounded([x, y + 1.0, z], [x, y - 1.0, z], &mut budget)
            .map_err(|_| "domination_helo_floor_query_failed")?
            .nearest
        else {
            return Ok(None);
        };
        if !(0.0..=0.2).contains(&(y - floor.position[1]))
            || reserved.iter().any(|position| {
                (position.x - feet.x).abs() < 0.7
                    && (position.y - feet.y).abs() < 1.8
                    && (position.z - feet.z).abs() < 0.7
            })
        {
            return Ok(None);
        }
        // The portable model conservatively encloses the standing capsule.
        let centre = [x, y + 0.9, z];
        let half_extents = [0.35, 0.9, 0.35];
        if scene
            .sweep_aabb(centre, centre, half_extents, &mut budget)
            .map_err(|_| "domination_helo_clearance_query_failed")?
            .is_some()
        {
            return Ok(None);
        }
        if collisions.positive_box_hit(centre, half_extents, &mut budget)? {
            return Ok(None);
        }
        let shape = dynamic_collision::Geometry::Box {
            half_extents,
            radius: 0.0,
        };
        let limits = dynamic_collision::QueryLimits::default();
        let mut supports = limits.max_support_points;
        for body in dynamic.bodies() {
            if dynamic_collision::geometry::penetrates(
                &shape,
                dynamic_collision::Pose::at(centre),
                &body.shape,
                body.pose,
                limits,
                &mut supports,
            )? {
                return Ok(None);
            }
        }
        Ok(Some(feet))
    }

    fn prepare_helo_exits(
        &self,
        carrier: &Carrier,
        pose: FlightPose,
        basis: BodyBasis,
        frame: &mut HeloFrame<'_>,
    ) -> Result<Vec<Exit>, &'static str> {
        let mut exits = Vec::new();
        let mut reserved = Vec::new();
        for (seat, player) in carrier.motion.passengers() {
            let actor = self
                .active
                .iter()
                .chain(self.peers.values())
                .find(|actor| {
                    actor.reservation.player == player
                        && actor.helo_seat
                            == Some(domination_helo::PassengerSeat {
                                object_index: carrier.source.object_index,
                                seat,
                            })
                })
                .ok_or("domination_helo_rider_missing")?;
            let (generation, state) = match actor.life {
                CharacterLife::Alive { generation, .. } => (generation, 1),
                CharacterLife::Incapacitated { generation, .. } => (generation, 7),
                _ => return Err("domination_helo_rider_generation"),
            };
            let dynamic = self.owned_dynamic_collision_snapshot(frame.row6, Some(player))?;
            let position = domination_helo::settle_disembark_with_fallback(
                pose.position,
                basis.forward,
                basis.right,
                || Some(frame.random.disembark_offset()),
                |candidate| {
                    self.qualify_helo_exit(candidate, &dynamic, &reserved, frame.collisions)
                },
            )?;
            let exit = vehicle_exit::BunkerExit {
                position,
                packed_yaw: basis.packed_yaw(),
                candidate_index: 0,
            };
            let state = replication::CharacterState {
                state,
                stance: 0,
                user_data: [0; 3],
            };
            let packet = ground_seats::transition(
                2,
                frame.now.as_millis() as u32,
                player,
                state,
                Some(exit),
                None,
                self.wire_profile,
            )?;
            exits.push(Exit {
                player,
                generation,
                state,
                pose: exit,
                packet,
            });
            reserved.push(position);
        }
        Ok(exits)
    }

    pub(crate) fn stage_helo_frame(
        &mut self,
        carrier: &mut Carrier,
        mut frame: HeloFrame<'_>,
    ) -> Result<(bool, Vec<u16>), &'static str> {
        let mut exited = Vec::new();
        let now = frame.now;
        let tick = carrier.motion.advance(frame.dt)?;
        let mut respawn_damage = false;
        let (state, elapsed) = match tick {
            FlightTick::Waiting => return Ok((false, exited)),
            FlightTick::Moving {
                state,
                elapsed,
                pose,
            } => {
                if state == FlightState::Inbound
                    && carrier.published_state == FlightState::Dormant
                    && carrier.damage.state == replication::VehicleState::Destroyed
                {
                    carrier.damage.state = replication::VehicleState::Operational;
                    respawn_damage = true;
                }
                let basis = carrier.motion.body_basis();
                if carrier.current_pose != Some(pose) || carrier.live_basis != Some(basis) {
                    frame.collisions.place_helo(
                        carrier.source.object_index,
                        carrier.source.faction,
                        basis.place(pose.position)?,
                        frame.assets,
                    )?;
                    carrier.live_basis = Some(basis);
                    carrier.current_pose = Some(pose);
                }
                for actor in self
                    .active
                    .iter_mut()
                    .chain(self.peers.values_mut())
                    .filter(|actor| {
                        actor
                            .helo_seat
                            .is_some_and(|seat| seat.object_index == carrier.source.object_index)
                    })
                {
                    actor.tracking.server_position(
                        pose.position,
                        player_tracking::PositionOrigin::ServerMount,
                        now.as_millis() as u64,
                    );
                }
                (state, elapsed)
            }
            FlightTick::CycleEnded => {
                carrier.current_pose = None;
                carrier.live_basis = None;
                frame.collisions.remove_helo(carrier.source.object_index);
                (FlightState::Dormant, 0)
            }
            FlightTick::DisembarkRequired { pose } => {
                let exits = if carrier.motion.passengers().next().is_some() {
                    match self.prepare_helo_exits(
                        carrier,
                        pose,
                        carrier
                            .live_basis
                            .ok_or("domination_helo_live_basis_required")?,
                        &mut frame,
                    ) {
                        Ok(exits) => exits,
                        // Keep the occupants at hover while the current world
                        // has no proven usable exit for the complete cabin.
                        Err("domination_helo_disembark_unresolved") => {
                            return Ok((false, exited));
                        }
                        Err(error) => return Err(error),
                    }
                } else {
                    Vec::new()
                };
                let passengers = carrier.motion.passengers().collect::<Vec<_>>();
                carrier.motion.depart_after_disembark(&passengers)?;
                for exit in exits {
                    exited.push(exit.player);
                    let actor = self
                        .active
                        .iter_mut()
                        .chain(self.peers.values_mut())
                        .find(|actor| actor.reservation.player == exit.player)
                        .ok_or("domination_helo_rider_missing")?;
                    actor.fall_tracker.reset(
                        exit.generation,
                        actor.latest_owner_sequence,
                        exit.pose.position.y,
                    )?;
                    actor.helo_seat = None;
                    actor.flash_yaw = None;
                    actor.last_exit = None;
                    actor.owner_fields.state = Some(exit.state);
                    actor.owner_fields.position = Some(exit.pose.position);
                    actor.owner_fields.yaw = Some(exit.pose.packed_yaw);
                    actor.tracking.server_position(
                        exit.pose.position,
                        player_tracking::PositionOrigin::ServerExit,
                        now.as_millis() as u64,
                    );
                    for recipient in self
                        .active
                        .iter_mut()
                        .chain(self.peers.values_mut())
                        .filter(|actor| actor.helo_creation_sent)
                    {
                        recipient.queue_shared(exit.packet.clone());
                    }
                }
                (FlightState::Outbound, 0)
            }
        };
        let old_time = std::mem::replace(&mut carrier.path_elapsed, elapsed);
        let old_state = std::mem::replace(&mut carrier.published_state, state);
        if old_time == elapsed && old_state == state {
            return Ok((false, exited));
        }
        let damage_packet = respawn_damage
            .then(|| {
                crate::domination_helo_damage::damage_delta(
                    carrier.source.object_index,
                    Some(carrier.damage.state),
                    Some(carrier.damage.wire),
                    now,
                    self.wire_profile,
                )
            })
            .transpose()?;
        let packet = HeloUpdate {
            object_index: carrier.source.object_index,
            path_elapsed: (old_time != elapsed).then_some(elapsed),
            flight_state: (old_state != state).then_some(state),
        }
        .encode(2, now.as_millis() as u32, self.wire_profile)?;
        for actor in self
            .active
            .iter_mut()
            .chain(self.peers.values_mut())
            .filter(|actor| actor.helo_creation_sent)
        {
            if let Some(ref damage) = damage_packet {
                actor.queue_shared(damage.clone());
            }
            actor.queue_shared(packet.clone());
        }
        Ok((true, exited))
    }
}
