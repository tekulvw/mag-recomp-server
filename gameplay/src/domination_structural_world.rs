//! Damage from Structural objectives through the owned character transaction.
use crate::{
    acquisition_character_effects::{self as character, NativeState},
    acquisition_ground_damage, acquisition_ground_runtime as ground,
    acquisition_volume_damage::TargetKind,
    active_ability_effects::{BlastFalloff, BombletSample, GasVolume},
    domination_projectile_collision::{
        BodyBinding, ContactBindings, SequenceBinding, SurfaceBinding,
    },
    domination_structural_effects::{
        BlastProperties, Catalog as StructuralCatalog, Creation, InstantaneousBlast,
        PlaybackChange, TerrainMaterialGrid,
    },
    dynamic_collision::{Geometry, Pose, QueryLimits, geometry},
    replication::ObjectiveState,
    *,
};
use std::collections::{BTreeMap, BTreeSet};

#[path = "hospital_rocket_damage.rs"]
mod hospital_rocket_damage;

const CHARACTERS_PER_UPDATE: u16 = 16;
const CHARACTER_FAMILY_CAPACITY: u16 = 256;
const AREA_DAMAGE_COOLDOWN: Duration = Duration::from_secs(1);

#[cfg(test)]
#[path = "structural_area_mount_tests.rs"]
mod mount_tests;

#[derive(Clone, Default)]
pub struct AreaDamageState {
    round: Option<u64>,
    cursors: BTreeMap<u16, u16>,
    cooldowns: BTreeMap<u16, (u32, Duration)>,
}

#[derive(Clone, Copy, Debug)]
pub struct AreaUpdate {
    pub objective_index: u16,
    pub objective_position: [f32; 3],
    /// State whose update body ran. Its tail may execute after a transition.
    pub updated_state: ObjectiveState,
    pub world: u32,
    pub properties: BlastProperties,
}

/// One result from the radial overlap and penetration collector, in native
/// pair order. The position is the point sampled by the character callback.
#[derive(Clone, Copy, Debug)]
pub struct BlastObservation {
    pub target: BlastTarget,
    pub origin_index: usize,
    pub sample_position: [f32; 3],
    pub line_of_damage: bool,
}

#[derive(Clone, Copy, Debug)]
pub enum BlastTarget {
    Character(character::Target),
    Ground(GroundBlastTarget),
    Helo { index: u16, world: u32 },
}

#[derive(Clone, Copy, Debug)]
pub struct GroundBlastTarget {
    key: ground::Key,
    team: u8,
    world: u32,
}

#[derive(Clone, Copy, Debug)]
pub enum BlastSource {
    Structural { objective_id: u32 },
    Hospital { body_id: u32 },
    Ground { body_id: u32 },
    Helo { body_id: u32 },
    HeloGrenade,
    AirStrike,
    CommandBomblet(BlastFalloff),
    GuidedArtillery,
    GasPulse(BlastFalloff),
}

impl BlastSource {
    fn supported_mode(self, game_type: i8) -> bool {
        game_type == 2
            || (game_type == 1 && (self.is_command() || matches!(self, Self::Structural { .. })))
    }

    fn is_command(self) -> bool {
        matches!(
            self,
            Self::AirStrike | Self::CommandBomblet(_) | Self::GuidedArtillery | Self::GasPulse(_)
        )
    }

    fn needs_cover_ray(self) -> bool {
        !matches!(self, Self::GuidedArtillery | Self::GasPulse(_))
    }

    fn accepts_body(self, kind: TargetKind) -> bool {
        match self {
            Self::HeloGrenade => false,
            Self::GuidedArtillery => kind == TargetKind::GroundVehicle,
            Self::GasPulse(_) => kind == TargetKind::Character,
            Self::CommandBomblet(_) => {
                matches!(kind, TargetKind::Character | TargetKind::GroundVehicle)
            }
            _ => true,
        }
    }

    pub(crate) fn scale(
        self,
        blast: &InstantaneousBlast,
        distance: f32,
    ) -> Result<f32, &'static str> {
        match self {
            Self::GuidedArtillery => Ok(1.),
            Self::CommandBomblet(falloff) | Self::GasPulse(falloff) => {
                native_command_falloff(falloff, distance)
            }
            _ => blast.scale(distance),
        }
    }

    fn vehicle_body(self) -> Option<u32> {
        match self {
            Self::Structural { .. }
            | Self::HeloGrenade
            | Self::AirStrike
            | Self::CommandBomblet(_)
            | Self::GuidedArtillery
            | Self::GasPulse(_) => None,
            Self::Hospital { body_id } | Self::Ground { body_id } | Self::Helo { body_id } => {
                Some(body_id)
            }
        }
    }
}

#[derive(Clone, Debug)]
pub enum DamageWork {
    Area(AreaUpdate),
    Blast {
        source: BlastSource,
        attacker: u32,
        blast: InstantaneousBlast,
        observations: Vec<BlastObservation>,
    },
}

pub struct PreparedDamage {
    pub character_candidate: character::Candidate,
    pub area_state: AreaDamageState,
}

#[derive(Clone, Copy)]
pub struct BlastCollision<'a> {
    pub filter_rows: &'a [u32; 32],
    pub limits: QueryLimits,
    pub sequences: &'a SequenceCollisionState,
    pub material_costs: &'a BTreeMap<u8, f32>,
    pub terrain_materials: Option<&'a TerrainMaterialGrid>,
}

#[derive(Clone, Copy)]
struct BlastRay {
    start: [f64; 3],
    end: [f64; 3],
    excludes: [Option<u64>; 2],
    penetration: f32,
    zero_penetration_strike: bool,
    filter_static: bool,
}

impl BlastCollision<'_> {
    fn strike_impact(
        &self,
        scene: &collision::CollisionWorld,
        snapshot: &dynamic_collision::Snapshot,
        [x, z]: [f32; 2],
        [min_y, max_y]: [f32; 2],
        pad: f32,
    ) -> Result<Option<[f32; 3]>, &'static str> {
        let top = max_y + pad;
        let bottom = min_y - pad;
        if min_y > max_y || top <= bottom || ![x, z, top, bottom].iter().all(|v| v.is_finite()) {
            return Err("invalid_air_strike_vertical_bounds");
        }
        let start = [x, top, z].map(f64::from);
        let end = [x, bottom, z].map(f64::from);
        let row = self.filter_rows[19];
        let mut mesh_budget = 1_000_000;
        let mut hits = scene
            .filtered_material_ray_hits(start, end, row, &mut mesh_budget)
            .map_err(|_| "air_strike_static_impact_ray_failed")?;
        self.sequences.filtered_ray_hits(
            start,
            end,
            row,
            [None; 2],
            &mut mesh_budget,
            &mut hits,
        )?;
        let mut first = hits.iter().map(|hit| hit.fraction).min_by(f64::total_cmp);
        let mut shape_budget = self.limits.max_support_points;
        let point = Geometry::Box {
            half_extents: [0.; 3],
            radius: 0.,
        };
        for body in snapshot
            .bodies()
            .iter()
            .filter(|body| row & (1 << body.layer) != 0)
        {
            if let Some((fraction, _)) = geometry::sweep(
                &point,
                Pose::at(start),
                end,
                &body.shape,
                body.pose,
                geometry::SweepLimits {
                    tolerance: self.limits.boundary_epsilon,
                    extra: 0.,
                    query: self.limits,
                },
                &mut shape_budget,
            )? && fraction > 0.
                && fraction < first.unwrap_or(1.)
            {
                first = Some(fraction);
            }
        }
        Ok(first.map(|t| {
            let t = t as f32;
            [x, t.mul_add(bottom, (1. - t) * top) + 0.5, z]
        }))
    }

    fn clear_path(
        &self,
        scene: &collision::CollisionWorld,
        snapshot: &dynamic_collision::Snapshot,
        ray: BlastRay,
        budget: &mut usize,
    ) -> Result<bool, &'static str> {
        let BlastRay {
            start,
            end,
            excludes,
            penetration,
            zero_penetration_strike,
            filter_static,
        } = ray;
        if start == end {
            return Ok(true);
        }
        let row = self.filter_rows[24];
        let mut hits = if filter_static {
            scene.filtered_material_ray_hits(start, end, row, budget)
        } else {
            scene.material_ray_hits(start, end, budget)
        }
        .map_err(|_| "domination_structural_blast_static_query_failed")?;
        if zero_penetration_strike && !hits.is_empty() {
            return Ok(false);
        }
        for hit in hits.iter_mut().filter(|hit| hit.terrain) {
            let terrain = self
                .terrain_materials
                .ok_or("domination_terrain_material_grid_required")?;
            let x = (start[0] + (end[0] - start[0]) * hit.fraction) as f32;
            let z = (start[2] + (end[2] - start[2]) * hit.fraction) as f32;
            hit.material_index = Some(terrain.sample(x, z)?);
        }
        if zero_penetration_strike {
            // The retail callback blocks on a null or impenetrable material,
            // and on every material whose cost is at least the remaining power.
            // All qualified costs are -1 or nonnegative; AirStrike power is zero.
            self.sequences
                .filtered_ray_hits(start, end, row, excludes, budget, &mut hits)?;
            if !hits.is_empty() {
                return Ok(false);
            }
        } else {
            self.sequences
                .material_ray_hits(start, end, row, excludes, budget, &mut hits)?;
            if !can_penetrate(&mut hits, penetration, self.material_costs)? {
                return Ok(false);
            }
        }
        if !snapshot.positive_ray_hit_ignoring(
            start,
            end,
            excludes,
            *self.filter_rows,
            self.limits,
        )? {
            return Ok(true);
        }
        if zero_penetration_strike {
            return Ok(false);
        }
        // The row24 collector leaves fallback off for primitive ground roots.
        // Their null material blocks damage. Refuse unqualified eligible roots.
        if snapshot
            .bodies()
            .iter()
            .filter(|root| {
                !excludes.contains(&Some(root.target.key.id))
                    && self.filter_rows[24] & (1 << root.layer) != 0
            })
            .all(|root| {
                root.layer == 22
                    && root.target.kind == TargetKind::GroundVehicle
                    && matches!(
                        root.shape.as_ref(),
                        Geometry::Hull { .. } | Geometry::Box { .. }
                    )
            })
        {
            Ok(false)
        } else {
            Err("domination_structural_blast_dynamic_material_required")
        }
    }
}

pub(crate) struct DamageCommit {
    pub audit: Vec<Value>,
    pub messages: Vec<OutboundMessage>,
    pub changed_ground: BTreeSet<u16>,
    pub helo_doses: Vec<crate::domination_helo_damage::RadialDose>,
}

