//! Route gating of the static navmesh by the placed structures standing now.
//!
//! # Behaviour
//!
//! The navmesh is baked from the static world, so its polygons run straight
//! through gates, bunker shells and other placed structures. Polygons cannot
//! be split at runtime, so each polygon a structure touches is instead
//! described by where a bot can stand inside it:
//!
//! * the polygon is sampled on a grid of cells (0.25 m, coarser for very large
//!   polygons); a cell is free when the bot body standing on the polygon
//!   surface there, lifted by its step height so floors and sills do not
//!   count, does not penetrate a structure;
//! * free cells are grouped into connected regions;
//! * for each link to a neighbouring polygon, the regions touching the shared
//!   edge are recorded, and among them the "through" regions, which also
//!   touch another link (a dead-end pocket is not a way through).
//!
//! A route may cross the polygon from one link (or its start point) to another
//! link (or its goal) only when the two share a region. An entry edge that
//! several through regions touch (a wall crossing the edge itself) cannot
//! tell which one the route arrived in, so each of them must reach the exit.
//! The search pays a prohibitive cost for a sealed crossing, so it goes
//! around through any opening (another gate, a bunker door); when none
//! exists the route is cut at the sealed crossing and ends as a partial route
//! in front of it. Gates are rebuilt whenever the structure snapshot changes,
//! so a destroyed gate opens its road again.
use crate::bot_structure_collision::{Obstacle, STRUCTURE_WORK_LIMIT, StructureObstacles};
use crate::navmesh::{NavMesh, PolyGate};
use std::collections::{BTreeMap, BTreeSet, BinaryHeap};
use std::sync::Arc;

const CELL_M: f64 = 0.25;
const MAX_CELLS_PER_AXIS: usize = 192;
const MAX_REGIONS: usize = 64;
/// Extra room route legs keep from structures, beyond the body's own.
const STEER_MARGIN_M: f64 = 0.3;

/// Identities of the structures a polygon's gate was computed from.
pub(crate) type GateSources = Vec<(String, String, [u64; 12])>;
/// Gated polygons with their sources, and how many were reused.
type Gates = (Vec<(GateSources, PolyGate)>, usize);

/// Compute and install the gates of `snapshot` on `mesh` now, unless they
/// are current. A polygon whose structures are unchanged since the previous
/// gates reuses its gate. Returns (gated polygons, of them reused) when it
/// rebuilt the gates. Runs off the world tick; routes use the previous gates
/// until the new ones are installed.
pub(crate) fn install_now(
    mesh: &NavMesh,
    snapshot: Option<&Arc<StructureObstacles>>,
) -> Result<Option<(usize, usize)>, &'static str> {
    let key = snapshot.map(|s| s.key());
    if mesh.structure_gate_key() == key {
        return Ok(None);
    }
    let (gates, reused) = match snapshot {
        Some(snapshot) => match gates(mesh, snapshot) {
            Ok(gates) => gates,
            Err(error) => {
                // Never leave stale gates: routes fall back to the static mesh.
                mesh.set_structure_gates(Some(snapshot), &[], Default::default())
                    .map_err(|e| e.0)?;
                return Err(error);
            }
        },
        None => (Vec::new(), 0),
    };
    let count = gates.len();
    let (gates, cache): (Vec<PolyGate>, _) = gates
        .into_iter()
        .map(|(sources, gate)| (gate.clone(), (gate.poly, (sources, gate))))
        .unzip();
    mesh.set_structure_gates(snapshot, &gates, cache)
        .map_err(|e| e.0)?;
    Ok(Some((count, reused)))
}

/// Every gated polygon of `snapshot`, with the structures it was computed
/// from, and how many were reused from the installed gates.
fn gates(mesh: &NavMesh, snapshot: &StructureObstacles) -> Result<Gates, &'static str> {
    let body = mesh.body();
    let reach = body.radius + 2.0 * CELL_M;
    let mut polys = BTreeSet::new();
    for obstacle in snapshot.obstacles() {
        let min = [
            obstacle.min[0] - reach,
            obstacle.min[1] - body.height - 1.0,
            obstacle.min[2] - reach,
        ];
        let max = [
            obstacle.max[0] + reach,
            obstacle.max[1] + 1.0,
            obstacle.max[2] + reach,
        ];
        polys.extend(mesh.polys_in_box(min, max).map_err(|e| e.0)?);
    }
    let previous = mesh.structure_gate_cache();
    let mut gates = Vec::new();
    let mut reused = 0;
    for poly in polys {
        let info = mesh.poly_info(poly).map_err(|e| e.0)?;
        let relevant = relevant(snapshot, &info.vertices, body);
        if relevant.is_empty() {
            continue;
        }
        let sources: GateSources = relevant.iter().map(|o| o.identity()).collect();
        if let Some((old, gate)) = previous.get(&poly)
            && *old == sources
        {
            gates.push((sources, gate.clone()));
            reused += 1;
            continue;
        }
        if let Some(gate) = poly_gate(mesh, &relevant, poly, &info, body)? {
            gates.push((sources, gate));
        }
    }
    Ok((gates, reused))
}

