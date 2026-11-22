//! Classified Ground damage from ordinary frags and Acquisition infantry rockets.
use super::*;
use crate::{
    dynamic_collision_runtime::{OwnerCollisionView, vehicle_pose},
    hospital_rocket_runtime::LaunchSource,
    infantry_rockets::Blast as InfantryRocketBlast,
    projectile::{ProjectileTerminal, ProjectileTerminalReason},
};

#[cfg(test)]
#[path = "domination_frag_ground_damage_tests.rs"]
mod domination_tests;

#[derive(Default)]
pub(crate) struct GroundFragCandidate {
    pub(crate) after: Option<ground::Runtime>,
    pub(crate) packet: Option<Vec<u8>>,
    pub(crate) audit: Vec<Value>,
}

struct GroundBlastDose {
    origin: [f64; 3],
    amount_hp: f32,
    half_radius: f32,
    outer_radius: f32,
    penetration: f32,
    projectile_id: u32,
    frag_slot_and_generation: Option<(u8, u32)>,
}

struct GroundBlastSource {
    attacker: u32,
    team: u8,
}

struct GroundBlastPlan<'a> {
    rules: &'a acquisition_frag_ground::Catalog,
    policy: &'a acquisition_ground_damage::Policy,
    collision: BlastCollision<'a>,
    scene: &'a collision::CollisionWorld,
    snapshot: dynamic_collision::Snapshot,
    staged: &'a ground::Runtime,
    source: GroundBlastSource,
    overlap_budget: usize,
    static_budget: usize,
    candidate: GroundFragCandidate,
    changes: BTreeMap<u16, u32>,
}

impl<'a> GroundBlastPlan<'a> {
    fn new(
        rules: &'a acquisition_frag_ground::Catalog,
        policy: &'a acquisition_ground_damage::Policy,
        collision: BlastCollision<'a>,
        scene: &'a collision::CollisionWorld,
        snapshot: dynamic_collision::Snapshot,
        staged: &'a ground::Runtime,
        source: GroundBlastSource,
    ) -> Self {
        Self {
            rules,
            policy,
            collision,
            scene,
            snapshot,
            staged,
            source,
            overlap_budget: collision.limits.max_support_points,
            static_budget: 1_000_000,
            candidate: GroundFragCandidate::default(),
            changes: BTreeMap::new(),
        }
    }

    fn apply<W: Write>(
        &mut self,
        engine: &Engine<W>,
        dose: GroundBlastDose,
    ) -> Result<(), &'static str> {
        let origin_f32 = dose.origin.map(|component| component as f32);
        if !dose.amount_hp.is_finite()
            || dose.amount_hp < 0.
            || !dose.half_radius.is_finite()
            || dose.half_radius <= 0.
            || !dose.outer_radius.is_finite()
            || dose.outer_radius <= dose.half_radius
            || !dose.penetration.is_finite()
            || dose.penetration < 0.
        {
            return Err(if dose.frag_slot_and_generation.is_some() {
                "invalid_frag_ground_dose"
            } else {
                "invalid_infantry_rocket_ground_dose"
            });
        }
        if dose.amount_hp == 0. {
            return Ok(());
        }

