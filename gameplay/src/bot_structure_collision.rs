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
//!   (query layer 1) collides with, with its pose and world bounds, indexed
//!   on a coarse horizontal grid;
//! * body sweeps and ground rays in `bot_physics` test the snapshot after the
//!   static world, so an intact gate or bunker wall stops a bot, a roof or
//!   floor carries it, and a destroyed structure's surfaces (rubble, gaps)
//!   are whatever its destroyed state placed;
//! * a body a structure appears on is not trapped: a moving sweep ignores the
//!   triangles it already penetrates and still stops at every other one;
//! * a body standing in a structure's enclosed room (free space no opening
//!   connects to the outside, such as a closed bunker a life was started in
//!   on foot) is not held by that structure: it walks out, and routes from
//!   there are planned on the static mesh. Other bodies still cannot enter;
//! * the snapshot is rebuilt only when the structures change (their stamp or
//!   the movement row), once per world tick at most, and shared by reference.
//!   The costly analysis (enclosed rooms, route gates) runs on a worker
//!   thread; until it finishes the previous analysis stays in use.
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

impl Obstacle {
    fn local(&self, p: [f64; 3]) -> [f64; 3] {
        let d: [f64; 3] = std::array::from_fn(|i| p[i] - self.pose.translation[i]);
        std::array::from_fn(|axis| {
            (0..3)
                .map(|row| d[row] * self.pose.rotation[row][axis])
                .sum()
        })
    }

    /// Whether feet at `p` stand in this structure's enclosed room, once its
    /// rooms are analysed.
    fn encloses(&self, p: [f64; 3]) -> bool {
        interior(self.mesh.source_sha256()).is_some_and(|room| room.contains(self.local(p)))
    }

    /// Identity of this placement: label, mesh and exact pose.
    pub(crate) fn identity(&self) -> (String, String, [u64; 12]) {
        let mut pose = [0u64; 12];
        for i in 0..3 {
            pose[i] = self.pose.translation[i].to_bits();
            for j in 0..3 {
                pose[3 + i * 3 + j] = self.pose.rotation[i][j].to_bits();
            }
        }
        (
            self.label.clone(),
            self.mesh.source_sha256().to_owned(),
            pose,
        )
    }
}

/// Horizontal cell of the obstacle index, in metres.
const INDEX_CELL_M: f64 = 16.0;