impl AreaDamageState {
    fn next_slots(
        &mut self,
        objective_index: u16,
        state: ObjectiveState,
        properties: BlastProperties,
    ) -> Option<std::ops::Range<u16>> {
        if !matches!(state, ObjectiveState::Completed | ObjectiveState::Repairing)
            || properties.character_damage_over_time_radius == 0.0
            || properties.character_damage_over_time_value <= 0.0
        {
            return None;
        }
        let cursor = self.cursors.entry(objective_index).or_default();
        let scanned = *cursor..(*cursor + CHARACTERS_PER_UPDATE);
        *cursor = scanned.end % CHARACTER_FAMILY_CAPACITY;
        Some(scanned)
    }

    fn cooling_down(&self, slot: u16, generation: u32, now: Duration) -> bool {
        self.cooldowns
            .get(&slot)
            .is_some_and(|&(current, until)| current == generation && now < until)
    }
}

impl<W: Write> Engine<W> {
    /// Resolve a sampled bomblet against the current world. The vertical
    /// bounds are from the authored level header. A successful cast supplies
    /// the sole origin of this blast; a qualified miss spends the slot.
    pub fn collect_air_strike_blast(
        &self,
        attacker: u32,
        xz: [f32; 2],
        vertical_bounds: [f32; 2],
        mut blast: InstantaneousBlast,
        collision: BlastCollision<'_>,
    ) -> Result<Option<DamageWork>, &'static str> {
        if !BlastSource::AirStrike.supported_mode(self.match_profile().game_type)
            || !blast.origins.is_empty()
            || blast.material_penetration != 0.
        {
            return Err("invalid_air_strike_blast_context");
        }
        let Some(origin) = self.command_impact(xz, vertical_bounds, 100., collision)? else {
            return Ok(None);
        };
        blast.origins.push(origin);
        self.collect_domination_blast(BlastSource::AirStrike, attacker, blast, collision)
            .map(Some)
    }

    pub(crate) fn collect_command_bomblet(
        &self,
        attacker: u32,
        sample: BombletSample,
        vertical_bounds: [f32; 2],
        padded: bool,
        collision: BlastCollision<'_>,
    ) -> Result<Option<DamageWork>, &'static str> {
        let pad = if padded { 100. } else { 0. };
        let Some(origin) = self.command_impact(sample.xz, vertical_bounds, pad, collision)? else {
            return Ok(None);
        };
        let rule = sample.blast;
        let falloff = BlastFalloff {
            inner_radius: rule.inner_radius,
            outer_radius: rule.outer_radius,
            inner_scale: rule.inner_scale,
            outer_scale: rule.outer_scale,
        };
        self.collect_domination_blast(
            BlastSource::CommandBomblet(falloff),
            attacker,
            InstantaneousBlast {
                origins: vec![origin],
                inner_radius: rule.inner_radius,
                outer_radius: rule.outer_radius,
                damage: rule.damage,
                material_penetration: rule.material_penetration,
            },
            collision,
        )
        .map(Some)
    }

    fn command_impact(
        &self,
        xz: [f32; 2],
        vertical_bounds: [f32; 2],
        pad: f32,
        collision: BlastCollision<'_>,
    ) -> Result<Option<[f32; 3]>, &'static str> {
        let scene = self
            .collision_world
            .as_ref()
            .filter(|scene| {
                scene.stats().triangles != 0 && scene.metadata().map_name == self.map_name()
            })
            .ok_or("air_strike_static_collision_required")?;
        let snapshot = self.dynamic_collision_snapshot(collision.filter_rows[6])?;
        collision.strike_impact(scene, &snapshot, xz, vertical_bounds, pad)
    }

    pub(crate) fn collect_guided_artillery_blast(
        &self,
        attacker: u32,
        origin: [f32; 3],
        radius: f32,
        collision: BlastCollision<'_>,
    ) -> Result<DamageWork, &'static str> {
        self.collect_domination_blast(
            BlastSource::GuidedArtillery,
            attacker,
            InstantaneousBlast {
                origins: vec![origin],
                inner_radius: radius,
                outer_radius: radius,
                damage: 10_000.,
                material_penetration: 0.,
            },
            collision,
        )
    }

    pub(crate) fn collect_gas_pulse_characters(
        &self,
        volume: GasVolume,
        collision: BlastCollision<'_>,
    ) -> Result<Vec<BlastObservation>, &'static str> {
        let DamageWork::Blast { observations, .. } = self.collect_domination_blast(
            BlastSource::GasPulse(volume.falloff),
            u32::MAX,
            InstantaneousBlast {
                origins: vec![volume.origin],
                inner_radius: volume.falloff.inner_radius,
                outer_radius: volume.falloff.outer_radius,
                damage: volume.damage_per_second * 0.5,
                material_penetration: 0.,
            },
            collision,
        )?
        else {
            unreachable!("radial collector always produces blast work");
        };
        Ok(observations)
    }

    /// Collect radial pairs through current Engine bodies and the source-bound world.
    pub fn collect_domination_blast(
        &self,
        source: BlastSource,
        attacker: u32,
        blast: InstantaneousBlast,
        collision: BlastCollision<'_>,
    ) -> Result<DamageWork, &'static str> {
        let filter_rows = collision.filter_rows;
        let limits = collision.limits;
        if !source.supported_mode(self.match_profile().game_type)
            || (source.is_command() && blast.material_penetration != 0.)
            || matches!(
                source,
                BlastSource::Structural { objective_id: 0 }
                    | BlastSource::Hospital { body_id: 0 }
                    | BlastSource::Ground { body_id: 0 }
                    | BlastSource::Helo { body_id: 0 }
            )
            || blast.origins.is_empty()
            || blast.origins.len() > 2
            || ![
                blast.damage,
                blast.inner_radius,
                blast.outer_radius,
                blast.material_penetration,
            ]
            .into_iter()
            .all(|value| value.is_finite() && value >= 0.0)
            || blast.inner_radius > blast.outer_radius
            || !blast
                .origins
                .iter()
                .flatten()
                .all(|value| value.is_finite())
        {
            return Err("invalid_domination_structural_damage_context");
        }
        let mut observations = Vec::new();
        if blast.outer_radius == 0.0 || blast.damage == 0.0 {
            return Ok(DamageWork::Blast {
                source,
                attacker,
                blast,
                observations,
            });
        }
        let snapshot = self.dynamic_collision_snapshot(filter_rows[6])?;
        let scene = if source.needs_cover_ray() {
            Some(
                self.collision_world
                    .as_ref()
                    .filter(|scene| {
                        scene.stats().triangles != 0 && scene.metadata().map_name == self.map_name()
                    })
                    .ok_or("domination_structural_static_collision_required")?,
            )
        } else {
            None
        };
        let world = snapshot.context().0.world_id;
        let sphere = Geometry::Sphere {
            radius: f64::from(blast.outer_radius),
        };
        let mut budget = limits.max_support_points;
        let mut static_budget = 1_000_000;
        for (origin_index, origin) in blast.origins.iter().enumerate() {
            for body in snapshot
                .bodies()
                .iter()
                .filter(|body| source.accepts_body(body.target.kind))
            {
                if source.vehicle_body().map(u64::from) == Some(body.target.key.id)
                    || filter_rows[6] & (1 << body.layer) == 0
                    || !geometry::penetrates(
                        &sphere,
                        Pose::at(origin.map(f64::from)),
                        &body.shape,
                        body.pose,
                        limits,
                        &mut budget,
                    )?
                {
                    continue;
                }
                let Some((target, position)) = self.domination_blast_target(body.target, world)?
                else {
                    continue;
                };
                let (_, distance_squared) = separation(*origin, position)?;
                if source.scale(&blast, distance_squared.sqrt())? == 0.0 {
                    continue;
                }
                if let Some(scene) = scene
                    && !collision.clear_path(
                        scene,
                        &snapshot,
                        BlastRay {
                            start: origin.map(f64::from),
                            end: position.map(f64::from),
                            excludes: [
                                source.vehicle_body().map(u64::from),
                                Some(body.target.key.id),
                            ],
                            penetration: blast.material_penetration,
                            zero_penetration_strike: source.is_command(),
                            filter_static: source.is_command(),
                        },
                        &mut static_budget,
                    )?
                {
                    continue;
                }
                observations.push(BlastObservation {
                    target,
                    origin_index,
                    sample_position: position,
                    line_of_damage: true,
                });
            }
            if !matches!(source, BlastSource::GasPulse(_)) && filter_rows[6] & (1 << 22) != 0 {
                for (&index, surface) in &collision.sequences.helos {
                    let target_id = u64::from(0x0280_0000 | u32::from(index));
                    if source.vehicle_body().map(u64::from) == Some(target_id) {
                        continue;
                    }
                    let position = surface.pose.translation.map(|component| component as f32);
                    let (_, distance_squared) = separation(*origin, position)?;
                    if source.scale(&blast, distance_squared.sqrt())? == 0.0 {
                        continue;
                    }
                    let local = surface.local(origin.map(f64::from));
                    if !surface.overlaps_sphere(local, &sphere, limits, &mut budget)? {
                        continue;
                    }
                    let clear_path = match scene {
                        Some(scene) => collision.clear_path(
                            scene,
                            &snapshot,
                            BlastRay {
                                start: origin.map(f64::from),
                                end: surface.pose.translation,
                                excludes: [source.vehicle_body().map(u64::from), Some(target_id)],
                                penetration: blast.material_penetration,
                                zero_penetration_strike: source.is_command(),
                                filter_static: source.is_command(),
                            },
                            &mut static_budget,
                        )?,
                        None => true,
                    };
                    if clear_path {
                        observations.push(BlastObservation {
                            target: BlastTarget::Helo { index, world },
                            origin_index,
                            sample_position: position,
                            line_of_damage: true,
                        });
                    }
                }
            }
        }
        Ok(DamageWork::Blast {
            source,
            attacker,
            blast,
            observations,
        })
    }

    fn domination_blast_target(
        &self,
        body: acquisition_volume_damage::Target,
        world: u32,
    ) -> Result<Option<(BlastTarget, [f32; 3])>, &'static str> {
        let id = u32::try_from(body.key.id).map_err(|_| "invalid_domination_structural_target")?;
        match body.kind {
            TargetKind::Character => {
                let target = character::Target {
                    character_id: id,
                    generation: body.key.generation,
                    world,
                };
                let actor = self.active.iter().chain(self.peers.values()).find(|actor| {
                    actor.reservation.world == world && actor.reservation.player == id as u16
                });
                let (mut position, stance) = if let Some(actor) = actor {
                    if actor.is_mounted() || actor.life.damage_health().is_none() {
                        return Ok(None);
                    }
                    // Before the first owner state, the spawned on-foot
                    // authority publishes standing stance zero.
                    let stance = actor.owner_fields.state.map_or(0, |state| state.stance);
                    (self.acquisition_character_position(target)?, stance)
                } else {
                    let bot = self
                        .bot_registry
                        .bots
                        .get(&(id as u16))
                        .ok_or("domination_structural_blast_unqualified_stance")?;
                    if bot.health == 0 {
                        return Ok(None);
                    }
                    let runtime = bot
                        .target
                        .runtime
                        .filter(|runtime| runtime.generation == body.key.generation)
                        .ok_or("domination_structural_blast_bot_pose_missing")?;
                    // The server bot authority publishes standing stance zero.
                    (runtime.physical_position().map(|value| value as f32), 0)
                };
                position[1] += [2.0, 0.8, 0.3, 1.8]
                    .get(usize::from(stance))
                    .ok_or("domination_structural_blast_unqualified_stance")?;
                Ok(Some((BlastTarget::Character(target), position)))
            }
            TargetKind::GroundVehicle => {
                let key = ground::Key {
                    index: id as u16,
                    generation: body.key.generation,
                };
                if key.object_id() != id {
                    return Err("invalid_domination_ground_blast_identity");
                }
                let vehicle = self
                    .ground_runtime
                    .as_ref()
                    .and_then(|runtime| runtime.get(key.index))
                    .filter(|vehicle| vehicle.key == key)
                    .ok_or("stale_domination_ground_blast_identity")?;
                if vehicle.state != 2
                    || vehicle.body != ground::Body::Operational
                    || vehicle.is_dead()
                    || vehicle.is_dying()
                {
                    return Ok(None);
                }
                let p = vehicle.pose.position;
                Ok(Some((
                    BlastTarget::Ground(GroundBlastTarget {
                        key,
                        team: body.team,
                        world,
                    }),
                    [p.x, p.y, p.z],
                )))
            }
            TargetKind::Mine | TargetKind::Other => {
                Err("domination_structural_blast_non_character_route_unqualified")
            }
        }
    }

    /// Clone both supported effect domains. The outer objective owner records
    /// its candidate, checks the character transaction fence, and installs both.
    pub fn prepare_domination_structural_damage(
        &mut self,
        previous_area_state: &AreaDamageState,
        native: &BTreeMap<u32, NativeState>,
        now: Duration,
        work: &[DamageWork],
    ) -> Result<PreparedDamage, &'static str> {
        let mut area_state = previous_area_state.clone();
        let character_candidate =
            self.prepare_acquisition_character_callback(native, now, |engine, native| {
                engine.stage_domination_structural_damage(&mut area_state, native, now, work)
            })?;
        Ok(PreparedDamage {
            character_candidate,
            area_state,
        })
    }

    /// Available to the outer Domination callback's one character transaction
    /// so objective, hospital and blast effects observe one evolving candidate.
    pub(crate) fn stage_domination_structural_damage(
        &mut self,
        state: &mut AreaDamageState,
        native: &mut BTreeMap<u32, NativeState>,
        now: Duration,
        work: &[DamageWork],
    ) -> Result<(Vec<Value>, Vec<OutboundMessage>), &'static str> {
        let commit = self.stage_domination_damage_with_ground(state, native, now, work, None)?;
        Ok((commit.audit, commit.messages))
    }

    /// Stage a blast in collector order, including the caller-owned detached ground world.
    pub(crate) fn stage_domination_damage_with_ground(
        &mut self,
        state: &mut AreaDamageState,
        native: &mut BTreeMap<u32, NativeState>,
        now: Duration,
        work: &[DamageWork],
        ground_policy: Option<&acquisition_ground_damage::Policy>,
    ) -> Result<DamageCommit, &'static str> {
        let game_type = self.match_profile().game_type;
        if !matches!(game_type, 1 | 2)
            || work.len() > 128
            || work.iter().any(|operation| match operation {
                DamageWork::Area(_) => false,
                DamageWork::Blast { source, blast, .. } => {
                    !source.supported_mode(game_type)
                        || matches!(source, BlastSource::GasPulse(_))
                        || (source.is_command() && blast.material_penetration != 0.)
                }
            })
        {
            return Err("invalid_domination_structural_damage_context");
        }
        if state.round != Some(self.round_number) {
            *state = AreaDamageState {
                round: Some(self.round_number),
                ..Default::default()
            };
        }
        let mut output = DamageOutput {
            now,
            native,
            audit: Vec::new(),
            messages: Vec::new(),
            ground_policy,
            ground_changes: BTreeSet::new(),
            helo_doses: Vec::new(),
        };
        for operation in work {
            match operation {
                DamageWork::Area(update) => {
                    self.stage_structural_area(*update, state, &mut output)?
                }
                DamageWork::Blast {
                    source,
                    attacker,
                    blast,
                    observations,
                } => {
                    self.stage_structural_blast(
                        *source,
                        *attacker,
                        blast,
                        observations,
                        &mut output,
                    )?;
                }
            }
        }
        Ok(DamageCommit {
            audit: output.audit,
            messages: output.messages,
            changed_ground: output.ground_changes,
            helo_doses: output.helo_doses,
        })
    }

    fn stage_structural_area(
        &mut self,
        update: AreaUpdate,
        state: &mut AreaDamageState,
        output: &mut DamageOutput<'_>,
    ) -> Result<(), &'static str> {
        let Some(slots) = state.next_slots(
            update.objective_index,
            update.updated_state,
            update.properties,
        ) else {
            return Ok(());
        };
        if !update
            .objective_position
            .iter()
            .all(|axis| axis.is_finite())
        {
            return Err("invalid_structural_area_origin");
        }
        output.audit.push(
            json!({"op":"domination_structural_area_scan", "objective":update.objective_index,
            "first_slot":slots.start,"end_slot":slots.end}),
        );
        let radius = update.properties.character_damage_over_time_radius;
        for slot in slots {
            let Some(target) = self.structural_area_target(update.world, slot)? else {
                continue;
            };
            if state.cooling_down(slot, target.generation, output.now) {
                continue;
            }
            if self.structural_area_hidden_ground_seat(target)? {
                continue;
            }
            let position = self.structural_area_position(target)?;
            let (displacement, distance_squared) = separation(update.objective_position, position)?;
            if distance_squared >= radius * radius {
                continue;
            }
            let native = output
                .native
                .get(&target.character_id)
                .ok_or("domination_structural_area_protection_missing")?;
            let protection = native
                .invulnerability
                .ok_or("domination_structural_area_protection_missing")?;
            if protection != 0 {
                continue;
            }
            state.cooldowns.insert(
                slot,
                (
                    target.generation,
                    output.now.saturating_add(AREA_DAMAGE_COOLDOWN),
                ),
            );
            output.apply(
                self,
                target,
                character::Request::StructuralArea {
                    raw_hp: update.properties.character_damage_over_time_value,
                    from_direction: direction(displacement, distance_squared),
                },
            )?;
        }
        Ok(())
    }

    fn structural_area_target(
        &self,
        world: u32,
        slot: u16,
    ) -> Result<Option<character::Target>, &'static str> {
        let character_id = 0x0380_0000 | u32::from(slot);
        if let Some(actor) = self
            .active
            .iter()
            .chain(self.peers.values())
            .find(|actor| actor.reservation.world == world && actor.reservation.player == slot)
        {
            let Some((generation, _)) = actor.life.damage_health() else {
                return Ok(None);
            };
            return Ok(Some(character::Target {
                character_id,
                generation,
                world,
            }));
        }
        let Some(bot) = self.bot_registry.bots.get(&slot) else {
            return Ok(None);
        };
        if bot.health == 0 {
            return Ok(None);
        }
        let runtime = bot
            .target
            .runtime
            .ok_or("domination_structural_area_bot_pose_missing")?;
        Ok(Some(character::Target {
            character_id,
            generation: runtime.generation,
            world,
        }))
    }

    /// Retail skips a resolved, occupied hidden seat before its radius test.
    /// Ground seat authority is server-owned; visibility comes from its source
    /// catalog, independently of vehicle kind or the actor's cached position.
    fn structural_area_hidden_ground_seat(
        &self,
        target: character::Target,
    ) -> Result<bool, &'static str> {
        let Some(actor) = self.active.iter().chain(self.peers.values()).find(|actor| {
            actor.reservation.world == target.world
                && actor.reservation.player == target.character_id as u16
        }) else {
            return Ok(false);
        };
        let Some(seat) = actor.ground_seat else {
            return Ok(false);
        };
        let catalog = self
            .ground_vehicles
            .as_ref()
            .ok_or("structural_area_ground_catalog_required")?;
        let vehicle = catalog
            .vehicle(seat.vehicle_index)
            .ok_or("structural_area_ground_vehicle_missing")?;
        let supported = vehicle.passenger_seats.contains(&seat.seat)
            || catalog
                .driver_seat(seat.vehicle_index)
                .is_some_and(|driver| driver.seat == seat.seat)
            || (seat.seat == 1 && catalog.turret_policy(seat.vehicle_index).is_some());
        if !supported {
            return Err("structural_area_ground_seat_unbound");
        }
        if self
            .ground_runtime
            .as_ref()
            .and_then(|runtime| runtime.get(seat.vehicle_index))
            .is_none()
            || self
                .active
                .iter()
                .chain(self.peers.values())
                .filter(|other| {
                    other.reservation.world == target.world && other.ground_seat == Some(seat)
                })
                .count()
                != 1
        {
            return Err("structural_area_ground_occupant_unbound");
        }
        catalog
            .seat_occupant_visible(seat.vehicle_index, seat.seat)
            .map(|visible| !visible)
            .ok_or("structural_area_ground_seat_visibility_required")
    }

    fn structural_area_position(
        &self,
        target: character::Target,
    ) -> Result<[f32; 3], &'static str> {
        if let Some(actor) = self.active.iter().chain(self.peers.values()).find(|actor| {
            actor.reservation.world == target.world
                && actor.reservation.player == target.character_id as u16
        }) {
            if let (Some(index), Some(seat)) = (actor.mounted_bunker, actor.mounted_seat) {
                let position = self.domination_bunker_seat_position(index, seat)?;
                return Ok([position.x, position.y, position.z]);
            }
            if actor.helo_seat.is_some() {
                let position = Self::domination_helo_passenger_position(actor)?;
                return Ok([position.x, position.y, position.z]);
            }
            if actor.is_mounted() && actor.ground_seat.is_none() {
                return Err("domination_structural_area_mount_pose_unqualified");
            }
        }
        // The native mounted character update copies the associated ground
        // vehicle's root into the character position sampled by this callback.
        self.acquisition_character_position(target)
    }

    fn stage_structural_blast(
        &mut self,
        source: BlastSource,
        attacker: u32,
        blast: &InstantaneousBlast,
        observations: &[BlastObservation],
        output: &mut DamageOutput<'_>,
    ) -> Result<(), &'static str> {
        if observations.len() > 512 || blast.origins.len() > 2 {
            return Err("domination_structural_blast_observation_limit");
        }
        for observation in observations {
            let origin = *blast
                .origins
                .get(observation.origin_index)
                .ok_or("domination_structural_blast_origin_missing")?;
            let (displacement, distance_squared) = separation(origin, observation.sample_position)?;
            let scale = source.scale(blast, distance_squared.sqrt())?;
            if scale <= 0.0 || blast.damage == 0.0 || !observation.line_of_damage {
                continue;
            }
            let target = match observation.target {
                BlastTarget::Character(target) => target,
                BlastTarget::Ground(target) => {
                    output.apply_ground(self, target, attacker, blast.damage, scale)?;
                    continue;
                }
                BlastTarget::Helo { index, world } => {
                    output
                        .helo_doses
                        .push(crate::domination_helo_damage::RadialDose {
                            index,
                            world,
                            attacker,
                            source_team: None,
                            damage: item_catalog::TargetDamageParameters {
                                amount: blast.damage,
                                armor_pierce: 0.,
                                armor_redux: 0.,
                            },
                            scale,
                            source_object: u32::MAX,
                        });
                    continue;
                }
            };
            let raw_hp = blast.damage * scale;
            let from_direction = direction(displacement, distance_squared);
            let request = match source {
                BlastSource::Structural { objective_id } => {
                    character::Request::StructuralExplosion {
                        raw_hp,
                        from_direction,
                        attacker,
                        objective_id,
                    }
                }
                BlastSource::Hospital { .. }
                | BlastSource::Ground { .. }
                | BlastSource::Helo { .. } => character::Request::VehicleExplosion {
                    raw_hp,
                    from_direction,
                    attacker,
                },
                BlastSource::HeloGrenade => return Err("invalid_domination_helo_grenade_target"),
                BlastSource::GuidedArtillery | BlastSource::GasPulse(_) => {
                    return Err("invalid_command_blast_character_target");
                }
                BlastSource::AirStrike | BlastSource::CommandBomblet(_) => {
                    character::Request::AirStrikeExplosion {
                        raw_hp,
                        from_direction,
                        attacker,
                    }
                }
            };
            output.apply(self, target, request)?;
        }
        Ok(())
    }
}

