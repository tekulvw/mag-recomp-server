//! Bounded family-2 helicopter flight from authored retail spline data.

use crate::replication::CharacterPosition;
use serde::Deserialize;

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Knot {
    pub position_xyz: [f32; 3],
    pub normalized_parameter: f32,
    pub tangent_xyz: [f32; 3],
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoredFlight {
    pub inbound_flight_seconds: f32,
    pub outbound_flight_seconds: f32,
    pub hover_position_delay_seconds: f32,
    pub drop_point_index: usize,
    pub knots: Vec<Knot>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum FlightState {
    Dormant = 0,
    Inbound = 2,
    Hover = 3,
    Outbound = 4,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FlightPose {
    pub position: CharacterPosition,
    pub forward: [f32; 3],
}

struct PathSample {
    pose: FlightPose,
    normalized_speed: f32,
    curvature: f32,
}

impl AuthoredFlight {
    pub fn validate(&self) -> Result<(), &'static str> {
        let count = self.knots.len();
        if !(3..=128).contains(&count)
            || self.drop_point_index == 0
            || self.drop_point_index >= count - 1
            || ![self.inbound_flight_seconds, self.outbound_flight_seconds]
                .into_iter()
                .all(|n| n.is_finite() && n > 0.0 && n <= 60.0)
            || !(0.0..=60.0).contains(&self.hover_position_delay_seconds)
            || self.knots[0].normalized_parameter != 0.0
            || self.knots[count - 1].normalized_parameter != 1.0
            || !self
                .knots
                .windows(2)
                .all(|pair| pair[0].normalized_parameter < pair[1].normalized_parameter)
        {
            return Err("invalid_domination_helo_route");
        }
        for knot in &self.knots {
            if !knot
                .position_xyz
                .iter()
                .all(|value| value.is_finite() && value.abs() < 2048.0)
            {
                return Err("invalid_domination_helo_position");
            }
            if !knot
                .tangent_xyz
                .iter()
                .all(|value| value.is_finite() && value.abs() < 2048.0)
            {
                return Err("invalid_domination_helo_tangent");
            }
        }
        Ok(())
    }

    /// The two halves share the authored drop knot and retain derivative vectors
    /// copied by the native subpath constructor.
    pub fn sample(
        &self,
        phase: FlightState,
        normalized_elapsed: f32,
    ) -> Result<FlightPose, &'static str> {
        self.sample_motion(phase, normalized_elapsed)
            .map(|sample| sample.pose)
    }

    fn sample_motion(
        &self,
        phase: FlightState,
        normalized_elapsed: f32,
    ) -> Result<PathSample, &'static str> {
        self.validate()?;
        if !(0.0..=1.0).contains(&normalized_elapsed) {
            return Err("invalid_domination_helo_path_time");
        }
        let split = self.drop_point_index;
        let (knots, time) = match phase {
            FlightState::Inbound => (&self.knots[..=split], normalized_elapsed),
            FlightState::Hover => (&self.knots[..=split], 1.0),
            FlightState::Outbound => (&self.knots[split..], normalized_elapsed),
            FlightState::Dormant => return Err("domination_helo_inactive"),
        };
        let first = knots[0].normalized_parameter;
        let last = knots[knots.len() - 1].normalized_parameter;
        let t = if time == 1.0 {
            last
        } else {
            (last - first).mul_add(time, first)
        };
        let right = knots
            .partition_point(|knot| knot.normalized_parameter <= t)
            .clamp(1, knots.len() - 1);
        let a = knots[right - 1];
        let b = knots[right];
        let u = (t - a.normalized_parameter) / (b.normalized_parameter - a.normalized_parameter);
        let segment_scale = (last - first) / (b.normalized_parameter - a.normalized_parameter);
        hermite(a, b, u, segment_scale)
    }
}

fn hermite(a: Knot, b: Knot, t: f32, segment_scale: f32) -> Result<PathSample, &'static str> {
    let t2 = t * t;
    let t3 = t2 * t;
    let h00 = 2.0 * t3 - 3.0 * t2 + 1.0;
    let h01 = -2.0 * t3 + 3.0 * t2;
    let h10 = t3 - 2.0 * t2 + t;
    let h11 = t3 - t2;
    let point: [f32; 3] = std::array::from_fn(|i| {
        h00 * a.position_xyz[i]
            + h01 * b.position_xyz[i]
            + h10 * a.tangent_xyz[i]
            + h11 * b.tangent_xyz[i]
    });
    let rate: [f32; 3] = std::array::from_fn(|i| {
        (6.0 * t2 - 6.0 * t) * (a.position_xyz[i] - b.position_xyz[i])
            + (3.0 * t2 - 4.0 * t + 1.0) * a.tangent_xyz[i]
            + (3.0 * t2 - 2.0 * t) * b.tangent_xyz[i]
    });
    let norm = rate.iter().map(|value| value * value).sum::<f32>().sqrt();
    if !norm.is_finite() || norm < f32::EPSILON {
        return Err("domination_helo_heading_unresolved");
    }
    let [x, y, z] = point;
    let position = CharacterPosition { x, y, z };
    if !point
        .iter()
        .all(|value| value.is_finite() && value.abs() < 2048.0)
    {
        return Err("invalid_domination_helo_position");
    }
    let acceleration = std::array::from_fn(|i| {
        (12.0 * t - 6.0) * (a.position_xyz[i] - b.position_xyz[i])
            + (6.0 * t - 4.0) * a.tangent_xyz[i]
            + (6.0 * t - 2.0) * b.tangent_xyz[i]
    });
    let curvature = magnitude(cross(rate, acceleration)) / (norm * norm * norm);
    Ok(PathSample {
        pose: FlightPose {
            position,
            forward: rate.map(|value| value / norm),
        },
        normalized_speed: norm * segment_scale,
        curvature,
    })
}