/// Structures that can reach a body standing anywhere on the polygon.
fn relevant<'a>(
    snapshot: &'a StructureObstacles,
    vertices: &[[f64; 3]],
    body: crate::bot_physics::Body,
) -> Vec<&'a Obstacle> {
    if vertices.len() < 3 {
        return Vec::new();
    }
    let lo = |axis: usize| {
        vertices
            .iter()
            .map(|v| v[axis])
            .fold(f64::INFINITY, f64::min)
    };
    let hi = |axis: usize| {
        vertices
            .iter()
            .map(|v| v[axis])
            .fold(f64::NEG_INFINITY, f64::max)
    };
    snapshot
        .obstacles()
        .iter()
        .filter(|o| {
            o.min[0] <= hi(0) + body.radius
                && o.max[0] >= lo(0) - body.radius
                && o.min[2] <= hi(2) + body.radius
                && o.max[2] >= lo(2) - body.radius
                && o.min[1] <= hi(1) + body.height + 0.5
                && o.max[1] >= lo(1) - 0.5
        })
        .collect()
}

fn inside_convex(vertices: &[[f64; 3]], x: f64, z: f64) -> bool {
    let n = vertices.len();
    let mut sign = 0.0;
    for i in 0..n {
        let a = vertices[i];
        let b = vertices[(i + 1) % n];
        let cross = (b[0] - a[0]) * (z - a[2]) - (b[2] - a[2]) * (x - a[0]);
        if cross.abs() < 1e-9 {
            continue;
        }
        if sign == 0.0 {
            sign = cross.signum();
        } else if cross.signum() != sign {
            return false;
        }
    }
    true
}