struct DamageOutput<'a> {
    now: Duration,
    native: &'a mut BTreeMap<u32, NativeState>,
    audit: Vec<Value>,
    messages: Vec<OutboundMessage>,
    ground_policy: Option<&'a acquisition_ground_damage::Policy>,
    ground_changes: BTreeSet<u16>,
    helo_doses: Vec<crate::domination_helo_damage::RadialDose>,
}

impl DamageOutput<'_> {
    fn apply<W: Write>(
        &mut self,
        engine: &mut Engine<W>,
        target: character::Target,
        request: character::Request,
    ) -> Result<(), &'static str> {
        let state = self
            .native
            .get_mut(&target.character_id)
            .ok_or("domination_structural_character_state_missing")?;
        let plan = engine.plan_acquisition_character_effect(target, request, *state, self.now)?;
        self.audit.push(plan.audit().clone());
        self.messages
            .extend(engine.commit_acquisition_character_effect(plan, state)?);
        Ok(())
    }

    fn apply_ground<W: Write>(
        &mut self,
        engine: &mut Engine<W>,
        target: GroundBlastTarget,
        attacker: u32,
        raw_hp: f32,
        scale: f32,
    ) -> Result<(), &'static str> {
        let GroundBlastTarget { key, team, world } = target;
        let creation = engine
            .current_ground_vehicle(key.index)
            .ok_or("domination_ground_blast_creation_missing")?;
        let friendly = engine.character_team_in_world(attacker, world) == Some(team);
        let amount = self
            .ground_policy
            .ok_or("domination_ground_damage_policy_required")?
            .blast_amount(
                &creation,
                item_catalog::TargetDamageParameters {
                    amount: raw_hp,
                    armor_pierce: 0.0,
                    armor_redux: 0.0,
                },
                scale,
                friendly,
            )?;
        let changes = engine
            .ground_runtime
            .as_mut()
            .ok_or("domination_ground_blast_runtime_missing")?
            .apply_explosion_damage(key, amount, attacker)?;
        if !changes.is_empty() {
            self.ground_changes.insert(key.index);
            self.audit
                .push(json!({"op":"domination_ground_radial_damage",
                "object":key.object_id(),"actor":attacker,"amount":amount}));
        }
        Ok(())
    }
}

