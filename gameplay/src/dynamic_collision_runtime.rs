//! Read-only collection of actual Engine objects, separate from callback effects.
use crate::{
    acquisition_atomic::WorldKey,
    acquisition_volume_damage::{Target, TargetKey, TargetKind},
    dynamic_collision::{Catalog, Pose, Snapshot},
    *,
};
use serde::Deserialize;
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VehicleBinding {
    pub vehicle_type_token: u32,
    pub faction: u8,
    pub template: String,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeBindings {
    pub character_template: String,
    pub vehicles: Vec<VehicleBinding>,
}
impl RuntimeBindings {
    pub fn validate_for(
        &self,
        catalog: &Catalog,
        vehicles: Option<&ground_vehicles::Catalog>,
    ) -> Result<(), &'static str> {
        if catalog.template_kind(&self.character_template) != Some(TargetKind::Character)
            || self.vehicles.len() > 256
        {
            return Err("invalid_dynamic_body_bindings");
        }
        let mut seen = std::collections::BTreeSet::new();
        for b in &self.vehicles {
            if b.faction > 2
                || !seen.insert((b.vehicle_type_token, b.faction))
                || catalog.template_kind(&b.template) != Some(TargetKind::GroundVehicle)
            {
                return Err("invalid_dynamic_body_bindings");
            }
        }
        if let Some(vehicles) = vehicles {
            for v in vehicles.creations() {
                if !self
                    .vehicles
                    .iter()
                    .any(|b| b.vehicle_type_token == v.vehicle_type_token && b.faction == v.faction)
                {
                    return Err("missing_dynamic_ground_template");
                }
            }
        }
        Ok(())
    }
}
pub(crate) struct Resources {
    origin: std::sync::Arc<()>,
    pub(crate) catalog: Catalog,
    bindings: RuntimeBindings,
}
impl Resources {
    pub(crate) fn validate_profiles(
        &self,
        registry: &match_profiles::Registry,
    ) -> Result<(), &'static str> {
        registry.validate_dynamic_bodies(&self.catalog, &self.bindings)
    }
    pub(crate) fn vehicle_template(
        &self,
        v: &ground_vehicles::Creation,
    ) -> Result<&str, &'static str> {
        self.bindings
            .vehicles
            .iter()
            .find(|b| b.vehicle_type_token == v.vehicle_type_token && b.faction == v.faction)
            .map(|b| b.template.as_str())
            .ok_or("missing_dynamic_ground_template")
    }
    pub(crate) fn validate_vehicles(
        &self,
        vehicles: &ground_vehicles::Catalog,
    ) -> Result<(), &'static str> {
        for v in vehicles.creations() {
            self.vehicle_template(v)?;
        }
        Ok(())
    }
}
fn point(p: replication::CharacterPosition) -> [f64; 3] {
    [p.x.into(), p.y.into(), p.z.into()]
}
fn vehicle_pose(v: &ground_vehicles::Creation) -> Pose {
    let transform = vehicle_exit::BunkerTransform {
        position: v.position,
        rotation_xyzw: v.rotation,
    };
    let columns = [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]].map(|v| transform.rotate(v));
    Pose {
        translation: point(v.position),
        rotation: std::array::from_fn(|i| std::array::from_fn(|j| columns[j][i].into())),
    }
}
impl<W: Write> Engine<W> {
    /// Query the actual current owned world for one allocated spawning chassis.
    /// The caller supplies the current filter table and explicit sweep tuning.
    /// This prepares collision results only; it does not publish corrections or
    /// acknowledge the complete native respawn command before its consumers run.
    pub fn acquisition_spawn_collisions(
        &self,
        source: acquisition_ground_runtime::Key,
        root: &str,
        rows: [u32; 32],
        settings: acquisition_world::Settings,
    ) -> Result<acquisition_world::SpawnResolution, &'static str> {
        let runtime = self
            .ground_runtime
            .as_ref()
            .and_then(|r| r.get(source.index))
            .filter(|v| v.key == source)
            .ok_or("stale_ground_vehicle_key")?;
        let creation = self
            .current_ground_vehicle(source.index)
            .ok_or("ground_vehicle_runtime_required")?;
        let statics = self
            .collision_world
            .as_ref()
            .ok_or("world_cast_static_geometry_unavailable")?;
        if self
            .spawn_catalog
            .as_ref()
            .is_none_or(|s| s.map_name() != statics.metadata().map_name)
        {
            return Err("world_cast_map_mismatch");
        }
        let snapshot = self.dynamic_collision_snapshot(rows[6])?;
        let filters = acquisition_world::Filters::owned_entities(&snapshot, statics, rows)?;
        let mut world = acquisition_world::World::new(&snapshot, statics, filters, settings)?;
        let target = Target {
            key: TargetKey {
                id: u64::from(source.object_id()),
                generation: runtime.key.generation,
            },
            kind: TargetKind::GroundVehicle,
            team: creation.team,
        };
        let key = acquisition_world::RootKey {
            entity: target.key.id,
            generation: target.key.generation,
            root: root.to_owned(),
        };
        if !world.has_root(&key) {
            let resources = self
                .dynamic_body_resources
                .as_ref()
                .ok_or("dynamic_body_resources_unavailable")?;
            world.add_allocated_spawn(
                &resources.catalog,
                resources.vehicle_template(&creation)?,
                target,
                vehicle_pose(&creation),
                root,
            )?;
        }
        let operational = self
            .ground_runtime
            .as_ref()
            .ok_or("ground_vehicle_runtime_required")?
            .vehicles()
            .filter(|v| v.state == 2)
            .map(|v| (u64::from(v.key.object_id()), v.key.generation))
            .collect();
        world.resolve_spawn(&key, &operational)
    }
    /// Explicit external resources only. Configuration must precede admission;
    /// this does not install Acquisition callbacks or create any mine/vehicle.
    pub fn with_dynamic_collision_bodies(
        mut self,
        catalog: Catalog,
        bindings: RuntimeBindings,
    ) -> Result<Self, &'static str> {
        if self.active.is_some()
            || !self.peers.is_empty()
            || self.wire_profile.application_id() != 20231
            || self.match_profile().game_type != 1
        {
            return Err("dynamic_bodies_require_idle_retail_acquisition");
        }
        bindings.validate_for(&catalog, self.ground_vehicles.as_ref())?;
        let resources = Resources {
            origin: std::sync::Arc::new(()),
            catalog,
            bindings,
        };
        if let Some(vehicles) = &self.ground_vehicles {
            resources.validate_vehicles(vehicles)?;
        }
        if let Some(profiles) = &self.match_profiles {
            resources.validate_profiles(profiles)?;
        }
        self.dynamic_body_resources = Some(resources);
        Ok(self)
    }
    /// Complete for the Engine's currently owned objects. Initial source inputs
    /// do not establish mutable native ground lifecycle or live mine authority.
    /// Ground poses, incarnations and root membership come from the owned
    /// mutable runtime; the catalog supplies immutable identity and geometry.
    /// Client vehicle driving admission is a separate unfinished consumer.
    pub fn dynamic_collision_snapshot(
        &self,
        current_filter_row6: u32,
    ) -> Result<Snapshot, &'static str> {
        if self.failed {
            return Err("engine_unavailable");
        }
        if self.match_profile().game_type != 1 {
            return Err("dynamic_bodies_require_acquisition");
        }
        let resources = self
            .dynamic_body_resources
            .as_ref()
            .ok_or("dynamic_body_resources_unavailable")?;
        let first = self
            .active
            .iter()
            .chain(self.peers.values())
            .next()
            .ok_or("dynamic_body_world_unavailable")?;
        let world = WorldKey {
            world_id: first.reservation.world,
            incarnation: self.round_number.max(1),
        };
        let mut roster = Vec::new();
        let mut rows = Vec::new();
        let mut character = |id: u16,
                             generation: u32,
                             team: u8,
                             position: [f64; 3],
                             dead: bool|
         -> Result<(), &'static str> {
            let target = Target {
                key: TargetKey {
                    id: u64::from(0x03800000 | u32::from(id)),
                    generation: generation.max(1),
                },
                kind: TargetKind::Character,
                team,
            };
            roster.push(target);
            rows.push(resources.catalog.instantiate(
                &resources.bindings.character_template,
                target,
                Pose::at(position),
                dead,
            )?);
            Ok(())
        };
        for actor in self.active.iter().chain(self.peers.values()) {
            if actor.reservation.world != world.world_id {
                return Err("dynamic_body_world_mismatch");
            }
            let (generation, dead) = match actor.life {
                CharacterLife::AwaitingFirstSpawn => (1, false),
                CharacterLife::Alive { generation, .. }
                | CharacterLife::Incapacitated { generation, .. } => (generation, false),
                CharacterLife::AwaitingRespawn { generation } => (generation, true),
            };
            let position = if let Some(seat) = actor.ground_seat {
                point(
                    self.current_ground_vehicle(seat.vehicle_index)
                        .ok_or("dynamic_body_mount_vehicle_unavailable")?
                        .position,
                )
            } else if actor.mounted_seat.is_some() {
                return Err("dynamic_body_mount_pose_unqualified");
            } else if let Some(sample) = actor.tracking.sample {
                point(sample.position)
            } else if matches!(actor.life, CharacterLife::AwaitingFirstSpawn) {
                [0.; 3]
            } else {
                return Err("dynamic_body_character_pose_unavailable");
            };
            character(
                actor.reservation.player,
                generation,
                actor.team(),
                position,
                dead,
            )?;
            if let Some(target) = &actor.development_target {
                let (position, generation) = target
                    .runtime
                    .map_or((point(target.position), target.generation), |r| {
                        (r.physical_position(), r.generation)
                    });
                character(
                    target.object_index,
                    generation,
                    target.squad.team(),
                    position,
                    actor.dummy_health == Some(0),
                )?;
            } else if actor.dummy_created {
                character(
                    dummy::OBJECT_INDEX,
                    1,
                    actor.team(),
                    point(
                        self.development_dummy_position
                            .ok_or("dynamic_dummy_pose_unavailable")?,
                    ),
                    actor.dummy_health == Some(0),
                )?;
            }
        }
        for bot in self.bot_registry.bots.values() {
            let runtime = bot
                .target
                .runtime
                .ok_or("dynamic_body_bot_runtime_unavailable")?;
            character(
                bot.target.object_index,
                runtime.generation,
                bot.target.squad.team(),
                runtime.physical_position(),
                bot.health == 0,
            )?;
        }
        if let Some(vehicles) = &self.ground_vehicles {
            for v in vehicles.creations() {
                let runtime = self
                    .ground_runtime
                    .as_ref()
                    .and_then(|r| r.get(v.index))
                    .ok_or("dynamic_ground_authority_unavailable")?;
                let current = self
                    .current_ground_vehicle(v.index)
                    .ok_or("dynamic_ground_authority_unavailable")?;
                let target = Target {
                    key: TargetKey {
                        id: u64::from(v.object_id()),
                        generation: runtime.key.generation,
                    },
                    kind: TargetKind::GroundVehicle,
                    team: v.team,
                };
                roster.push(target);
                let template = resources.vehicle_template(v)?;
                rows.push(match &runtime.body {
                    acquisition_ground_runtime::Body::Inactive => resources
                        .catalog
                        .instantiate_inactive_ground(template, target, vehicle_pose(&current))?,
                    acquisition_ground_runtime::Body::Operational => resources
                        .catalog
                        .instantiate(template, target, vehicle_pose(&current), false)?,
                    acquisition_ground_runtime::Body::Hulk { template } => resources
                        .catalog
                        .instantiate(template, target, vehicle_pose(&current), false)?,
                });
            }
        }
        // There is no accepted mine creation/registry in this Engine. These empty
        // owned mine rows do not claim support for native mines or unknown input.
        Snapshot::capture(world, self.sequence, &roster, rows, current_filter_row6)
            .map(|v| v.bind_engine(&resources.origin))
    }
    /// The callback owner must check this while serializing actual authority,
    /// before record/install. The caller must also recheck external current filter
    /// authority. Snapshot creation/query itself never mutates Engine state.
    pub fn dynamic_collision_snapshot_is_current(&self, snapshot: &Snapshot) -> bool {
        let Some(first) = self.active.iter().chain(self.peers.values()).next() else {
            return false;
        };
        !self.failed
            && self
                .dynamic_body_resources
                .as_ref()
                .is_some_and(|v| snapshot.is_from_engine(&v.origin))
            && snapshot.context()
                == (
                    WorldKey {
                        world_id: first.reservation.world,
                        incarnation: self.round_number.max(1),
                    },
                    self.sequence,
                )
    }
}
