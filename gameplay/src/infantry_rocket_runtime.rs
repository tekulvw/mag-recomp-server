//! Owner-bound launcher budgets and server-owned infantry rocket flights.
use crate::{
    hospital_rocket_runtime::LaunchSource,
    infantry_rockets::{self, Catalog, RoundBudget, Variant as RocketVariant},
    projectile::ProjectileCreation,
    projectile_motion::{Limits, Motion},
    projectile_replication::ProjectileRemoval,
    replication::{CharacterPosition, UpdateHeader},
    *,
};

use std::{collections::BTreeMap, sync::Arc};

const CLEANUP_GRACE: Duration = Duration::from_millis(500);
const REQUEST_BYTES: usize = 57;

#[derive(Clone, Debug, Default)]
pub(crate) struct State {
    slots: [Option<Box<Slot>>; projectile::MAX_PROJECTILE_SLOTS],
    budget: Option<LifeBudget>,
    guided_budget: Option<LifeBudget>,
    underbarrel_budgets: [Option<LifeBudget>; underbarrel_launchers::MAX_VARIANTS_PER_FACTION],
}

#[derive(Clone, Copy, Debug)]
struct LifeBudget {
    life: u32,
    item: u16,
    rounds: RoundBudget,
}

#[derive(Clone, Debug)]
struct Slot {
    flight: Flight,
    cleanup: Option<Duration>,
}

#[derive(Clone, Debug)]
struct Flight {
    source: LaunchSource,
    creation: ProjectileCreation,
    fingerprint: [u8; REQUEST_BYTES],
    variant: Variant,
    rotation: [[f64; 3]; 3],
    origin: [f64; 3],
    motion: FlightMotion,
    last_published: Duration,
    guided_target: Option<crate::guided_launcher_warnings::Target>,
}

/// Both launchers share wire slots and atomic terminal publication. Equipment,
/// ammunition budgets and propulsion remain separate.
#[derive(Clone, Copy, Debug)]
enum Variant {
    Rocket(RocketVariant),
    Guided(crate::guided_launchers::Variant),
    Underbarrel {
        weapon: underbarrel_launchers::Variant,
        ammo: underbarrel_launchers::Authored,
        token: u32,
    },
}

impl Variant {
    fn item(self) -> u16 {
        match self {
            Self::Guided(v) => Self::Rocket(v.base()).item(),
            Self::Rocket(v) => v.item_id,
            Self::Underbarrel { weapon, .. } => weapon.attachment_item_id,
        }
    }
    fn is_underbarrel(self) -> bool {
        matches!(self, Self::Underbarrel { .. })
    }
    fn weapon(self) -> infantry_rockets::Weapon {
        match self {
            Self::Guided(v) => Self::Rocket(v.base()).weapon(),
            Self::Rocket(v) => v.weapon,
            Self::Underbarrel { weapon, .. } => infantry_rockets::Weapon {
                magazine_size: weapon.magazine_capacity,
                initial_magazine_count: weapon.initial_magazine_count,
                refire_seconds: weapon.refire_seconds,
                reload_seconds: weapon.reload_seconds,
            },
        }
    }
    fn token(self) -> u32 {
        match self {
            Self::Guided(v) => Self::Rocket(v.base()).token(),
            Self::Rocket(v) => v.ammo_token,
            Self::Underbarrel { token, .. } => token,
        }
    }
    fn spine(self) -> [f64; 3] {
        match self {
            Self::Guided(v) => Self::Rocket(v.base()).spine(),
            Self::Rocket(v) => v.flight.capsule_spine_local_xyz,
            Self::Underbarrel { ammo, .. } => ammo.capsule_height,
        }
    }
    fn radius(self) -> f64 {
        match self {
            Self::Guided(v) => Self::Rocket(v.base()).radius(),
            Self::Rocket(v) => v.flight.radius_m,
            Self::Underbarrel { ammo, .. } => ammo.radius,
        }
    }
    fn speed(self) -> f32 {
        match self {
            Self::Guided(v) => Self::Rocket(v.base()).speed(),
            Self::Rocket(v) => v.flight.speed_mps,
            Self::Underbarrel { ammo, .. } => ammo.launch_velocity,
        }
    }
    fn blast(self) -> hospital_rockets::Blast {
        match self {
            Self::Guided(v) => Self::Rocket(v.base()).blast(),
            Self::Rocket(v) => hospital_rockets::Blast {
                character_hp: v.blast.character_hp,
                vehicle_hp: v.blast.vehicle_hp,
                objective_hp: 0.,
                half_radius_m: v.blast.half_radius_m,
                outer_radius_m: v.blast.outer_radius_m,
                material_penetration: v.blast.material_penetration,
            },
            Self::Underbarrel { ammo, .. } => hospital_rockets::Blast {
                character_hp: ammo.damage.character,
                vehicle_hp: ammo.damage.vehicle,
                // No logical-objective callback has been qualified for this path.
                objective_hp: 0.,
                half_radius_m: ammo.half_damage_radius,
                outer_radius_m: ammo.full_damage_radius,
                material_penetration: ammo.material_penetration,
            },
        }
    }
    fn vehicle_damage(self) -> item_catalog::TargetDamageParameters {
        match self {
            Self::Guided(v) => Self::Rocket(v.base()).vehicle_damage(),
            Self::Rocket(v) => item_catalog::TargetDamageParameters {
                amount: v.blast.vehicle_hp,
                armor_pierce: 0.,
                armor_redux: 0.,
            },
            Self::Underbarrel { ammo, .. } => item_catalog::TargetDamageParameters {
                amount: ammo.damage.vehicle,
                armor_pierce: ammo.damage.vehicle_apierce,
                armor_redux: ammo.damage.vehicle_aredux,
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum FlightMotion {
    Rocket(Motion),
    Guided(crate::guided_launcher_motion::Motion),
    Ballistic(ballistic_projectile_motion::Motion<projectile_scene::OwnedObstacleId>),
}
impl FlightMotion {
    fn position(&self) -> [f64; 3] {
        match self {
            Self::Rocket(m) => m.position(),
            Self::Guided(m) => m.position(),
            Self::Ballistic(m) => m.position(),
        }
    }
    fn advanced_at_ms(&self) -> u64 {
        match self {
            Self::Rocket(m) => m.advanced_at_ms(),
            Self::Guided(m) => m.advanced_at_ms(),
            Self::Ballistic(m) => m.advanced_at_ms(),
        }
    }
}

pub(crate) enum LaunchDisposition {
    Created(Box<Launch>),
    Duplicate,
}

pub(crate) struct Launch {
    pub(crate) state: State,
    pub(crate) packet: Vec<u8>,
    pub(crate) audit: Value,
}

fn equipped<'a>(actor: &Active, catalog: &'a Catalog) -> Option<&'a RocketVariant> {
    let kit = actor.equipped_loadout()?;
    let inventory = kit
        .inventories
        .get(usize::from(kit.handle.inventory_index))?;
    let item = inventory.slots[usize::from(infantry_rockets::GEAR_SLOT)]?;
    catalog.variant(actor.reservation.character.faction, item)
}

fn equipped_underbarrel<'a>(
    actor: &Active,
    catalog: &'a underbarrel_launchers::Catalog,
) -> Option<&'a underbarrel_launchers::Variant> {
    let kit = actor.equipped_loadout()?;
    let inventory = kit
        .inventories
        .get(usize::from(kit.handle.inventory_index))?;
    catalog.variant(
        actor.reservation.character.faction,
        inventory.slots[0]?,
        inventory.slots[17]?,
    )
}