        let sphere = Geometry::Sphere {
            radius: f64::from(dose.outer_radius),
        };
        let mut seen = BTreeSet::new();
        for body in self.snapshot.bodies() {
            let key = body.target.key;
            if body.target.kind != TargetKind::GroundVehicle
                || self.collision.filter_rows[6] & (1 << body.layer) == 0
                || seen.contains(&(key.id, key.generation))
            {
                continue;
            }
            let index = key.id as u16;
            let current = self.candidate.after.as_ref().unwrap_or(self.staged);
            let Some(vehicle) = current.get(index).filter(|vehicle| {
                key.id == u64::from(vehicle.key.object_id())
                    && vehicle.key.generation == key.generation
                    && matches!(vehicle.source_spawn_type, Some(3 | 7 | 9))
                    && vehicle.state == 2
                    && vehicle.body == ground::Body::Operational
                    && !vehicle.is_dead()
                    && !vehicle.is_dying()
            }) else {
                continue;
            };
            let creation = engine
                .current_ground_vehicle_in(index, self.staged)
                .ok_or("unknown_ground_vehicle")?;
            if !self.rules.accepts_root(&creation, &body.root)?
                || !geometry::penetrates(
                    &sphere,
                    Pose::at(dose.origin),
                    &body.shape,
                    body.pose,
                    self.collision.limits,
                    &mut self.overlap_budget,
                )?
            {
                continue;
            }
            // Chassis and wheel roots resolve to the same allocated vehicle.
            // Apply one dose per vehicle per explosion.
            seen.insert((key.id, key.generation));
            let pose = vehicle_pose(&creation);
            let anchor = [
                creation.position.x,
                creation.position.y,
                creation.position.z,
            ];
            let (displacement, distance_squared) = separation(origin_f32, anchor)?;
            let scale = armor::native_radial_scale(
                distance_squared.sqrt(),
                dose.half_radius,
                dose.outer_radius,
            )?;
            if scale <= 0.
                || !self.collision.clear_path(
                    self.scene,
                    &self.snapshot,
                    BlastRay {
                        start: dose.origin,
                        end: pose.translation,
                        excludes: [Some(key.id), None],
                        penetration: dose.penetration,
                        zero_penetration_strike: false,
                        filter_static: true,
                    },
                    &mut self.static_budget,
                )?
            {
                continue;
            }
            let location = self.rules.location(&creation, pose, dose.origin)?;
            let friendly = self.source.team == creation.team;
            let amount = self.policy.classified_vehicle_blast_amount(
                &creation,
                dose.amount_hp,
                scale,
                location,
                friendly,
            )?;
            if amount == 0. {
                continue;
            }
            let user_data = character::explosion_direction_user_data(direction(
                displacement,
                distance_squared,
            ))?;
            let (key, health_before) = (vehicle.key, vehicle.health_quarter_points);
            let after = self
                .candidate
                .after
                .get_or_insert_with(|| self.staged.clone());
            if after
                .apply_classified_vehicle_blast_damage(
                    key,
                    amount,
                    self.source.attacker,
                    location,
                    user_data,
                )?
                .is_empty()
            {
                continue;
            }
            self.changes.insert(index, 1 << 18);
            let mut row = json!({
                "vehicle_index":index,
                "vehicle_generation":key.generation, "location":location, "root":body.root,
                "projectile_id":dose.projectile_id, "source_object":u32::MAX,
                "attacker":self.source.attacker,
                "friendly":friendly, "radial_scale":scale, "amount_hp":amount,
                "health_before_quarter_points":health_before,
                "health_after_quarter_points":after.get(index).unwrap().health_quarter_points,
            });
            if let Some((slot, generation)) = dose.frag_slot_and_generation {
                row["slot"] = json!(slot);
                row["generation"] = json!(generation);
            }
            self.candidate.audit.push(row);
        }
        Ok(())
    }

    fn finish<W: Write>(
        mut self,
        engine: &Engine<W>,
        now: Duration,
    ) -> Result<GroundFragCandidate, &'static str> {
        if self.changes.is_empty() {
            self.candidate.after = None;
        } else if let Some(after) = &self.candidate.after {
            self.candidate.packet = engine.encode_ground_changes(after, &self.changes, now)?;
        }
        Ok(self.candidate)
    }
}

impl<W: Write> Engine<W> {
    pub(crate) fn ground_frag_areas(&self) -> Option<&acquisition_frag_ground::Catalog> {
        if let Some(world) = &self.domination_gameplay {
            return world.state.frag_ground_areas.as_deref();
        }
        self.acquisition_gameplay
            .as_ref()?
            .setup
            .frag_ground_areas
            .as_deref()
    }

    fn ground_blast_static_world(&self) -> Option<&collision::CollisionWorld> {
        self.collision_world.as_ref().filter(|world| {
            world.stats().triangles != 0 && world.metadata().map_name == self.map_name()
        })
    }

