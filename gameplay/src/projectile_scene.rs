//! The admitted static reference pose and current owned projectile obstacles.
use crate::{
    Engine,
    acquisition_volume_damage::{TargetKey, TargetKind},
    collision::CollisionWorld,
    domination_projectile_collision::ContactBindings,
    domination_structural_world::SequenceCollisionState,
    dynamic_collision::{QueryLimits, Snapshot},
    projectile_contact::{self, ConvexObstacle, ObstacleId, Sweep, TriangleId},
};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct OwnedContact {
    pub(crate) obstacle: OwnedObstacleId,
    pub(crate) fraction: f64,
    pub(crate) center: [f64; 3],
    pub(crate) normal: Option<[f64; 3]>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum OwnedObstacleId {
    Static {
        instance: String,
        triangle: usize,
    },
    Sequence {
        placement: String,
        instance: String,
        triangle: usize,
    },
    Dynamic {
        key: TargetKey,
        root: String,
    },
}

impl From<ObstacleId<'_>> for OwnedObstacleId {
    fn from(id: ObstacleId<'_>) -> Self {
        match id {
            ObstacleId::Triangle(TriangleId::Static { instance, triangle }) => Self::Static {
                instance: instance.to_owned(),
                triangle,
            },
            ObstacleId::Triangle(TriangleId::Sequence {
                placement,
                instance,
                triangle,
            }) => Self::Sequence {
                placement: placement.to_owned(),
                instance: instance.to_owned(),
                triangle,
            },
            ObstacleId::Dynamic { key, root } => Self::Dynamic {
                key,
                root: root.to_owned(),
            },
        }
    }
}

fn discovery_error(error: &'static str) -> &'static str {
    match error {
        "world_cast_work_limit" => "projectile_contact_work_limit",
        other => other,
    }
}

pub(crate) struct Scene<'a> {
    static_world: &'a CollisionWorld,
    overlays: &'a SequenceCollisionState,
    snapshot: &'a Snapshot,
    bindings: &'a ContactBindings,
    filter_rows: [u32; 32],
    dynamic_verified: bool,
}

impl<'a> Scene<'a> {
    pub(crate) fn new(
        static_world: &'a CollisionWorld,
        overlays: &'a SequenceCollisionState,
        snapshot: &'a Snapshot,
        bindings: &'a ContactBindings,
        filter_rows: [u32; 32],
    ) -> Self {
        Self {
            static_world,
            overlays,
            snapshot,
            bindings,
            filter_rows,
            dynamic_verified: false,
        }
    }

