//! Classified Ground damage at an ordinary frag's accepted owner-event fuse.
use super::*;
use crate::{
    dynamic_collision_runtime::{OwnerCollisionView, vehicle_pose},
    projectile::{ProjectileTerminal, ProjectileTerminalReason},
};

#[derive(Default)]
pub(crate) struct GroundFragCandidate {
    pub(crate) after: Option<ground::Runtime>,
    pub(crate) packet: Option<Vec<u8>>,
    pub(crate) audit: Vec<Value>,
}

impl<W: Write> Engine<W> {
    pub(crate) fn prepare_acquisition_ground_frag(
        &self,
        actor: &Active,
        staged: &ground::Runtime,
        owner_position: Option<replication::CharacterPosition>,
        terminals: &[ProjectileTerminal],
        now: Duration,
    ) -> Result<GroundFragCandidate, &'static str> {
        let runtime = self
            .acquisition_gameplay
            .as_ref()
            .ok_or("acquisition_frag_ground_requires_gameplay")?;
        let rules = runtime
            .setup
            .frag_ground_areas
            .as_deref()
            .ok_or("acquisition_frag_ground_requires_areas")?;
        let policy = runtime
            .setup
            .ground_damage
            .as_ref()
            .ok_or("acquisition_frag_ground_requires_damage_policy")?;
        let collision = runtime
            .blast_collision()
            .ok_or("acquisition_frag_ground_requires_structural")?;
        let scene = self
            .collision_world
            .as_ref()
            .filter(|world| {
                world.stats().triangles != 0 && world.metadata().map_name == self.map_name()
            })
            .ok_or("acquisition_frag_ground_requires_static_world")?;
        let snapshot = self.owner_collision_snapshot(
            collision.filter_rows[6],
            OwnerCollisionView {
                active_position: owner_position,
                ground: Some(staged),
            },
        )?;
        if snapshot.context().0.world_id != actor.reservation.world {
            return Err("stale_acquisition_frag_ground_world");
        }
        let catalog = self.item_catalog.retail_combat()?;
        let mut overlap_budget = collision.limits.max_support_points;
        let mut static_budget = 1_000_000;
        let mut candidate = GroundFragCandidate::default();
        let mut changes = BTreeMap::new();
        let player = u32::from(actor.reservation.player);
        let attacker = 0x0380_0000 | player;
        for terminal in terminals {
            let Some(position) = terminal
                .position
                .filter(|_| terminal.reason == ProjectileTerminalReason::ReportedDetonation)
            else {
                continue;
            };
            let frag = catalog
                .grenade_for_damage(terminal.item_id)
                .map_err(|_| "acquisition_frag_ground_item_unavailable")?;
            let amount_hp = frag.damage.vehicle as f32;
            let half_radius = frag.half_radius as f32;
            let outer_radius = frag.full_radius as f32;
            let penetration =
                frag.material_penetration
                    .ok_or("acquisition_frag_ground_material_required")? as f32;
            if !amount_hp.is_finite()
                || amount_hp < 0.
                || !half_radius.is_finite()
                || half_radius <= 0.
                || !outer_radius.is_finite()
                || outer_radius <= half_radius
                || !penetration.is_finite()
                || penetration < 0.
            {
                return Err("invalid_acquisition_frag_ground_dose");
            }
            if amount_hp == 0. {
                continue;
            }
            let origin_f32 = [position.x, position.y, position.z];
            let origin = origin_f32.map(f64::from);
            let sphere = Geometry::Sphere {
                radius: f64::from(outer_radius),
            };
            let projectile_id = ((16 + u32::from(terminal.slot)) << 24) | 0x0080_0000 | player;
            let mut seen = BTreeSet::new();
            for body in snapshot.bodies() {
                let key = body.target.key;
                if body.target.kind != TargetKind::GroundVehicle
                    || collision.filter_rows[6] & (1 << body.layer) == 0
                    || seen.contains(&(key.id, key.generation))
                {
                    continue;
                }
                let index = key.id as u16;
                let current = candidate.after.as_ref().unwrap_or(staged);
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
                let creation = self
                    .current_ground_vehicle_in(index, staged)
                    .ok_or("unknown_ground_vehicle")?;
                if !rules.accepts_root(&creation, &body.root)?
                    || !geometry::penetrates(
                        &sphere,
                        Pose::at(origin),
                        &body.shape,
                        body.pose,
                        collision.limits,
                        &mut overlap_budget,
                    )?
                {
                    continue;
                }
                // Both source roots can resolve to this vehicle. Apply one dose
                // per allocated vehicle per terminal, regardless of which root hit.
                seen.insert((key.id, key.generation));
                let pose = vehicle_pose(&creation);
                let anchor = [
                    creation.position.x,
                    creation.position.y,
                    creation.position.z,
                ];
                let (displacement, distance_squared) = separation(origin_f32, anchor)?;
                let scale =
                    armor::native_radial_scale(distance_squared.sqrt(), half_radius, outer_radius)?;
                if scale <= 0.
                    || !collision.clear_path(
                        scene,
                        &snapshot,
                        BlastRay {
                            start: origin,
                            end: pose.translation,
                            excludes: [Some(key.id), None],
                            penetration,
                            zero_penetration_strike: false,
                            filter_static: true,
                        },
                        &mut static_budget,
                    )?
                {
                    continue;
                }
                let location = rules.location(&creation, pose, origin)?;
                let friendly = actor.team() == creation.team;
                let amount = policy.classified_vehicle_blast_amount(
                    &creation, amount_hp, scale, location, friendly,
                )?;
                if amount == 0. {
                    continue;
                }
                let user_data = character::explosion_direction_user_data(direction(
                    displacement,
                    distance_squared,
                ))?;
                let (key, health_before) = (vehicle.key, vehicle.health_quarter_points);
                let after = candidate.after.get_or_insert_with(|| staged.clone());
                if after
                    .apply_classified_vehicle_blast_damage(
                        key, amount, attacker, location, user_data,
                    )?
                    .is_empty()
                {
                    continue;
                }
                changes.insert(index, 1 << 18);
                candidate.audit.push(json!({
                    "vehicle_index":index, "slot":terminal.slot, "generation":terminal.generation,
                    "vehicle_generation":key.generation, "location":location, "root":body.root,
                    "projectile_id":projectile_id, "source_object":u32::MAX, "attacker":attacker,
                    "friendly":friendly, "radial_scale":scale, "amount_hp":amount,
                    "health_before_quarter_points":health_before,
                    "health_after_quarter_points":after.get(index).unwrap().health_quarter_points,
                }));
            }
        }
        if changes.is_empty() {
            candidate.after = None;
        } else if let Some(after) = &candidate.after {
            candidate.packet = self.encode_ground_changes(after, &changes, now)?;
        }
        Ok(candidate)
    }
}
