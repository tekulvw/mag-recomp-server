//! Indexed hospital secondary launch admission and server-owned projectile flight.
use super::*;
use crate::{
    acquisition_volume_damage::{TargetKey, TargetKind},
    domination_structural_world::{
        BlastCollision, GroundRocketCandidate, GroundRocketDose, HeloRocketCandidate,
        HeloRocketDose, HospitalBlastCandidate,
    },
    dynamic_collision::QueryLimits,
    hospital_rockets::{Blast, Muzzle, MuzzleNode, Weapon},
    projectile::{
        MAX_PROJECTILE_SLOTS, ProjectileCreation, ProjectileOwnerFields, ProjectileOwnerUpdate,
    },
    projectile_contact::SphereSweep,
    projectile_motion::{Advance, Contact, Limits, Motion, Terminal, TerminalReason},
    projectile_replication::{ProjectileOwnerDelta, ProjectileRemoval},
    projectile_scene::{OwnedContact, OwnedObstacleId, Scene},
    replication::{CharacterPosition, UpdateHeader},
};

const REQUEST_BYTES: usize = 57;
const ORIGIN_TOLERANCE_M: f64 = 2.0;
const MIN_FORWARD_DOT: f64 = 0.94;
const MAX_LIFETIME_MS: u64 = 70_000;
const MAX_ADVANCES_PER_TICK: usize = 16;
const QUERY_RETRY_MS: u64 = 2_000;
const POSITION_INTERVAL: Duration = Duration::from_millis(200);
const CLEANUP_GRACE: Duration = Duration::from_millis(500);

#[derive(Clone, Debug, Default)]
pub(crate) struct Flights {
    slots: [Option<Slot>; MAX_PROJECTILE_SLOTS],
}

#[derive(Clone, Debug)]
enum Slot {
    Flying(Flight),
    Cleanup { flight: Flight, due: Duration },
}

#[derive(Clone, Debug)]
pub(crate) struct Flight {
    source: LaunchSource,
    hospital: u16,
    grant_version: u8,
    fingerprint: [u8; REQUEST_BYTES],
    creation: ProjectileCreation,
    motion: Motion,
    blast: Blast,
    last_published: Duration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Magazine {
    rounds: u8,
    reserve_magazines: u8,
    next_fire: Duration,
    refill: Option<Duration>,
}

impl Magazine {
    fn after_shot(
        current: Option<Self>,
        weapon: Weapon,
        now: Duration,
    ) -> Result<Self, &'static str> {
        let mut next = current.unwrap_or(Self {
            rounds: weapon.magazine_size,
            reserve_magazines: weapon.initial_magazine_count.saturating_sub(1),
            next_fire: now,
            refill: None,
        });
        if now < next.next_fire {
            return Err("hospital_rocket_refire");
        }
        if next.rounds == 0 {
            let refill = next.refill.ok_or("hospital_rocket_ammo_empty")?;
            if now < refill {
                return Err("hospital_rocket_reloading");
            }
            if !weapon.infinite_ammo {
                next.reserve_magazines -= 1;
            }
            next.refill = None;
            next.rounds = weapon.magazine_size;
        }
        next.rounds = next
            .rounds
            .checked_sub(1)
            .ok_or("hospital_rocket_ammo_empty")?;
        next.next_fire = after_seconds(now, weapon.refire_seconds)?;
        if next.rounds == 0 && (weapon.infinite_ammo || next.reserve_magazines > 0) {
            next.refill = Some(after_seconds(now, weapon.reload_seconds)?);
        }
        Ok(next)
    }
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct LaunchSource {
    pub(crate) allocation_sequence: u64,
    pub(crate) reservation: String,
    pub(crate) world: u32,
    pub(crate) round: u64,
    pub(crate) match_id: String,
    pub(crate) character_id: Option<u32>,
    pub(crate) player: u16,
    pub(crate) life_generation: u32,
    pub(crate) team: u8,
    pub(crate) item: u16,
    pub(crate) projectile_id: u32,
}

pub(crate) enum LaunchDisposition {
    Created(Box<Launch>),
    Duplicate,
}

pub(crate) struct Launch {
    pub(crate) hospital: u16,
    pub(crate) slot: u8,
    pub(crate) turret: hospital_turrets::Runtime,
    pub(crate) flight: Flight,
    pub(crate) packet: Vec<u8>,
    pub(crate) audit: serde_json::Value,
}

impl Flights {
    pub(crate) fn occupies(&self, slot: u8) -> bool {
        self.slots
            .get(usize::from(slot))
            .is_some_and(Option::is_some)
    }