/// Retail 7a23d4 evaluates after the sphere-to-shape overlap. A sampled
/// point outside the sphere can still receive the authored outer scale.
pub(crate) fn native_command_falloff(
    falloff: BlastFalloff,
    distance: f32,
) -> Result<f32, &'static str> {
    let BlastFalloff {
        inner_radius,
        inner_scale,
        outer_radius,
        outer_scale,
    } = falloff;
    if !distance.is_finite()
        || distance < 0.
        || !inner_radius.is_finite()
        || !outer_radius.is_finite()
        || inner_radius <= 0.
        || outer_radius < inner_radius
        || !(0. ..=1.).contains(&inner_scale)
        || !(0. ..=1.).contains(&outer_scale)
    {
        return Err("invalid_command_blast_falloff");
    }
    if distance <= inner_radius {
        return Ok((inner_scale - 1.).mul_add(distance / inner_radius, 1.));
    }
    if distance >= outer_radius {
        return Ok(outer_scale);
    }
    let width = outer_radius - inner_radius;
    let fraction = if width >= 0.00001 {
        (distance - inner_radius) / width
    } else {
        0.
    };
    Ok((outer_scale - inner_scale).mul_add(fraction, inner_scale))
}

fn separation(origin: [f32; 3], point: [f32; 3]) -> Result<([f32; 3], f32), &'static str> {
    let displacement: [f32; 3] = std::array::from_fn(|axis| point[axis] - origin[axis]);
    let [x, y, z] = displacement;
    let distance_squared = (x * x + y * y) + z * z;
    if !distance_squared.is_finite() {
        return Err("invalid_domination_structural_damage_distance");
    }
    Ok((displacement, distance_squared))
}

fn can_penetrate(
    hits: &mut [collision::MaterialRayHit],
    initial: f32,
    costs: &BTreeMap<u8, f32>,
) -> Result<bool, &'static str> {
    if !initial.is_finite() || initial < 0.0 {
        return Err("invalid_domination_material_penetration");
    }
    hits.sort_by(|left, right| left.fraction.total_cmp(&right.fraction));
    let mut remaining = 1.0_f32;
    for hit in hits {
        // The original material table reserves its final slot for collision-null.
        if hit.material_index == Some(u8::MAX) {
            return Ok(false);
        }
        let cost = hit
            .material_index
            .and_then(|index| costs.get(&index))
            .copied()
            .ok_or("domination_structural_blast_material_penetration_required")?;
        let available = initial * remaining;
        if cost == -1.0 || available <= cost {
            return Ok(false);
        }
        if hit.front {
            remaining *= 1.0 - cost / available;
        }
    }
    Ok(true)
}

fn direction(displacement: [f32; 3], distance_squared: f32) -> [f32; 3] {
    if distance_squared == 0.0 {
        [0.0; 3]
    } else {
        let distance = distance_squared.sqrt();
        displacement.map(|axis| axis / distance)
    }
}

impl SequencePackBinding {
    fn projectile_binding(&self, mesh: &collision::CollisionWorld) -> Option<SequenceBinding> {
        Some(SequenceBinding {
            source_sha256: self.source_sha256.clone(),
            neutral_sha256: self.neutral_sha256.clone(),
            collision_tag: self.collision_tag.try_into().ok()?,
            part_ids: projectile_part_ids(mesh),
        })
    }
}

