//! State-bound falling damage from original retail MAG evidence.

use serde::{Deserialize, Serialize};
use std::{io, path::Path};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rules {
    pub schema_version: u32,
    pub source_build: String,
    pub application_id: u32,
    pub min_height: f32,
    pub damage_factor: f32,
    pub minimum_damage_exclusive: f32,
    pub jump_fall_state: u8,
    pub damage_type: u8,
    pub damage_source_type: u8,
}

impl Rules {
    /// Shared native raw falling formula, without an invented impact modifier.
    pub fn damage_for_drop(&self, drop_height: f32) -> Result<Option<FallDamage>, &'static str> {
        if !drop_height.is_finite() {
            return Err("invalid_fall_position");
        }
        if drop_height < self.min_height {
            return Ok(None);
        }
        let excess_height = drop_height - self.min_height;
        let amount = (excess_height * excess_height) * self.damage_factor;
        if !amount.is_finite() {
            return Err("invalid_fall_damage");
        }
        Ok(
            (amount > self.minimum_damage_exclusive).then_some(FallDamage {
                drop_height,
                excess_height,
                amount,
            }),
        )
    }
    /// Apply each planned drop to the previous prediction's remaining health.
    /// Equal-to-health damage and a fractional remainder below 0.25HP are lethal.
    pub fn predict_drop(
        &self,
        drop_height: f32,
        health_quarter_points: u16,
    ) -> Result<FallPrediction, &'static str> {
        let hit = self.damage_for_drop(drop_height)?;
        let remaining_health_quarter_points = crate::damage::remaining_health_quarter_points(
            health_quarter_points,
            hit.map_or(0.0, |h| h.amount),
        )?;
        Ok(FallPrediction {
            hit,
            remaining_health_quarter_points,
        })
    }
    pub fn load(path: impl AsRef<Path>) -> io::Result<Self> {
        let rules: Self = serde_json::from_reader(std::fs::File::open(path)?)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        if rules.schema_version != 1
            || rules.source_build != "retail"
            || rules.application_id != 20231
            || !rules.min_height.is_finite()
            || rules.min_height < 0.0
            || !rules.damage_factor.is_finite()
            || rules.damage_factor <= 0.0
            || !rules.minimum_damage_exclusive.is_finite()
            || rules.minimum_damage_exclusive < 0.0
            || rules.jump_fall_state >= 64
            || rules.damage_type != 5
            || rules.damage_source_type != 1
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid falling-damage rules",
            ));
        }
        Ok(rules)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct FallDamage {
    pub drop_height: f32,
    pub excess_height: f32,
    pub amount: f32,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct FallPrediction {
    pub hit: Option<FallDamage>,
    pub remaining_health_quarter_points: u16,
}
impl FallPrediction {
    pub fn lethal(self) -> bool {
        self.remaining_health_quarter_points == 0
    }
}

/// Authoritative solver-state landing tracker. Reset with each new life/teleport;
/// unlike owner packet tracking it has no transport sequence or client state.
#[derive(Clone, Copy, Debug, Default)]
pub struct PhysicalTracker {
    generation: Option<u32>,
    previous: Option<crate::bot_physics::Motion>,
    departure_y: Option<f32>,
}
impl PhysicalTracker {
    pub fn reset(
        &mut self,
        generation: u32,
        motion: crate::bot_physics::Motion,
    ) -> Result<(), &'static str> {
        if motion
            .position
            .iter()
            .any(|v| !v.is_finite() || v.abs() > f32::MAX as f64)
        {
            return Err("invalid_fall_position");
        }
        *self = Self {
            generation: Some(generation),
            previous: Some(motion),
            departure_y: None,
        };
        Ok(())
    }
    pub fn observe(
        &mut self,
        rules: &Rules,
        motion: &crate::bot_physics::Motion,
        generation: u32,
    ) -> Result<Option<FallDamage>, &'static str> {
        if self.generation != Some(generation) {
            return Err("stale_fall_life");
        }
        if motion
            .position
            .iter()
            .any(|v| !v.is_finite() || v.abs() > f32::MAX as f64)
        {
            return Err("invalid_fall_position");
        }
        let previous = self.previous.ok_or("missing_fall_position")?;
        let mut departure = self.departure_y;
        if !motion.grounded && departure.is_none() {
            departure = Some(previous.position[1] as f32);
        }
        let hit = if motion.grounded {
            departure
                .take()
                .map(|y| rules.damage_for_drop(y - motion.position[1] as f32))
                .transpose()?
                .flatten()
        } else {
            None
        };
        self.previous = Some(*motion);
        self.departure_y = departure;
        Ok(hit)
    }
}

/// Caller validates transport ownership, per-field versions and readiness first.
/// Clone before staging publication; commit the clone after the event record succeeds.
#[derive(Clone, Debug, Default)]
pub struct Tracker {
    generation: u32,
    last_sequence: u32,
    state: Option<u8>,
    position_y: Option<f32>,
    fall_start_y: Option<f32>,
}

impl Tracker {
    /// Call for spawn, server teleport, mounting/unmounting and death. Preserve
    /// the accepted client-sequence floor across resets within the same session.
    pub fn reset(
        &mut self,
        generation: u32,
        sequence_floor: u32,
        position_y: f32,
    ) -> Result<(), &'static str> {
        if !position_y.is_finite() {
            return Err("invalid_fall_position");
        }
        *self = Self {
            generation,
            last_sequence: sequence_floor,
            position_y: Some(position_y),
            ..Self::default()
        };
        Ok(())
    }

    pub fn observe(
        &mut self,
        rules: &Rules,
        generation: u32,
        sequence: u32,
        state: Option<u8>,
        position_y: Option<f32>,
    ) -> Result<Option<FallDamage>, &'static str> {
        if generation != self.generation {
            return Err("stale_fall_life");
        }
        if sequence <= self.last_sequence {
            return Err("stale_fall_sequence");
        }
        if state.is_some_and(|value| value >= 64) {
            return Err("invalid_fall_state");
        }
        if position_y.is_some_and(|value| !value.is_finite()) {
            return Err("invalid_fall_position");
        }
        let state = state.or(self.state);
        let position_y = position_y.or(self.position_y);
        let mut start_y = self.fall_start_y;
        let mut result = None;
        if state == Some(rules.jump_fall_state) {
            if start_y.is_none() {
                start_y = Some(position_y.ok_or("missing_fall_position")?);
            }
        } else if state.is_some()
            && let Some(start) = start_y.take()
        {
            let drop_height = start - position_y.ok_or("missing_fall_position")?;
            result = rules.damage_for_drop(drop_height)?;
        }
        self.last_sequence = sequence;
        self.state = state;
        self.position_y = position_y;
        self.fall_start_y = start_y;
        Ok(result)
    }
}
