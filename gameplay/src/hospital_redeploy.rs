//! Indexed Hospital terminal occupancy and reliable owner publication.
use crate::*;

pub(crate) struct HospitalRedeploy {
    pub(crate) association: protocol::VictimWasMounted,
    pub(crate) ownership: Vec<u8>,
}

impl<W: Write> Engine<W> {
    pub(crate) fn prepare_hospital_redeploy(
        &self,
        actor: &Active,
        now: Duration,
    ) -> Result<HospitalRedeploy, &'static str> {
        let index = actor
            .mounted_bunker
            .ok_or("hospital_redeploy_index_required")?;
        let seat = actor
            .mounted_seat
            .ok_or("hospital_redeploy_seat_required")?;
        if actor.ground_seat.is_some() || actor.helo_seat.is_some() {
            return Err("hospital_redeploy_conflicting_mount");
        }
        let world = self
            .bunker_world
            .as_ref()
            .ok_or("hospital_redeploy_world_required")?;
        let bunker = world
            .hospital(index)
            .ok_or("hospital_redeploy_index_unbound")?;
        if actor.emplacement_creation.is_none()
            || bunker.seat_position(seat).is_none()
            || (seat == 0 && !bunker.turret_qualified)
            || self
                .active
                .iter()
                .chain(self.peers.values())
                .filter(|other| {
                    other.reservation.world == actor.reservation.world
                        && other.mounted_bunker == Some(index)
                        && other.mounted_seat == Some(seat)
                })
                .count()
                != 1
            || (seat == 0
                && world
                    .hospital_turrets
                    .get(&index)
                    .is_some_and(|turret| turret.has_grant() && !turret.grant_matches(actor)))
        {
            return Err("hospital_redeploy_occupant_unbound");
        }
        // Both the mandatory ownership correction and the following terminal
        // health relay must fit before the recorded state transition commits.
        self.check_hospital_turret_queue(
            self.active
                .iter()
                .chain(self.peers.values())
                .filter(|recipient| {
                    recipient.reservation.world == actor.reservation.world
                        && recipient.initial_objects.is_some()
                        && recipient.emplacement_creation.is_some()
                }),
            1,
            |recipient| {
                if recipient.reservation.id == actor.reservation.id {
                    usize::from(actor.ordered_health_pending && !actor.pending_shared.is_empty())
                } else {
                    usize::from(self.maximum_players > 1)
                }
            },
        )
        .map_err(|_| "hospital_redeploy_publication_budget_exceeded")?;
        let ownership = self
            .bunker_ownership_after(index, actor.reservation.player, None)
            .encode_bunker_update_with_profile(
                index,
                1,
                now.as_millis() as u32,
                self.wire_profile,
            )?;
        let association = if actor
            .owner_fields
            .state
            .is_some_and(|state| state.state == 14)
        {
            protocol::VictimWasMounted::AssociatedVehicle
        } else {
            protocol::VictimWasMounted::NoAssociatedVehicle
        };
        Ok(HospitalRedeploy {
            association,
            ownership,
        })
    }
}

#[cfg(test)]
#[path = "hospital_redeploy_tests.rs"]
mod tests;