/// Structures standing now, with the movement row they were filtered by.
pub(crate) struct StructureObstacles {
    key: (u64, u32),
    obstacles: Vec<Obstacle>,
    index: std::collections::HashMap<(i64, i64), Vec<u32>>,
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
            .collect::<Vec<_>>();
        let mut index: std::collections::HashMap<(i64, i64), Vec<u32>> = Default::default();
        for (n, o) in obstacles.iter().enumerate() {
            let (x0, z0) = cell_of(o.min);
            let (x1, z1) = cell_of(o.max);
            for x in x0..=x1 {
                for z in z0..=z1 {
                    index.entry((x, z)).or_default().push(n as u32);
                }
            }
        }
        Self {
            key,
            obstacles,
            index,
        }
    }

    pub(crate) fn key(&self) -> (u64, u32) {
        self.key
    }

    pub(crate) fn obstacles(&self) -> &[Obstacle] {
        &self.obstacles
    }

    /// Obstacles whose bounds overlap the box `min`..`max`, each once.
    fn near(&self, min: [f64; 3], max: [f64; 3]) -> Vec<&Obstacle> {
        let overlaps = |o: &Obstacle| (0..3).all(|i| o.min[i] <= max[i] && min[i] <= o.max[i]);
        let (x0, z0) = cell_of(min);
        let (x1, z1) = cell_of(max);
        if (x1 - x0 + 1) * (z1 - z0 + 1) > 64 {
            return self.obstacles.iter().filter(|o| overlaps(o)).collect();
        }
        let mut found: Vec<u32> = Vec::new();
        for x in x0..=x1 {
            for z in z0..=z1 {
                if let Some(ids) = self.index.get(&(x, z)) {
                    found.extend(ids);
                }
            }
        }
        found.sort_unstable();
        found.dedup();
        found
            .into_iter()
            .map(|n| &self.obstacles[n as usize])
            .filter(|o| overlaps(o))
            .collect()
    }

    /// Whether feet at `p` stand in an enclosed room of a structure.
    pub(crate) fn enclosed(&self, p: [f64; 3]) -> bool {
        self.near(p, p).into_iter().any(|o| o.encloses(p))
    }

    /// Nearest structure contact of a world-axis-aligned box swept from centre
    /// `start` to `end`: (fraction, contact normal, obstacle label). Each
    /// obstacle sees only the part of the sweep near its bounds, in legs of at
    /// most `SWEEP_LEG_M`, so a long sweep never gathers a whole structure's
    /// triangles at once. A structure whose enclosed room holds the body at
    /// `start` does not stop it.
    pub(crate) fn sweep_aabb(
        &self,
        start: [f64; 3],
        end: [f64; 3],
        half: [f64; 3],
        budget: &mut usize,
    ) -> Result<Option<SweepContact<'_>>, &'static str> {
        const SWEEP_LEG_M: f64 = 2.0;
        let min = std::array::from_fn(|i| start[i].min(end[i]) - half[i]);
        let max = std::array::from_fn(|i| start[i].max(end[i]) + half[i]);
        let feet = [start[0], start[1] - half[1], start[2]];
        let delta: [f64; 3] = std::array::from_fn(|i| end[i] - start[i]);
        let length = (0..3).map(|i| delta[i] * delta[i]).sum::<f64>().sqrt();
        let mut nearest: Option<SweepContact<'_>> = None;
        for obstacle in self.near(min, max) {
            let Some((t0, t1)) = clip(start, delta, obstacle, half) else {
                continue;
            };
            if nearest.is_some_and(|(f, ..)| f <= t0) || obstacle.encloses(feet) {
                continue;
            }
            let legs = ((t1 - t0) * length / SWEEP_LEG_M).ceil().max(1.0) as usize;
            for leg in 0..legs {
                let a = t0 + (t1 - t0) * leg as f64 / legs as f64;
                let b = t0 + (t1 - t0) * (leg + 1) as f64 / legs as f64;
                let from = std::array::from_fn(|i| start[i] + delta[i] * a);
                let to = std::array::from_fn(|i| start[i] + delta[i] * b);
                if let Some((fraction, normal)) =
                    obstacle
                        .mesh
                        .posed_sweep_aabb(obstacle.pose, from, to, half, true, budget)?
                {
                    let fraction = a + (b - a) * fraction;
                    if nearest.is_none_or(|(f, ..)| fraction < f) {
                        nearest = Some((fraction, normal, obstacle.label.as_str()));
                    }
                    break;
                }
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

fn cell_of(p: [f64; 3]) -> (i64, i64) {
    (
        (p[0] / INDEX_CELL_M).floor() as i64,
        (p[2] / INDEX_CELL_M).floor() as i64,
    )
}

/// The part [t0, t1] of the sweep `start + t * delta` whose box (half
/// extents `half`) overlaps the obstacle's bounds; None when it never does.
fn clip(start: [f64; 3], delta: [f64; 3], o: &Obstacle, half: [f64; 3]) -> Option<(f64, f64)> {
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    for i in 0..3 {
        let lo = o.min[i] - half[i];
        let hi = o.max[i] + half[i];
        if delta[i].abs() < 1e-12 {
            if start[i] < lo || start[i] > hi {
                return None;
            }
            continue;
        }
        let a = (lo - start[i]) / delta[i];
        let b = (hi - start[i]) / delta[i];
        t0 = t0.max(a.min(b));
        t1 = t1.min(a.max(b));
        if t0 > t1 {
            return None;
        }
    }
    Some((t0, t1))
}

/// A structure mesh's enclosed rooms, in its own frame: cells of a
/// horizontal grid where a standing body fits at the structure's base level
/// (local height 0) but cannot reach the mesh's outer bounds without crossing
/// the mesh.
pub(crate) struct Interior {
    origin: [f64; 2],
    cell: f64,
    size: [usize; 2],
    enclosed: Vec<bool>,
}

impl Interior {
    fn contains(&self, local: [f64; 3]) -> bool {
        if !(-1.0..=2.5).contains(&local[1]) {
            return false;
        }
        let x = ((local[0] - self.origin[0]) / self.cell).floor();
        let z = ((local[2] - self.origin[1]) / self.cell).floor();
        x >= 0.0
            && z >= 0.0
            && (x as usize) < self.size[0]
            && (z as usize) < self.size[1]
            && self.enclosed[x as usize + z as usize * self.size[0]]
    }

    /// Analyse one mesh; None when it encloses no room of a square metre.
    pub(crate) fn of(mesh: &CollisionWorld, body: crate::bot_physics::Body) -> Option<Self> {
        const CELL: f64 = 0.25;
        let bounds = mesh.stats().bounds?;
        let origin = [bounds.min[0] - 2.0 * CELL, bounds.min[2] - 2.0 * CELL];
        let nx = ((bounds.max[0] - bounds.min[0]) / CELL).ceil() as usize + 4;
        let nz = ((bounds.max[2] - bounds.min[2]) / CELL).ceil() as usize + 4;
        if nx * nz > 256 * 256 {
            return None;
        }
        let half = [
            body.radius,
            (body.height - body.max_step) * 0.5,
            body.radius,
        ];
        let mut free = vec![false; nx * nz];
        for z in 0..nz {
            for x in 0..nx {
                let at = [
                    origin[0] + (x as f64 + 0.5) * CELL,
                    body.max_step + half[1],
                    origin[1] + (z as f64 + 0.5) * CELL,
                ];
                let mut work = STRUCTURE_WORK_LIMIT;
                free[x + z * nx] = matches!(
                    mesh.posed_sweep_aabb(Pose::at([0.0; 3]), at, at, half, false, &mut work),
                    Ok(None)
                );
            }
        }
        // Free cells the outer border reaches are outside.
        let mut outside = vec![false; nx * nz];
        let mut stack: Vec<usize> = (0..nx * nz)
            .filter(|&i| {
                let (x, z) = (i % nx, i / nx);
                (x == 0 || z == 0 || x + 1 == nx || z + 1 == nz) && free[i]
            })
            .collect();
        for &i in &stack {
            outside[i] = true;
        }
        while let Some(i) = stack.pop() {
            let (x, z) = (i % nx, i / nx);
            let mut next = Vec::with_capacity(4);
            if x > 0 {
                next.push(i - 1);
            }
            if x + 1 < nx {
                next.push(i + 1);
            }
            if z > 0 {
                next.push(i - nx);
            }
            if z + 1 < nz {
                next.push(i + nx);
            }
            for j in next {
                if free[j] && !outside[j] {
                    outside[j] = true;
                    stack.push(j);
                }
            }
        }
        let room: Vec<bool> = (0..nx * nz).map(|i| free[i] && !outside[i]).collect();
        if room.iter().filter(|e| **e).count() < 16 {
            return None;
        }
        // The room's walls belong to it too, up to 1.5 m from its free space,
        // so a body leaving it is not caught halfway; free outside space
        // never does.
        let reach = (1.5 / CELL).ceil() as i64;
        let mut enclosed = room.clone();
        for i in (0..nx * nz).filter(|&i| room[i]) {
            let (x, z) = ((i % nx) as i64, (i / nx) as i64);
            for dz in -reach..=reach {
                for dx in -reach..=reach {
                    let (cx, cz) = (x + dx, z + dz);
                    if cx >= 0 && cz >= 0 && (cx as usize) < nx && (cz as usize) < nz {
                        let j = cx as usize + cz as usize * nx;
                        if !outside[j] {
                            enclosed[j] = true;
                        }
                    }
                }
            }
        }
        Some(Self {
            origin,
            cell: CELL,
            size: [nx, nz],
            enclosed,
        })
    }
}

type Interiors = std::collections::HashMap<String, Option<Arc<Interior>>>;
fn interiors() -> &'static std::sync::Mutex<Interiors> {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<Interiors>> = std::sync::OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// The analysed rooms of the mesh with this source hash; None while it is not
/// analysed or encloses no room.
fn interior(sha256: &str) -> Option<Arc<Interior>> {
    interiors().lock().ok()?.get(sha256).cloned().flatten()
}

/// Analyse every mesh of `snapshot` not analysed yet; returns how many.
pub(crate) fn analyse_interiors(snapshot: &StructureObstacles) -> usize {
    let body = crate::bot_physics::Body::default();
    let mut done = 0;
    for obstacle in snapshot.obstacles() {
        let sha = obstacle.mesh.source_sha256();
        if interiors()
            .lock()
            .is_ok_and(|cache| cache.contains_key(sha))
        {
            continue;
        }
        let room = Interior::of(&obstacle.mesh, body).map(Arc::new);
        if let Ok(mut cache) = interiors().lock() {
            cache.insert(sha.to_owned(), room);
        }
        done += 1;
    }
    done
}

/// What one off-tick analysis of a snapshot did.
pub(crate) struct AnalysisReport {
    key: (u64, u32),
    interiors_analysed: usize,
    gated_polygons: Option<usize>,
    reused_polygons: usize,
    gate_error: Option<&'static str>,
    analysis_us: u64,
}

/// A snapshot's enclosed-room and route-gate analysis on a worker thread.
pub(crate) struct Analysis {
    job: std::thread::JoinHandle<AnalysisReport>,
}

fn start_analysis(
    snapshot: Arc<StructureObstacles>,
    mesh: Option<Arc<crate::navmesh::NavMesh>>,
) -> Analysis {
    let job = std::thread::spawn(move || {
        let began = std::time::Instant::now();
        let interiors_analysed = analyse_interiors(&snapshot);
        let (gated_polygons, reused_polygons, gate_error) = match &mesh {
            Some(mesh) => {
                match crate::navmesh_structure_gates::install_now(mesh, Some(&snapshot)) {
                    Ok(Some((gated, reused))) => (Some(gated), reused, None),
                    Ok(None) => (None, 0, None),
                    Err(error) => (None, 0, Some(error)),
                }
            }
            None => (None, 0, None),
        };
        AnalysisReport {
            key: snapshot.key(),
            interiors_analysed,
            gated_polygons,
            reused_polygons,
            gate_error,
            analysis_us: began.elapsed().as_micros() as u64,
        }
    });
    Analysis { job }
}

impl<W: Write> crate::Engine<W> {
    /// The current mode's placed structures and its movement filter row.
    fn bot_structure_source(
        &self,
    ) -> Option<(
        &crate::domination_structural_world::SequenceCollisionState,
        u32,
    )> {
        match (&self.acquisition_gameplay, &self.domination_gameplay) {
            (Some(runtime), _) => runtime
                .blast_collision()
                .map(|c| (c.sequences, c.filter_rows[CHARACTER_QUERY_LAYER])),
            (None, Some(world)) => {
                let c = world.frag_blast_collision();
                Some((c.sequences, c.filter_rows[CHARACTER_QUERY_LAYER]))
            }
            (None, None) => self
                .sabotage_structures
                .as_ref()
                .map(crate::sabotage_structures::Runtime::collision),
        }
    }

    /// Place and remove the Sabotage objectives' collision for the states
    /// they are in now; each change is recorded as `sabotage_structures`.
    fn sync_sabotage_structures(&mut self, wall: u64) -> Result<(), &'static str> {
        let (Some(runtime), Some(world)) =
            (self.sabotage_structures.as_mut(), self.objective_world.as_ref())
        else {
            return Ok(());
        };
        for change in runtime.sync(world)? {
            self.record(
                serde_json::json!({
                    "op": "sabotage_structures",
                    "object_index": change.object_index,
                    "state": change.state,
                    "slot": change.sequence,
                    "packs": change.packs,
                }),
                wall,
            )?;
        }
        Ok(())
    }

    /// Bring the structure snapshot bot movement and routing see up to date
    /// with the current mode's structures. Cheap when nothing changed. A new
    /// snapshot is recorded as `bot_structure_obstacles`; its analysis runs on
    /// a worker thread and is recorded as `bot_structure_analysis` once done.
    pub(crate) fn refresh_bot_structure_obstacles(
        &mut self,
        wall: u64,
    ) -> Result<(), &'static str> {
        self.sync_sabotage_structures(wall)?;
        let wanted = self
            .bot_structure_source()
            .map(|(sequences, row)| (sequences.revision(), row));
        let current = self
            .collision_world
            .as_ref()
            .and_then(|w| w.bot_structures())
            .map(StructureObstacles::key);
        if self.collision_world.is_none() {
            return Ok(());
        }
        let mut records = Vec::new();
        if current != wanted {
            let began = std::time::Instant::now();
            let snapshot = self
                .bot_structure_source()
                .map(|(sequences, row)| Arc::new(StructureObstacles::new(sequences, row)));
            let snapshot_us = began.elapsed().as_micros() as u64;
            records.push(serde_json::json!({
                "op": "bot_structure_obstacles",
                "obstacles": snapshot.as_ref().map(|s| s.obstacles().iter().map(|o| serde_json::json!({
                    "label": o.label,
                    "min": o.min.map(|v| (v * 10.).round() / 10.),
                    "max": o.max.map(|v| (v * 10.).round() / 10.),
                })).collect::<Vec<_>>()),
                "movement_row": wanted.map(|k| k.1),
                "snapshot_us": snapshot_us,
            }));
            if let Some(world) = self.collision_world.as_mut() {
                world.set_bot_structures(snapshot);
            }
        }
        // Collect a finished analysis; then start one for a newer snapshot.
        if let Some(analysis) = self.bot_structure_analysis.take() {
            if analysis.job.is_finished() {
                if let Ok(report) = analysis.job.join() {
                    records.push(serde_json::json!({
                        "op": "bot_structure_analysis",
                        "current": Some(report.key) == wanted,
                        "interiors_analysed": report.interiors_analysed,
                        "gated_polygons": report.gated_polygons,
                        "reused_polygons": report.reused_polygons,
                        "gate_error": report.gate_error,
                        "analysis_us": report.analysis_us,
                    }));
                }
            } else {
                self.bot_structure_analysis = Some(analysis);
            }
        }
        if self.bot_structure_analysis.is_none() && self.bot_structure_analysed != wanted {
            let snapshot = self
                .collision_world
                .as_ref()
                .and_then(|w| w.bot_structures_arc().cloned());
            match snapshot {
                Some(snapshot) => {
                    self.bot_structure_analysis =
                        Some(start_analysis(snapshot, self.navmesh_world.clone()));
                }
                None => {
                    if let Some(mesh) = &self.navmesh_world {
                        crate::navmesh_structure_gates::install_now(mesh, None)?;
                    }
                }
            }
            self.bot_structure_analysed = wanted;
        }
        for record in records {
            self.record(record, wall)?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "bot_structure_collision_tests.rs"]
mod tests;