impl State {
    pub(crate) fn guided_targets<'a>(
        &'a self,
        actor: &'a Active,
        round: u64,
    ) -> impl Iterator<Item = &'a crate::guided_launcher_warnings::Target> {
        self.slots
            .iter()
            .flatten()
            .filter(move |s| {
                s.cleanup.is_none()
                    && s.flight.source.allocation_sequence == actor.allocation_sequence
                    && s.flight.source.reservation == actor.reservation.id
                    && s.flight.source.world == actor.reservation.world
                    && s.flight.source.player == actor.reservation.player
                    && s.flight.source.round == round
                    && s.flight.source.match_id == actor.reward_match_id
                    && s.flight.source.character_id == actor.reservation.character.character_id
                    && s.flight.source.team == actor.team()
            })
            .filter_map(|s| s.flight.guided_target.as_ref())
    }
    pub(crate) fn can_resupply_guided(
        actor: &Active,
        catalog: &crate::guided_launchers::Catalog,
    ) -> bool {
        crate::guided_launchers::owns_skill(actor)
            && crate::guided_launchers::equipped(actor, catalog).is_some()
    }
    pub(crate) fn refill_guided_for(
        actor: &Active,
        catalog: &crate::guided_launchers::Catalog,
        staged: Option<Self>,
    ) -> Result<Option<Self>, &'static str> {
        let CharacterLife::Alive { generation, .. } = actor.life else {
            return Ok(staged);
        };
        let Some(v) = crate::guided_launchers::equipped(actor, catalog) else {
            return Ok(staged);
        };
        let current = staged.as_ref().unwrap_or(&actor.infantry_rocket_state);
        let Some(budget) = current
            .guided_budget
            .filter(|b| b.life == generation && b.item == v.item_id)
        else {
            return Ok(staged);
        };
        let rounds = budget.rounds.refilled();
        if rounds == budget.rounds {
            return Ok(staged);
        }
        let mut next = staged.unwrap_or_else(|| actor.infantry_rocket_state.clone());
        next.guided_budget = Some(LifeBudget { rounds, ..budget });
        Ok(Some(next))
    }

    pub(crate) fn occupies(&self, slot: u8) -> bool {
        self.slots
            .get(usize::from(slot))
            .is_some_and(Option::is_some)
    }

    pub(crate) fn snapshot_packets(
        &self,
        timestamp: u32,
        profile: wire_profile::WireProfile,
    ) -> Result<Vec<Vec<u8>>, &'static str> {
        self.slots
            .iter()
            .flatten()
            .filter(|slot| slot.cleanup.is_none())
            .map(|slot| {
                let mut creation = slot.flight.creation;
                creation.position = wire_position(slot.flight.motion.position());
                if let FlightMotion::Guided(motion) = &slot.flight.motion {
                    creation.rotation_xyzw = heading_quaternion(motion.heading());
                }
                creation.encode_with_profile(header(creation.slot, 1, timestamp), profile)
            })
            .collect()
    }

    pub(crate) fn removals(&self) -> impl Iterator<Item = ProjectileRemoval> + '_ {
        self.slots
            .iter()
            .flatten()
            .map(|slot| slot.flight.removal())
    }

    pub(crate) fn can_resupply(actor: &Active, catalog: &Catalog) -> bool {
        equipped(actor, catalog).is_some()
    }

    pub(crate) fn can_resupply_underbarrel(
        actor: &Active,
        catalog: &underbarrel_launchers::Catalog,
    ) -> bool {
        equipped_underbarrel(actor, catalog).is_some()
    }

    fn underbarrel_budget_index(&self, life: u32, item: u16) -> Option<usize> {
        self.underbarrel_budgets.iter().position(|entry| {
            entry.is_some_and(|budget| budget.life == life && budget.item == item)
        })
    }

    /// Merge into an already staged rocket refill rather than overwriting it.
    pub(crate) fn refill_underbarrel_for(
        actor: &Active,
        catalog: &underbarrel_launchers::Catalog,
        staged: Option<Self>,
    ) -> Result<Option<Self>, &'static str> {
        let CharacterLife::Alive { generation, .. } = actor.life else {
            return Ok(staged);
        };
        let Some(variant) = equipped_underbarrel(actor, catalog) else {
            return Ok(staged);
        };
        let current = staged.as_ref().unwrap_or(&actor.infantry_rocket_state);
        let Some(index) = current.underbarrel_budget_index(generation, variant.attachment_item_id)
        else {
            return Ok(staged);
        };
        let budget = current.underbarrel_budgets[index].expect("matched launcher budget");
        let rounds = budget.rounds.refilled();
        if rounds == budget.rounds {
            return Ok(staged);
        }
        let mut next = staged.unwrap_or_else(|| actor.infantry_rocket_state.clone());
        next.underbarrel_budgets[index] = Some(LifeBudget { rounds, ..budget });
        Ok(Some(next))
    }

    pub(crate) fn refill_for(
        actor: &Active,
        catalog: &Catalog,
    ) -> Result<Option<Self>, &'static str> {
        let CharacterLife::Alive { generation, .. } = actor.life else {
            return Ok(None);
        };
        let Some(variant) = equipped(actor, catalog) else {
            return Ok(None);
        };
        let Some(budget) = actor
            .infantry_rocket_state
            .budget
            .filter(|budget| budget.life == generation && budget.item == variant.item_id)
        else {
            return Ok(None);
        };
        let rounds = budget.rounds.refilled();
        if rounds == budget.rounds {
            return Ok(None);
        }
        let mut next = actor.infantry_rocket_state.clone();
        next.budget = Some(LifeBudget { rounds, ..budget });
        Ok(Some(next))
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
    pub(crate) fn plan_infantry_rocket_launch(
        &self,
        actor: &Active,
        request: protocol::FireGrenade,
        fingerprint: &[u8],
        now: Duration,
        next_sequence: u32,
        modifiers: &skill_catalog::OwnedModifiers,
    ) -> Result<LaunchDisposition, &'static str> {
        let variant = match request.ammo_index {
            infantry_rockets::GEAR_SLOT => {
                if let Some(v) = self
                    .guided_launchers
                    .as_deref()
                    .and_then(|c| crate::guided_launchers::equipped(actor, c))
                {
                    if !crate::guided_launchers::owns_skill(actor) {
                        return Err("guided_launcher_requires_owned_skill25");
                    }
                    Variant::Guided(*v)
                } else {
                    Variant::Rocket(
                        *equipped(
                            actor,
                            self.infantry_rockets
                                .as_deref()
                                .ok_or("infantry_rocket_rules_required")?,
                        )
                        .ok_or("infantry_rocket_weapon_unqualified")?,
                    )
                }
            }
            17 => {
                let catalog = self
                    .underbarrel_launchers
                    .as_deref()
                    .ok_or("underbarrel_launcher_rules_required")?;
                Variant::Underbarrel {
                    weapon: *equipped_underbarrel(actor, catalog)
                        .ok_or("underbarrel_launcher_weapon_unqualified")?,
                    ammo: catalog.ammo().authored,
                    token: catalog.ammo().token,
                }
            }
            _ => return Err("infantry_rocket_weapon_unqualified"),
        };
        if actor.wire_profile.application_id() != 20231
            || !matches!(self.match_profile().game_type, 1 | 2)
            || !self.match_started
            || self.match_terminal.is_some()
            || !self.round_combat_allowed(now)
        {
            return Err("infantry_rocket_combat_inactive");
        }
        let player = actor.reservation.player;
        let shooter_id = 0x0380_0000 | u32::from(player);
        if request.shooter_id != shooter_id || request.shooter_client_index != player {
            return Err("unowned_projectile_shooter");
        }
        let CharacterLife::Alive {
            generation,
            health_quarter_points: 1..,
        } = actor.life
        else {
            return Err("infantry_rocket_requires_living_owner");
        };
        if actor.is_mounted()
            || !actor.entered_game
            || !actor.ready_for_updates
            || actor.initial_objects.is_none()
        {
            return Err("infantry_rocket_requires_living_on_foot_owner");
        }
        let slot = u8::try_from(request.projectile_index)
            .ok()
            .filter(|slot| *slot < projectile::projectile_slot_count(self.wire_profile))
            .ok_or("invalid_infantry_rocket_slot")?;
        let fingerprint: [u8; REQUEST_BYTES] = fingerprint
            .try_into()
            .map_err(|_| "invalid_infantry_rocket_launch_bytes")?;
        let state = &actor.infantry_rocket_state;
        if let Some(entry) = &state.slots[usize::from(slot)] {
            let previous = &entry.flight;
            if previous.fingerprint == fingerprint
                && previous.source.allocation_sequence == actor.allocation_sequence
                && previous.source.life_generation == generation
                && previous.source.item == variant.item()
                && previous.source.round == self.round_number
            {
                return Ok(LaunchDisposition::Duplicate);
            }
            return Err("projectile_slot_in_use");
        }
        if actor
            .projectiles
            .get(slot)
            .is_some_and(|frag| !frag.completed)
            || actor.hospital_rocket_flights.occupies(slot)
            || self.projectile_cleanup_pending(actor, slot)
        {
            return Err("projectile_slot_in_use");
        }

        let rotation = rigid_rotation(request.rotation_xyzw)?;
        let origin = request.position.map(f64::from);
        let position = wire_position(origin);
        position.validate()?;
        let power = request.throw_power;
        // The native writer clamps before packing, despite the wider decoder.
        if !(0.0..=1.0).contains(&power) {
            return Err("invalid_infantry_rocket_power");
        }
        let speed =
            power.mul_add(modifiers.value("kCharModGrenadeThrowPower"), power) * variant.speed();
        let velocity = std::array::from_fn(|axis| {
            // The authored velocity lies along the request quaternion's +Z.
            rotation[axis][2] * f64::from(speed) + f64::from(request.initial_velocity[axis])
        });
        let maximum_speed = velocity.into_iter().fold(0.0_f64, f64::hypot) * (1.0 + 1e-12);
        let target = if matches!(variant, Variant::Guided(_)) && request.target_id != u32::MAX {
            Some(self.guided_target(actor.reservation.world, request.target_id)?)
        } else {
            None
        };
        let motion = match variant {
            Variant::Guided(v) => {
                let anchor = target
                    .as_ref()
                    .map(|t| {
                        self.guided_anchor(t)
                            .ok_or("guided_target_anchor_unavailable")
                    })
                    .transpose()?;
                let range = if target.is_some() {
                    self.guided_launchers
                        .as_ref()
                        .ok_or("guided_launcher_rules_required")?
                        .guidance
                        .guided_max_range_m
                } else {
                    v.steering.dumb_max_range_m
                };
                let seed = (actor.allocation_sequence as u32).wrapping_mul(0x9e37_79b9)
                    ^ (self.round_number as u32).rotate_left(11)
                    ^ (now.as_millis() as u32).rotate_left(17)
                    ^ u32::from(player)
                    ^ u32::from(slot);
                FlightMotion::Guided(crate::guided_launcher_motion::Motion::launch(
                    origin,
                    velocity,
                    [rotation[0][2], rotation[1][2], rotation[2][2]],
                    millis(now)?,
                    v.steering,
                    v.flight.speed_mps,
                    range,
                    authored_millis(v.flight.lifetime_seconds)?,
                    authored_millis(v.flight.armed_time_seconds)?,
                    anchor,
                    seed,
                )?)
            }
            Variant::Rocket(v) => FlightMotion::Rocket(Motion::launch(
                origin,
                velocity,
                millis(now)?,
                Limits {
                    radius: v.flight.radius_m,
                    lifetime_ms: authored_millis(v.flight.lifetime_seconds)?,
                    grace_ms: authored_millis(v.flight.age_grace_seconds)?,
                    maximum_speed_mps: maximum_speed,
                    maximum_duration_ms: 70_000,
                },
            )?),
            Variant::Underbarrel { ammo, .. } => {
                FlightMotion::Ballistic(ballistic_projectile_motion::Motion::launch(
                    origin,
                    velocity,
                    millis(now)?,
                    ballistic_projectile_motion::Limits {
                        armed_ms: authored_millis(ammo.armed_time)?,
                        lifetime_ms: authored_millis(ammo.lifetime)?,
                        grace_ms: 2_000,
                        linear_damping: f64::from(ammo.linear_damping),
                        restitution: f64::from(ammo.restitution),
                        maximum_speed_mps: maximum_speed.max(1.),
                        maximum_duration_ms: 70_000,
                    },
                )?)
            }
        };
        let underbarrel_index = if variant.is_underbarrel() {
            Some(
                state
                    .underbarrel_budget_index(generation, variant.item())
                    .or_else(|| {
                        state
                            .underbarrel_budgets
                            .iter()
                            .position(|entry| entry.is_none_or(|budget| budget.life != generation))
                    })
                    .ok_or("underbarrel_launcher_budget_capacity")?,
            )
        } else {
            None
        };
        let current_budget = match underbarrel_index {
            Some(index) => state.underbarrel_budgets[index],
            None if matches!(variant, Variant::Guided(_)) => state.guided_budget,
            None => state.budget,
        };
        let rounds = match current_budget {
            Some(budget) if budget.life == generation && budget.item == variant.item() => {
                budget.rounds
            }
            _ => RoundBudget::new(variant.weapon())?,
        }
        .after_shot(now)?;
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
            item: variant.item(),
            projectile_id: ((16 + u32::from(slot)) << 24) | 0x0080_0000 | u32::from(player),
        };
        let creation = ProjectileCreation {
            slot,
            object_index: player,
            owner_index: player,
            ammo_token: variant.token(),
            position,
            rotation_xyzw: request.rotation_xyzw,
            shooter_id,
        };
        let packet = creation.encode_with_profile(
            header(slot, next_sequence, now.as_millis() as u32),
            self.wire_profile,
        )?;
        let audit = json!({"type":"infantry_rocket_launch","source":source,"allocation_sequence":actor.allocation_sequence,"slot":slot,
            "origin":origin,"velocity":velocity,"gear":request.ammo_index,
            "motion_authority":if variant.is_underbarrel() { "server_bounded_ballistic" } else if matches!(variant,Variant::Guided(_)){"server_guided_or_unlocked_dumb"}else { "server_straight" },
            "launch_target":request.target_id,
            "rounds_remaining":rounds.remaining()});
        let mut state = state.clone();
        let budget = Some(LifeBudget {
            life: generation,
            item: variant.item(),
            rounds,
        });
        if let Some(index) = underbarrel_index {
            state.underbarrel_budgets[index] = budget;
        } else if matches!(variant, Variant::Guided(_)) {
            state.guided_budget = budget;
        } else {
            state.budget = budget;
        }
        state.slots[usize::from(slot)] = Some(Box::new(Slot {
            flight: Flight {
                source,
                creation,
                fingerprint,
                variant,
                rotation,
                origin,
                motion,
                last_published: now,
                guided_target: target,
            },
            cleanup: None,
        }));
        Ok(LaunchDisposition::Created(Box::new(Launch {
            state,
            packet,
            audit,
        })))
    }
}