pub fn quantize_path_time(normalized_elapsed: f32) -> Result<u8, &'static str> {
    if !(0.0..=1.0).contains(&normalized_elapsed) {
        return Err("invalid_domination_helo_path_time");
    }
    Ok(normalized_elapsed.mul_add(255.0, 0.5) as u8)
}

/// One fixed, native Helo clock update. The authoritative caller controls the
/// game update cadence and installs the resulting copies with rider changes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FlightClock {
    elapsed_seconds: f32,
    speed_factor: f32,
    braking: bool,
    accelerating: bool,
}

impl FlightClock {
    pub fn tick(
        &mut self,
        dt: f32,
        duration: f32,
        speed_up_seconds: f32,
        slow_down_seconds: f32,
    ) -> Result<f32, &'static str> {
        if !dt.is_finite()
            || dt < 0.0
            || !(0.0..=60.0).contains(&duration)
            || duration == 0.0
            || !(0.0..=duration).contains(&speed_up_seconds)
            || !(0.0..=duration).contains(&slow_down_seconds)
        {
            return Err("invalid_domination_helo_clock");
        }
        let candidate = (self.elapsed_seconds + dt).min(duration);
        self.accelerating = candidate < speed_up_seconds;
        self.braking = duration - candidate < slow_down_seconds;
        let speed = if self.accelerating {
            (candidate / speed_up_seconds).max(0.01)
        } else if self.braking {
            (1.0 - (candidate - (duration - slow_down_seconds)) / slow_down_seconds).max(0.01)
        } else {
            1.0
        };
        self.speed_factor = speed;
        self.elapsed_seconds = (self.elapsed_seconds + dt * speed).min(duration);
        Ok(self.elapsed_seconds / duration)
    }
}

#[derive(Clone, Debug)]
pub struct HeloCreation {
    pub base: crate::replication::DefensiveEmplacementCreation,
    pub flight_state: FlightState,
    pub path_elapsed: u8,
}

impl HeloCreation {
    pub fn encode(
        &self,
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        Self::encode_list(std::slice::from_ref(self), profile)
    }