    pub(crate) fn launch(&mut self, slot: u8, flight: Flight) {
        let entry = &mut self.slots[usize::from(slot)];
        debug_assert!(entry.is_none());
        *entry = Some(Slot::Flying(flight));
    }

    pub(crate) fn snapshot_packets(
        &self,
        timestamp: u32,
        profile: wire_profile::WireProfile,
    ) -> Result<Vec<Vec<u8>>, &'static str> {
        self.slots
            .iter()
            .filter_map(|entry| match entry.as_ref()? {
                Slot::Flying(flight) => Some(flight),
                Slot::Cleanup { .. } => None,
            })
            .map(|flight| {
                let mut creation = flight.creation;
                creation.position = wire_position(flight.motion.position());
                creation
                    .encode_with_profile(projectile_header(creation.slot, 1, timestamp), profile)
            })
            .collect()
    }

    pub(crate) fn removals(&self) -> impl Iterator<Item = ProjectileRemoval> + '_ {
        self.slots
            .iter()
            .flatten()
            .map(|slot| slot.flight().removal())
    }
}

impl Slot {
    fn flight(&self) -> &Flight {
        match self {
            Self::Flying(flight) | Self::Cleanup { flight, .. } => flight,
        }
    }
}

impl Flight {
    fn removal(&self) -> ProjectileRemoval {
        ProjectileRemoval {
            slot: self.creation.slot,
            object_index: self.source.player,
        }
    }
}

impl<W: Write> Engine<W> {
    pub(crate) fn plan_hospital_rocket_launch(
        &self,
        actor: &Active,
        request: protocol::FireGrenade,
        fingerprint: &[u8],
        now: Duration,
        next_sequence: u32,
    ) -> Result<LaunchDisposition, &'static str> {
        let catalog = self
            .hospital_rockets
            .as_ref()
            .ok_or("hospital_rocket_rules_required")?;
        if self.wire_profile.application_id() != 20231
            || self.match_profile().game_type != 2
            || !self.match_started
            || !self.round_combat_allowed(now)
        {
            return Err("hospital_rocket_combat_inactive");
        }
        let player = actor.reservation.player;
        let shooter_id = 0x0380_0000 | u32::from(player);
        if request.shooter_id != shooter_id || request.shooter_client_index != player {
            return Err("unowned_projectile_shooter");
        }
        // The first replacement policy admits stationary, untargeted full-power
        // launches. Original modifiers and inherited launch motion can vary.
        if request.ammo_index != 1
            || request.throw_power != 1.0
            || request.initial_velocity != [0.0; 3]
            || request.target_id != u32::MAX
        {
            return Err("unsupported_hospital_rocket_launch_parameters");
        }
        let slot = u8::try_from(request.projectile_index)
            .ok()
            .filter(|slot| *slot < projectile::projectile_slot_count(self.wire_profile))
            .ok_or("invalid_hospital_rocket_slot")?;
        let CharacterLife::Alive { generation, .. } = actor.life else {
            return Err("hospital_rocket_requires_living_owner");
        };
        let grant = self.hospital_secondary_for_actor(actor)?;
        let variant = catalog
            .secondary_for(grant.faction, grant.item_id)
            .ok_or("hospital_rocket_weapon_unqualified")?;
        let fingerprint: [u8; REQUEST_BYTES] = fingerprint
            .try_into()
            .map_err(|_| "invalid_hospital_rocket_launch_bytes")?;
        if let Some(entry) = &actor.hospital_rocket_flights.slots[usize::from(slot)] {
            let previous = entry.flight();
            if previous.fingerprint == fingerprint
                && previous.hospital == grant.index
                && previous.grant_version == grant.version
                && previous.source.life_generation == generation
            {
                return Ok(LaunchDisposition::Duplicate);
            }
            return Err("projectile_slot_in_use");
        }
        if actor
            .projectiles
            .get(slot)
            .is_some_and(|frag| !frag.completed)
            || actor.infantry_rocket_state.occupies(slot)
        {
            return Err("projectile_slot_in_use");
        }
        if self.projectile_cleanup_pending(actor, slot) {
            return Err("projectile_cleanup_pending");
        }
        self.check_hospital_turret_queue(
            self.peers
                .values()
                .filter(|peer| peer.initial_objects.is_some()),
            1,
            |_| 0,
        )?;

