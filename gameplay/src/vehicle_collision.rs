//! Ground-vehicle impacts on characters.
//!
//! The retail client of the player driving a ground vehicle detects when the
//! vehicle touches a character and sends message `0x020d` naming the vehicle
//! (the attacker) and the character (the defender). The server decides the
//! damage. An ordinary collision hurts only above a minimum closing speed and
//! grows linearly with the speed above it. A catastrophic collision is a flat
//! amount; against the vehicle's own side it is first divided by the match's
//! friendly-fire multiplier, so the scaled result still kills a teammate
//! whenever friendly fire is on at all. The driver is the attacker of record.
//!
//! All tuning comes from an explicit external rules file
//! (`--vehicle-collision-rules`); nothing here is a game preset.
use crate::replication::CharacterPosition;
use serde::Deserialize;
use std::{io, path::Path};

/// Retail `NetMessageVehicle` kinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReportKind {
    /// The driver reports its own vehicle as stuck.
    Stuck,
    /// An ordinary collision; damage depends on the closing speed.
    CharacterCollision,
    /// A collision the client judged catastrophic; damage is flat.
    Catastrophic,
}
impl ReportKind {
    pub(crate) fn from_wire(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Stuck),
            1 => Some(Self::CharacterCollision),
            2 => Some(Self::Catastrophic),
            _ => None,
        }
    }
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Stuck => "stuck",
            Self::CharacterCollision => "character_collision",
            Self::Catastrophic => "catastrophic",
        }
    }
}

/// Client message `0x020d`: one byte kind, then the attacker and defender IDs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VehicleReport {
    pub kind: ReportKind,
    pub attacker: u32,
    pub defender: u32,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Rules {
    pub schema_version: u32,
    pub source_build: String,
    pub application_id: u32,
    /// Closing speed (m/s) at or below which an ordinary collision does nothing.
    pub minimum_velocity_mps: f32,
    /// Raw damage per m/s of closing speed above the minimum.
    pub damage_per_mps: f32,
    /// Raw damage of a catastrophic collision.
    pub catastrophic_damage_hp: f32,
    /// A character that left this vehicle this recently is not hit by it.
    pub recent_exit_grace_ms: u64,
    /// A motion sample older than this counts as standing still.
    pub motion_freshness_ms: u64,
    /// Shortest interval a velocity estimate divides by.
    pub minimum_motion_interval_ms: f32,
    /// Native damage kind (ExternalKinetic, 5) without the KineticResponse flag.
    pub damage_type: u8,
    /// Native DamageSourceType of the death notice (Vehicle, 3).
    pub damage_source_type: u8,
    /// Audit only.
    #[serde(default)]
    pub provenance: Option<serde_json::Value>,
}
impl Rules {
    pub fn load(path: impl AsRef<Path>) -> io::Result<Self> {
        let text = std::fs::read_to_string(path)?;
        Self::from_json(&text)
    }
    pub fn from_json(text: &str) -> io::Result<Self> {
        let invalid = |_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid vehicle collision rules",
            )
        };
        let rules: Self = serde_json::from_str(text).map_err(invalid)?;
        rules
            .validate()
            .map_err(|code| io::Error::new(io::ErrorKind::InvalidData, code))?;
        Ok(rules)
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        let finite_non_negative = |v: f32| v.is_finite() && v >= 0.0;
        if self.schema_version != 1
            || self.source_build != "retail"
            || self.application_id != 20231
            || !finite_non_negative(self.minimum_velocity_mps)
            || !self.damage_per_mps.is_finite()
            || self.damage_per_mps <= 0.0
            || !self.catastrophic_damage_hp.is_finite()
            || self.catastrophic_damage_hp <= 0.0
            || self.recent_exit_grace_ms > 60_000
            || self.motion_freshness_ms == 0
            || self.motion_freshness_ms > 60_000
            || !self.minimum_motion_interval_ms.is_finite()
            || self.minimum_motion_interval_ms <= 0.0
            || self.damage_type != 5
            || self.damage_source_type != 3
            || self
                .provenance
                .as_ref()
                .is_some_and(|value| !value.is_object())
        {
            return Err("invalid_vehicle_collision_rules");
        }
        Ok(())
    }

    /// Velocity from the two latest accepted positions, or `None` (standing
    /// still) when there is no earlier sample or the latest one is stale.
    pub fn estimated_velocity(
        &self,
        previous: Option<MotionSample>,
        latest: Option<MotionSample>,
        now_ms: u64,
    ) -> Option<[f32; 3]> {
        let (previous, latest) = (previous?, latest?);
        if now_ms > latest.at_ms.saturating_add(self.motion_freshness_ms)
            || latest.at_ms < previous.at_ms
        {
            return None;
        }
        let elapsed_ms =
            ((latest.at_ms - previous.at_ms) as f32).max(self.minimum_motion_interval_ms);
        let seconds = elapsed_ms / 1000.0;
        let velocity = [
            (latest.position.x - previous.position.x) / seconds,
            (latest.position.y - previous.position.y) / seconds,
            (latest.position.z - previous.position.z) / seconds,
        ];
        velocity.iter().all(|v| v.is_finite()).then_some(velocity)
    }

    /// Raw damage before friendly-fire scaling, armor and owned modifiers.
    /// `friendly_fire` is the match multiplier when the victim is on the
    /// vehicle's side, otherwise `None`.
    pub fn raw_damage(
        &self,
        kind: ReportKind,
        closing_speed_mps: f32,
        friendly_fire: Option<f32>,
    ) -> Option<f32> {
        let amount = match kind {
            ReportKind::Stuck => return None,
            ReportKind::CharacterCollision => {
                if closing_speed_mps.is_nan() || closing_speed_mps <= self.minimum_velocity_mps {
                    return None;
                }
                self.damage_per_mps * (closing_speed_mps - self.minimum_velocity_mps)
            }
            // The later friendly-fire scaling cancels this division.
            ReportKind::Catastrophic => match friendly_fire {
                Some(multiplier) if multiplier > 0.0 => self.catastrophic_damage_hp / multiplier,
                _ => self.catastrophic_damage_hp,
            },
        };
        (amount.is_finite() && amount > 0.0).then_some(amount)
    }
}