    pub(crate) fn encode_list(
        carriers: &[Self],
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        use crate::replication::{BitWriter, UpdateHeader};
        let first = carriers.first().ok_or("missing_domination_helo_creation")?;
        let sequence = first.base.sequence;
        let timestamp_ms = first.base.timestamp_ms;
        let mut bits = BitWriter::with_profile(profile);
        if sequence <= 1 || carriers.len() > 8 {
            return Err("invalid_domination_helo_creation");
        }
        UpdateHeader {
            sequence,
            nak_sequence: 0,
            client_index: 0,
            object_type_mask: 1 << 2,
            timestamp_ms,
        }
        .encode(&mut bits)?;
        let mut seen = [false; 8];
        for carrier in carriers {
            let base = &carrier.base;
            if base.object_index >= 8
                || base.sequence != sequence
                || base.timestamp_ms != timestamp_ms
                || base.faction > 2
                || base.modifier >= 64
                || base.team > 1
                || base.spawn_point_index >= 128
                || std::mem::replace(&mut seen[usize::from(base.object_index)], true)
            {
                return Err("invalid_domination_helo_creation");
            }
            bits.write(1, 1)?;
            bits.write(base.object_index.into(), 16)?;
            base.encode_base_vehicle_fields(&mut bits)?;
            bits.write(1, 1)?;
            bits.write(carrier.path_elapsed.into(), 8)?;
            bits.write(1, 1)?;
            bits.write(carrier.flight_state as u32, 8)?;
        }
        bits.write(0, 1)?;
        Ok(bits.finish())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HeloUpdate {
    pub object_index: u16,
    pub path_elapsed: Option<u8>,
    pub flight_state: Option<FlightState>,
}

impl HeloUpdate {
    pub fn encode(
        self,
        sequence: u32,
        timestamp_ms: u32,
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        use crate::replication::{BitWriter, UpdateHeader};

        if sequence <= 1
            || self.object_index >= 8
            || (self.path_elapsed.is_none() && self.flight_state.is_none())
        {
            return Err("invalid_domination_helo_update");
        }
        let mut bits = BitWriter::with_profile(profile);
        UpdateHeader {
            sequence,
            nak_sequence: 0,
            client_index: 0,
            object_type_mask: 1 << 2,
            timestamp_ms,
        }
        .encode(&mut bits)?;
        bits.write(1, 1)?;
        bits.write(self.object_index.into(), 16)?;
        for _ in 0..25 {
            bits.write(0, 1)?;
        }
        for value in [
            self.path_elapsed,
            self.flight_state.map(|state| state as u8),
        ] {
            bits.write(u32::from(value.is_some()), 1)?;
            if let Some(value) = value {
                bits.write(value.into(), 8)?;
            }
        }
        bits.write(0, 1)?;
        Ok(bits.finish())
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FlightTiming {
    pub acceleration_seconds: f32,
    pub braking_seconds: f32,
}

#[derive(Clone, Debug)]
pub struct Helo {
    route: AuthoredFlight,
    timing: FlightTiming,
    clock: FlightClock,
    attitude: FlightAttitude,
    state: FlightState,
    wait_seconds: f32,
    hover_age_seconds: f32,
    retreat_requested: bool,
    restart_pending: bool,
    cycle_objective: Option<u16>,
    cycle_enabled: bool,
    seats: [Option<u16>; 16],
    passenger_start: u8,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FlightTick {
    Waiting,
    Moving {
        state: FlightState,
        elapsed: u8,
        pose: FlightPose,
    },
    DisembarkRequired {
        pose: FlightPose,
    },
    CycleEnded,
}

impl Helo {
    pub fn fresh(
        route: AuthoredFlight,
        timing: FlightTiming,
        start_delay_max: f32,
        native_random_30bit: u32,
        passenger_start: u8,
    ) -> Result<Self, &'static str> {
        route.validate()?;
        if !matches!(passenger_start, 1 | 2)
            || native_random_30bit >= 1 << 30
            || !(0.0..=60.0).contains(&start_delay_max)
        {
            return Err("invalid_domination_helo_inputs");
        }
        FlightClock::default().tick(
            0.0,
            route.inbound_flight_seconds,
            0.0,
            timing.braking_seconds,
        )?;
        FlightClock::default().tick(
            0.0,
            route.outbound_flight_seconds,
            timing.acceleration_seconds,
            0.0,
        )?;
        Ok(Self {
            route,
            timing,
            clock: FlightClock::default(),
            attitude: FlightAttitude::default(),
            state: FlightState::Dormant,
            wait_seconds: (native_random_30bit as f32 * f32::from_bits(0x30800000))
                * start_delay_max,
            hover_age_seconds: 0.0,
            retreat_requested: false,
            restart_pending: false,
            cycle_objective: None,
            cycle_enabled: false,
            seats: [None; 16],
            passenger_start,
        })
    }

    pub fn set_cycle(&mut self, enabled: bool, objective: Option<u16>) {
        self.cycle_enabled = enabled;
        self.cycle_objective = objective;
    }

    pub fn reschedule(&mut self, delay_seconds: f32) -> Result<(), &'static str> {
        if self.state != FlightState::Dormant || !(0.0..=120.0).contains(&delay_seconds) {
            return Err("invalid_domination_helo_restart");
        }
        self.wait_seconds = delay_seconds;
        self.restart_pending = false;
        Ok(())
    }

    pub(crate) fn reset_destroyed_flight(&mut self) {
        self.state = FlightState::Dormant;
        self.clock = FlightClock::default();
        self.attitude = FlightAttitude::default();
        self.hover_age_seconds = 0.0;
        self.retreat_requested = false;
        self.restart_pending = true;
        self.seats = [None; 16];
    }

    pub fn cycle_objective(&self) -> Option<u16> {
        self.cycle_objective
    }

    /// The qualified low-health branch uses the same current-world
    /// disembark transition as the elapsed disabled-FSP fallback.
    pub(crate) fn request_retreat_if_hovering(&mut self) {
        if self.state == FlightState::Hover {
            self.retreat_requested = true;
        }
    }

    fn hover_disembark_due(&self) -> bool {
        self.retreat_requested || (!self.cycle_enabled && self.hover_age_seconds > 1.5)
    }

    pub fn advance(&mut self, dt: f32) -> Result<FlightTick, &'static str> {
        if !dt.is_finite() || dt < 0.0 {
            return Err("invalid_domination_helo_tick");
        }
        if self.state == FlightState::Dormant {
            if self.restart_pending || !self.cycle_enabled {
                return Ok(FlightTick::Waiting);
            }
            self.wait_seconds = (self.wait_seconds - dt).max(0.0);
            if self.wait_seconds > 0.0 {
                return Ok(FlightTick::Waiting);
            }
            self.clock = FlightClock::default();
            self.state = FlightState::Inbound;
            self.attitude = FlightAttitude::default();
            return self.moving(0.0, dt);
        }
        if self.state == FlightState::Hover {
            self.hover_age_seconds += dt;
            let pose = self.route.sample(FlightState::Hover, 1.0)?;
            // The general FSP flag stays set for a healthy active carrier.
            // The inherited disable hook clears it, enabling the strict
            // phase-age exit gate. A qualified damaged retreat shares that exit.
            if self.hover_disembark_due() {
                return Ok(FlightTick::DisembarkRequired { pose });
            }
            return Ok(FlightTick::Moving {
                state: FlightState::Hover,
                elapsed: 255,
                pose,
            });
        }
        let (duration, speed_up, slow_down) = if self.state == FlightState::Inbound {
            (
                self.route.inbound_flight_seconds,
                0.0,
                self.timing.braking_seconds,
            )
        } else {
            (
                self.route.outbound_flight_seconds,
                self.timing.acceleration_seconds,
                0.0,
            )
        };
        let elapsed = self.clock.tick(dt, duration, speed_up, slow_down)?;
        let update = self.moving(elapsed, dt)?;
        if elapsed == 1.0 {
            if self.state == FlightState::Inbound {
                self.state = FlightState::Hover;
                self.hover_age_seconds = 0.0;
                self.retreat_requested = false;
                return Ok(FlightTick::Moving {
                    state: FlightState::Hover,
                    elapsed: 255,
                    pose: self.route.sample(FlightState::Hover, 1.0)?,
                });
            }
            self.state = FlightState::Dormant;
            // The caller must apply the source respawn delay before the next cycle.
            self.restart_pending = true;
            return Ok(FlightTick::CycleEnded);
        }
        Ok(update)
    }

    fn moving(&mut self, elapsed: f32, dt: f32) -> Result<FlightTick, &'static str> {
        let sample = self.route.sample_motion(self.state, elapsed)?;
        let duration = if self.state == FlightState::Outbound {
            self.route.outbound_flight_seconds
        } else {
            self.route.inbound_flight_seconds
        };
        let lookahead = self
            .route
            .sample_motion(self.state, (elapsed + 1.5 / duration).min(1.0))?;
        self.attitude.step(&sample, &lookahead, self.clock, dt)?;
        Ok(FlightTick::Moving {
            state: self.state,
            elapsed: quantize_path_time(elapsed)?,
            pose: sample.pose,
        })
    }

    pub fn body_axes(&self) -> Option<([f32; 3], [f32; 3])> {
        self.attitude.yaw.map(|_| {
            let basis = self.body_basis();
            (basis.forward, basis.right)
        })
    }

    pub(crate) fn body_basis(&self) -> crate::domination_helo_engine::BodyBasis {
        self.attitude.body_basis()
    }

    /// Admit a previously qualified spawn-wave passenger to the native
    /// reverse seat search. Gunner seats require separate ownership authority.
    pub fn board_passenger(&mut self, actor: u16) -> Result<u8, &'static str> {
        if !matches!(self.state, FlightState::Inbound | FlightState::Hover)
            || !self.cycle_enabled
            || actor == u16::MAX
            || self.seats.contains(&Some(actor))
        {
            return Err("domination_helo_boarding_unavailable");
        }
        let seat = (usize::from(self.passenger_start)..self.seats.len())
            .rev()
            .find(|&seat| self.seats[seat].is_none())
            .ok_or("domination_helo_full")?;
        self.seats[seat] = Some(actor);
        Ok(seat as u8)
    }