    pub(crate) fn with_dynamic_catalog<W: std::io::Write>(
        mut self,
        engine: &Engine<W>,
    ) -> Result<Self, &'static str> {
        let resources = engine
            .dynamic_body_resources
            .as_ref()
            .ok_or("dynamic_body_resources_unavailable")?;
        self.bindings.validate_dynamic(&resources.catalog)?;
        if !engine.dynamic_collision_snapshot_is_current(self.snapshot) {
            return Err("projectile_dynamic_collision_snapshot_stale");
        }
        self.dynamic_verified = true;
        Ok(self)
    }

    pub(crate) fn query(
        &self,
        query: impl Into<Sweep>,
        budget: &mut usize,
    ) -> Result<Option<OwnedContact>, &'static str> {
        self.bindings.validate_static(self.static_world)?;
        if !self.dynamic_verified {
            return Err("projectile_dynamic_catalog_unverified");
        }
        let limits = QueryLimits::default();
        if *budget > limits.max_support_points {
            return Err("projectile_contact_invalid_limits");
        }
        let query = query.into();
        let sweep = query.discovery_envelope(limits.boundary_epsilon)?;
        let row = self.filter_rows[23];
        self.bindings.validate_scene_filter(row)?;
        let coverage = self.bindings.coverage();
        let mut triangles = self
            .static_world
            .projectile_obstacles(sweep.start, sweep.end, sweep.radius, coverage, row, budget)
            .map_err(discovery_error)?;
        let surfaces = self.overlays.projectile_surfaces(budget)?;
        for (placement, surface) in &surfaces {
            triangles.extend(
                surface
                    .projectile_obstacles(placement, &sweep, row, self.bindings, budget)
                    .map_err(discovery_error)?,
            );
        }
        let mut convex = Vec::new();
        for body in self.snapshot.bodies() {
            *budget = budget
                .checked_sub(1)
                .ok_or("projectile_contact_work_limit")?;
            if body.target.kind == TargetKind::GroundVehicle
                && self.bindings.ground_phantom(&body.root)
            {
                continue;
            }
            if row & (1 << body.layer) != 0 {
                convex.push(ConvexObstacle {
                    target: body.target,
                    root: &body.root,
                    shape: &body.shape,
                    pose: body.pose,
                });
            }
        }
        Ok(
            projectile_contact::sweep(query, coverage, &triangles, &convex, limits, budget)?.map(
                |contact| OwnedContact {
                    obstacle: contact.obstacle.into(),
                    fraction: contact.fraction,
                    center: contact.center,
                    normal: contact.normal,
                },
            ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        acquisition_atomic::WorldKey,
        acquisition_volume_damage::Target,
        dynamic_collision::{Catalog, Pose},
        projectile_contact::{CapsuleSweep, SphereSweep},
    };
    use serde_json::json;

    fn static_world(properties: bool) -> CollisionWorld {
        let mut instance = json!({
            "id": "synthetic-wall",
            "vertices_xyz": [[6.,-2.,-2.],[6.,2.,-2.],[6.,0.,2.]],
            "triangles": [[0,1,2]]
        });
        if properties {
            instance["collision"] = json!({
                "filter_info":9, "ray_child_filter_info":0,
                "convex_radius":0., "provenance":"synthetic", "child_filter_policy":"uniform"
            });
        }
        let raw = json!({
            "format":"mag_collision_world", "schema_version":1, "coordinate_space":"world",
            "metadata":{"map_name":"synthetic","build":"synthetic","coverage":"provisional",
                "provenance":["synthetic"],"unknowns":[]},
            "instances":[instance]
        });
        CollisionWorld::from_bytes(
            &serde_json::to_vec(&raw).unwrap(),
            crate::collision::CollisionLimits::default(),
        )
        .unwrap()
    }

    fn bindings(world: &CollisionWorld) -> ContactBindings {
        let digest = "a".repeat(64);
        let hospital: Vec<_> = (0..3)
            .map(|faction| {
                json!({
                    "binding":{"asset_sha256":digest,"faction":faction,
                        "packfile_sha256":digest,"part_ids":["hospital-face"]},
                    "contact":{"root_filter_info":22,"child_filter_info":0,"convex_radius":0.}
                })
            })
            .collect();
        ContactBindings::from_json(&serde_json::to_vec(&json!({
            "schema":"mag_domination_projectile_contacts_v1","map_name":"synthetic",
            "retail_elf_sha256":digest,"proof_sha256":digest,
            "static_world_sha256":world.source_sha256(),"dynamic_catalog_sha256":digest,
            "static_coverage_policy":"represented_reference_pose","ground_phantom_roots":["wheel"],
            "sequence":[{"binding":{"source_sha256":digest,"neutral_sha256":digest,
                "collision_tag":9,"part_ids":["synthetic-wall"]},
                "contact":{"root_filter_info":9,"child_filter_info":0,"convex_radius":0.}}],
            "hospital":hospital,"helo":[]
        })).unwrap()).unwrap()
    }

    fn snapshot() -> Snapshot {
        let digest = "a".repeat(64);
        let catalog = Catalog::from_json(&json!({
            "format":"mag_dynamic_bodies", "schema_version":1,
            "source":{"native_binding_sha256":digest,"shape_data_sha256":digest},
            "templates":[{"id":"ground", "kind":"ground_vehicle","roots":[
                {"id":"chassis","layer":22,"translation":[1.,0.,0.],"rotation":"entity",
                    "shape":{"kind":"box","half_extents":[0.2,0.2,0.2],"radius":0.1}},
                {"id":"wheel","layer":22,"translation":[-1.,0.,0.],"rotation":"entity",
                    "shape":{"kind":"box","half_extents":[0.2,0.2,0.2],"radius":0.1}}]},
            {"id":"character", "kind":"character","roots":[
                {"id":"capsule","layer":21,"dead_layer":6,"translation":[0.,0.,0.],"rotation":"identity",
                    "shape":{"kind":"capsule","a":[0.,0.,0.],"b":[0.,1.,0.],"radius":0.4}}]}]
        }).to_string()).unwrap();
        let ground = Target {
            key: TargetKey {
                id: 1,
                generation: 1,
            },
            kind: TargetKind::GroundVehicle,
            team: 0,
        };
        let origin = Target {
            key: TargetKey {
                id: 2,
                generation: 1,
            },
            kind: TargetKind::Character,
            team: 0,
        };
        Snapshot::capture(
            WorldKey {
                world_id: 1,
                incarnation: 1,
            },
            1,
            &[ground, origin],
            vec![
                catalog
                    .instantiate("ground", ground, Pose::at([4., 0., 0.]), false)
                    .unwrap(),
                catalog
                    .instantiate("character", origin, Pose::at([0., 0., 0.]), false)
                    .unwrap(),
            ],
            0,
        )
        .unwrap()
    }

    fn sweep(associated_character: Option<TargetKey>) -> SphereSweep {
        SphereSweep {
            start: [-1., 0., 0.],
            end: [8., 0., 0.],
            radius: 0.1,
            associated_character,
        }
    }

    #[test]
    fn one_scene_compares_live_static_and_solid_roots_and_excludes_only_associated_character() {
        let world = static_world(true);
        let bindings = bindings(&world);
        let snapshot = snapshot();
        let mut overlays = SequenceCollisionState::default();
        overlays.synthetic_projectile_hospital(4, 4., "a".repeat(64));
        let mut filters = [0; 32];
        filters[23] = (1 << 0) | (1 << 9) | (1 << 21) | (1 << 22);
        let key = Some(TargetKey {
            id: 2,
            generation: 1,
        });
        let run = |overlays, filters, key, budget| {
            let mut scene = Scene::new(&world, overlays, &snapshot, &bindings, filters);
            scene.dynamic_verified = true;
            scene.query(sweep(key), &mut { budget })
        };
        assert!(matches!(
            run(&overlays, filters, None, 10000).unwrap().unwrap().obstacle,
            OwnedObstacleId::Dynamic { root, .. } if root == "capsule"
        ));
        assert!(matches!(
            run(&overlays, filters, key, 10000).unwrap().unwrap().obstacle,
            OwnedObstacleId::Sequence { placement, .. } if placement == "hospital-4"
        ));
        let mut moved_overlays = overlays.clone();
        moved_overlays.synthetic_projectile_hospital(4, 7., "a".repeat(64));
        assert!(matches!(
            run(&moved_overlays, filters, key, 10000).unwrap().unwrap().obstacle,
            OwnedObstacleId::Dynamic { root, .. } if root == "chassis"
        ));
        filters[23] &= !(1 << 22);
        assert!(matches!(
            run(&moved_overlays, filters, key, 10000)
                .unwrap()
                .unwrap()
                .obstacle,
            OwnedObstacleId::Static { .. }
        ));
        assert_eq!(
            run(&moved_overlays, filters, key, 2),
            Err("projectile_contact_work_limit")
        );
    }

    #[test]
    fn projectile_capsule_discovers_forward_extension_but_tests_the_exact_shape() {
        let world = static_world(true);
        let bindings = bindings(&world);
        let snapshot = snapshot();
        let mut overlays = SequenceCollisionState::default();
        overlays.synthetic_projectile_hospital(4, 4., "a".repeat(64));
        let mut filters = [0; 32];
        filters[23] = (1 << 0) | (1 << 9);
        let mut scene = Scene::new(&world, &overlays, &snapshot, &bindings, filters);
        scene.dynamic_verified = true;
        let capsule = CapsuleSweep {
            start: [3., 0., 0.],
            end: [5.6, 0., 0.],
            spine: [[0., 0., -0.25], [0., 0., 0.5]],
            radius: 0.05,
            rotation: [[0., 0., 1.], [0., 1., 0.], [-1., 0., 0.]],
            associated_character: None,
        };
        let hit = scene.query(capsule, &mut 10_000).unwrap().unwrap();
        assert!(matches!(hit.obstacle, OwnedObstacleId::Static { .. }));
        assert!((hit.center[0] - 5.45).abs() < 1e-6);
        assert_eq!(
            scene.query(
                SphereSweep {
                    start: capsule.start,
                    end: capsule.end,
                    radius: capsule.radius,
                    associated_character: None
                },
                &mut 10_000
            ),
            Ok(None)
        );

        // Orientation perpendicular to motion still discovers the candidate
        // through the same broad envelope, but exact contact is clear.
        assert_eq!(
            scene.query(
                CapsuleSweep {
                    rotation: Pose::at([0.; 3]).rotation,
                    ..capsule
                },
                &mut 10_000
            ),
            Ok(None)
        );
        filters[23] = (1 << 0) | (1 << 22);
        scene.filter_rows = filters;
        let hit = scene
            .query(
                CapsuleSweep {
                    end: [3.6, 0., 0.],
                    ..capsule
                },
                &mut 10_000,
            )
            .unwrap()
            .unwrap();
        assert!(
            matches!(hit.obstacle, OwnedObstacleId::Sequence { placement, .. }
            if placement == "hospital-4")
        );
        assert!((hit.center[0] - 3.45).abs() < 1e-6);
        assert_eq!(
            scene.query(capsule, &mut 2),
            Err("projectile_contact_work_limit")
        );
    }

    #[test]
    fn acquisition_projectile_scene_keeps_ground_chassis_and_rejects_foreign_overlays() {
        let world = static_world(true);
        let mut raw = crate::domination_projectile_collision::acquisition_tests::acquisition();
        raw["static_world_sha256"] = json!(world.source_sha256());
        let bindings =
            ContactBindings::from_acquisition_json(&serde_json::to_vec(&raw).unwrap()).unwrap();
        let snapshot = snapshot();
        let mut overlays = SequenceCollisionState::default();
        let mut filters = [0; 32];
        filters[23] = raw["filter_row23"].as_u64().unwrap() as u32;
        let associated = Some(TargetKey {
            id: 2,
            generation: 1,
        });
        let run = |filters, overlays: &SequenceCollisionState| {
            let mut scene = Scene::new(&world, overlays, &snapshot, &bindings, filters);
            scene.dynamic_verified = true;
            scene.query(sweep(associated), &mut 10_000)
        };
        assert!(matches!(
            run(filters, &overlays).unwrap().unwrap().obstacle,
            OwnedObstacleId::Dynamic { root, .. } if root == "chassis"
        ));
        let mut stale_filters = filters;
        stale_filters[23] &= !(1 << 22);
        assert_eq!(
            run(stale_filters, &overlays),
            Err("projectile_contact_filter_mismatch")
        );
        overlays.synthetic_projectile_hospital(4, 7., "a".repeat(64));
        assert_eq!(
            run(filters, &overlays),
            Err("projectile_overlay_scope_mismatch")
        );
    }

    #[test]
    fn absent_static_or_overlay_proof_fails_closed() {
        let world = static_world(false);
        let contact_bindings = bindings(&world);
        let snapshot = snapshot();
        let mut overlays = SequenceCollisionState::default();
        let mut filters = [0; 32];
        filters[23] = (1 << 0) | (1 << 9) | (1 << 22);
        let mut scene = Scene::new(&world, &overlays, &snapshot, &contact_bindings, filters);
        scene.dynamic_verified = true;
        assert_eq!(
            scene.query(sweep(None), &mut 1000),
            Err("projectile_static_properties_unavailable")
        );

        overlays.synthetic_projectile_hospital(4, 4., "b".repeat(64));
        let world = static_world(true);
        let contact_bindings = bindings(&world);
        let mut scene = Scene::new(&world, &overlays, &snapshot, &contact_bindings, filters);
        scene.dynamic_verified = true;
        assert_eq!(
            scene.query(sweep(None), &mut 1000),
            Err("projectile_overlay_binding_unavailable")
        );
    }
}