/// One accepted position and the server time it arrived.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MotionSample {
    pub position: CharacterPosition,
    pub at_ms: u64,
}

fn length(v: [f32; 3]) -> f32 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

/// Closing speed along the vehicle's direction of travel. With only one of
/// the two moving it is that one's speed; with both moving it is the vehicle's
/// speed less the character's speed component along the vehicle's direction,
/// as a magnitude.
pub fn closing_speed(vehicle: Option<[f32; 3]>, character: Option<[f32; 3]>) -> f32 {
    let vehicle_speed = vehicle.map_or(0.0, length);
    let character_speed = character.map_or(0.0, length);
    if vehicle_speed.is_nan() || vehicle_speed <= 0.0 {
        return character_speed.max(0.0);
    }
    if character_speed.is_nan() || character_speed <= 0.0 {
        return vehicle_speed;
    }
    let (v, c) = (vehicle.unwrap(), character.unwrap());
    let cosine = (v[0] * c[0] + v[1] * c[1] + v[2] * c[2]) / (vehicle_speed * character_speed);
    (vehicle_speed - character_speed * cosine).abs()
}

#[cfg(test)]
pub(crate) use tests::synthetic_rules;

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn synthetic_rules() -> Rules {
        Rules::from_json(
            &serde_json::json!({
                "schema_version":1,"source_build":"retail","application_id":20231,
                "minimum_velocity_mps":4.0,"damage_per_mps":20.0,"catastrophic_damage_hp":250.0,
                "recent_exit_grace_ms":1500,"motion_freshness_ms":2500,
                "minimum_motion_interval_ms":40.0,"damage_type":5,"damage_source_type":3,
                "provenance":{"source":"synthetic"}
            })
            .to_string(),
        )
        .unwrap()
    }
    fn at(x: f32, at_ms: u64) -> MotionSample {
        MotionSample {
            position: CharacterPosition { x, y: 0.0, z: 0.0 },
            at_ms,
        }
    }

    #[test]
    fn rules_reject_other_kinds_sources_and_bad_tuning() {
        for (field, value) in [
            ("damage_type", serde_json::json!(13)),
            ("damage_source_type", serde_json::json!(2)),
            ("damage_per_mps", serde_json::json!(0.0)),
            ("catastrophic_damage_hp", serde_json::json!(-1.0)),
            ("minimum_velocity_mps", serde_json::json!(-0.5)),
            ("motion_freshness_ms", serde_json::json!(0)),
            ("minimum_motion_interval_ms", serde_json::json!(0.0)),
            ("application_id", serde_json::json!(20230)),
            ("provenance", serde_json::json!("text")),
        ] {
            let mut document = serde_json::json!({
                "schema_version":1,"source_build":"retail","application_id":20231,
                "minimum_velocity_mps":4.0,"damage_per_mps":20.0,"catastrophic_damage_hp":250.0,
                "recent_exit_grace_ms":1500,"motion_freshness_ms":2500,
                "minimum_motion_interval_ms":40.0,"damage_type":5,"damage_source_type":3
            });
            assert!(Rules::from_json(&document.to_string()).is_ok());
            document[field] = value;
            assert!(Rules::from_json(&document.to_string()).is_err(), "{field}");
        }
        assert!(Rules::from_json(r#"{"schema_version":1}"#).is_err());
    }

    #[test]
    fn ordinary_collisions_hurt_only_above_the_minimum_closing_speed() {
        let rules = synthetic_rules();
        let kind = ReportKind::CharacterCollision;
        assert_eq!(rules.raw_damage(kind, 3.0, None), None);
        assert_eq!(rules.raw_damage(kind, 4.0, None), None);
        assert_eq!(rules.raw_damage(kind, 6.5, None), Some(50.0));
        // Friendly fire does not change the raw amount of an ordinary hit.
        assert_eq!(rules.raw_damage(kind, 6.5, Some(0.5)), Some(50.0));
        assert_eq!(rules.raw_damage(kind, f32::NAN, None), None);
        assert_eq!(rules.raw_damage(ReportKind::Stuck, 30.0, None), None);
    }

    #[test]
    fn catastrophic_collisions_pre_divide_by_friendly_fire_for_teammates() {
        let rules = synthetic_rules();
        let kind = ReportKind::Catastrophic;
        assert_eq!(rules.raw_damage(kind, 0.0, None), Some(250.0));
        assert_eq!(rules.raw_damage(kind, 0.0, Some(0.5)), Some(500.0));
        assert_eq!(rules.raw_damage(kind, 0.0, Some(1.0)), Some(250.0));
        // No friendly fire: the flat amount is later scaled to nothing.
        assert_eq!(rules.raw_damage(kind, 0.0, Some(0.0)), Some(250.0));
    }

    #[test]
    fn closing_speed_follows_the_vehicle_direction() {
        assert_eq!(closing_speed(None, None), 0.0);
        assert_eq!(closing_speed(Some([3.0, 0.0, 4.0]), None), 5.0);
        assert_eq!(closing_speed(None, Some([0.0, 0.0, 2.0])), 2.0);
        // Running away along the vehicle's path lowers the closing speed.
        assert_eq!(
            closing_speed(Some([10.0, 0.0, 0.0]), Some([3.0, 0.0, 0.0])),
            7.0
        );
        // Running into it raises it.
        assert_eq!(
            closing_speed(Some([10.0, 0.0, 0.0]), Some([-3.0, 0.0, 0.0])),
            13.0
        );
        // Crossing at a right angle leaves only the vehicle's speed.
        assert_eq!(
            closing_speed(Some([10.0, 0.0, 0.0]), Some([0.0, 0.0, 3.0])),
            10.0
        );
        // A faster character in the same direction gives a magnitude.
        assert_eq!(
            closing_speed(Some([2.0, 0.0, 0.0]), Some([5.0, 0.0, 0.0])),
            3.0
        );
    }

    #[test]
    fn velocity_uses_the_two_latest_samples_and_expires() {
        let rules = synthetic_rules();
        assert_eq!(
            rules.estimated_velocity(None, Some(at(1.0, 100)), 100),
            None
        );
        assert_eq!(
            rules.estimated_velocity(Some(at(0.0, 0)), Some(at(1.0, 100)), 150),
            Some([10.0, 0.0, 0.0])
        );
        // Close samples divide by the minimum interval, not their gap.
        assert_eq!(
            rules.estimated_velocity(Some(at(0.0, 90)), Some(at(0.4, 100)), 100),
            Some([10.0, 0.0, 0.0])
        );
        assert_eq!(
            rules.estimated_velocity(Some(at(0.0, 0)), Some(at(1.0, 100)), 2600),
            Some([10.0, 0.0, 0.0])
        );
        assert_eq!(
            rules.estimated_velocity(Some(at(0.0, 0)), Some(at(1.0, 100)), 2601),
            None
        );
        assert_eq!(
            rules.estimated_velocity(Some(at(0.0, 200)), Some(at(1.0, 100)), 200),
            None
        );
    }

    #[test]
    fn report_kinds_match_the_wire() {
        assert_eq!(ReportKind::from_wire(0), Some(ReportKind::Stuck));
        assert_eq!(
            ReportKind::from_wire(1),
            Some(ReportKind::CharacterCollision)
        );
        assert_eq!(ReportKind::from_wire(2), Some(ReportKind::Catastrophic));
        assert_eq!(ReportKind::from_wire(3), None);
    }
}