    pub fn passengers(&self) -> impl Iterator<Item = (u8, u16)> + '_ {
        self.seats
            .iter()
            .enumerate()
            .filter_map(|(seat, actor)| actor.map(|id| (seat as u8, id)))
    }

    pub(crate) fn dismount_passenger(
        &mut self,
        seat: PassengerSeat,
        actor: u16,
    ) -> Result<(), &'static str> {
        let slot = self
            .seats
            .get_mut(usize::from(seat.seat))
            .filter(|slot| **slot == Some(actor))
            .ok_or("domination_helo_seat_not_owned")?;
        *slot = None;
        Ok(())
    }

    /// Called on the staged carrier only after the authoritative transaction
    /// stages character dismounts for the entire ascending seat list.
    pub fn depart_after_disembark(
        &mut self,
        disembarked: &[(u8, u16)],
    ) -> Result<(), &'static str> {
        if self.state != FlightState::Hover
            || !self.hover_disembark_due()
            || !self.passengers().eq(disembarked.iter().copied())
        {
            return Err("incomplete_domination_helo_disembark");
        }
        self.seats.fill(None);
        self.clock = FlightClock::default();
        self.state = FlightState::Outbound;
        Ok(())
    }
}

/// Ground test for a current live class-5 disembark candidate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DisembarkGround {
    candidate: CharacterPosition,
}

impl DisembarkGround {
    pub fn new(candidate: CharacterPosition) -> Result<Self, &'static str> {
        candidate.validate()?;
        Ok(Self { candidate })
    }

    pub fn ray(self) -> ([f64; 3], [f64; 3]) {
        let CharacterPosition { x, y, z } = self.candidate;
        (
            [x.into(), (y - 3.0).into(), z.into()],
            [x.into(), (y - 50.0).into(), z.into()],
        )
    }

    pub fn settled(self, floor_y: f32) -> Option<(CharacterPosition, [f64; 3])> {
        let ([_, start, _], [_, end, _]) = self.ray();
        if !((end as f32)..=(start as f32)).contains(&floor_y)
            || (floor_y - self.candidate.y).abs() > 20.0
        {
            return None;
        }
        let position = CharacterPosition {
            y: floor_y + 0.1,
            ..self.candidate
        };
        position.validate().ok()?;
        let clearance_center = [
            position.x.into(),
            (floor_y + f32::from_bits(0x3ee83826)).into(),
            position.z.into(),
        ];
        Some((position, clearance_center))
    }
}

/// The native unsigned float packer writes zero integer and fractional bits
/// for a negative coordinate. Keep that representation separate from the
/// actual mounted position used by authoritative world queries.
pub(crate) fn mounted_wire_position(position: CharacterPosition) -> CharacterPosition {
    CharacterPosition {
        x: position.x.max(0.0),
        y: position.y,
        z: position.z.max(0.0),
    }
}

