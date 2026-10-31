//! Bounded deterministic terrain-query path planning for future bot goal requests.
//! This module does not publish poses or infer traversability from straight lines.

use std::{
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet, BinaryHeap},
    sync::Arc,
};

pub type Point = [f64; 3];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueryError {
    Unavailable,
    Invalid,
    Limit,
    Failed,
}

/// Implementations must validate the full body/ground corridor, including cliffs.
/// Results must remain stable for the lifetime of one planning request.
pub trait Traversability {
    fn ground(&self, probe: Point) -> Result<Option<Point>, QueryError>;
    fn traversable(&self, from: Point, to: Point) -> Result<bool, QueryError>;
}

/// Restricts the requested X/Z search domain; it never grants geometry coverage.
/// Physics metadata remains the authority for verified ground and obstacles.
#[derive(Clone, Copy, Debug)]
pub struct PlanningBounds {
    pub min_xz: [f64; 2],
    pub max_xz: [f64; 2],
}

/// Read-only adapter to the same terrain/body queries used by bot physics.
/// The caller supplies explicit vertical projection bounds for this single-layer
/// grid; the adapter never fabricates terrain or promotes ground-only coverage.
pub struct WorldNavigation<'a> {
    terrain: crate::bot_physics::WorldTerrain<'a>,
    body: crate::bot_physics::Body,
    min_xz: [f64; 2],
    max_xz: [f64; 2],
    up: f64,
    down: f64,
}
fn physics_error(error: crate::bot_physics::PhysicsError) -> QueryError {
    use crate::bot_physics::PhysicsError;
    match error {
        PhysicsError::Unavailable | PhysicsError::MapMismatch | PhysicsError::CoverageUnknown => {
            QueryError::Unavailable
        }
        PhysicsError::InvalidQuery => QueryError::Invalid,
        PhysicsError::QueryFailed => QueryError::Failed,
    }
}
impl<'a> WorldNavigation<'a> {
    pub fn new(
        world: Option<&'a crate::collision::CollisionWorld>,
        map_name: &str,
        body: crate::bot_physics::Body,
        bounds: PlanningBounds,
        up: f64,
        down: f64,
    ) -> Result<Self, PlanError> {
        body.validate().map_err(|e| query_error(physics_error(e)))?;
        let PlanningBounds { min_xz, max_xz } = bounds;
        if !up.is_finite()
            || !down.is_finite()
            || up < 0.0
            || down < 0.0
            || up + down <= 0.0
            || up + down > 64.0
            || (0..2).any(|i| {
                !min_xz[i].is_finite()
                    || !max_xz[i].is_finite()
                    || min_xz[i] >= max_xz[i]
                    || min_xz[i].abs() > 10_000_000.0
                    || max_xz[i].abs() > 10_000_000.0
            })
        {
            return Err(PlanError::InvalidRequest);
        }
        let terrain = crate::bot_physics::WorldTerrain::new(world, map_name)
            .map_err(|e| query_error(physics_error(e)))?;
        Ok(Self {
            terrain,
            body,
            min_xz,
            max_xz,
            up,
            down,
        })
    }
    fn covered(&self, p: Point) -> Result<(), QueryError> {
        if !valid(p) {
            return Err(QueryError::Invalid);
        }
        if [p[0], p[2]].iter().enumerate().any(|(i, v)| {
            *v - self.body.radius < self.min_xz[i] || *v + self.body.radius > self.max_xz[i]
        }) {
            return Err(QueryError::Unavailable);
        }
        Ok(())
    }
}
impl Traversability for WorldNavigation<'_> {
    fn ground(&self, probe: Point) -> Result<Option<Point>, QueryError> {
        self.covered(probe)?;
        self.terrain
            .sample_ground(probe, self.up, self.down)
            .map(|g| g.map(|g| g.position))
            .map_err(physics_error)
    }
    fn traversable(&self, from: Point, to: Point) -> Result<bool, QueryError> {
        self.covered(from)?;
        self.covered(to)?;
        // Physics checks local edges up to 8m. Large configured grid cells may
        // still use their cardinal edges; a too-long diagonal is never assumed.
        if (to[0] - from[0]).hypot(to[2] - from[2]) > 8.0 {
            return Ok(false);
        }
        self.terrain
            .can_traverse(from, to, self.body)
            .map_err(physics_error)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub cell_size: f64,
    pub radius_cells: i32,
    pub max_expansions: usize,
    pub max_queries: usize,
    pub max_waypoints: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            cell_size: 1.0,
            radius_cells: 32,
            max_expansions: 2048,
            max_queries: 16384,
            max_waypoints: 1024,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanError {
    InvalidRequest,
    InvalidStart,
    InvalidGoal,
    GeometryUnavailable,
    QueryLimit,
    QueryFailed,
    SearchLimit,
    RouteLimit,
    Unreachable,
    RouteInvalidated,
    ReplanLimit,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Route {
    pub waypoints: Vec<Point>,
    pub expansions: usize,
    pub queries: usize,
}
type Cell = (i32, i32);

impl Limits {
    fn validate(self) -> Result<(), PlanError> {
        if !self.cell_size.is_finite()
            || !(0.25..=8.0).contains(&self.cell_size)
            || !(1..=128).contains(&self.radius_cells)
            || !(1..=16384).contains(&self.max_expansions)
            || !(1..=262144).contains(&self.max_queries)
            || !(1..=4096).contains(&self.max_waypoints)
        {
            return Err(PlanError::InvalidRequest);
        }
        Ok(())
    }
}
fn valid(p: Point) -> bool {
    p.iter().all(|v| v.is_finite() && v.abs() <= 10_000_000.0)
}
fn query_error(e: QueryError) -> PlanError {
    match e {
        QueryError::Unavailable => PlanError::GeometryUnavailable,
        QueryError::Invalid => PlanError::InvalidRequest,
        QueryError::Limit => PlanError::QueryLimit,
        QueryError::Failed => PlanError::QueryFailed,
    }
}
struct Search<'a, Q: ?Sized> {
    query: &'a Q,
    start: Point,
    limits: Limits,
    queries: usize,
    ground: BTreeMap<Cell, Option<Point>>,
    edges: BTreeMap<(Cell, Cell), bool>,
}
impl<Q: Traversability + ?Sized> Search<'_, Q> {
    fn charge(&mut self) -> Result<(), PlanError> {
        if self.queries >= self.limits.max_queries {
            return Err(PlanError::QueryLimit);
        }
        self.queries += 1;
        Ok(())
    }
    fn project(&mut self, p: Point) -> Result<Option<Point>, PlanError> {
        self.charge()?;
        let found = self.query.ground(p).map_err(query_error)?;
        if found
            .is_some_and(|v| !valid(v) || (v[0] - p[0]).abs() > 1e-7 || (v[2] - p[2]).abs() > 1e-7)
        {
            return Err(PlanError::InvalidRequest);
        }
        Ok(found)
    }
    fn segment(&mut self, a: Point, b: Point) -> Result<bool, PlanError> {
        self.charge()?;
        self.query.traversable(a, b).map_err(query_error)
    }
    fn point(&mut self, c: Cell) -> Result<Option<Point>, PlanError> {
        if let Some(v) = self.ground.get(&c) {
            return Ok(*v);
        }
        let p = [
            self.start[0] + f64::from(c.0) * self.limits.cell_size,
            self.start[1],
            self.start[2] + f64::from(c.1) * self.limits.cell_size,
        ];
        let found = self.project(p)?;
        self.ground.insert(c, found);
        Ok(found)
    }
    fn edge(&mut self, a: Cell, b: Cell) -> Result<bool, PlanError> {
        if let Some(v) = self.edges.get(&(a, b)) {
            return Ok(*v);
        }
        let allowed = match (self.point(a)?, self.point(b)?) {
            (Some(from), Some(to)) => self.segment(from, to)?,
            _ => false,
        };
        self.edges.insert((a, b), allowed);
        Ok(allowed)
    }
    fn neighbor(&mut self, a: Cell, b: Cell) -> Result<bool, PlanError> {
        if !self.edge(a, b)? {
            return Ok(false);
        }
        if a.0 != b.0 && a.1 != b.1 {
            // Both side cells and both side corridors must be safe. A clear
            // diagonal alone does not establish clearance around a corner.
            let x = (b.0, a.1);
            let z = (a.0, b.1);
            return Ok(self.edge(a, x)?
                && self.edge(x, b)?
                && self.edge(a, z)?
                && self.edge(z, b)?);
        }
        Ok(true)
    }
}
fn heuristic(a: Cell, b: Cell) -> u32 {
    let x = a.0.abs_diff(b.0);
    let z = a.1.abs_diff(b.1);
    1414 * x.min(z) + 1000 * x.abs_diff(z)
}
/// Grid coordinates are relative to the actual start, with a checked final
/// connection to the exact goal. Failure proves only no route in this bounded
/// grid/search budget. No fallback treats missing geometry as open space.
/// Per-search caches deliberately expire when the query's world may change.
pub fn plan_route<Q: Traversability + ?Sized>(
    query: &Q,
    start: Point,
    goal: Point,
    limits: Limits,
) -> Result<Route, PlanError> {
    limits.validate()?;
    if !valid(start) || !valid(goal) {
        return Err(PlanError::InvalidRequest);
    }
    let offset = (
        (goal[0] - start[0]) / limits.cell_size,
        (goal[2] - start[2]) / limits.cell_size,
    );
    if offset.0.abs() > f64::from(limits.radius_cells)
        || offset.1.abs() > f64::from(limits.radius_cells)
    {
        return Err(PlanError::InvalidGoal);
    }
    let target = (offset.0.round() as i32, offset.1.round() as i32);
    let mut search = Search {
        query,
        start,
        limits,
        queries: 0,
        ground: BTreeMap::new(),
        edges: BTreeMap::new(),
    };
    let start = search.project(start)?.ok_or(PlanError::InvalidStart)?;
    let goal = search.project(goal)?.ok_or(PlanError::InvalidGoal)?;
    search.start = start;
    search.ground.insert((0, 0), Some(start));
    let mut open = BinaryHeap::new();
    let mut costs = BTreeMap::from([((0, 0), 0u32)]);
    let mut parents = BTreeMap::new();
    let mut closed = BTreeSet::new();
    let mut expansions = 0;
    open.push(Reverse((
        heuristic((0, 0), target),
        heuristic((0, 0), target),
        (0, 0),
        0u32,
    )));
    while let Some(Reverse((_, _, cell, cost))) = open.pop() {
        if closed.contains(&cell) || costs.get(&cell) != Some(&cost) {
            continue;
        }
        if expansions >= limits.max_expansions {
            return Err(PlanError::SearchLimit);
        }
        expansions += 1;
        closed.insert(cell);
        if cell == target
            && let Some(last) = search.point(cell)?
            && search.segment(last, goal)?
        {
            let mut waypoints = vec![goal];
            let mut cursor = cell;
            loop {
                let point = search.point(cursor)?.ok_or(PlanError::RouteInvalidated)?;
                if waypoints.last() != Some(&point) {
                    waypoints.push(point);
                }
                if waypoints.len() > limits.max_waypoints {
                    return Err(PlanError::RouteLimit);
                }
                if cursor == (0, 0) {
                    break;
                }
                cursor = parents[&cursor];
            }
            waypoints.reverse();
            return Ok(Route {
                waypoints,
                expansions,
                queries: search.queries,
            });
        }
        for (dx, dz) in [
            (-1, -1),
            (-1, 0),
            (-1, 1),
            (0, -1),
            (0, 1),
            (1, -1),
            (1, 0),
            (1, 1),
        ] {
            let next = (cell.0 + dx, cell.1 + dz);
            if next.0.abs() > limits.radius_cells
                || next.1.abs() > limits.radius_cells
                || closed.contains(&next)
            {
                continue;
            }
            let next_cost = cost + if dx != 0 && dz != 0 { 1414 } else { 1000 };
            if costs.get(&next).is_some_and(|old| *old <= next_cost) {
                continue;
            }
            if !search.neighbor(cell, next)? {
                continue;
            }
            costs.insert(next, next_cost);
            parents.insert(next, cell);
            let h = heuristic(next, target);
            open.push(Reverse((next_cost + h, h, next, next_cost)));
        }
    }
    Err(PlanError::Unreachable)
}

