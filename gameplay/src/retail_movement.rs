//! Default retail player speed/stamina budget, independent of pathfinding.
//!
//! Public embedded metadata is synthetic; runtime movement rules remain in this module.
//! No skill, leadership, armor, animation or native AI speed bonus is applied.
//! `advance` uses the minimum directional cap while facing is unknown. A caller
//! may use `advance_direction` only when its published facing is established.
//! Advance a candidate Copy over actual simulated time and commit it with the
//! accepted movement, preserving stamina across commands within the same life.

pub const CATALOG_JSON: &str = include_str!("../data/synthetic-movement-metadata.json");
pub const WALK_FORWARD_MPS: f64 = 4.5;
pub const WALK_SIDE_MPS: f64 = 4.0;
pub const WALK_BACKWARD_MPS: f64 = 2.7;
pub const SPRINT_FORWARD_MPS: f64 = 6.0;
pub const SPRINT_SIDE_MPS: f64 = 5.5;
pub const SPRINT_BACKWARD_MPS: f64 = 5.0;
pub const STAMINA_MAX: f64 = 1.0;
pub const STAMINA_DRAIN_PER_SECOND: f64 = 0.1;
pub const STAMINA_RECOVERY_PER_SECOND: f64 = 0.1;
pub const SPRINT_ENTRY_THRESHOLD: f64 = 0.15;

