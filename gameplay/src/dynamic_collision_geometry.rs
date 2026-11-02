//! Bounded convex-core distance bounds. Shape radii are applied separately.
use super::{Geometry, Pose, QueryLimits};
pub type Point = [f64; 3];
pub fn add(a: Point, b: Point) -> Point {
    std::array::from_fn(|i| a[i] + b[i])
}
pub fn sub(a: Point, b: Point) -> Point {
    std::array::from_fn(|i| a[i] - b[i])
}
fn scale(a: Point, s: f64) -> Point {
    a.map(|v| v * s)
}
pub fn dot(a: Point, b: Point) -> f64 {
    (a[0] * b[0] + a[1] * b[1]) + a[2] * b[2]
}
pub fn rotate(p: Pose, v: Point) -> Point {
    p.rotation.map(|row| dot(row, v))
}
fn local_direction(p: Pose, v: Point) -> Point {
    std::array::from_fn(|i| {
        (p.rotation[0][i] * v[0] + p.rotation[1][i] * v[1]) + p.rotation[2][i] * v[2]
    })
}
pub(crate) fn support(
    shape: &Geometry,
    pose: Pose,
    direction: Point,
    budget: &mut usize,
) -> Result<Point, &'static str> {
    let direction = local_direction(pose, direction);
    let mut visit = |p| {
        if *budget == 0 {
            return Err("dynamic_collision_work_limit");
        }
        *budget -= 1;
        Ok(p)
    };
    let local = match shape {
        Geometry::Sphere { .. } => visit([0.; 3])?,
        Geometry::Capsule { a, b, .. } => {
            visit(*a)?;
            visit(*b)?;
            if dot(*a, direction) >= dot(*b, direction) {
                *a
            } else {
                *b
            }
        }
        Geometry::Box { half_extents, .. } => visit(std::array::from_fn(|i| {
            if direction[i] >= 0. {
                half_extents[i]
            } else {
                -half_extents[i]
            }
        }))?,
        Geometry::Hull { vertices, .. } => hull_support(vertices, direction, &mut visit)?,
        Geometry::Triangle { vertices, .. } => hull_support(vertices, direction, &mut visit)?,
    };
    Ok(add(pose.translation, rotate(pose, local)))
}
fn hull_support(
    vertices: &[Point],
    direction: Point,
    visit: &mut impl FnMut(Point) -> Result<Point, &'static str>,
) -> Result<Point, &'static str> {
    let mut best = visit(vertices[0])?;
    let mut score = dot(best, direction);
    for &v in &vertices[1..] {
        visit(v)?;
        let next = dot(v, direction);
        if next > score {
            best = v;
            score = next;
        }
    }
    Ok(best)
}
// Project origin onto every nonempty simplex face. Singular affine faces are
// handled by their lower-dimensional subsets, not arbitrary direction nudges.
fn closest(simplex: &[Point]) -> Result<(Point, Vec<Point>), &'static str> {
    let mut best: Option<(f64, Point, Vec<Point>)> = None;
    for mask in 1..(1usize << simplex.len()) {
        let points: Vec<_> = simplex
            .iter()
            .enumerate()
            .filter(|(i, _)| mask & (1 << i) != 0)
            .map(|(_, p)| *p)
            .collect();
        let n = points.len() - 1;
        let mut matrix = [[0.; 4]; 3];
        let edges: Vec<_> = points[1..].iter().map(|&v| sub(v, points[0])).collect();
        for i in 0..n {
            for j in 0..n {
                matrix[i][j] = dot(edges[i], edges[j]);
            }
            matrix[i][n] = -dot(edges[i], points[0]);
        }
        let mut singular = false;
        for col in 0..n {
            let pivot = (col..n)
                .max_by(|&i, &j| matrix[i][col].abs().total_cmp(&matrix[j][col].abs()))
                .unwrap();
            if matrix[pivot][col].abs() < 1e-24 {
                singular = true;
                break;
            }
            matrix.swap(pivot, col);
            let divisor = matrix[col][col];
            for value in &mut matrix[col][col..=n] {
                *value /= divisor;
            }
            let pivot_row = matrix[col];
            for (i, row) in matrix.iter_mut().enumerate().take(n) {
                if i != col {
                    let factor = row[col];
                    for (value, pivot) in row[col..=n].iter_mut().zip(&pivot_row[col..=n]) {
                        *value -= factor * pivot;
                    }
                }
            }
        }
        if singular {
            continue;
        }
        let mut weights = vec![1.; points.len()];
        for i in 0..n {
            weights[i + 1] = matrix[i][n];
            weights[0] -= weights[i + 1];
        }
        if weights.iter().any(|&v| v < 0. || !v.is_finite()) {
            continue;
        }
        let mut point = [0.; 3];
        let mut kept = Vec::new();
        for (&p, &w) in points.iter().zip(&weights) {
            point = add(point, scale(p, w));
            if w > 0. {
                kept.push(p);
            }
        }
        let squared = dot(point, point);
        if squared.is_finite() && best.as_ref().is_none_or(|(old, ..)| squared < *old) {
            best = Some((squared, point, kept));
        }
    }
    best.map(|(_, point, kept)| (point, kept))
        .ok_or("dynamic_collision_simplex_failed")
}
pub fn penetrates(
    a: &Geometry,
    ap: Pose,
    b: &Geometry,
    bp: Pose,
    limits: QueryLimits,
    budget: &mut usize,
) -> Result<bool, &'static str> {
    let radius = a.radius() + b.radius();
    let support_pair = |direction, budget: &mut usize| -> Result<Point, &'static str> {
        Ok(sub(
            support(a, ap, direction, budget)?,
            support(b, bp, scale(direction, -1.), budget)?,
        ))
    };
    let mut simplex = vec![support_pair([1., 0., 0.], budget)?];
    let mut last_upper = f64::INFINITY;
    for _ in 0..limits.max_iterations {
        let (nearest, face) = closest(&simplex)?;
        simplex = face;
        let upper = dot(nearest, nearest).sqrt();
        if !upper.is_finite() {
            return Err("dynamic_collision_nonfinite_distance");
        }
        if upper + limits.boundary_epsilon < radius {
            return Ok(true);
        }
        if upper == 0. {
            return if radius > 0. {
                Ok(true)
            } else {
                Err("dynamic_collision_boundary_uncertain")
            };
        }
        let p = support_pair(scale(nearest, -1.), budget)?;
        let lower = (dot(nearest, p) / upper).max(0.);
        if lower > upper + limits.boundary_epsilon.max(upper * 1e-12) {
            return Err("dynamic_collision_inconsistent_distance_bounds");
        }
        if lower > radius + limits.boundary_epsilon {
            return Ok(false);
        }
        if simplex.contains(&p)
            || (last_upper.is_finite()
                && (last_upper - upper).abs() <= f64::EPSILON * last_upper.max(1.))
        {
            return Err("dynamic_collision_boundary_uncertain");
        }
        if simplex.len() >= 4 {
            return Err("dynamic_collision_simplex_failed");
        }
        simplex.push(p);
        last_upper = upper;
    }
    Err("dynamic_collision_iteration_limit")
}