    pub(crate) fn prepare_ground_frag(
        &self,
        actor: &Active,
        staged: &ground::Runtime,
        owner_position: Option<replication::CharacterPosition>,
        terminals: &[ProjectileTerminal],
        now: Duration,
    ) -> Result<GroundFragCandidate, &'static str> {
        let rules = self
            .ground_frag_areas()
            .ok_or("frag_ground_requires_areas")?;
        let policy = self
            .ground_damage_policy()
            .ok_or("frag_ground_requires_damage_policy")?;
        let collision = if let Some(world) = &self.domination_gameplay {
            world.frag_blast_collision()
        } else {
            self.acquisition_gameplay
                .as_ref()
                .and_then(|runtime| runtime.blast_collision())
                .ok_or("acquisition_frag_ground_requires_structural")?
        };
        let scene = self
            .ground_blast_static_world()
            .ok_or("frag_ground_requires_static_world")?;
        let snapshot = self.owner_collision_snapshot(
            collision.filter_rows[6],
            OwnerCollisionView {
                active_position: owner_position,
                ground: Some(staged),
            },
        )?;
        if snapshot.context().0.world_id != actor.reservation.world {
            return Err("stale_frag_ground_world");
        }

        let catalog = self.item_catalog.retail_combat()?;
        let player = u32::from(actor.reservation.player);
        let source = GroundBlastSource {
            attacker: 0x0380_0000 | player,
            team: actor.team(),
        };
        let mut plan =
            GroundBlastPlan::new(rules, policy, collision, scene, snapshot, staged, source);
        for terminal in terminals {
            let Some(position) = terminal
                .position
                .filter(|_| terminal.reason == ProjectileTerminalReason::ReportedDetonation)
            else {
                continue;
            };
            let frag = catalog
                .grenade_for_damage(terminal.item_id)
                .map_err(|_| "frag_ground_item_unavailable")?;
            plan.apply(
                self,
                GroundBlastDose {
                    origin: [position.x, position.y, position.z].map(f64::from),
                    amount_hp: frag.damage.vehicle as f32,
                    half_radius: frag.half_radius as f32,
                    outer_radius: frag.full_radius as f32,
                    penetration: frag
                        .material_penetration
                        .ok_or("frag_ground_material_required")?
                        as f32,
                    projectile_id: ((16 + u32::from(terminal.slot)) << 24) | 0x0080_0000 | player,
                    frag_slot_and_generation: Some((terminal.slot, terminal.generation)),
                },
            )?;
        }
        plan.finish(self, now)
    }

    /// Builds the Ground part of an infantry rocket terminal transaction.
    /// The terminal caller records the combined effects before installing them.
    pub(crate) fn prepare_infantry_rocket_acquisition_ground(
        &self,
        source: &LaunchSource,
        origin: [f64; 3],
        blast: InfantryRocketBlast,
        now: Duration,
    ) -> Result<GroundFragCandidate, &'static str> {
        let attacker = self.hospital_rocket_blast_attacker(source)?;
        if !origin
            .into_iter()
            .all(|component| (component as f32).is_finite())
        {
            return Err("invalid_infantry_rocket_ground_dose");
        }
        let runtime = self
            .acquisition_gameplay
            .as_ref()
            .ok_or("infantry_rocket_acquisition_ground_requires_world")?;
        let staged = self
            .ground_runtime
            .as_ref()
            .ok_or("infantry_rocket_acquisition_ground_requires_runtime")?;
        let rules = runtime
            .setup
            .frag_ground_areas
            .as_deref()
            .ok_or("infantry_rocket_acquisition_ground_requires_areas")?;
        let policy = runtime
            .setup
            .ground_damage
            .as_ref()
            .ok_or("infantry_rocket_acquisition_ground_requires_damage_policy")?;
        let collision = runtime
            .blast_collision()
            .ok_or("infantry_rocket_acquisition_ground_requires_structural")?;
        let scene = self
            .ground_blast_static_world()
            .ok_or("infantry_rocket_acquisition_ground_requires_static_world")?;
        let snapshot = self.dynamic_collision_snapshot(collision.filter_rows[6])?;
        if snapshot.context().0.world_id != source.world {
            return Err("stale_infantry_rocket_acquisition_ground_world");
        }

        let mut plan = GroundBlastPlan::new(
            rules,
            policy,
            collision,
            scene,
            snapshot,
            staged,
            GroundBlastSource {
                attacker,
                team: source.team,
            },
        );
        plan.apply(
            self,
            GroundBlastDose {
                origin,
                amount_hp: blast.vehicle_hp,
                half_radius: blast.half_radius_m,
                outer_radius: blast.outer_radius_m,
                penetration: blast.material_penetration,
                projectile_id: source.projectile_id,
                frag_slot_and_generation: None,
            },
        )?;
        plan.finish(self, now)
    }
}
