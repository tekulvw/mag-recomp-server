//! Source-bound ground seating. All model seats and exit offsets are external.
use crate::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PassengerSeat {
    pub vehicle_index: u16,
    pub seat: u8,
}
impl PassengerSeat {
    pub(crate) fn character_state(self) -> replication::CharacterState {
        replication::CharacterState {
            state: 14,
            stance: 0,
            user_data: [1, self.vehicle_index as u8, self.seat],
        }
    }
}
impl Active {
    pub(crate) fn is_mounted(&self) -> bool {
        self.mounted_seat.is_some() || self.ground_seat.is_some()
    }
}
impl<W: Write> Engine<W> {
    pub(crate) fn ground_exit(
        &self,
        vehicle: &ground_vehicles::Creation,
        actor_player: u16,
    ) -> Result<vehicle_exit::BunkerExit, &'static str> {
        self.ground_exit_with_reserved(vehicle, actor_player, &[])
    }

    pub(crate) fn ground_exit_with_reserved(
        &self,
        vehicle: &ground_vehicles::Creation,
        actor_player: u16,
        reserved: &[replication::CharacterPosition],
    ) -> Result<vehicle_exit::BunkerExit, &'static str> {
        let exit = vehicle.exit_policy().ok_or("ground_exit_policy_required")?;
        let mut selected = exit.exit(u32::MAX)?;
        if self.ground_driver_seat(vehicle.index).is_none() {
            return Ok(selected);
        }
        // Existing stationary offsets only identify model candidates. At an
        // Escort's current pose, independently ground and qualify each one.
        let world = self
            .collision_world
            .as_ref()
            .filter(|scene| {
                scene.stats().triangles != 0
                    && self
                        .spawn_catalog
                        .as_ref()
                        .is_some_and(|spawn| spawn.map_name() == scene.metadata().map_name)
            })
            .ok_or("ground_exit_current_geometry_required")?;
        let dynamic = self
            .dynamic_body_resources
            .as_ref()
            .map(|_| self.dynamic_collision_snapshot(u32::MAX))
            .transpose()?;
        let pose = [vehicle.position.x, vehicle.position.y, vehicle.position.z].map(f64::from);
        let limits = dynamic_collision::QueryLimits::default();
        let mut dynamic_budget = limits.max_support_points;
        let mut static_budget = 1_000_000;
        let character_half_extents = [0.35, 0.9, 0.35];

        for (index, candidate) in exit.candidates.iter().enumerate() {
            let offset = exit
                .transform
                .rotate(candidate.ranking_offset)
                .map(f64::from);
            let [x, y, z] = std::array::from_fn(|axis| pose[axis] + offset[axis]);
            let Some(floor) = world
                .raycast_segment_bounded([x, y + 1.8, z], [x, y - 50., z], &mut static_budget)
                .map_err(|_| "ground_exit_static_query_failed")?
                .nearest
            else {
                continue;
            };
            if (floor.position[1] - y).abs() > 2.5 {
                continue;
            }
            let grounded = correction_ack::quantized(replication::CharacterPosition {
                x: x as f32,
                y: (floor.position[1] + 0.1) as f32,
                z: z as f32,
            })?;
            let [x_wire, y_wire, z_wire] = [grounded.x, grounded.y, grounded.z].map(f64::from);
            let Some(wire_floor) = world
                .raycast_segment_bounded(
                    [x_wire, y_wire + 1.8, z_wire],
                    [x_wire, y_wire - 50., z_wire],
                    &mut static_budget,
                )
                .map_err(|_| "ground_exit_static_query_failed")?
                .nearest
            else {
                continue;
            };
            if !(0. ..=0.2).contains(&(y_wire - wire_floor.position[1])) {
                continue;
            }
            if reserved.iter().any(|other| {
                (other.x - grounded.x).abs() < 2. * character_half_extents[0] as f32
                    && (other.y - grounded.y).abs() < 2. * character_half_extents[1] as f32
                    && (other.z - grounded.z).abs() < 2. * character_half_extents[2] as f32
            }) {
                continue;
            }
            let centre = [x_wire, y_wire + character_half_extents[1], z_wire];
            // AABBs conservatively enclose the selected standing body and
            // native LOS sphere. Borders and unresolved geometry deny exits.
            if world
                .sweep_aabb(centre, centre, character_half_extents, &mut static_budget)
                .map_err(|_| "ground_exit_static_query_failed")?
                .is_some()
                || world
                    .sweep_aabb([x, y, z], pose, [0.25; 3], &mut static_budget)
                    .map_err(|_| "ground_exit_static_query_failed")?
                    .is_some()
            {
                continue;
            }
            if let Some(dynamic) = &dynamic {
                let actor = u64::from(0x0380_0000 | u32::from(actor_player));
                let upright = dynamic_collision::Geometry::Box {
                    half_extents: character_half_extents,
                    radius: 0.,
                };
                let mut occupied = false;
                for body in dynamic
                    .bodies()
                    .iter()
                    .filter(|body| body.target.key.id != actor)
                {
                    if dynamic_collision::geometry::penetrates(
                        &upright,
                        dynamic_collision::Pose::at(centre),
                        &body.shape,
                        body.pose,
                        limits,
                        &mut dynamic_budget,
                    )? {
                        occupied = true;
                        break;
                    }
                }
                if occupied {
                    continue;
                }
            }
            selected.candidate_index = index;
            selected.position = grounded;
            return Ok(selected);
        }
        Err("ground_exit_no_qualified_current_candidate")
    }

    pub(crate) fn ground_occupancy_full(&self, vehicle: &ground_vehicles::Creation) -> bool {
        let occupants = self
            .peers
            .values()
            .chain(self.active.iter())
            .filter(|actor| {
                actor
                    .ground_seat
                    .is_some_and(|seat| seat.vehicle_index == vehicle.index)
            })
            .count();
        occupants >= usize::from(vehicle.maximum_occupancy.unwrap_or(1))
    }

    pub(crate) fn ground_passenger_seat(
        &self,
        vehicle: &ground_vehicles::Creation,
    ) -> Option<PassengerSeat> {
        let occupants: Vec<_> = self
            .peers
            .values()
            .chain(self.active.iter())
            .filter_map(|actor| actor.ground_seat)
            .filter(|seat| seat.vehicle_index == vehicle.index)
            .collect();
        if occupants.len() >= usize::from(vehicle.maximum_occupancy?) {
            return None;
        }
        vehicle
            .passenger_seats
            .iter()
            .copied()
            .filter(|seat| !occupants.iter().any(|used| used.seat == *seat))
            .max()
            .map(|seat| PassengerSeat {
                vehicle_index: vehicle.index,
                seat,
            })
    }
}

