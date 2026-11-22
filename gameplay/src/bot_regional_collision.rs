//! Current represented collision guard for an explicitly bound Acquisition region.
use super::*;
use crate::actor_collision::Bounds;

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegionalCollision {
    pub(crate) world_sha256: String,
    pub(crate) spawn_sha256: String,
    pub(crate) ground_sha256: String,
    pub(crate) dynamic_catalog_sha256: String,
    pub(crate) structural_clv_sha256: String,
    min: [f64; 3],
    max: [f64; 3],
}

impl RegionalCollision {
    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        let hashes = [
            &self.world_sha256,
            &self.spawn_sha256,
            &self.ground_sha256,
            &self.dynamic_catalog_sha256,
            &self.structural_clv_sha256,
        ];
        if !hashes.into_iter().all(|hash| {
            hash.len() == 64
                && hash
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        }) || (0..3).any(|i| {
            !self.min[i].is_finite()
                || !self.max[i].is_finite()
                || !(0.0 < self.max[i] - self.min[i] && self.max[i] - self.min[i] <= 128.)
        }) {
            return Err("bot_regional_invalid_scope");
        }
        Ok(())
    }

    pub(crate) fn static_world<'a, W: Write>(
        &self,
        engine: &'a Engine<W>,
    ) -> Result<&'a collision::CollisionWorld, &'static str> {
        let world = engine
            .collision_world
            .as_ref()
            .ok_or("bot_regional_world_unavailable")?;
        let dynamic = engine
            .dynamic_body_resources
            .as_ref()
            .ok_or("bot_regional_dynamic_unavailable")?;
        if engine.match_profile().game_type != 1
            || engine.wire_profile.application_id() != 20231
            || world.metadata().map_name != engine.map_name()
            || world.source_sha256() != self.world_sha256
            || engine
                .spawn_catalog
                .as_ref()
                .and_then(|spawn| spawn.source_sha256())
                != Some(self.spawn_sha256.as_str())
            || engine
                .ground_vehicles
                .as_ref()
                .map(ground_vehicles::Catalog::raw_sha256)
                != Some(self.ground_sha256.as_str())
            || dynamic.catalog.fingerprint() != self.dynamic_catalog_sha256
            || !world.metadata().physics_regions.iter().any(|region| {
                region.ground_verified
                    && region.obstacles_verified
                    && !region.provenance.is_empty()
                    && (0..3).all(|i| region.min[i] <= self.min[i] && region.max[i] >= self.max[i])
            })
        {
            return Err("bot_regional_source_mismatch");
        }
        let ground = engine
            .ground_vehicles
            .as_ref()
            .ok_or("bot_regional_ground_unavailable")?;
        let acquisition = engine
            .acquisition_gameplay
            .as_ref()
            .ok_or("bot_regional_structural_unavailable")?;
        for (_, objective) in acquisition.state.objectives() {
            if let Some(fsp) = objective.fsp {
                let creation = ground
                    .vehicle(fsp.vehicle.index)
                    .ok_or("bot_regional_spawn_chassis_unavailable")?;
                if fsp.source.pose.position != creation.position
                    || fsp.source.pose.rotation != creation.rotation
                {
                    return Err("bot_regional_respawn_pose_mismatch");
                }
            }
        }
        Ok(world)
    }

    pub(crate) fn contains_body(&self, position: [f64; 3], body: bot_physics::Body) -> bool {
        let bounds = Bounds::body(position, body);
        (0..3).all(|i| bounds.min[i] >= self.min[i] && bounds.max[i] <= self.max[i])
    }

    pub(crate) fn validate_navmesh(&self, mesh: &navmesh::Artifact) -> Result<(), &'static str> {
        if mesh.world_sha256 != self.world_sha256
            || (0..3).any(|i| mesh.bounds.min[i] < self.min[i] || mesh.bounds.max[i] > self.max[i])
            || !mesh.drop_links.is_empty()
        {
            return Err("bot_regional_navmesh_source_mismatch");
        }
        Ok(())
    }

    /// Native Acquisition ejection begins only after the spawned chassis
    /// penetrates a Character root. No admitted bot root can enter those
    /// source-fixed chassis domains, even while the chassis is unregistered.
    pub(crate) fn validate_spawn_clearance<W: Write>(
        &self,
        engine: &Engine<W>,
    ) -> Result<(), &'static str> {
        use acquisition_volume_damage::{Target, TargetKey, TargetKind};
        let resources = engine
            .dynamic_body_resources
            .as_ref()
            .ok_or("bot_regional_dynamic_unavailable")?;
        let mut budget = 262_144;
        let mut reach: f64 = 0.;
        for dead in [false, true] {
            let character = resources.catalog.instantiate(
                &resources.bindings().character_template,
                Target {
                    key: TargetKey {
                        id: 1,
                        generation: 1,
                    },
                    kind: TargetKind::Character,
                    team: 0,
                },
                dynamic_collision::Pose::at([0.; 3]),
                dead,
            )?;
            for root in character.bodies {
                let bounds = root.bot_regional_bounds(&mut budget)?;
                let radius = root.shape.radius();
                // Bound a rotated root by its farthest possible core corner.
                // The shape's spherical margin is added once after rotation.
                let norm: f64 = (0..3)
                    .map(|i| {
                        (bounds.min[i].abs().max(bounds.max[i].abs()) - radius)
                            .max(0.)
                            .powi(2)
                    })
                    .sum();
                reach = reach.max(norm.sqrt() + radius);
            }
        }
        // Owned collider snapshots use BotRuntime::physical_position, so
        // replicated wire precision does not affect pair selection. Include
        // roundoff for the f32 stored-position fallback on dead/initial lives.
        let character_domain = Bounds {
            min: self
                .min
                .map(|v| v - reach - (v.abs() + reach) * f64::from(f32::EPSILON)),
            max: self
                .max
                .map(|v| v + reach + (v.abs() + reach) * f64::from(f32::EPSILON)),
        };
        let ground = engine
            .ground_vehicles
            .as_ref()
            .ok_or("bot_regional_ground_unavailable")?;
        for source in ground.creations() {
            let placed = resources.catalog.instantiate(
                resources.vehicle_template(source)?,
                Target {
                    key: TargetKey {
                        id: u64::from(source.object_id()),
                        generation: 1,
                    },
                    kind: TargetKind::GroundVehicle,
                    team: source.team,
                },
                dynamic_collision_runtime::vehicle_pose(source),
                false,
            )?;
            let chassis = placed
                .bodies
                .iter()
                .find(|body| body.root == "chassis")
                .ok_or("bot_regional_spawn_chassis_unavailable")?;
            let bounds = chassis.bot_regional_bounds(&mut budget)?;
            if (0..3).all(|i| {
                bounds.min[i] <= character_domain.max[i] && character_domain.min[i] <= bounds.max[i]
            }) {
                return Err("bot_regional_spawn_clearance_overlaps_region");
            }
        }
        Ok(())
    }

    pub(crate) fn guarded_world(
        &self,
        world: &collision::CollisionWorld,
        obstacles: &[Bounds],
    ) -> collision::CollisionWorld {
        world.with_bot_regional_guard(bot_physics::RegionalGuard::new(
            Bounds {
                min: self.min,
                max: self.max,
            },
            obstacles,
        ))
    }

    pub(crate) fn capture<W: Write>(
        &self,
        engine: &Engine<W>,
        now: Duration,
        moving_bot: Option<u16>,
    ) -> Result<collision::CollisionWorld, &'static str> {
        let world = self.static_world(engine)?;
        // Capture stores all roots; the filter is used only by the other snapshot consumers.
        let snapshot = engine.dynamic_collision_snapshot(u32::MAX)?;
        let mut budget = 262_144;
        let mut obstacles = engine
            .acquisition_gameplay
            .as_ref()
            .ok_or("bot_regional_structural_unavailable")?
            .bot_regional_structural_bounds(&self.structural_clv_sha256, &mut budget)?;
        for body in snapshot.bodies() {
            use acquisition_volume_damage::TargetKind;
            match body.target.kind {
                TargetKind::GroundVehicle => {}
                TargetKind::Character => {
                    let id = body.target.key.id as u16;
                    if let Some(bot) = engine.bot_registry.bots.get(&id) {
                        if bot.health > 0 || moving_bot == Some(id) {
                            continue; // ActorGrid
                        }
                        // The server-owned bot has a known last position and the
                        // captured template's authored dead root.
                    } else {
                        let actor = engine
                            .active
                            .iter()
                            .chain(engine.peers.values())
                            .find(|actor| actor.reservation.player == id)
                            .ok_or("bot_regional_character_unqualified")?;
                        if actor.is_mounted() {
                            let seat = actor
                                .ground_seat
                                .ok_or("bot_regional_carrier_unqualified")?;
                            if !engine.ground_vehicle_is_operational(seat.vehicle_index) {
                                return Err("bot_regional_carrier_unqualified");
                            }
                            continue;
                        }
                        if !matches!(actor.life, CharacterLife::AwaitingFirstSpawn)
                            && (!actor.life.is_alive()
                                || actor.tracking.sample.is_none()
                                || actor
                                    .owner_fields
                                    .state
                                    .is_none_or(|state| state.stance != 0))
                        {
                            return Err("bot_regional_character_unqualified");
                        }
                        if crate::bot_registry::human_in_actor_grid(actor, now) {
                            continue;
                        }
                    }
                }
                _ => return Err("bot_regional_dynamic_category_unavailable"),
            }
            obstacles.push(body.bot_regional_bounds(&mut budget)?);
        }
        if !engine.dynamic_collision_snapshot_is_current(&snapshot) {
            return Err("bot_regional_dynamic_snapshot_stale");
        }
        Ok(self.guarded_world(world, &obstacles))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acquisition_ground_runtime::{Pose, QualifiedRespawn};
    use crate::replication::CharacterPosition;

    fn fixture() -> (Engine<Vec<u8>>, RegionalCollision) {
        let mut engine = crate::acquisition_structural::connected_tests::fixture();
        // This fixture installs synthetic tracking without a spawn event.
        // Provide the standing posture that normal spawn authority records.
        engine.active.as_mut().unwrap().owner_fields.state = Some(replication::CharacterState {
            state: 1,
            stance: 0,
            user_data: [0; 3],
        });

        let floor = crate::bot_registry::identity_tests::engine("Regional")
            .collision_world
            .unwrap();
        engine.collision_world = Some(floor);
        let scope = RegionalCollision {
            world_sha256: engine
                .collision_world
                .as_ref()
                .unwrap()
                .source_sha256()
                .into(),
            spawn_sha256: engine
                .spawn_catalog
                .as_ref()
                .unwrap()
                .source_sha256()
                .unwrap()
                .into(),
            ground_sha256: engine.ground_vehicles.as_ref().unwrap().raw_sha256().into(),
            dynamic_catalog_sha256: engine
                .dynamic_body_resources
                .as_ref()
                .unwrap()
                .catalog
                .fingerprint()
                .into(),
            structural_clv_sha256: "a".repeat(64),
            min: [-9., -4., -9.],
            max: [9., 7., 9.],
        };
        scope.validate().unwrap();
        (engine, scope)
    }

    fn set_ground(
        engine: &mut Engine<Vec<u8>>,
        x: f32,
        z: f32,
    ) -> (crate::acquisition_ground_runtime::Key, Pose) {
        let creation = engine
            .ground_vehicles
            .as_ref()
            .unwrap()
            .vehicle(4)
            .unwrap()
            .clone();
        let ground = engine.ground_runtime.as_mut().unwrap();
        let key = ground.get(4).unwrap().key;
        let pose = Pose {
            position: CharacterPosition { x, y: 1., z },
            rotation: [0., 0., 0., 1.],
        };
        ground
            .respawn_existing(
                key,
                QualifiedRespawn {
                    spawn_point_index: creation.spawn_point_index,
                    spawn_point_token: creation.spawn_point_token,
                    active: true,
                    pose,
                },
                true,
            )
            .unwrap();
        ground.update(key, 0., 0).unwrap();
        (key, pose)
    }

    #[test]
    fn regional_guard_tracks_current_ground_pose_and_rejects_unknown_sources() {
        let (mut engine, scope) = fixture();
        let actor = engine.active.as_mut().unwrap();
        let original = actor.owner_fields.state;
        actor.ready_for_updates = true;
        actor.entered_game = true;
        actor.initial_objects = Some(Vec::new());
        actor.spawn_delta = Some(Vec::new());
        assert!(crate::bot_registry::human_in_actor_grid(
            actor,
            Duration::ZERO
        ));
        for stance in [Some(2), None] {
            engine.active.as_mut().unwrap().owner_fields.state =
                stance.map(|stance| replication::CharacterState {
                    state: 1,
                    stance,
                    user_data: [0; 3],
                });
            assert_eq!(
                scope.capture(&engine, Duration::ZERO, None).unwrap_err(),
                "bot_regional_character_unqualified"
            );
        }
        engine.active.as_mut().unwrap().owner_fields.state = original;
        let (key, pose) = set_ground(&mut engine, 4., 2.);

        let body = bot_physics::Body::default();
        let now = Duration::ZERO;
        let world = scope.capture(&engine, now, None).unwrap();
        let terrain = bot_physics::WorldTerrain::new(Some(&world), engine.map_name()).unwrap();
        let start = [2., 0.005, 2.];
        let motion = terrain.start_motion(start, body).unwrap();
        assert!(matches!(
            terrain.advance(motion, [4., 0., 0.], 0.5, body),
            Err(bot_physics::PhysicsError::RegionalBlocked)
        ));
        assert!(matches!(
            terrain.start_motion([4., 0.005, 2.], body),
            Err(bot_physics::PhysicsError::RegionalBlocked)
        ));

        let stale = engine.dynamic_collision_snapshot(u32::MAX).unwrap();
        let ground = engine.ground_runtime.as_mut().unwrap();
        ground
            .force_pose(
                key,
                Pose {
                    position: CharacterPosition {
                        z: 8.,
                        ..pose.position
                    },
                    ..pose
                },
            )
            .unwrap();
        engine
            .record(json!({"op":"synthetic_accepted_ground_update"}), 1)
            .unwrap();
        assert!(!engine.dynamic_collision_snapshot_is_current(&stale));
        let world = scope.capture(&engine, now, None).unwrap();
        let terrain = bot_physics::WorldTerrain::new(Some(&world), engine.map_name()).unwrap();
        let resumed = terrain.advance(motion, [4., 0., 0.], 0.5, body).unwrap();
        assert!(resumed.position[0] > 3.5);

        let mut wrong_scope = scope.clone();
        wrong_scope.structural_clv_sha256 = "b".repeat(64);
        assert_eq!(
            wrong_scope.capture(&engine, now, None).unwrap_err(),
            "bot_regional_structural_source_mismatch"
        );
        engine.dynamic_body_resources = None;
        assert_eq!(
            scope.capture(&engine, now, None).unwrap_err(),
            "bot_regional_dynamic_unavailable"
        );
    }
    #[test]
    fn regional_guard_registry_holds_and_resumes_the_same_life() {
        let (mut engine, scope) = fixture();
        engine.bot_registry =
            crate::bot_registry::identity_tests::populated_engine("Regional Tick").bot_registry;
        engine
            .bot_registry
            .config
            .as_mut()
            .unwrap()
            .regional_collision = Some(scope);
        let active = engine.active.as_mut().unwrap();
        active.ready_for_updates = true;
        active.entered_game = true;
        active.initial_objects = Some(Vec::new());
        let id = engine
            .bot_registry
            .bots
            .iter()
            .find(|(_, bot)| bot.target.squad.team() != active.team())
            .map(|(id, _)| *id)
            .unwrap();
        let terrain =
            bot_physics::WorldTerrain::new(engine.collision_world.as_ref(), engine.map_name())
                .unwrap();
        let body = bot_physics::Body::default();
        let bot = engine.bot_registry.bots.get_mut(&id).unwrap();
        let runtime = bot.target.runtime.as_mut().unwrap();
        let mut start = runtime.physical_position();
        // This synthetic floor fixture spawns at exact triangle tangency.
        // Begin the walking trial with the normal physical skin clearance.
        start[1] += body.skin;
        runtime.physics_motion = Some(terrain.start_motion(start, body).unwrap());
        runtime.position.y = start[1] as f32;
        let before = *runtime;
        let mut destination = before.position;
        destination.x += 2.;
        bot.target.begin_regional_test_walk(destination);
        let (key, pose) = set_ground(&mut engine, before.position.x + 1.65, before.position.z);
        engine.tick_bots(Duration::from_millis(250), 1).unwrap();
        let bot = &engine.bot_registry.bots[&id];
        let held = bot.target.runtime.unwrap();
        assert_eq!(held.physical_position(), before.physical_position());
        assert_eq!(held.last_update, Duration::from_millis(250));
        assert_eq!(held.navigation_waypoint, before.navigation_waypoint);
        assert_eq!(held.generation, before.generation);
        assert_eq!(bot.health, 400);
        assert!(bot.target.pending_move.is_some());
        assert!(!String::from_utf8_lossy(&engine.writer).contains("synthetic_regional_walk"));

        engine
            .ground_runtime
            .as_mut()
            .unwrap()
            .force_pose(
                key,
                Pose {
                    position: CharacterPosition {
                        z: 8.,
                        ..pose.position
                    },
                    ..pose
                },
            )
            .unwrap();
        engine
            .record(json!({"op":"synthetic_accepted_ground_update"}), 2)
            .unwrap();
        engine.tick_bots(Duration::from_millis(500), 2).unwrap();
        let resumed = engine.bot_registry.bots[&id].target.runtime.unwrap();
        let distance = resumed.physical_position()[0] - before.physical_position()[0];
        assert!(
            distance > 0.45 && distance < 0.55,
            "applied ordinary interval, never caught up: {distance}"
        );
        assert_eq!(resumed.generation, before.generation);
        assert!(String::from_utf8_lossy(&engine.writer).contains("synthetic_regional_walk"));
    }
    #[test]
    fn regional_guard_excludes_authored_spawn_blocker_domains() {
        let (mut engine, mut scope) = fixture();
        assert!(scope.validate_spawn_clearance(&engine).is_ok());
        engine.bot_registry =
            crate::bot_registry::identity_tests::engine("Regional Respawn").bot_registry;
        engine
            .bot_registry
            .config
            .as_mut()
            .unwrap()
            .regional_collision = Some(scope.clone());
        let source = engine.ground_vehicles.as_ref().unwrap().vehicle(4).unwrap();
        let position = source.position;
        // The body-foot box stops short of the chassis; the bound Character
        // root extends beyond the box and would admit a blocker pair.
        scope.min = [
            f64::from(position.x) - 4.,
            f64::from(position.y),
            f64::from(position.z),
        ];
        scope.max = [
            f64::from(position.x) - 1.2,
            f64::from(position.y) + 2.,
            f64::from(position.z) + 2.,
        ];
        assert_eq!(
            scope.validate_spawn_clearance(&engine),
            Err("bot_regional_spawn_clearance_overlaps_region")
        );
        let before = engine.ground_runtime.clone();
        let sequence = engine.sequence;
        let request = crate::acquisition_spawn_effects::RespawnRequest {
            source: before.as_ref().unwrap().get(4).unwrap().key,
            point: QualifiedRespawn {
                spawn_point_index: source.spawn_point_index,
                spawn_point_token: source.spawn_point_token,
                active: true,
                pose: Pose {
                    position: CharacterPosition {
                        x: position.x + 1.,
                        ..position
                    },
                    rotation: source.rotation,
                },
            },
            initial: true,
            physics: Default::default(),
            filter_rows: [u32::MAX; 32],
            now: Duration::ZERO,
        };
        assert_eq!(
            engine
                .prepare_existing_acquisition_respawn(request, &Default::default())
                .err(),
            Some("bot_regional_respawn_pose_mismatch")
        );
        assert_eq!(engine.ground_runtime, before);
        assert_eq!(engine.sequence, sequence);
    }
    #[test]
    fn regional_guard_uses_the_current_distinct_hulk_geometry() {
        use acquisition_ground_runtime::{Body as GroundBody, Capabilities, Destruction, Runtime};
        let (mut engine, mut scope) = fixture();
        let mut catalog = crate::synthetic_dynamic_collision::fixture();
        catalog["templates"].as_array_mut().unwrap().push(json!({
            "id":"invented-hulk","kind":"ground_vehicle","roots":[{
                "id":"wreck","layer":22,"translation":[0.,0.,-3.],"rotation":"entity",
                "shape":{"kind":"box","half_extents":[0.4,0.8,0.7],"radius":0.1}
            }]
        }));
        let bindings = engine
            .dynamic_body_resources
            .as_ref()
            .unwrap()
            .bindings()
            .clone();
        let actor = engine.active.take();
        engine = engine
            .with_dynamic_collision_bodies(
                dynamic_collision::Catalog::from_json(&catalog.to_string()).unwrap(),
                bindings,
            )
            .unwrap();
        engine.active = actor;
        scope.dynamic_catalog_sha256 = engine
            .dynamic_body_resources
            .as_ref()
            .unwrap()
            .catalog
            .fingerprint()
            .into();
        engine.ground_runtime = Some(Runtime::regional_hulk_fixture(
            engine.ground_vehicles.as_ref().unwrap(),
            4,
        ));
        let (key, mut pose) = set_ground(&mut engine, 4., 5.);
        let body = bot_physics::Body::default();
        let now = Duration::ZERO;
        let world = scope.capture(&engine, now, None).unwrap();
        let terrain = bot_physics::WorldTerrain::new(Some(&world), engine.map_name()).unwrap();
        let motion = terrain.start_motion([2., 0.005, 2.], body).unwrap();
        assert!(
            terrain
                .advance(motion, [4., 0., 0.], 0.5, body)
                .unwrap()
                .position[0]
                > 3.5
        );

        let stale = engine.dynamic_collision_snapshot(u32::MAX).unwrap();
        let mut ground = engine.ground_runtime.take().unwrap();
        ground
            .configure(
                key,
                Capabilities {
                    destruction: Some(Destruction {
                        damage: 0.,
                        full_radius: 0.,
                        half_radius: 0.,
                    }),
                    hulk_template: Some("invented-hulk".into()),
                    delivered_inert_seconds: None,
                },
            )
            .unwrap();
        ground.apply_lethal(key, 1, false).unwrap();
        ground.update(key, 0., 0).unwrap();
        assert_eq!(
            ground.get(4).unwrap().body,
            GroundBody::Hulk {
                template: "invented-hulk".into()
            }
        );
        engine.ground_runtime = Some(ground);
        engine
            .record(json!({"op":"synthetic_hulk_lifecycle"}), 1)
            .unwrap();
        assert!(!engine.dynamic_collision_snapshot_is_current(&stale));
        let world = scope.capture(&engine, now, None).unwrap();
        let terrain = bot_physics::WorldTerrain::new(Some(&world), engine.map_name()).unwrap();
        assert!(matches!(
            terrain.advance(motion, [4., 0., 0.], 0.5, body),
            Err(bot_physics::PhysicsError::RegionalBlocked)
        ));

        pose.position.z = 8.;
        engine
            .ground_runtime
            .as_mut()
            .unwrap()
            .force_pose(key, pose)
            .unwrap();
        let world = scope.capture(&engine, now, None).unwrap();
        let terrain = bot_physics::WorldTerrain::new(Some(&world), engine.map_name()).unwrap();
        assert!(
            terrain
                .advance(motion, [4., 0., 0.], 0.5, body)
                .unwrap()
                .position[0]
                > 3.5
        );

        let bindings = engine
            .dynamic_body_resources
            .as_ref()
            .unwrap()
            .bindings()
            .clone();
        let actor = engine.active.take();
        catalog["templates"].as_array_mut().unwrap().pop();
        engine = engine
            .with_dynamic_collision_bodies(
                dynamic_collision::Catalog::from_json(&catalog.to_string()).unwrap(),
                bindings,
            )
            .unwrap();
        engine.active = actor;
        scope.dynamic_catalog_sha256 = engine
            .dynamic_body_resources
            .as_ref()
            .unwrap()
            .catalog
            .fingerprint()
            .into();
        assert_eq!(
            scope.capture(&engine, now, None).unwrap_err(),
            "missing_dynamic_body_template"
        );
    }
}