/// Share immutable route data between compatible callers, but maintain each
/// bot's progress separately. Every target is checked again against current
/// terrain before the movement owner may consume it. No pose is generated.
pub struct RouteFollower {
    route: Arc<Route>,
    next: usize,
}
impl RouteFollower {
    pub fn new(route: Arc<Route>) -> Result<Self, PlanError> {
        if route.waypoints.is_empty()
            || route.waypoints.len() > 4096
            || route.waypoints.iter().any(|p| !valid(*p))
        {
            return Err(PlanError::InvalidRequest);
        }
        Ok(Self { route, next: 0 })
    }
    /// Resume already validated immutable route data with caller-owned progress.
    pub(crate) fn resume(route: Arc<Route>, next: usize) -> Result<Self, PlanError> {
        if route.waypoints.is_empty()
            || route.waypoints.len() > 4096
            || next > route.waypoints.len()
        {
            return Err(PlanError::InvalidRequest);
        }
        Ok(Self { route, next })
    }
    pub(crate) fn cursor(&self) -> usize {
        self.next
    }

    /// For a solver-confirmed grounded body, center-ground waypoint Y may be
    /// lower than its supported footprint on a slope. Consume by X/Z precision
    /// only after the actual body-to-waypoint support query succeeds.
    pub(crate) fn next_grounded_target<Q: Traversability + ?Sized>(
        &mut self,
        query: &Q,
        current: Point,
    ) -> Result<Option<Point>, PlanError> {
        if !valid(current) {
            return Err(PlanError::InvalidRequest);
        }
        while let Some(target) = self.route.waypoints.get(self.next).copied() {
            if (target[0] - current[0]).hypot(target[2] - current[2]) > 1.0 / 32.0 {
                break;
            }
            if !query.traversable(current, target).map_err(query_error)? {
                return Err(PlanError::RouteInvalidated);
            }
            self.next += 1;
        }
        self.next_target(query, current)
    }