/// The caller supplies the current body orientation and performs the native
/// current-world terrain, clearance and occupancy tests for each candidate.
pub fn settle_disembark(
    position: CharacterPosition,
    forward: [f32; 3],
    right: [f32; 3],
    mut settle: impl FnMut(CharacterPosition) -> Result<Option<CharacterPosition>, &'static str>,
) -> Result<CharacterPosition, &'static str> {
    settle_disembark_with_fallback(position, forward, right, || None, &mut settle)
}

pub(crate) fn settle_disembark_with_fallback(
    position: CharacterPosition,
    forward: [f32; 3],
    right: [f32; 3],
    fallback: impl FnOnce() -> Option<[f32; 2]>,
    mut settle: impl FnMut(CharacterPosition) -> Result<Option<CharacterPosition>, &'static str>,
) -> Result<CharacterPosition, &'static str> {
    position.validate()?;
    for axis in [forward, right] {
        if (axis.iter().map(|v| v * v).sum::<f32>() - 1.0).abs() >= 0.001
            || axis.iter().any(|v| !v.is_finite())
        {
            return Err("invalid_domination_helo_body_basis");
        }
    }
    for row in 0..=3 {
        for lateral in [-1.1, 1.1] {
            let backward = -2.0 * row as f32;
            let [x, y, z] = std::array::from_fn(|axis| {
                [position.x, position.y, position.z][axis]
                    + forward[axis] * backward
                    + right[axis] * lateral
            });
            let candidate = CharacterPosition { x, y, z };
            if let Some(feet) = settle(candidate)? {
                feet.validate()?;
                return Ok(feet);
            }
        }
    }
    if let Some([x, z]) = fallback()
        && let Some(feet) = settle(CharacterPosition {
            x: position.x + x,
            y: position.y,
            z: position.z + z,
        })?
    {
        feet.validate()?;
        return Ok(feet);
    }
    Err("domination_helo_disembark_unresolved")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        replication::{BitReader, ObjectUpdate},
        wire_profile::WireProfile,
    };

    fn route() -> AuthoredFlight {
        AuthoredFlight {
            inbound_flight_seconds: 20.0,
            outbound_flight_seconds: 15.0,
            hover_position_delay_seconds: 0.5,
            drop_point_index: 1,
            knots: vec![
                Knot {
                    position_xyz: [20.0, 10.0, 20.0],
                    normalized_parameter: 0.0,
                    tangent_xyz: [0.0, 0.0, 10.0],
                },
                Knot {
                    position_xyz: [20.0, 10.0, 30.0],
                    normalized_parameter: 0.3,
                    tangent_xyz: [0.0, 0.0, 10.0],
                },
                Knot {
                    position_xyz: [20.0, 10.0, 40.0],
                    normalized_parameter: 1.0,
                    tangent_xyz: [0.0, 0.0, 10.0],
                },
            ],
        }
    }

    #[test]
    fn route_halves_share_drop_pose_and_clock_brakes_and_accelerates() {
        let route = route();
        assert_eq!(
            route.sample(FlightState::Inbound, 1.0),
            route.sample(FlightState::Outbound, 0.0)
        );
        assert_eq!(
            route.sample(FlightState::Inbound, 0.5).unwrap().position.z,
            25.0
        );
        assert_eq!(
            route.sample(FlightState::Outbound, 0.5).unwrap().position.z,
            35.0
        );
        let mut clock = FlightClock::default();
        assert_eq!(clock.tick(1.0, 20.0, 2.0, 0.0).unwrap(), 0.025);
        assert_eq!(clock.tick(18.0, 20.0, 0.0, 0.0).unwrap(), 0.925);
        assert!(clock.tick(1.0, 20.0, 0.0, 2.0).unwrap() < 0.95);
    }

    #[test]
    fn departure_requires_every_staged_passenger_and_settled_feet() {
        let timing = FlightTiming {
            acceleration_seconds: 1.7,
            braking_seconds: 1.75,
        };
        let mut helo = Helo::fresh(route(), timing, 5.0, 0, 2).unwrap();
        assert_eq!(helo.advance(0.0), Ok(FlightTick::Waiting));
        helo.set_cycle(true, Some(9));
        assert!(matches!(
            helo.advance(0.0),
            Ok(FlightTick::Moving {
                state: FlightState::Inbound,
                ..
            })
        ));
        assert_eq!(helo.board_passenger(4), Ok(15));
        assert_eq!(helo.board_passenger(5), Ok(14));
        let reached_hover = (0..1000).any(|_| {
            matches!(
                helo.advance(0.05).unwrap(),
                FlightTick::Moving {
                    state: FlightState::Hover,
                    ..
                }
            )
        });
        assert!(reached_hover);
        assert!(matches!(
            helo.advance(1.5).unwrap(),
            FlightTick::Moving {
                state: FlightState::Hover,
                ..
            }
        ));
        helo.set_cycle(false, Some(9));
        assert!(matches!(
            helo.advance(0.0).unwrap(),
            FlightTick::Moving {
                state: FlightState::Hover,
                ..
            }
        ));
        let pose = (0..1000)
            .find_map(|_| match helo.advance(0.05).unwrap() {
                FlightTick::DisembarkRequired { pose } => Some(pose),
                _ => None,
            })
            .expect("bounded test flight reaches its drop point");
        assert_eq!(
            helo.depart_after_disembark(&[(15, 4)]),
            Err("incomplete_domination_helo_disembark")
        );
        let feet = settle_disembark(
            pose.position,
            [0.0, 0.0, 1.0],
            [1.0, 0.0, 0.0],
            |candidate| {
                Ok(
                    (candidate.z < 29.0 && candidate.x > 20.0).then_some(CharacterPosition {
                        y: 7.0,
                        ..candidate
                    }),
                )
            },
        )
        .unwrap();
        assert_eq!(
            feet,
            CharacterPosition {
                x: 21.1,
                y: 7.0,
                z: 28.0
            }
        );
        assert_eq!(helo.depart_after_disembark(&[(14, 5), (15, 4)]), Ok(()));
        assert!(matches!(
            helo.advance(0.0),
            Ok(FlightTick::Moving {
                state: FlightState::Outbound,
                ..
            })
        ));
    }

    #[test]
    fn helo_random_exit_runs_after_the_authored_eight_and_still_requires_clear_feet() {
        let position = CharacterPosition {
            x: 20.0,
            y: 30.0,
            z: 40.0,
        };
        let mut count = 0;
        let feet = settle_disembark_with_fallback(
            position,
            [0., 0., 1.],
            [1., 0., 0.],
            || Some([3.6, -3.2]),
            |candidate| {
                count += 1;
                if count == 9 {
                    Ok(Some(CharacterPosition {
                        y: 25.0,
                        ..candidate
                    }))
                } else {
                    Ok(None)
                }
            },
        )
        .unwrap();
        assert_eq!(count, 9);
        assert_eq!(
            feet,
            CharacterPosition {
                x: 23.6,
                y: 25.0,
                z: 36.8
            }
        );
        assert_eq!(
            settle_disembark_with_fallback(
                position,
                [0., 0., 1.],
                [1., 0., 0.],
                || Some([3.6, -3.2]),
                |_| Ok(None)
            ),
            Err("domination_helo_disembark_unresolved")
        );
    }

    #[test]
    fn native_path_and_state_use_two_appended_bytes() {
        let profile = WireProfile::prototype();
        let payload = HeloUpdate {
            object_index: 3,
            path_elapsed: Some(128),
            flight_state: Some(FlightState::Inbound),
        }
        .encode(2, 42, profile)
        .unwrap();
        let mut bits = BitReader::with_profile(&payload, profile);
        bits.position = 128 + profile.object_type_bits();
        assert_eq!(
            ObjectUpdate::decode_with_profile(&payload, profile)
                .unwrap()
                .header
                .object_type_mask,
            1 << 2
        );
        assert_eq!(bits.read(1).unwrap(), 1);
        assert_eq!(bits.read(16).unwrap(), 3);
        assert_eq!(bits.read(25).unwrap(), 0);
        assert_eq!(bits.read(1).unwrap(), 1);
        assert_eq!(bits.read(8).unwrap(), 128);
        assert_eq!(bits.read(1).unwrap(), 1);
        assert_eq!(bits.read(8).unwrap(), 2);
        assert_eq!(bits.read(1).unwrap(), 0);
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FleetSource {
    schema: String,
    map: String,
    spawn_catalog_sha256: String,
    pub source_receipt_sha256: [String; 3],
    pub carriers: Vec<CarrierSource>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CarrierSource {
    pub object_index: u16,
    pub spawn_index: u16,
    pub spawn_token: u32,
    pub vehicle_type_token: u32,
    pub health_quarter_points: u16,
    pub faction: u8,
    pub passenger_start: u8,
    pub squads: [u8; 2],
    pub start_delay_max_seconds: f32,
    pub route: AuthoredFlight,
    pub acceleration_seconds: f32,
    pub braking_seconds: f32,
}

impl FleetSource {
    pub fn load(
        bytes: &[u8],
        spawns: &crate::spawn_catalog::SpawnCatalog,
    ) -> Result<Self, &'static str> {
        const MAX_SOURCE_BYTES: usize = 256 * 1024;
        if bytes.len() > MAX_SOURCE_BYTES {
            return Err("domination_helo_source_too_large");
        }
        let source: Self =
            serde_json::from_slice(bytes).map_err(|_| "invalid_domination_helo_source")?;
        let policy = spawns.match_profile();
        if source.schema != "mag_domination_helo_fleet_v1"
            || spawns.map_name() != source.map
            || spawns.source_sha256() != Some(source.spawn_catalog_sha256.as_str())
            || !policy.auto_assign_teams
            || policy.game_type != 2
            || source.carriers.len() != 4
            || !source
                .source_receipt_sha256
                .iter()
                .all(|hash| hash.len() == 64 && hash.as_bytes().iter().all(u8::is_ascii_hexdigit))
        {
            return Err("unbound_domination_helo_source");
        }
        let mut indices = std::collections::BTreeSet::new();
        for (index, carrier) in source.carriers.iter().enumerate() {
            let point = spawns
                .point(carrier.spawn_index)
                .ok_or("unbound_domination_helo_point")?;
            if carrier.object_index != index as u16
                || !indices.insert(carrier.spawn_index)
                || carrier.spawn_token != point.token
                || point.active
                || carrier.vehicle_type_token == 0
                || !(1..4096).contains(&carrier.health_quarter_points)
                || carrier.faction != policy.attacking_faction
                || carrier.squads[0] >= carrier.squads[1]
                || carrier.squads[1] >= 16
                || !matches!(point.kind, crate::spawn_catalog::SpawnKind::Unsupported)
            {
                return Err("domination_helo_point_mismatch");
            }
            carrier.fresh(0)?;
        }
        Ok(source)
    }
}

impl CarrierSource {
    pub fn fresh(&self, native_random_30bit: u32) -> Result<Helo, &'static str> {
        Helo::fresh(
            self.route.clone(),
            FlightTiming {
                acceleration_seconds: self.acceleration_seconds,
                braking_seconds: self.braking_seconds,
            },
            self.start_delay_max_seconds,
            native_random_30bit,
            self.passenger_start,
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PassengerSeat {
    pub(crate) object_index: u16,
    pub(crate) seat: u8,
}

impl PassengerSeat {
    pub(crate) fn character_state(self) -> crate::replication::CharacterState {
        crate::replication::CharacterState {
            state: 14,
            stance: 0,
            user_data: [2, self.object_index as u8, self.seat],
        }
    }
}

#[derive(Clone)]
pub(crate) struct FleetRuntime {
    pub(crate) carriers: [Carrier; 4],
    pub(crate) last_tick: Option<std::time::Duration>,
    pub(crate) random: FlightRandom,
    initial_random: FlightRandom,
}

#[derive(Clone)]
pub(crate) struct Carrier {
    pub(crate) source: CarrierSource,
    pub(crate) damage: crate::domination_helo_damage::HullDamage,
    pub(crate) respawn_wave: Option<u32>,
    pub(crate) spawn_cycle_age: f32,
    pub(crate) observed_wave: Option<u32>,
    pub(crate) motion: Helo,
    initial_motion: Helo,
    pub(crate) current_pose: Option<FlightPose>,
    pub(crate) path_elapsed: u8,
    pub(crate) published_state: FlightState,
    pub(crate) live_basis: Option<crate::domination_helo_engine::BodyBasis>,
}

impl FleetRuntime {
    pub(crate) fn from_source(source: FleetSource, random_seed: u32) -> Result<Self, &'static str> {
        if source.carriers.len() != 4 {
            return Err("invalid_domination_helo_fleet");
        }
        let mut random = FlightRandom::from_seed(random_seed);
        let carriers = source
            .carriers
            .into_iter()
            .map(|source| {
                let motion = source.fresh(random.next_30bit())?;
                Ok(Carrier {
                    damage: crate::domination_helo_damage::HullDamage::fresh(
                        source.health_quarter_points,
                    ),
                    source,
                    respawn_wave: None,
                    spawn_cycle_age: 0.,
                    observed_wave: None,
                    initial_motion: motion.clone(),
                    motion,
                    current_pose: None,
                    path_elapsed: 0,
                    published_state: FlightState::Dormant,
                    live_basis: None,
                })
            })
            .collect::<Result<Vec<_>, &'static str>>()?
            .try_into()
            .map_err(|_| "invalid_domination_helo_fleet")?;
        Ok(Self {
            carriers,
            last_tick: None,
            random: random.clone(),
            initial_random: random,
        })
    }

    pub(crate) fn reset(&mut self) {
        self.last_tick = None;
        self.random = self.initial_random.clone();
        for carrier in &mut self.carriers {
            carrier.motion = carrier.initial_motion.clone();
            carrier.respawn_wave = None;
            carrier.spawn_cycle_age = 0.;
            carrier.observed_wave = None;
            carrier.damage = crate::domination_helo_damage::HullDamage::fresh(
                carrier.source.health_quarter_points,
            );
            carrier.current_pose = None;
            carrier.path_elapsed = 0;
            carrier.published_state = FlightState::Dormant;
            carrier.live_basis = None;
        }
    }

    pub(crate) fn by_spawn_mut(&mut self, spawn: u16) -> Option<&mut Carrier> {
        self.carriers
            .iter_mut()
            .find(|carrier| carrier.source.spawn_index == spawn)
    }

    pub(crate) fn schedule_ended(&mut self) -> Result<(), &'static str> {
        for carrier in &mut self.carriers {
            if carrier.motion.restart_pending {
                let delay = (self.random.next_30bit() as f32 * f32::from_bits(0x30800000))
                    * carrier.source.start_delay_max_seconds;
                carrier.motion.reschedule(delay)?;
            }
        }
        Ok(())
    }

    pub(crate) fn remove_passenger(&mut self, actor: u16) {
        for carrier in &mut self.carriers {
            for seat in &mut carrier.motion.seats {
                if *seat == Some(actor) {
                    *seat = None;
                }
            }
        }
    }
}

impl Carrier {
    pub(crate) fn creation(&self, sequence: u32, timestamp_ms: u32) -> HeloCreation {
        use crate::replication::DefensiveEmplacementCreation;
        HeloCreation {
            base: DefensiveEmplacementCreation {
                sequence,
                timestamp_ms,
                object_index: self.source.object_index,
                owner_index: u16::MAX,
                vehicle_type_token: self.source.vehicle_type_token,
                faction: self.source.faction,
                modifier: 0,
                state: self.damage.state,
                turret_owners: [u16::MAX; 2],
                damage: self.damage.wire,
                team: 0,
                spawn_point_index: self.source.spawn_index,
            },
            flight_state: self.motion.state,
            path_elapsed: self.path_elapsed,
        }
    }
}

/// Scoped copy of the original 32-slot shuffled RNG for the Helo lifecycle.
/// The owning runtime supplies its game seed and stages this stream with the fleet.
#[derive(Clone)]
pub(crate) struct FlightRandom {
    lcg: u32,
    previous: u32,
    slots: [u32; 32],
}

impl FlightRandom {
    fn from_seed(mut seed: u32) -> Self {
        for _ in 0..8 {
            seed = Self::step(seed);
        }
        let slots = std::array::from_fn(|_| {
            seed = Self::step(seed);
            seed
        });
        Self {
            lcg: seed,
            previous: slots[31],
            slots,
        }
    }

    fn step(seed: u32) -> u32 {
        seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223)
    }

    pub(crate) fn disembark_offset(&mut self) -> [f32; 2] {
        std::array::from_fn(|_| (self.next_30bit() % 8000) as f32 / 1000.0 - 4.0)
    }

    fn next_30bit(&mut self) -> u32 {
        self.lcg = Self::step(self.lcg);
        let index = (self.previous & 31) as usize;
        self.previous = std::mem::replace(&mut self.slots[index], self.lcg);
        self.previous & ((1 << 30) - 1)
    }
}