fn rigid_rotation(rotation: [f32; 4]) -> Result<[[f64; 3]; 3], &'static str> {
    let q = rotation.map(f64::from);
    let norm = q.into_iter().fold(0.0_f64, f64::hypot);
    if !norm.is_finite() || (norm - 1.0).abs() > 1e-5 {
        return Err("invalid_infantry_rocket_rotation");
    }
    let [x, y, z, w] = q.map(|value| value / norm);
    Ok([
        [
            1. - 2. * (y * y + z * z),
            2. * (x * y - z * w),
            2. * (x * z + y * w),
        ],
        [
            2. * (x * y + z * w),
            1. - 2. * (x * x + z * z),
            2. * (y * z - x * w),
        ],
        [
            2. * (x * z - y * w),
            2. * (y * z + x * w),
            1. - 2. * (x * x + y * y),
        ],
    ])
}

fn wire_position([x, y, z]: [f64; 3]) -> CharacterPosition {
    CharacterPosition {
        x: x as f32,
        y: y as f32,
        z: z as f32,
    }
}
fn header(slot: u8, sequence: u32, timestamp_ms: u32) -> UpdateHeader {
    UpdateHeader {
        sequence,
        nak_sequence: 0,
        client_index: 0,
        object_type_mask: 1 << (16 + slot),
        timestamp_ms,
    }
}
fn millis(now: Duration) -> Result<u64, &'static str> {
    u64::try_from(now.as_millis()).map_err(|_| "infantry_rocket_time_overflow")
}
fn authored_millis(seconds: f32) -> Result<u64, &'static str> {
    millis(
        Duration::try_from_secs_f64(seconds.into())
            .map_err(|_| "invalid_infantry_rocket_duration")?,
    )
}

