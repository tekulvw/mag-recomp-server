//! Per-emplacement hospital seating, distinct from the legacy Alpha turret.
use crate::*;

impl<W: Write> Engine<W> {
    /// Hospital points seat a life in their own indexed emplacement: always in
    /// Domination, and in Acquisition once the profile binds its field
    /// hospitals' emplacements. Otherwise the legacy single Alpha bunker rules.
    pub(crate) fn indexed_hospital_seats(&self) -> bool {
        let profile = self.match_profile();
        (profile.game_type == 2 && profile.auto_assign_teams)
            || self
                .bunker_world
                .as_ref()
                .is_some_and(structures::BunkerWorld::has_indexed_hospitals)
    }

    pub(crate) fn bunker_ownership_after(
        &self,
        index: u16,
        player: u16,
        seat: Option<u8>,
    ) -> replication::AlphaVehicleOwnership {
        let owner = self
            .peers
            .values()
            .chain(self.active.iter())
            .filter(|actor| {
                actor.reservation.player != player && actor.mounted_bunker.unwrap_or(0) == index
            })
            .filter_map(|actor| {
                actor
                    .mounted_seat
                    .map(|seat| (actor.reservation.player, seat))
            })
            .chain(seat.map(|seat| (player, seat)))
            .min_by_key(|(player, seat)| (*seat != 0, *player));
        replication::AlphaVehicleOwnership {
            owner: owner.map_or(0x105, |(player, _)| player),
            turret_owner: owner
                .filter(|(_, seat)| *seat == 0)
                .map_or(u16::MAX, |(player, _)| player),
        }
    }