/// Shared mounted state plus an optional common owner grant or revocation.
pub(crate) fn transition(
    sequence: u32,
    timestamp_ms: u32,
    player: u16,
    state: replication::CharacterState,
    exit: Option<vehicle_exit::BunkerExit>,
    authority: Option<(u16, u16)>,
    profile: wire_profile::WireProfile,
) -> Result<Vec<u8>, &'static str> {
    if sequence <= 1 || player >= 256 {
        return Err("invalid_ground_seat_transition");
    }
    let mut bits = replication::BitWriter::with_profile(profile);
    replication::UpdateHeader {
        sequence,
        nak_sequence: 0,
        client_index: 0,
        object_type_mask: (1 << 3) | (u32::from(authority.is_some()) << 1),
        timestamp_ms,
    }
    .encode(&mut bits)?;
    if let Some((vehicle, owner)) = authority {
        ground_vehicles::write_owner_field(&mut bits, vehicle, owner)?;
    }
    bits.write(1, 1)?;
    bits.write(player.into(), 16)?;
    for field in 0..22 {
        bits.write(
            u32::from(field == 4 || (exit.is_some() && [5, 7].contains(&field))),
            1,
        )?;
        match field {
            4 => state.encode(&mut bits)?,
            5 => {
                if let Some(exit) = exit {
                    exit.position.encode(&mut bits)?;
                }
            }
            7 => {
                if let Some(exit) = exit {
                    bits.write(exit.packed_yaw.into(), 10)?;
                }
            }
            _ => (),
        }
    }
    bits.write(0, 1)?;
    Ok(bits.finish())
}

/// Manual remounting has no implicit model radius or freshness timeout.
pub(crate) fn authorize_mount_position(
    active: &Active,
    vehicle: &ground_vehicles::Creation,
    now: Duration,
) -> Result<(), &'static str> {
    let distance = vehicle
        .maximum_mount_distance_m
        .ok_or("ground_mount_distance_policy_required")?;
    let maximum_age = vehicle
        .maximum_mount_position_age_ms
        .ok_or("ground_mount_freshness_policy_required")?;
    let sample = active
        .tracking
        .sample
        .ok_or("ground_mount_position_unavailable")?;
    let age = (now.as_millis() as u64)
        .checked_sub(sample.received_at_ms)
        .ok_or("ground_mount_position_stale")?;
    if age > maximum_age
        || (matches!(
            sample.origin,
            player_tracking::PositionOrigin::ClientReported
        ) && (sample.client_sequence != Some(active.tracking.accepted_position_sequence)
            || sample
                .client_sequence
                .is_none_or(|sequence| sequence > active.latest_owner_sequence)))
    {
        return Err("ground_mount_position_stale");
    }
    sample.position.validate()?;
    let delta = [
        sample.position.x - vehicle.position.x,
        sample.position.y - vehicle.position.y,
        sample.position.z - vehicle.position.z,
    ];
    let squared = delta
        .iter()
        .map(|value| f64::from(*value).powi(2))
        .sum::<f64>();
    if squared > f64::from(distance).powi(2) {
        return Err("ground_mount_out_of_range");
    }
    Ok(())
}
