//! Bounded development bot locomotion over caller-supplied world triangles.
//! Uses a conservative upright box, not native Havok character/capsule parity.
//! Supplied coverage/provenance remains authoritative only for that subset;
//! missing ground is a hole, never an invented plane. Optional player bodies use the same sweeps.
use crate::collision::{CollisionWorld, RayHit};
use serde::{Deserialize, Serialize};
pub type Point = [f64; 3];
const WORK_LIMIT: usize = 16_384;
const SUBSTEP: f64 = 0.05;
const GRAVITY: f64 = 9.81;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhysicsError {
    Unavailable,
    MapMismatch,
    QueryFailed,
    InvalidQuery,
    CoverageUnknown,
}
impl PhysicsError {
    pub fn code(self) -> &'static str {
        match self {
            Self::Unavailable => "bot_physics_geometry_unavailable",
            Self::MapMismatch => "bot_physics_map_mismatch",
            Self::QueryFailed => "bot_physics_query_failed_or_budget",
            Self::InvalidQuery => "bot_physics_invalid_query",
            Self::CoverageUnknown => "bot_physics_geometry_coverage_unknown",
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct GroundSample {
    pub position: Point,
    pub normal: Point,
}
/// Offline evidence for one directed descent at one explicitly tested speed.
/// Callers bind it to their world hash/body profile and must test each supported
/// gait; success at one speed does not establish a continuous speed range.
#[derive(Clone, Debug, Serialize)]
pub struct DescentTrace {
    pub launch: Point,
    pub landing: Point,
    pub speed: f64,
    pub elapsed_seconds: f64,
    pub max_drop_height: f64,
    pub landing_drop_heights: Vec<f64>,
    pub samples: Vec<Motion>,
}
#[derive(Clone, Copy, Debug)]
pub struct Body {
    pub radius: f64,
    pub height: f64,
    pub max_step: f64,
    pub max_slope_degrees: f64,
    pub ground_snap: f64,
    pub skin: f64,
}
impl Default for Body {
    fn default() -> Self {
        Self {
            radius: 0.35,
            height: 1.8,
            max_step: 0.4,
            max_slope_degrees: 45.0,
            ground_snap: 0.2,
            skin: 0.005,
        }
    }
}
impl Body {
    pub fn validate(self) -> Result<(), PhysicsError> {
        if [
            self.radius,
            self.height,
            self.max_step,
            self.max_slope_degrees,
            self.ground_snap,
            self.skin,
        ]
        .iter()
        .any(|v| !v.is_finite())
            || !(0.05..=2.0).contains(&self.radius)
            || !(0.2..=4.0).contains(&self.height)
            || !(0.0..=1.0).contains(&self.max_step)
            || !(0.0..=60.0).contains(&self.max_slope_degrees)
            || !(0.0..=1.0).contains(&self.ground_snap)
            || !(0.001..=0.05).contains(&self.skin)
        {
            return Err(PhysicsError::InvalidQuery);
        }
        Ok(())
    }
    fn slope_cos(self) -> f64 {
        self.max_slope_degrees.to_radians().cos()
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Motion {
    pub position: Point,
    pub vertical_velocity: f64,
    /// Gravity projected onto steep contacts carries horizontal momentum.
    /// Commanded movement remains separate and cannot turn this into uphill lift.
    #[serde(default)]
    pub slide_velocity: [f64; 2],
    pub grounded: bool,
    pub blocked: bool,
}
impl Motion {
    pub fn new(position: Point) -> Self {
        Self {
            position,
            vertical_velocity: 0.0,
            slide_velocity: [0.0; 2],
            grounded: false,
            blocked: false,
        }
    }
}
pub struct WorldTerrain<'a> {
    world: &'a CollisionWorld,
    actors: Option<crate::actor_collision::Query<'a>>,
    actor_contact: std::cell::Cell<Option<u16>>,
}
impl<'a> WorldTerrain<'a> {
    pub fn new(world: Option<&'a CollisionWorld>, map_name: &str) -> Result<Self, PhysicsError> {
        let world = world.ok_or(PhysicsError::Unavailable)?;
        if world.metadata().map_name != map_name {
            return Err(PhysicsError::MapMismatch);
        }
        if world.stats().triangles == 0 {
            return Err(PhysicsError::Unavailable);
        }
        Ok(Self {
            world,
            actors: None,
            actor_contact: std::cell::Cell::new(None),
        })
    }
    pub(crate) fn with_actors(mut self, query: crate::actor_collision::Query<'a>) -> Self {
        self.actors = Some(query);
        self
    }
    pub(crate) fn actor_contact(&self) -> Option<u16> {
        self.actor_contact.get()
    }
    /// Requires explicit source-verified coverage, including the complete body
    /// corridor and vertical step/snap margin. Triangle presence/bounds and global
    /// coverage labels alone never establish obstacle coverage.
    pub fn require_horizontal_coverage(
        &self,
        from: Point,
        to: Point,
        body: Body,
    ) -> Result<(), PhysicsError> {
        body.validate()?;
        if !valid(from) || !valid(to) {
            return Err(PhysicsError::InvalidQuery);
        }
        let margin = [body.radius, body.ground_snap + body.skin, body.radius];
        let upper = [
            body.radius,
            body.height + body.max_step + body.skin,
            body.radius,
        ];
        let covered = self.world.metadata().physics_regions.iter().any(|r| {
            r.ground_verified
                && r.obstacles_verified
                && (0..3).all(|i| {
                    from[i].min(to[i]) - margin[i] >= r.min[i]
                        && from[i].max(to[i]) + upper[i] <= r.max[i]
                })
        });
        if covered {
            Ok(())
        } else {
            Err(PhysicsError::CoverageUnknown)
        }
    }
    /// Refuses known initial penetration. Surface tangency is allowed; no
    /// depenetration/ground plane is invented to make an invalid spawn usable.
    pub fn start_motion(&self, position: Point, body: Body) -> Result<Motion, PhysicsError> {
        body.validate()?;
        if !valid(position) {
            return Err(PhysicsError::InvalidQuery);
        }
        let mut budget = WORK_LIMIT;
        if self.sweep(position, position, body, &mut budget)?.is_some() {
            return Err(PhysicsError::InvalidQuery);
        }
        Ok(Motion::new(position))
    }
    pub fn metadata(&self) -> &crate::collision::WorldMetadata {
        self.world.metadata()
    }
    /// Bounded, query-only directed link validation with actual physical state
    /// feedback. Both supplied endpoints are feet positions; no pose is snapped
    /// to either endpoint. None means blocked, unsupported, or not reached.
    /// Fixed 50ms calls reset the work budget each call: production followers
    /// must separately validate their actual call cadence and corridor handling.
    pub fn validate_descent(
        &self,
        launch: Point,
        landing: Point,
        speed: f64,
        b: Body,
        seconds: f64,
    ) -> Result<Option<DescentTrace>, PhysicsError> {
        if !valid(landing)
            || !speed.is_finite()
            || !(0.05..=6.0).contains(&speed)
            || !seconds.is_finite()
            || !(0.05..=10.0).contains(&seconds)
            || landing[1] >= launch[1]
            || (landing[0] - launch[0]).hypot(landing[2] - launch[2]) > 16.0
        {
            return Err(PhysicsError::InvalidQuery);
        }
        self.require_horizontal_coverage(launch, landing, b)?;
        let mut m = self.start_motion(launch, b)?;
        // Verify stable source support instead of trusting a navmesh vertex Y.
        for _ in 0..20 {
            m = self.advance(m, [0.0; 3], SUBSTEP, b)?;
            if m.blocked || length(sub(m.position, launch)) > 0.05 {
                return Ok(None);
            }
        }
        if !m.grounded {
            return Ok(None);
        }
        let launch = m.position;
        let mut samples = vec![m];
        let mut peak = launch[1];
        let mut max_drop_height: f64 = 0.0;
        let mut departure_y = None;
        let mut landing_drop_heights = Vec::new();
        for _ in 0..(seconds / SUBSTEP).ceil() as usize {
            let previous = m;
            let d = (landing[0] - m.position[0]).hypot(landing[2] - m.position[2]);
            let v = if d <= 1.0 / 32.0 {
                [0.0; 3]
            } else {
                let speed = speed.min(d / SUBSTEP);
                [
                    (landing[0] - m.position[0]) / d * speed,
                    0.0,
                    (landing[2] - m.position[2]) / d * speed,
                ]
            };
            m = self.advance(m, v, SUBSTEP, b)?;
            samples.push(m);
            if m.blocked {
                return Ok(None);
            }
            if !m.grounded && departure_y.is_none() {
                departure_y = Some(previous.position[1]);
            }
            if m.grounded
                && let Some(y) = departure_y.take()
            {
                landing_drop_heights.push((y - m.position[1]).max(0.0));
            }
            peak = peak.max(m.position[1]);
            max_drop_height = max_drop_height.max(peak - m.position[1]);
            if m.grounded
                && (landing[0] - m.position[0]).hypot(landing[2] - m.position[2]) <= 1.0 / 32.0
                && (landing[1] - m.position[1]).abs() <= 0.6
            {
                return Ok(Some(DescentTrace {
                    launch,
                    landing: m.position,
                    speed,
                    elapsed_seconds: (samples.len() - 1) as f64 * SUBSTEP,
                    max_drop_height,
                    landing_drop_heights,
                    samples,
                }));
            }
        }
        Ok(None)
    }
    pub fn sample_ground(
        &self,
        position: Point,
        up: f64,
        down: f64,
    ) -> Result<Option<GroundSample>, PhysicsError> {
        let mut budget = WORK_LIMIT;
        self.ground(position, up, down, &mut budget)
    }
    fn ground(
        &self,
        position: Point,
        up: f64,
        down: f64,
        budget: &mut usize,
    ) -> Result<Option<GroundSample>, PhysicsError> {
        if !valid(position)
            || !up.is_finite()
            || !down.is_finite()
            || up < 0.0
            || down < 0.0
            || up + down <= 0.0
            || up + down > 1000.0
        {
            return Err(PhysicsError::InvalidQuery);
        }
        let start = [position[0], position[1] + up, position[2]];
        let end = [position[0], position[1] - down, position[2]];
        let hit = self
            .world
            .raycast_segment_bounded(start, end, budget)
            .map_err(|_| PhysicsError::QueryFailed)?
            .nearest;
        Ok(hit.map(|h| GroundSample {
            position: h.position,
            normal: h
                .normal
                .map(|v| v * if h.normal[1] < 0.0 { -1.0 } else { 1.0 }),
        }))
    }
    fn sweep(
        &self,
        from: Point,
        to: Point,
        body: Body,
        budget: &mut usize,
    ) -> Result<Option<RayHit>, PhysicsError> {
        let center = |p: Point| [p[0], p[1] + body.height * 0.5, p[2]];
        let mut nearest = self
            .world
            .sweep_aabb(
                center(from),
                center(to),
                [body.radius, body.height * 0.5, body.radius],
                budget,
            )
            .map_err(|_| PhysicsError::QueryFailed)?;
        if let Some(query) = self.actors
            && let Some(hit) = query
                .grid
                .sweep(query.actor, from, to, body, budget)
                .map_err(|_| PhysicsError::QueryFailed)?
            && nearest.as_ref().is_none_or(|h| hit.fraction < h.fraction)
        {
            let mut position = std::array::from_fn(|i| from[i] + (to[i] - from[i]) * hit.fraction);
            position[1] += body.height * 0.5;
            nearest = Some(RayHit {
                instance_id: format!("dynamic-character:{}", hit.id),
                triangle_index: 0,
                fraction: hit.fraction,
                position,
                normal: hit.normal,
            });
        }
        Ok(nearest)
    }
    /// Ground-supported local edge query for navigation. Bounds length to 8m,
    /// samples support every 25cm, rejects missing/steep ground and unsupported
    /// steps, then tests the same conservative body used by locomotion.
    pub fn can_traverse(&self, from: Point, to: Point, body: Body) -> Result<bool, PhysicsError> {
        body.validate()?;
        if !valid(from) || !valid(to) {
            return Err(PhysicsError::InvalidQuery);
        }
        self.require_horizontal_coverage(from, to, body)?;
        let distance = (to[0] - from[0]).hypot(to[2] - from[2]);
        if distance > 8.0 {
            return Err(PhysicsError::InvalidQuery);
        }
        let mut budget = WORK_LIMIT;
        let count = (distance / 0.25).ceil().max(1.0) as usize;
        let mut previous = from;
        for i in 0..=count {
            let f = i as f64 / count as f64;
            let probe = add(from, scale(sub(to, from), f));
            let Some(sample) = self.ground(
                probe,
                body.max_step + body.skin,
                body.max_step + body.ground_snap + body.skin,
                &mut budget,
            )?
            else {
                return Ok(false);
            };
            if sample.normal[1] < body.slope_cos()
                || (sample.position[1] + body.skin - probe[1]).abs()
                    > body.max_step + body.ground_snap + body.skin
            {
                return Ok(false);
            }
            let mut height = sample.position[1];
            for offset in [
                [-body.radius, -body.radius],
                [-body.radius, body.radius],
                [body.radius, -body.radius],
                [body.radius, body.radius],
            ] {
                let point = [probe[0] + offset[0], probe[1], probe[2] + offset[1]];
                let Some(g) = self.ground(
                    point,
                    body.max_step + body.skin,
                    body.max_step + body.ground_snap + body.skin,
                    &mut budget,
                )?
                else {
                    return Ok(false);
                };
                if g.normal[1] < body.slope_cos() {
                    return Ok(false);
                }
                height = height.max(g.position[1]);
            }
            // Center/corner rays establish sampled ground and slope policy, but
            // can miss a ridge inside the footprint. Use the same full-body
            // downward contact as locomotion to establish supported feet height.
            let high = [probe[0], height + body.skin + body.max_step, probe[2]];
            let low = [probe[0], height - body.ground_snap, probe[2]];
            let Some(contact) = self.sweep(high, low, body, &mut budget)? else {
                return Ok(false);
            };
            if contact.fraction <= 1e-9 || contact.normal[1] < body.slope_cos() {
                return Ok(false);
            }
            let next = [
                probe[0],
                contact.position[1] - body.height * 0.5 + body.skin,
                probe[2],
            ];
            if i == 0 {
                previous = next;
            }
            if (next[1] - previous[1]).abs() > body.max_step + body.ground_snap {
                return Ok(false);
            }
            // Lift to the higher endpoint plus step clearance, sweep up/across,
            // then down. This permits supported small steps, not wall shortcuts.
            self.require_horizontal_coverage(previous, next, body)?;
            if i > 0 && !self.edge_clear(previous, next, body, &mut budget)? {
                return Ok(false);
            }
            previous = next;
        }
        Ok(true)
    }
    fn edge_clear(
        &self,
        from: Point,
        to: Point,
        b: Body,
        budget: &mut usize,
    ) -> Result<bool, PhysicsError> {
        let top = from[1].max(to[1]) + b.max_step;
        let a = [from[0], top, from[2]];
        let c = [to[0], top, to[2]];
        if self.sweep(from, a, b, budget)?.is_some()
            || self.sweep(a, c, b, budget)?.is_some()
            || self.sweep(c, to, b, budget)?.is_some()
        {
            return Ok(false);
        }
        Ok(true)
    }
    /// Integrates at most one second in <=50ms substeps. Each substep permits
    /// at most three speculative, three rejected-step fallback, and three gravity
    /// contacts, all sharing the original strict 16,384 BVH work budget.
    /// Errors return no partially updated state, so callers retain old position.
    pub fn advance(
        &self,
        m: Motion,
        velocity: Point,
        dt: f64,
        b: Body,
    ) -> Result<Motion, PhysicsError> {
        self.advance_observed(m, velocity, dt, b, |_| {})
    }
    /// Observe every successful <=50ms substep, including an intermediate
    /// departure and landing in a longer call. Stage observations in a clone:
    /// a later query error aborts the whole call and its staged fall state.
    pub fn advance_observed(
        &self,
        mut m: Motion,
        velocity: Point,
        dt: f64,
        b: Body,
        mut observe: impl FnMut(&Motion),
    ) -> Result<Motion, PhysicsError> {
        b.validate()?;
        if !valid(m.position)
            || !valid(velocity)
            || !m.vertical_velocity.is_finite()
            || m.vertical_velocity.abs() > 100.0
            || m.slide_velocity
                .iter()
                .any(|v| !v.is_finite() || v.abs() > 100.0)
            || velocity[1] != 0.0
            || velocity[0].hypot(velocity[2]) > 128.0
            || !dt.is_finite()
            || dt <= 0.0
            || dt > 1.0
        {
            return Err(PhysicsError::InvalidQuery);
        }
        if velocity[0] != 0.0 || velocity[2] != 0.0 {
            self.require_horizontal_coverage(m.position, add(m.position, scale(velocity, dt)), b)?;
        }
        self.actor_contact.set(None);
        let count = (dt / SUBSTEP).ceil() as usize;
        let h = dt / count as f64;
        let mut budget = WORK_LIMIT;
        m.blocked = false;
        for _ in 0..count {
            // Establish support only from supplied ground. Airborne starts do not
            // climb or snap upward into a plane; a vertical sweep lands them.
            if m.grounded {
                // Contact belongs to the whole body footprint. A center ray can
                // miss a ramp/edge supporting a box corner and falsely disable
                // the step lift on every subsequent tick.
                let down = add(m.position, [0.0, -b.ground_snap - b.skin, 0.0]);
                let support = self.sweep(m.position, down, b, &mut budget)?;
                m.grounded = support.is_some_and(|g| g.normal[1] >= b.slope_cos());
            }
            let delta = [velocity[0] * h, 0.0, velocity[2] * h];
            let origin = m.position;
            let before_step = m;
            let contact_before_step = self.actor_contact.get();
            if delta[0] != 0.0 || delta[2] != 0.0 {
                self.require_horizontal_coverage(origin, add(origin, delta), b)?;
            }
            let mut lifted = false;
            if m.grounded && (delta[0] != 0.0 || delta[2] != 0.0) {
                let up = add(origin, [0.0, b.max_step, 0.0]);
                if self.sweep(origin, up, b, &mut budget)?.is_none() {
                    m.position = up;
                    lifted = true;
                }
            }
            let (p, blocked) = self.slide(m.position, delta, b, &mut budget)?;
            m.position = p;
            m.blocked |= blocked;
            if lifted {
                let down = add(m.position, [0.0, -b.max_step - b.ground_snap, 0.0]);
                if let Some(hit) = self.sweep(m.position, down, b, &mut budget)?
                    && hit.normal[1] >= b.slope_cos()
                {
                    m.position = stop_before(m.position, down, hit.fraction, b.skin);
                    m.grounded = true;
                    m.vertical_velocity = 0.0;
                    m.slide_velocity = [0.0; 2];
                    observe(&m);
                    continue;
                }
                // The lifted displacement was speculative. Restoring only Y
                // leaves its new XZ inside a steep side/overhang. Roll back the
                // whole attempt (including blocked status) and sweep normally.
                m = before_step;
                self.actor_contact.set(contact_before_step);
                let (p, blocked) = self.slide(origin, delta, b, &mut budget)?;
                m.position = p;
                m.blocked |= blocked;
                m.grounded = false;
            }
            if m.grounded {
                m.vertical_velocity = 0.0;
                m.slide_velocity = [0.0; 2];
                observe(&m);
                continue;
            }
            let movement = [
                m.slide_velocity[0],
                m.vertical_velocity - 0.5 * GRAVITY * h,
                m.slide_velocity[1],
            ];
            let falling = [
                m.slide_velocity[0],
                (m.vertical_velocity - GRAVITY * h).max(-55.0),
                m.slide_velocity[1],
            ];
            let (p, v, grounded, blocked) =
                self.descend(m.position, scale(movement, h), falling, b, &mut budget)?;
            m.position = p;
            m.vertical_velocity = v[1];
            m.slide_velocity = [v[0], v[2]];
            m.grounded = grounded;
            m.blocked |= blocked;
            observe(&m);
        }
        Ok(m)
    }
    /// Project gravity and its remaining displacement along steep surfaces.
    /// Walkable upward-facing contacts land; walls still block. Each projected
    /// leg is swept with the full body and shares the caller's original budget.
    fn descend(
        &self,
        mut p: Point,
        mut delta: Point,
        mut v: Point,
        b: Body,
        budget: &mut usize,
    ) -> Result<(Point, Point, bool, bool), PhysicsError> {
        let mut blocked = false;
        for _ in 0..3 {
            if length(delta) < 1e-9 {
                return Ok((p, v, false, blocked));
            }
            let end = add(p, delta);
            if delta[0] != 0.0 || delta[2] != 0.0 {
                self.require_horizontal_coverage(p, end, b)?;
            }
            let Some(hit) = self.sweep(p, end, b, budget)? else {
                return Ok((end, v, false, blocked));
            };
            self.actor_contact.set(
                hit.instance_id
                    .strip_prefix("dynamic-character:")
                    .and_then(|id| id.parse().ok()),
            );
            p = stop_before(p, end, hit.fraction, b.skin);
            let n = hit.normal;
            if delta[1] <= 0.0 && n[1] >= b.slope_cos() {
                return Ok((p, [0.0; 3], true, blocked));
            }
            // A steep upward-facing surface permits descent, but does not count
            // as standing support. Vertical walls/ceilings retain blocked status.
            blocked |= n[1] <= 1e-6;
            delta = scale(delta, 1.0 - hit.fraction);
            let into = dot(delta, n);
            if into < 0.0 {
                delta = sub(delta, scale(n, into));
            }
            let into = dot(v, n);
            if into < 0.0 {
                v = sub(v, scale(n, into));
            }
        }
        // Unresolved contact chains stop both remaining displacement and
        // momentum. Retaining repeatedly projected velocity at a stationary
        // blocked corner accumulates unsimulated speed on every gravity tick.
        // Keep airborne status: gravity and newly clear contact legs still run
        // normally on the next substep, without inventing standing support.
        Ok((p, [0.0; 3], false, true))
    }
    fn slide(
        &self,
        mut p: Point,
        mut delta: Point,
        b: Body,
        budget: &mut usize,
    ) -> Result<(Point, bool), PhysicsError> {
        let mut blocked = false;
        for _ in 0..3 {
            if length(delta) < 1e-9 {
                break;
            }
            let end = add(p, delta);
            let Some(hit) = self.sweep(p, end, b, budget)? else {
                p = end;
                break;
            };
            blocked = true;
            self.actor_contact.set(
                hit.instance_id
                    .strip_prefix("dynamic-character:")
                    .and_then(|id| id.parse().ok()),
            );
            p = stop_before(p, end, hit.fraction, b.skin);
            let mut normal = [hit.normal[0], 0.0, hit.normal[2]];
            let len = length(normal);
            if len < 1e-9 {
                break;
            }
            normal = scale(normal, 1.0 / len);
            delta = scale(delta, 1.0 - hit.fraction);
            let into = dot(delta, normal);
            if into >= 0.0 {
                break;
            }
            delta = sub(delta, scale(normal, into));
        }
        Ok((p, blocked))
    }
}
fn valid(p: Point) -> bool {
    p.iter().all(|v| v.is_finite() && v.abs() <= 10_000_000.0)
}
fn add(a: Point, b: Point) -> Point {
    std::array::from_fn(|i| a[i] + b[i])
}
fn sub(a: Point, b: Point) -> Point {
    std::array::from_fn(|i| a[i] - b[i])
}
fn scale(a: Point, s: f64) -> Point {
    a.map(|v| v * s)
}
fn dot(a: Point, b: Point) -> f64 {
    (0..3).map(|i| a[i] * b[i]).sum()
}
fn length(a: Point) -> f64 {
    dot(a, a).sqrt()
}
fn stop_before(a: Point, b: Point, f: f64, skin: f64) -> Point {
    let d = sub(b, a);
    add(a, scale(d, (f - skin / length(d).max(skin)).max(0.0)))
}
