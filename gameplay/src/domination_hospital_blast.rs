//! Vehicle-channel blast damage against current operational Domination Hospitals.
use super::*;
use crate::{
    dynamic_collision_runtime::OwnerCollisionView,
    hospital_rocket_runtime::LaunchSource,
    hospital_rockets::Blast,
    projectile::{ProjectileTerminal, ProjectileTerminalReason},
    structures::HospitalBlastDose,
};

#[derive(Default)]
pub(crate) struct HospitalBlastCandidate {
    pub(crate) patch: structures::HospitalDamagePatch,
    pub(crate) packet: Option<Vec<u8>>,
    pub(crate) audit: Vec<Value>,
}

struct HospitalBlastCredit {
    world: u32,
    attacker: u32,
    team: u8,
}

struct HospitalVehicleBlast {
    origin: [f64; 3],
    projectile_id: u32,
    vehicle_hp: f32,
    half_radius: f32,
    outer_radius: f32,
    penetration: f32,
}

impl<W: Write> Engine<W> {
    pub(crate) fn prepare_domination_hospital_frags(
        &self,
        actor: &Active,
        terminals: &[ProjectileTerminal],
        owner_view: OwnerCollisionView<'_>,
        now: Duration,
    ) -> Result<HospitalBlastCandidate, &'static str> {
        let world = self
            .domination_gameplay
            .as_ref()
            .ok_or("domination_hospital_blast_world_required")?;
        let catalog = self.item_catalog.retail_combat()?;
        let player = u32::from(actor.reservation.player);
        let mut blasts = Vec::new();
        for terminal in terminals {
            let Some(position) = terminal
                .position
                .filter(|_| terminal.reason == ProjectileTerminalReason::ReportedDetonation)
            else {
                continue;
            };
            let frag = catalog
                .grenade_for_damage(terminal.item_id)
                .map_err(|_| "domination_hospital_blast_frag_item_unavailable")?;
            let vehicle_hp = frag.damage.vehicle as f32;
            if vehicle_hp == 0. {
                continue;
            }
            blasts.push(HospitalVehicleBlast {
                origin: [position.x, position.y, position.z].map(f64::from),
                projectile_id: ((16 + u32::from(terminal.slot)) << 24) | 0x0080_0000 | player,
                vehicle_hp,
                half_radius: frag.half_radius as f32,
                outer_radius: frag.full_radius as f32,
                penetration: frag
                    .material_penetration
                    .ok_or("domination_hospital_blast_frag_material_required")?
                    as f32,
            });
        }
        self.prepare_hospital_vehicle_blasts(
            HospitalBlastCredit {
                world: actor.reservation.world,
                attacker: 0x0380_0000 | player,
                team: actor.team(),
            },
            &blasts,
            world.frag_blast_collision(),
            Some(owner_view),
            now,
        )
    }

    pub(crate) fn prepare_hospital_rocket_hospital_blast(
        &self,
        source: &LaunchSource,
        origin: [f64; 3],
        blast: Blast,
        collision: BlastCollision<'_>,
        now: Duration,
    ) -> Result<HospitalBlastCandidate, &'static str> {
        let attacker = self.hospital_rocket_blast_attacker(source)?;
        self.prepare_hospital_vehicle_blasts(
            HospitalBlastCredit {
                world: source.world,
                attacker,
                team: source.team,
            },
            &[HospitalVehicleBlast {
                origin,
                projectile_id: source.projectile_id,
                vehicle_hp: blast.vehicle_hp,
                half_radius: blast.half_radius_m,
                outer_radius: blast.outer_radius_m,
                penetration: blast.material_penetration,
            }],
            collision,
            None,
            now,
        )
    }

    fn prepare_hospital_vehicle_blasts(
        &self,
        credit: HospitalBlastCredit,
        blasts: &[HospitalVehicleBlast],
        collision: BlastCollision<'_>,
        owner_view: Option<OwnerCollisionView<'_>>,
        now: Duration,
    ) -> Result<HospitalBlastCandidate, &'static str> {
        if blasts.iter().any(|blast| {
            !blast
                .origin
                .map(|value| value as f32)
                .into_iter()
                .all(f32::is_finite)
                || !blast.vehicle_hp.is_finite()
                || blast.vehicle_hp <= 0.
                || !blast.half_radius.is_finite()
                || blast.half_radius <= 0.
                || !blast.outer_radius.is_finite()
                || blast.outer_radius <= blast.half_radius
                || !blast.penetration.is_finite()
                || blast.penetration < 0.
        }) {
            return Err("invalid_domination_hospital_vehicle_blast");
        }
        let mut candidate = HospitalBlastCandidate::default();
        if blasts.is_empty() || collision.filter_rows[6] & (1 << 22) == 0 {
            return Ok(candidate);
        }
        let state = &self
            .domination_gameplay
            .as_ref()
            .ok_or("domination_hospital_blast_world_required")?
            .state;
        let rules = state
            .hospital_blast_areas
            .as_deref()
            .ok_or("domination_hospital_blast_areas_required")?;
        let bunkers = self
            .bunker_world
            .as_ref()
            .ok_or("domination_hospital_blast_bunkers_required")?;
        let targets: Vec<_> = bunkers.damageable_domination_hospitals().collect();
        if targets.is_empty() {
            return Ok(candidate);
        }
        let assets = state
            .hospital_hulks
            .as_deref()
            .ok_or("domination_hospital_blast_requires_body_sources")?;
        let row = collision.filter_rows[6];
        let snapshot = match owner_view {
            Some(view) => self.owner_collision_snapshot(row, view)?,
            None => self.dynamic_collision_snapshot(row)?,
        };
        if snapshot.context().0.world_id != credit.world {
            return Err("stale_domination_hospital_blast_world");
        }
        let scene = self
            .collision_world
            .as_ref()
            .filter(|world| {
                world.stats().triangles != 0 && world.metadata().map_name == self.map_name()
            })
            .ok_or("domination_hospital_blast_requires_static_world")?;
        let mut overlap_budget = collision.limits.max_support_points;
        let mut static_budget = 1_000_000;
        let mut after: Option<structures::BunkerWorld> = None;
        let mut changed = BTreeSet::new();
        for blast in blasts {
            let origin = blast.origin;
            let sphere = Geometry::Sphere {
                radius: f64::from(blast.outer_radius),
            };
            for target in &targets {
                let index = target.index;
                if after
                    .as_ref()
                    .is_some_and(|current| !current.hospital_damageable(index))
                {
                    continue;
                }
                let surface = collision
                    .sequences
                    .hospitals
                    .get(&index)
                    .ok_or("domination_hospital_blast_mesh_required")?;
                let live_pose = operational_hospital_pose(target)?;
                let expected_mesh = assets
                    .by_faction
                    .get(&target.faction)
                    .and_then(|asset| asset.operational.as_ref())
                    .ok_or("unbound_domination_hospital_operational_mesh")?;
                if surface.pose.translation != live_pose.translation
                    || surface.pose.rotation != live_pose.rotation
                    || surface.mesh.source_sha256() != expected_mesh.source_sha256()
                {
                    return Err("domination_hospital_blast_mesh_mismatch");
                }
                if !surface.overlaps_sphere(
                    surface.local(origin),
                    &sphere,
                    collision.limits,
                    &mut overlap_budget,
                )? {
                    continue;
                }
                let (displacement, squared) = separation(
                    origin.map(|axis| axis as f32),
                    live_pose.translation.map(|axis| axis as f32),
                )?;
                let scale = armor::native_radial_scale(
                    squared.sqrt(),
                    blast.half_radius,
                    blast.outer_radius,
                )?;
                let packed_target = 0x0d80_0000 | u32::from(index);
                if scale <= 0.
                    || !collision.clear_path(
                        scene,
                        &snapshot,
                        BlastRay {
                            start: origin,
                            end: live_pose.translation,
                            excludes: [Some(u64::from(packed_target)), None],
                            penetration: blast.penetration,
                            zero_penetration_strike: false,
                            filter_static: true,
                        },
                        &mut static_budget,
                    )?
                {
                    continue;
                }
                let location = rules.location(target.faction, live_pose, origin)?;
                let user_data =
                    character::explosion_direction_user_data(direction(displacement, squared))?;
                let Some(hit) = after
                    .get_or_insert_with(|| bunkers.clone())
                    .apply_classified_hospital_blast(
                        index,
                        HospitalBlastDose {
                            vehicle_hp: blast.vehicle_hp,
                            radial_scale: scale,
                            source_team: credit.team,
                            attacker: credit.attacker,
                            location,
                            user_data,
                        },
                        now,
                    )?
                else {
                    continue;
                };
                changed.insert(index);
                candidate.audit.push(json!({
                    "vehicle_index":index,"target":packed_target,"location":location,
                    "projectile_id":blast.projectile_id,"attacker":credit.attacker,
                    "friendly":target.team == credit.team,"radial_scale":scale,"amount_hp":hit.amount_hp,
                    "last_source_object":u32::MAX,"user_data":user_data,
                    "health_before_quarter_points":hit.before_quarters,
                    "health_after_quarter_points":hit.after_quarters,
                }));
            }
        }
        if let Some(after) = after {
            candidate.patch = after.hospital_damage_patch(&changed)?;
            candidate.packet = candidate.patch.encode(now, self.wire_profile)?;
        }
        Ok(candidate)
    }
}

#[cfg(test)]
#[path = "domination_hospital_blast_tests.rs"]
pub(crate) mod tests;
