use super::*;
use crate::{
    bot_physics::{Body, Motion, WorldTerrain},
    fall_damage::Rules,
};
use std::{
    cmp::Reverse,
    collections::{BTreeMap, BinaryHeap},
};
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WaypointKind {
    #[default]
    Walk,
    DropStart,
    DropEnd,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DropLink {
    pub id: u32,
    pub world_sha256: String,
    pub launch: Point,
    pub landing: Point,
    pub speed: f64,
    pub max_drop_height: f64,
    pub trajectory: Vec<Motion>,
    pub trajectory_sha256: String,
}
impl DropLink {
    fn valid(&self, world: &str) -> bool {
        self.id > 0
            && self.world_sha256 == world
            && valid_point(self.launch)
            && valid_point(self.landing)
            && self.launch[1] > self.landing[1]
            && (0.05..=6.).contains(&self.speed)
            && self.max_drop_height.is_finite()
            && (0. ..=64.).contains(&self.max_drop_height)
            && self.max_drop_height + 0.000001
                >= self
                    .trajectory
                    .iter()
                    .map(|m| m.position[1])
                    .fold(self.launch[1], f64::max)
                    - self
                        .trajectory
                        .iter()
                        .map(|m| m.position[1])
                        .fold(self.launch[1], f64::min)
            && !self.trajectory.is_empty()
            && self.trajectory.len() <= 220
            && self
                .trajectory
                .iter()
                .all(|m| valid_point(m.position) && !m.blocked)
            && self.trajectory.last().is_some_and(|m| {
                m.grounded
                    && (0..3)
                        .map(|i| (m.position[i] - self.landing[i]).powi(2))
                        .sum::<f64>()
                        < 0.01
            })
            && serde_json::to_vec(&self.trajectory)
                .is_ok_and(|b| sha256(&b) == self.trajectory_sha256)
    }
    pub fn certify(
        world: &crate::collision::CollisionWorld,
        world_sha256: &str,
        id: u32,
        launch: Point,
        landing: Point,
        speed: f64,
    ) -> Result<Self, Error> {
        if !valid_sha(world_sha256) || id == 0 {
            return err("navmesh_invalid_drop");
        }
        let terrain = WorldTerrain::new(Some(world), &world.metadata().map_name)
            .map_err(|e| Error(e.code()))?;
        let trace = terrain
            .validate_descent(launch, landing, speed, Body::default(), 10.)
            .map_err(|e| Error(e.code()))?
            .ok_or(Error("navmesh_drop_not_traversable"))?;
        let trajectory_sha256 = sha256(
            &serde_json::to_vec(&trace.samples).map_err(|_| Error("navmesh_drop_serialization"))?,
        );
        Ok(Self {
            id,
            world_sha256: world_sha256.into(),
            launch: trace.launch,
            landing: trace.landing,
            speed,
            max_drop_height: (trace.max_drop_height * 100.).ceil() / 100.,
            trajectory: trace.samples,
            trajectory_sha256,
        })
    }
}
#[derive(Clone)]
struct Label {
    node: usize,
    hp: u16,
    cost: u64,
    parent: Option<usize>,
    via: Option<usize>,
}
#[allow(clippy::too_many_arguments)]
fn choose_links_with_costs(
    links: &[(i32, i32, &DropLink)],
    start: (i32, Point),
    goals: &[(i32, Point)],
    rules: &Rules,
    hp: u16,
    penalties: &BTreeMap<u32, f64>,
    first_drop: Option<u32>,
    exit_goal_weight: f64,
) -> Result<Vec<usize>, Error> {
    if hp == 0 || links.len() > 64 || !(0. ..=1.).contains(&exit_goal_weight) {
        return err("navmesh_invalid_drop_request");
    }
    if penalties
        .values()
        .any(|v| !v.is_finite() || !(0. ..=1024.).contains(v))
    {
        return err("navmesh_invalid_drop_cost");
    }
    let mut labels = vec![Label {
        node: links.len(),
        hp,
        cost: 0,
        parent: None,
        via: None,
    }];
    let mut queue = BinaryHeap::from([Reverse((0u16, 0u64, 0usize))]);
    let mut best = BTreeMap::new();
    best.insert((links.len(), hp), 0u64);
    while let Some(Reverse((_, _, index))) = queue.pop() {
        let label = labels[index].clone();
        let (component, position) = if label.node == links.len() {
            start
        } else {
            (links[label.node].1, links[label.node].2.landing)
        };
        if goals.iter().any(|g| g.0 == component)
            && (first_drop.is_none() || label.node != links.len())
        {
            let mut path = Vec::new();
            let mut cursor = index;
            while let Some(parent) = labels[cursor].parent {
                path.push(labels[cursor].via.unwrap());
                cursor = parent;
            }
            path.reverse();
            return Ok(path);
        }
        for (edge, (source, _, link)) in links.iter().enumerate() {
            if *source != component
                || (label.node == links.len() && first_drop.is_some_and(|id| link.id != id))
            {
                continue;
            }
            let prediction = rules
                .predict_drop(link.max_drop_height as f32, label.hp)
                .map_err(Error)?;
            let remaining = prediction.remaining_health_quarter_points;
            if remaining == 0 {
                continue;
            }
            let length = (position[0] - link.launch[0]).hypot(position[2] - link.launch[2])
                + (link.launch[0] - link.landing[0]).hypot(link.launch[2] - link.landing[2]);
            let cost = label.cost
                + ((length + penalties.get(&link.id).copied().unwrap_or(0.)) * 1000.).ceil() as u64;
            if best.get(&(edge, remaining)).is_some_and(|old| *old <= cost) {
                continue;
            }
            // A cheaper label with at least this much HP dominates this candidate.
            if best
                .range((edge, remaining)..=(edge, u16::MAX))
                .any(|(_, old)| *old <= cost)
            {
                continue;
            }
            if labels.len() >= 4096 {
                return err("navmesh_drop_search_limit");
            }
            best.insert((edge, remaining), cost);
            let next = labels.len();
            labels.push(Label {
                node: edge,
                hp: remaining,
                cost,
                parent: Some(index),
                via: Some(edge),
            });
            // Leaving the start component (a spawn) is ranked by how soon the
            // bot is out: approach, queue penalty and flight. The remaining
            // distance to the goal only counts with the configured weight.
            let weight = if label.node == links.len() {
                exit_goal_weight
            } else {
                1.
            };
            let suffix = weight
                * goals
                    .iter()
                    .map(|(_, goal)| (link.landing[0] - goal[0]).hypot(link.landing[2] - goal[2]))
                    .fold(f64::INFINITY, f64::min);
            queue.push(Reverse((
                hp - remaining,
                cost.saturating_add((suffix * 1000.).ceil() as u64),
                next,
            )));
        }
    }
    err("navmesh_no_survivable_drop_route")
}
impl NavMesh {
    pub fn validate_drop_links(&self) -> Result<(), Error> {
        let mut ids = std::collections::BTreeSet::new();
        if self.artifact.drop_links.len() > 64 {
            return err("navmesh_drop_link_limit");
        }
        for link in &self.artifact.drop_links {
            if !link.valid(&self.artifact.world_sha256) || !ids.insert(link.id) {
                return err("navmesh_invalid_drop");
            }
            self.component_id(link.launch)?;
            self.component_id(link.landing)?;
        }
        Ok(())
    }
    pub fn route_with_drops(
        &self,
        start: Point,
        goal: Point,
        radius: f64,
        rules: &Rules,
        hp: u16,
    ) -> Result<Route, Error> {
        self.route_with_drop_costs(start, goal, radius, rules, hp, &BTreeMap::new(), None)
    }
    #[allow(clippy::too_many_arguments)]
    pub fn route_with_drop_costs(
        &self,
        start: Point,
        goal: Point,
        radius: f64,
        rules: &Rules,
        hp: u16,
        penalties: &BTreeMap<u32, f64>,
        first_drop: Option<u32>,
    ) -> Result<Route, Error> {
        self.route_with_drop_costs_weighted(
            start, goal, radius, rules, hp, penalties, first_drop, 1.,
        )
    }
    /// As `route_with_drop_costs`, with the first drop out of the start
    /// component chosen by `exit_goal_weight` (0: fastest exit only, 1: whole
    /// route).
    #[allow(clippy::too_many_arguments)]
    pub fn route_with_drop_costs_weighted(
        &self,
        start: Point,
        goal: Point,
        radius: f64,
        rules: &Rules,
        hp: u16,
        penalties: &BTreeMap<u32, f64>,
        first_drop: Option<u32>,
        exit_goal_weight: f64,
    ) -> Result<Route, Error> {
        let began = Instant::now();
        if !valid_point(goal) || !radius.is_finite() || !(0.05..=5.).contains(&radius) {
            return err("navmesh_invalid_request");
        }
        // Immutable link certificates were validated once when this mesh loaded.
        let source = (self.component_id(start)?, start);
        let goals = self.goals(goal, radius)?;
        if goals.is_empty() {
            return err("navmesh_invalid_goal");
        }
        let links: Vec<_> = self
            .artifact
            .drop_links
            .iter()
            .map(|l| {
                Ok((
                    self.component_id(l.launch)?,
                    self.component_id(l.landing)?,
                    l,
                ))
            })
            .collect::<Result<_, Error>>()?;
        let selected = choose_links_with_costs(
            &links,
            source,
            &goals,
            rules,
            hp,
            penalties,
            first_drop,
            exit_goal_weight,
        )?;
        let mut route = Route {
            format: "mag_navmesh_route_v1".into(),
            status: Status::Complete,
            world_sha256: self.artifact.world_sha256.clone(),
            profile: self.artifact.profile.clone(),
            start,
            requested_goal: goal,
            projected_start: start,
            projected_goal: goal,
            waypoints: Vec::new(),
            corridor_refs: Vec::new(),
            length_m: 0.,
            query_us: 0,
            waypoint_kinds: Vec::new(),
            waypoint_drop_ids: Vec::new(),
            predicted_remaining_health: Some(hp),
        };
        let mut from = start;
        let mut remaining = hp;
        for selection in selected
            .iter()
            .copied()
            .map(Some)
            .chain(std::iter::once(None))
        {
            let link = selection.map(|i| links[i].2);
            let target = link.map_or(goal, |l| l.launch);
            let ground = self.route(from, target, if link.is_some() { 0.75 } else { radius })?;
            if ground.status != Status::Complete {
                return err("navmesh_ground_segment_incomplete");
            }
            if route.waypoints.is_empty() {
                route.projected_start = ground.projected_start;
            }
            route.waypoints.extend(ground.waypoints.iter().copied());
            route
                .waypoint_kinds
                .extend(vec![WaypointKind::Walk; ground.waypoints.len()]);
            route
                .waypoint_drop_ids
                .extend(vec![None; ground.waypoints.len()]);
            route.corridor_refs.extend(ground.corridor_refs);
            if let Some(link) = link {
                // Supported launch/landing are steering hints only; never assign pose Y.
                route.waypoints.push(link.launch);
                route.waypoint_kinds.push(WaypointKind::DropStart);
                route.waypoint_drop_ids.push(Some(link.id));
                route.waypoints.push(link.landing);
                route.waypoint_kinds.push(WaypointKind::DropEnd);
                route.waypoint_drop_ids.push(Some(link.id));
                remaining = rules
                    .predict_drop(link.max_drop_height as f32, remaining)
                    .map_err(Error)?
                    .remaining_health_quarter_points;
                from = link.landing;
            } else {
                route.projected_goal = ground.projected_goal;
            }
        }
        route.length_m = route
            .waypoints
            .windows(2)
            .map(|p| (p[1][0] - p[0][0]).hypot(p[1][2] - p[0][2]))
            .sum();
        if !route.length_m.is_finite()
            || route.length_m > 2048.
            || route.waypoints.len() > 1024
            || route.corridor_refs.len() > 1024
        {
            return err("navmesh_route_limit");
        }
        route.corridor_refs.sort_unstable();
        route.corridor_refs.dedup();
        route.query_us = began.elapsed().as_micros().min(u64::MAX as u128) as u64;
        route.predicted_remaining_health = Some(remaining);
        Ok(route)
    }
}

#[cfg(test)]
mod exit_weight_tests {
    use super::*;

    fn link(id: u32, launch: Point, landing: Point) -> DropLink {
        DropLink {
            id,
            world_sha256: String::new(),
            launch,
            landing,
            speed: 4.5,
            max_drop_height: 1.,
            trajectory: Vec::new(),
            trajectory_sha256: String::new(),
        }
    }

    #[test]
    fn a_spawn_exit_is_chosen_by_how_soon_the_bot_is_out_unless_weighted_to_the_goal() {
        // SYNTHETIC rules: no drop here is high enough to hurt.
        let rules = Rules {
            schema_version: 1,
            source_build: "synthetic".into(),
            application_id: 20231,
            min_height: 50.,
            damage_factor: 1.,
            minimum_damage_exclusive: 0.,
            jump_fall_state: 0,
            damage_type: 5,
            damage_source_type: 1,
        };
        // Link 1 is the nearest way out but lands far from the goal; link 2 is
        // a longer walk whose landing is close to the goal.
        let near_exit = link(1, [3., 10., 0.], [3., 0., -40.]);
        let near_goal = link(2, [20., 10., 0.], [60., 0., 0.]);
        let links = [(1, 2, &near_exit), (1, 2, &near_goal)];
        let start = (1, [0., 10., 0.]);
        let goals = [(2, [100., 0., 0.])];
        let choose = |weight, penalties: &BTreeMap<u32, f64>| {
            choose_links_with_costs(&links, start, &goals, &rules, 400, penalties, None, weight)
                .unwrap()
                .iter()
                .map(|&i| links[i].2.id)
                .collect::<Vec<_>>()
        };
        let free = BTreeMap::new();
        assert_eq!(
            choose(1., &free),
            [2],
            "whole-route cost prefers the landing by the goal"
        );
        assert_eq!(
            choose(0., &free),
            [1],
            "exit-first takes the nearest way out"
        );
        // A queue on the nearest exit makes the other one faster to leave by.
        let queued = BTreeMap::from([(1, 30.)]);
        assert_eq!(choose(0., &queued), [2]);
        assert!(
            choose_links_with_costs(&links, start, &goals, &rules, 400, &free, None, 1.5).is_err(),
            "weights outside 0..=1 are rejected"
        );
    }
}
