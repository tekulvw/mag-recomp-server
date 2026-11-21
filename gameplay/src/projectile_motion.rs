//! Server-owned straight flight approximates the native RocketStraight velocity
//! refresh with a constant world-space velocity. Native generic overage runs an
//! explosion callback after the authored lifetime and its grace period. This
//! state machine emits one terminal at contact or that bounded age. The caller
//! records and publishes it before retirement according to its local queue policy.

type Point = [f64; 3];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Limits {
    pub radius: f64,
    pub lifetime_ms: u64,
    pub grace_ms: u64,
    pub maximum_speed_mps: f64,
    pub maximum_duration_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Motion {
    origin: Point,
    world_velocity: Point,
    position: Point,
    radius: f64,
    launched_at_ms: u64,
    advanced_at_ms: u64,
    expires_at_ms: u64,
    terminal: bool,
}

pub struct Contact<T> {
    /// First contact along the requested segment, from 0 at start to 1 at end.
    pub fraction: f64,
    /// Owned contact details supplied by the caller, such as identity and normal.
    pub obstacle: T,
}

#[derive(Debug, PartialEq)]
pub enum TerminalReason<T> {
    Contact(T),
    AgeExpired,
}

pub struct Terminal<T> {
    /// Sphere centre at contact or expiry.
    pub position: Point,
    /// Milliseconds since launch. A contact can lie between whole tick times.
    pub elapsed_ms: f64,
    pub reason: TerminalReason<T>,
}

pub enum Advance<T> {
    Flying(Motion),
    Terminal { next: Motion, terminal: Terminal<T> },
    AlreadyTerminal,
}

impl Motion {
    /// The supplied velocity already includes any qualified launch components.
    pub fn launch(
        origin: Point,
        world_velocity: Point,
        now_ms: u64,
        limits: Limits,
    ) -> Result<Self, &'static str> {
        let duration = limits
            .lifetime_ms
            .checked_add(limits.grace_ms)
            .filter(|duration| *duration <= limits.maximum_duration_ms)
            .ok_or("invalid_projectile_motion_duration")?;
        if limits.lifetime_ms == 0 || !limits.radius.is_finite() || limits.radius <= 0. {
            return Err("invalid_projectile_motion_limits");
        }
        if !origin.into_iter().all(f64::is_finite) {
            return Err("invalid_projectile_motion_origin");
        }
        let speed = world_velocity[0]
            .hypot(world_velocity[1])
            .hypot(world_velocity[2]);
        if !speed.is_finite()
            || speed <= 0.
            || !limits.maximum_speed_mps.is_finite()
            || speed > limits.maximum_speed_mps
        {
            return Err("invalid_projectile_motion_velocity");
        }
        let expires_at_ms = now_ms
            .checked_add(duration)
            .ok_or("projectile_motion_time_overflow")?;
        let motion = Self {
            origin,
            world_velocity,
            position: origin,
            radius: limits.radius,
            launched_at_ms: now_ms,
            advanced_at_ms: now_ms,
            expires_at_ms,
            terminal: false,
        };
        motion.position_at(expires_at_ms)?;
        Ok(motion)
    }

    pub fn position(self) -> Point {
        self.position
    }

    pub fn advanced_at_ms(self) -> u64 {
        self.advanced_at_ms
    }

    /// Plans one continuous segment without changing self. The caller commits
    /// the returned candidate only after any associated recording succeeds.
    pub fn advance<T>(
        &self,
        now_ms: u64,
        query: impl FnOnce(Point, Point, f64) -> Result<Option<Contact<T>>, &'static str>,
    ) -> Result<Advance<T>, &'static str> {
        if now_ms < self.advanced_at_ms {
            return Err("projectile_motion_time_reversed");
        }
        if self.terminal {
            return Ok(Advance::AlreadyTerminal);
        }
        if now_ms == self.advanced_at_ms {
            return Ok(Advance::Flying(*self));
        }

        let end_ms = now_ms.min(self.expires_at_ms);
        let end = self.position_at(end_ms)?;
        let mut next = Self {
            position: end,
            advanced_at_ms: end_ms,
            ..*self
        };
        let terminal =
            if let Some(Contact { fraction, obstacle }) = query(self.position, end, self.radius)? {
                if !(0. ..=1.).contains(&fraction) {
                    return Err("invalid_projectile_contact_fraction");
                }
                let elapsed_ms = (self.advanced_at_ms - self.launched_at_ms) as f64
                    + (end_ms - self.advanced_at_ms) as f64 * fraction;
                next.position = self.position_at_elapsed_ms(elapsed_ms)?;
                Terminal {
                    position: next.position,
                    elapsed_ms,
                    reason: TerminalReason::Contact(obstacle),
                }
            } else if end_ms == self.expires_at_ms {
                Terminal {
                    position: next.position,
                    elapsed_ms: (end_ms - self.launched_at_ms) as f64,
                    reason: TerminalReason::AgeExpired,
                }
            } else {
                return Ok(Advance::Flying(next));
            };
        next.terminal = true;
        Ok(Advance::Terminal { next, terminal })
    }

    fn position_at(self, now_ms: u64) -> Result<Point, &'static str> {
        self.position_at_elapsed_ms((now_ms - self.launched_at_ms) as f64)
    }

    fn position_at_elapsed_ms(self, elapsed_ms: f64) -> Result<Point, &'static str> {
        let seconds = elapsed_ms / 1_000.;
        let position = std::array::from_fn(|axis| {
            self.world_velocity[axis].mul_add(seconds, self.origin[axis])
        });
        if position.into_iter().all(f64::is_finite) {
            Ok(position)
        } else {
            Err("projectile_motion_position_overflow")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits() -> Limits {
        Limits {
            radius: 0.25,
            lifetime_ms: 1_000,
            grace_ms: 200,
            maximum_speed_mps: 20.,
            maximum_duration_ms: 2_000,
        }
    }

    fn launch() -> Motion {
        Motion::launch([10., 20., 30.], [12., 0., 0.], 100, limits()).unwrap()
    }

    #[test]
    fn straight_motion_queries_from_last_commit_and_returns_a_new_candidate() {
        let motion = launch();
        let Advance::Flying(next) = motion
            .advance(350, |start, end, radius| {
                assert_eq!(start, [10., 20., 30.]);
                assert_eq!(end, [13., 20., 30.]);
                assert_eq!(radius, 0.25);
                Ok(None::<Contact<u32>>)
            })
            .unwrap()
        else {
            panic!("expected open flight");
        };
        assert_eq!(motion.position(), [10., 20., 30.]);
        assert_eq!(next.position(), [13., 20., 30.]);
        assert_eq!(next.advanced_at_ms(), 350);
        let Advance::Flying(next) = next
            .advance(600, |start, end, _| {
                assert_eq!(start, [13., 20., 30.]);
                assert_eq!(end, [16., 20., 30.]);
                Ok(None::<Contact<u32>>)
            })
            .unwrap()
        else {
            panic!("expected continued flight");
        };
        assert_eq!(next.position(), [16., 20., 30.]);
    }

    #[test]
    fn straight_motion_consumes_contact_once_and_interpolates_its_center() {
        let Advance::Terminal { next, terminal } = launch()
            .advance(600, |_, _, _| {
                Ok(Some(Contact {
                    fraction: 0.25,
                    obstacle: 7u32,
                }))
            })
            .unwrap()
        else {
            panic!("expected contact");
        };
        assert_eq!(terminal.reason, TerminalReason::Contact(7));
        assert_eq!(terminal.position, [11.5, 20., 30.]);
        assert_eq!(terminal.elapsed_ms, 125.);
        assert_eq!(next.position(), terminal.position);
        assert!(matches!(
            next.advance::<u32>(700, |_, _, _| panic!("terminal swept again")),
            Ok(Advance::AlreadyTerminal)
        ));
    }

    #[test]
    fn straight_motion_clips_to_age_boundary_and_prefers_contact_at_that_boundary() {
        for has_contact in [false, true] {
            let Advance::Terminal { next, terminal } = launch()
                .advance(9_000, |start, end, _| {
                    assert_eq!(start, [10., 20., 30.]);
                    assert_eq!(end, [24.4, 20., 30.]);
                    Ok(has_contact.then_some(Contact {
                        fraction: 1.,
                        obstacle: 8,
                    }))
                })
                .unwrap()
            else {
                panic!("expected a terminal at the age boundary");
            };
            let reason = if has_contact {
                TerminalReason::Contact(8)
            } else {
                TerminalReason::AgeExpired
            };
            assert_eq!(terminal.reason, reason);
            assert_eq!(terminal.position, [24.4, 20., 30.]);
            assert_eq!(terminal.elapsed_ms, 1_200.);
            assert_eq!(next.advanced_at_ms(), 1_300);
        }
    }

    #[test]
    fn straight_motion_keeps_same_timestamp_quiet_and_accepts_first_segment_start_overlap() {
        let motion = launch();
        let Advance::Flying(still) = motion
            .advance::<u32>(100, |_, _, _| panic!("empty interval swept"))
            .unwrap()
        else {
            panic!("same timestamp must preserve the live candidate");
        };
        assert_eq!(still, motion);
        let Advance::Terminal { terminal, .. } = still
            .advance(101, |_, _, _| {
                Ok(Some(Contact {
                    fraction: 0.,
                    obstacle: 5,
                }))
            })
            .unwrap()
        else {
            panic!("expected starting overlap");
        };
        assert_eq!(terminal.reason, TerminalReason::Contact(5));
        assert_eq!(terminal.position, [10., 20., 30.]);
        assert_eq!(terminal.elapsed_ms, 0.);
    }

    #[test]
    fn straight_motion_rejects_query_errors_without_advancing() {
        let motion = launch();
        for fraction in [f64::NAN, f64::INFINITY, -0.01, 1.01] {
            assert_eq!(
                motion
                    .advance(125, |_, _, _| Ok(Some(Contact {
                        fraction,
                        obstacle: ()
                    })))
                    .err(),
                Some("invalid_projectile_contact_fraction")
            );
        }
        assert_eq!(
            motion
                .advance::<u32>(125, |_, _, _| Err("unavailable"))
                .err(),
            Some("unavailable")
        );
        assert_eq!(
            motion
                .advance::<u32>(99, |_, _, _| panic!("backward time swept"))
                .err(),
            Some("projectile_motion_time_reversed")
        );
        assert_eq!(motion, launch());
    }

    #[test]
    fn straight_motion_rejects_invalid_launch_parameters_and_time_overflow() {
        let origin = [0.; 3];
        let velocity = [12., 0., 0.];
        for policy in [
            Limits {
                radius: 0.,
                ..limits()
            },
            Limits {
                radius: f64::NAN,
                ..limits()
            },
            Limits {
                maximum_speed_mps: f64::INFINITY,
                ..limits()
            },
            Limits {
                lifetime_ms: 0,
                ..limits()
            },
            Limits {
                maximum_duration_ms: 1_199,
                ..limits()
            },
            Limits {
                grace_ms: u64::MAX,
                ..limits()
            },
        ] {
            assert!(Motion::launch(origin, velocity, 0, policy).is_err());
        }
        for velocity in [[0.; 3], [21., 0., 0.], [f64::NAN, 0., 0.]] {
            assert!(Motion::launch(origin, velocity, 0, limits()).is_err());
        }
        assert!(Motion::launch([f64::INFINITY, 0., 0.], velocity, 0, limits()).is_err());
        assert!(Motion::launch(origin, velocity, u64::MAX, limits()).is_err());
    }
}
