//! Staged Helo passenger release for owned terminal redeploy.

use crate::domination_helo::FleetRuntime;
use crate::protocol::VictimWasMounted;
use crate::{Active, Engine};
use std::io::Write;

impl<W: Write> Engine<W> {
    pub(crate) fn prepare_helo_redeploy(
        &self,
        actor: &Active,
    ) -> Result<(FleetRuntime, VictimWasMounted), &'static str> {
        let seat = actor.helo_seat.ok_or("helo_redeploy_seat_required")?;
        if actor.ground_seat.is_some()
            || actor.mounted_seat.is_some()
            || actor.mounted_bunker.is_some()
        {
            return Err("helo_redeploy_conflicting_mount");
        }
        let fleet = self
            .domination_gameplay
            .as_ref()
            .and_then(|world| world.state.helo.as_ref())
            .ok_or("helo_redeploy_fleet_required")?;
        let carrier_index = usize::from(seat.object_index);
        let source = &fleet
            .carriers
            .get(carrier_index)
            .ok_or("helo_redeploy_carrier_missing")?
            .source;
        if source.object_index != seat.object_index
            || seat.seat < source.passenger_start
            || !actor.helo_creation_sent
            || source.faction != actor.reservation.character.faction
            || !source
                .squads
                .contains(&(actor.squad_catalog.squad_index() as u8))
        {
            return Err("helo_redeploy_carrier_source_mismatch");
        }

        // Require one exact row in the fleet and one matching actor mirror.
        let player = actor.reservation.player;
        let fleet_rows = fleet
            .carriers
            .iter()
            .enumerate()
            .flat_map(|(index, carrier)| {
                carrier
                    .motion
                    .passengers()
                    .filter_map(move |(slot, rider)| (rider == player).then_some((index, slot)))
            });
        let actor_mirrors = self
            .active
            .iter()
            .chain(self.peers.values())
            .filter(|other| {
                other.reservation.world == actor.reservation.world
                    && (other.reservation.player == player || other.helo_seat == Some(seat))
            })
            .map(|other| (other.reservation.id.as_str(), other.helo_seat));
        if !fleet_rows.eq([(carrier_index, seat.seat)])
            || !actor_mirrors.eq([(actor.reservation.id.as_str(), Some(seat))])
        {
            return Err("helo_redeploy_occupant_unbound");
        }

        // Terminal redeploy samples association before releasing the row. No
        // disembark pose or operational hull is needed for this life transition.
        let association = if actor
            .owner_fields
            .state
            .is_some_and(|state| state.state == 14)
        {
            VictimWasMounted::AssociatedVehicle
        } else {
            VictimWasMounted::NoAssociatedVehicle
        };
        let mut candidate = fleet.clone();
        candidate.carriers[carrier_index]
            .motion
            .dismount_passenger(seat, player)?;
        Ok((candidate, association))
    }
}

#[cfg(test)]
#[path = "helo_redeploy_tests.rs"]
mod tests;
