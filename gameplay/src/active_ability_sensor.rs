//! Detection parameters and bounded scans for the native command Sensor.

use serde::{Deserialize, Serialize};
use std::{ops::Range, time::Duration};

/// Native global detection refresh applied by both Sensor variants.
pub const MARK_DURATION: Duration = Duration::from_secs(5);
/// Native squad member indices scanned by one attacker Sensor update.
pub const SQUAD_MEMBER_SLOTS: Range<u8> = 0..8;
/// Native opponent squad pool indices for the attacker Sensor.
pub const DEFENDER_SQUAD_SLOTS: Range<u16> = 16..32;

#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SensorRules {
    pub delay_ms: u32,
    pub duration_ms: u32,
    pub attacker_radius: f32,
    pub defender_radius: f32,
    pub speed_threshold: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Detector {
    radius: f32,
    radius_squared: f32,
    speed_threshold: f32,
}

impl Detector {
    pub fn new(radius: f32, speed_threshold: f32) -> Result<Self, &'static str> {
        let radius_squared = radius * radius;
        if !radius_squared.is_finite()
            || radius <= 0.
            || !speed_threshold.is_finite()
            || speed_threshold < 0.
        {
            return Err("invalid_command_sensor_detector");
        }
        Ok(Self {
            radius,
            radius_squared,
            speed_threshold,
        })
    }

    pub fn radius(self) -> f32 {
        self.radius
    }

    pub fn speed_threshold(self) -> f32 {
        self.speed_threshold
    }

    /// Call only for an eligible owned target. Speed comes from authoritative
    /// movement history, and the caller supplies the ability or objective origin.
    pub fn detects(self, origin: [f32; 3], position: [f32; 3], speed: f32) -> bool {
        let [dx, dy, dz] = std::array::from_fn(|axis| position[axis] - origin[axis]);
        let distance_squared = (dx * dx + dy * dy) + dz * dz;
        speed.is_finite() && speed >= self.speed_threshold && distance_squared < self.radius_squared
    }
}

/// The caller enables surveillance on each live objective in the strike
/// platoon's actual objective list and evaluates its own objective state gate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Surveillance {
    pub detector: Detector,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SquadScan {
    pub slot: u16,
    pub origin: [f32; 3],
    pub detector: Detector,
}

/// Maintain one cursor per objective. Four native character pool slots are
/// visited per update, including unallocated or ineligible slots.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ObjectiveCursor {
    next: u16,
}

impl ObjectiveCursor {
    pub fn take(&mut self, capacity: u16) -> Range<u16> {
        let start = if self.next < capacity { self.next } else { 0 };
        let end = start.saturating_add(4).min(capacity);
        self.next = if end == capacity { 0 } else { end };
        start..end
    }
}

/// Retail objective updates run surveillance for primary and secondary
/// categories, plus the defensive emplacement subtype in any category.
pub fn objective_supports_surveillance(category: u32, subtype: u32) -> bool {
    category <= 3 || subtype == 3
}