        let mut turret = grant.turret.clone();
        let magazine = Magazine::after_shot(turret.rocket_magazine, variant.weapon, now)?;
        turret.rocket_magazine = Some(magazine);
        let (origin, orientation) =
            muzzle_pose(grant.body, variant.muzzle, grant.yaw, grant.pitch)?;
        let aim_forward = rotate(orientation, [0.0, 0.0, 1.0]);
        let reported_origin = request.position.map(f64::from);
        let reported_rotation = unit(request.rotation_xyzw.map(f64::from))?;
        let forward = rotate(reported_rotation, [0., 0., 1.]);
        let origin_error = (0..3)
            .map(|axis| origin[axis] - reported_origin[axis])
            .fold(0.0_f64, f64::hypot);
        let forward_dot = aim_forward
            .into_iter()
            .zip(forward)
            .map(|(a, b)| a * b)
            .sum::<f64>();
        if !origin_error.is_finite()
            || origin_error > ORIGIN_TOLERANCE_M
            || forward_dot < MIN_FORWARD_DOT
        {
            return Err("hospital_rocket_reported_muzzle_mismatch");
        }
        let speed = f64::from(variant.flight.speed_mps);
        let motion = Motion::launch(
            origin,
            forward.map(|axis| axis * speed),
            millis(now)?,
            Limits {
                radius: variant.flight.radius_m.into(),
                lifetime_ms: authored_millis(variant.flight.lifetime_seconds)?,
                grace_ms: authored_millis(variant.flight.age_grace_seconds)?,
                maximum_speed_mps: speed * (1.0 + 1e-12),
                maximum_duration_ms: MAX_LIFETIME_MS,
            },
        )?;
        let source = LaunchSource {
            allocation_sequence: actor.allocation_sequence,
            reservation: actor.reservation.id.clone(),
            world: actor.reservation.world,
            round: self.round_number,
            match_id: actor.reward_match_id.clone(),
            character_id: actor.reservation.character.character_id,
            player,
            life_generation: generation,
            team: actor.team(),
            item: variant.secondary_item_id,
            projectile_id: ((16 + u32::from(slot)) << 24) | 0x0080_0000 | u32::from(player),
        };
        let creation = ProjectileCreation {
            slot,
            object_index: player,
            owner_index: player,
            ammo_token: variant.ammo_token,
            position: wire_position(origin),
            rotation_xyzw: reported_rotation.map(|axis| axis as f32),
            shooter_id,
        };
        let packet = creation.encode_with_profile(
            projectile_header(slot, next_sequence, now.as_millis() as u32),
            self.wire_profile,
        )?;
        let audit = json!({
            "type":"hospital_rocket_launch","disposition":"created",
            "hospital":grant.index,"slot":slot,"grant_version":grant.version,
            "source":source,"origin":origin,"forward":forward,
            "ammo_remaining":magazine.rounds,
        });
        Ok(LaunchDisposition::Created(Box::new(Launch {
            hospital: grant.index,
            slot,
            turret,
            flight: Flight {
                source,
                hospital: grant.index,
                grant_version: grant.version,
                fingerprint,
                creation,
                motion,
                blast: variant.blast,
                last_published: now,
            },
            packet,
            audit,
        })))
    }
}

