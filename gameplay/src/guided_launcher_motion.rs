//! Original steering algebra with bounded replacement substeps and per-flight RNG.
//! Native PRNG identity, SIMD rounding and Havok interleave are not claimed.
use serde::Deserialize;
type Point = [f64; 3];
const SUBSTEP_MS: u64 = 10;
const MAX_SUBSTEPS: usize = 1_201;
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Steering {
    pub guided_decision_min_seconds: f32,
    pub guided_decision_max_seconds: f32,
    pub guided_max_direction_change: f32,
    pub dumb_decision_min_seconds: f32,
    pub dumb_decision_max_seconds: f32,
    pub dumb_direction_change_min: f32,
    pub dumb_direction_change_max: f32,
    pub dumb_cruise_range_m: f32,
    pub dumb_max_range_m: f32,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Decision {
    pub heading: Point,
    pub bias: f64,
    pub lost: bool,
}
fn dot(a: Point, b: Point) -> f64 {
    a.into_iter().zip(b).map(|(x, y)| x * y).sum()
}
fn sub(a: Point, b: Point) -> Point {
    std::array::from_fn(|i| a[i] - b[i])
}
fn cross(a: Point, b: Point) -> Point {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn normalized(v: Point) -> Point {
    let length = dot(v, v).sqrt();
    if length > f32::MIN_POSITIVE as f64 {
        v.map(|x| x / length)
    } else {
        [0.; 3]
    }
}
fn guide(
    heading: Point,
    position: Point,
    anchor: Option<Point>,
    bias: f64,
    initial: f64,
    cap: f64,
) -> Decision {
    let Some(anchor) = anchor else {
        return Decision {
            heading,
            bias,
            lost: true,
        };
    };
    let mut delta = sub(anchor, position);
    delta[1] += bias;
    let distance = dot(delta, delta).sqrt();
    if distance <= f32::MIN_POSITIVE as f64 {
        return Decision {
            heading,
            bias,
            lost: false,
        };
    }
    let desired = normalized(delta);
    let forward = dot(heading, desired);
    if forward < 0. {
        return Decision {
            heading,
            bias,
            lost: true,
        };
    }
    let ratio = distance / initial;
    let next_bias = if ratio > 0. {
        (0.1 * ratio).max(0.5)
    } else {
        bias
    };
    let perp = std::array::from_fn(|i| desired[i] - heading[i] * forward);
    let out = if dot(perp, perp).sqrt() > cap {
        let unit = normalized(perp);
        normalized(std::array::from_fn(|i| {
            (1. - cap) * heading[i] + cap * unit[i]
        }))
    } else {
        desired
    };
    Decision {
        heading: out,
        bias: next_bias,
        lost: false,
    }
}
fn clock(timer: &mut f64, dt: f64, interval: impl FnOnce() -> f64) -> bool {
    if *timer < 0. {
        return false;
    }
    *timer -= dt;
    if *timer <= 0. {
        *timer = interval();
        true
    } else {
        false
    }
}
fn dumb(heading: Point, min: f64, max: f64, mut draw: impl FnMut() -> f64) -> Point {
    let mut transverse = cross(heading, [0., 1., 0.]);
    if dot(transverse, transverse) < 1e-10 {
        transverse = cross(heading, [0., 0., 1.]);
    }
    let first = normalized(cross(transverse, heading));
    let second = normalized(cross(heading, first));
    let sign1 = if draw() < 0.5 { -1. } else { 1. };
    let mag1 = min + (max - min) * draw();
    let sign2 = if draw() < 0.5 { -1. } else { 1. };
    let mag2 = min + (max - min) * draw();
    normalized(std::array::from_fn(|i| {
        heading[i] + first[i] * sign1 * mag1 + second[i] * sign2 * mag2
    }))
}
#[derive(Clone, Copy, Debug, PartialEq)]
struct Random(u32);
impl Random {
    fn draw(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        ((x & 0x3fff_ffff) as f32) * (1. / 1_073_741_824.)
    }
}
#[derive(Clone, Debug, PartialEq)]
enum Propulsion {
    Guided { initial: f64, bias: f64, lost: bool },
    Dumb { cruise: bool },
}
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Motion {
    origin: Point,
    position: Point,
    velocity: Point,
    heading: Point,
    launched: u64,
    advanced: u64,
    clock: f64,
    random: Random,
    kind: Propulsion,
    steering: Steering,
    speed: f64,
    range: f64,
    lifetime_ms: u64,
    armed_ms: u64,
}
pub(crate) enum Advance<T> {
    Flying(Motion),
    Terminal {
        position: Point,
        reason: Terminal<T>,
    },
}
pub(crate) enum Terminal<T> {
    Contact(T),
    Prearm,
    Age,
    Range,
}
impl Motion {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn launch(
        origin: Point,
        velocity: Point,
        heading: Point,
        now: u64,
        steering: Steering,
        speed: f32,
        range: f32,
        lifetime_ms: u64,
        armed_ms: u64,
        target: Option<Point>,
        seed: u32,
    ) -> Result<Self, &'static str> {
        if !origin
            .into_iter()
            .chain(velocity)
            .chain(heading)
            .all(f64::is_finite)
            || dot(heading, heading) < 0.99
            || lifetime_ms == 0
            || armed_ms > lifetime_ms
        {
            return Err("invalid_guided_motion_launch");
        }
        let kind = if let Some(anchor) = target {
            let delta = sub(anchor, origin);
            let initial = dot(delta, delta).sqrt();
            if !initial.is_finite() || initial <= 0. {
                return Err("guided_degenerate_target_anchor");
            }
            Propulsion::Guided {
                initial,
                bias: initial * 0.1,
                lost: false,
            }
        } else {
            Propulsion::Dumb { cruise: true }
        };
        let mut motion = Self {
            origin,
            position: origin,
            velocity,
            heading: normalized(heading),
            launched: now,
            advanced: now,
            clock: 0.,
            random: Random(seed.max(1)),
            kind,
            steering,
            speed: f64::from(speed),
            range: f64::from(range),
            lifetime_ms,
            armed_ms,
        };
        motion.clock = motion.interval();
        Ok(motion)
    }
    fn interval(&mut self) -> f64 {
        let (min, max) = match self.kind {
            Propulsion::Guided { .. } => (
                self.steering.guided_decision_min_seconds,
                self.steering.guided_decision_max_seconds,
            ),
            Propulsion::Dumb { .. } => (
                self.steering.dumb_decision_min_seconds,
                self.steering.dumb_decision_max_seconds,
            ),
        };
        f64::from((max - min).mul_add(self.random.draw(), min))
    }
    pub(crate) fn position(&self) -> Point {
        self.position
    }
    pub(crate) fn advanced_at_ms(&self) -> u64 {
        self.advanced
    }
    pub(crate) fn heading(&self) -> Point {
        self.heading
    }
    #[cfg(test)]
    pub(crate) fn lost(&self) -> bool {
        matches!(self.kind, Propulsion::Guided { lost: true, .. })
    }
    pub(crate) fn advance<T>(
        &self,
        now: u64,
        mut target: impl FnMut() -> Option<Point>,
        mut query: impl FnMut(
            Point,
            Point,
            Point,
        )
            -> Result<Option<crate::projectile_motion::Contact<T>>, &'static str>,
    ) -> Result<Advance<T>, &'static str> {
        if now < self.advanced {
            return Err("guided_motion_time_reversed");
        }
        let mut next = self.clone();
        let mut count = 0;
        while next.advanced < now {
            count += 1;
            if count > MAX_SUBSTEPS {
                return Err("guided_motion_substep_work_limit");
            }
            let end = next.advanced.saturating_add(SUBSTEP_MS).min(now);
            let dt = (end - next.advanced) as f64 / 1000.;
            // Original outer-update ordering: strict age, cruise, current-root range, clock.
            if end - next.launched > next.lifetime_ms {
                return Ok(Advance::Terminal {
                    position: next.position,
                    reason: Terminal::Age,
                });
            }
            let displacement = sub(next.position, next.origin);
            let squared = dot(displacement, displacement);
            if let Propulsion::Dumb { cruise } = &mut next.kind
                && squared > f64::from(next.steering.dumb_cruise_range_m).powi(2)
            {
                *cruise = false;
            }
            if squared > next.range * next.range {
                return Ok(Advance::Terminal {
                    position: next.position,
                    reason: Terminal::Range,
                });
            }
            let mut timer = next.clock;
            let due = clock(&mut timer, dt, || next.interval());
            next.clock = timer;
            if due {
                match next.kind {
                    Propulsion::Guided {
                        initial,
                        bias,
                        lost: false,
                    } => {
                        let decision = guide(
                            next.heading,
                            next.position,
                            target(),
                            bias,
                            initial,
                            f64::from(next.steering.guided_max_direction_change),
                        );
                        next.heading = decision.heading;
                        next.kind = Propulsion::Guided {
                            initial,
                            bias: decision.bias,
                            lost: decision.lost,
                        };
                    }
                    Propulsion::Guided { lost: true, .. } | Propulsion::Dumb { cruise: true } => (),
                    Propulsion::Dumb { cruise: false } => {
                        next.heading = dumb(
                            next.heading,
                            f64::from(next.steering.dumb_direction_change_min),
                            f64::from(next.steering.dumb_direction_change_max),
                            || f64::from(next.random.draw()),
                        );
                    }
                }
                next.velocity = next.heading.map(|v| v * next.speed);
            }
            let destination =
                std::array::from_fn(|i| next.velocity[i].mul_add(dt, next.position[i]));
            if !destination.into_iter().all(f64::is_finite) {
                return Err("guided_motion_position_overflow");
            }
            if let Some(hit) = query(next.position, destination, next.heading)? {
                if !(0. ..=1.).contains(&hit.fraction) {
                    return Err("invalid_guided_contact_fraction");
                }
                let elapsed = (next.advanced - next.launched) as f64
                    + (end - next.advanced) as f64 * hit.fraction;
                let position = std::array::from_fn(|i| {
                    (destination[i] - next.position[i]).mul_add(hit.fraction, next.position[i])
                });
                // Second native timer starts at zero. Later Dampen/interleave is outside
                // this explicit retire-without-damage pre-arm policy.
                let reason = if elapsed < next.armed_ms as f64 {
                    Terminal::Prearm
                } else {
                    Terminal::Contact(hit.obstacle)
                };
                return Ok(Advance::Terminal { position, reason });
            }
            next.position = destination;
            next.advanced = end;
        }
        Ok(Advance::Flying(next))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn guided_independent_capped_and_bias_vectors() {
        let cases = [
            (
                [0.2, -0.5, 5.],
                0.5,
                10.,
                [0.03996803834887158, 0., 0.9992009587217894],
                0.5,
            ),
            (
                [3., 4., 4.],
                0.5,
                10.,
                [0.04163385811749713, 0.0624507871762457, 0.9971792822954754],
                0.5,
            ),
            ([0., -7.5, 80.], 7.5, 10., [0., 0., 1.], 0.8),
        ];
        for (anchor, bias, initial, expected, next_bias) in cases {
            let out = guide([0., 0., 1.], [0.; 3], Some(anchor), bias, initial, 0.07);
            for (a, b) in out.heading.into_iter().zip(expected) {
                assert!((a - b).abs() < 1e-12, "{out:?}");
            }
            assert!((out.bias - next_bias).abs() < 1e-12);
            assert!(!out.lost);
        }
    }
}

#[cfg(test)]
mod boundary_tests {
    use super::*;
    fn rules() -> Steering {
        Steering {
            guided_decision_min_seconds: 0.01,
            guided_decision_max_seconds: 0.1,
            guided_max_direction_change: 0.07,
            dumb_decision_min_seconds: 0.01,
            dumb_decision_max_seconds: 0.01,
            dumb_direction_change_min: 0.005,
            dumb_direction_change_max: 0.005,
            dumb_cruise_range_m: 2.,
            dumb_max_range_m: 150.,
        }
    }
    #[test]
    fn guided_clock_equality_overshoot_negative_and_dumb_draw_order() {
        for (timer, dt, due, expected) in [
            (0.03, 0.01, false, 0.02),
            (0.01, 0.01, true, 0.055),
            (0.01, 0.05, true, 0.055),
            (-0.001, 0.05, false, -0.001),
        ] {
            let mut clock_value = timer;
            let mut draws = 0;
            assert_eq!(
                clock(&mut clock_value, dt, || {
                    draws += 1;
                    0.055
                }),
                due
            );
            assert!((clock_value - expected).abs() < 1e-12);
            assert_eq!(draws, usize::from(due));
        }
        let mut draws = [0.75, 0.1, 0.1, 0.9].into_iter();
        let out = dumb([0., 0., 1.], 0.005, 0.005, || draws.next().unwrap());
        for (a, b) in out.into_iter().zip([
            0.004999875004687304,
            0.004999875004687304,
            0.9999750009374608,
        ]) {
            assert!((a - b).abs() < 1e-12);
        }
        assert_eq!(draws.next(), None);
    }
    #[test]
    fn guided_loss_latches_heading_and_old_bias_degenerate_holds() {
        for anchor in [None, Some([1., 0., -2.])] {
            let d = guide([0., 0., 1.], [0.; 3], anchor, 0.5, 10., 0.07);
            assert!(d.lost);
            assert_eq!(d.heading, [0., 0., 1.]);
        }
        let d = guide([0., 0., 1.], [0.; 3], Some([0., -0.5, 0.]), 0.5, 10., 0.07);
        assert_eq!(d.heading, [0., 0., 1.]);
        assert_eq!(d.bias, 0.5);
        assert!(!d.lost);
        let mut motion = Motion::launch(
            [0.; 3],
            [3., 0., 0.],
            [0., 0., 1.],
            0,
            rules(),
            60.,
            150.,
            10_000,
            100,
            Some([0., 0., 10.]),
            1,
        )
        .unwrap();
        motion.clock = 0.;
        let Advance::Flying(next) = motion
            .advance(
                10,
                || None,
                |_, _, _| Ok(None::<crate::projectile_motion::Contact<()>>),
            )
            .unwrap()
        else {
            panic!()
        };
        assert!(next.lost());
        assert_eq!(next.velocity, [0., 0., 60.]);
        let Advance::Flying(after) = next
            .advance(
                20,
                || Some([1., 0., 10.]),
                |_, _, _| Ok(None::<crate::projectile_motion::Contact<()>>),
            )
            .unwrap()
        else {
            panic!()
        };
        assert!(after.lost());
        assert_eq!(after.heading, [0., 0., 1.]);
    }
    #[test]
    fn guided_active_age_range_and_prearm_contact_boundaries_are_strict() {
        let mut motion = Motion::launch(
            [0.; 3],
            [0., 0., 1.],
            [0., 0., 1.],
            0,
            rules(),
            60.,
            150.,
            10_000,
            100,
            None,
            1,
        )
        .unwrap();
        motion.advanced = 9_990;
        motion.clock = -1.;
        let Advance::Flying(at) = motion
            .advance(
                10_000,
                || None,
                |_, _, _| Ok(None::<crate::projectile_motion::Contact<()>>),
            )
            .unwrap()
        else {
            panic!("equality continues")
        };
        assert!(matches!(
            at.advance(
                10_001,
                || None,
                |_, _, _| Ok(None::<crate::projectile_motion::Contact<()>>)
            )
            .unwrap(),
            Advance::Terminal {
                reason: Terminal::Age,
                ..
            }
        ));
        motion.advanced = 0;
        motion.position = [0., 0., 150.];
        assert!(matches!(
            motion
                .advance(
                    1,
                    || None,
                    |_, _, _| Ok(None::<crate::projectile_motion::Contact<()>>)
                )
                .unwrap(),
            Advance::Flying(_)
        ));
        motion.position = [0., 0., 150.0001];
        assert!(matches!(
            motion
                .advance(
                    1,
                    || None,
                    |_, _, _| Ok(None::<crate::projectile_motion::Contact<()>>)
                )
                .unwrap(),
            Advance::Terminal {
                reason: Terminal::Range,
                ..
            }
        ));
        motion.position = [0.; 3];
        for (time, prearm) in [(99, true), (100, false)] {
            motion.advanced = time - 1;
            let out = motion
                .advance(
                    time,
                    || None,
                    |_, _, _| {
                        Ok(Some(crate::projectile_motion::Contact {
                            fraction: 1.,
                            obstacle: (),
                        }))
                    },
                )
                .unwrap();
            assert_eq!(
                matches!(
                    out,
                    Advance::Terminal {
                        reason: Terminal::Prearm,
                        ..
                    }
                ),
                prearm
            );
        }
    }
}