/// Neutral shape resources extracted from the placed map. Every pack is
/// validated against its source digest before being used by a sequence.
pub struct SequenceCollisionAssets {
    packs: BTreeMap<String, Vec<CollisionPack>>,
    parents: BTreeMap<String, BTreeMap<String, ParentReference>>,
    absent: BTreeSet<String>,
}
struct CollisionPack {
    binding: SequencePackBinding,
    mesh: collision::CollisionWorld,
}
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SequencePackBinding {
    pub source_sha256: String,
    pub neutral_sha256: String,
    pub parent_token: String,
    pub collision_tag: i32,
    pub material_indices: Vec<u8>,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ParentReference {
    source_sha256: String,
    pose: Pose,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CollisionIndex {
    schema: String,
    map: String,
    level_archive_sha256: String,
    physics_sha256: String,
    geometry_sha256: String,
    resources: BTreeMap<String, Vec<SequencePackBinding>>,
    parent_reference_poses: BTreeMap<String, BTreeMap<String, ParentReference>>,
}
#[derive(Clone, Default)]
pub struct SequenceCollisionState {
    active: BTreeMap<(u16, u8), Vec<SequenceSurface>>,
    hulks: BTreeMap<u16, SequenceSurface>,
    hospitals: BTreeMap<u16, SequenceSurface>,
    helos: BTreeMap<u16, SequenceSurface>,
}
#[derive(Clone)]
pub(crate) struct SequenceSurface {
    projectile: Option<SurfaceBinding>,
    pose: Pose,
    mesh: collision::CollisionWorld,
}

pub struct HospitalHulkAssets {
    by_faction: BTreeMap<u8, HospitalHulk>,
}

struct HospitalHulk {
    layer: u8,
    mesh: collision::CollisionWorld,
    operational: Option<collision::CollisionWorld>,
    projectile: Option<BodyBinding>,
    damage: HospitalDestruction,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct HospitalOperational {
    body_local: serde_json::Value,
    material_index: u8,
    collision_tag: u8,
    layer: u8,
    motion_type: u8,
    packfile_sha256: String,
    source_member_sha256: String,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct HospitalDestruction {
    raw_hp: f32,
    half_radius: f32,
    outer_radius: f32,
    material_penetration: f32,
    hulk_fade_seconds: f32,
}

impl HospitalHulkAssets {
    pub fn from_json(
        raw: &[u8],
        map: &str,
        retail_elf_sha256: &str,
        structures_sha256: &str,
    ) -> Result<Self, &'static str> {
        #[derive(serde::Deserialize)]
        struct Document {
            schema: String,
            main_archive_sha256: String,
            retail_elf_sha256: String,
            structures_sha256: String,
            factions: BTreeMap<u8, Faction>,
        }
        #[derive(serde::Deserialize)]
        struct Faction {
            body_local: serde_json::Value,
            destruction_damage: HospitalDestruction,
            collision_tag: u8,
            material_indices: Vec<u8>,
            initial_rigid_body_filter: u8,
            motion_type: u8,
            collision_margin: f64,
            extrusion_xyz: [f64; 3],
            packfile_sha256: String,
            source_member_sha256: String,
            operational: Option<HospitalOperational>,
        }
        if raw.len() > 4 * 1024 * 1024 || map.is_empty() {
            return Err("invalid_domination_hulk_asset_size_or_map");
        }
        let doc: Document =
            serde_json::from_slice(raw).map_err(|_| "invalid_domination_hulk_assets")?;
        let digest = |value: &str| {
            value.len() == 64
                && value
                    .bytes()
                    .all(|v| v.is_ascii_hexdigit() && !v.is_ascii_uppercase())
        };
        if doc.schema != "mag_private_retail_hospital_hulks_v1"
            || doc.main_archive_sha256
                != "a2e8345cd9f5c650ac22544096ba83e8c9e6f0fffd0ef69eb8791fb265f9baad"
            || !digest(retail_elf_sha256)
            || !digest(structures_sha256)
            || doc.retail_elf_sha256 != retail_elf_sha256
            || doc.structures_sha256 != structures_sha256
            || !doc.factions.keys().copied().eq(0..3)
        {
            return Err("domination_hulk_asset_source_mismatch");
        }
        use sha2::Digest;
        let asset_sha256 = format!("{:x}", sha2::Sha256::digest(raw));
        let mut by_faction = BTreeMap::new();
        for (faction, mut row) in doc.factions {
            if row.collision_tag != 9
                || row.material_indices.len() != 1
                || row.initial_rigid_body_filter != 17
                || row.motion_type != 6
                || row.collision_margin != 0.0
                || row.extrusion_xyz != [0.0; 3]
                || !digest(&row.packfile_sha256)
                || !digest(&row.source_member_sha256)
            {
                return Err("unqualified_domination_hulk_shape");
            }
            let damage = row.destruction_damage;
            if ![
                damage.raw_hp,
                damage.half_radius,
                damage.outer_radius,
                damage.material_penetration,
            ]
            .into_iter()
            .all(|value| value.is_finite() && value > 0.0)
                || damage.outer_radius < damage.half_radius
                || damage.hulk_fade_seconds != 0.0
            {
                return Err("invalid_domination_hospital_destruction");
            }
            let instance = row
                .body_local
                .as_object_mut()
                .ok_or("invalid_domination_hulk_shape")?;
            instance.insert("material_index".into(), row.material_indices[0].into());
            let document = json!({
                "format": "mag_collision_world",
                "schema_version": 1,
                "coordinate_space": "world",
                "metadata": {
                    "map_name": map, "build": "retail-bcus98110-01.00",
                    "coverage": "provisional",
                    "provenance": [row.packfile_sha256, row.source_member_sha256],
                    "unknowns": [],
                },
                "instances": [row.body_local]
            });
            let mesh = collision::CollisionWorld::from_bytes(
                &serde_json::to_vec(&document).map_err(|_| "invalid_domination_hulk_shape")?,
                collision::CollisionLimits::default(),
            )
            .map_err(|_| "invalid_domination_hulk_mesh")?;
            if mesh.stats().triangles == 0 {
                return Err("empty_domination_hulk_mesh");
            }
            let mut projectile = row.operational.as_ref().map(|body| BodyBinding {
                asset_sha256: asset_sha256.clone(),
                faction,
                packfile_sha256: body.packfile_sha256.clone(),
                part_ids: Vec::new(),
            });
            let operational = row.operational.map(|mut body| {
                if body.layer != 22 || body.motion_type != 6 || body.collision_tag != 9
                    || !digest(&body.packfile_sha256) || !digest(&body.source_member_sha256)
                    || body.material_index > 26 && body.material_index != u8::MAX
                {
                    return Err("unqualified_domination_hospital_body");
                }
                body.body_local.as_object_mut()
                    .ok_or("invalid_domination_hospital_body")?
                    .insert("material_index".into(), body.material_index.into());
                let document = json!({
                    "format": "mag_collision_world", "schema_version": 1, "coordinate_space": "world",
                    "metadata": {
                        "map_name": map, "build": "retail-bcus98110-01.00",
                        "coverage": "provisional",
                        "provenance": [body.packfile_sha256, body.source_member_sha256],
                        "unknowns": [],
                    },
                    "instances": [body.body_local],
                });
                let mesh = collision::CollisionWorld::from_bytes(
                    &serde_json::to_vec(&document).map_err(|_| "invalid_domination_hospital_body")?,
                    collision::CollisionLimits::default(),
                ).map_err(|_| "invalid_domination_hospital_mesh")?;
                if mesh.stats().triangles == 0 {
                    return Err("empty_domination_hospital_mesh");
                }
                Ok(mesh)
            }).transpose()?;
            if let (Some(binding), Some(mesh)) = (&mut projectile, &operational) {
                binding.part_ids = projectile_part_ids(mesh);
            }
            by_faction.insert(
                faction,
                HospitalHulk {
                    layer: row.initial_rigid_body_filter,
                    mesh,
                    operational,
                    projectile,
                    damage,
                },
            );
        }
        Ok(Self { by_faction })
    }

    pub fn hospital_blast(
        &self,
        faction: u8,
        origin: [f32; 3],
    ) -> Result<InstantaneousBlast, &'static str> {
        let damage = &self
            .by_faction
            .get(&faction)
            .ok_or("unbound_domination_hospital_hulk")?
            .damage;
        if !origin.into_iter().all(f32::is_finite) {
            return Err("invalid_domination_hospital_blast_origin");
        }
        Ok(InstantaneousBlast {
            origins: vec![origin],
            inner_radius: damage.half_radius,
            outer_radius: damage.outer_radius,
            damage: damage.raw_hp,
            material_penetration: damage.material_penetration,
        })
    }
}

/// Original keyframed transport meshes, bound to the selected fleet source.
pub struct HeloCollisionAssets {
    projectile: BTreeMap<u8, BodyBinding>,
    by_faction: BTreeMap<u8, collision::CollisionWorld>,
}

impl HeloCollisionAssets {
    pub fn from_json(
        raw: &[u8],
        map: &str,
        retail_elf_sha256: &str,
        models_receipt_sha256: &str,
    ) -> Result<Self, &'static str> {
        use sha2::Digest;

        #[derive(serde::Deserialize)]
        struct Document {
            schema: String,
            main_archive_sha256: String,
            game_archive_sha256: String,
            retail_elf_sha256: String,
            collision_layer: u8,
            motion_type: u8,
            variants: BTreeMap<String, Variant>,
        }
        #[derive(serde::Deserialize)]
        struct Variant {
            body_local: Vec<serde_json::Value>,
            collision_tag: u8,
            material_indices: Vec<u8>,
            packfile_sha256: String,
            physics_member_sha256: String,
        }

        if raw.len() > 4 * 1024 * 1024
            || map.is_empty()
            || format!("{:x}", sha2::Sha256::digest(raw)) != models_receipt_sha256
        {
            return Err("domination_helo_mesh_source_mismatch");
        }
        let doc: Document =
            serde_json::from_slice(raw).map_err(|_| "invalid_domination_helo_mesh_assets")?;
        if doc.schema != "mag_private_retail_domination_helo_models_v1"
            || doc.main_archive_sha256
                != "a2e8345cd9f5c650ac22544096ba83e8c9e6f0fffd0ef69eb8791fb265f9baad"
            || doc.game_archive_sha256
                != "b49c9435be729832029ad7019874d478080dc9764ebde1809b2851d353bc769f"
            || doc.retail_elf_sha256 != retail_elf_sha256
            || doc.collision_layer != 22
            || doc.motion_type != 6
            || doc.variants.len() != 3
        {
            return Err("domination_helo_mesh_source_mismatch");
        }
        let mut projectile = BTreeMap::new();
        let mut by_faction = BTreeMap::new();
        for (faction, name) in [(0, "Valor"), (1, "Raven"), (2, "SVER")] {
            let row = doc
                .variants
                .get(name)
                .ok_or("unbound_domination_helo_faction_mesh")?;
            if row.collision_tag != 9
                || !(1..=2).contains(&row.body_local.len())
                || row.body_local.len() != row.material_indices.len()
            {
                return Err("unqualified_domination_helo_mesh");
            }
            let mut instances = row.body_local.clone();
            for (instance, material) in instances.iter_mut().zip(&row.material_indices) {
                let fields = instance
                    .as_object_mut()
                    .ok_or("invalid_domination_helo_mesh")?;
                let shape_id = fields
                    .remove("shape_id")
                    .ok_or("invalid_domination_helo_mesh")?;
                fields.insert("id".into(), shape_id);
                fields.insert("material_index".into(), u32::from(*material).into());
            }
            let document = json!({
                "format": "mag_collision_world",
                "schema_version": 1,
                "coordinate_space": "world",
                "metadata": {
                    "map_name": map, "build": "retail-bcus98110-01.00",
                    "coverage": "provisional",
                    "provenance": [row.packfile_sha256, row.physics_member_sha256],
                    "unknowns": [],
                },
                "instances": instances,
            });
            let mesh = collision::CollisionWorld::from_bytes(
                &serde_json::to_vec(&document).map_err(|_| "invalid_domination_helo_mesh")?,
                collision::CollisionLimits::default(),
            )
            .map_err(|_| "invalid_domination_helo_mesh")?;
            if mesh.stats().triangles == 0 {
                return Err("empty_domination_helo_mesh");
            }
            projectile.insert(
                faction,
                BodyBinding {
                    asset_sha256: models_receipt_sha256.to_owned(),
                    faction,
                    packfile_sha256: row.packfile_sha256.clone(),
                    part_ids: projectile_part_ids(&mesh),
                },
            );
            by_faction.insert(faction, mesh);
        }
        Ok(Self {
            by_faction,
            projectile,
        })
    }

    fn mesh(&self, faction: u8) -> Result<collision::CollisionWorld, &'static str> {
        self.by_faction
            .get(&faction)
            .cloned()
            .ok_or("unbound_domination_helo_faction_mesh")
    }
}

fn projectile_part_ids(mesh: &collision::CollisionWorld) -> Vec<String> {
    let mut ids = mesh.instance_ids().to_vec();
    ids.sort_unstable();
    ids
}

impl SequenceCollisionAssets {
    pub(crate) fn projectile_bindings(&self) -> Result<BTreeSet<SequenceBinding>, &'static str> {
        self.packs
            .values()
            .flatten()
            .map(|pack| {
                pack.binding
                    .projectile_binding(&pack.mesh)
                    .ok_or("invalid_projectile_sequence_tag")
            })
            .collect()
    }
}