enum End {
    Detonation(Terminal<OwnedContact>),
    Retired(&'static str),
}

struct MotionInstallation {
    flight: Flight,
    packet: Option<Vec<u8>>,
}

struct RocketEffects {
    character: acquisition_character_effects::Candidate,
    native: std::collections::BTreeMap<u32, acquisition_character_effects::NativeState>,
    ground: Option<GroundRocketCandidate>,
    helo: Option<HeloRocketCandidate>,
    hospital: Option<HospitalBlastCandidate>,
}

impl<W: Write> Engine<W> {
    pub(crate) fn tick_hospital_rockets(
        &mut self,
        now: Duration,
        wall: u64,
    ) -> Result<Vec<OutboundMessage>, &'static str> {
        if self.hospital_rockets.is_none() {
            return Ok(Vec::new());
        }
        let mut flying = Vec::new();
        let mut cleanup = Vec::new();
        for actor in self.active.iter().chain(self.peers.values()) {
            for slot in actor.hospital_rocket_flights.slots.iter().flatten() {
                match slot {
                    Slot::Flying(flight) => flying.push(flight.clone()),
                    Slot::Cleanup { flight, due } if *due <= now => cleanup.push(flight.clone()),
                    Slot::Cleanup { .. } => (),
                }
            }
        }
        self.finish_rocket_cleanup(cleanup, now, wall)?;
        if flying.is_empty() {
            return Ok(Vec::new());
        }
        let now_ms = millis(now)?;
        // Rotate the bounded sweep window across the canonical player/slot order.
        flying.sort_unstable_by_key(|flight| (flight.source.player, flight.creation.slot));
        let start =
            (now_ms / world_tick::WORLD_TICK_INTERVAL.as_millis() as u64) as usize % flying.len();
        flying.rotate_left(start);
        flying.truncate(MAX_ADVANCES_PER_TICK);

        let advances = if self.round_combat_allowed(now) {
            self.plan_rocket_advances(&flying, now_ms)
        } else {
            Err("hospital_rocket_combat_ended")
        };
        let advances = match advances {
            Ok(advances) => advances,
            Err(code) => (0..flying.len()).map(|_| Err(code)).collect(),
        };
        let mut motions = Vec::new();
        let mut ends = Vec::new();
        let mut new_packets = 0;
        for (mut flight, advance) in flying.into_iter().zip(advances) {
            match advance {
                Ok(Advance::Flying(next)) if next != flight.motion => {
                    let position = wire_position(next.position());
                    if position.validate().is_err() {
                        ends.push((flight, End::Retired("hospital_rocket_wire_boundary")));
                        continue;
                    }
                    let mut packet = None;
                    if now.saturating_sub(flight.last_published) >= POSITION_INTERVAL
                        && self.rocket_publications_fit(flight.source.world, new_packets + 1)
                    {
                        packet =
                            Some(flight.encode_delta(position, None, now, self.wire_profile)?);
                        flight.last_published = now;
                        new_packets += 1;
                    }
                    flight.motion = next;
                    motions.push(MotionInstallation { flight, packet });
                }
                Ok(Advance::Terminal { terminal, .. }) => {
                    if wire_position(terminal.position).validate().is_ok() {
                        ends.push((flight, End::Detonation(terminal)));
                    } else {
                        ends.push((flight, End::Retired("hospital_rocket_wire_boundary")));
                    }
                }
                Err(code)
                    if code.ends_with("_work_limit")
                        && now_ms.saturating_sub(flight.motion.advanced_at_ms())
                            < QUERY_RETRY_MS =>
                {
                    // The shared work budget may recover next tick. Keep the
                    // last committed segment origin and allow reliable drain.
                }
                Err(code) => ends.push((flight, End::Retired(code))),
                Ok(Advance::Flying(_) | Advance::AlreadyTerminal) => (),
            }
        }

        if !motions.is_empty() {
            self.record(
                json!({"op":"hospital_rocket_motion","timestamp_ms":now_ms,
                    "positions":motions.iter().map(|entry|json!({
                        "projectile":entry.flight.source.projectile_id,
                        "position":entry.flight.motion.position(),
                        "packet":entry.packet.as_ref().map(|wire|protocol::encode_hex(wire)),
                    })).collect::<Vec<_>>()}),
                wall,
            )?;
            for motion in motions {
                let flight = motion.flight;
                if let Some(packet) = motion.packet {
                    self.queue_rocket_packet(flight.source.world, &packet);
                }
                let slot = self.rocket_slot(&flight);
                *slot = Some(Slot::Flying(flight));
            }
        }

        let mut messages = Vec::new();
        for (flight, end) in ends {
            messages.extend(self.finish_rocket(flight, end, now, wall)?);
        }
        Ok(messages)
    }