const SWEEP_WINDOW: usize = 16;
const QUERY_RETRY_MS: u64 = 2_000;
const POSITION_INTERVAL: Duration = Duration::from_millis(200);

use crate::{
    acquisition_volume_damage::{TargetKey, TargetKind},
    domination_structural_world::{
        BlastCollision, GroundRocketCandidate, GroundRocketDose, HeloRocketCandidate,
        HeloRocketDose, HospitalBlastCandidate,
    },
    dynamic_collision::QueryLimits,
    projectile_contact::CapsuleSweep,
    projectile_motion::{Advance, Contact, TerminalReason},
    projectile_replication::ProjectileOwnerDelta,
    projectile_scene::{OwnedContact, Scene},
};

enum Step {
    Move(FlightMotion),
    Explode {
        position: [f64; 3],
        reason: &'static str,
        contact: Option<OwnedContact>,
    },
    Retire(&'static str),
}

struct Effects {
    character: acquisition_character_effects::Candidate,
    native: BTreeMap<u32, acquisition_character_effects::NativeState>,
    ground: Option<GroundRocketCandidate>,
    helo: Option<HeloRocketCandidate>,
    hospital: Option<HospitalBlastCandidate>,
}

impl Flight {
    fn delta(
        &self,
        position: [f64; 3],
        state: Option<u8>,
        now: Duration,
        profile: wire_profile::WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        ProjectileOwnerDelta {
            header: header(self.creation.slot, 1, now.as_millis() as u32),
            update: &projectile::ProjectileOwnerUpdate {
                slot: self.creation.slot,
                object_index: self.source.player,
                fields: projectile::ProjectileOwnerFields {
                    state,
                    position: Some(wire_position(position)),
                    rotation_xyzw: match &self.motion {
                        FlightMotion::Guided(m) => Some(heading_quaternion(m.heading())),
                        _ => None,
                    },
                },
            },
        }
        .encode_with_profile(profile)
    }

