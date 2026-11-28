//! Owned Acquisition ground state. Creation resources remain immutable.
//!
//! Effects are ordered requests, not proof that occupants, world collision or
//! replication have consumed them. Run these operations on the same isolated
//! candidate as their consumers, and install only after every required effect
//! succeeds. In particular, respawn blockers and destruction explosions cannot
//! be acknowledged by ignoring the corresponding event.
use crate::{ground_vehicles, replication::CharacterPosition};
use serde::Deserialize;
use std::collections::BTreeMap;

#[cfg(test)]
#[path = "acquisition_ground_runtime_tests.rs"]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key {
    pub index: u16,
    /// Entity incarnation, unchanged by respawn of the existing allocation.
    pub generation: u32,
}
impl Key {
    pub fn object_id(self) -> u32 {
        0x01000000 | u32::from(self.index)
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub position: CharacterPosition,
    pub rotation: [f32; 4],
}
impl Pose {
    fn validate(self) -> Result<(), &'static str> {
        self.position.validate()?;
        let norm = self.rotation.iter().map(|v| v * v).sum::<f32>();
        if !norm.is_finite() || (norm - 1.).abs() > 0.001 {
            return Err("invalid_ground_rotation");
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Body {
    Inactive,
    Operational,
    /// A separate, explicitly supplied collision template. Never use operational
    /// roots as a substitute for a destroyed hulk.
    Hulk {
        template: String,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Destruction {
    pub damage: f32,
    pub full_radius: f32,
    pub half_radius: f32,
}
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capabilities {
    pub destruction: Option<Destruction>,
    pub hulk_template: Option<String>,
    pub delivered_inert_seconds: Option<f32>,
}
impl Capabilities {
    fn validate(&self) -> Result<(), &'static str> {
        if self.destruction.is_some_and(|v| {
            ![v.damage, v.full_radius, v.half_radius]
                .iter()
                .all(|v| v.is_finite() && *v >= 0.)
        }) || self
            .hulk_template
            .as_ref()
            .is_some_and(|v| v.is_empty() || v.len() > 256)
            || self
                .delivered_inert_seconds
                .is_some_and(|v| !v.is_finite() || v <= 0.)
        {
            return Err("invalid_ground_lifecycle_capabilities");
        }
        Ok(())
    }
}
/// The wrapper must resolve this exact current source point. Pose already
/// includes the source vehicle's placement offset; this layer never adds it.
#[derive(Clone, Copy, Debug)]
pub struct QualifiedRespawn {
    pub spawn_point_index: u16,
    pub spawn_point_token: u32,
    pub active: bool,
    pub pose: Pose,
}
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    DamageChanged {
        health_quarter_points: u16,
        last_attacker: u32,
    },
    DeactivateHulk,
    ForcePosition(CharacterPosition),
    ForceRotation([f32; 4]),
    DamageReset,
    StateChanged(u8),
    KillSpawnBlockers,
    SpawnCounterIncremented(u16),
    VehicleDestroyed {
        attacker: u32,
    },
    KillOccupants {
        attacker: u32,
        position: CharacterPosition,
    },
    DestructionExplosion(Destruction),
    RequestFspDeactivation {
        objective: u32,
        attacker: u32,
        forced: bool,
    },
    DismountOccupants,
    RemovePhysics,
    AddPhysics,
    ActivateHulk {
        template: String,
    },
    SpawnDelaySet(u32),
    QueueRespawn,
}
#[derive(Clone, Debug, PartialEq)]
pub struct LifecycleEvent {
    pub vehicle: Key,
    pub effect: Effect,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Vehicle {
    pub key: Key,
    pub(crate) vehicle_type_token: u32,
    pub(crate) faction: u8,
    pub pose: Pose,
    /// Retail numeric values: allocated0, operational2, dying3, dead4, vaporized5,
    /// inert7, delivered8. Intermediate1/6 are retained by respawn guards.
    pub state: u8,
    pub state_age_seconds: f32,
    pub health_quarter_points: u16,
    pub maximum_health_quarter_points: u16,
    health_hp: f32,
    damage_conditions_resolved: bool,
    damage_location_and_type: u8,
    damage_user_data: u32,
    damage_source_object: u32,
    pub last_attacker: u32,
    pub(crate) bot_plant_source: Option<crate::objective_control::BotPlantSource>,
    pub damage_changed: bool,
    pub vaporize_on_death: bool,
    pub pending_death: bool,
    pub body: Body,
    pub associated_objective: Option<u32>,
    pub spawn_cycle_enabled: bool,
    pub spawning: bool,
    pub spawn_counter: u16,
    pub spawn_delay_counter: u32,
    pub next_spawn_timestamp_ms: u64,
    pub spawn_elapsed_seconds: f32,
    pub inert_seconds: f32,
    pub source_spawn_type: Option<u8>,
    pub source_spawn_point_index: u16,
    pub source_spawn_point_token: u32,
    capabilities: Capabilities,
    driver_position_at_ms: Option<u64>,
    /// The accepted driver position before the current pose, and its time.
    driver_previous_position: Option<(CharacterPosition, u64)>,
    driver_position_credit_m: f64,
    pub(crate) driver_controls: DriverControls,
    pub(crate) turret: Option<crate::hospital_turrets::Runtime>,
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct DriverControls {
    pub brake: Option<bool>,
    pub throttle: Option<f32>,
    pub smoke_screen: Option<bool>,
}

impl Vehicle {
    pub(crate) fn health_hp(&self) -> f32 {
        self.health_hp
    }
    /// The two latest accepted driver positions, oldest first.
    pub(crate) fn driver_motion(
        &self,
    ) -> (
        Option<crate::vehicle_collision::MotionSample>,
        Option<crate::vehicle_collision::MotionSample>,
    ) {
        let sample = |(position, at_ms)| crate::vehicle_collision::MotionSample { position, at_ms };
        (
            self.driver_previous_position.map(sample),
            self.driver_position_at_ms
                .map(|at| sample((self.pose.position, at))),
        )
    }
    pub fn is_dead(&self) -> bool {
        self.health_quarter_points == 0
    }
    fn health_percent(&self) -> f32 {
        let max_hp = f32::from(self.maximum_health_quarter_points) / 4.0;
        (self.health_hp / max_hp) * 100.0
    }

    pub(crate) fn is_dying(&self) -> bool {
        !self.is_dead() && self.health_percent() < 15.0
    }

    pub(crate) fn damage_wire(&self) -> crate::replication::VehicleDamage {
        let conditions = if self.damage_conditions_resolved {
            let percent = self.health_percent();
            u8::from(percent < 70.0)
                | (u8::from(percent < 40.0) << 1)
                | (u8::from(percent < 15.0) << 2)
                | (u8::from(self.is_dead()) << 3)
        } else {
            0
        };
        crate::replication::VehicleDamage {
            last_location_and_type: self.damage_location_and_type,
            health_quarter_points: self.health_quarter_points,
            conditions,
            last_attacker: self.last_attacker,
            last_source_object: self.damage_source_object,
            last_user_data: self.damage_user_data,
        }
    }

    fn set_damage_health(
        &mut self,
        health_hp: f32,
        attacker: u32,
        location_and_type: u8,
        user_data: u32,
    ) -> LifecycleEvent {
        self.last_attacker = attacker;
        self.bot_plant_source = None;
        // Untyped damage retains its existing condition-suppression behavior.
        self.damage_conditions_resolved = location_and_type != 0;
        self.damage_location_and_type = location_and_type;
        self.damage_user_data = user_data;
        self.damage_source_object = u32::MAX;
        let event = self.set_health(health_hp);
        self.pending_death = self.is_dead();
        event
    }

    fn set_health(&mut self, health_hp: f32) -> LifecycleEvent {
        self.health_hp = health_hp;
        self.health_quarter_points = (health_hp * 4.0).trunc() as u16;
        self.damage_changed = true;
        self.event(Effect::DamageChanged {
            health_quarter_points: self.health_quarter_points,
            last_attacker: self.last_attacker,
        })
    }

    fn event(&self, effect: Effect) -> LifecycleEvent {
        LifecycleEvent {
            vehicle: self.key,
            effect,
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Runtime {
    vehicles: BTreeMap<u16, Vehicle>,
    initial: BTreeMap<u16, Vehicle>,
}
impl Runtime {
    /// Initial authority requires no invented physics or property defaults.
    /// The creation position is the source-qualified final placed position.
    pub fn from_catalog(catalog: &ground_vehicles::Catalog) -> Result<Self, &'static str> {
        let mut runtime = Self::default();
        for creation in catalog.creations() {
            let spawn_type = catalog.source_spawn_type(creation);
            runtime.insert_creation(creation, spawn_type)?;
        }
        runtime.configure_turrets(catalog);
        Ok(runtime)
    }
    #[cfg(test)]
    pub(crate) fn regional_hulk_fixture(catalog: &ground_vehicles::Catalog, index: u16) -> Self {
        let mut runtime = Self::default();
        for creation in catalog.creations() {
            let source = if creation.index == index {
                Some(7)
            } else {
                catalog.source_spawn_type(creation)
            };
            runtime.insert_creation(creation, source).unwrap();
        }
        runtime
    }

    /// Explicit bindings are useful for a source adapter or synthetic fixtures;
    /// this constructor does not qualify arbitrary caller data as native proof.
    pub fn from_creation(
        creation: &ground_vehicles::Creation,
        source_spawn_type: Option<u8>,
    ) -> Result<Self, &'static str> {
        let mut runtime = Self::default();
        runtime.insert_creation(creation, source_spawn_type)?;
        Ok(runtime)
    }
    fn insert_creation(
        &mut self,
        creation: &ground_vehicles::Creation,
        source_spawn_type: Option<u8>,
    ) -> Result<(), &'static str> {
        let pose = Pose {
            position: creation.position,
            rotation: creation.rotation,
        };
        pose.validate()?;
        if creation.index >= 32
            || !matches!(creation.initial_state, 0 | 2)
            || creation.health_quarter_points == 0
            || creation.health_quarter_points >= 4096
            || source_spawn_type.is_some_and(|v| !(3..=9).contains(&v))
            || self.vehicles.contains_key(&creation.index)
        {
            return Err("invalid_ground_lifecycle_creation");
        }
        let vehicle = Vehicle {
            key: Key {
                index: creation.index,
                generation: 1,
            },
            vehicle_type_token: creation.vehicle_type_token,
            faction: creation.faction,
            pose,
            state: creation.initial_state,
            state_age_seconds: 0.,
            health_quarter_points: creation.health_quarter_points,
            maximum_health_quarter_points: creation.health_quarter_points,
            health_hp: f32::from(creation.health_quarter_points) / 4.0,
            damage_conditions_resolved: false,
            damage_location_and_type: 0,
            damage_user_data: 0,
            damage_source_object: u32::MAX,
            last_attacker: u32::MAX,
            bot_plant_source: None,
            damage_changed: false,
            vaporize_on_death: false,
            pending_death: false,
            body: if creation.initial_state == 2 {
                Body::Operational
            } else {
                Body::Inactive
            },
            associated_objective: None,
            spawn_cycle_enabled: false,
            spawning: true,
            spawn_counter: 0,
            spawn_delay_counter: 0,
            next_spawn_timestamp_ms: 0,
            spawn_elapsed_seconds: 0.,
            inert_seconds: 0.,
            source_spawn_type,
            source_spawn_point_index: creation.spawn_point_index,
            source_spawn_point_token: creation.spawn_point_token,
            capabilities: Capabilities::default(),
            driver_position_at_ms: None,
            driver_previous_position: None,
            driver_position_credit_m: 0.,
            driver_controls: DriverControls::default(),
            turret: None,
        };
        self.initial.insert(creation.index, vehicle.clone());
        self.vehicles.insert(creation.index, vehicle);
        Ok(())
    }
    pub(crate) fn configure_turrets(&mut self, catalog: &ground_vehicles::Catalog) {
        for vehicles in [&mut self.initial, &mut self.vehicles] {
            for (&index, vehicle) in vehicles {
                vehicle.turret = catalog
                    .turret_policy(index)
                    .map(crate::ground_turrets::Policy::initial_turret);
            }
        }
    }

    pub(crate) fn install_turret(&mut self, index: u16, turret: crate::hospital_turrets::Runtime) {
        if let Some(current) = self
            .vehicles
            .get_mut(&index)
            .and_then(|vehicle| vehicle.turret.as_mut())
        {
            *current = turret;
        }
    }

    pub(crate) fn revoke_turret(&mut self, index: u16) {
        if let Some(turret) = self
            .vehicles
            .get_mut(&index)
            .and_then(|vehicle| vehicle.turret.as_mut())
        {
            turret.revoke();
        }
    }

    pub fn get(&self, index: u16) -> Option<&Vehicle> {
        self.vehicles.get(&index)
    }
    pub fn vehicles(&self) -> impl Iterator<Item = &Vehicle> {
        self.vehicles.values()
    }
    fn mutate<T>(
        &mut self,
        key: Key,
        action: impl FnOnce(&mut Vehicle) -> Result<T, &'static str>,
    ) -> Result<T, &'static str> {
        let current = self
            .vehicles
            .get(&key.index)
            .ok_or("unknown_ground_vehicle")?;
        if current.key != key {
            return Err("stale_ground_vehicle_generation");
        }
        let mut candidate = current.clone();
        let result = action(&mut candidate)?;
        self.vehicles.insert(key.index, candidate);
        Ok(result)
    }
    pub fn configure(&mut self, key: Key, capabilities: Capabilities) -> Result<(), &'static str> {
        capabilities.validate()?;
        self.mutate(key, |v| {
            v.capabilities = capabilities.clone();
            Ok(())
        })?;
        // Keep source properties across round resets, without retaining live state.
        self.initial
            .get_mut(&key.index)
            .ok_or("unknown_ground_vehicle")?
            .capabilities = capabilities;
        Ok(())
    }
    /// New round / new entity incarnation. Fail atomically on generation overflow.
    pub fn reset(&mut self) -> Result<(), &'static str> {
        let mut next = self.initial.clone();
        for (index, vehicle) in &mut next {
            vehicle.key.generation = self.vehicles[index]
                .key
                .generation
                .checked_add(1)
                .ok_or("ground_generation_exhausted")?;
        }
        self.vehicles = next;
        Ok(())
    }
    /// A different WorldKey supplies incarnation isolation. This infallible
    /// post-record reset retains source properties and resets local generations.
    pub(crate) fn reset_for_new_world(&mut self) {
        self.vehicles = self.initial.clone();
    }
    pub(crate) fn begin_driver(&mut self, index: u16, now_ms: u64) -> Result<(), &'static str> {
        let key = self.get(index).ok_or("unknown_ground_vehicle")?.key;
        self.mutate(key, |vehicle| {
            vehicle.driver_position_at_ms = Some(now_ms);
            vehicle.driver_previous_position = None;
            vehicle.driver_position_credit_m = 1. / 16.;
            Ok(())
        })
    }