    fn plan_rocket_advances(
        &self,
        flights: &[Flight],
        now_ms: u64,
    ) -> Result<Vec<Result<Advance<OwnedContact>, &'static str>>, &'static str> {
        let state = &self
            .domination_gameplay
            .as_ref()
            .ok_or("hospital_rocket_domination_world_required")?
            .state;
        let static_world = self
            .collision_world
            .as_ref()
            .ok_or("hospital_rocket_static_world_required")?;
        let bindings = self
            .projectile_contacts
            .as_ref()
            .ok_or("hospital_rocket_contact_bindings_required")?;
        let snapshot = self.dynamic_collision_snapshot(state.filter_rows[6])?;
        let scene = Scene::new(
            static_world,
            &state.collisions,
            &snapshot,
            bindings,
            state.filter_rows,
        )
        .with_dynamic_catalog(self)?;
        let mut budget = QueryLimits::default().max_support_points;
        Ok(flights
            .iter()
            .map(|flight| {
                let associated_character = self.rocket_associated_character(flight, &snapshot);
                flight.motion.advance(now_ms, |start, end, radius| {
                    let sweep = SphereSweep {
                        start,
                        end,
                        radius,
                        associated_character,
                    };
                    scene.query(sweep, &mut budget).map(|contact| {
                        contact.map(|obstacle| Contact {
                            fraction: obstacle.fraction,
                            obstacle,
                        })
                    })
                })
            })
            .collect())
    }

    fn rocket_associated_character(
        &self,
        flight: &Flight,
        snapshot: &dynamic_collision::Snapshot,
    ) -> Option<TargetKey> {
        let source = &flight.source;
        if !self.active.iter().chain(self.peers.values()).any(|actor| {
            source.round == self.round_number
                && actor.allocation_sequence == source.allocation_sequence
                && actor.reservation.id == source.reservation
                && actor.reservation.world == source.world
                && actor.reservation.player == source.player
                && actor.reward_match_id == source.match_id
                && actor.reservation.character.character_id == source.character_id
                && actor.team() == source.team
        }) {
            return None;
        }
        // The native association follows the same Character allocation. Its
        // current body can belong to a later life within the same reservation.
        snapshot.bodies().iter().find_map(|body| {
            (body.target.kind == TargetKind::Character
                && body.target.key.id == u64::from(flight.creation.shooter_id))
            .then_some(body.target.key)
        })
    }

    fn finish_rocket(
        &mut self,
        flight: Flight,
        end: End,
        now: Duration,
        wall: u64,
    ) -> Result<Vec<OutboundMessage>, &'static str> {
        let world = flight.source.world;
        if !self.rocket_publications_fit(world, 1) {
            // Retry with the prior committed flight and keep draining the FIFO.
            return Ok(Vec::new());
        }