fn cross([ax, ay, az]: [f32; 3], [bx, by, bz]: [f32; 3]) -> [f32; 3] {
    [ay * bz - az * by, az * bx - ax * bz, ax * by - ay * bx]
}

fn magnitude(vector: [f32; 3]) -> f32 {
    vector.iter().map(|value| value * value).sum::<f32>().sqrt()
}

/// The native controller accumulates each angle error over its five most
/// recent updates. Gains are constructor values for the Helo controller.
#[derive(Clone, Debug, Default)]
struct AngleWindow {
    correction: [f32; 5],
    cursor: usize,
    total: f32,
}

impl AngleWindow {
    fn update(&mut self, error: f32, dt: f32, proportional: f32, integral: f32) -> f32 {
        let previous = std::mem::replace(&mut self.correction[self.cursor], dt * error);
        let change = proportional.mul_add(error, integral * self.total);
        self.total = dt * error + (self.total - previous);
        self.cursor = (self.cursor + 1) % self.correction.len();
        if error < 0.0 {
            change.max(error)
        } else {
            change.min(error)
        }
    }
}

#[derive(Clone, Debug, Default)]
struct FlightAttitude {
    yaw: Option<f32>,
    pitch: f32,
    roll: f32,
    yaw_window: AngleWindow,
    pitch_window: AngleWindow,
    roll_window: AngleWindow,
}