fn poly_gate(
    mesh: &NavMesh,
    relevant: &[&Obstacle],
    poly: u64,
    info: &crate::navmesh::PolyInfo,
    body: crate::bot_physics::Body,
) -> Result<Option<PolyGate>, &'static str> {
    let fold = |axis: usize, f: fn(f64, f64) -> f64, init: f64| {
        info.vertices.iter().map(|v| v[axis]).fold(init, f)
    };
    let (min_x, max_x) = (
        fold(0, f64::min, f64::INFINITY),
        fold(0, f64::max, f64::NEG_INFINITY),
    );
    let (min_z, max_z) = (
        fold(2, f64::min, f64::INFINITY),
        fold(2, f64::max, f64::NEG_INFINITY),
    );
    let extent = (max_x - min_x).max(max_z - min_z);
    let cell = CELL_M.max(extent / MAX_CELLS_PER_AXIS as f64);
    let nx = ((max_x - min_x) / cell).ceil().max(1.0) as usize;
    let nz = ((max_z - min_z) / cell).ceil().max(1.0) as usize;
    let centre = |x: usize, z: usize| {
        [
            min_x + (x as f64 + 0.5) * cell,
            min_z + (z as f64 + 0.5) * cell,
        ]
    };
    let mut cells = Vec::new();
    for z in 0..nz {
        for x in 0..nx {
            let [px, pz] = centre(x, z);
            if inside_convex(&info.vertices, px, pz) {
                cells.push((x, z, [px, pz]));
            }
        }
    }
    let xz: Vec<[f64; 2]> = cells.iter().map(|c| c.2).collect();
    let heights = mesh.poly_heights(poly, &xz).map_err(|e| e.0)?;
    // 0: outside or blocked; 1: free (labelled below).
    let mut grid = vec![0u8; nx * nz];
    let mut blocked_any = false;
    let lift = body.max_step;
    let half = [body.radius, (body.height - lift) * 0.5, body.radius];
    for ((x, z, [px, pz]), height) in cells.iter().zip(heights) {
        let Some(y) = height else { continue };
        let at = [*px, y + lift + half[1], *pz];
        let mut blocked = false;
        for obstacle in relevant {
            let mut work = STRUCTURE_WORK_LIMIT;
            if obstacle
                .mesh
                .posed_sweep_aabb(obstacle.pose, at, at, half, false, &mut work)?
                .is_some()
            {
                blocked = true;
                break;
            }
        }
        if blocked {
            blocked_any = true;
        } else {
            grid[x + z * nx] = 1;
        }
    }
    if !blocked_any {
        return Ok(None);
    }
    // Connected free regions, 4-neighbour.
    let mut labels = vec![0u8; nx * nz];
    let mut regions = 0usize;
    for start in 0..grid.len() {
        if grid[start] == 0 || labels[start] != 0 {
            continue;
        }
        regions += 1;
        if regions > MAX_REGIONS {
            // Too fragmented to describe: leave the polygon to body physics.
            return Ok(None);
        }
        labels[start] = regions as u8;
        let mut stack = vec![start];
        while let Some(i) = stack.pop() {
            let (x, z) = (i % nx, i / nx);
            let mut visit = |j: usize| {
                if grid[j] != 0 && labels[j] == 0 {
                    labels[j] = regions as u8;
                    stack.push(j);
                }
            };
            if x > 0 {
                visit(i - 1);
            }
            if x + 1 < nx {
                visit(i + 1);
            }
            if z > 0 {
                visit(i - nx);
            }
            if z + 1 < nz {
                visit(i + nx);
            }
        }
    }
    // Regions at each shared edge: the labelled cells nearest each edge sample.
    let near_cells = |p: [f64; 2]| -> u64 {
        let cx = ((p[0] - min_x) / cell).floor() as i64;
        let cz = ((p[1] - min_z) / cell).floor() as i64;
        let mut mask = 0u64;
        for dz in -1..=1 {
            for dx in -1..=1 {
                let (x, z) = (cx + dx, cz + dz);
                if x < 0 || z < 0 || x >= nx as i64 || z >= nz as i64 {
                    continue;
                }
                let label = labels[x as usize + z as usize * nx];
                if label != 0 {
                    mask |= 1 << (label - 1);
                }
            }
        }
        mask
    };
    let mut links = Vec::with_capacity(info.links.len());
    for (neighbour, left, right) in &info.links {
        let length = (right[0] - left[0]).hypot(right[2] - left[2]);
        let samples = ((length / (cell * 0.5)).ceil() as usize).max(1);
        let mut mask = 0u64;
        for k in 0..=samples {
            let t = k as f64 / samples as f64;
            mask |= near_cells([
                left[0] + (right[0] - left[0]) * t,
                left[2] + (right[2] - left[2]) * t,
            ]);
        }
        links.push((*neighbour, mask));
    }
    // A through region touches at least two links.
    let mut seen = 0u64;
    let mut through = 0u64;
    for &(_, mask) in &links {
        through |= seen & mask;
        seen |= mask;
    }
    Ok(Some(PolyGate {
        poly,
        origin_xz: [min_x, min_z],
        cell,
        size: [nx, nz],
        labels,
        links: links
            .into_iter()
            .map(|(neighbour, mask)| (neighbour, mask, mask & through))
            .collect(),
    }))
}

/// The body as structure tests see it: lifted by its step height, so floors,
/// sills and the ground under a structure do not count.
fn lifted(body: crate::bot_physics::Body) -> (f64, [f64; 3]) {
    let half = [
        body.radius,
        (body.height - body.max_step) * 0.5,
        body.radius,
    ];
    (body.max_step + half[1], half)
}