/// Original retail yaw basis: 0=+Z, 256=+X, 512=-Z, 768=-X.
/// Matches RadiansToPackedYaw's f32 divide/multiply, truncation and low10 mask.
/// The caller must publish this yaw with the corresponding movement pose.
pub fn facing_yaw(dx: f64, dz: f64) -> Result<u16, &'static str> {
    if !dx.is_finite() || !dz.is_finite() || dx.hypot(dz) <= f64::EPSILON {
        return Err("invalid_movement_heading");
    }
    let radians = dx.atan2(dz) as f32;
    let packed = ((radians / std::f32::consts::TAU) * 1024.0).trunc() as i32;
    Ok((packed & 1023) as u16)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MovementIntent {
    Idle,
    Walk,
    Sprint,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MovementDirection {
    /// Minimum of normal directional caps, safe with unknown wire facing.
    Unknown,
    Forward,
    Sideways,
    Backward,
}

impl MovementDirection {
    pub fn caps(self) -> (f64, f64) {
        match self {
            Self::Forward => (WALK_FORWARD_MPS, SPRINT_FORWARD_MPS),
            Self::Sideways => (WALK_SIDE_MPS, SPRINT_SIDE_MPS),
            Self::Unknown | Self::Backward => (WALK_BACKWARD_MPS, SPRINT_BACKWARD_MPS),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MovementState {
    stamina: f64,
    sprinting: bool,
}

impl Default for MovementState {
    fn default() -> Self {
        // Original retail constructor initializes Character+0x738 to zero.
        // This is conservative; later native spawn reset semantics are unknown.
        Self {
            stamina: 0.0,
            sprinting: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MovementBudget {
    pub walk_seconds: f64,
    pub sprint_seconds: f64,
    pub distance_cap_m: f64,
    pub stamina_end: f64,
    pub sprinting_end: bool,
}

impl MovementState {
    pub fn new(stamina: f64) -> Result<Self, &'static str> {
        if !stamina.is_finite() || !(0.0..=STAMINA_MAX).contains(&stamina) {
            return Err("invalid_movement_stamina");
        }
        Ok(Self {
            stamina,
            sprinting: false,
        })
    }

    pub fn stamina(self) -> f64 {
        self.stamina
    }
    pub fn sprinting(self) -> bool {
        self.sprinting
    }

    pub fn advance(
        &mut self,
        intent: MovementIntent,
        dt_seconds: f64,
    ) -> Result<MovementBudget, &'static str> {
        self.advance_direction(intent, dt_seconds, MovementDirection::Unknown)
    }

    pub fn advance_direction(
        &mut self,
        intent: MovementIntent,
        dt_seconds: f64,
        direction: MovementDirection,
    ) -> Result<MovementBudget, &'static str> {
        if !dt_seconds.is_finite() || !(0.0..=60.0).contains(&dt_seconds) {
            return Err("invalid_movement_elapsed_time");
        }
        if dt_seconds == 0.0 {
            return Ok(MovementBudget {
                walk_seconds: 0.0,
                sprint_seconds: 0.0,
                distance_cap_m: 0.0,
                stamina_end: self.stamina,
                sprinting_end: self.sprinting,
            });
        }
        let can_sprint = intent == MovementIntent::Sprint
            && if self.sprinting {
                self.stamina > 0.0
            } else {
                self.stamina > SPRINT_ENTRY_THRESHOLD
            };
        let sprint_seconds = if can_sprint {
            dt_seconds.min(self.stamina / STAMINA_DRAIN_PER_SECOND)
        } else {
            0.0
        };
        let recovery_seconds = dt_seconds - sprint_seconds;
        let walk_seconds = if intent == MovementIntent::Idle {
            0.0
        } else {
            recovery_seconds
        };
        self.stamina = (self.stamina - sprint_seconds * STAMINA_DRAIN_PER_SECOND
            + recovery_seconds * STAMINA_RECOVERY_PER_SECOND)
            .clamp(0.0, STAMINA_MAX);
        // Do not restart sprint within the same tick after exhaustion. The caller
        // gets an exact sprint/walk split instead of a full tick at sprint speed.
        self.sprinting = can_sprint && sprint_seconds == dt_seconds && self.stamina > 0.0;
        let (walk, sprint) = direction.caps();
        Ok(MovementBudget {
            walk_seconds,
            sprint_seconds,
            distance_cap_m: walk_seconds * walk + sprint_seconds * sprint,
            stamina_end: self.stamina,
            sprinting_end: self.sprinting,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-10, "{a} != {b}");
    }
    #[test]
    fn original_yaw_cardinals_wrap_and_diagonals() {
        for (x, z, expected) in [
            (0.0, 1.0, 0),
            (1.0, 0.0, 256),
            (0.0, -1.0, 512),
            (-1.0, 0.0, 768),
            (1.0, 1.0, 128),
            (-1.0, 1.0, 896),
        ] {
            assert_eq!(facing_yaw(x, z), Ok(expected));
        }
        for (x, z) in [(0.0, 0.0), (f64::NAN, 1.0), (1.0, f64::INFINITY)] {
            assert!(facing_yaw(x, z).is_err());
        }
        assert!(facing_yaw(-0.00001, 1.0).unwrap() <= 1023);
    }
    #[test]
    fn initial_empty_recovers_and_requires_strict_entry_threshold() {
        let mut state = MovementState::default();
        let budget = state.advance(MovementIntent::Sprint, 1.0).unwrap();
        close(budget.sprint_seconds, 0.0);
        close(state.stamina(), 0.1);
        let mut exact = MovementState::new(0.15).unwrap();
        close(
            exact
                .advance(MovementIntent::Sprint, 0.25)
                .unwrap()
                .sprint_seconds,
            0.0,
        );
        assert!(
            exact
                .advance(MovementIntent::Sprint, 0.25)
                .unwrap()
                .sprint_seconds
                > 0.0
        );
    }
    #[test]
    fn exhaustion_splits_tick_and_recovers_without_invented_delay() {
        let mut state = MovementState::new(0.16).unwrap();
        let b = state
            .advance_direction(MovementIntent::Sprint, 2.0, MovementDirection::Forward)
            .unwrap();
        close(b.sprint_seconds, 1.6);
        close(b.walk_seconds, 0.4);
        close(b.distance_cap_m, 1.6 * 6.0 + 0.4 * 4.5);
        close(state.stamina(), 0.04);
        assert!(!state.sprinting());
        close(
            state
                .advance(MovementIntent::Idle, 0.5)
                .unwrap()
                .distance_cap_m,
            0.0,
        );
        close(state.stamina(), 0.09);
    }
    #[test]
    fn continued_sprint_below_entry_threshold_stops_at_zero() {
        let mut state = MovementState::new(0.2).unwrap();
        state.advance(MovementIntent::Sprint, 1.0).unwrap();
        assert!(state.sprinting());
        close(state.stamina(), 0.1);
        let b = state.advance(MovementIntent::Sprint, 1.0).unwrap();
        close(b.sprint_seconds, 1.0);
        close(state.stamina(), 0.0);
        assert!(!state.sprinting());
        close(
            state
                .advance(MovementIntent::Sprint, 0.25)
                .unwrap()
                .sprint_seconds,
            0.0,
        );
    }
    #[test]
    fn directional_caps_and_unknown_facing_are_bounded() {
        for (d, w, s) in [
            (MovementDirection::Unknown, 2.7, 5.0),
            (MovementDirection::Forward, 4.5, 6.0),
            (MovementDirection::Sideways, 4.0, 5.5),
            (MovementDirection::Backward, 2.7, 5.0),
        ] {
            let mut state = MovementState::new(1.0).unwrap();
            close(
                state
                    .advance_direction(MovementIntent::Walk, 0.25, d)
                    .unwrap()
                    .distance_cap_m,
                w * 0.25,
            );
            close(
                state
                    .advance_direction(MovementIntent::Sprint, 0.25, d)
                    .unwrap()
                    .distance_cap_m,
                s * 0.25,
            );
        }
    }
    #[test]
    fn recovery_clamps_and_candidate_copy_is_independent() {
        let original = MovementState::new(1.0).unwrap();
        let mut candidate = original;
        candidate.advance(MovementIntent::Sprint, 1.0).unwrap();
        close(original.stamina(), 1.0);
        candidate.advance(MovementIntent::Walk, 60.0).unwrap();
        close(candidate.stamina(), 1.0);
        assert!(!candidate.sprinting());
    }
    #[test]
    fn invalid_elapsed_and_stamina_cannot_mutate_state() {
        let mut state = MovementState::default();
        for dt in [-1.0, f64::NAN, f64::INFINITY, 60.01] {
            assert!(state.advance(MovementIntent::Sprint, dt).is_err());
            assert_eq!(state, MovementState::default());
        }
        for amount in [-0.01, 1.01, f64::NAN] {
            assert!(MovementState::new(amount).is_err());
        }
    }
}