        let terminal = match end {
            End::Detonation(terminal) => terminal,
            End::Retired(reason) => {
                self.retire_rocket(flight, reason, now, wall)?;
                return Ok(Vec::new());
            }
        };
        let packet = flight.encode_delta(
            wire_position(terminal.position),
            Some(2),
            now,
            self.wire_profile,
        )?;
        let due = now
            .checked_add(CLEANUP_GRACE)
            .ok_or("projectile_cleanup_time_overflow")?;
        let effects = self.prepare_rocket_effects(&flight, terminal.position, now);
        let (effects, damage_denial) = match effects {
            Ok(effects) => {
                self.check_acquisition_character_effects(&effects.character, &effects.native)?;
                let ground_packets = usize::from(
                    effects
                        .ground
                        .as_ref()
                        .is_some_and(|candidate| candidate.packet.is_some()),
                );
                let helo_packets = effects
                    .helo
                    .as_ref()
                    .map_or(0, |candidate| candidate.packets.len());
                let common_packets = 1 + ground_packets + helo_packets;
                let has_hospital_packet = effects
                    .hospital
                    .as_ref()
                    .is_some_and(|effect| effect.packet.is_some());
                let capacity = if has_hospital_packet {
                    effects
                        .character
                        .check_extra_shared_for(self, world, |actor| {
                            let hospital =
                                crate::events::hospital_blast_damage_recipient(actor, world);
                            let base = actor.initial_objects.is_some();
                            (base || hospital).then_some(
                                usize::from(base) * common_packets + usize::from(hospital),
                            )
                        })
                } else {
                    effects
                        .character
                        .check_extra_shared(self, world, common_packets)
                };
                if capacity.is_err() {
                    return Ok(Vec::new());
                }
                (Some(effects), None)
            }
            Err(
                "reward_outbox_full"
                | "reward_notifications_full"
                | "hospital_rocket_publication_budget_exceeded",
            ) => {
                return Ok(Vec::new());
            }
            Err(code) => (None, Some(code)),
        };
        let terminal_reason = match &terminal.reason {
            TerminalReason::Contact(contact) => {
                json!({"kind":"contact","obstacle":contact_identity(&contact.obstacle),
                    "fraction":contact.fraction,"center":contact.center,"normal":contact.normal})
            }
            TerminalReason::AgeExpired => json!({"kind":"aged"}),
        };
        let mut audit = json!({"op":"hospital_rocket_terminal","source":flight.source,
                "coverage":"represented_world_reference_pose","terminal":terminal_reason,
                "elapsed_ms":terminal.elapsed_ms,"position":terminal.position,
                "effects":effects.as_ref().map(|effects|effects.character.audit()),
                "ground_effects":effects.as_ref().and_then(|effects|effects.ground.as_ref()).map(|candidate|&candidate.audit),
                "helo_effects":effects.as_ref().and_then(|effects|effects.helo.as_ref()).map(|candidate|&candidate.audit),
                "messages":effects.as_ref().map(|effects|effects.character.messages()),
                "damage_denial":damage_denial,"packet":protocol::encode_hex(&packet)});
        if let Some(hospital) = effects
            .as_ref()
            .and_then(|effects| effects.hospital.as_ref())
        {
            audit["hospital_effects"] = json!(hospital.audit);
        }
        self.record(audit, wall)?;
        let messages = effects.map_or_else(Vec::new, |effects| {
            let RocketEffects {
                character,
                mut native,
                ground,
                helo,
                hospital,
            } = effects;
            let messages = self.install_character_effects_after_record(character, &mut native);
            if let Some(candidate) = ground {
                if let Some(after) = candidate.after {
                    self.ground_runtime = Some(after);
                }
                if let Some(packet) = candidate.packet {
                    self.queue_rocket_packet(world, &packet);
                }
            }
            if let Some(candidate) = helo {
                if let Some(after) = candidate.after {
                    self.domination_gameplay
                        .as_mut()
                        .expect("prepared rocket Helo world")
                        .state
                        .helo = Some(after);
                }
                for packet in candidate.packets {
                    self.queue_rocket_helo_packet(world, &packet);
                }
            }
            if let Some(candidate) = hospital
                && let Some(packet) = candidate.packet
            {
                candidate.patch.install(
                    self.bunker_world
                        .as_mut()
                        .expect("prepared rocket Hospital world"),
                );
                for actor in self
                    .active
                    .iter_mut()
                    .chain(self.peers.values_mut())
                    .filter(|actor| crate::events::hospital_blast_damage_recipient(actor, world))
                {
                    actor.queue_shared(packet.clone());
                }
            }
            messages
        });
        self.queue_rocket_packet(world, &packet);
        let slot = self.rocket_slot(&flight);
        *slot = Some(Slot::Cleanup { flight, due });
        Ok(messages)
    }

    fn prepare_rocket_effects(
        &mut self,
        flight: &Flight,
        position: [f64; 3],
        now: Duration,
    ) -> Result<RocketEffects, &'static str> {
        let state = &self
            .domination_gameplay
            .as_ref()
            .ok_or("hospital_rocket_domination_world_required")?
            .state;
        let helo_rules = state.hospital_rocket_helo_areas.clone();
        let hospital_blast = state.hospital_blast_areas.is_some();
        let filter_rows = state.filter_rows;
        let sequences = state.collisions.clone();
        let costs = state.structural.material_penetration_costs().clone();
        let terrain = std::sync::Arc::clone(&state.terrain_materials);
        let collision = BlastCollision {
            filter_rows: &filter_rows,
            limits: QueryLimits::default(),
            sequences: &sequences,
            material_costs: &costs,
            terrain_materials: Some(&terrain),
        };
        let (character, native) = self.prepare_hospital_rocket_character_blast(
            &flight.source,
            position,
            flight.blast,
            collision,
            now,
        )?;
        let ground = self
            .hospital_rocket_ground
            .as_ref()
            .map(|rules| {
                self.prepare_hospital_rocket_ground_blast(
                    &flight.source,
                    position,
                    GroundRocketDose::from(flight.blast),
                    rules,
                    self.projectile_contacts
                        .as_ref()
                        .ok_or("hospital_rocket_contact_bindings_required")?,
                    collision,
                    now,
                )
            })
            .transpose()?;
        let helo = helo_rules
            .as_deref()
            .map(|rules| {
                self.prepare_hospital_rocket_helo_blast(
                    &flight.source,
                    position,
                    HeloRocketDose::from(flight.blast),
                    rules,
                    collision,
                    now,
                )
            })
            .transpose()?;
        let hospital = hospital_blast
            .then(|| {
                self.prepare_hospital_rocket_hospital_blast(
                    &flight.source,
                    position,
                    flight.blast,
                    collision,
                    now,
                )
            })
            .transpose()?;
        Ok(RocketEffects {
            character,
            native,
            ground,
            helo,
            hospital,
        })
    }

    fn finish_rocket_cleanup(
        &mut self,
        cleanup: Vec<Flight>,
        now: Duration,
        wall: u64,
    ) -> Result<(), &'static str> {
        if cleanup.is_empty() {
            return Ok(());
        }
        let pending = self.queued_projectile_terminals();
        let held = now
            .checked_add(CLEANUP_GRACE)
            .ok_or("projectile_cleanup_time_overflow")?;
        for flight in cleanup {
            if pending.contains(&(flight.creation.slot, flight.source.player)) {
                if let Some(Slot::Cleanup { due, .. }) = self.rocket_slot(&flight) {
                    *due = held;
                }
            } else if self.rocket_publications_fit(flight.source.world, 1) {
                self.retire_rocket(flight, "terminal_cleanup", now, wall)?;
            }
        }
        Ok(())
    }

    fn retire_rocket(
        &mut self,
        flight: Flight,
        reason: &'static str,
        now: Duration,
        wall: u64,
    ) -> Result<(), &'static str> {
        let packet =
            flight
                .removal()
                .encode_with_profile(1, now.as_millis() as u32, self.wire_profile)?;
        self.record(
            json!({"op":"hospital_rocket_retired","source":flight.source,
            "reason":reason,"packet":protocol::encode_hex(&packet)}),
            wall,
        )?;
        self.queue_rocket_packet(flight.source.world, &packet);
        *self.rocket_slot(&flight) = None;
        Ok(())
    }

    fn rocket_slot(&mut self, flight: &Flight) -> &mut Option<Slot> {
        let id = &flight.source.reservation;
        let actor = self
            .active
            .as_mut()
            .filter(|actor| actor.reservation.id == *id)
            .or_else(|| self.peers.get_mut(id))
            .expect("prepared projectile reservation");
        &mut actor.hospital_rocket_flights.slots[usize::from(flight.creation.slot)]
    }

    fn rocket_publications_fit(&self, world: u32, count: usize) -> bool {
        self.active
            .iter()
            .chain(self.peers.values())
            .filter(|actor| actor.reservation.world == world && actor.initial_objects.is_some())
            .all(|actor| {
                !actor.replication_backlogged
                    && actor.ordinary_shared_len() + count <= multiplayer::MAX_SHARED_QUEUE
            })
    }

    fn queue_rocket_helo_packet(&mut self, world: u32, packet: &[u8]) {
        for actor in self
            .active
            .iter_mut()
            .chain(self.peers.values_mut())
            .filter(|actor| {
                actor.reservation.world == world
                    && actor.initial_objects.is_some()
                    && actor.helo_creation_sent
            })
        {
            actor.queue_shared(packet.to_vec());
        }
    }

    fn queue_rocket_packet(&mut self, world: u32, packet: &[u8]) {
        for actor in self
            .active
            .iter_mut()
            .chain(self.peers.values_mut())
            .filter(|actor| actor.reservation.world == world && actor.initial_objects.is_some())
        {
            actor.queue_shared(packet.to_vec());
        }
    }
}

