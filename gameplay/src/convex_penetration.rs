//! Bounded convex-core penetration. External shape radii are applied by callers.
use crate::dynamic_collision::geometry::{dot, sub, support};
use crate::dynamic_collision::{Geometry, Pose, QueryLimits};
type Point = [f64; 3];

fn scale(a: Point, s: f64) -> Point {
    a.map(|v| v * s)
}
fn cross(a: Point, b: Point) -> Point {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn length(a: Point) -> f64 {
    dot(a, a).sqrt()
}
fn charge(budget: &mut usize) -> Result<(), &'static str> {
    if *budget == 0 {
        return Err("dynamic_collision_work_limit");
    }
    *budget -= 1;
    Ok(())
}
#[derive(Clone, Copy)]
struct Plane {
    normal: Point,
    distance: f64,
}

/// Supporting planes of the current inner support hull. Flat hulls additionally
/// include supporting edge planes so coplanar disjoint origins cannot pass as
/// touching. Every returned plane has all sampled points on its inner side.
fn planes(points: &[Point], epsilon: f64, budget: &mut usize) -> Result<Vec<Plane>, &'static str> {
    let origin = points[0];
    let edge = points
        .iter()
        .map(|&p| sub(p, origin))
        .max_by(|&a, &b| dot(a, a).total_cmp(&dot(b, b)))
        .unwrap();
    let span = length(edge);
    if span <= epsilon {
        return Err("convex_penetration_indeterminate_direction");
    }
    let axis = scale(edge, 1. / span);
    let perpendicular = points
        .iter()
        .map(|&p| cross(axis, sub(p, origin)))
        .max_by(|&a, &b| dot(a, a).total_cmp(&dot(b, b)))
        .unwrap();
    let area = length(perpendicular);
    let mut result = Vec::new();
    let mut push = |normal: Point, distance: f64| {
        if !result.iter().any(|p: &Plane| {
            dot(p.normal, normal) > 1. - 1e-12 && (p.distance - distance).abs() <= epsilon
        }) {
            result.push(Plane { normal, distance });
        }
    };
    if area <= epsilon {
        // One-dimensional core: endpoints plus two independent line normals.
        for n in [axis, scale(axis, -1.)] {
            let d = points
                .iter()
                .map(|&p| dot(n, p))
                .fold(f64::NEG_INFINITY, f64::max);
            push(n, d);
        }
        let component = (0..3)
            .min_by(|&i, &j| axis[i].abs().total_cmp(&axis[j].abs()))
            .unwrap();
        let mut basis = [0.; 3];
        basis[component] = 1.;
        let first = cross(axis, basis);
        let first = scale(first, 1. / length(first));
        let second = cross(axis, first);
        for n in [first, scale(first, -1.), second, scale(second, -1.)] {
            push(n, dot(n, origin));
        }
        return Ok(result);
    }
    let flat_normal = scale(perpendicular, 1. / area);
    let flat = points
        .iter()
        .all(|&p| dot(flat_normal, sub(p, origin)).abs() <= epsilon);
    if flat {
        for n in [flat_normal, scale(flat_normal, -1.)] {
            push(n, dot(n, origin));
        }
        for i in 0..points.len() {
            for j in i + 1..points.len() {
                charge(budget)?;
                let n = cross(sub(points[j], points[i]), flat_normal);
                let len = length(n);
                if len <= epsilon {
                    continue;
                }
                let n = scale(n, 1. / len);
                let d = dot(n, points[i]);
                let mut positive = false;
                let mut negative = false;
                for &p in points {
                    charge(budget)?;
                    let side = dot(n, p) - d;
                    positive |= side > epsilon;
                    negative |= side < -epsilon;
                }
                if !positive {
                    push(n, d);
                } else if !negative {
                    push(scale(n, -1.), -d);
                }
            }
        }
    } else {
        for i in 0..points.len() {
            for j in i + 1..points.len() {
                for k in j + 1..points.len() {
                    charge(budget)?;
                    let e = sub(points[j], points[i]);
                    let en = length(e);
                    if en <= epsilon {
                        continue;
                    }
                    let n = cross(scale(e, 1. / en), sub(points[k], points[i]));
                    let len = length(n);
                    if len <= epsilon {
                        continue;
                    }
                    let n = scale(n, 1. / len);
                    let d = dot(n, points[i]);
                    let mut positive = false;
                    let mut negative = false;
                    for &p in points {
                        charge(budget)?;
                        let side = dot(n, p) - d;
                        positive |= side > epsilon;
                        negative |= side < -epsilon;
                        if positive && negative {
                            break;
                        }
                    }
                    if !positive {
                        push(n, d);
                    } else if !negative {
                        push(scale(n, -1.), -d);
                    }
                }
            }
        }
    }
    if result.is_empty() {
        return Err("convex_penetration_hull_unresolved");
    }
    Ok(result)
}

