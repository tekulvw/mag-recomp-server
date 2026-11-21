//! Earliest swept sphere contact over an explicitly scoped set of solid obstacles.
use crate::{
    acquisition_volume_damage::{Target, TargetKey, TargetKind},
    dynamic_collision::{Geometry, Pose, QueryLimits, geometry},
};
use std::{cmp::Ordering, collections::BTreeSet};

type Point = [f64; 3];
const WORK_LIMIT: &str = "projectile_contact_work_limit";

/// Coverage of the caller's prefiltered candidates for the entire segment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Coverage {
    CompleteCurrent,
    /// Explicit replacement policy allowing qualified reference-pose obstacles.
    ReferencePose,
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum TriangleId<'a> {
    Static {
        instance: &'a str,
        triangle: usize,
    },
    Sequence {
        placement: &'a str,
        instance: &'a str,
        triangle: usize,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObstacleId<'a> {
    Triangle(TriangleId<'a>),
    Dynamic { key: TargetKey, root: &'a str },
}

impl Ord for ObstacleId<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Triangle(left), Self::Triangle(right)) => left.cmp(right),
            (Self::Triangle(_), Self::Dynamic { .. }) => Ordering::Less,
            (Self::Dynamic { .. }, Self::Triangle(_)) => Ordering::Greater,
            (
                Self::Dynamic {
                    key: left,
                    root: left_root,
                },
                Self::Dynamic {
                    key: right,
                    root: right_root,
                },
            ) => {
                (left.id, left.generation, left_root).cmp(&(right.id, right.generation, right_root))
            }
        }
    }
}

impl PartialOrd for ObstacleId<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

pub struct SphereSweep {
    pub start: Point,
    pub end: Point,
    pub radius: f64,
    /// The current physical Character key corresponding to the launch association.
    pub associated_character: Option<TargetKey>,
}

pub struct TriangleObstacle<'a> {
    pub id: TriangleId<'a>,
    pub vertices: [Point; 3],
    pub pose: Pose,
    pub margin: f64,
}

pub struct ConvexObstacle<'a> {
    pub target: Target,
    pub root: &'a str,
    pub shape: &'a Geometry,
    pub pose: Pose,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Contact<'a> {
    pub obstacle: ObstacleId<'a>,
    pub fraction: f64,
    /// Position of the sphere centre on the swept segment.
    pub center: Point,
    /// Obstacle-to-sphere normal, absent when the convex cores already intersect.
    pub normal: Option<Point>,
}

