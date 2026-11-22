//! Bounded translated projectile motion. The caller supplies collision queries
//! and commits the returned candidate only after recording it.

type Point = [f64; 3];

const GRAVITY: Point = [0., -9.81, 0.];
const STEP_MS: u64 = 20;
const MAX_STEPS: u64 = 16;
const CONTACT_BACKOFF_M: f64 = 0.0001;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Limits {
    pub armed_ms: u64,
    pub lifetime_ms: u64,
    pub grace_ms: u64,
    pub linear_damping: f64,
    pub restitution: f64,
    pub maximum_speed_mps: f64,
    pub maximum_duration_ms: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Motion<K> {
    position: Point,
    velocity: Point,
    launched_at_ms: u64,
    advanced_at_ms: u64,
    expires_at_ms: u64,
    limits: Limits,
    blocked_contact: Option<K>,
    terminal: bool,
}

pub(crate) struct Contact<T, K> {
    pub fraction: f64,
    pub normal: Option<Point>,
    pub key: K,
    pub obstacle: T,
}

#[derive(Debug, PartialEq)]
pub(crate) enum TerminalReason<T> {
    Contact(T),
    AgeExpired,
}

pub(crate) struct Terminal<T> {
    pub position: Point,
    pub elapsed_ms: f64,
    pub reason: TerminalReason<T>,
}

pub(crate) enum Advance<T, K> {
    Flying(Motion<K>),
    Terminal {
        next: Motion<K>,
        terminal: Terminal<T>,
    },
    AlreadyTerminal,
}

impl<K: Clone + Eq> Motion<K> {
    pub fn launch(
        origin: Point,
        velocity: Point,
        now_ms: u64,
        limits: Limits,
    ) -> Result<Self, &'static str> {
        let duration = limits
            .lifetime_ms
            .checked_add(limits.grace_ms)
            .filter(|&duration| duration <= limits.maximum_duration_ms)
            .ok_or("invalid_ballistic_duration")?;
        if limits.lifetime_ms == 0
            || limits.armed_ms > duration
            || !limits.linear_damping.is_finite()
            || limits.linear_damping < 0.
            || !(0. ..=1.).contains(&limits.restitution)
        {
            return Err("invalid_ballistic_limits");
        }
        if !origin.into_iter().all(f64::is_finite) {
            return Err("invalid_ballistic_origin");
        }
        let speed = length(velocity);
        if !speed.is_finite()
            || !limits.maximum_speed_mps.is_finite()
            || limits.maximum_speed_mps <= 0.
            || speed > limits.maximum_speed_mps
        {
            return Err("invalid_ballistic_velocity");
        }
        Ok(Self {
            position: origin,
            velocity,
            launched_at_ms: now_ms,
            advanced_at_ms: now_ms,
            expires_at_ms: now_ms
                .checked_add(duration)
                .ok_or("ballistic_time_overflow")?,
            limits,
            blocked_contact: None,
            terminal: false,
        })
    }

    pub fn position(&self) -> Point {
        self.position
    }
    #[cfg(test)]
    pub fn velocity(&self) -> Point {
        self.velocity
    }
    pub fn advanced_at_ms(&self) -> u64 {
        self.advanced_at_ms
    }

    /// Approximate translation using bounded sweeps, gravity and linear
    /// velocity damping. The arm gate uses the enclosing simulation update end,
    /// as the original timer updates precede contact consumption. Catch-up stops
    /// at the bounded cursor and must continue in a later call.
    pub fn advance<T>(
        &self,
        now_ms: u64,
        mut query: impl FnMut(Point, Point) -> Result<Option<Contact<T, K>>, &'static str>,
    ) -> Result<Advance<T, K>, &'static str> {
        if now_ms < self.advanced_at_ms {
            return Err("ballistic_time_reversed");
        }
        if self.terminal {
            return Ok(Advance::AlreadyTerminal);
        }
        let mut next = self.clone();
        let update_end = now_ms
            .min(self.expires_at_ms)
            .min(self.advanced_at_ms.saturating_add(STEP_MS * MAX_STEPS));
        let armed = update_end - self.launched_at_ms >= self.limits.armed_ms;
        while next.advanced_at_ms < update_end {
            if let Some(terminal) = next.step(update_end, armed, &mut query)? {
                next.terminal = true;
                return Ok(Advance::Terminal { next, terminal });
            }
        }
        if update_end != self.expires_at_ms {
            return Ok(Advance::Flying(next));
        }
        next.terminal = true;
        let terminal = Terminal {
            position: next.position,
            elapsed_ms: (update_end - self.launched_at_ms) as f64,
            reason: TerminalReason::AgeExpired,
        };
        Ok(Advance::Terminal { next, terminal })
    }

    fn step<T>(
        &mut self,
        update_end: u64,
        armed: bool,
        query: &mut impl FnMut(Point, Point) -> Result<Option<Contact<T, K>>, &'static str>,
    ) -> Result<Option<Terminal<T>>, &'static str> {
        let start_ms = self.advanced_at_ms;
        let step_ms = (update_end - start_ms).min(STEP_MS);
        let dt = step_ms as f64 / 1_000.;
        let start = self.position;
        let delta = std::array::from_fn(|i| (self.velocity[i] + 0.5 * GRAVITY[i] * dt) * dt);
        let end = std::array::from_fn(|i| start[i] + delta[i]);
        let damping = (1. - self.limits.linear_damping * dt).max(0.);
        self.velocity = std::array::from_fn(|i| (self.velocity[i] + GRAVITY[i] * dt) * damping);
        if !end.into_iter().all(f64::is_finite) || !self.velocity.into_iter().all(f64::is_finite) {
            return Err("ballistic_motion_overflow");
        }
        self.advanced_at_ms += step_ms;
        if start == end {
            return Ok(None);
        }
        let Some(Contact {
            fraction,
            normal,
            key,
            obstacle,
        }) = query(start, end)?
        else {
            self.blocked_contact = None;
            self.position = end;
            return Ok(None);
        };
        if !(0. ..=1.).contains(&fraction) {
            return Err("invalid_ballistic_contact_fraction");
        }
        let approaching = normal.and_then(unit).filter(|&n| {
            fraction > 0. && self.blocked_contact.as_ref() != Some(&key) && dot(delta, n) < 0.
        });
        if let Some(n) = approaching {
            if armed {
                self.position = along(start, delta, fraction);
                return Ok(Some(Terminal {
                    position: self.position,
                    elapsed_ms: (start_ms - self.launched_at_ms) as f64 + step_ms as f64 * fraction,
                    reason: TerminalReason::Contact(obstacle),
                }));
            }
            // Source restitution applies only to the incoming normal component
            // in this bounded approximation. Stop this step at contact and let
            // the next step query the new separating motion.
            let impulse = -(1. + self.limits.restitution) * dot(self.velocity, n).min(0.);
            self.velocity = along(self.velocity, n, impulse);
        } else {
            // No certified separating response. Retain this obstacle identity
            // until a clear sweep, without granting a delayed armed event.
            self.velocity = [0.; 3];
        }
        if !self.velocity.into_iter().all(f64::is_finite) {
            return Err("ballistic_motion_overflow");
        }
        self.blocked_contact = Some(key);
        // Back up along the already traversable part of the incoming sweep.
        // A normal-direction nudge could cross a different obstacle.
        let distance = length(delta);
        if !distance.is_finite() || distance == 0. {
            return Err("ballistic_motion_overflow");
        }
        self.position = along(
            start,
            delta,
            (fraction - CONTACT_BACKOFF_M / distance).max(0.),
        );
        Ok(None)
    }
}

fn along(start: Point, vector: Point, scale: f64) -> Point {
    std::array::from_fn(|i| vector[i].mul_add(scale, start[i]))
}

fn length(v: Point) -> f64 {
    v[0].hypot(v[1]).hypot(v[2])
}

fn unit(v: Point) -> Option<Point> {
    let length = length(v);
    (length.is_finite() && length > 0.).then(|| v.map(|x| x / length))
}

fn dot(a: Point, b: Point) -> f64 {
    a[0].mul_add(b[0], a[1].mul_add(b[1], a[2] * b[2]))
}

#[cfg(test)]
#[path = "ballistic_projectile_motion_tests.rs"]
mod tests;