    /// Validate bounds before applying a sparse report to an isolated candidate.
    /// The caller supplies the authenticated, currently seated Ground driver.
    pub(crate) fn apply_driver_dynamics(
        &mut self,
        report: ground_vehicles::OwnerDynamics,
        policy: ground_vehicles::DriverSeat,
        now_ms: u64,
    ) -> Result<(), &'static str> {
        let key = self.get(report.index).ok_or("unknown_ground_vehicle")?.key;
        self.mutate(key, |vehicle| {
            if vehicle.state != 2 || vehicle.is_dead() || vehicle.body != Body::Operational {
                return Err("ground_vehicle_not_operational");
            }
            if let Some(position) = report.position {
                let elapsed = now_ms
                    .checked_sub(
                        vehicle
                            .driver_position_at_ms
                            .ok_or("ground_driver_grant_required")?,
                    )
                    .ok_or("stale_ground_vehicle_pose_time")?;
                let previous = vehicle.pose.position;
                let squared = [
                    position.x - previous.x,
                    position.y - previous.y,
                    position.z - previous.z,
                ]
                .iter()
                .map(|v| f64::from(*v).powi(2))
                .sum::<f64>();
                // A fixed-point cell allows axis truncation. Cap time credit so an
                // idle connection cannot accumulate an unbounded position jump.
                let speed = f64::from(policy.maximum_speed_mps);
                let credit = (vehicle.driver_position_credit_m
                    + speed * elapsed.min(1000) as f64 / 1000.)
                    .min(speed + 1. / 16.);
                if squared > credit * credit {
                    return Err("ground_vehicle_owner_pose_out_of_range");
                }
                vehicle.driver_position_credit_m = (credit - squared.sqrt()).max(0.);
                vehicle.driver_previous_position =
                    vehicle.driver_position_at_ms.map(|at| (previous, at));
                vehicle.pose.position = position;
                vehicle.driver_position_at_ms = Some(now_ms);
            }
            if let Some(quaternion) = report.rotation_xyzw {
                let norm_squared = quaternion.iter().map(|value| value * value).sum::<f32>();
                if !norm_squared.is_finite() || (norm_squared - 1.).abs() > 0.02 {
                    return Err("invalid_ground_vehicle_owner_rotation");
                }
                let norm = norm_squared.sqrt();
                vehicle.pose.rotation = quaternion.map(|value| value / norm);
            }
            if report.throttle.is_some_and(|value| value.abs() > 1.) {
                return Err("invalid_ground_vehicle_owner_throttle");
            }
            vehicle.pose.validate()?;
            if let Some(brake) = report.brake {
                vehicle.driver_controls.brake = Some(brake);
            }
            if let Some(throttle) = report.throttle {
                vehicle.driver_controls.throttle = Some(throttle);
            }
            if let Some(smoke_screen) = report.smoke_screen {
                vehicle.driver_controls.smoke_screen = Some(smoke_screen);
            }
            Ok(())
        })
    }