/// Compare all eligible candidates with one shared work budget. The caller
/// supplies the frozen poses, applies collision filters and qualifies coverage.
/// Starting overlap is a contact even when the sphere is moving away. An exact
/// fraction tie picks the lowest obstacle ID in static, sequence, dynamic order.
pub fn sweep_sphere<'a>(
    query: SphereSweep,
    coverage: Coverage,
    triangles: &[TriangleObstacle<'a>],
    convex: &[ConvexObstacle<'a>],
    limits: QueryLimits,
    budget: &mut usize,
) -> Result<Option<Contact<'a>>, &'static str> {
    if coverage == Coverage::Unavailable {
        return Err("projectile_contact_coverage_unavailable");
    }
    if limits.max_support_points == 0
        || limits.max_support_points > 100_000_000
        || *budget > limits.max_support_points
        || !(1..=256).contains(&limits.max_iterations)
        || !limits.boundary_epsilon.is_finite()
        || limits.boundary_epsilon <= 0.
        || limits.boundary_epsilon > 0.01
    {
        return Err("projectile_contact_invalid_limits");
    }
    let start = Pose::at(query.start);
    let sphere = Geometry::Sphere {
        radius: query.radius,
    };
    if query.start == query.end
        || start.validate().is_err()
        || Pose::at(query.end).validate().is_err()
        || sphere.validate().is_err()
        || query.associated_character.is_some_and(invalid_target_key)
    {
        return Err("projectile_contact_invalid_segment");
    }
    let mut collector = Collector {
        start,
        end: query.end,
        sphere,
        limits,
        budget,
        seen: BTreeSet::new(),
        nearest: None,
    };
    for obstacle in triangles {
        let shape = Geometry::Triangle {
            vertices: obstacle.vertices,
            radius: obstacle.margin,
        };
        validate_triangle(obstacle)?;
        collector.consider(
            ObstacleId::Triangle(obstacle.id),
            &shape,
            obstacle.pose,
            false,
        )?;
    }
    for obstacle in convex {
        if obstacle.root.trim().is_empty()
            || invalid_target_key(obstacle.target.key)
            || matches!(obstacle.shape, Geometry::Triangle { .. })
        {
            return Err("projectile_contact_invalid_root");
        }
        let ignore = obstacle.target.kind == TargetKind::Character
            && query.associated_character == Some(obstacle.target.key);
        collector.consider(
            ObstacleId::Dynamic {
                key: obstacle.target.key,
                root: obstacle.root,
            },
            obstacle.shape,
            obstacle.pose,
            ignore,
        )?;
    }
    Ok(collector.nearest)
}

fn invalid_target_key(key: TargetKey) -> bool {
    key.id == 0 || key.generation == 0
}

fn validate_triangle(obstacle: &TriangleObstacle<'_>) -> Result<(), &'static str> {
    let names_valid = match obstacle.id {
        TriangleId::Static { instance, .. } => !instance.trim().is_empty(),
        TriangleId::Sequence {
            placement,
            instance,
            ..
        } => !placement.trim().is_empty() && !instance.trim().is_empty(),
    };
    let [a, b, c] = obstacle.vertices;
    let u = geometry::sub(b, a);
    let v = geometry::sub(c, a);
    let cross = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ];
    let area_squared = geometry::dot(cross, cross);
    if !names_valid || !area_squared.is_finite() || area_squared == 0. {
        return Err("projectile_contact_invalid_triangle");
    }
    Ok(())
}

struct Collector<'a, 'b> {
    start: Pose,
    end: Point,
    sphere: Geometry,
    limits: QueryLimits,
    budget: &'b mut usize,
    seen: BTreeSet<ObstacleId<'a>>,
    nearest: Option<Contact<'a>>,
}

impl<'a> Collector<'a, '_> {
    fn consider(
        &mut self,
        obstacle: ObstacleId<'a>,
        shape: &Geometry,
        pose: Pose,
        ignore: bool,
    ) -> Result<(), &'static str> {
        // Shape validation visits each stored hull vertex before the sweep.
        let shape_points = match shape {
            Geometry::Sphere { .. } | Geometry::Box { .. } => 1,
            Geometry::Capsule { .. } => 2,
            Geometry::Triangle { .. } => 3,
            Geometry::Hull { vertices, .. } => vertices.len(),
        };
        *self.budget = self
            .budget
            .checked_sub(1 + shape_points)
            .ok_or(WORK_LIMIT)?;
        if !self.seen.insert(obstacle) {
            return Err("projectile_contact_duplicate_obstacle");
        }
        shape.validate()?;
        pose.validate()?;
        if ignore {
            return Ok(());
        }
        let Some((fraction, normal)) = self.contact(shape, pose).map_err(query_error)? else {
            return Ok(());
        };
        if self.nearest.is_none_or(|nearest| {
            fraction
                .total_cmp(&nearest.fraction)
                .then_with(|| obstacle.cmp(&nearest.obstacle))
                .is_lt()
        }) {
            let start = self.start.translation;
            let center =
                std::array::from_fn(|axis| start[axis] + fraction * (self.end[axis] - start[axis]));
            self.nearest = Some(Contact {
                obstacle,
                fraction,
                center,
                normal,
            });
        }
        Ok(())
    }

    fn contact(
        &mut self,
        shape: &Geometry,
        pose: Pose,
    ) -> Result<Option<(f64, Option<Point>)>, &'static str> {
        let tolerance = self.limits.boundary_epsilon;
        let initial = geometry::distance(
            &self.sphere,
            self.start,
            shape,
            pose,
            self.limits,
            self.budget,
        )?;
        if initial.upper <= self.sphere.radius() + shape.radius() + tolerance {
            return Ok(Some((0., initial.normal)));
        }

        // The shared character sweep has a separating-overlap escape. The
        // projectile already handled starting overlaps, so that branch is moot.
        geometry::sweep(
            &self.sphere,
            self.start,
            self.end,
            shape,
            pose,
            geometry::SweepLimits {
                tolerance,
                extra: 0.,
                query: self.limits,
            },
            self.budget,
        )
        .map(|contact| contact.map(|(fraction, normal)| (fraction, Some(normal))))
    }
}