    fn step(
        &self,
        now_ms: u64,
        associated_character: Option<TargetKey>,
        target_anchor: Option<[f64; 3]>,
        mut query: impl FnMut(CapsuleSweep) -> Result<Option<OwnedContact>, &'static str>,
    ) -> Result<Step, &'static str> {
        let spine = self.variant.spine();
        if let FlightMotion::Guided(motion) = &self.motion {
            use crate::guided_launcher_motion::{Advance as GuidedAdvance, Terminal as Reason};
            return Ok(
                match motion.advance(
                    now_ms,
                    || target_anchor,
                    |start, end, heading| {
                        query(CapsuleSweep {
                            start,
                            end,
                            radius: self.variant.radius(),
                            associated_character,
                            spine: [spine.map(|v| -0.5 * v), spine.map(|v| 0.5 * v)],
                            rotation: rigid_rotation(heading_quaternion(heading))?,
                        })
                        .map(|hit| {
                            hit.map(|hit| Contact {
                                fraction: hit.fraction,
                                obstacle: hit,
                            })
                        })
                    },
                )? {
                    GuidedAdvance::Flying(next) => Step::Move(FlightMotion::Guided(next)),
                    GuidedAdvance::Terminal { position, reason } => match reason {
                        Reason::Prearm => Step::Retire("guided_rocket_prearm_contact_unqualified"),
                        Reason::Contact(contact) => Step::Explode {
                            position: contact.center,
                            reason: "contact",
                            contact: Some(contact),
                        },
                        Reason::Age | Reason::Range => Step::Explode {
                            position,
                            reason: if matches!(reason, Reason::Age) {
                                "age"
                            } else {
                                "range"
                            },
                            contact: None,
                        },
                    },
                },
            );
        }
        if let FlightMotion::Ballistic(motion) = &self.motion {
            let advance = motion.advance(now_ms, |start, end| {
                query(CapsuleSweep {
                    start,
                    end,
                    radius: self.variant.radius(),
                    associated_character,
                    spine: [spine.map(|axis| -0.5 * axis), spine.map(|axis| 0.5 * axis)],
                    rotation: self.rotation,
                })
                .map(|hit| {
                    hit.map(|obstacle| ballistic_projectile_motion::Contact {
                        fraction: obstacle.fraction,
                        normal: obstacle.normal,
                        key: obstacle.obstacle.clone(),
                        obstacle,
                    })
                })
            })?;
            use ballistic_projectile_motion::{
                Advance as BallisticAdvance, TerminalReason as Reason,
            };
            return Ok(match advance {
                BallisticAdvance::Flying(next) => Step::Move(FlightMotion::Ballistic(next)),
                BallisticAdvance::Terminal { next, terminal } => {
                    debug_assert!(terminal.elapsed_ms <= next.advanced_at_ms() as f64);
                    Step::Explode {
                        position: terminal.position,
                        reason: match terminal.reason {
                            Reason::Contact(_) => "contact",
                            Reason::AgeExpired => "age",
                        },
                        contact: match terminal.reason {
                            Reason::Contact(hit) => Some(hit),
                            Reason::AgeExpired => None,
                        },
                    }
                }
                BallisticAdvance::AlreadyTerminal => return Err("unexpected_ballistic_terminal"),
            });
        }
        let (FlightMotion::Rocket(motion), Variant::Rocket(variant)) = (&self.motion, self.variant)
        else {
            return Err("launcher_motion_kind_mismatch");
        };
        let advance = motion.advance(now_ms, |start, end, radius| {
            query(CapsuleSweep {
                start,
                end,
                radius,
                associated_character,
                spine: [spine.map(|axis| -0.5 * axis), spine.map(|axis| 0.5 * axis)],
                rotation: self.rotation,
            })
            .map(|hit| {
                hit.map(|obstacle| Contact {
                    fraction: obstacle.fraction,
                    obstacle,
                })
            })
        })?;
        let at_sample = |position: [f64; 3]| {
            let displacement = position
                .into_iter()
                .zip(self.origin)
                .fold(0.0_f64, |norm, (value, origin)| norm.hypot(value - origin));
            displacement > f64::from(variant.flight.auto_explode_distance_m)
        };
        Ok(match advance {
            Advance::Terminal { terminal, .. } => match terminal.reason {
                TerminalReason::Contact(contact) => {
                    if ((terminal.elapsed_ms / 1000.0) as f32) < variant.flight.armed_time_seconds {
                        // Native pre-arm response is outside this replacement.
                        // Retire without damage instead of bouncing or tunnelling.
                        Step::Retire("infantry_rocket_prearm_contact_unqualified")
                    } else {
                        Step::Explode {
                            position: contact.center,
                            reason: "contact",
                            contact: Some(contact),
                        }
                    }
                }
                TerminalReason::AgeExpired => Step::Explode {
                    position: terminal.position,
                    reason: if at_sample(terminal.position) {
                        "range"
                    } else {
                        "age"
                    },
                    contact: None,
                },
            },
            Advance::Flying(next) if next != *motion && at_sample(next.position()) => {
                Step::Explode {
                    position: next.position(),
                    reason: "range",
                    contact: None,
                }
            }
            Advance::Flying(next) => Step::Move(FlightMotion::Rocket(next)),
            Advance::AlreadyTerminal => return Err("infantry_rocket_unexpected_terminal"),
        })
    }
}