impl FlightAttitude {
    fn step(
        &mut self,
        sample: &PathSample,
        lookahead: &PathSample,
        clock: FlightClock,
        dt: f32,
    ) -> Result<(), &'static str> {
        let target_yaw = heading(sample.pose.forward)?;
        let current_yaw = self.yaw.unwrap_or(target_yaw);
        let yaw = current_yaw
            + self
                .yaw_window
                .update(angle_delta(target_yaw, current_yaw), dt, 0.05, 0.8);
        self.yaw = Some(yaw.rem_euclid(std::f32::consts::TAU));

        let lookahead_yaw = heading(lookahead.pose.forward)?;
        let roll_sign = if angle_delta(lookahead_yaw, yaw) > 0.0 {
            -1.0
        } else {
            1.0
        };
        let roll =
            roll_sign * 0.8 * (lookahead.curvature * clock.speed_factor / 0.01).clamp(0.0, 1.0);
        self.roll += self.roll_window.update(roll - self.roll, dt, 0.01, 0.1);

        let speed = if clock.braking {
            sample.normalized_speed * clock.speed_factor
        } else {
            sample.normalized_speed
        };
        let pitch = 0.17 * (speed / 850.0).clamp(0.0, 1.0);
        let pitch = if clock.braking {
            -pitch * 5.0
        } else if clock.accelerating {
            (pitch / clock.speed_factor).min(0.35)
        } else {
            pitch
        };
        self.pitch += self.pitch_window.update(pitch - self.pitch, dt, 0.01, 0.3);
        Ok(())
    }

    fn body_basis(&self) -> crate::domination_helo_engine::BodyBasis {
        let (sy, cy) = self.yaw.unwrap_or_default().sin_cos();
        let (sr, cr) = self.roll.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        crate::domination_helo_engine::BodyBasis {
            right: [cy * cr, sr, -sy * cr],
            forward: [sy * cp + cy * sr * sp, -cr * sp, cy * cp - sy * sr * sp],
        }
    }
}

fn heading([x, _, z]: [f32; 3]) -> Result<f32, &'static str> {
    if x.abs().max(z.abs()) <= f32::EPSILON {
        return Err("domination_helo_heading_unresolved");
    }
    Ok(x.atan2(z))
}

fn angle_delta(target: f32, current: f32) -> f32 {
    (target - current + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU)
        - std::f32::consts::PI
}