impl HospitalHulkAssets {
    pub(crate) fn projectile_bindings(&self) -> BTreeSet<BodyBinding> {
        self.by_faction
            .values()
            .filter_map(|row| row.projectile.clone())
            .collect()
    }
}

impl HeloCollisionAssets {
    pub(crate) fn projectile_bindings(&self) -> impl Iterator<Item = &BodyBinding> {
        self.projectile.values()
    }
}

impl SequenceCollisionState {
    pub(crate) fn projectile_surfaces(
        &self,
        budget: &mut usize,
    ) -> Result<Vec<(String, &SequenceSurface)>, &'static str> {
        let mut surfaces = Vec::new();
        for ((objective, event), rows) in &self.active {
            for (slot, surface) in rows.iter().enumerate() {
                *budget = budget
                    .checked_sub(1)
                    .ok_or("projectile_contact_work_limit")?;
                surfaces.push((format!("objective-{objective}-{event}-{slot}"), surface));
            }
        }
        for (label, rows) in [
            ("hospital", &self.hospitals),
            ("hulk", &self.hulks),
            ("helo", &self.helos),
        ] {
            for (index, surface) in rows {
                *budget = budget
                    .checked_sub(1)
                    .ok_or("projectile_contact_work_limit")?;
                surfaces.push((format!("{label}-{index}"), surface));
            }
        }
        Ok(surfaces)
    }
}

impl SequenceSurface {
    pub(crate) fn projectile_obstacles<'a>(
        &'a self,
        placement: &'a str,
        sweep: &crate::projectile_contact::SphereSweep,
        layer_mask: u32,
        bindings: &ContactBindings,
        budget: &mut usize,
    ) -> Result<Vec<crate::projectile_contact::TriangleObstacle<'a>>, &'static str> {
        use crate::projectile_contact::{TriangleId, TriangleObstacle};
        let (root, properties) = self
            .projectile
            .as_ref()
            .ok_or("projectile_overlay_binding_unavailable")?
            .layers(bindings)?;
        if layer_mask & (1 << root) == 0 {
            return Ok(Vec::new());
        }
        let properties = properties.ok_or("projectile_overlay_binding_unavailable")?;
        if layer_mask & (1 << properties.child_filter_info) == 0 {
            return Ok(Vec::new());
        }
        let start = self.local(sweep.start);
        let end = self.local(sweep.end);
        let extent = sweep.radius + properties.convex_radius;
        let min = std::array::from_fn(|axis| start[axis].min(end[axis]) - extent);
        let max = std::array::from_fn(|axis| start[axis].max(end[axis]) + extent);
        Ok(self
            .mesh
            .local_projectile_triangles_in_box(min, max, budget)?
            .map(|(instance, triangle, vertices)| TriangleObstacle {
                id: TriangleId::Sequence {
                    placement,
                    instance,
                    triangle,
                },
                vertices,
                pose: self.pose,
                margin: properties.convex_radius,
            })
            .collect())
    }

    fn overlaps_sphere(
        &self,
        local: [f64; 3],
        sphere: &Geometry,
        limits: QueryLimits,
        budget: &mut usize,
    ) -> Result<bool, &'static str> {
        let radius = sphere.radius();
        let triangles = self
            .mesh
            .local_triangles_in_box(
                local.map(|axis| axis - radius),
                local.map(|axis| axis + radius),
                budget,
            )
            .map_err(|_| "domination_helo_blast_overlap_query_failed")?;
        for triangle in triangles {
            if geometry::penetrates(
                sphere,
                Pose::at(local),
                &Geometry::Triangle {
                    vertices: triangle,
                    radius: 0.0,
                },
                Pose::at([0.; 3]),
                limits,
                budget,
            )? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn local(&self, point: [f64; 3]) -> [f64; 3] {
        let delta = geometry::sub(point, self.pose.translation);
        std::array::from_fn(|axis| {
            (0..3)
                .map(|row| delta[row] * self.pose.rotation[row][axis])
                .sum()
        })
    }
}

impl SequenceCollisionAssets {
    pub fn load(root: &std::path::Path, effects: &StructuralCatalog) -> Result<Self, &'static str> {
        use std::io::Read;
        let mut input = String::new();
        std::fs::File::open(root.join("index.json"))
            .map_err(|_| "structural_collision_index_missing")?
            .take(512 * 1024 + 1)
            .read_to_string(&mut input)
            .map_err(|_| "structural_collision_index_invalid")?;
        if input.len() > 512 * 1024 {
            return Err("structural_collision_index_too_large");
        }
        let index: CollisionIndex =
            serde_json::from_str(&input).map_err(|_| "structural_collision_index_invalid")?;
        if index.schema != "mag_domination_structural_collision_v1"
            || index.map != effects.map
            || index.level_archive_sha256 != effects.level_archive_sha256
            || index.physics_sha256 != effects.physics_sha256
            || index.geometry_sha256 != effects.global_data_sha256
            || index.resources.len() != effects.bound_collision_count()
        {
            return Err("structural_collision_source_mismatch");
        }
        let mut packs = BTreeMap::new();
        for (token, resources) in index.resources {
            let source = effects
                .pack_hashes(&token)
                .ok_or("structural_collision_resource_unbound")?;
            if resources
                .iter()
                .map(|resource| &resource.source_sha256)
                .ne(source.iter())
            {
                return Err("structural_collision_pack_binding_mismatch");
            }
            let mut entries = Vec::new();
            for binding in resources {
                if binding.parent_token.len() != 8
                    || !binding.parent_token.bytes().all(|b| b.is_ascii_hexdigit())
                {
                    return Err("structural_collision_parent_token_invalid");
                }
                let mesh = collision::CollisionWorld::load(
                    root.join(format!("{}.json", binding.source_sha256)),
                    collision::CollisionLimits::default(),
                )
                .map_err(|_| "structural_collision_mesh_invalid")?;
                if mesh.source_sha256() != binding.neutral_sha256
                    || mesh.metadata().map_name != index.map
                    || mesh.stats().triangles == 0
                    || !mesh
                        .metadata()
                        .provenance
                        .iter()
                        .any(|item| item == &format!("packfile_sha256={}", binding.source_sha256))
                {
                    return Err("structural_collision_mesh_source_mismatch");
                }
                entries.push(CollisionPack { binding, mesh });
            }
            packs.insert(token, entries);
        }
        for parents in index.parent_reference_poses.values() {
            for parent in parents.values() {
                parent.pose.validate()?;
                if parent.source_sha256.len() != 64
                    || !parent.source_sha256.bytes().all(|b| b.is_ascii_hexdigit())
                {
                    return Err("structural_collision_geometry_source_invalid");
                }
            }
        }
        Ok(Self {
            packs,
            parents: index.parent_reference_poses,
            absent: effects.confirmed_absent_collisions().clone(),
        })
    }

    /// Fresh native rigid instances copy their original resource transforms.
    /// The animation creation event sets the composed root afterwards.
    pub fn reference_pose(
        &self,
        objective_root: Pose,
        creation: &Creation,
        pack: &SequencePackBinding,
    ) -> Result<Pose, &'static str> {
        objective_root.validate()?;
        let translation = creation.position.map(f64::from);
        let event = match creation.rotation {
            [0., 0., 0.] => Pose::at(translation),
            [180., 0., 180.] => Pose {
                translation,
                rotation: [[-1., 0., 0.], [0., 1., 0.], [0., 0., -1.]],
            },
            _ => return Err("structural_collision_unqualified_event_rotation"),
        };
        let composed = compose_pose(objective_root, event);
        if pack.parent_token == "00000000" {
            return Ok(composed);
        }
        let parent = self
            .parents
            .get(&creation.geometry_token)
            .and_then(|geometry| geometry.get(&pack.parent_token))
            .ok_or("structural_collision_parent_pose_missing")?;
        Ok(compose_pose(composed, parent.pose))
    }
}

fn compose_pose(parent: Pose, local: Pose) -> Pose {
    Pose {
        translation: geometry::add(
            parent.translation,
            geometry::rotate(parent, local.translation),
        ),
        rotation: std::array::from_fn(|row| {
            std::array::from_fn(|column| {
                (0..3)
                    .map(|axis| parent.rotation[row][axis] * local.rotation[axis][column])
                    .sum()
            })
        }),
    }
}