impl Flight {
    fn encode_delta(
        &self,
        position: CharacterPosition,
        state: Option<u8>,
        now: Duration,
        profile: wire_profile::WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        ProjectileOwnerDelta {
            header: projectile_header(self.creation.slot, 1, now.as_millis() as u32),
            update: &ProjectileOwnerUpdate {
                slot: self.creation.slot,
                object_index: self.source.player,
                fields: ProjectileOwnerFields {
                    state,
                    position: Some(position),
                    rotation_xyzw: None,
                },
            },
        }
        .encode_with_profile(profile)
    }
}

fn contact_identity(obstacle: &OwnedObstacleId) -> serde_json::Value {
    match obstacle {
        OwnedObstacleId::Static { instance, triangle } => {
            json!({"kind":"static","instance":instance,"triangle":triangle})
        }
        OwnedObstacleId::Sequence {
            placement,
            instance,
            triangle,
        } => {
            json!({"kind":"sequence","placement":placement,"instance":instance,"triangle":triangle})
        }
        OwnedObstacleId::Dynamic { key, root } => {
            json!({"kind":"dynamic","id":key.id,"generation":key.generation,"root":root})
        }
    }
}

fn projectile_header(slot: u8, sequence: u32, timestamp_ms: u32) -> UpdateHeader {
    UpdateHeader {
        sequence,
        nak_sequence: 0,
        client_index: 0,
        object_type_mask: 1 << (16 + slot),
        timestamp_ms,
    }
}