    pub fn next_target<Q: Traversability + ?Sized>(
        &mut self,
        query: &Q,
        current: Point,
    ) -> Result<Option<Point>, PlanError> {
        if !valid(current) {
            return Err(PlanError::InvalidRequest);
        }
        // Account for native 1/32m position precision and the physics skin.
        // Any small corner shortcut is checked from the actual current pose
        // to the new target below, rather than trusting the original edges.
        while self.route.waypoints.get(self.next).is_some_and(|target| {
            (0..3)
                .map(|i| (target[i] - current[i]).powi(2))
                .sum::<f64>()
                <= (1.0_f64 / 32.0).powi(2)
        }) {
            self.next += 1;
        }
        let Some(target) = self.route.waypoints.get(self.next).copied() else {
            return Ok(None);
        };
        if !query.traversable(current, target).map_err(query_error)? {
            return Err(PlanError::RouteInvalidated);
        }
        Ok(Some(target))
    }
}

/// One integration hook for future explicit bot goal requests. Callers supply
/// map/body-specific terrain and a current pose. Failed attempts also consume
/// the request's bounded replan allowance; there is no tick-driven retry loop.
pub struct GoalRequest {
    goal: Point,
    limits: Limits,
    attempts_left: u8,
}
impl GoalRequest {
    pub fn new(goal: Point, limits: Limits, max_replans: u8) -> Result<Self, PlanError> {
        limits.validate()?;
        if !valid(goal) || max_replans > 3 {
            return Err(PlanError::InvalidRequest);
        }
        Ok(Self {
            goal,
            limits,
            attempts_left: max_replans + 1,
        })
    }
    pub fn plan<Q: Traversability + ?Sized>(
        &mut self,
        query: &Q,
        current: Point,
    ) -> Result<RouteFollower, PlanError> {
        if self.attempts_left == 0 {
            return Err(PlanError::ReplanLimit);
        }
        self.attempts_left -= 1;
        RouteFollower::new(Arc::new(plan_route(
            query,
            current,
            self.goal,
            self.limits,
        )?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, collections::BTreeSet};
    struct Map {
        blocked: BTreeSet<(i32, i32)>,
        cliff: BTreeSet<(i32, i32)>,
        calls: Cell<usize>,
        unavailable: bool,
    }
    impl Map {
        fn empty() -> Self {
            Self {
                blocked: BTreeSet::new(),
                cliff: BTreeSet::new(),
                calls: Cell::new(0),
                unavailable: false,
            }
        }
    }
    impl Traversability for Map {
        fn ground(&self, p: Point) -> Result<Option<Point>, QueryError> {
            self.calls.set(self.calls.get() + 1);
            if self.unavailable {
                return Err(QueryError::Unavailable);
            }
            if self
                .blocked
                .contains(&(p[0].round() as i32, p[2].round() as i32))
            {
                return Ok(None);
            }
            Ok(Some([
                p[0],
                if self
                    .cliff
                    .contains(&(p[0].round() as i32, p[2].round() as i32))
                {
                    10.0
                } else {
                    0.0
                },
                p[2],
            ]))
        }
        fn traversable(&self, a: Point, b: Point) -> Result<bool, QueryError> {
            self.calls.set(self.calls.get() + 1);
            Ok((a[1] - b[1]).abs() <= 0.5)
        }
    }
    fn p(x: f64, z: f64) -> Point {
        [x, 0.0, z]
    }
    fn limits() -> Limits {
        Limits {
            radius_cells: 8,
            ..Limits::default()
        }
    }
    #[test]
    fn obstacle_detour_is_deterministic_and_grounded() {
        let mut map = Map::empty();
        map.blocked.extend([(2, -1), (2, 0), (2, 1)]);
        let a = plan_route(&map, p(0.0, 0.0), p(4.0, 0.0), limits()).unwrap();
        let b = plan_route(&map, p(0.0, 0.0), p(4.0, 0.0), limits()).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.waypoints.first(), Some(&p(0.0, 0.0)));
        assert_eq!(a.waypoints.last(), Some(&p(4.0, 0.0)));
        assert!(a.waypoints.iter().any(|v| v[2].abs() >= 2.0));
        assert!(a.waypoints.iter().all(|v| v[1] == 0.0));
    }
    #[test]
    fn blocked_diagonal_cannot_cut_a_corner() {
        let mut map = Map::empty();
        map.blocked.extend([(1, 0), (0, 1), (-1, 0), (0, -1)]);
        assert_eq!(
            plan_route(&map, p(0.0, 0.0), p(1.0, 1.0), limits()),
            Err(PlanError::Unreachable)
        );
    }
    #[test]
    fn cliff_edges_are_not_walkable() {
        let mut map = Map::empty();
        for z in -8..=8 {
            map.cliff.insert((2, z));
        }
        assert_eq!(
            plan_route(&map, p(0.0, 0.0), p(4.0, 0.0), limits()),
            Err(PlanError::Unreachable)
        );
    }
    #[test]
    fn invalid_endpoints_and_missing_geometry_fail_closed() {
        let mut map = Map::empty();
        map.blocked.insert((0, 0));
        assert_eq!(
            plan_route(&map, p(0.0, 0.0), p(4.0, 0.0), limits()),
            Err(PlanError::InvalidStart)
        );
        assert_eq!(
            plan_route(&map, p(4.0, 0.0), p(0.0, 0.0), limits()),
            Err(PlanError::InvalidGoal)
        );
        assert_eq!(
            plan_route(&map, [f64::NAN, 0.0, 0.0], p(1.0, 0.0), limits()),
            Err(PlanError::InvalidRequest)
        );
        map.unavailable = true;
        assert_eq!(
            plan_route(&map, p(4.0, 0.0), p(5.0, 0.0), limits()),
            Err(PlanError::GeometryUnavailable)
        );
    }
    #[test]
    fn budgets_and_bounds_are_hard_limits() {
        let map = Map::empty();
        assert_eq!(
            plan_route(
                &map,
                p(0.0, 0.0),
                p(4.0, 0.0),
                Limits {
                    max_expansions: 1,
                    ..limits()
                }
            ),
            Err(PlanError::SearchLimit)
        );
        map.calls.set(0);
        assert_eq!(
            plan_route(
                &map,
                p(0.0, 0.0),
                p(4.0, 0.0),
                Limits {
                    max_queries: 1,
                    ..limits()
                }
            ),
            Err(PlanError::QueryLimit)
        );
        assert_eq!(map.calls.get(), 1);
        assert_eq!(
            plan_route(&map, p(0.0, 0.0), p(20.0, 0.0), limits()),
            Err(PlanError::InvalidGoal)
        );
        assert_eq!(
            plan_route(
                &map,
                p(0.0, 0.0),
                p(4.0, 0.0),
                Limits {
                    max_waypoints: 2,
                    ..limits()
                }
            ),
            Err(PlanError::RouteLimit)
        );
    }
    #[test]
    fn off_grid_goal_uses_a_checked_final_connection() {
        let map = Map::empty();
        let goal = p(3.3, 0.2);
        let route = plan_route(&map, p(0.0, 0.0), goal, limits()).unwrap();
        assert_eq!(route.waypoints.last(), Some(&goal));
        assert_eq!(route.queries, map.calls.get());
    }
    #[test]
    fn goal_requests_bound_successful_and_failed_replans() {
        let mut map = Map::empty();
        let mut request = GoalRequest::new(p(3.0, 0.0), limits(), 1).unwrap();
        assert!(request.plan(&map, p(0.0, 0.0)).is_ok());
        map.unavailable = true;
        assert!(matches!(
            request.plan(&map, p(0.0, 0.0)),
            Err(PlanError::GeometryUnavailable)
        ));
        assert!(matches!(
            request.plan(&map, p(0.0, 0.0)),
            Err(PlanError::ReplanLimit)
        ));
        assert!(matches!(
            GoalRequest::new(p(1.0, 0.0), limits(), 4),
            Err(PlanError::InvalidRequest)
        ));
    }
    #[test]
    fn followers_revalidate_stop_and_track_shared_routes_independently() {
        struct Gate(bool);
        impl Traversability for Gate {
            fn ground(&self, p: Point) -> Result<Option<Point>, QueryError> {
                Ok(Some(p))
            }
            fn traversable(&self, _a: Point, _b: Point) -> Result<bool, QueryError> {
                Ok(self.0)
            }
        }
        let route = Arc::new(Route {
            waypoints: vec![p(0.0, 0.0), p(1.0, 0.0), p(1.0, 1.0)],
            expansions: 0,
            queries: 0,
        });
        let mut a = RouteFollower::new(route.clone()).unwrap();
        let mut b = RouteFollower::new(route).unwrap();
        assert_eq!(
            a.next_target(&Gate(true), p(0.0, 0.0)),
            Ok(Some(p(1.0, 0.0)))
        );
        assert_eq!(
            a.next_target(&Gate(false), p(0.5, 0.0)),
            Err(PlanError::RouteInvalidated)
        );
        assert_eq!(
            a.next_target(&Gate(true), p(1.0, 0.0)),
            Ok(Some(p(1.0, 1.0)))
        );
        assert_eq!(
            b.next_target(&Gate(true), p(0.0, 0.0)),
            Ok(Some(p(1.0, 0.0)))
        );
        assert_eq!(a.next_target(&Gate(true), p(1.0, 1.0)), Ok(None));
    }
    #[test]
    fn invalid_query_outputs_and_all_limit_fields_are_rejected() {
        struct Bad;
        impl Traversability for Bad {
            fn ground(&self, _p: Point) -> Result<Option<Point>, QueryError> {
                Ok(Some([f64::NAN, 0.0, 0.0]))
            }
            fn traversable(&self, _a: Point, _b: Point) -> Result<bool, QueryError> {
                panic!("invalid ground must stop the search")
            }
        }
        assert_eq!(
            plan_route(&Bad, p(0.0, 0.0), p(1.0, 0.0), limits()),
            Err(PlanError::InvalidRequest)
        );
        let map = Map::empty();
        for bad in [
            Limits {
                cell_size: f64::NAN,
                ..limits()
            },
            Limits {
                radius_cells: 129,
                ..limits()
            },
            Limits {
                max_queries: 0,
                ..limits()
            },
            Limits {
                max_expansions: 16385,
                ..limits()
            },
            Limits {
                max_waypoints: 4097,
                ..limits()
            },
        ] {
            assert_eq!(
                plan_route(&map, p(0.0, 0.0), p(1.0, 0.0), bad),
                Err(PlanError::InvalidRequest)
            );
        }
        assert_eq!(map.calls.get(), 0);
    }
    #[test]
    fn native_precision_arrival_rechecks_the_new_corner_segment() {
        struct Gate(bool);
        impl Traversability for Gate {
            fn ground(&self, p: Point) -> Result<Option<Point>, QueryError> {
                Ok(Some(p))
            }
            fn traversable(&self, _a: Point, _b: Point) -> Result<bool, QueryError> {
                Ok(self.0)
            }
        }
        let route = Arc::new(Route {
            waypoints: vec![p(0.0, 0.0), p(1.0, 0.0), p(1.0, 1.0)],
            expansions: 0,
            queries: 0,
        });
        let mut follower = RouteFollower::new(route).unwrap();
        assert_eq!(
            follower.next_target(&Gate(true), [0.0, 0.005, 0.0]),
            Ok(Some(p(1.0, 0.0)))
        );
        assert_eq!(
            follower.next_target(&Gate(false), [0.99, 0.005, 0.0]),
            Err(PlanError::RouteInvalidated)
        );
        assert_eq!(
            follower.next_target(&Gate(true), [0.99, 0.005, 0.0]),
            Ok(Some(p(1.0, 1.0)))
        );
    }
}