/// Steer a route's straight path around structures standing inside its
/// corridor polygons. The polygon gates already chose a corridor whose
/// polygons connect around every structure; a segment the body cannot follow
/// straight is replaced by a shortest grid path over free cells inside those
/// polygons, then shortened to the fewest straight legs. A segment that no
/// grid path replaces is kept: body physics still stops at the structure.
pub(crate) fn refine(
    mesh: &NavMesh,
    snapshot: &StructureObstacles,
    waypoints: Vec<[f64; 3]>,
    corridor: &[u64],
) -> Result<Vec<[f64; 3]>, crate::navmesh::Error> {
    if waypoints.len() < 2 || snapshot.obstacles().is_empty() {
        return Ok(waypoints);
    }
    let body = mesh.body();
    let (up, half) = lifted(body);
    // Legs keep this much more room from a structure than the body needs,
    // so bots crossing in opposite directions next to a wall can pass; a
    // passage narrower than that is still used at the body's own width.
    let wide = [half[0] + STEER_MARGIN_M, half[1], half[2] + STEER_MARGIN_M];
    let centre = |p: [f64; 3]| [p[0], p[1] + up, p[2]];
    let clear = |a: [f64; 3], b: [f64; 3]| -> bool {
        let mut work = STRUCTURE_WORK_LIMIT;
        matches!(
            snapshot.sweep_aabb(centre(a), centre(b), wide, &mut work),
            Ok(None)
        )
    };
    if waypoints.windows(2).all(|w| clear(w[0], w[1])) {
        return Ok(waypoints);
    }
    let mut polygons: Option<Vec<Vec<[f64; 3]>>> = None;
    let mut out = vec![waypoints[0]];
    for w in waypoints.windows(2) {
        if clear(w[0], w[1]) {
            out.push(w[1]);
            continue;
        }
        let polygons = match &mut polygons {
            Some(p) => p,
            None => polygons.insert(
                corridor
                    .iter()
                    .filter_map(|&poly| mesh.poly_info(poly).ok().map(|i| i.vertices))
                    .collect(),
            ),
        };
        match detour(snapshot, polygons, w[0], w[1], wide, up)
            .or_else(|| detour(snapshot, polygons, w[0], w[1], half, up))
        {
            Some(points) => out.extend(points),
            None => out.push(w[1]),
        }
    }
    Ok(out)
}