/// Expands an inner Minkowski support hull until its nearest supporting plane
/// agrees with an exact support query within the configured distance tolerance.
/// This is bounded polytope refinement, not a claim of native Havok arithmetic.
/// Positive depth excludes external radii; outward A escape is minus the
/// Minkowski boundary's outward normal. Work exhaustion never fabricates depth.
pub(crate) fn penetration(
    a: &Geometry,
    ap: Pose,
    b: &Geometry,
    bp: Pose,
    limits: QueryLimits,
    budget: &mut usize,
) -> Result<(f64, [f64; 3]), &'static str> {
    if !limits.boundary_epsilon.is_finite()
        || limits.boundary_epsilon < 0.
        || limits.max_iterations == 0
    {
        return Err("invalid_convex_penetration_limits");
    }
    let pair = |direction: Point, budget: &mut usize| -> Result<Point, &'static str> {
        let p = sub(
            support(a, ap, direction, budget)?,
            support(b, bp, scale(direction, -1.), budget)?,
        );
        if !p.iter().all(|v| v.is_finite()) {
            return Err("convex_penetration_nonfinite");
        }
        Ok(p)
    };
    let mut points = Vec::new();
    // Axis and diagonal extremes seed a closed support hull without relying on
    // a particular zero-distance GJK simplex or a center-derived fake normal.
    for x in -1..=1 {
        for y in -1..=1 {
            for z in -1..=1 {
                if x == 0 && y == 0 && z == 0 {
                    continue;
                }
                let p = pair([x as f64, y as f64, z as f64], budget)?;
                if !points.contains(&p) {
                    points.push(p);
                }
            }
        }
    }
    let extent = points.iter().map(|&p| length(p)).fold(1., f64::max);
    let epsilon = limits.boundary_epsilon.max(extent * 1e-12);
    for _ in 0..limits.max_iterations {
        let hull = planes(&points, epsilon, budget)?;
        // Negative planes must be expanded first to establish that the origin
        // is inside the true hull, including lower-dimensional containment.
        let nearest = hull
            .into_iter()
            .min_by(|a, b| a.distance.total_cmp(&b.distance))
            .unwrap();
        let p = pair(nearest.normal, budget)?;
        let gap = dot(nearest.normal, p) - nearest.distance;
        if !gap.is_finite() || gap < -epsilon {
            return Err("convex_penetration_inconsistent_support");
        }
        if gap <= epsilon {
            if nearest.distance < -epsilon {
                return Err("convex_penetration_disjoint_cores");
            }
            return Ok((nearest.distance.max(0.), scale(nearest.normal, -1.)));
        }
        if points.len() >= 96 || points.iter().any(|&q| length(sub(p, q)) <= epsilon) {
            return Err("convex_penetration_hull_unresolved");
        }
        points.push(p);
    }
    Err("convex_penetration_iteration_limit")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn box_core() -> Geometry {
        Geometry::Box {
            half_extents: [1.; 3],
            radius: 0.,
        }
    }
    fn query(
        a: &Geometry,
        ap: Pose,
        b: &Geometry,
        bp: Pose,
    ) -> Result<(f64, [f64; 3]), &'static str> {
        penetration(a, ap, b, bp, QueryLimits::default(), &mut 1_000_000)
    }
    #[test]
    fn overlapping_boxes_choose_minimum_exit_and_outward_normal() {
        let (d, n) = query(
            &box_core(),
            Pose::at([1.5, 0., 0.]),
            &box_core(),
            Pose::at([0.; 3]),
        )
        .unwrap();
        assert!((d - 0.5).abs() < 1e-7, "depth {d}");
        assert!(
            n[0] > 0.999999 && n[1].abs() < 1e-7 && n[2].abs() < 1e-7,
            "normal {n:?}"
        );
    }
    #[test]
    fn point_inside_box_ignores_external_radius() {
        let (d, n) = query(
            &Geometry::Sphere { radius: 9. },
            Pose::at([0.7, 0.1, 0.]),
            &box_core(),
            Pose::at([0.; 3]),
        )
        .unwrap();
        assert!((d - 0.3).abs() < 1e-7, "depth {d}");
        assert!(n[0] > 0.999999);
    }
    #[test]
    fn capsule_segment_inside_hull_uses_real_exit_plane() {
        let hull = Geometry::Hull {
            vertices: vec![
                [-1., -1., -1.],
                [-1., -1., 1.],
                [-1., 1., -1.],
                [-1., 1., 1.],
                [1., -1., -1.],
                [1., -1., 1.],
                [1., 1., -1.],
                [1., 1., 1.],
            ],
            radius: 0.,
        };
        let cap = Geometry::Capsule {
            a: [0., -0.5, 0.],
            b: [0., 0.5, 0.],
            radius: 0.4,
        };
        let (d, n) = query(&cap, Pose::at([0.8, 0., 0.]), &hull, Pose::at([0.; 3])).unwrap();
        assert!((d - 0.2).abs() < 1e-7, "depth {d}");
        assert!(n[0] > 0.999999);
    }
    #[test]
    fn triangle_touch_has_a_certified_plane() {
        let tri = Geometry::Triangle {
            vertices: [[-1., 0., -1.], [1., 0., -1.], [0., 0., 1.]],
            radius: 0.,
        };
        let (d, n) = query(
            &Geometry::Sphere { radius: 1. },
            Pose::at([0.; 3]),
            &tri,
            Pose::at([0.; 3]),
        )
        .unwrap();
        assert!(d.abs() < 1e-7);
        assert!(n[1].abs() > 0.999999, "normal {n:?}");
    }
    #[test]
    fn disjoint_cores_are_not_accepted_even_when_radii_overlap() {
        assert!(
            query(
                &box_core(),
                Pose::at([3., 0., 0.]),
                &box_core(),
                Pose::at([0.; 3])
            )
            .is_err()
        );
        let tri = Geometry::Triangle {
            vertices: [[-1., 0., -1.], [1., 0., -1.], [0., 0., 1.]],
            radius: 0.,
        };
        assert!(
            query(
                &Geometry::Sphere { radius: 9. },
                Pose::at([0.9, 0., 0.9]),
                &tri,
                Pose::at([0.; 3])
            )
            .is_err()
        );
    }
    #[test]
    fn exhausted_budget_is_explicit() {
        assert!(
            penetration(
                &box_core(),
                Pose::at([0.; 3]),
                &box_core(),
                Pose::at([0.; 3]),
                QueryLimits::default(),
                &mut 0
            )
            .is_err()
        );
    }
    #[test]
    fn rotated_box_exit_plane_follows_rigid_pose() {
        let b = Geometry::Box {
            half_extents: [2., 1., 0.5],
            radius: 0.,
        };
        let bp = Pose {
            translation: [0.; 3],
            rotation: [[0., 0., 1.], [0., 1., 0.], [-1., 0., 0.]],
        };
        let (d, n) = query(
            &Geometry::Sphere { radius: 1. },
            Pose::at([0.4, 0., 0.]),
            &b,
            bp,
        )
        .unwrap();
        assert!((d - 0.1).abs() < 1e-7, "depth {d}");
        assert!(n[0] > 0.999999);
    }
    #[test]
    fn coincident_point_cores_have_no_certified_direction() {
        let sphere = Geometry::Sphere { radius: 1. };
        assert!(query(&sphere, Pose::at([0.; 3]), &sphere, Pose::at([0.; 3])).is_err());
    }
    #[test]
    fn slanted_tetrahedron_face_is_the_shortest_exit() {
        let hull = Geometry::Hull {
            vertices: vec![[0., 0., 0.], [2., 0., 0.], [0., 2., 0.], [0., 0., 2.]],
            radius: 0.,
        };
        let (d, n) = query(
            &Geometry::Sphere { radius: 1. },
            Pose::at([0.6; 3]),
            &hull,
            Pose::at([0.; 3]),
        )
        .unwrap();
        assert!((d - 0.2 / 3f64.sqrt()).abs() < 1e-7, "depth {d}");
        for c in n {
            assert!((c - 1. / 3f64.sqrt()).abs() < 1e-7, "normal {n:?}");
        }
    }
    #[test]
    fn oriented_box_overlap_uses_an_oblique_separating_axis() {
        let a = Geometry::Box {
            half_extents: [0.1; 3],
            radius: 0.,
        };
        let b = box_core();
        let theta = 20f64.to_radians();
        let (s, c) = theta.sin_cos();
        let bp = Pose {
            translation: [0.; 3],
            rotation: [[c, -s, 0.], [s, c, 0.], [0., 0., 1.]],
        };
        let (d, n) = query(&a, Pose::at([c, s, 0.]), &b, bp).unwrap();
        assert!((d - 0.1 * (c + s)).abs() < 1e-7, "depth {d}");
        assert!(
            (n[0] - c).abs() < 1e-7 && (n[1] - s).abs() < 1e-7 && n[2].abs() < 1e-7,
            "normal {n:?}"
        );
    }
}