impl<W: Write> Engine<W> {
    pub(crate) fn tick_infantry_rockets(
        &mut self,
        now: Duration,
        wall: u64,
    ) -> Result<Vec<OutboundMessage>, &'static str> {
        if self.infantry_rockets.is_none()
            && self.underbarrel_launchers.is_none()
            && self.guided_launchers.is_none()
        {
            return Ok(Vec::new());
        }
        let mut flights = Vec::new();
        let mut cleanup = Vec::new();
        for actor in self.active.iter().chain(self.peers.values()) {
            for slot in actor.infantry_rocket_state.slots.iter().flatten() {
                match slot.cleanup {
                    None => flights.push(slot.flight.clone()),
                    Some(due) if due <= now => cleanup.push(slot.flight.clone()),
                    Some(_) => (),
                }
            }
        }
        if !cleanup.is_empty() {
            let queued = self.queued_projectile_terminals();
            let due = now
                .checked_add(CLEANUP_GRACE)
                .ok_or("projectile_cleanup_time_overflow")?;
            for flight in cleanup {
                if queued.contains(&(flight.creation.slot, flight.source.player)) {
                    if let Some(slot) = self.infantry_slot(&flight).as_deref_mut() {
                        slot.cleanup = Some(due);
                    }
                } else if self.infantry_publications_fit(flight.source.world, 1) {
                    self.retire_infantry_rocket(&flight, "terminal_cleanup", now, wall)?;
                }
            }
        }
        if flights.is_empty() {
            return Ok(Vec::new());
        }
        let now_ms = millis(now)?;
        flights.sort_unstable_by_key(|flight| (flight.source.player, flight.creation.slot));
        let bucket = now_ms / world_tick::WORLD_TICK_INTERVAL.as_millis() as u64;
        let start = (bucket % flights.len() as u64) as usize * SWEEP_WINDOW % flights.len();
        flights.rotate_left(start);
        flights.truncate(SWEEP_WINDOW);