impl SequenceCollisionState {
    pub fn prepare_reference_playback(
        &mut self,
        objective: u16,
        root: Pose,
        change: PlaybackChange<'_>,
        assets: &SequenceCollisionAssets,
    ) -> Result<(), &'static str> {
        self.prepare_playback(objective, change, assets, |creation, pack| {
            assets.reference_pose(root, creation, pack)
        })
    }

    /// Replace collision on the detached world. The caller resolves every
    /// animated or linked bone pose from current world authority.
    pub fn prepare_playback(
        &mut self,
        objective: u16,
        change: PlaybackChange<'_>,
        assets: &SequenceCollisionAssets,
        mut resolve_pose: impl FnMut(&Creation, &SequencePackBinding) -> Result<Pose, &'static str>,
    ) -> Result<(), &'static str> {
        let mut placements = Vec::with_capacity(change.starts.len());
        for creation in change.starts {
            if creation.collision_token == "00000000"
                || assets.absent.contains(&creation.collision_token)
            {
                continue;
            }
            let packs = assets
                .packs
                .get(&creation.collision_token)
                .ok_or("structural_collision_asset_missing")?;
            let mut surfaces = Vec::with_capacity(packs.len());
            for pack in packs {
                let pose = resolve_pose(creation, &pack.binding)?;
                pose.validate()?;
                surfaces.push(SequenceSurface {
                    projectile: pack
                        .binding
                        .projectile_binding(&pack.mesh)
                        .map(SurfaceBinding::Sequence),
                    pose,
                    mesh: pack.mesh.clone(),
                });
            }
            placements.push(((objective, creation.event_index), surfaces));
        }
        for creation in change.stops {
            self.active.remove(&(objective, creation.event_index));
        }
        self.active.extend(placements);
        Ok(())
    }

    pub fn install_hospital_hulk(
        &mut self,
        index: u16,
        faction: u8,
        pose: Pose,
        assets: &HospitalHulkAssets,
    ) -> Result<(), &'static str> {
        pose.validate()?;
        let hulk = assets
            .by_faction
            .get(&faction)
            .ok_or("unbound_domination_hospital_hulk")?;
        self.hulks.insert(
            index,
            SequenceSurface {
                pose,
                mesh: hulk.mesh.clone(),
                projectile: Some(SurfaceBinding::HospitalHulk { layer: hulk.layer }),
            },
        );
        Ok(())
    }

    pub fn remove_hospital_hulk(&mut self, index: u16) {
        self.hulks.remove(&index);
    }

    pub(crate) fn install_operational_hospital(
        &mut self,
        hospital: crate::structures::DominationBunker<'_>,
        assets: &HospitalHulkAssets,
    ) -> Result<(), &'static str> {
        let mesh = assets
            .by_faction
            .get(&hospital.faction)
            .and_then(|row| row.operational.as_ref())
            .ok_or("domination_hospital_operational_collision_required")?
            .clone();
        let transform = hospital.exit_policy.transform;
        let columns = [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]].map(|axis| transform.rotate(axis));
        let pose = Pose {
            translation: [
                transform.position.x,
                transform.position.y,
                transform.position.z,
            ]
            .map(f64::from),
            rotation: std::array::from_fn(|i| std::array::from_fn(|j| columns[j][i].into())),
        };
        pose.validate()?;
        self.hulks.remove(&hospital.index);
        self.hospitals.insert(
            hospital.index,
            SequenceSurface {
                pose,
                mesh,
                projectile: assets.by_faction[&hospital.faction]
                    .projectile
                    .clone()
                    .map(SurfaceBinding::Hospital),
            },
        );
        Ok(())
    }

    pub(crate) fn remove_operational_hospital(&mut self, index: u16) {
        self.hospitals.remove(&index);
    }

    pub(crate) fn place_helo(
        &mut self,
        index: u16,
        faction: u8,
        pose: Pose,
        assets: &HeloCollisionAssets,
    ) -> Result<(), &'static str> {
        pose.validate()?;
        let mesh = assets.mesh(faction)?;
        self.helos.insert(
            index,
            SequenceSurface {
                pose,
                mesh,
                projectile: assets
                    .projectile
                    .get(&faction)
                    .cloned()
                    .map(SurfaceBinding::Helo),
            },
        );
        Ok(())
    }

    pub(crate) fn remove_helo(&mut self, index: u16) {
        self.helos.remove(&index);
    }

    /// Check active objective collision against a world-axis-aligned body box.
    pub fn positive_box_hit(
        &self,
        centre: [f64; 3],
        half_extents: [f64; 3],
        budget: &mut usize,
    ) -> Result<bool, &'static str> {
        self.positive_box_hit_except_helo(centre, half_extents, budget, None)
    }

    pub(crate) fn positive_box_hit_except_helo(
        &self,
        centre: [f64; 3],
        half_extents: [f64; 3],
        budget: &mut usize,
        excluded_helo: Option<u16>,
    ) -> Result<bool, &'static str> {
        let helos = self
            .helos
            .iter()
            .filter_map(|(&index, surface)| (Some(index) != excluded_helo).then_some(surface));
        for surface in self
            .active
            .values()
            .flatten()
            .chain(self.hulks.values())
            .chain(self.hospitals.values())
            .chain(helos)
        {
            if surface
                .mesh
                .overlaps_placed_aabb(surface.pose, centre, half_extents, budget)
                .map_err(|_| "objective_collision_mesh_box_failed")?
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn filtered_ray_hits(
        &self,
        start: [f64; 3],
        end: [f64; 3],
        row: u32,
        excludes: [Option<u64>; 2],
        budget: &mut usize,
        hits: &mut Vec<collision::MaterialRayHit>,
    ) -> Result<(), &'static str> {
        for surface in self.active.values().flatten() {
            hits.extend(
                surface
                    .mesh
                    .filtered_material_ray_hits(
                        surface.local(start),
                        surface.local(end),
                        row,
                        budget,
                    )
                    .map_err(|_| "air_strike_sequence_filter_query_failed")?,
            );
        }
        let fixed_layer = [
            (&self.hulks, 17, 0x0d80_0000u64),
            (&self.hospitals, 22, 0x0d80_0000),
            (&self.helos, 22, 0x0280_0000),
        ];
        for (surfaces, layer, family) in fixed_layer {
            if row & (1 << layer) == 0 {
                continue;
            }
            for (&index, surface) in surfaces {
                if excludes.contains(&Some(family | u64::from(index))) {
                    continue;
                }
                hits.extend(
                    surface
                        .mesh
                        .material_ray_hits(surface.local(start), surface.local(end), budget)
                        .map_err(|_| "air_strike_overlay_ray_failed")?,
                );
            }
        }
        Ok(())
    }

    fn material_ray_hits(
        &self,
        start: [f64; 3],
        end: [f64; 3],
        row24: u32,
        excludes: [Option<u64>; 2],
        budget: &mut usize,
        hits: &mut Vec<collision::MaterialRayHit>,
    ) -> Result<(), &'static str> {
        let helos = self.helos.iter().filter_map(|(&index, surface)| {
            (row24 & (1 << 22) != 0
                && !excludes.contains(&Some(u64::from(0x0280_0000 | u32::from(index)))))
            .then_some(surface)
        });
        let hospitals = self.hospitals.iter().filter_map(|(&index, surface)| {
            (row24 & (1 << 22) != 0
                && !excludes.contains(&Some(u64::from(0x0d80_0000 | u32::from(index)))))
            .then_some(surface)
        });
        for surface in self.active.values().flatten().chain(helos).chain(hospitals) {
            hits.extend(
                surface
                    .mesh
                    .material_ray_hits(surface.local(start), surface.local(end), budget)
                    .map_err(|_| "objective_collision_mesh_material_ray_failed")?,
            );
        }
        Ok(())
    }

    /// Query moving mesh instances using their qualified current rigid pose.
    pub fn positive_ray_hit(
        &self,
        start: [f64; 3],
        end: [f64; 3],
        budget: &mut usize,
    ) -> Result<bool, &'static str> {
        for surface in self.active.values().flatten() {
            let ray = surface
                .mesh
                .raycast_segment_bounded(surface.local(start), surface.local(end), budget)
                .map_err(|_| "structural_collision_mesh_ray_failed")?;
            if ray
                .nearest
                .is_some_and(|hit| hit.fraction > 0.0 && hit.fraction < 1.0)
            {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

#[cfg(test)]
impl SequenceCollisionState {
    pub(crate) fn synthetic_projectile_hospital(&mut self, index: u16, x: f64, digest: String) {
        let payload = json!({
            "format":"mag_collision_world", "schema_version":1, "coordinate_space":"world",
            "metadata":{"map_name":"synthetic","build":"synthetic","coverage":"provisional",
                "provenance":["synthetic"],"unknowns":[]},
            "instances":[{"id":"hospital-face", "material_index":1,
                "vertices_xyz":[[0.,-2.,-2.],[0.,2.,-2.],[0.,0.,2.]],"triangles":[[0,1,2]]}]
        });
        let mesh = collision::CollisionWorld::from_bytes(
            &serde_json::to_vec(&payload).unwrap(),
            collision::CollisionLimits::default(),
        )
        .unwrap();
        self.hospitals.insert(
            index,
            SequenceSurface {
                projectile: Some(SurfaceBinding::Hospital(BodyBinding {
                    asset_sha256: digest.clone(),
                    faction: 0,
                    packfile_sha256: digest,
                    part_ids: vec!["hospital-face".to_owned()],
                })),
                pose: Pose::at([x, 0., 0.]),
                mesh,
            },
        );
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn command_blast_falloff_retains_authored_edge_values_after_overlap() {
        let falloff = crate::active_ability_effects::BlastFalloff {
            inner_radius: 20.,
            inner_scale: 0.75,
            outer_radius: 20.,
            outer_scale: 1.,
        };
        assert_eq!(native_command_falloff(falloff, 10.).unwrap(), 0.875);
        assert_eq!(native_command_falloff(falloff, 20.).unwrap(), 0.75);
        assert_eq!(native_command_falloff(falloff, 20.5).unwrap(), 1.);
    }

    use super::*;

    #[test]
    fn acquisition_admits_authored_structure_blasts_and_strikes() {
        assert!(BlastSource::AirStrike.supported_mode(1));
        assert!(BlastSource::AirStrike.supported_mode(2));
        assert!(!BlastSource::AirStrike.supported_mode(0));
        assert!(BlastSource::Structural { objective_id: 1 }.supported_mode(1));
        assert!(!BlastSource::Structural { objective_id: 1 }.supported_mode(0));
        assert!(!BlastSource::Ground { body_id: 1 }.supported_mode(1));
    }

    #[test]
    fn zero_penetration_air_strike_blocks_on_unlabelled_terrain() {
        let payload = json!({
            "format":"mag_collision_world", "schema_version":1, "coordinate_space":"world",
            "metadata":{"map_name":"synthetic","build":"synthetic","coverage":"provisional",
                "provenance":["synthetic"],"unknowns":[]},
            "instances":[{"id":"unlabelled_terrain", "terrain_material":true,
                "collision":{"filter_info":9,"convex_radius":0.,"child_filter_policy":"uniform","provenance":"synthetic"},
                "vertices_xyz":[[0.,0.,0.],[3.,0.,0.],[0.,0.,3.]],"triangles":[[0,1,2]]}]
        });
        let scene = collision::CollisionWorld::from_bytes(
            &serde_json::to_vec(&payload).unwrap(),
            Default::default(),
        )
        .unwrap();
        let snapshot = dynamic_collision::Snapshot::capture(
            crate::acquisition_atomic::WorldKey {
                world_id: 1,
                incarnation: 1,
            },
            0,
            &[],
            vec![],
            0,
        )
        .unwrap();
        let mut rows = [0; 32];
        rows[24] = 1 << 9;
        let collision = BlastCollision {
            filter_rows: &rows,
            limits: Default::default(),
            sequences: &SequenceCollisionState::default(),
            material_costs: &BTreeMap::new(),
            terrain_materials: None,
        };
        let ray = BlastRay {
            start: [1., 1., 1.],
            end: [1., -1., 1.],
            excludes: [None; 2],
            penetration: 0.,
            zero_penetration_strike: true,
            filter_static: true,
        };
        assert_eq!(
            collision.clear_path(&scene, &snapshot, ray, &mut 100),
            Ok(false)
        );
        let old_source = BlastRay {
            zero_penetration_strike: false,
            filter_static: false,
            ..ray
        };
        assert_eq!(
            collision.clear_path(&scene, &snapshot, old_source, &mut 100),
            Err("domination_terrain_material_grid_required"),
        );
    }

    #[test]
    fn cluster_impact_uses_nearest_eligible_live_surface_then_elevates_hit() {
        let payload = json!({
            "format":"mag_collision_world", "schema_version":1, "coordinate_space":"world",
            "metadata":{"map_name":"synthetic","build":"synthetic","coverage":"complete",
                "provenance":["synthetic"],"unknowns":[]},
            "instances":[
                {"id":"ground","vertices_xyz":[[0.,1.,0.],[3.,1.,0.],[0.,1.,3.]],"triangles":[[0,1,2]],
                "collision":{"filter_info":9,"convex_radius":0.,"child_filter_policy":"uniform","provenance":"synthetic"}},
                {"id":"filtered","vertices_xyz":[[0.,4.,0.],[3.,4.,0.],[0.,4.,3.]],"triangles":[[0,1,2]],
                "collision":{"filter_info":9,"ray_child_filter_info":22,"convex_radius":0.,"child_filter_policy":"uniform","provenance":"synthetic"}}
            ]
        });
        let scene = collision::CollisionWorld::from_bytes(
            &serde_json::to_vec(&payload).unwrap(),
            Default::default(),
        )
        .unwrap();
        let snapshot = dynamic_collision::Snapshot::capture(
            crate::acquisition_atomic::WorldKey {
                world_id: 1,
                incarnation: 1,
            },
            0,
            &[],
            vec![],
            0,
        )
        .unwrap();
        let mut sequences = SequenceCollisionState::default();
        sequences.hulks.insert(
            1,
            SequenceSurface {
                projectile: None,
                pose: Pose::at([0., 1., 0.]),
                mesh: scene.clone(),
            },
        );
        let mut rows = [0; 32];
        rows[19] = (1 << 9) | (1 << 17);
        let context = BlastCollision {
            filter_rows: &rows,
            limits: Default::default(),
            sequences: &sequences,
            material_costs: &BTreeMap::new(),
            terrain_materials: None,
        };
        let static_only = BlastCollision {
            sequences: &SequenceCollisionState::default(),
            ..context
        };
        let static_hit = static_only
            .strike_impact(&scene, &snapshot, [1., 1.], [0., 8.], 100.)
            .unwrap()
            .unwrap();
        assert!((static_hit[1] - 1.5).abs() < 0.00002);

        // The known layer-17 overlay contributes both triangles at its live pose.
        let hit = context
            .strike_impact(&scene, &snapshot, [1., 1.], [0., 8.], 100.)
            .unwrap();
        let hit = hit.unwrap();
        assert_eq!([hit[0], hit[2]], [1., 1.]);
        assert!((hit[1] - 5.5).abs() < 0.00002);
        let miss = context
            .strike_impact(&scene, &snapshot, [8., 8.], [0., 8.], 100.)
            .unwrap();
        assert_eq!(miss, None);
        assert!(
            context
                .strike_impact(&scene, &snapshot, [1., 1.], [8., 0.], 100.)
                .is_err()
        );
    }

    #[test]
    fn blast_material_budget_depletes_only_on_front_faces() {
        let costs = BTreeMap::from([(1, 1.0), (2, -1.0)]);
        let hit = |fraction, material_index, front| collision::MaterialRayHit {
            fraction,
            material_index: Some(material_index),
            terrain: false,
            front,
        };
        let mut via_back = [hit(0.8, 1, true), hit(0.2, 1, false)];
        assert_eq!(can_penetrate(&mut via_back, 1.7, &costs), Ok(true));
        let mut via_front = [hit(0.8, 1, true), hit(0.2, 1, true)];
        assert_eq!(can_penetrate(&mut via_front, 1.7, &costs), Ok(false));
        assert_eq!(
            can_penetrate(&mut [hit(0.2, 1, true)], 1.0, &costs),
            Ok(false)
        );
        assert_eq!(
            can_penetrate(&mut [hit(0.2, 2, false)], 1.7, &costs),
            Ok(false)
        );
        assert!(can_penetrate(&mut [hit(0.2, 3, true)], 1.7, &costs).is_err());
    }

    #[test]
    fn scans_sixteen_slots_per_state_update_and_keeps_independent_cursors() {
        let properties = BlastProperties {
            subtype: 0,
            full_damage_radius: 0.,
            half_damage_radius: 0.,
            damage_value: 0.,
            material_penetration: 0.,
            character_damage_over_time_radius: 2.,
            character_damage_over_time_value: 3.,
        };
        let mut state = AreaDamageState::default();
        assert_eq!(
            state.next_slots(1, ObjectiveState::Deactivated, properties),
            None
        );
        for first in (0..256).step_by(16) {
            assert_eq!(
                state.next_slots(1, ObjectiveState::Completed, properties),
                Some(first..first + 16)
            );
        }
        assert_eq!(
            state.next_slots(1, ObjectiveState::Repairing, properties),
            Some(0..16)
        );
        assert_eq!(
            state.next_slots(2, ObjectiveState::Completed, properties),
            Some(0..16)
        );
        assert_eq!(
            state.next_slots(1, ObjectiveState::Completed, properties),
            Some(16..32)
        );
    }

    #[test]
    fn mounted_helo_mesh_uses_the_live_pose_and_filter_row() {
        let payload = json!({
            "format":"mag_collision_world", "schema_version":1, "coordinate_space":"world",
            "metadata":{"map_name":"synthetic","build":"synthetic","coverage":"provisional",
                "provenance":["synthetic"],"unknowns":[]},
            "instances":[{"id":"synthetic_surface", "material_index":1,
                "vertices_xyz":[[0.,0.,0.],[0.,2.,0.],[0.,0.,2.]],"triangles":[[0,1,2]]}]
        });
        let mesh = collision::CollisionWorld::from_bytes(
            &serde_json::to_vec(&payload).unwrap(),
            collision::CollisionLimits::default(),
        )
        .unwrap();
        let assets = HeloCollisionAssets {
            projectile: BTreeMap::new(),
            by_faction: BTreeMap::from([(0, mesh)]),
        };
        let mut state = SequenceCollisionState::default();
        state
            .place_helo(7, 0, Pose::at([4., 0., 0.]), &assets)
            .unwrap();
        let left = [3., 0.25, 0.25];
        let right = [5., 0.25, 0.25];
        let mut hits = Vec::new();
        state
            .material_ray_hits(left, right, 0, [None; 2], &mut 100, &mut hits)
            .unwrap();
        assert!(hits.is_empty());
        for (start, end) in [(left, right), (right, left)] {
            hits.clear();
            state
                .material_ray_hits(start, end, 1 << 22, [None; 2], &mut 100, &mut hits)
                .unwrap();
            assert_eq!(hits.len(), 1);
            assert_eq!(hits[0].material_index, Some(1));
            assert_eq!(hits[0].fraction, 0.5);
            assert!(hits[0].front);
        }
        let surface = &state.helos[&7];
        let radius = Geometry::Sphere { radius: 0.5 };
        assert!(
            surface
                .overlaps_sphere(
                    surface.local([3.75, 0.25, 0.25]),
                    &radius,
                    QueryLimits::default(),
                    &mut 1000
                )
                .unwrap()
        );
        assert!(
            !surface
                .overlaps_sphere(
                    surface.local([2., 0.25, 0.25]),
                    &radius,
                    QueryLimits::default(),
                    &mut 1000
                )
                .unwrap()
        );
        let centre = [4., 0.25, 0.25];
        assert!(state.positive_box_hit(centre, [0.1; 3], &mut 100).unwrap());
        assert!(
            !state
                .positive_box_hit_except_helo(centre, [0.1; 3], &mut 100, Some(7))
                .unwrap()
        );
        state
            .place_helo(7, 0, Pose::at([10., 0., 0.]), &assets)
            .unwrap();
        assert!(!state.positive_box_hit(centre, [0.1; 3], &mut 100).unwrap());
        state.remove_helo(7);
        assert!(
            !state
                .positive_box_hit([10., 0.25, 0.25], [0.1; 3], &mut 100)
                .unwrap()
        );
    }

    #[test]
    fn playback_installs_a_transformed_mesh_and_removes_it_on_stop() {
        let path = std::env::temp_dir().join(format!(
            "mag-structural-sequence-{}-test.json",
            std::process::id()
        ));
        let payload = json!({
            "format":"mag_collision_world", "schema_version":1, "coordinate_space":"world",
            "metadata":{"map_name":"synthetic","build":"synthetic","coverage":"provisional",
                "provenance":["synthetic"],"unknowns":[]},
            "instances":[{"id":"synthetic_surface",
                "vertices_xyz":[[0.,0.,0.],[0.,2.,0.],[0.,0.,2.]],"triangles":[[0,1,2]]}]
        });
        std::fs::write(&path, payload.to_string()).unwrap();
        let mesh =
            collision::CollisionWorld::load(&path, collision::CollisionLimits::default()).unwrap();
        std::fs::remove_file(path).unwrap();
        let binding = SequencePackBinding {
            source_sha256: "0".repeat(64),
            neutral_sha256: "1".repeat(64),
            parent_token: "00000000".to_owned(),
            collision_tag: 0,
            material_indices: Vec::new(),
        };
        let assets = SequenceCollisionAssets {
            parents: BTreeMap::new(),
            absent: BTreeSet::new(),
            packs: BTreeMap::from([("00000004".to_owned(), vec![CollisionPack { binding, mesh }])]),
        };
        let creation = Creation {
            event_index: 0,
            object_token: "00000001".to_owned(),
            geometry_token: "00000002".to_owned(),
            collision_token: "00000004".to_owned(),
            skeleton_token: "00000000".to_owned(),
            position: [0.; 3],
            rotation: [0.; 3],
            start_seconds: 0.,
            duration_seconds: -1.,
        };
        let mut state = SequenceCollisionState::default();
        let ray = |state: &SequenceCollisionState| {
            state
                .positive_ray_hit([3., 0.25, 0.25], [5., 0.25, 0.25], &mut 100)
                .unwrap()
        };
        let box_query = |state: &SequenceCollisionState| {
            state
                .positive_box_hit([4., 0.25, 0.25], [0.1; 3], &mut 100)
                .unwrap()
        };
        assert!(!ray(&state));
        assert!(!box_query(&state));
        state
            .prepare_playback(
                1,
                PlaybackChange {
                    stops: vec![],
                    starts: vec![&creation],
                },
                &assets,
                |_, _| Ok(Pose::at([4., 0., 0.])),
            )
            .unwrap();
        assert!(ray(&state));
        assert!(box_query(&state));
        state
            .prepare_playback(
                1,
                PlaybackChange {
                    stops: vec![&creation],
                    starts: vec![],
                },
                &assets,
                |_, _| Err("no_start_expected"),
            )
            .unwrap();
        assert!(!ray(&state));
        assert!(!box_query(&state));
    }

    #[test]
    fn cooldown_is_shared_across_objectives_and_fenced_by_character_generation() {
        let state = AreaDamageState {
            cooldowns: BTreeMap::from([(7, (2, Duration::from_secs(1)))]),
            ..Default::default()
        };
        assert!(state.cooling_down(7, 2, Duration::from_millis(999)));
        assert!(!state.cooling_down(7, 2, Duration::from_secs(1)));
        assert!(!state.cooling_down(7, 3, Duration::from_millis(999)));
    }
}