pub(crate) struct Distance {
    pub lower: f64,
    pub upper: f64,
    /// Obstacle B toward moving A. None means intersecting convex cores.
    pub normal: Option<Point>,
}
/// Closest convex-core bounds, refined independently of the contact radius.
pub(crate) fn distance(
    a: &Geometry,
    ap: Pose,
    b: &Geometry,
    bp: Pose,
    limits: QueryLimits,
    budget: &mut usize,
) -> Result<Distance, &'static str> {
    let pair = |direction, budget: &mut usize| -> Result<Point, &'static str> {
        Ok(sub(
            support(a, ap, direction, budget)?,
            support(b, bp, scale(direction, -1.), budget)?,
        ))
    };
    let mut simplex = vec![pair([1., 0., 0.], budget)?];
    for _ in 0..limits.max_iterations {
        let (nearest, face) = closest(&simplex)?;
        simplex = face;
        let upper = dot(nearest, nearest).sqrt();
        if !upper.is_finite() {
            return Err("dynamic_collision_nonfinite_distance");
        }
        if upper <= limits.boundary_epsilon {
            return Ok(Distance {
                lower: 0.,
                upper,
                normal: None,
            });
        }
        let normal = scale(nearest, 1. / upper);
        let p = pair(scale(normal, -1.), budget)?;
        let lower = dot(normal, p).max(0.);
        if lower > upper + limits.boundary_epsilon.max(upper * 1e-12) {
            return Err("dynamic_collision_inconsistent_distance_bounds");
        }
        if upper - lower <= limits.boundary_epsilon {
            return Ok(Distance {
                lower,
                upper,
                normal: Some(normal),
            });
        }
        if simplex.contains(&p) || simplex.len() >= 4 {
            return Err("dynamic_collision_distance_unresolved");
        }
        simplex.push(p);
    }
    Err("dynamic_collision_iteration_limit")
}

/// Conservative advancement of a fixed-orientation convex A. Explicit contact
/// tolerance is replacement tuning, not a claimed universal native default.
pub(crate) struct SweepLimits {
    pub tolerance: f64,
    pub extra: f64,
    pub query: QueryLimits,
}
pub(crate) fn sweep(
    a: &Geometry,
    ap: Pose,
    target: Point,
    b: &Geometry,
    bp: Pose,
    settings: SweepLimits,
    budget: &mut usize,
) -> Result<Option<(f64, Point)>, &'static str> {
    let SweepLimits {
        tolerance,
        extra,
        query: limits,
    } = settings;
    let delta = sub(target, ap.translation);
    let radius = a.radius() + b.radius();
    let mut fraction = 0.;
    for _ in 0..limits.max_iterations {
        let mut pose = ap;
        pose.translation = add(ap.translation, scale(delta, fraction));
        let d = distance(a, pose, b, bp, limits, budget)?;
        let (signed, normal) = if let Some(normal) = d.normal {
            (d.upper - radius, normal)
        } else {
            let (depth, normal) =
                crate::convex_penetration::penetration(a, pose, b, bp, limits, budget)?;
            (-depth - radius, normal)
        };
        let projected = dot(delta, normal);
        if fraction == 0.
            && signed <= tolerance
            && (signed + projected > 0. || projected + extra >= 0.)
        {
            return Ok(None);
        }
        if signed <= tolerance {
            return Ok(Some((fraction, normal)));
        }
        if projected >= 0. {
            return Ok(None);
        }
        let step = (d.lower - radius - tolerance) / -projected;
        if step < 0. {
            return Err("world_cast_distance_unresolved");
        }
        let next = fraction + step;
        if next > 1. {
            return Ok(None);
        }
        if next <= fraction || step <= limits.boundary_epsilon {
            return Ok(Some((fraction, normal)));
        }
        fraction = next;
    }
    Err("world_cast_iteration_limit")
}
