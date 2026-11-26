//! A mode's placed structures (bunkers, gates, fortifications, field
//! hospitals) as obstacles for bot movement and routing.
//!
//! # Behaviour
//!
//! The static map world has no placed structures: each one is a sequence
//! surface the mode places, swaps and removes as the structure changes state
//! (intact, destroyed, repaired), held by `SequenceCollisionState`. Bot
//! movement must collide with what stands now:
//!
//! * a snapshot lists every placed surface the character movement filter row
//!   (query layer 1) collides with, with its pose and world bounds;
//! * body sweeps and ground rays in `bot_physics` test the snapshot after the
//!   static world, so an intact gate or bunker wall stops a bot, a roof or
//!   floor carries it, and a destroyed structure's surfaces (rubble, gaps)
//!   are whatever its destroyed state placed;
//! * a body a structure appears on is not trapped: a moving sweep ignores the
//!   triangles it already penetrates and still stops at every other one;
//! * the snapshot is rebuilt only when the structures change (their stamp or
//!   the movement row), once per world tick at most, and shared by reference.
//!
//! Routing uses the same snapshot: see `navmesh_structure_gates`.
use crate::collision::CollisionWorld;
use crate::dynamic_collision::Pose;
use std::io::Write;
use std::sync::Arc;

/// Retail query layer of character (infantry) movement.
pub(crate) const CHARACTER_QUERY_LAYER: usize = 1;
/// Work units one structure query may spend, separate from the static budget.
pub(crate) const STRUCTURE_WORK_LIMIT: usize = 16_384;

/// A sweep's nearest structure contact: (fraction, contact normal, obstacle label).
pub(crate) type SweepContact<'a> = (f64, [f64; 3], &'a str);
/// A segment's nearest structure crossing: (fraction, position, normal).
pub(crate) type RayCrossing = (f64, [f64; 3], [f64; 3]);

pub(crate) struct Obstacle {
    pub(crate) label: String,
    pub(crate) pose: Pose,
    pub(crate) mesh: CollisionWorld,
    pub(crate) min: [f64; 3],
    pub(crate) max: [f64; 3],
}

/// Structures standing now, with the movement row they were filtered by.
pub(crate) struct StructureObstacles {
    key: (u64, u32),
    obstacles: Vec<Obstacle>,
}

impl std::fmt::Debug for StructureObstacles {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StructureObstacles")
            .field("key", &self.key)
            .field("obstacles", &self.obstacles.len())
            .finish()
    }
}

impl StructureObstacles {
    pub(crate) fn new(
        sequences: &crate::domination_structural_world::SequenceCollisionState,
        row: u32,
    ) -> Self {
        Self::from_surfaces(
            (sequences.revision(), row),
            sequences.movement_obstacles(row),
        )
    }

    pub(crate) fn from_surfaces(
        key: (u64, u32),
        surfaces: Vec<(String, Pose, CollisionWorld)>,
    ) -> Self {
        let obstacles = surfaces
            .into_iter()
            .filter_map(|(label, pose, mesh)| {
                let local = mesh.stats().bounds?;
                let centre: [f64; 3] = std::array::from_fn(|i| (local.min[i] + local.max[i]) * 0.5);
                let half: [f64; 3] = std::array::from_fn(|i| (local.max[i] - local.min[i]) * 0.5);
                let offset = crate::dynamic_collision::geometry::rotate(pose, centre);
                let extent: [f64; 3] = pose
                    .rotation
                    .map(|r| (0..3).map(|i| r[i].abs() * half[i]).sum());
                Some(Obstacle {
                    label,
                    pose,
                    mesh,
                    min: std::array::from_fn(|i| pose.translation[i] + offset[i] - extent[i]),
                    max: std::array::from_fn(|i| pose.translation[i] + offset[i] + extent[i]),
                })
            })
            .collect();
        Self { key, obstacles }
    }

    pub(crate) fn key(&self) -> (u64, u32) {
        self.key
    }

    pub(crate) fn obstacles(&self) -> &[Obstacle] {
        &self.obstacles
    }

    fn near(&self, min: [f64; 3], max: [f64; 3]) -> impl Iterator<Item = &Obstacle> {
        self.obstacles
            .iter()
            .filter(move |o| (0..3).all(|i| o.min[i] <= max[i] && min[i] <= o.max[i]))
    }

