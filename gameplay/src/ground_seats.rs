//! Generic passenger authority. All model seats and exit offsets are external.
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

/// Character-only state correction. Passenger occupancy does not grant vehicle
/// owner or turret authority; no unevidenced ground movement fields are emitted.
pub(crate) fn transition(
    sequence: u32,
    timestamp_ms: u32,
    player: u16,
    state: replication::CharacterState,
    exit: Option<vehicle_exit::BunkerExit>,
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
        object_type_mask: 1 << 3,
        timestamp_ms,
    }
    .encode(&mut bits)?;
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
