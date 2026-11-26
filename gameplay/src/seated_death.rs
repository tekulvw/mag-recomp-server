//! A death in a seat is a seat exit: docs/seated-death-dismount.md.
//!
//! Every lethal or downing Character update for a human is built by
//! `Engine::character_damage_update`, and every seat mirror is released on
//! death by `Active::release_seat_on_death`, so no death path can publish a
//! downed state that a still-seated client would ignore.
use crate::*;

/// The seat a character occupied, in the family's own identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SeatIdentity {
    /// `index` is None for the legacy Alpha bunker.
    Bunker {
        index: Option<u16>,
        seat: u8,
    },
    Ground(ground_seats::PassengerSeat),
    Helo(domination_helo::PassengerSeat),
}

impl SeatIdentity {
    /// Whether a dismount request names this seat.
    pub(crate) fn matches(self, request: &protocol::DismountSeat) -> bool {
        match self {
            Self::Bunker { index, seat } => {
                request.vehicle_id == 0x0d00_0000 | u32::from(index.unwrap_or(0))
                    && request.seat_id == u32::from(seat)
            }
            Self::Ground(seat) => {
                request.vehicle_id == 0x0100_0000 | u32::from(seat.vehicle_index)
                    && request.seat_id == u32::from(seat.seat)
            }
            Self::Helo(seat) => {
                request.vehicle_id & 0xff7f_ffff == 0x0200_0000 | u32::from(seat.object_index)
                    && request.seat_id == u32::from(seat.seat)
            }
        }
    }
}

impl Active {
    /// The seat this character currently occupies, if any.
    pub(crate) fn seat_identity(&self) -> Option<SeatIdentity> {
        if let Some(seat) = self.ground_seat {
            Some(SeatIdentity::Ground(seat))
        } else if let Some(seat) = self.helo_seat {
            Some(SeatIdentity::Helo(seat))
        } else {
            self.mounted_seat.map(|seat| SeatIdentity::Bunker {
                index: self.mounted_bunker,
                seat,
            })
        }
    }

    /// The only place a death releases a character's seat mirrors. The seat
    /// is remembered so a dismount retry from the dead occupant can be
    /// answered with the terminal exit instead of a refusal.
    pub(crate) fn release_seat_on_death(&mut self) {
        if let Some(seat) = self.seat_identity() {
            self.terminal_seat = Some(seat);
        }
        self.mounted_seat = None;
        self.mounted_bunker = None;
        self.ground_seat = None;
        self.helo_seat = None;
    }
}

impl<W: Write> Engine<W> {
    fn actor_for_character(&self, character_id: u32) -> Option<&Active> {
        if character_id >> 16 != 0x0380 {
            return None;
        }
        self.active
            .iter()
            .chain(self.peers.values())
            .find(|actor| u32::from(actor.reservation.player) == character_id & 0xffff)
    }

    /// The terminal exit for a character still in its seat.
    pub(crate) fn seated_death_exit(
        &self,
        actor: &Active,
    ) -> Option<replication::TerminalSeatExit> {
        self.seat_exit(actor, actor.seat_identity()?)
    }

    /// Resolve where `actor` leaves `seat`: the family's current exit, its
    /// authored exit, or finally the occupant's last tracked pose.
    pub(crate) fn seat_exit(
        &self,
        actor: &Active,
        seat: SeatIdentity,
    ) -> Option<replication::TerminalSeatExit> {
        let player = actor.reservation.player;
        let tracked = || {
            actor
                .tracking
                .sample
                .map(|sample| sample.position)
                .or(actor.owner_fields.position)
                .and_then(|position| correction_ack::quantized(position).ok())
                .map(|position| (position, actor.owner_fields.yaw.unwrap_or(0) & 1023))
        };
        let mut bunker_ownership = None;
        let mut ground_owner = None;
        let pose = match seat {
            SeatIdentity::Bunker {
                index: Some(index), ..
            } => {
                bunker_ownership = Some((index, self.bunker_ownership_after(index, player, None)));
                self.current_bunker_exit(index, player, u32::MAX)
                    .ok()
                    .or_else(|| {
                        self.bunker_world
                            .as_ref()?
                            .hospital(index)?
                            .exit_policy
                            .exit(u32::MAX)
                            .ok()
                    })
                    .and_then(|exit| {
                        Some((
                            correction_ack::quantized(exit.position).ok()?,
                            exit.packed_yaw,
                        ))
                    })
            }
            SeatIdentity::Bunker { index: None, .. } => {
                bunker_ownership = Some((0, self.alpha_ownership_after(player, None)));
                self.bunker_exit_policy
                    .as_ref()
                    .and_then(|policy| policy.exit(u32::MAX).ok())
                    .and_then(|exit| {
                        Some((
                            correction_ack::quantized(exit.position).ok()?,
                            exit.packed_yaw,
                        ))
                    })
            }
            SeatIdentity::Ground(seat) => {
                if self.ground_driver_owner(seat.vehicle_index) == Some(player) {
                    ground_owner = Some((seat.vehicle_index, ground_vehicles::SERVER_OWNER));
                }
                self.current_ground_vehicle(seat.vehicle_index)
                    .and_then(|vehicle| {
                        self.ground_exit(&vehicle, player)
                            .ok()
                            .or_else(|| vehicle.exit_policy()?.exit(u32::MAX).ok())
                    })
                    .and_then(|exit| {
                        Some((
                            correction_ack::quantized(exit.position).ok()?,
                            exit.packed_yaw,
                        ))
                    })
            }
            SeatIdentity::Helo(seat) => self
                .domination_gameplay
                .as_ref()
                .and_then(|world| world.state.helo.as_ref())
                .and_then(|fleet| fleet.carriers.get(usize::from(seat.object_index)))
                .and_then(|carrier| {
                    let pose = carrier.current_pose.as_ref()?;
                    let position = correction_ack::quantized(
                        domination_helo::mounted_wire_position(pose.position),
                    )
                    .ok()?;
                    let yaw = carrier.live_basis.map_or(0, |basis| basis.packed_yaw());
                    Some((position, yaw))
                }),
        };
        let (position, packed_yaw) = pose.or_else(tracked)?;
        Some(replication::TerminalSeatExit {
            position,
            packed_yaw,
            bunker_ownership,
            ground_owner,
        })
    }

    /// Build a human or bot Character damage update. A lethal or downing
    /// update (forced state 7) for a seated human carries the seat exit.
    pub(crate) fn character_damage_update(
        &self,
        sequence: u32,
        timestamp_ms: u32,
        object_index: u16,
        damage: replication::CharacterDamage,
        downed: bool,
    ) -> replication::CharacterDamageUpdate {
        let seated_exit = downed
            .then(|| self.actor_for_character(0x0380_0000 | u32::from(object_index)))
            .flatten()
            .and_then(|actor| self.seated_death_exit(actor));
        replication::CharacterDamageUpdate {
            sequence,
            timestamp_ms,
            object_index,
            damage,
            forced_state: downed.then_some(replication::CharacterState {
                state: 7,
                stance: 0,
                user_data: [0; 3],
            }),
            seated_exit,
        }
    }
}

#[cfg(test)]
#[path = "seated_death_tests.rs"]
pub(crate) mod tests;