    /// Nearest structure contact of a world-axis-aligned box swept from centre
    /// `start` to `end`: (fraction, contact normal, obstacle label).
    pub(crate) fn sweep_aabb(
        &self,
        start: [f64; 3],
        end: [f64; 3],
        half: [f64; 3],
        budget: &mut usize,
    ) -> Result<Option<SweepContact<'_>>, &'static str> {
        let min = std::array::from_fn(|i| start[i].min(end[i]) - half[i]);
        let max = std::array::from_fn(|i| start[i].max(end[i]) + half[i]);
        let mut nearest: Option<SweepContact<'_>> = None;
        for obstacle in self.near(min, max) {
            if let Some((fraction, normal)) =
                obstacle
                    .mesh
                    .posed_sweep_aabb(obstacle.pose, start, end, half, true, budget)?
                && nearest.is_none_or(|(f, ..)| fraction < f)
            {
                nearest = Some((fraction, normal, obstacle.label.as_str()));
            }
        }
        Ok(nearest)
    }

    /// Whether a body standing at `feet` penetrates a structure, lifted by
    /// its step height so the floor it stands on does not count.
    pub(crate) fn body_penetrates(
        &self,
        feet: [f64; 3],
        body: crate::bot_physics::Body,
    ) -> Result<bool, &'static str> {
        let half = [
            body.radius,
            (body.height - body.max_step) * 0.5,
            body.radius,
        ];
        let centre = [feet[0], feet[1] + body.max_step + half[1], feet[2]];
        let min = std::array::from_fn(|i| centre[i] - half[i]);
        let max = std::array::from_fn(|i| centre[i] + half[i]);
        for obstacle in self.near(min, max) {
            let mut work = STRUCTURE_WORK_LIMIT;
            if obstacle
                .mesh
                .posed_sweep_aabb(obstacle.pose, centre, centre, half, false, &mut work)?
                .is_some()
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Nearest structure crossing of a segment: (fraction, position, normal).
    pub(crate) fn raycast(
        &self,
        start: [f64; 3],
        end: [f64; 3],
        budget: &mut usize,
    ) -> Result<Option<RayCrossing>, &'static str> {
        let min = std::array::from_fn(|i| start[i].min(end[i]));
        let max = std::array::from_fn(|i| start[i].max(end[i]));
        let mut nearest: Option<RayCrossing> = None;
        for obstacle in self.near(min, max) {
            if let Some(hit) = obstacle
                .mesh
                .posed_raycast(obstacle.pose, start, end, budget)?
                && nearest.is_none_or(|(f, ..)| hit.0 < f)
            {
                nearest = Some(hit);
            }
        }
        Ok(nearest)
    }
}

impl<W: Write> crate::Engine<W> {
    /// Bring the structure snapshot bot movement and routing see up to date
    /// with the current mode's structures. Cheap when nothing changed; a
    /// rebuild is recorded as `bot_structure_obstacles`.
    pub(crate) fn refresh_bot_structure_obstacles(
        &mut self,
        wall: u64,
    ) -> Result<(), &'static str> {
        let source = match (&self.acquisition_gameplay, &self.domination_gameplay) {
            (Some(runtime), _) => runtime.blast_collision(),
            (None, Some(world)) => Some(world.frag_blast_collision()),
            (None, None) => None,
        }
        .map(|c| (c.sequences, c.filter_rows[CHARACTER_QUERY_LAYER]));
        let Some(world) = self.collision_world.as_mut() else {
            return Ok(());
        };
        let wanted = source.map(|(sequences, row)| (sequences.revision(), row));
        let current = world.bot_structures().map(StructureObstacles::key);
        let began = std::time::Instant::now();
        let rebuilt = current != wanted;
        let snapshot = if rebuilt {
            let snapshot =
                source.map(|(sequences, row)| Arc::new(StructureObstacles::new(sequences, row)));
            world.set_bot_structures(snapshot.clone());
            snapshot
        } else {
            world.bot_structures_arc().cloned()
        };
        let gated = match &self.navmesh_world {
            Some(mesh) => crate::navmesh_structure_gates::apply(mesh, snapshot.as_ref()),
            None => Ok(None),
        };
        if rebuilt || !matches!(gated, Ok(None)) {
            let (gated_polygons, error) = match gated {
                Ok(count) => (count, None),
                Err(error) => (None, Some(error)),
            };
            self.record(
                serde_json::json!({
                    "op": "bot_structure_obstacles",
                    "obstacles": snapshot.as_ref().map_or(0, |s| s.obstacles().len()),
                    "movement_row": wanted.map(|k| k.1),
                    "gated_polygons": gated_polygons,
                    "gate_error": error,
                    "build_us": began.elapsed().as_micros() as u64,
                }),
                wall,
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "bot_structure_collision_tests.rs"]
mod tests;