/// Grid path from `a` to `b` (excluding `a`) over cells inside the corridor
/// polygons where the lifted body clears every structure.
fn detour(
    snapshot: &StructureObstacles,
    polygons: &[Vec<[f64; 3]>],
    a: [f64; 3],
    b: [f64; 3],
    half: [f64; 3],
    up: f64,
) -> Option<Vec<[f64; 3]>> {
    const MARGIN_M: f64 = 8.0;
    const MAX_AXIS: usize = 320;
    const MAX_EXPANSIONS: usize = 8_000;
    let min = [a[0].min(b[0]) - MARGIN_M, a[2].min(b[2]) - MARGIN_M];
    let max = [a[0].max(b[0]) + MARGIN_M, a[2].max(b[2]) + MARGIN_M];
    let cell = CELL_M.max((max[0] - min[0]).max(max[1] - min[1]) / MAX_AXIS as f64);
    let nx = ((max[0] - min[0]) / cell).ceil() as i64;
    let nz = ((max[1] - min[1]) / cell).ceil() as i64;
    let length = (b[0] - a[0]).hypot(b[2] - a[2]).max(1e-9);
    // Height along the segment; structures sit on locally even ground.
    let height = |x: f64, z: f64| {
        let t = (((x - a[0]) * (b[0] - a[0]) + (z - a[2]) * (b[2] - a[2])) / (length * length))
            .clamp(0.0, 1.0);
        a[1] + (b[1] - a[1]) * t
    };
    let point = |c: (i64, i64)| {
        let x = min[0] + (c.0 as f64 + 0.5) * cell;
        let z = min[1] + (c.1 as f64 + 0.5) * cell;
        [x, height(x, z), z]
    };
    let window: Vec<&Vec<[f64; 3]>> = polygons
        .iter()
        .filter(|p| {
            let lo = |axis: usize| p.iter().map(|v| v[axis]).fold(f64::INFINITY, f64::min);
            let hi = |axis: usize| p.iter().map(|v| v[axis]).fold(f64::NEG_INFINITY, f64::max);
            lo(0) <= max[0] && hi(0) >= min[0] && lo(2) <= max[1] && hi(2) >= min[1]
        })
        .collect();
    let mut free_cache: BTreeMap<(i64, i64), bool> = BTreeMap::new();
    let mut free = |c: (i64, i64)| -> bool {
        if c.0 < 0 || c.1 < 0 || c.0 >= nx || c.1 >= nz {
            return false;
        }
        *free_cache.entry(c).or_insert_with(|| {
            let p = point(c);
            if !window.iter().any(|poly| inside_convex(poly, p[0], p[2])) {
                return false;
            }
            let mut work = STRUCTURE_WORK_LIMIT;
            !snapshot
                .obstacles()
                .iter()
                .filter(|o| {
                    o.min[0] <= p[0] + half[0]
                        && o.max[0] >= p[0] - half[0]
                        && o.min[2] <= p[2] + half[2]
                        && o.max[2] >= p[2] - half[2]
                })
                .any(|o| {
                    let at = [p[0], p[1] + up, p[2]];
                    !matches!(
                        o.mesh
                            .posed_sweep_aabb(o.pose, at, at, half, false, &mut work),
                        Ok(None)
                    )
                })
        })
    };
    let cell_of = |p: [f64; 3]| {
        (
            ((p[0] - min[0]) / cell).floor() as i64,
            ((p[2] - min[1]) / cell).floor() as i64,
        )
    };
    // Nearest free cell to an endpoint, within a metre.
    let settle = |c: (i64, i64), free: &mut dyn FnMut((i64, i64)) -> bool| {
        let reach = (1.0 / cell).ceil() as i64;
        let mut best = None;
        for dz in -reach..=reach {
            for dx in -reach..=reach {
                let d = dx * dx + dz * dz;
                if best.is_some_and(|(bd, _)| bd <= d) {
                    continue;
                }
                if free((c.0 + dx, c.1 + dz)) {
                    best = Some((d, (c.0 + dx, c.1 + dz)));
                }
            }
        }
        best.map(|(_, c)| c)
    };
    let start = settle(cell_of(a), &mut free)?;
    let goal = settle(cell_of(b), &mut free)?;
    // A* over 8 neighbours; a diagonal needs both of its side cells free.
    let h = |c: (i64, i64)| {
        let (dx, dz) = ((c.0 - goal.0).abs(), (c.1 - goal.1).abs());
        (dx.max(dz) * 10 + dx.min(dz) * 4) as u64
    };
    let mut open = BinaryHeap::new();
    let mut cost: BTreeMap<(i64, i64), u64> = BTreeMap::new();
    let mut parent: BTreeMap<(i64, i64), (i64, i64)> = BTreeMap::new();
    cost.insert(start, 0);
    open.push(std::cmp::Reverse((h(start), start)));
    let mut expansions = 0;
    let mut found = false;
    while let Some(std::cmp::Reverse((_, c))) = open.pop() {
        if c == goal {
            found = true;
            break;
        }
        expansions += 1;
        if expansions > MAX_EXPANSIONS {
            return None;
        }
        let base = cost[&c];
        for (dx, dz) in [
            (1, 0),
            (-1, 0),
            (0, 1),
            (0, -1),
            (1, 1),
            (1, -1),
            (-1, 1),
            (-1, -1),
        ] {
            let n = (c.0 + dx, c.1 + dz);
            if !free(n)
                || (dx != 0 && dz != 0 && (!free((c.0 + dx, c.1)) || !free((c.0, c.1 + dz))))
            {
                continue;
            }
            let next = base + if dx != 0 && dz != 0 { 14 } else { 10 };
            if cost.get(&n).is_none_or(|&old| next < old) {
                cost.insert(n, next);
                parent.insert(n, c);
                open.push(std::cmp::Reverse((next + h(n), n)));
            }
        }
    }
    if !found {
        return None;
    }
    let mut cells = vec![goal];
    while let Some(&p) = parent.get(cells.last().unwrap()) {
        cells.push(p);
    }
    cells.reverse();
    // Fewest straight legs whose cells are all free.
    let mut line_free = |from: (i64, i64), to: (i64, i64)| {
        let steps = (to.0 - from.0).abs().max((to.1 - from.1).abs()) * 2;
        (0..=steps.max(1)).all(|k| {
            let t = k as f64 / steps.max(1) as f64;
            let x = from.0 as f64 + (to.0 - from.0) as f64 * t;
            let z = from.1 as f64 + (to.1 - from.1) as f64 * t;
            // Every cell the leg's body footprint touches.
            [
                (x.floor(), z.floor()),
                (x.ceil(), z.ceil()),
                (x.floor(), z.ceil()),
                (x.ceil(), z.floor()),
            ]
            .into_iter()
            .all(|(cx, cz)| free((cx as i64, cz as i64)))
        })
    };
    let mut legs = Vec::new();
    let mut i = 0;
    while i + 1 < cells.len() {
        let mut j = cells.len() - 1;
        while j > i + 1 && !line_free(cells[i], cells[j]) {
            j -= 1;
        }
        legs.push(point(cells[j]));
        i = j;
    }
    legs.pop();
    legs.push(b);
    Some(legs)
}