    /// Accepted authoritative movement only. This is a pose setter, not driving,
    /// collision integration, or admission of client-supplied physics.
    pub fn force_pose(
        &mut self,
        key: Key,
        pose: Pose,
    ) -> Result<Vec<LifecycleEvent>, &'static str> {
        pose.validate()?;
        self.mutate(key, |v| {
            v.pose = pose;
            Ok(vec![
                v.event(Effect::ForcePosition(pose.position)),
                v.event(Effect::ForceRotation(pose.rotation)),
            ])
        })
    }
    pub fn enable_cycle(&mut self, key: Key, objective: u32) -> Result<(), &'static str> {
        if objective & 0xffff0000 != 0x0c800000 || objective as u16 == u16::MAX {
            return Err("invalid_ground_associated_objective");
        }
        self.mutate(key, |v| {
            v.associated_objective = Some(objective);
            v.spawn_cycle_enabled = true;
            v.spawning = true;
            Ok(())
        })
    }
    pub fn disable_cycle(&mut self, key: Key) -> Result<(), &'static str> {
        self.mutate(key, |v| {
            v.spawn_cycle_enabled = false;
            v.spawning = false;
            Ok(())
        })
    }
    /// Ground wrapper cleanup happens even when the base respawn is a healthy
    /// no-op. The point's u16 spawn counter increments after either outcome.
    pub fn respawn_existing(
        &mut self,
        key: Key,
        source: QualifiedRespawn,
        initial: bool,
    ) -> Result<Vec<LifecycleEvent>, &'static str> {
        source.pose.validate()?;
        self.mutate(key, |v| {
            if source.spawn_point_index != v.source_spawn_point_index
                || source.spawn_point_token != v.source_spawn_point_token
            {
                return Err("ground_respawn_source_mismatch");
            }
            let source_type = v
                .source_spawn_type
                .ok_or("ground_source_type_unavailable")?;
            if matches!(source_type, 7 | 9) && !source.active {
                return Err("ground_respawn_source_inactive");
            }
            let mut events = vec![v.event(Effect::DeactivateHulk)];
            if matches!(v.body, Body::Hulk { .. }) {
                v.body = Body::Inactive;
            }
            let direct = matches!(v.state, 0 | 1 | 4 | 5 | 6);
            if direct || (!initial && v.is_dead()) {
                if let Some(turret) = &mut v.turret {
                    turret.revoke();
                }
                v.pose = source.pose;
                v.driver_position_at_ms = None;
                v.driver_previous_position = None;
                v.driver_position_credit_m = 0.;
                v.driver_controls = DriverControls::default();
                events.push(v.event(Effect::ForcePosition(v.pose.position)));
                events.push(v.event(Effect::ForceRotation(v.pose.rotation)));
                if !initial {
                    v.set_damage_health(
                        f32::from(v.maximum_health_quarter_points) / 4.0,
                        u32::MAX,
                        0,
                        0,
                    );
                    events.push(v.event(Effect::DamageReset));
                }
                v.vaporize_on_death = false;
                if direct {
                    v.state = 2;
                    v.state_age_seconds = 0.;
                    // Server state changes here; Ground Update registers roots
                    // later. Allocated geometry still serves the blocker query.
                    events.push(v.event(Effect::StateChanged(v.state)));
                }
                events.push(v.event(Effect::KillSpawnBlockers));
            }
            v.spawn_counter = v.spawn_counter.wrapping_add(1);
            events.push(v.event(Effect::SpawnCounterIncremented(v.spawn_counter)));
            Ok(events)
        })
    }
    /// Evaluated health damage only: callers own native damage admission and
    /// modifiers. Lethal health changes do not synchronously invoke OnDeath.
    pub fn apply_damage(
        &mut self,
        key: Key,
        quarter_points: u16,
        attacker: u32,
    ) -> Result<Vec<LifecycleEvent>, &'static str> {
        self.mutate(key, |v| {
            if v.is_dead() || quarter_points == 0 {
                return Ok(vec![]);
            }
            let hp = (v.health_hp - f32::from(quarter_points) / 4.0).max(0.0);
            Ok(vec![v.set_damage_health(hp, attacker, 0, 0)])
        })
    }

    /// Apply the ordinary live Ground model's percent heal while preserving
    /// the existing damage attribution and lifecycle.
    pub(crate) fn heal_percent(
        &mut self,
        key: Key,
        percent: f32,
    ) -> Result<Vec<LifecycleEvent>, &'static str> {
        if !percent.is_finite() {
            return Err("invalid_acquisition_ground_heal_percent");
        }
        self.mutate(key, |v| {
            if percent <= 0.0
                || v.state != 2
                || v.body != Body::Operational
                || v.is_dead()
                || v.is_dying()
            {
                return Ok(vec![]);
            }
            let max_hp = f32::from(v.maximum_health_quarter_points) / 4.0;
            let amount_hp = ((percent / 100.0) * max_hp).min(max_hp - v.health_hp);
            if amount_hp < 0.0005 {
                return Ok(vec![]);
            }
            let hp = (v.health_hp + amount_hp).min(max_hp);
            if hp == v.health_hp {
                return Ok(vec![]);
            }
            v.damage_conditions_resolved = true;
            Ok(vec![v.set_health(hp)])
        })
    }

    /// Retail ordinary damage retains float HP and applies the model's
    /// sub-one-HP dead floor before the network quarter-unit projection.
    pub(crate) fn apply_bullet_damage(
        &mut self,
        key: Key,
        amount_hp: f32,
        attacker: u32,
        location: u8,
        user_data: u32,
    ) -> Result<Vec<LifecycleEvent>, &'static str> {
        if location > 15 {
            return Err("invalid_acquisition_ground_hit_location");
        }
        self.apply_weapon_damage(key, amount_hp, attacker, 0x10 | location, user_data)
    }

    pub(crate) fn apply_grenade_damage(
        &mut self,
        key: Key,
        amount_hp: f32,
        attacker: u32,
        source_object: u32,
    ) -> Result<Vec<LifecycleEvent>, &'static str> {
        let events = self.apply_explosion_damage(key, amount_hp, attacker)?;
        if !events.is_empty() {
            self.vehicles
                .get_mut(&key.index)
                .unwrap()
                .damage_source_object = source_object;
        }
        Ok(events)
    }

    /// Discard the staged input from a driver who lost authority in this event.
    /// The independently accepted damage in the same candidate is preserved.
    pub(crate) fn discard_driver_input(&mut self, before: &Self, index: u16) {
        let (Some(candidate), Some(original)) = (self.vehicles.get_mut(&index), before.get(index))
        else {
            return;
        };
        candidate.pose = original.pose;
        candidate.driver_position_at_ms = original.driver_position_at_ms;
        candidate.driver_previous_position = original.driver_previous_position;
        candidate.driver_position_credit_m = original.driver_position_credit_m;
        candidate.driver_controls = original.driver_controls;
    }

    pub(crate) fn apply_explosion_damage(
        &mut self,
        key: Key,
        amount_hp: f32,
        attacker: u32,
    ) -> Result<Vec<LifecycleEvent>, &'static str> {
        self.apply_weapon_damage(key, amount_hp, attacker, 0xa0, 0)
    }

    pub(crate) fn apply_blast_with_bot_origin(
        &mut self,
        key: Key,
        amount_hp: f32,
        attacker: u32,
        origin: Option<crate::objective_control::BotPlantSource>,
    ) -> Result<Vec<LifecycleEvent>, &'static str> {
        if origin.is_some() && attacker != u32::MAX {
            return Err("bot_plant_attacker_must_be_uncredited");
        }
        let events = self.apply_explosion_damage(key, amount_hp, attacker)?;
        if let Some(source) = origin
            && !events.is_empty()
        {
            self.vehicles
                .get_mut(&key.index)
                .expect("accepted blast vehicle")
                .bot_plant_source = Some(source);
        }
        Ok(events)
    }

    pub(crate) fn apply_classified_vehicle_blast_damage(
        &mut self,
        key: Key,
        amount_hp: f32,
        attacker: u32,
        location: u8,
        user_data: u32,
    ) -> Result<Vec<LifecycleEvent>, &'static str> {
        if !(1..=6).contains(&location) {
            return Err("invalid_acquisition_ground_hit_location");
        }
        self.apply_weapon_damage(key, amount_hp, attacker, 0xa0 | location, user_data)
    }

    fn apply_weapon_damage(
        &mut self,
        key: Key,
        amount_hp: f32,
        attacker: u32,
        location_and_type: u8,
        user_data: u32,
    ) -> Result<Vec<LifecycleEvent>, &'static str> {
        if !amount_hp.is_finite() || amount_hp < 0.0 {
            return Err("invalid_acquisition_ground_weapon_damage");
        }
        self.mutate(key, |v| {
            if v.state != 2
                || v.body != Body::Operational
                || v.is_dead()
                || v.is_dying()
                || amount_hp == 0.0
            {
                return Ok(vec![]);
            }
            let remaining = v.health_hp - amount_hp;
            let hp = if remaining < 1.0 { 0.0 } else { remaining };
            if hp == v.health_hp {
                return Ok(vec![]);
            }
            Ok(vec![v.set_damage_health(
                hp,
                attacker,
                location_and_type,
                user_data,
            )])
        })
    }

    pub(crate) fn critical_decay(
        &mut self,
        key: Key,
        loss_percent_per_second: f32,
        dt: f32,
    ) -> Result<Vec<LifecycleEvent>, &'static str> {
        if !dt.is_finite() || dt < 0.0 || !(0.0..=100.0).contains(&loss_percent_per_second) {
            return Err("invalid_acquisition_ground_critical_delta");
        }
        self.mutate(key, |v| {
            if !matches!(v.state, 2 | 3) || v.body != Body::Operational || !v.is_dying() {
                return Ok(vec![]);
            }
            let max_hp = f32::from(v.maximum_health_quarter_points) / 4.0;
            let loss = ((loss_percent_per_second * 0.01) * max_hp) * dt;
            let remaining = v.health_hp - loss;
            let hp = if remaining < 1.0 { 0.0 } else { remaining };
            if hp == v.health_hp {
                return Ok(vec![]);
            }
            let source = v.damage_source_object;
            let bot_plant_source = v.bot_plant_source;
            let event = v.set_damage_health(
                hp,
                v.last_attacker,
                v.damage_location_and_type,
                v.damage_user_data,
            );
            v.damage_source_object = source;
            v.bot_plant_source = bot_plant_source;
            Ok(vec![event])
        })
    }

    pub fn apply_lethal(
        &mut self,
        key: Key,
        attacker: u32,
        vaporize: bool,
    ) -> Result<Vec<LifecycleEvent>, &'static str> {
        self.mutate(key, |v| {
            let mut events = vec![];
            if !v.is_dead() {
                events.push(v.set_damage_health(0.0, attacker, 0, 0));
            }
            v.vaporize_on_death = vaporize;
            Ok(events)
        })
    }
    pub fn deliver(&mut self, key: Key) -> Result<Vec<LifecycleEvent>, &'static str> {
        self.mutate(key, |v| {
            if v.state != 2 || v.body != Body::Operational || v.is_dead() {
                return Err("ground_delivery_requires_operational_vehicle");
            }
            let seconds = v
                .capabilities
                .delivered_inert_seconds
                .ok_or("ground_delivery_timer_unavailable")?;
            v.state = 8;
            if let Some(turret) = &mut v.turret {
                turret.revoke();
            }
            v.state_age_seconds = 0.;
            v.inert_seconds = seconds;
            Ok(vec![v.event(Effect::StateChanged(8))])
        })
    }
    /// Commit state6 after the manager admits this existing APC or MotorPool
    /// vehicle to its team queue. No FSP association or health changes occur.
    pub fn admit_queued_respawn(&mut self, key: Key) -> Result<Vec<LifecycleEvent>, &'static str> {
        self.mutate(key, |vehicle| {
            if !matches!(vehicle.source_spawn_type, Some(3 | 7))
                || vehicle.body == Body::Operational
                || vehicle.spawn_delay_counter != 0
                || !matches!(vehicle.state, 4 | 5)
            {
                return Err("ground_vehicle_not_ready_for_spawn_queue");
            }
            vehicle.state = 6;
            vehicle.state_age_seconds = 0.;
            Ok(vec![vehicle.event(Effect::StateChanged(6))])
        })
    }

    /// Base state update precedes ground teardown. For type9, destruction only
    /// requests objective deactivation; this does not enter the objective state.
    /// No generic reinforcement timer is synthesized for an Escort. The caller
    /// supplies GameState's current next vehicle-spawn timestamp for the actual
    /// vehicle team (or occupant team when native vehicle team is FF); this is
    /// not an elapsed-wall-clock estimate of missed reinforcement cycles.
    pub fn update(
        &mut self,
        key: Key,
        dt: f32,
        next_spawn_timestamp_ms: u64,
    ) -> Result<Vec<LifecycleEvent>, &'static str> {
        if !dt.is_finite() || dt < 0. {
            return Err("invalid_ground_update_delta");
        }
        self.mutate(key, |v| {
            let age = v.state_age_seconds + dt;
            let spawn_age = v.spawn_elapsed_seconds + dt;
            if !age.is_finite() || !spawn_age.is_finite() {
                return Err("invalid_ground_update_delta");
            }
            v.state_age_seconds = age;
            v.spawn_elapsed_seconds = spawn_age;
            if next_spawn_timestamp_ms > v.next_spawn_timestamp_ms {
                v.next_spawn_timestamp_ms = next_spawn_timestamp_ms;
                v.spawn_elapsed_seconds = 0.;
                v.spawn_delay_counter = v.spawn_delay_counter.saturating_sub(1);
            }
            let mut events = vec![];
            if !matches!(v.state, 1 | 4 | 5 | 6) && v.is_dead() {
                let destruction = v
                    .capabilities
                    .destruction
                    .ok_or("ground_destruction_properties_unavailable")?;
                if !v.vaporize_on_death
                    && v.body == Body::Operational
                    && v.source_spawn_type != Some(9)
                    && v.capabilities.hulk_template.is_none()
                {
                    return Err("ground_hulk_template_unavailable");
                }
                v.state = if v.vaporize_on_death { 5 } else { 4 };
                v.state_age_seconds = 0.;
                v.pending_death = false;
                if let Some(turret) = &mut v.turret {
                    turret.revoke();
                }
                events.push(v.event(Effect::StateChanged(v.state)));
                events.push(v.event(Effect::VehicleDestroyed {
                    attacker: v.last_attacker,
                }));
                events.push(v.event(Effect::KillOccupants {
                    attacker: v.last_attacker,
                    position: v.pose.position,
                }));
                events.push(v.event(Effect::DestructionExplosion(destruction)));
            }
            if v.state == 2 && v.body == Body::Operational && v.is_dying() {
                v.state = 3;
                if let Some(turret) = &mut v.turret {
                    turret.revoke();
                }
                v.state_age_seconds = 0.;
                events.push(v.event(Effect::StateChanged(3)));
            }
            if v.inert_seconds > 0. {
                v.inert_seconds -= dt;
                if v.inert_seconds <= 0. && v.state != 7 {
                    v.state = 7;
                    v.state_age_seconds = 0.;
                    events.push(v.event(Effect::StateChanged(7)));
                }
            }
            if matches!(v.state, 4 | 5) && v.body == Body::Operational {
                let source_type = v
                    .source_spawn_type
                    .ok_or("ground_source_type_unavailable")?;
                if source_type == 9 {
                    let objective = v
                        .associated_objective
                        .ok_or("ground_associated_objective_unavailable")?;
                    events.push(v.event(Effect::RequestFspDeactivation {
                        objective,
                        attacker: v.last_attacker,
                        forced: true,
                    }));
                }
                events.push(v.event(Effect::DismountOccupants));
                events.push(v.event(Effect::RemovePhysics));
                v.body = Body::Inactive;
                // ActivateHulk itself rejects source type9, even for state4.
                if v.state != 5 && source_type != 9 {
                    let template = v
                        .capabilities
                        .hulk_template
                        .clone()
                        .ok_or("ground_hulk_template_unavailable")?;
                    v.body = Body::Hulk {
                        template: template.clone(),
                    };
                    events.push(v.event(Effect::ActivateHulk { template }));
                }
                v.spawn_delay_counter = 1;
                v.next_spawn_timestamp_ms = next_spawn_timestamp_ms;
                events.push(v.event(Effect::SpawnDelaySet(1)));
            }
            if v.state == 7 && v.body != Body::Inactive {
                v.body = Body::Inactive;
                events.push(v.event(Effect::RemovePhysics));
            } else if v.state == 2 && v.body != Body::Operational {
                if matches!(v.body, Body::Hulk { .. }) {
                    events.push(v.event(Effect::DeactivateHulk));
                }
                v.body = Body::Operational;
                events.push(v.event(Effect::AddPhysics));
            }
            if matches!(v.state, 4 | 5) && v.body != Body::Operational && v.spawn_delay_counter == 0
            {
                if matches!(v.source_spawn_type, Some(3 | 7)) {
                    events.push(v.event(Effect::QueueRespawn));
                } else if v.source_spawn_type != Some(9) && v.spawn_cycle_enabled {
                    return Err("ground_generic_requeue_unavailable");
                }
            }
            Ok(events)
        })
    }
}
