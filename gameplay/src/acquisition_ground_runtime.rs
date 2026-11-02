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
}
#[derive(Clone, Debug, PartialEq)]
pub struct LifecycleEvent {
    pub vehicle: Key,
    pub effect: Effect,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Vehicle {
    pub key: Key,
    pub pose: Pose,
    /// Retail numeric values: allocated0, operational2, dead4, vaporized5,
    /// inert7, delivered8. Intermediate1/6 are retained by respawn guards.
    pub state: u8,
    pub state_age_seconds: f32,
    pub health_quarter_points: u16,
    pub maximum_health_quarter_points: u16,
    pub last_attacker: u32,
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
}
impl Vehicle {
    pub fn is_dead(&self) -> bool {
        self.health_quarter_points == 0
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
            let spawn_type = catalog.vehicle_source(creation.index).map(|v| v.spawn_type);
            runtime.insert_creation(creation, spawn_type)?;
        }
        Ok(runtime)
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
            || source_spawn_type.is_some_and(|v| !(4..=9).contains(&v))
            || self.vehicles.contains_key(&creation.index)
        {
            return Err("invalid_ground_lifecycle_creation");
        }
        let vehicle = Vehicle {
            key: Key {
                index: creation.index,
                generation: 1,
            },
            pose,
            state: creation.initial_state,
            state_age_seconds: 0.,
            health_quarter_points: creation.health_quarter_points,
            maximum_health_quarter_points: creation.health_quarter_points,
            last_attacker: u32::MAX,
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
            spawning: false,
            spawn_counter: 0,
            spawn_delay_counter: 0,
            next_spawn_timestamp_ms: 0,
            spawn_elapsed_seconds: 0.,
            inert_seconds: 0.,
            source_spawn_type,
            source_spawn_point_index: creation.spawn_point_index,
            source_spawn_point_token: creation.spawn_point_token,
            capabilities: Capabilities::default(),
        };
        self.initial.insert(creation.index, vehicle.clone());
        self.vehicles.insert(creation.index, vehicle);
        Ok(())
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
                v.pose = source.pose;
                events.push(v.event(Effect::ForcePosition(v.pose.position)));
                events.push(v.event(Effect::ForceRotation(v.pose.rotation)));
                if !initial {
                    v.health_quarter_points = v.maximum_health_quarter_points;
                    v.last_attacker = u32::MAX;
                    v.damage_changed = true;
                    v.pending_death = false;
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
            v.health_quarter_points = v.health_quarter_points.saturating_sub(quarter_points);
            v.last_attacker = attacker;
            v.damage_changed = true;
            v.pending_death = v.is_dead();
            Ok(vec![v.event(Effect::DamageChanged {
                health_quarter_points: v.health_quarter_points,
                last_attacker: v.last_attacker,
            })])
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
                v.health_quarter_points = 0;
                v.last_attacker = attacker;
                v.damage_changed = true;
                v.pending_death = true;
                events.push(v.event(Effect::DamageChanged {
                    health_quarter_points: 0,
                    last_attacker: attacker,
                }));
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
            v.state_age_seconds = 0.;
            v.inert_seconds = seconds;
            Ok(vec![v.event(Effect::StateChanged(8))])
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
            if matches!(v.state, 4 | 5)
                && v.body != Body::Operational
                && v.spawn_cycle_enabled
                && v.spawn_delay_counter == 0
                && v.source_spawn_type != Some(9)
            {
                return Err("ground_generic_requeue_unavailable");
            }
            Ok(events)
        })
    }
}