fn query_error(code: &'static str) -> &'static str {
    match code {
        "dynamic_collision_work_limit" => WORK_LIMIT,
        _ => code,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FACE: [Point; 3] = [[4., -2., -2.], [4., 2., -2.], [4., 0., 2.]];
    const SPHERE: Geometry = Geometry::Sphere { radius: 0.5 };
    const CHARACTER: TargetKey = TargetKey {
        id: 0x0380_0001,
        generation: 2,
    };

    fn query() -> SphereSweep {
        SphereSweep {
            start: [0.; 3],
            end: [10., 0., 0.],
            radius: 0.1,
            associated_character: None,
        }
    }

    fn face(instance: &str) -> TriangleObstacle<'_> {
        TriangleObstacle {
            id: TriangleId::Static {
                instance,
                triangle: 0,
            },
            vertices: FACE,
            pose: Pose::at([0.; 3]),
            margin: 0.,
        }
    }

    fn body(root: &str, position: Point, kind: TargetKind) -> ConvexObstacle<'_> {
        ConvexObstacle {
            target: Target {
                key: CHARACTER,
                kind,
                team: 1,
            },
            root,
            shape: &SPHERE,
            pose: Pose::at(position),
        }
    }

    fn cast<'a>(
        query: SphereSweep,
        coverage: Coverage,
        triangles: &[TriangleObstacle<'a>],
        convex: &[ConvexObstacle<'a>],
    ) -> Result<Option<Contact<'a>>, &'static str> {
        let limits = QueryLimits::default();
        let mut budget = limits.max_support_points;
        sweep_sphere(query, coverage, triangles, convex, limits, &mut budget)
    }

    fn close(actual: f64, expected: f64) {
        assert!((actual - expected).abs() < 1e-6, "{actual} != {expected}");
    }

    #[test]
    fn projectile_contact_selects_earliest_across_static_and_current_bodies() {
        let wall = face("wall");
        let hit = cast(
            query(),
            Coverage::CompleteCurrent,
            &[wall],
            &[body("chassis", [2., 0., 0.], TargetKind::GroundVehicle)],
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            hit.obstacle,
            ObstacleId::Dynamic {
                key: CHARACTER,
                root: "chassis",
            }
        );
        close(hit.fraction, 0.14);
        close(hit.center[0], 1.4);
        assert_eq!(hit.normal, Some([-1., 0., 0.]));

        let wall = face("wall");
        let hit = cast(
            query(),
            Coverage::CompleteCurrent,
            &[wall],
            &[body("chassis", [8., 0., 0.], TargetKind::GroundVehicle)],
        )
        .unwrap()
        .unwrap();
        assert_eq!(hit.obstacle, ObstacleId::Triangle(face("wall").id));
        close(hit.fraction, 0.39);
        close(hit.center[0], 3.9);
    }

    #[test]
    fn projectile_contact_checks_the_sphere_against_triangle_edges() {
        let triangle = [[4., 0., 0.], [4., 1., 0.], [4., 0., 1.]];
        let obstacle = TriangleObstacle {
            vertices: triangle,
            ..face("edge")
        };
        let offset = |axis| SphereSweep {
            start: [0., axis, axis],
            end: [10., axis, axis],
            ..query()
        };
        assert!(
            cast(offset(0.58), Coverage::CompleteCurrent, &[obstacle], &[])
                .unwrap()
                .is_none()
        );
        let obstacle = TriangleObstacle {
            vertices: triangle,
            ..face("edge")
        };
        let hit = cast(offset(0.56), Coverage::CompleteCurrent, &[obstacle], &[])
            .unwrap()
            .unwrap();
        assert!((0.39..0.4).contains(&hit.fraction));
    }

    #[test]
    fn projectile_contact_keeps_start_overlap_and_endpoint_contact() {
        let case = |start, end| SphereSweep {
            start,
            end,
            ..query()
        };
        let roots = [body("chassis", [2., 0., 0.], TargetKind::GroundVehicle)];
        let overlap = cast(
            case([2.3, 0., 0.], [3., 0., 0.]),
            Coverage::CompleteCurrent,
            &[],
            &roots,
        )
        .unwrap()
        .unwrap();
        assert_eq!(overlap.fraction, 0.);
        assert_eq!(overlap.normal, Some([1., 0., 0.]));
        let coincident = cast(
            case([2., 0., 0.], [3., 0., 0.]),
            Coverage::CompleteCurrent,
            &[],
            &roots,
        )
        .unwrap()
        .unwrap();
        assert_eq!(coincident.fraction, 0.);
        assert!(coincident.normal.is_none());
        let endpoint = cast(
            case([0.; 3], [3.9, 0., 0.]),
            Coverage::CompleteCurrent,
            &[face("endpoint")],
            &[],
        )
        .unwrap()
        .unwrap();
        close(endpoint.fraction, 1.);
        close(endpoint.center[0], 3.9);
    }

    #[test]
    fn projectile_contact_ignores_only_the_exact_associated_character_root() {
        let associated = || SphereSweep {
            associated_character: Some(CHARACTER),
            ..query()
        };
        let contact = cast(
            associated(),
            Coverage::CompleteCurrent,
            &[],
            &[
                body("capsule", [1., 0., 0.], TargetKind::Character),
                body("chassis", [2., 0., 0.], TargetKind::GroundVehicle),
            ],
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            contact.obstacle,
            ObstacleId::Dynamic {
                key: CHARACTER,
                root: "chassis",
            }
        );
        let mut later = body("capsule", [1., 0., 0.], TargetKind::Character);
        later.target.key.generation += 1;
        let contact = cast(associated(), Coverage::CompleteCurrent, &[], &[later])
            .unwrap()
            .unwrap();
        assert_eq!(
            contact.obstacle,
            ObstacleId::Dynamic {
                key: TargetKey {
                    generation: 3,
                    ..CHARACTER
                },
                root: "capsule",
            }
        );
    }

    #[test]
    fn projectile_contact_resolves_ties_by_stable_identity() {
        let sequence = || TriangleObstacle {
            id: TriangleId::Sequence {
                placement: "objective-4",
                instance: "mesh",
                triangle: 0,
            },
            ..face("unused")
        };
        for triangles in [
            [face("z-wall"), sequence(), face("a-wall")],
            [face("a-wall"), face("z-wall"), sequence()],
        ] {
            let contact = cast(query(), Coverage::ReferencePose, &triangles, &[])
                .unwrap()
                .unwrap();
            assert_eq!(contact.obstacle, ObstacleId::Triangle(face("a-wall").id));
        }
    }

    #[test]
    fn projectile_contact_reports_unknown_coverage_and_work_exhaustion() {
        assert_eq!(
            cast(query(), Coverage::Unavailable, &[], &[]),
            Err("projectile_contact_coverage_unavailable")
        );
        assert!(
            cast(query(), Coverage::ReferencePose, &[], &[])
                .unwrap()
                .is_none()
        );
        let limits = QueryLimits::default();
        let walls = [face("first"), face("second")];
        let mut budget = 60;
        sweep_sphere(
            query(),
            Coverage::CompleteCurrent,
            &walls[..1],
            &[],
            limits,
            &mut budget,
        )
        .unwrap()
        .unwrap();
        assert!(budget < 60);
        let mut budget = 60 - budget;
        assert_eq!(
            sweep_sphere(
                query(),
                Coverage::CompleteCurrent,
                &walls,
                &[],
                limits,
                &mut budget,
            ),
            Err("projectile_contact_work_limit")
        );
    }

    #[test]
    fn projectile_contact_rejects_corrupt_geometry_and_duplicate_ids() {
        assert_eq!(
            cast(
                SphereSweep {
                    start: [f64::NAN, 0., 0.],
                    ..query()
                },
                Coverage::CompleteCurrent,
                &[],
                &[],
            ),
            Err("projectile_contact_invalid_segment")
        );
        assert_eq!(
            cast(
                query(),
                Coverage::CompleteCurrent,
                &[TriangleObstacle {
                    vertices: [[0.; 3]; 3],
                    ..face("degenerate")
                }],
                &[],
            ),
            Err("projectile_contact_invalid_triangle")
        );
        assert_eq!(
            cast(
                query(),
                Coverage::CompleteCurrent,
                &[face("duplicate"), face("duplicate")],
                &[],
            ),
            Err("projectile_contact_duplicate_obstacle")
        );
    }
}
