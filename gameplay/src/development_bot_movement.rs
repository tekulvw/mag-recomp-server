//! Bounded pose or opt-in physical movement for an existing server-owned bot.
//! Navigation is explicit; combat, respawn and ownership remain unchanged.
use super::*;
use development_target::{BotPhase, BotRuntime, BotStep, Target};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BotMoveMode {
    #[default]
    LabPose,
    Physical,
    Gravity,
    Navigation,
    Navmesh,
}
impl BotMoveMode {
    fn policy(self) -> &'static str {
        match self {
            Self::LabPose => "constant_height_lab_segment_not_ground_validated",
            Self::Physical => "verified_region_ground_and_obstacles_conservative_box",
            Self::Gravity => "supplied_ground_vertical_only_obstacle_coverage_not_claimed",
            Self::Navigation => "bounded_route_verified_ground_and_obstacles_physical_steps",
            Self::Navmesh => "recast_detour_static_corridor_physical_steps",
        }
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BotMove {
    #[serde(default)]
    pub mode: BotMoveMode,
    pub destination: replication::CharacterPosition,
    pub duration_ms: u32,
    pub update_ms: u32,
    /// Raw native lab facing, only when explicitly supplied. None preserves it.
    #[serde(default)]
    pub yaw: Option<u16>,
    #[serde(default)]
    pub sprint: bool,
}
impl BotMove {
    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        self.destination.validate()?;
        if self.mode == BotMoveMode::Navmesh && self.yaw.is_some() {
            return Err("bot_navmesh_requires_route_facing");
        }
        if self.sprint && self.mode != BotMoveMode::Navmesh {
            return Err("bot_sprint_requires_navmesh");
        }
        let maximum_duration = if self.mode == BotMoveMode::Navmesh {
            120000
        } else {
            60000
        };
        if !(1000..=maximum_duration).contains(&self.duration_ms)
            || !(100..=1000).contains(&self.update_ms)
            || self.yaw.is_some_and(|yaw| yaw > 1023)
        {
            return Err("invalid_bot_move");
        }
        Ok(())
    }
    fn publication_budget(&self) -> usize {
        ((self.duration_ms as usize)
            + if self.mode == BotMoveMode::LabPose {
                0
            } else {
                5000
            })
        .div_ceil(self.update_ms as usize)
            + 1
    }
    fn validate_from(&self, from: replication::CharacterPosition) -> Result<(), &'static str> {
        self.validate()?;
        if self.mode == BotMoveMode::LabPose && self.destination.y != from.y {
            return Err("bot_move_requires_constant_height");
        }
        let dx = self.destination.x - from.x;
        let dz = self.destination.z - from.z;
        let distance = dx.hypot(dz);
        if self.mode == BotMoveMode::Gravity {
            return if distance == 0.0 {
                Ok(())
            } else {
                Err("bot_gravity_requires_vertical_only")
            };
        }
        if (self.destination.y - from.y).abs() > 16.0 {
            return Err("bot_move_vertical_distance_limit");
        }
        let maximum_distance = if self.mode == BotMoveMode::Navmesh {
            256.0
        } else {
            64.0
        };
        if !(0.03125..=maximum_distance).contains(&distance) {
            return Err("bot_move_distance_limit");
        }
        if self.mode != BotMoveMode::Navmesh
            && f64::from(distance) / (f64::from(self.duration_ms) / 1000.)
                > retail_movement::WALK_BACKWARD_MPS
        {
            return Err(if self.mode == BotMoveMode::Navigation {
                "bot_navigation_speed_limit"
            } else {
                "bot_move_player_speed_limit"
            });
        }
        Ok(())
    }
}
pub(super) struct PendingMove {
    movement_id: String,
    from: replication::CharacterPosition,
    movement: BotMove,
    navigation: Option<NavigationMove>,
}
impl<W: Write> Engine<W> {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn queue_development_bot_move(
        &mut self,
        reservation_id: &str,
        world_id: u32,
        player_id: u16,
        movement_id: String,
        expected_owner_generation: u32,
        expected_bot_generation: u32,
        movement: BotMove,
        now: Duration,
        wall: u64,
    ) -> Result<Vec<OutboundMessage>, &'static str> {
        if !self.development_control {
            return Err("development_control_disabled");
        }
        if self.match_terminal.is_some() {
            return Err("match_finished");
        }
        if self.maximum_players != 1 || self.wire_profile.application_id() != 20231 {
            return Err("bot_move_requires_singleton_retail");
        }
        movement.validate()?;
        let actor = self.active.as_ref().ok_or("no_reservation")?;
        if actor.reservation.id != reservation_id
            || actor.reservation.world != world_id
            || actor.reservation.player != player_id
        {
            return Err("identity_mismatch");
        }
        if !matches!(actor.life,CharacterLife::Alive{generation,..} if generation==expected_owner_generation)
        {
            return Err("target_life_mismatch");
        }
        if !actor.entered_game || !actor.ready_for_updates || actor.mounted_seat.is_some() {
            return Err("target_requires_ready_on_foot_character");
        }
        let target = actor.development_target.as_ref().ok_or("bot_not_spawned")?;
        if target.generation != expected_owner_generation {
            return Err("target_life_mismatch");
        }
        let runtime = target.runtime.ok_or("bot_not_spawned")?;
        if runtime.generation != expected_bot_generation {
            return Err("bot_generation_mismatch");
        }
        if let Some(previous) = target.move_commands.get(&movement_id) {
            return if previous == &movement {
                Ok(Vec::new())
            } else {
                Err("bot_move_id_conflict")
            };
        }
        let initial_hold = target.script.as_ref().is_some_and(|script| {
            matches!(runtime.phase, BotPhase::Moving)
                && runtime.last_update == runtime.since
                && now.saturating_sub(runtime.since)
                    < Duration::from_millis(script.spawn_hold_ms.into())
        });
        if !matches!(runtime.phase, BotPhase::Complete) && !initial_hold {
            return Err("bot_move_requires_alive_idle_or_initial_hold");
        }
        if !actor.dummy_created
            || !target.roster_published
            || !actor.dummy_health.is_some_and(|hp| hp > 0)
        {
            return Err("bot_not_alive");
        }
        if target.move_commands.len() >= 16 {
            return Err("bot_move_command_limit");
        }
        movement.validate_from(runtime.position)?;
        if actor.replication_history.len()
            + movement.publication_budget()
            + development_target::BOT_HISTORY_HEADROOM
            >= MAX_REPLICATION_HISTORY
        {
            return Err("replication_history_limit");
        }
        // Exact retries returned above, before any geometry query or route cost.
        let navigation = if movement.mode == BotMoveMode::Navmesh {
            let mesh = self
                .navmesh_world
                .clone()
                .ok_or("bot_navmesh_unavailable")?;
            mesh.validate_coverage(
                self.collision_world
                    .as_ref()
                    .ok_or("bot_navmesh_geometry_unavailable")?,
            )
            .map_err(|e| e.0)?;
            Some(NavigationMove::plan_navmesh(
                mesh,
                self.map_name(),
                runtime.position,
                &movement,
                self.fall_damage_rules.as_ref(),
                actor.dummy_health.unwrap(),
            )?)
        } else if movement.mode == BotMoveMode::Navigation {
            match NavigationMove::plan(
                self.collision_world.as_ref(),
                self.map_name(),
                runtime.position,
                &movement,
            ) {
                Ok(route) => Some(route),
                Err(reason) => {
                    self.record(json!({"op":"development_bot_navigation_rejected","movement_id":movement_id,"reservation_id":reservation_id,"world_id":world_id,"player_id":player_id,"owner_life_generation":expected_owner_generation,"bot_generation":expected_bot_generation,"reason":reason,"position":runtime.position,"destination":movement.destination,"combat_authority":false}),wall)?;
                    return Err(reason);
                }
            }
        } else {
            None
        };
        let physics_motion = if movement.mode == BotMoveMode::LabPose {
            None
        } else {
            let terrain =
                bot_physics::WorldTerrain::new(self.collision_world.as_ref(), self.map_name())
                    .map_err(|e| e.code())?;
            if movement.mode == BotMoveMode::Physical {
                terrain
                    .require_horizontal_coverage(
                        point(runtime.position),
                        point(movement.destination),
                        bot_physics::Body::default(),
                    )
                    .map_err(|e| e.code())?;
            }
            // The starting column must contain actual supplied ground; never
            // synthesize a flat plane when geometry is absent or has a hole.
            if terrain
                .sample_ground(point(runtime.position), 0.4, 64.0)
                .map_err(|e| e.code())?
                .is_none()
            {
                return Err("bot_physics_ground_unavailable");
            }
            Some(match runtime.physics_motion {
                Some(motion) => motion,
                None => terrain
                    .start_motion(point(runtime.position), bot_physics::Body::default())
                    .map_err(|e| e.code())?,
            })
        };
        self.record(json!({"op":"development_bot_move_queued","reservation_id":reservation_id,"world_id":world_id,"player_id":player_id,
            "movement_id":movement_id,"owner_life_generation":expected_owner_generation,"bot_generation":expected_bot_generation,
            "object_id":dummy::OBJECT_ID,"owner_index":dummy::SERVER_OWNER,"from":runtime.position,"movement":movement,
            "cancelled_original_lifecycle":initial_hold,"navigation":navigation.as_ref().map(NavigationMove::receipt),"placement_policy":movement.mode.policy(),"collision_metadata":if movement.mode==BotMoveMode::LabPose {None} else {self.collision_world.as_ref().map(|w|w.metadata())},"combat_policy":"ordinary_client_reported_bullets_when_catalog_installed"}),wall)?;
        let target = self
            .active
            .as_mut()
            .unwrap()
            .development_target
            .as_mut()
            .unwrap();
        target
            .move_commands
            .insert(movement_id.clone(), movement.clone());
        target.pending_move = Some(PendingMove {
            movement_id,
            from: runtime.position,
            movement,
            navigation,
        });
        let mut navmesh_state = runtime.navmesh_state;
        navmesh_state.drop_wait = Duration::ZERO;
        navmesh_state.navigation_aborted = false;
        navmesh_state.movement.advance(
            retail_movement::MovementIntent::Idle,
            now.saturating_sub(runtime.last_update)
                .as_secs_f64()
                .min(10.),
        )?;
        target.runtime = Some(BotRuntime {
            navmesh_state,
            physics_motion,
            navigation_waypoint: 0,
            phase: BotPhase::Relocating,
            since: now,
            last_update: now,
            ..runtime
        });
        Ok(Vec::new())
    }
}
/// A world scheduler grants one certified passage; legacy admin movement is explicit.
#[derive(Clone, Copy, Default)]
pub(super) enum DropAdmission {
    #[default]
    Unrestricted,
    Permit {
        drop_id: Option<u32>,
        entry_buffer_m: f64,
    },
}
impl DropAdmission {
    fn permits(self, id: u32) -> bool {
        match self {
            Self::Unrestricted => true,
            Self::Permit { drop_id, .. } => drop_id == Some(id),
        }
    }
    fn entry_buffer(self) -> f64 {
        match self {
            Self::Unrestricted => 1.5,
            Self::Permit { entry_buffer_m, .. } => entry_buffer_m,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum DropPhase {
    Approaching,
    Traversing,
    Landed,
    Cleared,
    Retreating,
    Retreated,
}
#[derive(Clone, Copy, Debug)]
pub(super) struct DropPassage {
    pub id: u32,
    pub launch: navmesh::Point,
    pub landing: navmesh::Point,
    pub protected_bounds: crate::actor_collision::Bounds,
    pub phase: DropPhase,
    pub distance_to_launch: f64,
    pub route_distance_to_launch: f64,
}
fn bounds_intersect(a: crate::actor_collision::Bounds, b: crate::actor_collision::Bounds) -> bool {
    (0..3).all(|axis| a.min[axis] <= b.max[axis] && b.min[axis] <= a.max[axis])
}
pub(super) fn protected_drop_bounds(
    link: &navmesh::DropLink,
    entry_buffer_m: f64,
) -> crate::actor_collision::Bounds {
    let body = bot_physics::Body::default();
    let mut bounds = crate::actor_collision::Bounds::body(link.launch, body)
        .union(crate::actor_collision::Bounds::body(link.landing, body));
    for sample in &link.trajectory {
        bounds = bounds.union(crate::actor_collision::Bounds::body(sample.position, body));
    }
    bounds.union(crate::actor_collision::Bounds {
        min: [
            link.launch[0] - entry_buffer_m,
            link.launch[1],
            link.launch[2] - entry_buffer_m,
        ],
        max: [
            link.launch[0] + entry_buffer_m,
            link.launch[1] + body.height,
            link.launch[2] + entry_buffer_m,
        ],
    })
}
#[cfg(test)]
fn private_drop_trace(value: Value) {
    let Ok(path) = std::env::var("MAG_BOT_DROP_TRACE_PATH") else {
        return;
    };
    if let Ok(ids) = std::env::var("MAG_BOT_DROP_TRACE_IDS") {
        let id = value["drop_id"].as_u64().unwrap();
        if !ids.split(',').any(|v| v.parse::<u64>() == Ok(id)) {
            return;
        }
    }
    use std::io::Write as _;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    writeln!(file, "{value}").unwrap();
}
impl Target {
    pub(super) fn next_registry_drop(&self, entry_buffer_m: f64) -> Option<DropPassage> {
        let runtime = self.runtime?;
        let navigation = self.pending_move.as_ref()?.navigation.as_ref()?;
        let (_, route) = navigation.navmesh.as_ref()?;
        route
            .waypoint_kinds
            .iter()
            .enumerate()
            .filter_map(|(index, kind)| {
                (*kind == navmesh::WaypointKind::DropStart).then_some(index)
            })
            .find_map(|index| {
                let id = route.waypoint_drop_ids[index]?;
                let passage = navigation.drop_passage(runtime, id, entry_buffer_m)?;
                (passage.phase != DropPhase::Cleared).then_some(passage)
            })
    }
    pub(super) fn registry_drop_status(&self, id: u32, entry_buffer_m: f64) -> Option<DropPassage> {
        self.pending_move
            .as_ref()?
            .navigation
            .as_ref()?
            .drop_passage(self.runtime?, id, entry_buffer_m)
    }
    pub(super) fn registry_route(&self) -> Option<&navmesh::Route> {
        Some(
            self.pending_move
                .as_ref()?
                .navigation
                .as_ref()?
                .navmesh
                .as_ref()?
                .1
                .as_ref(),
        )
    }
    /// A failed certificate is never resumed. Only actual supported lower-side
    /// landing may request a new ordinary egress route while retaining ownership.
    pub(super) fn registry_aborted_drop(&self, entry_buffer_m: f64) -> Option<DropPassage> {
        let runtime = self.runtime?;
        let motion = runtime.physics_motion?;
        if !runtime.navmesh_state.navigation_aborted
            || !motion.grounded
            || runtime.navmesh_state.on_player
        {
            return None;
        }
        let navigation = self.pending_move.as_ref()?.navigation.as_ref()?;
        if let Some(egress) = &navigation.ground_egress {
            if egress.retreat {
                return None;
            }
            let mut passage = egress.passage;
            passage.phase = if bounds_intersect(
                passage.protected_bounds,
                crate::actor_collision::Bounds::body(motion.position, Default::default()),
            ) {
                DropPhase::Landed
            } else {
                DropPhase::Cleared
            };
            return Some(passage);
        }
        let (_, route) = navigation.navmesh.as_ref()?;
        if route.waypoint_kinds.get(runtime.navigation_waypoint)
            != Some(&navmesh::WaypointKind::DropEnd)
        {
            return None;
        }
        let id = *route
            .waypoint_drop_ids
            .get(runtime.navigation_waypoint)?
            .as_ref()?;
        let passage = navigation.drop_passage(runtime, id, entry_buffer_m)?;
        matches!(passage.phase, DropPhase::Landed | DropPhase::Cleared).then_some(passage)
    }
    /// Upper support after an aborted flight is a retreat, never a DropEnd.
    pub(super) fn registry_aborted_drop_retreat(&self, entry_buffer_m: f64) -> Option<DropPassage> {
        let runtime = self.runtime?;
        let motion = runtime.physics_motion?;
        if !motion.grounded || runtime.navmesh_state.on_player {
            return None;
        }
        let navigation = self.pending_move.as_ref()?.navigation.as_ref()?;
        if let Some(egress) = &navigation.ground_egress {
            return (egress.retreat && !egress.precommit)
                .then(|| navigation.drop_passage(runtime, egress.passage.id, entry_buffer_m))
                .flatten();
        }
        if !runtime.navmesh_state.navigation_aborted {
            return None;
        }
        let (_, route) = navigation.navmesh.as_ref()?;
        if route.waypoint_kinds.get(runtime.navigation_waypoint)
            != Some(&navmesh::WaypointKind::DropEnd)
        {
            return None;
        }
        let id = *route
            .waypoint_drop_ids
            .get(runtime.navigation_waypoint)?
            .as_ref()?;
        let mut passage = navigation.drop_passage(runtime, id, entry_buffer_m)?;
        if motion.position[1] <= passage.landing[1] + bot_physics::Body::default().max_step {
            return None;
        }
        passage.phase = DropPhase::Retreating;
        Some(passage)
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn begin_registry_aborted_drop_retreat(
        &mut self,
        world: &collision::CollisionWorld,
        mesh: Arc<navmesh::NavMesh>,
        map: &str,
        movement: BotMove,
        now: Duration,
        entry_buffer_m: f64,
        health: u16,
        rules: &fall_damage::Rules,
        actors: Option<crate::actor_collision::Query<'_>>,
    ) -> Result<(), &'static str> {
        let passage = self
            .registry_aborted_drop_retreat(entry_buffer_m)
            .ok_or("bot_retreat_requires_supported_upper_abort")?;
        self.begin_registry_aborted_ground_exit(
            world, mesh, map, movement, now, health, rules, actors, passage, true, false,
        )
    }
    /// An uncommitted approach may physically retreat without reporting a drop.
    pub(super) fn registry_grounded_passage_retreat(
        &self,
        entry_buffer_m: f64,
    ) -> Option<DropPassage> {
        let runtime = self.runtime?;
        let navigation = self.pending_move.as_ref()?.navigation.as_ref()?;
        let egress = navigation.ground_egress.as_ref()?;
        egress
            .precommit
            .then(|| navigation.drop_passage(runtime, egress.passage.id, entry_buffer_m))
            .flatten()
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn begin_registry_grounded_passage_retreat(
        &mut self,
        world: &collision::CollisionWorld,
        mesh: Arc<navmesh::NavMesh>,
        map: &str,
        movement: BotMove,
        now: Duration,
        entry_buffer_m: f64,
        health: u16,
        rules: &fall_damage::Rules,
        actors: Option<crate::actor_collision::Query<'_>>,
        drop_id: u32,
    ) -> Result<(), &'static str> {
        let runtime = self.runtime.ok_or("bot_not_spawned")?;
        let motion = runtime.physics_motion.ok_or("bot_physics_state_missing")?;
        let passage = self
            .registry_drop_status(drop_id, entry_buffer_m)
            .ok_or("bot_retreat_requires_uncommitted_passage")?;
        if passage.phase != DropPhase::Approaching
            || !motion.grounded
            || runtime.navmesh_state.on_player
            || !bounds_intersect(
                passage.protected_bounds,
                crate::actor_collision::Bounds::body(motion.position, Default::default()),
            )
        {
            return Err("bot_retreat_requires_supported_uncommitted_passage");
        }
        self.begin_registry_aborted_ground_exit(
            world, mesh, map, movement, now, health, rules, actors, passage, true, true,
        )
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn begin_registry_aborted_drop_egress(
        &mut self,
        world: &collision::CollisionWorld,
        mesh: Arc<navmesh::NavMesh>,
        map: &str,
        movement: BotMove,
        now: Duration,
        entry_buffer_m: f64,
        health: u16,
        rules: &fall_damage::Rules,
        actors: Option<crate::actor_collision::Query<'_>>,
    ) -> Result<(), &'static str> {
        let passage = self
            .registry_aborted_drop(entry_buffer_m)
            .ok_or("bot_egress_requires_supported_aborted_drop")?;
        self.begin_registry_aborted_ground_exit(
            world, mesh, map, movement, now, health, rules, actors, passage, false, false,
        )
    }
    #[allow(clippy::too_many_arguments)]
    fn begin_registry_aborted_ground_exit(
        &mut self,
        world: &collision::CollisionWorld,
        mesh: Arc<navmesh::NavMesh>,
        map: &str,
        movement: BotMove,
        now: Duration,
        health: u16,
        rules: &fall_damage::Rules,
        actors: Option<crate::actor_collision::Query<'_>>,
        passage: DropPassage,
        retreat: bool,
        precommit: bool,
    ) -> Result<(), &'static str> {
        let runtime = self.runtime.ok_or("bot_not_spawned")?;
        let initial = runtime.physics_motion.ok_or("bot_physics_state_missing")?;
        if movement.mode != BotMoveMode::Navmesh || movement.sprint {
            return Err("bot_egress_requires_normal_walk");
        }
        movement.validate_from(runtime.position)?;
        let target = point(movement.destination);
        if (target[0] - initial.position[0]).hypot(target[2] - initial.position[2]) > 8.
            || bounds_intersect(
                passage.protected_bounds,
                crate::actor_collision::Bounds::body(target, Default::default()),
            )
        {
            return Err("bot_egress_goal_not_clear");
        }
        mesh.validate_coverage(world).map_err(|e| e.0)?;
        mesh.component_id(target).map_err(|e| e.0)?;
        let terrain =
            bot_physics::WorldTerrain::new(Some(world), map).map_err(navigation_physics_error)?;
        let terrain = if let Some(actors) = actors {
            terrain.with_actors(actors)
        } else {
            terrain
        };
        let mut context = NavigationContext {
            state: runtime.navmesh_state,
            hp: health,
            rules: Some(rules),
            generation: runtime.generation,
            hits: Vec::new(),
            ground_events: Vec::new(),
            sprint: false,
            facing: None,
            recovered_contacts: 0,
            actors,
            waiting_on: None,
            avoiding: false,
            swept_bounds: None,
            admission: DropAdmission::Unrestricted,
            waiting_drop: None,
            waited_seconds: 0.,
        };
        let mut motion = initial;
        let mut samples = vec![motion];
        let mut reached = false;
        // Fresh proof from the exact committed physical state, bounded to5s.
        // A normal survivable descent is permitted; no trace sample is assigned.
        for _ in 0..100 {
            let dx = target[0] - motion.position[0];
            let dz = target[2] - motion.position[2];
            let distance = dx.hypot(dz);
            let speed = retail_movement::WALK_FORWARD_MPS.min(distance / 0.05);
            let velocity = if distance > 1.0 / 32.0 {
                [dx / distance * speed, 0., dz / distance * speed]
            } else {
                [0.; 3]
            };
            motion = context.advance(&terrain, motion, velocity, 0.05)?;
            if motion.blocked || context.hp == 0 {
                return Err("bot_egress_physical_proof_failed");
            }
            samples.push(motion);
            if motion.grounded
                && !context.state.on_player
                && (target[0] - motion.position[0]).hypot(target[2] - motion.position[2])
                    <= 1.0 / 32.0
                && !bounds_intersect(
                    passage.protected_bounds,
                    crate::actor_collision::Bounds::body(motion.position, Default::default()),
                )
                && mesh.component_id(motion.position).is_ok()
            {
                reached = true;
                break;
            }
        }
        if !reached {
            return Err("bot_egress_physical_proof_failed");
        }
        let length = (target[0] - initial.position[0]).hypot(target[2] - initial.position[2]);
        let navigation = NavigationMove {
            ground_egress: Some(Arc::new(GroundEgress {
                mesh,
                passage,
                retreat,
                precommit,
                target,
                samples,
            })),
            supported_recovery: false,
            navmesh: None,
            route: Arc::new(bot_navigation::Route {
                waypoints: vec![initial.position, target],
                expansions: 0,
                queries: 1,
            }),
            bounds: bot_navigation::PlanningBounds {
                min_xz: [0.; 2],
                max_xz: [1.; 2],
            },
            speed: retail_movement::WALK_FORWARD_MPS,
            length,
        };
        self.pending_move = Some(PendingMove {
            movement_id: format!(
                "population-egress-{}-{}",
                self.object_index, runtime.generation
            ),
            from: runtime.position,
            movement,
            navigation: Some(navigation),
        });
        let mut navmesh_state = runtime.navmesh_state;
        navmesh_state.navigation_aborted = false;
        navmesh_state.drop_wait = Duration::ZERO;
        self.runtime = Some(BotRuntime {
            phase: BotPhase::Relocating,
            since: now,
            last_update: now,
            navigation_waypoint: 0,
            navmesh_state,
            ..runtime
        });
        Ok(())
    }
    /// Progress toward the next production route waypoint, even on a dogleg
    /// that temporarily increases distance from the final destination.
    pub(super) fn registry_route_progress(&self) -> Option<(usize, f64)> {
        let runtime = self.runtime?;
        let route = &self.pending_move.as_ref()?.navigation.as_ref()?.route;
        let waypoint = runtime.navigation_waypoint;
        let target = route.waypoints.get(waypoint)?;
        Some((
            waypoint,
            point(runtime.position)
                .iter()
                .zip(target)
                .map(|(a, b)| (a - b).powi(2))
                .sum::<f64>()
                .sqrt(),
        ))
    }
    pub(super) fn begin_registry_move(
        &mut self,
        mesh: Arc<navmesh::NavMesh>,
        map: &str,
        movement: BotMove,
        now: Duration,
        health: u16,
        rules: Option<&fall_damage::Rules>,
    ) -> Result<(), &'static str> {
        self.begin_registry_move_with_tolerance(mesh, map, movement, now, health, rules, None)
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn begin_registry_move_with_tolerance(
        &mut self,
        mesh: Arc<navmesh::NavMesh>,
        map: &str,
        movement: BotMove,
        now: Duration,
        health: u16,
        rules: Option<&fall_damage::Rules>,
        goal_tolerance: Option<f64>,
    ) -> Result<(), &'static str> {
        self.begin_registry_move_with_drop_costs(
            mesh,
            map,
            movement,
            now,
            health,
            rules,
            goal_tolerance,
            &std::collections::BTreeMap::new(),
            None,
        )
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn begin_registry_move_with_drop_costs(
        &mut self,
        mesh: Arc<navmesh::NavMesh>,
        map: &str,
        movement: BotMove,
        now: Duration,
        health: u16,
        rules: Option<&fall_damage::Rules>,
        goal_tolerance: Option<f64>,
        penalties: &std::collections::BTreeMap<u32, f64>,
        first_drop: Option<u32>,
    ) -> Result<(), &'static str> {
        let runtime = self.runtime.ok_or("bot_not_spawned")?;
        movement.validate_from(runtime.position)?;
        let navigation = NavigationMove::plan_navmesh_with_drop_costs(
            mesh,
            map,
            runtime.position,
            &movement,
            rules,
            health,
            penalties,
            first_drop,
        )?;
        if let Some(tolerance) = goal_tolerance
            && navigation.navmesh.as_ref().is_none_or(|(_, route)| {
                point(movement.destination)
                    .iter()
                    .zip(route.projected_goal)
                    .map(|(a, b)| (a - b).powi(2))
                    .sum::<f64>()
                    .sqrt()
                    > tolerance
            })
        {
            return Err("bot_objective_projection_too_far");
        }
        self.pending_move = Some(PendingMove {
            movement_id: format!("population-{}-{}", self.object_index, runtime.generation),
            from: runtime.position,
            movement,
            navigation: Some(navigation),
        });
        let mut navmesh_state = runtime.navmesh_state;
        navmesh_state.drop_wait = Duration::ZERO;
        navmesh_state.navigation_aborted = false;
        self.runtime = Some(BotRuntime {
            navmesh_state,
            phase: BotPhase::Relocating,
            since: now,
            last_update: now,
            navigation_waypoint: 0,
            ..runtime
        });
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn begin_registry_ground_recovery(
        &mut self,
        world: &collision::CollisionWorld,
        mesh: Arc<navmesh::NavMesh>,
        map: &str,
        movement: BotMove,
        now: Duration,
        maximum_m: f64,
        entry_buffer: f64,
        actors: crate::actor_collision::Query<'_>,
        mut query_budget: usize,
    ) -> Result<(), &'static str> {
        let runtime = self.runtime.ok_or("bot_not_spawned")?;
        let initial = runtime.physics_motion.ok_or("bot_physics_state_missing")?;
        if !initial.grounded
            || runtime.navmesh_state.on_player
            || !runtime.navmesh_state.navigation_aborted
            || mesh.component_id(initial.position).is_ok()
            || movement.mode != BotMoveMode::Navigation
            || movement.sprint
            || !maximum_m.is_finite()
            || !(0.1..=8.).contains(&maximum_m)
        {
            return Err("bot_ground_recovery_requires_supported_offmesh_stop");
        }
        movement.validate_from(runtime.position)?;
        let target = point(movement.destination);
        if (target[0] - initial.position[0]).hypot(target[2] - initial.position[2]) > maximum_m
            || mesh.component_id(target).is_err()
        {
            return Err("bot_ground_recovery_goal_not_local_mesh");
        }
        let navigation = match NavigationMove::plan(Some(world), map, runtime.position, &movement) {
            Ok(route) => route,
            Err("bot_navigation_unreachable") => NavigationMove::supported_recovery_prefix(
                world, map, initial, &movement, maximum_m,
            )?,
            Err(e) => return Err(e),
        };
        if navigation.length > maximum_m {
            return Err("bot_ground_recovery_route_too_long");
        }
        if navigation
            .route
            .waypoints
            .last()
            .is_none_or(|p| mesh.component_id(*p).is_err())
        {
            return Err("bot_ground_recovery_endpoint_not_mesh");
        }
        let mut from = initial.position;
        let approach: Vec<_> = navigation
            .route
            .waypoints
            .iter()
            .map(|&to| {
                let segment = [from, to];
                from = to;
                segment
            })
            .collect();
        if mesh.artifact().drop_links.iter().any(|link| {
            let protected = protected_drop_bounds(link, entry_buffer);
            approach.iter().any(|&[a, b]| {
                bounds_intersect(
                    crate::actor_collision::Bounds::body(a, Default::default())
                        .union(crate::actor_collision::Bounds::body(b, Default::default())),
                    protected,
                )
            })
        }) {
            return Err("bot_ground_recovery_crosses_passage");
        }
        if crate::bot_traffic_flow::approach_blocker(
            actors.grid,
            actors.actor,
            &approach,
            &mut query_budget,
        )?
        .is_some()
        {
            return Err("bot_ground_recovery_route_occupied");
        }
        self.pending_move = Some(PendingMove {
            movement_id: format!("population-{}-{}", self.object_index, runtime.generation),
            from: runtime.position,
            movement,
            navigation: Some(navigation),
        });
        let mut navmesh_state = runtime.navmesh_state;
        navmesh_state.navigation_aborted = false;
        navmesh_state.drop_wait = Duration::ZERO;
        self.runtime = Some(BotRuntime {
            navmesh_state,
            phase: BotPhase::Relocating,
            since: now,
            last_update: now,
            navigation_waypoint: 0,
            ..runtime
        });
        Ok(())
    }
    pub(super) fn registry_ground_recovery(&self) -> bool {
        self.pending_move
            .as_ref()
            .is_some_and(|p| p.movement.mode == BotMoveMode::Navigation)
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn idle_registry_step(
        &self,
        now: Duration,
        profile: wire_profile::WireProfile,
        world: Option<&collision::CollisionWorld>,
        map: &str,
        health: u16,
        rules: Option<&fall_damage::Rules>,
        actors: crate::actor_collision::Query<'_>,
    ) -> Result<Option<BotStep>, &'static str> {
        let Some(mut runtime) = self.runtime else {
            return Ok(None);
        };
        let dt = now
            .saturating_sub(runtime.last_update)
            .as_secs_f64()
            .min(1.);
        if dt < 0.25 {
            return Ok(None);
        }
        let Some(prior) = runtime.physics_motion else {
            return Ok(None);
        };
        let terrain = bot_physics::WorldTerrain::new(world, map)
            .map_err(navigation_physics_error)?
            .with_actors(actors);
        let mut context = NavigationContext {
            state: runtime.navmesh_state,
            hp: health,
            rules,
            generation: runtime.generation,
            hits: Vec::new(),
            ground_events: Vec::new(),
            sprint: false,
            facing: None,
            recovered_contacts: 0,
            actors: Some(actors),
            waiting_on: None,
            admission: DropAdmission::Unrestricted,
            waiting_drop: None,
            waited_seconds: 0.,
            avoiding: false,
            swept_bounds: Some(crate::actor_collision::Bounds::body(
                prior.position,
                bot_physics::Body::default(),
            )),
        };
        let (motion, complete) = context.idle_physics(&terrain, prior, dt, 4)?;
        if !complete {
            return Err("bot_navigation_step_limit");
        }
        let hp = (context.hp < health).then_some(context.hp);
        let moved = (0..3).any(|i| (motion.position[i] - prior.position[i]).abs() > 1e-6);
        runtime.physics_motion = Some(motion);
        runtime.position = character_position(motion.position);
        runtime.navmesh_state = context.state;
        runtime.last_update = now;
        if hp == Some(0) {
            runtime.phase = BotPhase::Dead;
            runtime.since = now;
            runtime.physics_motion = None;
        }
        let packet = if moved || hp.is_some() {
            let fields = player_tracking::ClientCharacterFields {
                position: Some(runtime.position),
                state: (hp == Some(0)).then_some(replication::CharacterState {
                    state: 7,
                    stance: 0,
                    user_data: [0; 3],
                }),
                ..Default::default()
            };
            replication::multiplayer::CharacterOwnerDelta {
                sequence: 2,
                timestamp_ms: now.as_millis() as u32,
                player: self.object_index,
                fields: &fields,
            }
            .encode_with_damage(
                profile,
                hp.map(|health_quarter_points| replication::CharacterDamage {
                    last_location_and_type: 5 << 4,
                    health_quarter_points,
                    conditions: 0,
                    last_attacker: 0x03800000 | u32::from(self.object_index),
                    last_source_object: u32::MAX,
                    last_user_data: 0,
                }),
            )?
        } else {
            Vec::new()
        };
        Ok(Some(BotStep {
            packet,
            runtime,
            health: hp,
            swept_bounds: context.swept_bounds,
            receipt: json!({"op":"bot_idle_physics","ground_events":context.ground_events,"name":self.display_name.unwrap_or("BOT Mercer"),"object_id":0x03800000|u32::from(self.object_index),"generation":runtime.generation,"position":runtime.position,"health_quarter_points":context.hp,"simulated_seconds":dt,"player_collision":{"blocking_object_index":context.waiting_on}}),
        }))
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn movement_step_with_actors(
        &self,
        sequence: u32,
        now: Duration,
        profile: wire_profile::WireProfile,
        collision_world: Option<&collision::CollisionWorld>,
        map_name: &str,
        health: u16,
        fall_rules: Option<&fall_damage::Rules>,
        actors: Option<crate::actor_collision::Query<'_>>,
    ) -> Result<Option<BotStep>, &'static str> {
        self.movement_step_with_drop_admission(
            sequence,
            now,
            profile,
            collision_world,
            map_name,
            health,
            fall_rules,
            actors,
            DropAdmission::Unrestricted,
        )
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn movement_step_with_drop_admission(
        &self,
        sequence: u32,
        now: Duration,
        profile: wire_profile::WireProfile,
        collision_world: Option<&collision::CollisionWorld>,
        map_name: &str,
        health: u16,
        fall_rules: Option<&fall_damage::Rules>,
        actors: Option<crate::actor_collision::Query<'_>>,
        admission: DropAdmission,
    ) -> Result<Option<BotStep>, &'static str> {
        if let DropAdmission::Permit { entry_buffer_m, .. } = admission
            && (!entry_buffer_m.is_finite() || !(0.0..=5.0).contains(&entry_buffer_m))
        {
            return Err("bot_drop_admission_invalid_buffer");
        }
        let Some(mut runtime) = self.runtime else {
            return Ok(None);
        };
        let Some(pending) = &self.pending_move else {
            return Err("bot_move_state_missing");
        };
        if now.saturating_sub(runtime.last_update)
            < Duration::from_millis(pending.movement.update_ms.into())
        {
            return Ok(None);
        }
        let wall_elapsed = now.saturating_sub(runtime.since).as_millis();
        let mut elapsed = wall_elapsed.saturating_sub(runtime.navmesh_state.drop_wait.as_millis());
        let mut finished = elapsed >= u128::from(pending.movement.duration_ms);
        let fraction = (elapsed as f32 / pending.movement.duration_ms as f32).min(1.0);
        let from = pending.from;
        let destination = pending.movement.destination;
        let mut position = if finished {
            destination
        } else {
            replication::CharacterPosition {
                x: from.x + (destination.x - from.x) * fraction,
                y: from.y,
                z: from.z + (destination.z - from.z) * fraction,
            }
        };
        let mut physics_error = None;
        let mut simulated_seconds = None;
        let mut navigation_error = None;
        let mut navigation_receipt = None;
        let mut landing_health = None;
        let mut context = NavigationContext {
            state: runtime.navmesh_state,
            hp: health,
            rules: fall_rules,
            generation: runtime.generation,
            hits: Vec::new(),
            ground_events: Vec::new(),
            sprint: pending.movement.sprint,
            facing: None,
            recovered_contacts: 0,
            actors: None,
            waiting_on: None,
            admission: DropAdmission::Unrestricted,
            waiting_drop: None,
            waited_seconds: 0.,
            avoiding: false,
            swept_bounds: None,
        };
        context.admission = admission;
        context.actors = actors;
        context.swept_bounds = Some(crate::actor_collision::Bounds::body(
            runtime.physical_position(),
            bot_physics::Body::default(),
        ));
        if let Some(navigation) = &pending.navigation {
            let dt = now
                .saturating_sub(runtime.last_update)
                .as_secs_f64()
                .min(1.0);
            simulated_seconds = Some(dt);
            let horizontal_seconds =
                (f64::from(pending.movement.duration_ms + NAVIGATION_GRACE_MS) / 1000.0
                    - runtime
                        .last_update
                        .saturating_sub(runtime.since)
                        .saturating_sub(runtime.navmesh_state.drop_wait)
                        .as_secs_f64())
                .max(0.0)
                .min(dt);
            let prior = runtime.physics_motion.ok_or("bot_physics_state_missing")?;
            #[cfg(test)]
            if std::env::var_os("MAG_BOT_DROP_TRACE_PATH").is_some()
                && let Some(drop_id) = navigation.navmesh.as_ref().and_then(|(_, route)| {
                    route
                        .waypoint_drop_ids
                        .iter()
                        .skip(runtime.navigation_waypoint)
                        .find_map(|id| *id)
                })
            {
                let local_actors = actors.map(|query| query.grid.diagnostic_entries().into_iter().map(|(id,bounds,allow_overlap_escape)|json!({"id":id,"min":bounds.min,"max":bounds.max,"allow_overlap_escape":allow_overlap_escape})).collect::<Vec<_>>());
                private_drop_trace(
                    json!({"kind":"drop_update_input","drop_id":drop_id,"actor":actors.map(|q|q.actor),"generation":runtime.generation,"timestamp_ms":now.as_millis()as u64,"last_update_ms":runtime.last_update.as_millis()as u64,"motion":prior,"update_dt_s":dt,"horizontal_s":horizontal_seconds,"waypoint":runtime.navigation_waypoint,"local_actors":local_actors}),
                );
            }
            let result = navigation.advance(
                collision_world,
                map_name,
                prior,
                runtime.navigation_waypoint,
                dt,
                horizontal_seconds,
                Some(&mut context),
            );
            match result {
                Ok(step) => {
                    elapsed = elapsed.saturating_sub(
                        Duration::from_secs_f64(context.waited_seconds).as_millis(),
                    );
                    let drop_events:Vec<_>=navigation.navmesh.as_ref().map(|(_,route)| {
                        (runtime.navigation_waypoint..step.waypoint.min(route.waypoints.len())).filter_map(|index| {
                            let kind=route.waypoint_kinds[index];
                            (kind!=navmesh::WaypointKind::Walk).then(||json!({"kind":kind,"drop_id":route.waypoint_drop_ids[index],"waypoint_index":index}))
                        }).collect()
                    }).unwrap_or_default();
                    position = character_position(step.motion.position);
                    runtime.physics_motion = Some(step.motion);
                    runtime.navigation_waypoint = step.waypoint;
                    runtime.navmesh_state = context.state;
                    if context.hp < health {
                        landing_health = Some(context.hp);
                    }

                    navigation_error = step.stopped;
                    let deadline =
                        elapsed >= u128::from(pending.movement.duration_ms + NAVIGATION_GRACE_MS);
                    if navigation_error.is_none()
                        && !step.reached
                        && deadline
                        && step.motion.grounded
                    {
                        navigation_error = Some(if context.waiting_on.is_some() {
                            if (position.x - destination.x).hypot(position.z - destination.z) < 1.5
                            {
                                "bot_navigation_goal_occupied"
                            } else {
                                "bot_navigation_player_blocked_deadline"
                            }
                        } else {
                            "bot_navigation_deadline"
                        });
                    }
                    if navigation_error.is_none()
                        && !step.reached
                        && elapsed >= u128::from(pending.movement.duration_ms + NAVIGATION_GRACE_MS)
                    {
                        navigation_error = Some("bot_navigation_settle_deadline");
                    }
                    runtime.navmesh_state.navigation_aborted = navigation_error.is_some();
                    finished = step.reached || navigation_error.is_some();
                    let mut receipt = navigation.receipt();
                    receipt["recovered_contacts"] = json!(context.recovered_contacts);
                    receipt["stamina"] = json!(context.state.movement.stamina());
                    receipt["sprinting"] = json!(context.state.movement.sprinting());
                    receipt["landing_hits"] = json!(context.hits);
                    receipt["certified_drop_events"] = json!(drop_events);
                    receipt["health_before_quarter_points"] = json!(health);
                    receipt["health_after_quarter_points"] = json!(context.hp);
                    receipt["waypoint_index"] = json!(step.waypoint);
                    receipt["waiting_drop_id"] = json!(context.waiting_drop);
                    receipt["drop_wait_ms"] = json!(context.state.drop_wait.as_millis() as u64);
                    receipt["status"] = json!(if context.waiting_drop.is_some()
                        && navigation_error.is_none()
                    {
                        "waiting_for_drop_admission"
                    } else if context.waiting_on.is_some() && navigation_error.is_none() {
                        "waiting_for_player"
                    } else if navigation_error.is_some() {
                        "stopped"
                    } else if step.reached {
                        "reached"
                    } else {
                        "following"
                    });
                    navigation_receipt = Some(receipt);
                }
                Err(error) => {
                    position = runtime.position;
                    navigation_error = Some(error);
                    runtime.navmesh_state.navigation_aborted = true;
                    finished = true;
                    let mut receipt = navigation.receipt();
                    receipt["waypoint_index"] = json!(runtime.navigation_waypoint);
                    receipt["status"] = json!("stopped");
                    navigation_receipt = Some(receipt);
                }
            }
            if let Some(receipt) = navigation_receipt.as_mut() {
                receipt["within_grace"] =
                    json!(elapsed >= u128::from(pending.movement.duration_ms));
                receipt["wall_elapsed_ms"] = json!(wall_elapsed as u64);
                receipt["active_elapsed_ms"] = json!(elapsed as u64);
            }
        } else if pending.movement.mode != BotMoveMode::LabPose {
            // A late owner update never runs an unbounded catch-up or snaps to
            // the destination; unsimulated time is explicitly discarded.
            let dt = now
                .saturating_sub(runtime.last_update)
                .as_secs_f64()
                .min(1.0);
            simulated_seconds = Some(dt);
            let prior = runtime.physics_motion.ok_or("bot_physics_state_missing")?;
            let dx = f64::from(destination.x - runtime.position.x);
            let dz = f64::from(destination.z - runtime.position.z);
            let distance = dx.hypot(dz);
            let speed = f64::from((destination.x - from.x).hypot(destination.z - from.z))
                / (f64::from(pending.movement.duration_ms) / 1000.0);
            let remaining = (f64::from(pending.movement.duration_ms) / 1000.0
                - now.saturating_sub(runtime.since).as_secs_f64()
                + now.saturating_sub(runtime.last_update).as_secs_f64())
            .max(0.0)
            .min(dt);
            let factor = if pending.movement.mode == BotMoveMode::Physical && distance > 0.0001 {
                speed.min(distance / dt) * remaining / dt / distance
            } else {
                0.0
            };
            let result =
                bot_physics::WorldTerrain::new(collision_world, map_name).and_then(|terrain| {
                    let terrain = if let Some(query) = actors {
                        terrain.with_actors(query)
                    } else {
                        terrain
                    };
                    let result = terrain.advance(
                        prior,
                        [dx * factor, 0.0, dz * factor],
                        dt,
                        bot_physics::Body::default(),
                    );
                    context.waiting_on = terrain.actor_contact();
                    result
                });
            match result {
                Ok(motion) => {
                    position = character_position(motion.position);
                    runtime.physics_motion = Some(motion);
                    finished = finished
                        && (motion.grounded
                            || elapsed >= u128::from(pending.movement.duration_ms) + 5000);
                }
                Err(error) => {
                    position = runtime.position;
                    physics_error = Some(error.code());
                    finished = true;
                }
            }
        }
        if pending.movement.mode == BotMoveMode::LabPose {
            let dx = f64::from(position.x - runtime.position.x);
            let dz = f64::from(position.z - runtime.position.z);
            let d = dx.hypot(dz);
            let cap = retail_movement::WALK_BACKWARD_MPS
                * now.saturating_sub(runtime.last_update).as_secs_f64();
            if d > cap && d > 0. {
                position.x = runtime.position.x + (dx / d * cap) as f32;
                position.z = runtime.position.z + (dz / d * cap) as f32;
            }
        }
        if pending.movement.mode == BotMoveMode::LabPose
            && let Some(query) = actors
            && let Some(hit) = query.grid.sweep(
                query.actor,
                point(runtime.position),
                point(position),
                bot_physics::Body::default(),
                &mut 256,
            )?
        {
            position = runtime.position;
            context.waiting_on = Some(hit.id);
        }
        if finished
            && pending.movement.mode == BotMoveMode::LabPose
            && (position.x - destination.x).hypot(position.z - destination.z) > 0.03125
        {
            navigation_error = Some("bot_move_player_delay_deadline");
        }
        if finished
            && pending.movement.mode == BotMoveMode::Physical
            && context.waiting_on.is_some()
            && (position.x - destination.x).hypot(position.z - destination.z) > 0.03125
        {
            navigation_error = Some("bot_move_player_blocked_deadline");
        }
        if pending.movement.mode != BotMoveMode::Navmesh {
            runtime.navmesh_state.movement.advance(
                if pending.movement.mode == BotMoveMode::Gravity {
                    retail_movement::MovementIntent::Idle
                } else {
                    retail_movement::MovementIntent::Walk
                },
                now.saturating_sub(runtime.last_update)
                    .as_secs_f64()
                    .min(1.),
            )?;
        }
        if landing_health == Some(0) {
            runtime.phase = BotPhase::Dead;
            runtime.since = now;
            runtime.physics_motion = None;
            finished = true;
        }
        let fields = player_tracking::ClientCharacterFields {
            state: (landing_health == Some(0)).then_some(replication::CharacterState {
                state: 7,
                stance: 0,
                user_data: [0; 3],
            }),
            position: Some(position),
            yaw: if pending.movement.mode == BotMoveMode::Navmesh {
                context.facing
            } else {
                pending.movement.yaw
            },
            ..Default::default()
        };
        let packet = replication::multiplayer::CharacterOwnerDelta {
            sequence,
            timestamp_ms: now.as_millis() as u32,
            player: self.object_index,
            fields: &fields,
        }
        .encode_with_damage(
            profile,
            landing_health.map(|hp| replication::CharacterDamage {
                last_location_and_type: 5 << 4,
                health_quarter_points: hp,
                conditions: 0,
                last_attacker: (0x0380_0000 | u32::from(self.object_index)),
                last_source_object: u32::MAX,
                last_user_data: 0,
            }),
        )?;
        runtime.position = position;
        runtime.last_update = now;
        if finished && landing_health != Some(0) {
            runtime.phase = BotPhase::Complete;
        }
        Ok(Some(BotStep {
            packet,
            runtime,
            health: landing_health,
            swept_bounds: context.swept_bounds,
            receipt: json!({"object_id":(0x0380_0000 | u32::from(self.object_index)),"name":self.display_name.unwrap_or("BOT Mercer"),"owner_index":dummy::SERVER_OWNER,
            "team":self.squad.team(),"squad_index":self.squad.squad_index(),"faction":self.faction,"owner_life_generation":self.generation,"generation":runtime.generation,
            "phase":if physics_error.is_some() || navigation_error.is_some(){"move_stopped"}else if finished{"move_complete"}else{"move"},"route_leg":"redirect","movement_id":pending.movement_id,
            "position":position,"destination":destination,"sequence":sequence,"timestamp_ms":now.as_millis() as u32,
            "placement_policy":pending.movement.mode.policy(),"physics":runtime.physics_motion,"physics_error":physics_error,"navigation_error":navigation_error,"navigation":navigation_receipt,"ground_events":context.ground_events,
            "player_collision":{"blocking_object_index":context.waiting_on,"avoiding":context.avoiding,"policy":"swept_upright_box_players"},"facing_yaw":context.facing,"simulated_seconds":simulated_seconds,"settle_deadline_reached_airborne":physics_error.is_none() && (navigation_error.is_none() || navigation_error==Some("bot_navigation_settle_deadline")) && elapsed>=u128::from(pending.movement.duration_ms)+5000 && runtime.physics_motion.is_some_and(|m|!m.grounded),
            "collision_world_sha256":if pending.movement.mode==BotMoveMode::LabPose {None} else {collision_world.map(|w|w.source_sha256())},"combat_policy":"ordinary_client_reported_bullets_when_catalog_installed"}),
        }))
    }
}

const NAVIGATION_MAX_SPEED: f64 = retail_movement::WALK_BACKWARD_MPS;
const NAVIGATION_MAX_LENGTH: f64 = 128.0;
const NAVIGATION_STEP_LIMIT: usize = 16;
const NAVIGATION_GRACE_MS: u32 = 5000;
// A geometry work budget belongs to each physical call. Keep late owner
// updates from putting twenty dense-world substeps into that single budget.
const NAVIGATION_SOLVER_SLICE_SECONDS: f64 = 0.25;
const NAVIGATION_LIMITS: bot_navigation::Limits = bot_navigation::Limits {
    cell_size: 1.0,
    radius_cells: 64,
    max_expansions: 2048,
    max_queries: 16384,
    max_waypoints: 256,
};
#[derive(Clone, Copy, Default)]
pub(super) struct NavmeshRuntime {
    pub movement: retail_movement::MovementState,
    pub fall: Option<fall_damage::PhysicalTracker>,
    pub on_player: bool,
    pub avoid_left: bool,
    pub drop_wait: Duration,
    pub navigation_aborted: bool,
}
#[derive(Clone)]
struct NavigationContext<'a> {
    state: NavmeshRuntime,
    hp: u16,
    rules: Option<&'a fall_damage::Rules>,
    generation: u32,
    hits: Vec<fall_damage::FallDamage>,
    ground_events: Vec<Value>,
    sprint: bool,
    facing: Option<u16>,
    recovered_contacts: usize,
    actors: Option<crate::actor_collision::Query<'a>>,
    waiting_on: Option<u16>,
    avoiding: bool,
    swept_bounds: Option<crate::actor_collision::Bounds>,
    admission: DropAdmission,
    waiting_drop: Option<u32>,
    waited_seconds: f64,
}
impl NavigationContext<'_> {
    fn idle_physics(
        &mut self,
        terrain: &bot_physics::WorldTerrain<'_>,
        mut motion: bot_physics::Motion,
        mut seconds: f64,
        calls: usize,
    ) -> Result<(bot_physics::Motion, bool), &'static str> {
        for _ in 0..calls.min(4) {
            if seconds <= 1e-7 {
                break;
            }
            let slice = seconds.min(NAVIGATION_SOLVER_SLICE_SECONDS);
            motion = self.advance(terrain, motion, [0.; 3], slice)?;
            self.state
                .movement
                .advance(retail_movement::MovementIntent::Idle, slice)?;
            seconds = (seconds - slice).max(0.);
        }
        Ok((motion, seconds <= 1e-7))
    }
    fn advance(
        &mut self,
        terrain: &bot_physics::WorldTerrain<'_>,
        motion: bot_physics::Motion,
        velocity: [f64; 3],
        dt: f64,
    ) -> Result<bot_physics::Motion, &'static str> {
        if self.state.fall.is_none() {
            let mut tracker = fall_damage::PhysicalTracker::default();
            tracker.reset(self.generation, motion)?;
            self.state.fall = Some(tracker);
        }
        if velocity[0].hypot(velocity[2]) > 1e-9 {
            self.facing = Some(retail_movement::facing_yaw(velocity[0], velocity[2])?);
        }
        let mut error = None;
        let mut grounded = motion.grounded;
        let result = terrain
            .advance_observed(
                motion,
                velocity,
                dt,
                bot_physics::Body::default(),
                |sample| {
                    if sample.grounded!=grounded {
                        self.ground_events.push(json!({"kind":if sample.grounded {"landing"} else {"departure"},"position":sample.position}));
                        grounded=sample.grounded;
                    }
                    let bounds = crate::actor_collision::Bounds::body(
                        sample.position,
                        bot_physics::Body::default(),
                    );
                    self.swept_bounds =
                        Some(self.swept_bounds.map_or(bounds, |old| old.union(bounds)));
                    if let Some(rules) = self.rules {
                        match self.state.fall.as_mut().unwrap().observe(
                            rules,
                            sample,
                            self.generation,
                        ) {
                            Ok(Some(hit)) => match rules.predict_drop(hit.drop_height, self.hp) {
                                Ok(prediction) => {
                                    self.hp = prediction.remaining_health_quarter_points;
                                    self.hits.push(hit);
                                }
                                Err(e) => error = Some(e),
                            },
                            Ok(None) => {}
                            Err(e) => error = Some(e),
                        }
                    }
                },
            )
            .map_err(navigation_physics_error)?;
        if let Some(error) = error {
            return Err(error);
        }
        self.state.on_player = false;
        if result.grounded
            && let Some(query) = self.actors
        {
            let down = [
                result.position[0],
                result.position[1] - 0.02,
                result.position[2],
            ];
            if let Some(hit) = query.grid.sweep(
                query.actor,
                result.position,
                down,
                bot_physics::Body::default(),
                &mut 256,
            )? && hit.normal[1] > 0.7
            {
                self.state.on_player = true;
                self.waiting_on = Some(hit.id);
            }
        }
        Ok(result)
    }
}
struct GroundEgress {
    mesh: Arc<navmesh::NavMesh>,
    passage: DropPassage,
    retreat: bool,
    precommit: bool,
    target: navmesh::Point,
    samples: Vec<bot_physics::Motion>,
}
struct NavigationMove {
    supported_recovery: bool,
    ground_egress: Option<Arc<GroundEgress>>,
    navmesh: Option<(Arc<navmesh::NavMesh>, Arc<navmesh::Route>)>,
    route: Arc<bot_navigation::Route>,
    bounds: bot_navigation::PlanningBounds,
    speed: f64,
    length: f64,
}
struct NavigationStep {
    motion: bot_physics::Motion,
    waypoint: usize,
    reached: bool,
    stopped: Option<&'static str>,
}
fn navigation_error(error: bot_navigation::PlanError) -> &'static str {
    use bot_navigation::PlanError;
    match error {
        PlanError::InvalidRequest => "bot_navigation_invalid_request",
        PlanError::InvalidStart => "bot_navigation_invalid_start",
        PlanError::InvalidGoal => "bot_navigation_invalid_goal",
        PlanError::GeometryUnavailable => "bot_navigation_geometry_unavailable",
        PlanError::QueryLimit => "bot_navigation_query_limit",
        PlanError::QueryFailed => "bot_navigation_query_failed",
        PlanError::SearchLimit => "bot_navigation_search_limit",
        PlanError::RouteLimit => "bot_navigation_route_limit",
        PlanError::Unreachable => "bot_navigation_unreachable",
        PlanError::RouteInvalidated => "bot_navigation_route_invalidated",
        PlanError::ReplanLimit => "bot_navigation_replan_limit",
    }
}
fn navigation_physics_error(error: bot_physics::PhysicsError) -> &'static str {
    match error {
        bot_physics::PhysicsError::Unavailable
        | bot_physics::PhysicsError::MapMismatch
        | bot_physics::PhysicsError::CoverageUnknown => "bot_navigation_geometry_unavailable",
        bot_physics::PhysicsError::QueryFailed => "bot_navigation_query_failed",
        bot_physics::PhysicsError::InvalidQuery => "bot_navigation_invalid_request",
    }
}
impl NavigationMove {
    fn drop_passage(
        &self,
        runtime: BotRuntime,
        id: u32,
        entry_buffer_m: f64,
    ) -> Option<DropPassage> {
        if !entry_buffer_m.is_finite() || !(0.0..=5.0).contains(&entry_buffer_m) {
            return None;
        }
        if let Some(egress) = &self.ground_egress {
            if egress.passage.id != id {
                return None;
            }
            let motion = runtime.physics_motion?;
            let mut passage = egress.passage;
            passage.phase = if !motion.grounded || runtime.navmesh_state.on_player {
                DropPhase::Traversing
            } else if bounds_intersect(
                passage.protected_bounds,
                crate::actor_collision::Bounds::body(motion.position, Default::default()),
            ) {
                if egress.retreat {
                    DropPhase::Retreating
                } else {
                    DropPhase::Landed
                }
            } else if egress.mesh.component_id(motion.position).is_ok() {
                if egress.retreat {
                    DropPhase::Retreated
                } else {
                    DropPhase::Cleared
                }
            } else {
                if egress.retreat {
                    DropPhase::Retreating
                } else {
                    DropPhase::Landed
                }
            };
            return Some(passage);
        }
        let (mesh, route) = self.navmesh.as_ref()?;
        let start = route
            .waypoint_kinds
            .iter()
            .enumerate()
            .position(|(i, kind)| {
                *kind == navmesh::WaypointKind::DropStart
                    && route.waypoint_drop_ids.get(i) == Some(&Some(id))
            })?;
        let end = route
            .waypoint_kinds
            .iter()
            .enumerate()
            .skip(start + 1)
            .find_map(|(i, kind)| {
                (*kind == navmesh::WaypointKind::DropEnd
                    && route.waypoint_drop_ids.get(i) == Some(&Some(id)))
                .then_some(i)
            })?;
        let link = mesh
            .artifact()
            .drop_links
            .iter()
            .find(|link| link.id == id)?;
        let motion = runtime.physics_motion?;
        let protected_bounds = protected_drop_bounds(link, entry_buffer_m);
        let aborted_lower_landing = runtime.navmesh_state.navigation_aborted
            && motion.grounded
            && !runtime.navmesh_state.on_player
            && motion.position[1] <= link.landing[1] + bot_physics::Body::default().max_step;
        let phase = if runtime.navigation_waypoint <= start {
            DropPhase::Approaching
        } else if (runtime.navigation_waypoint <= end && !aborted_lower_landing)
            || !motion.grounded
            || runtime.navmesh_state.on_player
        {
            DropPhase::Traversing
        } else if bounds_intersect(
            protected_bounds,
            crate::actor_collision::Bounds::body(motion.position, bot_physics::Body::default()),
        ) {
            DropPhase::Landed
        } else {
            DropPhase::Cleared
        };
        let mut route_distance_to_launch = 0.;
        if runtime.navigation_waypoint <= start {
            let mut from = motion.position;
            for to in &route.waypoints[runtime.navigation_waypoint..=start] {
                route_distance_to_launch += (to[0] - from[0]).hypot(to[2] - from[2]);
                from = *to;
            }
        }
        Some(DropPassage {
            id,
            launch: link.launch,
            landing: link.landing,
            protected_bounds,
            phase,
            distance_to_launch: (link.launch[0] - motion.position[0])
                .hypot(link.launch[2] - motion.position[2]),
            route_distance_to_launch,
        })
    }
    #[allow(clippy::too_many_arguments)]
    fn wait_drop_admission(
        &self,
        terrain: &bot_physics::WorldTerrain<'_>,
        mesh: &navmesh::NavMesh,
        route: &navmesh::Route,
        motion: bot_physics::Motion,
        waypoint: usize,
        seconds: f64,
        calls: usize,
        id: u32,
        context: &mut NavigationContext<'_>,
    ) -> Result<NavigationStep, &'static str> {
        let staged = context.clone();
        let (mut held, complete) = context.idle_physics(terrain, motion, seconds, calls)?;
        if motion.grounded && held.grounded && !mesh.contains(held.position, route) {
            *context = staged;
            held = motion;
            context
                .state
                .movement
                .advance(retail_movement::MovementIntent::Idle, seconds)?;
        }
        context.waiting_drop = Some(id);
        if complete {
            context.waited_seconds += seconds;
            context.state.drop_wait += Duration::from_secs_f64(seconds);
        }
        Ok(NavigationStep {
            motion: held,
            waypoint,
            reached: false,
            stopped: (!complete).then_some("bot_navigation_step_limit"),
        })
    }
    fn advance_ground_egress(
        &self,
        terrain: &bot_physics::WorldTerrain<'_>,
        egress: &GroundEgress,
        mut motion: bot_physics::Motion,
        mut seconds: f64,
        context: &mut NavigationContext<'_>,
    ) -> Result<NavigationStep, &'static str> {
        // A proven precommit backout evacuates an already occupied gate; it
        // never enters or resumes its certificate and may not own the permit.
        // Committed lower egress and upper-abort retreat retain ownership.
        if !egress.precommit && !context.admission.permits(egress.passage.id) {
            let (held, complete) = context.idle_physics(terrain, motion, seconds, 4)?;
            context.waiting_drop = Some(egress.passage.id);
            if complete {
                context.waited_seconds += seconds;
                context.state.drop_wait += Duration::from_secs_f64(seconds);
            }
            return Ok(NavigationStep {
                motion: held,
                waypoint: 0,
                reached: false,
                stopped: (!complete).then_some("bot_navigation_step_limit"),
            });
        }
        for _ in 0..NAVIGATION_STEP_LIMIT {
            let dx = egress.target[0] - motion.position[0];
            let dz = egress.target[2] - motion.position[2];
            let distance = dx.hypot(dz);
            if motion.grounded
                && !context.state.on_player
                && distance <= 1.0 / 32.0
                && !bounds_intersect(
                    egress.passage.protected_bounds,
                    crate::actor_collision::Bounds::body(motion.position, Default::default()),
                )
                && egress.mesh.component_id(motion.position).is_ok()
            {
                return Ok(NavigationStep {
                    motion,
                    waypoint: 1,
                    reached: true,
                    stopped: None,
                });
            }
            if seconds <= 1e-7 {
                return Ok(NavigationStep {
                    motion,
                    waypoint: 0,
                    reached: false,
                    stopped: None,
                });
            }
            if !egress.samples.iter().any(|sample| {
                (0..3)
                    .map(|i| (sample.position[i] - motion.position[i]).powi(2))
                    .sum::<f64>()
                    <= 0.6 * 0.6
            }) {
                return Err("bot_egress_outside_validated_trajectory");
            }
            let slice = seconds.min(NAVIGATION_SOLVER_SLICE_SECONDS);
            let speed = retail_movement::WALK_FORWARD_MPS.min(distance / slice);
            let velocity = if distance > 1.0 / 32.0 {
                [dx / distance * speed, 0., dz / distance * speed]
            } else {
                [0.; 3]
            };
            motion = context.advance(terrain, motion, velocity, slice)?;
            context.state.movement.advance_direction(
                if distance > 1.0 / 32.0 {
                    retail_movement::MovementIntent::Walk
                } else {
                    retail_movement::MovementIntent::Idle
                },
                slice,
                retail_movement::MovementDirection::Forward,
            )?;
            seconds = (seconds - slice).max(0.);
            if motion.blocked || context.hp == 0 {
                return Ok(NavigationStep {
                    motion,
                    waypoint: 0,
                    reached: false,
                    stopped: Some(if context.hp == 0 {
                        "bot_navmesh_fall_lethal"
                    } else {
                        "bot_navigation_blocked"
                    }),
                });
            }
        }
        Ok(NavigationStep {
            motion,
            waypoint: 0,
            reached: false,
            stopped: Some("bot_navigation_step_limit"),
        })
    }
    fn plan(
        world: Option<&collision::CollisionWorld>,
        map_name: &str,
        start: replication::CharacterPosition,
        movement: &BotMove,
    ) -> Result<Self, &'static str> {
        let extent = f64::from(NAVIGATION_LIMITS.radius_cells) * NAVIGATION_LIMITS.cell_size
            + bot_physics::Body::default().radius;
        let bounds = bot_navigation::PlanningBounds {
            min_xz: [
                (f64::from(start.x) - extent).max(0.0),
                (f64::from(start.z) - extent).max(0.0),
            ],
            max_xz: [
                (f64::from(start.x) + extent).min(2047.96875),
                (f64::from(start.z) + extent).min(2047.96875),
            ],
        };
        let query = bot_navigation::WorldNavigation::new(
            world,
            map_name,
            bot_physics::Body::default(),
            bounds,
            16.0,
            16.0,
        )
        .map_err(navigation_error)?;
        let route = bot_navigation::plan_route(
            &query,
            point(start),
            point(movement.destination),
            NAVIGATION_LIMITS,
        )
        .map_err(navigation_error)?;
        for p in &route.waypoints {
            character_position(*p)
                .validate()
                .map_err(|_| "bot_navigation_wire_bounds")?;
        }
        let length = route
            .waypoints
            .windows(2)
            .map(|w| (w[1][0] - w[0][0]).hypot(w[1][2] - w[0][2]))
            .sum::<f64>();
        if length > NAVIGATION_MAX_LENGTH {
            return Err("bot_navigation_route_distance_limit");
        }
        let speed = length / (f64::from(movement.duration_ms) / 1000.0);
        if speed > NAVIGATION_MAX_SPEED {
            return Err("bot_navigation_speed_limit");
        }
        Ok(Self {
            supported_recovery: false,
            navmesh: None,
            ground_egress: None,
            route: Arc::new(route),
            bounds,
            speed,
            length,
        })
    }
    /// A physically supported edge can lack the full-footprint grid support
    /// required by A*. Prove only a short actual grounded walk back to mesh;
    /// neither project the initial body nor broaden general grid semantics.
    fn supported_recovery_prefix(
        world: &collision::CollisionWorld,
        map: &str,
        initial: bot_physics::Motion,
        movement: &BotMove,
        maximum: f64,
    ) -> Result<Self, &'static str> {
        let terrain = bot_physics::WorldTerrain::new(Some(world), map).map_err(|e| e.code())?;
        let body = bot_physics::Body::default();
        let target = point(movement.destination);
        let distance = (target[0] - initial.position[0]).hypot(target[2] - initial.position[2]);
        let speed = distance / (movement.duration_ms as f64 / 1000.);
        if !(0.05..=retail_movement::WALK_FORWARD_MPS).contains(&speed) {
            return Err("bot_ground_recovery_prefix_speed");
        }
        let mut motion = initial;
        let mut waypoints = vec![initial.position];
        let mut length = 0.;
        for _ in 0..(movement.duration_ms / 50).min(200) {
            let delta = [
                target[0] - motion.position[0],
                target[2] - motion.position[2],
            ];
            let remaining = delta[0].hypot(delta[1]);
            if remaining <= body.radius * 0.5 {
                break;
            }
            let step_speed = speed.min(remaining / 0.05);
            let next = terrain
                .advance(
                    motion,
                    [
                        delta[0] / remaining * step_speed,
                        0.,
                        delta[1] / remaining * step_speed,
                    ],
                    0.05,
                    body,
                )
                .map_err(|e| e.code())?;
            if !next.grounded || next.blocked {
                return Err("bot_ground_recovery_prefix_not_supported");
            }
            length += (next.position[0] - motion.position[0])
                .hypot(next.position[2] - motion.position[2]);
            if length > maximum || waypoints.len() >= NAVIGATION_LIMITS.max_waypoints {
                return Err("bot_ground_recovery_route_too_long");
            }
            waypoints.push(next.position);
            motion = next;
        }
        if (target[0] - motion.position[0]).hypot(target[2] - motion.position[2])
            > body.radius * 0.5
        {
            return Err("bot_ground_recovery_prefix_not_reached");
        }
        let bounds = bot_navigation::PlanningBounds {
            min_xz: [
                initial.position[0] - maximum - body.radius,
                initial.position[2] - maximum - body.radius,
            ],
            max_xz: [
                initial.position[0] + maximum + body.radius,
                initial.position[2] + maximum + body.radius,
            ],
        };
        Ok(Self {
            supported_recovery: true,
            navmesh: None,
            ground_egress: None,
            route: Arc::new(bot_navigation::Route {
                waypoints,
                expansions: 0,
                queries: 0,
            }),
            bounds,
            speed: length / (movement.duration_ms as f64 / 1000.),
            length,
        })
    }
    fn plan_navmesh(
        mesh: Arc<navmesh::NavMesh>,
        map_name: &str,
        start: replication::CharacterPosition,
        movement: &BotMove,
        rules: Option<&fall_damage::Rules>,
        health: u16,
    ) -> Result<Self, &'static str> {
        Self::plan_navmesh_with_drop_costs(
            mesh,
            map_name,
            start,
            movement,
            rules,
            health,
            &std::collections::BTreeMap::new(),
            None,
        )
    }
    #[allow(clippy::too_many_arguments)]
    fn plan_navmesh_with_drop_costs(
        mesh: Arc<navmesh::NavMesh>,
        map_name: &str,
        start: replication::CharacterPosition,
        movement: &BotMove,
        rules: Option<&fall_damage::Rules>,
        health: u16,
        penalties: &std::collections::BTreeMap<u32, f64>,
        first_drop: Option<u32>,
    ) -> Result<Self, &'static str> {
        if mesh.artifact().map_name != map_name {
            return Err("bot_navmesh_map_mismatch");
        }
        let route = if let Some(rules) = rules {
            mesh.route_with_drop_costs(
                point(start),
                point(movement.destination),
                5.0,
                rules,
                health,
                penalties,
                first_drop,
            )
        } else if mesh.artifact().drop_links.is_empty() {
            mesh.route(point(start), point(movement.destination), 5.0)
        } else {
            return Err("bot_navmesh_fall_rules_required");
        }
        .map_err(|e| e.0)?;
        if route.status != navmesh::Status::Complete {
            return Err(match route.status {
                navmesh::Status::Partial => "bot_navmesh_partial",
                navmesh::Status::Disconnected => "bot_navmesh_disconnected",
                navmesh::Status::OutOfNodes => "bot_navmesh_out_of_nodes",
                navmesh::Status::InvalidStart => "bot_navmesh_invalid_start",
                navmesh::Status::InvalidGoal => "bot_navmesh_invalid_goal",
                navmesh::Status::Complete => unreachable!(),
            });
        }
        if route.length_m > 256.0 || route.waypoints.len() < 2 {
            return Err("bot_navigation_route_distance_limit");
        }
        let speed = retail_movement::WALK_FORWARD_MPS;
        let points = route.waypoints.clone();
        for p in &points {
            character_position(*p)
                .validate()
                .map_err(|_| "bot_navigation_wire_bounds")?;
        }
        Ok(Self {
            length: route.length_m,
            speed,
            bounds: bot_navigation::PlanningBounds {
                min_xz: [0.; 2],
                max_xz: [1.; 2],
            },
            route: Arc::new(bot_navigation::Route {
                waypoints: points,
                expansions: 0,
                queries: 1,
            }),
            supported_recovery: false,
            navmesh: Some((mesh, Arc::new(route))),
            ground_egress: None,
        })
    }
    fn receipt(&self) -> Value {
        let mut receipt = json!({"route_length":self.length,"speed_mps":self.speed,"waypoint_count":self.route.waypoints.len(),"expansions":self.route.expansions,"queries":self.route.queries,"replans":0,"max_step_iterations":NAVIGATION_STEP_LIMIT,"solver_slice_ms":250,"grace_ms":NAVIGATION_GRACE_MS});
        if let Some(egress) = &self.ground_egress {
            receipt["planner"] = json!(if egress.precommit {
                "physics_validated_precommit_passage_retreat"
            } else if egress.retreat {
                "physics_validated_aborted_drop_retreat"
            } else {
                "physics_validated_aborted_drop_egress"
            });
            receipt["world_sha256"] = json!(egress.mesh.artifact().world_sha256);
            receipt["egress_drop_id"] = json!(egress.passage.id);
            receipt["proof_samples"] = json!(egress.samples.len());
        }
        if let Some((mesh, route)) = &self.navmesh {
            receipt["planner"] = json!("recast_detour");
            receipt["world_sha256"] = json!(mesh.artifact().world_sha256);
            receipt["corridor_polygons"] = json!(route.corridor_refs.len());
            receipt["query_us"] = json!(route.query_us);
            receipt["projected_goal"] = json!(route.projected_goal);
        }
        receipt
    }
    #[allow(clippy::too_many_arguments)]
    fn advance(
        &self,
        world: Option<&collision::CollisionWorld>,
        map_name: &str,
        mut motion: bot_physics::Motion,
        waypoint: usize,
        dt: f64,
        mut horizontal_seconds: f64,
        context: Option<&mut NavigationContext<'_>>,
    ) -> Result<NavigationStep, &'static str> {
        let terrain =
            bot_physics::WorldTerrain::new(world, map_name).map_err(navigation_physics_error)?;
        let terrain = if let Some(query) = context.as_ref().and_then(|c| c.actors) {
            terrain.with_actors(query)
        } else {
            terrain
        };
        let body = bot_physics::Body::default();
        if let Some(egress) = &self.ground_egress {
            return self.advance_ground_egress(
                &terrain,
                egress,
                motion,
                horizontal_seconds,
                context.ok_or("bot_navmesh_runtime_missing")?,
            );
        }
        if let Some((mesh, route)) = &self.navmesh {
            return self.advance_navmesh(
                &terrain,
                mesh,
                route,
                motion,
                waypoint,
                dt,
                horizontal_seconds,
                context.ok_or("bot_navmesh_runtime_missing")?,
            );
        }
        if self.supported_recovery {
            let mut cursor = waypoint;
            for _ in 0..NAVIGATION_STEP_LIMIT {
                let Some(target) = self.route.waypoints.get(cursor) else {
                    return Ok(NavigationStep {
                        motion,
                        waypoint: cursor,
                        reached: true,
                        stopped: None,
                    });
                };
                let delta = [
                    target[0] - motion.position[0],
                    target[2] - motion.position[2],
                ];
                let distance = delta[0].hypot(delta[1]);
                if distance <= body.skin {
                    cursor += 1;
                    continue;
                }
                if horizontal_seconds <= 1e-7 {
                    return Ok(NavigationStep {
                        motion,
                        waypoint: cursor,
                        reached: false,
                        stopped: None,
                    });
                }
                let slice = horizontal_seconds
                    .min(distance / self.speed)
                    .min(NAVIGATION_SOLVER_SLICE_SECONDS);
                motion = terrain
                    .advance(
                        motion,
                        [
                            delta[0] / distance * self.speed,
                            0.,
                            delta[1] / distance * self.speed,
                        ],
                        slice,
                        body,
                    )
                    .map_err(navigation_physics_error)?;
                horizontal_seconds = (horizontal_seconds - slice).max(0.);
                if motion.blocked || !motion.grounded {
                    return Ok(NavigationStep {
                        motion,
                        waypoint: cursor,
                        reached: false,
                        stopped: Some("bot_ground_recovery_prefix_obstructed"),
                    });
                }
            }
            return Ok(NavigationStep {
                motion,
                waypoint: cursor,
                reached: false,
                stopped: None,
            });
        }
        let query =
            bot_navigation::WorldNavigation::new(world, map_name, body, self.bounds, 16.0, 16.0)
                .map_err(navigation_error)?;
        let mut follower = bot_navigation::RouteFollower::resume(self.route.clone(), waypoint)
            .map_err(navigation_error)?;
        if !motion.grounded {
            // Airborne starts settle vertically before following any waypoint.
            let mut remaining = dt;
            for _ in 0..4 {
                let slice = remaining.min(NAVIGATION_SOLVER_SLICE_SECONDS);
                if slice <= 1e-7 {
                    break;
                }
                motion = terrain
                    .advance(motion, [0.0; 3], slice, body)
                    .map_err(navigation_physics_error)?;
                remaining = (remaining - slice).max(0.0);
            }
            return Ok(NavigationStep {
                motion,
                waypoint,
                reached: false,
                stopped: None,
            });
        }
        for _ in 0..NAVIGATION_STEP_LIMIT {
            let target = follower
                .next_grounded_target(&query, motion.position)
                .map_err(navigation_error)?;
            let Some(target) = target else {
                return Ok(NavigationStep {
                    motion,
                    waypoint: follower.cursor(),
                    reached: true,
                    stopped: None,
                });
            };
            if horizontal_seconds <= 1e-7 {
                return Ok(NavigationStep {
                    motion,
                    waypoint: follower.cursor(),
                    reached: false,
                    stopped: None,
                });
            }
            let dx = target[0] - motion.position[0];
            let dz = target[2] - motion.position[2];
            let distance = dx.hypot(dz);
            if distance < 1e-7 || self.speed <= 0.0 {
                return Ok(NavigationStep {
                    motion,
                    waypoint: follower.cursor(),
                    reached: false,
                    stopped: Some("bot_navigation_waypoint_height"),
                });
            }
            let step = horizontal_seconds
                .min(distance / self.speed)
                .min(NAVIGATION_SOLVER_SLICE_SECONDS);
            motion = terrain
                .advance(
                    motion,
                    [dx / distance * self.speed, 0.0, dz / distance * self.speed],
                    step,
                    body,
                )
                .map_err(navigation_physics_error)?;
            horizontal_seconds = (horizontal_seconds - step).max(0.0);
            if motion.blocked {
                return Ok(NavigationStep {
                    motion,
                    waypoint: follower.cursor(),
                    reached: false,
                    stopped: Some("bot_navigation_blocked"),
                });
            }
            if !motion.grounded {
                return Ok(NavigationStep {
                    motion,
                    waypoint: follower.cursor(),
                    reached: false,
                    stopped: None,
                });
            }
        }
        Ok(NavigationStep {
            motion,
            waypoint: follower.cursor(),
            reached: false,
            stopped: Some("bot_navigation_step_limit"),
        })
    }
    #[allow(clippy::too_many_arguments)]
    fn advance_navmesh(
        &self,
        terrain: &bot_physics::WorldTerrain<'_>,
        mesh: &navmesh::NavMesh,
        route: &navmesh::Route,
        mut motion: bot_physics::Motion,
        mut waypoint: usize,
        dt: f64,
        mut seconds: f64,
        context: &mut NavigationContext<'_>,
    ) -> Result<NavigationStep, &'static str> {
        use navmesh::WaypointKind;
        use retail_movement::MovementIntent;
        if waypoint > route.waypoints.len()
            || route.waypoint_kinds.len() != route.waypoints.len()
            || route.waypoint_drop_ids.len() != route.waypoints.len()
        {
            return Err("bot_navmesh_invalid_cursor");
        }
        if (!motion.grounded && waypoint == 0) || context.state.on_player {
            let mut remaining = dt;
            for _ in 0..4 {
                let slice = remaining.min(NAVIGATION_SOLVER_SLICE_SECONDS);
                if slice <= 1e-7 {
                    break;
                }
                motion = context.advance(terrain, motion, [0.; 3], slice)?;
                context
                    .state
                    .movement
                    .advance(MovementIntent::Idle, slice)?;
                remaining -= slice;
            }
            return Ok(NavigationStep {
                motion,
                waypoint,
                reached: false,
                stopped: None,
            });
        }
        for iteration in 0..NAVIGATION_STEP_LIMIT {
            while waypoint < route.waypoints.len()
                && (route.waypoints[waypoint][0] - motion.position[0])
                    .hypot(route.waypoints[waypoint][2] - motion.position[2])
                    <= 1.0 / 32.0
            {
                match route.waypoint_kinds[waypoint] {
                    WaypointKind::DropStart => {
                        let id =
                            route.waypoint_drop_ids[waypoint].ok_or("bot_navmesh_drop_identity")?;
                        if motion.grounded && !context.admission.permits(id) {
                            return self.wait_drop_admission(
                                terrain,
                                mesh,
                                route,
                                motion,
                                waypoint,
                                seconds,
                                NAVIGATION_STEP_LIMIT - iteration,
                                id,
                                context,
                            );
                        }
                        let rules = context.rules.ok_or("bot_navmesh_fall_rules_required")?;
                        let mut remaining = context.hp;
                        for i in waypoint..route.waypoints.len() {
                            if route.waypoint_kinds[i] == WaypointKind::DropStart {
                                let id = route.waypoint_drop_ids[i]
                                    .ok_or("bot_navmesh_drop_identity")?;
                                let link = mesh
                                    .artifact()
                                    .drop_links
                                    .iter()
                                    .find(|l| l.id == id)
                                    .ok_or("bot_navmesh_drop_identity")?;
                                remaining = rules
                                    .predict_drop(link.max_drop_height as f32, remaining)?
                                    .remaining_health_quarter_points;
                                if remaining == 0 {
                                    return Err("bot_navmesh_drop_health_changed");
                                }
                            }
                        }
                        if !motion.grounded {
                            break;
                        }
                        if let Some(query) = context.actors {
                            let id = route.waypoint_drop_ids[waypoint]
                                .ok_or("bot_navmesh_drop_identity")?;
                            let link = mesh
                                .artifact()
                                .drop_links
                                .iter()
                                .find(|l| l.id == id)
                                .ok_or("bot_navmesh_drop_identity")?;
                            let mut budget = 1024;
                            let mut blocker = None;
                            for pair in link.trajectory.windows(2) {
                                if let Some(hit) = query.grid.sweep(
                                    query.actor,
                                    pair[0].position,
                                    pair[1].position,
                                    bot_physics::Body::default(),
                                    &mut budget,
                                )? {
                                    blocker = Some(hit.id);
                                    break;
                                }
                            }
                            if let Some(id) = blocker {
                                context.waiting_on = Some(id);
                                let (motion, complete) = context.idle_physics(
                                    terrain,
                                    motion,
                                    seconds,
                                    NAVIGATION_STEP_LIMIT - iteration,
                                )?;
                                return Ok(NavigationStep {
                                    motion,
                                    waypoint,
                                    reached: false,
                                    stopped: (!complete).then_some("bot_navigation_step_limit"),
                                });
                            }
                        }
                        #[cfg(test)]
                        private_drop_trace(
                            json!({"kind":"drop_start_motion","drop_id":route.waypoint_drop_ids[waypoint],"actor":context.actors.map(|q|q.actor),"generation":context.generation,"motion":motion,"update_dt_s":dt,"remaining_s":seconds,"waypoint":waypoint,"target":route.waypoints.get(waypoint+1)}),
                        );
                        waypoint += 1;
                        break;
                    }
                    WaypointKind::DropEnd if !motion.grounded => break,
                    _ => waypoint += 1,
                }
            }
            if waypoint == route.waypoints.len() && motion.grounded {
                if let Some(query) = context.actors
                    && let Some(hit) = query.grid.sweep(
                        query.actor,
                        route.projected_goal,
                        route.projected_goal,
                        bot_physics::Body::default(),
                        &mut 256,
                    )?
                {
                    context.waiting_on = Some(hit.id);
                    let (motion, complete) = context.idle_physics(
                        terrain,
                        motion,
                        seconds,
                        NAVIGATION_STEP_LIMIT - iteration,
                    )?;
                    return Ok(NavigationStep {
                        motion,
                        waypoint,
                        reached: false,
                        stopped: (!complete).then_some("bot_navigation_step_limit"),
                    });
                }
                let distance = (0..3)
                    .map(|i| (motion.position[i] - route.requested_goal[i]).powi(2))
                    .sum::<f64>()
                    .sqrt();
                return Ok(NavigationStep {
                    motion,
                    waypoint,
                    reached: distance <= 5.,
                    stopped: if distance <= 5. {
                        None
                    } else {
                        Some("bot_navmesh_goal_distance")
                    },
                });
            }
            if seconds <= 1e-7 {
                return Ok(NavigationStep {
                    motion,
                    waypoint,
                    reached: false,
                    stopped: None,
                });
            }
            let drop_link = if route.waypoint_kinds.get(waypoint) == Some(&WaypointKind::DropEnd) {
                let id = route.waypoint_drop_ids[waypoint].ok_or("bot_navmesh_drop_identity")?;
                Some(
                    mesh.artifact()
                        .drop_links
                        .iter()
                        .find(|l| l.id == id)
                        .ok_or("bot_navmesh_drop_identity")?,
                )
            } else {
                None
            };
            let next_drop = if matches!(context.admission, DropAdmission::Unrestricted) {
                None
            } else {
                route
                    .waypoint_kinds
                    .iter()
                    .enumerate()
                    .skip(waypoint)
                    .find_map(|(i, kind)| {
                        if *kind != WaypointKind::DropStart {
                            return None;
                        }
                        let id = route.waypoint_drop_ids[i]?;
                        let link = mesh
                            .artifact()
                            .drop_links
                            .iter()
                            .find(|link| link.id == id)?;
                        Some((
                            id,
                            protected_drop_bounds(link, context.admission.entry_buffer()),
                        ))
                    })
            };
            if let Some(link) = drop_link {
                if (link.speed - retail_movement::WALK_FORWARD_MPS).abs() > 1e-9 {
                    return Err("bot_navmesh_drop_speed_profile");
                }
                if !link.trajectory.iter().any(|sample| {
                    (0..3)
                        .map(|i| (sample.position[i] - motion.position[i]).powi(2))
                        .sum::<f64>()
                        <= 0.6 * 0.6
                }) {
                    return Err("bot_navmesh_drop_outside_validated_trajectory");
                }
            } else if !mesh.contains(motion.position, route) {
                return Err("bot_navmesh_off_corridor");
            }
            let settling_launch = route.waypoint_kinds.get(waypoint)
                == Some(&WaypointKind::DropStart)
                && !motion.grounded
                && route.waypoints.get(waypoint).is_some_and(|p| {
                    (p[0] - motion.position[0]).hypot(p[2] - motion.position[2]) <= 1.0 / 32.0
                });
            let target = if settling_launch {
                motion.position
            } else {
                route
                    .waypoints
                    .get(waypoint)
                    .copied()
                    .unwrap_or(motion.position)
            };
            let dx = target[0] - motion.position[0];
            let dz = target[2] - motion.position[2];
            let distance = dx.hypot(dz);
            let intent = if distance <= 1e-7 {
                MovementIntent::Idle
            } else if context.sprint && drop_link.is_none() {
                MovementIntent::Sprint
            } else {
                MovementIntent::Walk
            };
            let mut slice = seconds.min(NAVIGATION_SOLVER_SLICE_SECONDS);
            let mut candidate = context.state.movement;
            let budget = candidate.advance_direction(
                intent,
                slice,
                retail_movement::MovementDirection::Forward,
            )?;
            if budget.sprint_seconds > 1e-9 && budget.sprint_seconds < slice {
                slice = budget.sprint_seconds;
            }
            let cap = if budget.sprint_seconds > 1e-9 {
                retail_movement::SPRINT_FORWARD_MPS
            } else {
                retail_movement::WALK_FORWARD_MPS
            };
            if distance > 1e-7 {
                slice = slice.min(distance / cap);
            }
            candidate = context.state.movement;
            let budget = candidate.advance_direction(
                intent,
                slice,
                retail_movement::MovementDirection::Forward,
            )?;
            let speed = budget.distance_cap_m / slice;
            let mut avoided_actor = None;
            let mut velocity = if distance > 1e-7 {
                [dx / distance * speed, 0., dz / distance * speed]
            } else {
                [0.; 3]
            };
            // Physics and avoidance update fall/stamina/receipt state. Stage
            // the whole slice so a rejected pose cannot leak those updates.
            let before_context = drop_link.is_none().then(|| context.clone());
            let goal_occupied = if let Some(query) = context.actors {
                query
                    .grid
                    .sweep(
                        query.actor,
                        route.projected_goal,
                        route.projected_goal,
                        bot_physics::Body::default(),
                        &mut 256,
                    )?
                    .is_some()
            } else {
                false
            };
            if drop_link.is_none()
                && motion.grounded
                && distance > 1.0
                && !(goal_occupied
                    && (route.projected_goal[0] - motion.position[0])
                        .hypot(route.projected_goal[2] - motion.position[2])
                        < 2.0)
                && let Some(query) = context.actors
            {
                let horizon = slice.max(0.25);
                let end = std::array::from_fn(|i| motion.position[i] + velocity[i] * horizon);
                if let Some(blocker) = query.grid.sweep(
                    query.actor,
                    motion.position,
                    end,
                    bot_physics::Body::default(),
                    &mut 256,
                )? {
                    let side = if context.state.avoid_left { -1. } else { 1. };
                    for side in [side, -side] {
                        let v = [
                            velocity[0] * 0.5 - velocity[2] * 0.8660254037844386 * side,
                            0.,
                            velocity[2] * 0.5 + velocity[0] * 0.8660254037844386 * side,
                        ];
                        let end = std::array::from_fn(|i| motion.position[i] + v[i] * horizon);
                        if mesh.contains(end, route)
                            && query
                                .grid
                                .sweep(
                                    query.actor,
                                    motion.position,
                                    end,
                                    bot_physics::Body::default(),
                                    &mut 256,
                                )?
                                .is_none()
                        {
                            velocity = v;
                            context.avoiding = true;
                            avoided_actor = Some(blocker.id);
                            break;
                        }
                    }
                }
            }
            if drop_link.is_none()
                && let Some((id, bounds)) = next_drop
                && !context.admission.permits(id)
                && bounds_intersect(
                    bounds,
                    crate::actor_collision::Bounds::body(
                        motion.position,
                        bot_physics::Body::default(),
                    )
                    .union(crate::actor_collision::Bounds::body(
                        std::array::from_fn(|i| motion.position[i] + velocity[i] * slice),
                        bot_physics::Body::default(),
                    )),
                )
            {
                *context = before_context.ok_or("bot_drop_admission_missing_stage")?;
                return self.wait_drop_admission(
                    terrain,
                    mesh,
                    route,
                    motion,
                    waypoint,
                    seconds,
                    NAVIGATION_STEP_LIMIT - iteration,
                    id,
                    context,
                );
            }
            let before = motion;
            motion = context.advance(terrain, motion, velocity, slice)?;
            #[cfg(test)]
            if let Some(link) = drop_link {
                private_drop_trace(
                    json!({"kind":"drop_slice_motion","drop_id":link.id,"actor":context.actors.map(|q|q.actor),"generation":context.generation,"before":before,"after":motion,"velocity":velocity,"slice_s":slice,"update_dt_s":dt,"remaining_s":seconds,"waypoint":waypoint,"actor_contact":context.waiting_on}),
                );
            }
            if drop_link.is_none()
                && before.grounded
                && motion.grounded
                && let Some((id, bounds)) = next_drop
                && !context.admission.permits(id)
                && bounds_intersect(
                    bounds,
                    crate::actor_collision::Bounds::body(
                        before.position,
                        bot_physics::Body::default(),
                    )
                    .union(crate::actor_collision::Bounds::body(
                        motion.position,
                        bot_physics::Body::default(),
                    )),
                )
            {
                *context = before_context.ok_or("bot_drop_admission_missing_stage")?;
                return self.wait_drop_admission(
                    terrain,
                    mesh,
                    route,
                    before,
                    waypoint,
                    seconds,
                    NAVIGATION_STEP_LIMIT - iteration,
                    id,
                    context,
                );
            }
            if let Some(before_context) = before_context
                && before.grounded
                && motion.grounded
                && !mesh.contains(motion.position, route)
            {
                *context = before_context;
                return Ok(NavigationStep {
                    motion: before,
                    waypoint,
                    reached: false,
                    stopped: Some("bot_navmesh_off_corridor"),
                });
            }
            if (motion.position[0] - before.position[0])
                .hypot(motion.position[2] - before.position[2])
                <= 0.001
                && terrain.actor_contact().is_some()
            {
                candidate = context.state.movement;
                candidate.advance(MovementIntent::Idle, slice)?;
            }
            context.state.movement = candidate;
            seconds = (seconds - slice).max(0.);
            if context.hp == 0 {
                return Ok(NavigationStep {
                    motion,
                    waypoint,
                    reached: false,
                    stopped: Some("bot_navmesh_fall_lethal"),
                });
            }
            if motion.blocked
                && let Some(id) = terrain.actor_contact().or(avoided_actor)
            {
                let remaining_distance =
                    (target[0] - motion.position[0]).hypot(target[2] - motion.position[2]);
                if !motion.grounded || distance - remaining_distance <= 0.001 {
                    context.waiting_on = Some(id);
                    if avoided_actor.is_some() {
                        context.state.avoid_left = !context.state.avoid_left;
                    }
                    let (motion, complete) = context.idle_physics(
                        terrain,
                        motion,
                        seconds,
                        NAVIGATION_STEP_LIMIT - iteration - 1,
                    )?;
                    return Ok(NavigationStep {
                        motion,
                        waypoint,
                        reached: false,
                        stopped: (!complete).then_some("bot_navigation_step_limit"),
                    });
                }
            }
            if motion.blocked {
                let remaining_distance =
                    (target[0] - motion.position[0]).hypot(target[2] - motion.position[2]);
                let certified = if let Some(link) = drop_link {
                    link.trajectory.iter().any(|sample| {
                        (0..3)
                            .map(|i| (sample.position[i] - motion.position[i]).powi(2))
                            .sum::<f64>()
                            <= 0.6 * 0.6
                    })
                } else {
                    mesh.contains(motion.position, route)
                };
                // A recovered contact is diagnostic; zero/backward progress or
                // departure from certified space remains a terminal stop.
                if !motion.grounded || distance - remaining_distance <= 0.001 || !certified {
                    return Ok(NavigationStep {
                        motion,
                        waypoint,
                        reached: false,
                        stopped: Some("bot_navigation_blocked"),
                    });
                }
                context.recovered_contacts += 1;
            }
        }
        Ok(NavigationStep {
            motion,
            waypoint,
            reached: false,
            stopped: if seconds > 1e-7 {
                Some("bot_navigation_step_limit")
            } else {
                None
            },
        })
    }
}

fn point(p: replication::CharacterPosition) -> bot_physics::Point {
    [f64::from(p.x), f64::from(p.y), f64::from(p.z)]
}
fn character_position(p: bot_physics::Point) -> replication::CharacterPosition {
    replication::CharacterPosition {
        x: p[0] as f32,
        y: p[1] as f32,
        z: p[2] as f32,
    }
}