        let steps = self
            .plan_infantry_steps(&flights, now, now_ms)
            .unwrap_or_else(|reason| (0..flights.len()).map(|_| Err(reason)).collect());
        let mut messages = Vec::new();
        for (mut flight, step) in flights.into_iter().zip(steps) {
            let step = match step {
                Ok(step) => step,
                Err(code)
                    if code.ends_with("_work_limit")
                        && now_ms.saturating_sub(flight.motion.advanced_at_ms())
                            < QUERY_RETRY_MS =>
                {
                    continue;
                }
                Err(code) => Step::Retire(code),
            };
            match step {
                Step::Move(next) if next != flight.motion => {
                    if wire_position(next.position()).validate().is_err() {
                        self.retire_infantry_rocket(
                            &flight,
                            "infantry_rocket_wire_boundary",
                            now,
                            wall,
                        )?;
                        continue;
                    }
                    flight.motion = next.clone();
                    let packet = (now.saturating_sub(flight.last_published) >= POSITION_INTERVAL
                        && self.infantry_publications_fit(flight.source.world, 1))
                    .then(|| flight.delta(next.position(), None, now, self.wire_profile))
                    .transpose()?;
                    self.record(json!({"op":"infantry_rocket_motion","source":flight.source,
                        "position":next.position(),"packet":packet.as_ref().map(|wire|protocol::encode_hex(wire))}), wall)?;
                    if let Some(packet) = packet {
                        self.queue_infantry_packet(flight.source.world, &packet);
                        flight.last_published = now;
                    }
                    flight.motion = next;
                    let slot = self.infantry_slot(&flight);
                    *slot = Some(Box::new(Slot {
                        flight,
                        cleanup: None,
                    }));
                }
                Step::Move(_) => (),
                Step::Retire(reason) => self.retire_infantry_rocket(&flight, reason, now, wall)?,
                Step::Explode {
                    position,
                    reason,
                    contact,
                } => {
                    if wire_position(position).validate().is_err() {
                        self.retire_infantry_rocket(
                            &flight,
                            "infantry_rocket_wire_boundary",
                            now,
                            wall,
                        )?;
                    } else {
                        messages.extend(self.finish_infantry_rocket(
                            &flight, position, reason, contact, now, wall,
                        )?);
                    }
                }
            }
        }
        Ok(messages)
    }

    fn plan_infantry_steps(
        &self,
        flights: &[Flight],
        now: Duration,
        now_ms: u64,
    ) -> Result<Vec<Result<Step, &'static str>>, &'static str> {
        if !self.round_combat_allowed(now) || self.match_terminal.is_some() {
            return Err("infantry_rocket_combat_ended");
        }
        let collision = self
            .domination_gameplay
            .as_ref()
            .map(|world| world.frag_blast_collision())
            .or_else(|| {
                self.acquisition_gameplay
                    .as_ref()
                    .and_then(|runtime| runtime.blast_collision())
            })
            .ok_or("infantry_rocket_collision_required")?;
        let snapshot = self.dynamic_collision_snapshot(collision.filter_rows[6])?;
        let scene = Scene::new(
            self.collision_world
                .as_ref()
                .ok_or("infantry_rocket_static_world_required")?,
            collision.sequences,
            &snapshot,
            self.projectile_contacts
                .as_deref()
                .ok_or("infantry_rocket_contact_bindings_required")?,
            *collision.filter_rows,
        )
        .with_dynamic_catalog(self)?;
        let mut budget = QueryLimits::default().max_support_points;
        Ok(flights
            .iter()
            .map(|flight| {
                let attacker = self.infantry_attacker(flight)?;
                let associated = snapshot.bodies().iter().find_map(|body| {
                    (body.target.kind == TargetKind::Character
                        && body.target.key.id == u64::from(attacker))
                    .then_some(body.target.key)
                });
                flight.step(
                    now_ms,
                    associated,
                    flight
                        .guided_target
                        .as_ref()
                        .and_then(|t| self.guided_anchor(t)),
                    |query| scene.query(query, &mut budget),
                )
            })
            .collect())
    }

    fn infantry_attacker(&self, flight: &Flight) -> Result<u32, &'static str> {
        let source = &flight.source;
        if source.round != self.round_number
            || !self.active.iter().chain(self.peers.values()).any(|actor| {
                actor.reservation.id == source.reservation
                    && actor.allocation_sequence == flight.source.allocation_sequence
                    && actor.reservation.world == source.world
                    && actor.reservation.player == source.player
                    && actor.reward_match_id == source.match_id
                    && actor.reservation.character.character_id == source.character_id
                    && actor.team() == source.team
            })
        {
            return Err("stale_infantry_rocket_source");
        }
        Ok(flight.creation.shooter_id)
    }

    fn infantry_slot(&mut self, flight: &Flight) -> &mut Option<Box<Slot>> {
        let id = &flight.source.reservation;
        let actor = self
            .active
            .as_mut()
            .filter(|actor| actor.reservation.id == *id)
            .or_else(|| self.peers.get_mut(id))
            .expect("owned infantry projectile reservation");
        &mut actor.infantry_rocket_state.slots[usize::from(flight.creation.slot)]
    }

    fn infantry_publications_fit(&self, world: u32, count: usize) -> bool {
        self.active
            .iter()
            .chain(self.peers.values())
            .filter(|actor| actor.reservation.world == world && actor.initial_objects.is_some())
            .all(|actor| {
                !actor.replication_backlogged
                    && actor.ordinary_shared_len().saturating_add(count)
                        <= multiplayer::MAX_SHARED_QUEUE
            })
    }

    fn queue_infantry_packet(&mut self, world: u32, packet: &[u8]) {
        for actor in self
            .active
            .iter_mut()
            .chain(self.peers.values_mut())
            .filter(|actor| actor.reservation.world == world && actor.initial_objects.is_some())
        {
            actor.queue_shared(packet.to_vec());
        }
    }

    fn retire_infantry_rocket(
        &mut self,
        flight: &Flight,
        reason: &'static str,
        now: Duration,
        wall: u64,
    ) -> Result<(), &'static str> {
        if !self.infantry_publications_fit(
            flight.source.world,
            1 + usize::from(self.guided_launchers.is_some()),
        ) {
            return Ok(());
        }
        let packet =
            flight
                .removal()
                .encode_with_profile(1, now.as_millis() as u32, self.wire_profile)?;
        self.record(
            json!({"op":"infantry_rocket_retired","source":flight.source,"allocation_sequence":flight.source.allocation_sequence,
            "reason":reason,"packet":protocol::encode_hex(&packet)}),
            wall,
        )?;
        self.queue_infantry_packet(flight.source.world, &packet);
        *self.infantry_slot(flight) = None;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn finish_infantry_rocket(
        &mut self,
        flight: &Flight,
        position: [f64; 3],
        reason: &'static str,
        contact: Option<OwnedContact>,
        now: Duration,
        wall: u64,
    ) -> Result<Vec<OutboundMessage>, &'static str> {
        let world = flight.source.world;
        if !self.infantry_publications_fit(world, 1 + usize::from(self.guided_launchers.is_some()))
        {
            return Ok(Vec::new());
        }
        let packet = flight.delta(position, Some(2), now, self.wire_profile)?;
        let due = now
            .checked_add(CLEANUP_GRACE)
            .ok_or("projectile_cleanup_time_overflow")?;
        let (effects, damage_denial) = match self.prepare_infantry_effects(flight, position, now) {
            Ok(effects) => (Some(effects), None),
            Err(
                "reward_outbox_full"
                | "reward_notifications_full"
                | "hospital_rocket_publication_budget_exceeded",
            ) => {
                return Ok(Vec::new());
            }
            Err(code) => (None, Some(code)),
        };
        if let Some(effects) = &effects {
            self.check_acquisition_character_effects(&effects.character, &effects.native)?;
            let ground = usize::from(
                effects
                    .ground
                    .as_ref()
                    .is_some_and(|candidate| candidate.packet.is_some()),
            );
            let helo = effects
                .helo
                .as_ref()
                .map_or(0, |candidate| candidate.packets.len());
            let hospital = effects
                .hospital
                .as_ref()
                .is_some_and(|candidate| candidate.packet.is_some());
            if effects
                .character
                .check_extra_shared_for(self, world, |actor| {
                    let base = actor.initial_objects.is_some();
                    let helo_participant = base && actor.helo_creation_sent;
                    let hospital_participant =
                        hospital && events::hospital_blast_damage_recipient(actor, world);
                    (base || hospital_participant).then_some(
                        usize::from(base)
                            * (1 + ground + usize::from(self.guided_launchers.is_some()))
                            + usize::from(helo_participant) * helo
                            + usize::from(hospital_participant),
                    )
                })
                .is_err()
            {
                return Ok(Vec::new());
            }
        }
        self.record(
            json!({"op":"infantry_rocket_terminal","source":flight.source,"allocation_sequence":flight.source.allocation_sequence,
            "terminal":reason,"position":position,
            "contact":contact.as_ref().map(|hit| json!({"obstacle":format!("{:?}",hit.obstacle),
                "fraction":hit.fraction,"center":hit.center,"normal":hit.normal})),
            "effects":effects.as_ref().map(|fx|fx.character.audit()),
            "ground_effects":effects.as_ref().and_then(|fx|fx.ground.as_ref()).map(|c|&c.audit),
            "helo_effects":effects.as_ref().and_then(|fx|fx.helo.as_ref()).map(|c|&c.audit),
            "hospital_effects":effects.as_ref().and_then(|fx|fx.hospital.as_ref()).map(|c|&c.audit),
            "damage_denial":damage_denial,"packet":protocol::encode_hex(&packet)}),
            wall,
        )?;
        let mut messages = Vec::new();
        if let Some(Effects {
            character,
            mut native,
            ground,
            helo,
            hospital,
        }) = effects
        {
            messages.extend(self.install_character_effects_after_record(character, &mut native));
            if let Some(candidate) = ground {
                if let Some(after) = candidate.after {
                    self.ground_runtime = Some(after);
                }
                if let Some(packet) = candidate.packet {
                    self.queue_infantry_packet(world, &packet);
                }
            }
            if let Some(candidate) = helo {
                if let Some(after) = candidate.after {
                    self.domination_gameplay
                        .as_mut()
                        .expect("prepared Helo world")
                        .state
                        .helo = Some(after);
                }
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
                    for packet in &candidate.packets {
                        actor.queue_shared(packet.clone());
                    }
                }
            }
            if let Some(candidate) = hospital
                && let Some(packet) = candidate.packet
            {
                candidate
                    .patch
                    .install(self.bunker_world.as_mut().expect("prepared Hospital world"));
                for actor in self
                    .active
                    .iter_mut()
                    .chain(self.peers.values_mut())
                    .filter(|actor| events::hospital_blast_damage_recipient(actor, world))
                {
                    actor.queue_shared(packet.clone());
                }
            }
        }
        self.queue_infantry_packet(world, &packet);
        if let Some(slot) = self.infantry_slot(flight).as_deref_mut() {
            slot.cleanup = Some(due);
        }
        Ok(messages)
    }

    fn prepare_infantry_effects(
        &mut self,
        flight: &Flight,
        position: [f64; 3],
        now: Duration,
    ) -> Result<Effects, &'static str> {
        self.infantry_attacker(flight)?;
        let blast = flight.variant.blast();
        let vehicle_damage = flight.variant.vehicle_damage();
        if let Some(world) = &self.domination_gameplay {
            let state = &world.state;
            let helo_rules = state.hospital_rocket_helo_areas.clone();
            let hospital_enabled = state.hospital_blast_areas.is_some();
            let filter_rows = state.filter_rows;
            let sequences = state.collisions.clone();
            let costs = state.structural.material_penetration_costs().clone();
            let terrain = Arc::clone(&state.terrain_materials);
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
                blast,
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
                        GroundRocketDose::equipment(vehicle_damage, blast),
                        rules,
                        self.projectile_contacts
                            .as_deref()
                            .ok_or("infantry_rocket_contact_bindings_required")?,
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
                        HeloRocketDose::equipment(vehicle_damage, blast),
                        rules,
                        collision,
                        now,
                    )
                })
                .transpose()?;
            let hospital = hospital_enabled
                .then(|| {
                    if flight.variant.is_underbarrel() {
                        self.prepare_equipment_hospital_blast(
                            &flight.source,
                            position,
                            vehicle_damage,
                            blast,
                            collision,
                            now,
                        )
                    } else {
                        self.prepare_hospital_rocket_hospital_blast(
                            &flight.source,
                            position,
                            blast,
                            collision,
                            now,
                        )
                    }
                })
                .transpose()?;
            Ok(Effects {
                character,
                native,
                ground,
                helo,
                hospital,
            })
        } else {
            let runtime = self
                .acquisition_gameplay
                .clone()
                .ok_or("infantry_rocket_acquisition_world_required")?;
            let collision = runtime
                .blast_collision()
                .ok_or("infantry_rocket_collision_required")?;
            let (character, native) = self.prepare_hospital_rocket_character_blast(
                &flight.source,
                position,
                blast,
                collision,
                now,
            )?;
            let ground = match flight.variant {
                Variant::Rocket(v) => self.prepare_infantry_rocket_acquisition_ground(
                    &flight.source,
                    position,
                    v.blast,
                    now,
                )?,
                Variant::Guided(v) => self.prepare_infantry_rocket_acquisition_ground(
                    &flight.source,
                    position,
                    v.blast,
                    now,
                )?,
                Variant::Underbarrel { .. } => self.prepare_underbarrel_acquisition_ground(
                    &flight.source,
                    position,
                    vehicle_damage,
                    blast,
                    now,
                )?,
            };
            let ground = Some(GroundRocketCandidate {
                after: ground.after,
                packet: ground.packet,
                audit: ground.audit,
            });
            Ok(Effects {
                character,
                native,
                ground,
                helo: None,
                hospital: None,
            })
        }
    }
}

#[cfg(test)]
#[path = "infantry_rocket_runtime_tests.rs"]
pub(crate) mod tests;

#[cfg(test)]
#[path = "underbarrel_launcher_runtime_tests.rs"]
mod underbarrel_tests;

fn heading_quaternion(heading: [f64; 3]) -> [f32; 4] {
    if heading[2] < -0.999_999_9 {
        return [0., 1., 0., 0.];
    }
    let w = ((1. + heading[2]) * 0.5).sqrt();
    let scale = 2. * w;
    [
        (-heading[1] / scale) as f32,
        (heading[0] / scale) as f32,
        0.,
        w as f32,
    ]
}

#[cfg(test)]
#[path = "guided_launcher_runtime_tests.rs"]
pub(crate) mod guided_tests;
