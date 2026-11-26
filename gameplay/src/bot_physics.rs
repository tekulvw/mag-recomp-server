//! Bounded development bot locomotion over caller-supplied world triangles.
//! The body is a conservative upright box by default, or an upright capsule
//! selected by an external body profile (`BodyShape`); neither claims native
//! character-controller parity.
//! Supplied coverage/provenance remains authoritative only for that subset;
//! missing ground is a hole, never an invented plane. Optional player bodies use the same sweeps.
use crate::collision::{CollisionWorld, RayHit};
#[path = "bot_physics_shape.rs"]
mod shape;
use serde::{Deserialize, Serialize};
pub use shape::{BODY_PROFILE_FORMAT, BodyShape};
pub type Point = [f64; 3];
const WORK_LIMIT: usize = 16_384;
const SUBSTEP: f64 = 0.05;
const GRAVITY: f64 = 9.81;
/// Fastest a capsule body is moved out of another capsule body it overlaps.
/// Each of a crowded pair moves away on its own update, so they part over a
/// few updates instead of being thrown apart.
const DEPENETRATION_MPS: f64 = 1.5;

/// How a body stands: upright, or crouched (behind low cover).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Stance {
    Standing,
    Crouched,
}
/// Height of the default box body crouched; a crouched body of any shape
/// keeps this proportion of its standing height.
pub(crate) const CROUCH_HEIGHT_M: f64 = 1.2;
/// The height `body` presents to shots and sight lines in `stance`: its own
/// height standing (the box's 1.8 m, or a capsule's top above its feet), and
/// the box's crouched proportion of it crouched. The single source of every
/// torso, eye, muzzle and aim point (torso = feet + 60% of this height). The
/// box gives exactly 1.8 m and `CROUCH_HEIGHT_M`.
pub(crate) fn stance_height(body: Body, stance: Stance) -> f64 {
    match stance {
        Stance::Standing => body.height,
        Stance::Crouched => CROUCH_HEIGHT_M * (body.height / Body::default().height),
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhysicsError {
    Unavailable,
    MapMismatch,
    QueryFailed,
    InvalidQuery,
    CoverageUnknown,
    RegionalBlocked,
}
impl PhysicsError {
    pub fn code(self) -> &'static str {
        match self {
            Self::Unavailable => "bot_physics_geometry_unavailable",
            Self::MapMismatch => "bot_physics_map_mismatch",
            Self::QueryFailed => "bot_physics_query_failed_or_budget",
            Self::InvalidQuery => "bot_physics_invalid_query",
            Self::RegionalBlocked => "bot_regional_collision_blocked",
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
    /// The swept shape. For a capsule, `radius` and `height` are its radius
    /// and the height of its top above the feet (set by `with_shape`), so
    /// footprint and coverage bounds stay correct for either shape.
    pub shape: BodyShape,
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
            shape: BodyShape::Box,
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
            || !self.shape.valid()
            || self.shape.capsule().is_some_and(|(capsule, centre)| {
                self.radius != capsule.radius
                    || self.height != centre + capsule.half_height + capsule.radius
            })
        {
            return Err(PhysicsError::InvalidQuery);
        }
        Ok(())
    }
    /// This body with `shape`. A capsule also sets `radius` and `height` from
    /// its dimensions; the box keeps the body's own.
    pub fn with_shape(self, shape: BodyShape) -> Self {
        match shape.capsule() {
            None => Self { shape, ..self },
            Some((capsule, centre)) => Self {
                radius: capsule.radius,
                height: centre + capsule.half_height + capsule.radius,
                shape,
                ..self
            },
        }
    }
    fn slope_cos(self) -> f64 {
        self.max_slope_degrees.to_radians().cos()
    }
    /// Height above the feet of the point the sweeps move: the box's centre
    /// or the capsule's.
    fn centre_height(self) -> f64 {
        match self.shape.capsule() {
            None => self.height * 0.5,
            Some((_, centre)) => centre,
        }
    }
    /// Lowest point of the body relative to its feet: zero for the box, the
    /// capsule's rounded bottom otherwise (it may sit slightly below the feet).
    pub(crate) fn bottom(self) -> f64 {
        match self.shape.capsule() {
            None => 0.0,
            Some((capsule, centre)) => centre - capsule.half_height - capsule.radius,
        }
    }
    fn is_capsule(self) -> bool {
        self.shape != BodyShape::Box
    }
    /// How far a capsule starting with its feet on the ground may need to
    /// rise to clear it: the depth its lower hemisphere reaches into a plane
    /// through the feet at the slope limit (on flat ground, just its reach
    /// below the feet), at most a step, plus the skin. None for the box,
    /// which starts only where it stands free.
    pub(crate) fn settle_lift(self) -> Option<f64> {
        let (capsule, centre) = self.shape.capsule()?;
        let sphere = centre - capsule.half_height;
        let depth = (capsule.radius / self.slope_cos() - sphere)
            .max(0.0)
            .min(self.max_step);
        (depth > 0.0).then_some(depth + self.skin)
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
#[derive(Clone, Debug)]
pub(crate) struct RegionalGuard {
    region: crate::actor_collision::Bounds,
    obstacles: Vec<crate::actor_collision::Bounds>,
    walking_coverage: bool,
}
impl RegionalGuard {
    pub(crate) fn new(
        region: crate::actor_collision::Bounds,
        obstacles: &[crate::actor_collision::Bounds],
    ) -> Self {
        Self {
            region,
            obstacles: obstacles.to_vec(),
            walking_coverage: false,
        }
    }

    pub(crate) fn with_walking_coverage(mut self) -> Self {
        self.walking_coverage = true;
        self
    }

    fn check_bounds(&self, range: crate::actor_collision::Bounds) -> Result<(), PhysicsError> {
        if (0..3).any(|i| range.min[i] < self.region.min[i] || range.max[i] > self.region.max[i]) {
            return Err(PhysicsError::RegionalBlocked);
        }
        if self
            .obstacles
            .iter()
            .any(|body| (0..3).all(|i| range.min[i] <= body.max[i] && body.min[i] <= range.max[i]))
        {
            return Err(PhysicsError::RegionalBlocked);
        }
        Ok(())
    }

    fn check(&self, from: Point, to: Point, body: Body) -> Result<(), PhysicsError> {
        use crate::actor_collision::Bounds;
        self.check_bounds(Bounds::body(from, body).union(Bounds::body(to, body)))
    }
}

pub struct WorldTerrain<'a> {
    world: &'a CollisionWorld,
    actors: Option<crate::actor_collision::Query<'a>>,
    actor_contact: std::cell::Cell<Option<u16>>,
    actor_touched: std::cell::Cell<Option<u16>>,
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
            actor_touched: std::cell::Cell::new(None),
        })
    }
    pub(crate) fn with_actors(mut self, query: crate::actor_collision::Query<'a>) -> Self {
        self.actors = Some(query);
        self
    }
    pub(crate) fn actor_contact(&self) -> Option<u16> {
        self.actor_contact.get()
    }
    /// The last body the latest `advance` touched, even when a later contact
    /// in the same call was static geometry (`actor_contact` is then None).
    pub(crate) fn actor_touched(&self) -> Option<u16> {
        self.actor_touched.get()
    }
    fn note_contact(&self, hit: &RayHit) {
        let id = hit
            .instance_id
            .strip_prefix("dynamic-character:")
            .and_then(|id| id.parse().ok());
        self.actor_contact.set(id);
        if id.is_some() {
            self.actor_touched.set(id);
        }
    }
    /// Move a capsule body that overlaps other capsule bodies part of the way
    /// out, at most `DEPENETRATION_MPS` for `h` seconds, sliding along static
    /// geometry and other bodies (never into them). The box, a body overlapping
    /// nothing, and a move the world cannot cover are left alone; contacts
    /// made while parting are not reported.
    fn depenetrate(&self, position: Point, h: f64, b: Body, budget: &mut usize) -> Point {
        let Some(query) = self.actors.filter(|_| b.is_capsule()) else {
            return position;
        };
        let Ok(Some(push)) = query.grid.separation(query.actor, position, b, budget) else {
            return position;
        };
        let depth = push[0].hypot(push[2]);
        if depth <= 1e-9 {
            return position;
        }
        let step = (depth + b.skin).min(DEPENETRATION_MPS * h);
        let delta = [push[0] / depth * step, 0.0, push[2] / depth * step];
        if self
            .require_horizontal_coverage(position, add(position, delta), b)
            .is_err()
        {
            return position;
        }
        let (contact, touched) = (self.actor_contact.get(), self.actor_touched.get());
        let parted = self.slide(position, delta, b, budget);
        self.actor_contact.set(contact);
        self.actor_touched.set(touched);
        parted.map_or(position, |(p, _)| p)
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
        let mut margin = [body.radius, body.ground_snap + body.skin, body.radius];
        if body.is_capsule() {
            // A capsule's rounded bottom may reach slightly below its feet.
            margin[1] -= body.bottom().min(0.0);
        }
        let upper = [
            body.radius,
            body.height + body.max_step + body.skin,
            body.radius,
        ];
        let min = std::array::from_fn(|i| from[i].min(to[i]) - margin[i]);
        let max = std::array::from_fn(|i| from[i].max(to[i]) + upper[i]);
        if let Some(guard) = self
            .world
            .bot_regional_guard()
            .filter(|guard| guard.walking_coverage)
        {
            let region = &guard.region;
            return (0..3)
                .all(|i| min[i] >= region.min[i] && max[i] <= region.max[i])
                .then_some(())
                .ok_or(PhysicsError::CoverageUnknown);
        }
        // World regions and any hash-bound coverage sidecar form one set.
        if self.world.covers_box(min, max) {
            Ok(())
        } else {
            Err(PhysicsError::CoverageUnknown)
        }
    }
    /// Refuses known initial penetration. Surface tangency is allowed; no
    /// depenetration/ground plane is invented to make an invalid spawn usable.
    ///
    /// A capsule's rounded bottom can overlap the ground its feet point stands
    /// on: below the feet when the capsule reaches under them, and on any
    /// walkable slope. Such a capsule starts settled instead: lifted by at most
    /// `Body::settle_lift` (its rounded bottom's reach below the feet on ground
    /// at the slope limit), which must be free, then swept back down onto the
    /// ground. Any other overlap is still refused, and the box is unchanged.
    pub fn start_motion(&self, position: Point, body: Body) -> Result<Motion, PhysicsError> {
        body.validate()?;
        if !valid(position) {
            return Err(PhysicsError::InvalidQuery);
        }
        let mut budget = WORK_LIMIT;
        if self.sweep(position, position, body, &mut budget)?.is_none() {
            return Ok(Motion::new(position));
        }
        let Some(lift) = body.settle_lift() else {
            return Err(PhysicsError::InvalidQuery);
        };
        let up = [position[0], position[1] + lift, position[2]];
        if self.sweep(up, up, body, &mut budget)?.is_some() {
            return Err(PhysicsError::InvalidQuery);
        }
        let hit = self
            .sweep(up, position, body, &mut budget)?
            .ok_or(PhysicsError::InvalidQuery)?;
        Ok(Motion::new(stop_before(
            up,
            position,
            hit.fraction,
            body.skin,
        )))
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
        if self
            .world
            .bot_regional_guard()
            .is_some_and(|guard| guard.walking_coverage)
        {
            return Err(PhysicsError::CoverageUnknown);
        }
        self.require_horizontal_coverage(launch, landing, b)?;
        let mut m = self.start_motion(launch, b)?;
        // Verify stable source support instead of trusting a navmesh vertex Y,
        // measured from where the body starts (a capsule starts settled just
        // above the vertex; the box exactly at it).
        let settled = m.position;
        for _ in 0..20 {
            m = self.advance(m, [0.0; 3], SUBSTEP, b)?;
            if m.blocked || length(sub(m.position, settled)) > 0.05 {
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
        if let Some(guard) = self.world.bot_regional_guard() {
            guard.check_bounds(crate::actor_collision::Bounds {
                min: end,
                max: start,
            })?;
        }
        let mut hit = self
            .world
            .movement_raycast_bounded(start, end, budget)
            .map_err(|_| PhysicsError::QueryFailed)?
            .nearest
            .map(|h| (h.fraction, h.position, h.normal));
        // Placed structures (bunker floors and roofs, ramps) carry a body too.
        if let Some(structures) = self.world.bot_structures() {
            let mut work = crate::bot_structure_collision::STRUCTURE_WORK_LIMIT;
            if let Some(found) = structures
                .raycast(start, end, &mut work)
                .map_err(|_| PhysicsError::QueryFailed)?
                && hit.is_none_or(|(f, ..)| found.0 < f)
            {
                hit = Some(found);
            }
        }
        Ok(hit.map(|(_, position, normal)| GroundSample {
            position,
            normal: normal.map(|v| v * if normal[1] < 0.0 { -1.0 } else { 1.0 }),
        }))
    }
    fn sweep(
        &self,
        from: Point,
        to: Point,
        body: Body,
        budget: &mut usize,
    ) -> Result<Option<RayHit>, PhysicsError> {
        if let Some(guard) = self.world.bot_regional_guard() {
            guard.check(from, to, body)?;
        }
        let lift = body.centre_height();
        let center = |p: Point| [p[0], p[1] + lift, p[2]];
        let mut nearest = match body.shape.capsule() {
            None => self.world.movement_sweep_aabb(
                center(from),
                center(to),
                [body.radius, body.height * 0.5, body.radius],
                budget,
            ),
            // The capsule hit's normal is its contact normal: the face normal
            // on a face, otherwise from the touched edge or corner toward the
            // axis, so a lip or crest supports the body only as it would a
            // rounded body.
            Some((capsule, _)) => self
                .world
                .movement_sweep_capsule(center(from), center(to), capsule, budget)
                .map(|hit| hit.map(|h| h.hit)),
        }
        .map_err(|_| PhysicsError::QueryFailed)?;
        // The mode's placed structures in their current state: intact walls,
        // gates and shells stop the body like static geometry does. Both body
        // shapes sweep their box (radius, height): conservative for a capsule,
        // whose lowest point may sit just below the feet.
        if let Some(structures) = self.world.bot_structures() {
            let mut work = crate::bot_structure_collision::STRUCTURE_WORK_LIMIT;
            let low = if body.shape.capsule().is_some() {
                body.bottom().min(0.0)
            } else {
                0.0
            };
            let half = (body.height - low) * 0.5;
            let middle = |p: Point| [p[0], p[1] + low + half, p[2]];
            if let Some((fraction, normal, label)) = structures
                .sweep_aabb(
                    middle(from),
                    middle(to),
                    [body.radius, half, body.radius],
                    &mut work,
                )
                .map_err(|_| PhysicsError::QueryFailed)?
                && nearest.as_ref().is_none_or(|h| fraction < h.fraction)
            {
                let mut position: Point =
                    std::array::from_fn(|i| from[i] + (to[i] - from[i]) * fraction);
                position[1] += lift;
                nearest = Some(RayHit {
                    instance_id: format!("structure:{label}"),
                    triangle_index: 0,
                    fraction,
                    position,
                    normal,
                });
            }
        }
        if let Some(query) = self.actors
            && let Some(hit) = query
                .grid
                .sweep(query.actor, from, to, body, budget)
                .map_err(|_| PhysicsError::QueryFailed)?
            && nearest.as_ref().is_none_or(|h| hit.fraction < h.fraction)
        {
            let mut position = std::array::from_fn(|i| from[i] + (to[i] - from[i]) * hit.fraction);
            position[1] += lift;
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
            // The box's corners, or four points on the capsule's rim.
            let reach = if body.is_capsule() {
                body.radius * std::f64::consts::FRAC_1_SQRT_2
            } else {
                body.radius
            };
            for offset in [
                [-reach, -reach],
                [-reach, reach],
                [reach, -reach],
                [reach, reach],
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
            let mut high = [probe[0], height + body.skin + body.max_step, probe[2]];
            let low = [probe[0], height - body.ground_snap, probe[2]];
            if body.is_capsule() {
                // Start the capsule's contact below any overhead obstacle it
                // fits under, as its headroom-limited step lift does.
                let base = [probe[0], height + body.skin, probe[2]];
                if let Some(hit) = self.sweep(base, high, body, &mut budget)? {
                    high = stop_before(base, high, hit.fraction, body.skin);
                }
            }
            let Some(contact) = self.sweep(high, low, body, &mut budget)? else {
                return Ok(false);
            };
            if contact.fraction <= 1e-9 || contact.normal[1] < body.slope_cos() {
                return Ok(false);
            }
            let next = [
                probe[0],
                contact.position[1] - body.centre_height() + body.skin,
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
        let mut top = from[1].max(to[1]) + b.max_step;
        if b.is_capsule() {
            // Lift only as far as the headroom over both ends allows, as
            // locomotion does, but at least to the higher end.
            let ceiling = top;
            for p in [from, to] {
                let up = [p[0], ceiling, p[2]];
                if let Some(hit) = self.sweep(p, up, b, budget)? {
                    top = top.min(stop_before(p, up, hit.fraction, b.skin)[1]);
                }
            }
            if top < from[1].max(to[1]) {
                return Ok(false);
            }
        }
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
        self.actor_touched.set(None);
        let count = (dt / SUBSTEP).ceil() as usize;
        let h = dt / count as f64;
        let mut budget = WORK_LIMIT;
        m.blocked = false;
        for _ in 0..count {
            m.position = self.depenetrate(m.position, h, b, &mut budget);
            // Establish support only from supplied ground. Airborne starts do not
            // climb or snap upward into a plane; a vertical sweep lands them.
            let mut support = None;
            if m.grounded {
                // Contact belongs to the whole body footprint. A center ray can
                // miss a ramp/edge supporting a box corner and falsely disable
                // the step lift on every subsequent tick.
                let down = add(m.position, [0.0, -b.ground_snap - b.skin, 0.0]);
                support = match self.sweep(m.position, down, b, &mut budget)? {
                    Some(hit) => self.support_normal(m.position, &hit, b, &mut budget)?,
                    None => None,
                };
                m.grounded = support.is_some();
            }
            let delta = [velocity[0] * h, 0.0, velocity[2] * h];
            let origin = m.position;
            let before_step = m;
            let contact_before_step = self.actor_contact.get();
            if delta[0] != 0.0 || delta[2] != 0.0 {
                self.require_horizontal_coverage(origin, add(origin, delta), b)?;
            }
            let mut lifted = false;
            let mut lift = b.max_step;
            if m.grounded && (delta[0] != 0.0 || delta[2] != 0.0) {
                let up = add(origin, [0.0, b.max_step, 0.0]);
                match self.sweep(origin, up, b, &mut budget)? {
                    None => {
                        m.position = up;
                        lifted = true;
                    }
                    // A capsule lifts as far as its headroom allows: under a
                    // beam it fits beneath, its rounded bottom still clears a
                    // lower lip.
                    Some(hit) if b.is_capsule() => {
                        let top = stop_before(origin, up, hit.fraction, b.skin);
                        if top[1] - origin[1] > b.skin {
                            lift = top[1] - origin[1];
                            m.position = top;
                            lifted = true;
                        }
                    }
                    Some(_) => {}
                }
            }
            let (p, blocked) = self.slide(m.position, delta, b, &mut budget)?;
            m.position = p;
            m.blocked |= blocked;
            if lifted {
                let down = add(m.position, [0.0, -lift - b.ground_snap, 0.0]);
                if let Some(hit) = self.sweep(m.position, down, b, &mut budget)?
                    && self.stands_on(
                        stop_before(m.position, down, hit.fraction, b.skin),
                        &hit,
                        b,
                        &mut budget,
                    )?
                {
                    m.position = stop_before(m.position, down, hit.fraction, b.skin);
                    m.grounded = true;
                    let run = delta[0].hypot(delta[2]);
                    let made = (m.position[0] - origin[0]).hypot(m.position[2] - origin[2]);
                    // The step lift raises the box's top by a step too. Under a
                    // beam or lintel the lifted body can be stopped where the
                    // unlifted one passes, so a stalled step also tries the move
                    // along the ground.
                    if blocked
                        && made < 0.5 * run
                        && let Some(normal) = support
                        && let Some(feet) =
                            self.surface_slide(origin, delta, normal, b, &mut budget)?
                        && (feet[0] - origin[0]).hypot(feet[2] - origin[2]) > made + 0.25 * run
                    {
                        m = before_step;
                        self.actor_contact.set(contact_before_step);
                        m.position = feet;
                        m.grounded = true;
                    }
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
            let run = delta[0].hypot(delta[2]);
            let made = (m.position[0] - origin[0]).hypot(m.position[2] - origin[2]);
            if m.blocked
                && run > 1e-9
                && made < 0.5 * run
                && let Some(normal) = support
                && let Some(feet) = self.surface_slide(origin, delta, normal, b, &mut budget)?
                && (feet[0] - origin[0]).hypot(feet[2] - origin[2]) > made + 0.25 * run
            {
                m = before_step;
                self.actor_contact.set(contact_before_step);
                m.position = feet;
                m.grounded = true;
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
    /// Whether a body whose feet rest at `feet`, with `hit` its first downward
    /// contact, stands there. A walkable contact holds it. So does an upward
    /// steep contact at the edge of its footprint (a rock lip, a kerb, a seam
    /// between terrain facets) when the ground under the body's centre is
    /// walkable and within a step below its feet: a rounded body standing
    /// there rests on that ground, and only the box's corner touches the lip.
    /// A body on steep ground throughout still slides.
    fn stands_on(
        &self,
        feet: Point,
        hit: &RayHit,
        b: Body,
        budget: &mut usize,
    ) -> Result<bool, PhysicsError> {
        Ok(self.support_normal(feet, hit, b, budget)?.is_some())
    }
    /// The walkable surface a standing body rests on (see `stands_on`).
    fn support_normal(
        &self,
        feet: Point,
        hit: &RayHit,
        b: Body,
        budget: &mut usize,
    ) -> Result<Option<Point>, PhysicsError> {
        if hit.normal[1] >= b.slope_cos() {
            return Ok(Some(hit.normal));
        }
        // A capsule's contact normal already says how a lip or crest holds
        // it; only the flat-bottomed box needs the ground under its centre.
        if b.is_capsule() || hit.normal[1] <= 1e-6 {
            return Ok(None);
        }
        Ok(self
            .ground(feet, b.skin, b.max_step + b.ground_snap + b.skin, budget)?
            .map(|g| g.normal)
            .filter(|n| n[1] >= b.slope_cos()))
    }
    /// A grounded move the stepping box could not make: carry it along the
    /// supporting surface instead and slide it off every contact in three
    /// dimensions, so a body pressed under a leaning overhang or into a
    /// slope seam moves along them as a rounded body would. It must end
    /// standing on walkable ground within a step; it never climbs a surface
    /// steeper than the body's slope limit, since only walkable support
    /// carries it upward.
    fn surface_slide(
        &self,
        origin: Point,
        delta: Point,
        support: Point,
        b: Body,
        budget: &mut usize,
    ) -> Result<Option<Point>, PhysicsError> {
        if support[1] < b.slope_cos() {
            return Ok(None);
        }
        // Follow the support plane: the same horizontal motion, with the rise
        // or fall the plane gives it.
        let rise = -(support[0] * delta[0] + support[2] * delta[2]) / support[1];
        let mut d = [delta[0], rise, delta[2]];
        // Leave the supporting surface by a hair, as far as headroom allows.
        let clearance = 2.0 * b.skin;
        let lift = add(origin, [0.0, clearance, 0.0]);
        let mut p = match self.sweep(origin, lift, b, budget)? {
            Some(hit) => stop_before(origin, lift, hit.fraction, b.skin),
            None => lift,
        };
        // The support is the first plane the move is held to; each contact
        // adds another, and between two planes the move follows their crease.
        let mut previous = support;
        for _ in 0..4 {
            if length(d) < 1e-9 {
                break;
            }
            let end = add(p, d);
            let Some(hit) = self.sweep(p, end, b, budget)? else {
                p = end;
                break;
            };
            if hit.instance_id.starts_with("dynamic-character:") {
                return Ok(None);
            }
            p = stop_before(p, end, hit.fraction, b.skin);
            d = scale(d, 1.0 - hit.fraction);
            let n = hit.normal;
            let into = dot(d, n);
            if into < 0.0 {
                d = sub(d, scale(n, into));
            }
            if dot(d, previous) < 0.0 {
                let crease = cross(previous, n);
                let len = length(crease);
                if len < 1e-6 {
                    break;
                }
                let crease = scale(crease, 1.0 / len);
                d = scale(crease, dot(d, crease));
            }
            previous = n;
            // Contacts may not carry the body up faster than walkable ground.
            if d[1] > 0.0 {
                let run = d[0].hypot(d[2]);
                d[1] = d[1].min(run * b.slope_cos().acos().tan());
            }
        }
        let down = add(p, [0.0, -b.max_step - b.ground_snap, 0.0]);
        let Some(hit) = self.sweep(p, down, b, budget)? else {
            return Ok(None);
        };
        let feet = stop_before(p, down, hit.fraction, b.skin);
        if !self.stands_on(feet, &hit, b, budget)? {
            return Ok(None);
        }
        Ok(Some(feet))
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
            self.note_contact(&hit);
            p = stop_before(p, end, hit.fraction, b.skin);
            let n = hit.normal;
            if delta[1] <= 0.0 && self.stands_on(p, &hit, b, budget)? {
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
            self.note_contact(&hit);
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
fn cross(a: Point, b: Point) -> Point {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
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

#[cfg(test)]
mod regional_guard_tests {
    use super::*;
    use crate::actor_collision::Bounds;

    #[test]
    fn regional_guard_blocks_a_swept_intruder_and_permits_a_cleared_path() {
        let region = Bounds {
            min: [-2., -2., -2.],
            max: [8., 8., 8.],
        };
        let obstacle = Bounds {
            min: [2., 2., -1.],
            max: [3., 3., 1.],
        };
        let body = Body::default();
        let guard = RegionalGuard::new(region, &[obstacle]);
        assert!(guard.check([0., 0., 0.], [0., 0., 0.], body).is_ok());
        assert!(guard.check([0., 0.4, 0.], [4., 0.4, 0.], body).is_err());
        assert!(
            RegionalGuard::new(region, &[])
                .check([0., 0.4, 0.], [4., 0.4, 0.], body)
                .is_ok()
        );
        assert!(guard.check([0., 0., 0.], [8., 0., 0.], body).is_err());
    }
}
#[cfg(test)]
#[path = "bot_physics_capsule_tests.rs"]
mod capsule_tests;
#[cfg(test)]
#[path = "bot_physics_steep_tests.rs"]
mod steep_tests;
#[cfg(test)]
mod stance_tests {
    use super::*;

    #[test]
    fn the_box_keeps_its_standing_and_crouched_heights_exactly() {
        let body = Body::default();
        assert_eq!(stance_height(body, Stance::Standing), 1.8);
        assert_eq!(stance_height(body, Stance::Crouched), CROUCH_HEIGHT_M);
        // A runtime box with another slope limit is the same box.
        let mesh_box = Body {
            max_slope_degrees: 54.,
            ..Default::default()
        }
        .with_shape(BodyShape::Box);
        assert_eq!(stance_height(mesh_box, Stance::Standing), 1.8);
        assert_eq!(stance_height(mesh_box, Stance::Crouched), CROUCH_HEIGHT_M);
        let feet = [3., 2., 1.];
        assert_eq!(
            crate::bot_perception::torso(feet, body),
            [3., 2. + 1.8 * 0.6, 1.]
        );
    }

    #[test]
    fn a_capsule_stands_and_crouches_at_its_own_height() {
        let capsule = Body::default().with_shape(BodyShape::Capsule {
            radius: 0.43,
            half_height: 0.51,
            centre_height: 0.91,
        });
        let standing = stance_height(capsule, Stance::Standing);
        assert!((standing - 1.85).abs() < 1e-12, "{standing}");
        let crouched = stance_height(capsule, Stance::Crouched);
        assert!((crouched - 1.2 * 1.85 / 1.8).abs() < 1e-12, "{crouched}");
        let torso = crate::bot_perception::torso([0.; 3], capsule);
        assert!((torso[1] - 1.85 * 0.6).abs() < 1e-12, "{torso:?}");
    }
}