fn after_seconds(now: Duration, seconds: f32) -> Result<Duration, &'static str> {
    now.checked_add(
        Duration::try_from_secs_f64(seconds.into())
            .map_err(|_| "invalid_hospital_rocket_duration")?,
    )
    .ok_or("hospital_rocket_time_overflow")
}

fn authored_millis(seconds: f32) -> Result<u64, &'static str> {
    millis(after_seconds(Duration::ZERO, seconds)?)
}

fn millis(now: Duration) -> Result<u64, &'static str> {
    u64::try_from(now.as_millis()).map_err(|_| "hospital_rocket_time_overflow")
}

fn wire_position([x, y, z]: [f64; 3]) -> CharacterPosition {
    CharacterPosition {
        x: x as f32,
        y: y as f32,
        z: z as f32,
    }
}

fn muzzle_pose(
    body: vehicle_exit::BunkerTransform,
    muzzle: Muzzle,
    yaw: f32,
    pitch: f32,
) -> Result<([f64; 3], [f64; 4]), &'static str> {
    // Preserve the original parent order. Local +Y yaw and +X pitch are the
    // replacement control basis. The transmitted direction is checked separately.
    let mut position = [
        body.position.x.into(),
        body.position.y.into(),
        body.position.z.into(),
    ];
    let mut rotation = unit(body.rotation_xyzw.map(f64::from))?;
    let (yaw_sin, yaw_cos) = (f64::from(yaw) * 0.5).sin_cos();
    let (pitch_sin, pitch_cos) = (f64::from(pitch) * 0.5).sin_cos();
    let identity = [0., 0., 0., 1.];
    let stages: [(MuzzleNode, [f64; 4]); 4] = [
        (muzzle.root, identity),
        (muzzle.yaw, [0., yaw_sin, 0., yaw_cos]),
        (muzzle.pitch, [pitch_sin, 0., 0., pitch_cos]),
        (muzzle.firepoint, identity),
    ];
    for (node, control) in stages {
        let offset = rotate(rotation, node.position_xyz);
        position = std::array::from_fn(|axis| position[axis] + offset[axis]);
        rotation = unit(multiply(multiply(rotation, node.rotation_xyzw), control))?;
    }
    Ok((position, rotation))
}

fn unit(quaternion: [f64; 4]) -> Result<[f64; 4], &'static str> {
    let norm = quaternion.into_iter().fold(0.0_f64, f64::hypot);
    if !norm.is_finite() || norm < 1e-6 {
        return Err("invalid_hospital_rocket_rotation");
    }
    Ok(quaternion.map(|component| component / norm))
}

fn multiply([x, y, z, w]: [f64; 4], [a, b, c, d]: [f64; 4]) -> [f64; 4] {
    [
        w * a + x * d + y * c - z * b,
        w * b - x * c + y * d + z * a,
        w * c + x * b - y * a + z * d,
        w * d - x * a - y * b - z * c,
    ]
}

fn rotate(rotation: [f64; 4], [x, y, z]: [f64; 3]) -> [f64; 3] {
    let [a, b, c, d] = rotation;
    let vector = multiply(multiply(rotation, [x, y, z, 0.]), [-a, -b, -c, d]);
    [vector[0], vector[1], vector[2]]
}

#[cfg(test)]
#[path = "hospital_rocket_runtime_tests.rs"]
pub(crate) mod tests;