    pub(crate) fn queue_bunker_ownership(
        &mut self,
        index: u16,
        now: Duration,
    ) -> Result<(), &'static str> {
        let packet = self
            .bunker_ownership_after(index, u16::MAX, None)
            .encode_bunker_update_with_profile(
                index,
                1,
                now.as_millis() as u32,
                self.wire_profile,
            )?;
        for actor in self
            .peers
            .values_mut()
            .chain(self.active.iter_mut())
            .filter(|actor| actor.initial_objects.is_some())
        {
            actor.queue_shared(packet.clone());
        }
        Ok(())
    }

    /// Prepare real current hospital occupants through the existing normal
    /// character-damage/death/reward transaction. No authority is installed here.
    pub fn prepare_hospital_occupants(
        &mut self,
        index: u16,
        last_attacker: u32,
        native: &std::collections::BTreeMap<u32, acquisition_character_effects::NativeState>,
        now: Duration,
    ) -> Result<acquisition_character_effects::Candidate, &'static str> {
        self.prepare_acquisition_character_callback(native, now, |engine, native| {
            engine.stage_bunker_occupants(index, last_attacker, native, now)
        })
    }

    pub(crate) fn stage_bunker_occupants(
        &mut self,
        index: u16,
        last_attacker: u32,
        native: &mut std::collections::BTreeMap<u32, acquisition_character_effects::NativeState>,
        now: Duration,
    ) -> Result<(Vec<Value>, Vec<OutboundMessage>), &'static str> {
        use acquisition_character_effects::{Request, Target};
        let bunker = self
            .bunker_world
            .as_ref()
            .and_then(|world| world.hospital(index))
            .ok_or("bunker_occupant_body_required")?;
        let vehicle_vector = [bunker.position.x, bunker.position.y, bunker.position.z];
        let mut occupants: Vec<_> = self
            .active
            .iter()
            .chain(self.peers.values())
            .filter(|actor| actor.mounted_bunker == Some(index))
            .filter_map(|actor| {
                Some((
                    actor.mounted_seat?,
                    Target {
                        character_id: 0x0380_0000 | u32::from(actor.reservation.player),
                        generation: actor.life.damage_health()?.0,
                        world: actor.reservation.world,
                    },
                ))
            })
            .collect();
        occupants.sort_by_key(|(seat, target)| (*seat, target.character_id));
        let before = self.bunker_ownership_after(index, u16::MAX, None);
        let mut audit = Vec::new();
        let mut messages = Vec::new();
        for (seat, target) in occupants {
            let state = native
                .get_mut(&target.character_id)
                .ok_or("bunker_occupant_native_state_missing")?;
            let plan = self.plan_acquisition_character_effect(
                target,
                Request::VehicleDeathOccupant {
                    last_attacker,
                    vehicle_vector,
                },
                *state,
                now,
            )?;
            audit.push(json!({"op":"hospital_occupant_damage","bunker_index":index,"seat":seat,
                "character_id":target.character_id,"generation":target.generation,"effect":plan.audit()}));
            messages.extend(self.commit_acquisition_character_effect(plan, state)?);
        }
        let after = self.bunker_ownership_after(index, u16::MAX, None);
        if (before.owner, before.turret_owner) != (after.owner, after.turret_owner) {
            self.queue_bunker_ownership(index, now)?;
        }
        Ok((audit, messages))
    }

    pub(crate) fn bunker_passenger_seat(
        &self,
        bunker: &structures::IndexedHospital<'_>,
        player: u16,
    ) -> Result<u8, &'static str> {
        let occupied: Vec<_> = self
            .peers
            .values()
            .chain(self.active.iter())
            .filter(|actor| {
                actor.reservation.player != player && actor.mounted_bunker == Some(bunker.index)
            })
            .filter_map(|actor| actor.mounted_seat)
            .collect();
        if occupied.len() >= usize::from(bunker.max_occupancy) {
            return Err("bunker_seats_full");
        }
        (1..=bunker.passenger_seats)
            .rev()
            .find(|seat| !occupied.contains(seat))
            .ok_or("bunker_seats_full")
    }

    /// Source-projected exits are static candidates. Recheck the quantized body
    /// against the current world and dynamic participants before admitting one.
    pub(crate) fn current_bunker_exit(
        &self,
        index: u16,
        player: u16,
        camera: u32,
    ) -> Result<vehicle_exit::BunkerExit, &'static str> {
        let bunker = self
            .bunker_world
            .as_ref()
            .and_then(|world| world.hospital(index))
            .ok_or("bunker_exit_policy_required")?;
        let policy = bunker.exit_policy;
        let preferred = policy.exit(camera)?;
        let scene = self
            .collision_world
            .as_ref()
            .filter(|scene| {
                scene.stats().triangles != 0 && scene.metadata().map_name == self.map_name()
            })
            .ok_or("bunker_exit_current_geometry_required")?;
        if self.dynamic_body_resources.is_none() {
            return Err("bunker_exit_dynamic_geometry_required");
        }
        let snapshot = self.owned_dynamic_collision_snapshot(u32::MAX, Some(player))?;
        let limits = dynamic_collision::QueryLimits::default();
        let mut support_budget = limits.max_support_points;
        let mut static_budget = 1_000_000;
        let half_extents = [0.35, 0.9, 0.35];
        let shape = dynamic_collision::Geometry::Box {
            half_extents,
            radius: 0.,
        };
        for candidate_index in std::iter::once(preferred.candidate_index)
            .chain((0..policy.candidates.len()).filter(|i| *i != preferred.candidate_index))
        {
            let offset = policy
                .transform
                .rotate(policy.candidates[candidate_index].position_offset);
            let position = correction_ack::quantized(replication::CharacterPosition {
                x: policy.transform.position.x + offset[0],
                y: policy.transform.position.y + offset[1],
                z: policy.transform.position.z + offset[2],
            })?;
            let centre = [
                f64::from(position.x),
                f64::from(position.y) + half_extents[1],
                f64::from(position.z),
            ];
            if scene
                .sweep_aabb(centre, centre, half_extents, &mut static_budget)
                .map_err(|_| "bunker_exit_static_query_failed")?
                .is_some()
            {
                continue;
            }
            if let Some(gameplay) = &self.domination_gameplay
                && gameplay.state.collisions.positive_box_hit(
                    centre,
                    half_extents,
                    &mut static_budget,
                )?
            {
                continue;
            }
            // Acquisition's Structural sequences (standing or wrecked
            // objective bodies) block an exit the same way.
            if let Some(collision) = self
                .acquisition_gameplay
                .as_ref()
                .and_then(|runtime| runtime.blast_collision())
                && collision
                    .sequences
                    .positive_box_hit(centre, half_extents, &mut static_budget)?
            {
                continue;
            }
            let mut occupied = false;
            for body in snapshot
                .bodies()
                .iter()
                .filter(|body| body.target.key.id != u64::from(0x0380_0000 | u32::from(player)))
            {
                if dynamic_collision::geometry::penetrates(
                    &shape,
                    dynamic_collision::Pose::at(centre),
                    &body.shape,
                    body.pose,
                    limits,
                    &mut support_budget,
                )? {
                    occupied = true;
                    break;
                }
            }
            if !occupied {
                return Ok(vehicle_exit::BunkerExit {
                    candidate_index,
                    position,
                    packed_yaw: preferred.packed_yaw,
                });
            }
        }
        Err("bunker_exit_no_qualified_current_candidate")
    }
}
