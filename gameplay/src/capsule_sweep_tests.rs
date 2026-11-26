// SYNTHETIC: every triangle, body and motion here is authored in code; the
// capsule dimensions are round test numbers, not any game's body profile.
use super::*;
use crate::collision::{CollisionLimits, CollisionWorld};
use serde_json::json;

const BODY: VerticalCapsule = VerticalCapsule {
    radius: 0.4,
    half_height: 0.5,
};

fn close(a: f64, b: f64, tolerance: f64) -> bool {
    (a - b).abs() <= tolerance
}
fn close3(a: Vec3, b: Vec3, tolerance: f64) -> bool {
    (0..3).all(|i| close(a[i], b[i], tolerance))
}
fn sweep(
    start: Vec3,
    end: Vec3,
    capsule: VerticalCapsule,
    v: [Vec3; 3],
) -> Option<TriangleContact> {
    sweep_capsule_triangle(start, sub(end, start), capsule, v)
}

/// A large floor triangle in the plane y = 0, wound so its normal is +Y.
const FLOOR: [Vec3; 3] = [[-10.0, 0.0, -10.0], [-10.0, 0.0, 30.0], [30.0, 0.0, -10.0]];

#[test]
fn head_on_face_hits_have_the_exact_fraction_and_face_normal() {
    // Falling: the bottom of the body (centre - 0.5 - 0.4) meets y = 0.
    let hit = sweep([1.0, 3.0, 2.0], [1.0, -1.0, 2.0], BODY, FLOOR).unwrap();
    assert!(close(hit.fraction, (3.0 - 0.9) / 4.0, 1e-12), "{hit:?}");
    assert_eq!(hit.feature, TriangleFeature::Face);
    assert!(close3(hit.normal, [0.0, 1.0, 0.0], 1e-12));
    assert!(close3(hit.contact, [1.0, 0.0, 2.0], 1e-12));
    // Walking into a vertical wall at x = 2 (normal toward the body: -X).
    let wall = [[2.0, -5.0, -5.0], [2.0, 5.0, -5.0], [2.0, 0.0, 5.0]];
    let hit = sweep([0.0, 0.0, 0.0], [4.0, 0.0, 0.0], BODY, wall).unwrap();
    assert!(close(hit.fraction, 1.6 / 4.0, 1e-12), "{hit:?}");
    assert_eq!(hit.feature, TriangleFeature::Face);
    assert!(close3(hit.normal, [-1.0, 0.0, 0.0], 1e-12));
    // Double-sided: the same wall from behind.
    let hit = sweep([4.0, 0.0, 0.0], [0.0, 0.0, 0.0], BODY, wall).unwrap();
    assert!(close(hit.fraction, 1.6 / 4.0, 1e-12));
    assert!(close3(hit.normal, [1.0, 0.0, 0.0], 1e-12));
}

#[test]
fn a_grazing_pass_by_an_edge_hits_only_inside_the_radius() {
    // Wall in x = 0 to the -Z side of a vertical edge at z = 0 (vertices 1-2).
    let wall = [[0.0, 0.0, -5.0], [0.0, 0.0, 0.0], [0.0, 4.0, 0.0]];
    let pass = |z: f64| sweep([3.0, 2.0, z], [-3.0, 2.0, z], BODY, wall);
    let hit = pass(0.3).unwrap();
    let x = (0.4f64 * 0.4 - 0.3 * 0.3).sqrt();
    assert!(close(hit.fraction, (3.0 - x) / 6.0, 1e-12), "{hit:?}");
    assert_eq!(hit.feature, TriangleFeature::Edge(1));
    assert!(close3(hit.normal, [x / 0.4, 0.0, 0.3 / 0.4], 1e-12));
    // The axis is parallel to the edge: any point along the overlap is a
    // closest point.
    assert!(hit.contact[0] == 0.0 && hit.contact[2] == 0.0, "{hit:?}");
    assert!((1.5..=2.5).contains(&hit.contact[1]), "{hit:?}");
    // Exactly tangent and just outside both pass.
    assert!(pass(0.4).is_none());
    assert!(pass(0.4001).is_none());
    // Just inside still hits, at the edge.
    assert_eq!(pass(0.3999).unwrap().feature, TriangleFeature::Edge(1));
}

#[test]
fn a_falling_body_can_land_on_a_lone_vertex() {
    let spike = [[0.0, 1.0, 0.0], [1.0, 0.0, 1.0], [-1.0, 0.0, 1.0]];
    let hit = sweep([0.1, 5.0, -0.2], [0.1, 0.0, -0.2], BODY, spike).unwrap();
    let bottom = 1.0 + (0.16f64 - 0.05).sqrt();
    assert!(
        close(hit.fraction, (5.0 - (bottom + 0.5)) / 5.0, 1e-12),
        "{hit:?}"
    );
    assert_eq!(hit.feature, TriangleFeature::Vertex(0));
    assert!(close3(hit.contact, spike[0], 1e-12));
    let expected = [0.1, bottom - 1.0, -0.2].map(|v| v / 0.4);
    assert!(close3(hit.normal, expected, 1e-12));
    // Falling parallel to a vertical wall edge lands on its top corner.
    let wall = [[0.0, 0.0, -5.0], [0.0, 0.0, 0.0], [0.0, 4.0, 0.0]];
    let hit = sweep([0.2, 10.0, 0.2], [0.2, 0.0, 0.2], BODY, wall).unwrap();
    let bottom = 4.0 + (0.16f64 - 0.08).sqrt();
    assert!(
        close(hit.fraction, (10.0 - bottom - 0.5) / 10.0, 1e-12),
        "{hit:?}"
    );
    assert_eq!(hit.feature, TriangleFeature::Vertex(2));
}

#[test]
fn a_slope_returns_its_own_face_normal() {
    let (sin, cos) = (30f64.to_radians().sin(), 30f64.to_radians().cos());
    // Plane y = x tan 30° (normal (-sin, cos, 0)), wound upward.
    let slope = [
        [-5.0, -5.0 * sin / cos, 5.0],
        [5.0, 5.0 * sin / cos, 0.0],
        [-5.0, -5.0 * sin / cos, -5.0],
    ];
    let hit = sweep([1.0, 4.0, 0.0], [1.0, -2.0, 0.0], BODY, slope).unwrap();
    let bottom = (0.4 + sin * 1.0) / cos;
    assert!(
        close(hit.fraction, (4.0 - bottom - 0.5) / 6.0, 1e-12),
        "{hit:?}"
    );
    assert_eq!(hit.feature, TriangleFeature::Face);
    assert!(close3(hit.normal, [-sin, cos, 0.0], 1e-12));
    // The contact is on the slope, not under the axis.
    assert!(close(hit.contact[0], 1.0 + 0.4 * sin, 1e-12));
}

fn world(instances: serde_json::Value) -> CollisionWorld {
    CollisionWorld::from_bytes(
        json!({
            "format":"mag_collision_world","schema_version":1,"coordinate_space":"world",
            "metadata":{"map_name":"synthetic","build":"synthetic","coverage":"provisional",
                "provenance":["synthetic"],"unknowns":["Not a game map"]},
            "instances":instances
        })
        .to_string()
        .as_bytes(),
        CollisionLimits::default(),
    )
    .unwrap()
}

#[test]
fn a_body_on_a_ridge_crest_is_held_by_the_edge_normal() {
    // Two roof planes sharing the crest y = 0, z = 0 along X.
    let roof = world(json!([{"id":"roof",
        "vertices_xyz":[[-2.,0.,0.],[2.,0.,0.],[0.,-2.,2.],[0.,-2.,-2.]],
        "triangles":[[0,2,1],[0,1,3]]}]));
    let mut budget = usize::MAX;
    let hit = roof
        .sweep_capsule([0.3, 3.0, 0.0], [0.3, -1.0, 0.0], BODY, &mut budget)
        .unwrap()
        .unwrap();
    assert!(close(hit.hit.fraction, (3.0 - 0.9) / 4.0, 1e-12), "{hit:?}");
    assert!(matches!(hit.feature, TriangleFeature::Edge(_)), "{hit:?}");
    assert!(close3(hit.hit.normal, [0.0, 1.0, 0.0], 1e-12));
    assert!(close3(hit.contact, [0.3, 0.0, 0.0], 1e-12));
    assert!(close3(hit.hit.position, [0.3, 0.9, 0.0], 1e-12));
    // Off the crest the same fall lands on a roof face instead.
    let hit = roof
        .sweep_capsule([0.3, 3.0, 1.0], [0.3, -3.0, 1.0], BODY, &mut budget)
        .unwrap()
        .unwrap();
    assert_eq!(hit.feature, TriangleFeature::Face);
    let n = std::f64::consts::FRAC_1_SQRT_2;
    assert!(close3(hit.hit.normal, [0.0, n, n], 1e-12));
}

#[test]
fn motions_that_never_come_within_the_radius_miss() {
    // Moving away, stopping short, passing beside and gliding above.
    assert!(sweep([0.0, 2.0, 0.0], [0.0, 4.0, 0.0], BODY, FLOOR).is_none());
    assert!(sweep([0.0, 3.0, 0.0], [0.0, 0.95, 0.0], BODY, FLOOR).is_none());
    assert!(sweep([0.0, 1.0, 0.0], [5.0, 1.0, 0.0], BODY, FLOOR).is_none());
    let small = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]];
    assert!(sweep([-0.5, 3.0, 2.0], [-0.5, -3.0, 2.0], BODY, small).is_none());
    assert!(sweep([2.0, 0.0, -2.0], [-2.0, 0.0, -2.0], BODY, small).is_none());
    // Ending exactly in contact is a hit at fraction one.
    let hit = sweep([0.0, 3.0, 0.0], [0.0, 0.9, 0.0], BODY, FLOOR).unwrap();
    assert!(close(hit.fraction, 1.0, 1e-12));
}

#[test]
fn initial_overlap_is_fraction_zero_without_depenetration() {
    // Bottom 0.1 m into the floor, moving sideways or away: still overlapping.
    for end in [[3.0, 0.8, 0.0], [0.0, 2.0, 0.0], [0.0, -1.0, 0.0]] {
        let hit = sweep([0.0, 0.8, 0.0], end, BODY, FLOOR).unwrap();
        assert_eq!(hit.fraction, 0.0);
        assert!(close3(hit.normal, [0.0, 1.0, 0.0], 1e-12), "{hit:?}");
    }
    // Axis through the floor: the face normal opposes the motion.
    let hit = sweep([0.0, 0.2, 0.0], [0.0, -1.0, 0.0], BODY, FLOOR).unwrap();
    assert_eq!(hit.fraction, 0.0);
    assert!(close3(hit.normal, [0.0, 1.0, 0.0], 1e-12));
    let hit = sweep([0.0, 0.2, 0.0], [0.0, 1.0, 0.0], BODY, FLOOR).unwrap();
    assert!(close3(hit.normal, [0.0, -1.0, 0.0], 1e-12));
    // Stationary: penetration is a hit, resting contact and clearance are not.
    let at = |y: f64| sweep([0.0, y, 0.0], [0.0, y, 0.0], BODY, FLOOR);
    assert_eq!(at(0.85).unwrap().fraction, 0.0);
    assert!(at(0.9).is_none());
    assert!(at(1.5).is_none());
}

#[test]
fn resting_contact_slides_across_shared_edges_but_blocks_approach() {
    let tiles = world(json!([{"id":"floor",
        "vertices_xyz":[[0.,0.,0.],[0.,0.,4.],[4.,0.,4.],[4.,0.,0.]],
        "triangles":[[0,1,2],[0,2,3]]}]));
    let mut budget = usize::MAX;
    // Resting exactly on the floor and walking across the diagonal seam.
    let rest = 0.9;
    for (from, to) in [
        ([1.0, rest, 3.0], [3.0, rest, 1.0]),
        ([0.5, rest, 2.0], [3.5, rest, 2.0]),
    ] {
        assert!(
            tiles
                .sweep_capsule(from, to, BODY, &mut budget)
                .unwrap()
                .is_none()
        );
    }
    // Pressing down from rest stops at once.
    let hit = tiles
        .sweep_capsule([2.0, rest, 1.0], [2.5, rest - 0.1, 1.0], BODY, &mut budget)
        .unwrap()
        .unwrap();
    assert_eq!(hit.hit.fraction, 0.0);
    assert!(close3(hit.hit.normal, [0.0, 1.0, 0.0], 1e-12));
}

#[test]
fn a_rounded_body_rides_over_a_lip_a_box_would_snag_on() {
    // A 5 cm step riser seen from the low side: the box's flat bottom face
    // meets the riser head-on; the capsule meets the lip with its rounded
    // bottom, whose normal points up the step.
    let lip = world(json!([{"id":"step",
        "vertices_xyz":[[1.,0.,-2.],[1.,0.05,-2.],[1.,0.05,2.],[1.,0.,2.],
                        [1.,0.05,-2.],[3.,0.05,-2.],[3.,0.05,2.],[1.,0.05,2.]],
        "triangles":[[0,1,2],[0,2,3],[4,6,5],[4,7,6]]}]));
    let mut budget = usize::MAX;
    let centre = 0.01 + 0.9;
    let hit = lip
        .sweep_capsule([0.0, centre, 0.0], [2.0, centre, 0.0], BODY, &mut budget)
        .unwrap()
        .unwrap();
    assert!(hit.hit.normal[1] > 0.5, "{hit:?}");
    assert!(matches!(hit.feature, TriangleFeature::Edge(_)));
    let half = BODY.half_extents();
    let boxed = lip
        .sweep_aabb(
            [0.0, 0.01 + half[1], 0.0],
            [2.0, 0.01 + half[1], 0.0],
            half,
            &mut budget,
        )
        .unwrap()
        .unwrap();
    assert!(close3(boxed.normal, [-1.0, 0.0, 0.0], 1e-12), "{boxed:?}");
}

fn layered(layer: u32) -> CollisionWorld {
    world(json!([{"id":"wall",
        "vertices_xyz":[[10.,0.,0.],[10.,0.,10.],[10.,4.,10.],[10.,4.,0.]],
        "triangles":[[0,1,2],[0,2,3]],
        "collision":{"filter_info":layer,"ray_child_filter_info":0,"convex_radius":0.,
            "provenance":"synthetic","child_filter_policy":"uniform"}}]))
}

#[test]
fn movement_sweeps_skip_layers_the_character_row_passes() {
    let (start, end) = ([5.0, 2.0, 5.0], [15.0, 2.0, 5.0]);
    let mut budget = usize::MAX;
    let vehicle_only = layered(11).with_character_movement_layers(Some(1 << 9));
    assert!(
        vehicle_only
            .movement_sweep_capsule(start, end, BODY, &mut budget)
            .unwrap()
            .is_none()
    );
    let hit = vehicle_only
        .sweep_capsule(start, end, BODY, &mut budget)
        .unwrap()
        .unwrap();
    assert!(close(hit.hit.fraction, (5.0 - 0.4) / 10.0, 1e-12));
    let static_wall = layered(9).with_character_movement_layers(Some(1 << 9));
    assert!(
        static_wall
            .movement_sweep_capsule(start, end, BODY, &mut budget)
            .unwrap()
            .is_some()
    );
}

/// A flat grid of `n` x `n` unit quads at y = 0.
fn grid(n: usize) -> CollisionWorld {
    let mut vertices = Vec::new();
    let mut triangles = Vec::new();
    for z in 0..=n {
        for x in 0..=n {
            vertices.push(json!([x as f64, 0.0, z as f64]));
        }
    }
    let at = |x: usize, z: usize| x + z * (n + 1);
    for z in 0..n {
        for x in 0..n {
            triangles.push(json!([at(x, z), at(x, z + 1), at(x + 1, z + 1)]));
            triangles.push(json!([at(x, z), at(x + 1, z + 1), at(x + 1, z)]));
        }
    }
    world(json!([{"id":"grid","vertices_xyz":vertices,"triangles":triangles}]))
}

#[test]
fn budget_is_accounted_like_the_box_sweep_and_exhaustion_is_an_error() {
    let floor = grid(16);
    let (start, end) = ([2.0, 1.5, 2.0], [9.0, 1.5, 9.0]);
    let half = BODY.half_extents();
    let mut boxed = 100_000;
    let mut capsule = 100_000;
    assert!(
        floor
            .sweep_aabb(start, end, half, &mut boxed)
            .unwrap()
            .is_none()
    );
    assert!(
        floor
            .sweep_capsule(start, end, BODY, &mut capsule)
            .unwrap()
            .is_none()
    );
    assert_eq!(boxed, capsule);
    let used = 100_000 - capsule;
    let mut short = used - 1;
    assert!(floor.sweep_capsule(start, end, BODY, &mut short).is_err());
    let mut exact = used;
    assert!(floor.sweep_capsule(start, end, BODY, &mut exact).is_ok());
    assert_eq!(exact, 0);
}

#[test]
fn invalid_bodies_and_points_are_errors() {
    let floor = grid(2);
    let mut budget = usize::MAX;
    for capsule in [
        VerticalCapsule {
            radius: 0.0,
            half_height: 0.5,
        },
        VerticalCapsule {
            radius: f64::NAN,
            half_height: 0.5,
        },
        VerticalCapsule {
            radius: 0.4,
            half_height: -0.1,
        },
        VerticalCapsule {
            radius: 0.4,
            half_height: 1e3,
        },
    ] {
        assert!(
            floor
                .sweep_capsule([1.0, 2.0, 1.0], [1.0, 0.0, 1.0], capsule, &mut budget)
                .is_err()
        );
    }
    assert!(
        floor
            .sweep_capsule([f64::NAN, 2.0, 1.0], [1.0, 0.0, 1.0], BODY, &mut budget)
            .is_err()
    );
    // A sphere (zero half-height) is accepted.
    let ball = VerticalCapsule {
        radius: 0.3,
        half_height: 0.0,
    };
    let hit = floor
        .sweep_capsule([1.0, 2.0, 1.0], [1.0, 0.0, 1.0], ball, &mut budget)
        .unwrap()
        .unwrap();
    assert!(close(hit.hit.fraction, 1.7 / 2.0, 1e-12));
}

#[test]
fn upright_capsules_collide_as_one_summed_capsule() {
    let other = VerticalCapsule {
        radius: 0.3,
        half_height: 0.6,
    };
    // Side by side, head-on along X: centres 0.7 apart at contact.
    let (t, n) =
        sweep_vertical_capsules([-3.0, 0.0, 0.0], [3.0, 0.0, 0.0], BODY, [0.0; 3], other).unwrap();
    assert!(close(t, (3.0 - 0.7) / 6.0, 1e-12));
    assert!(close3(n, [-1.0, 0.0, 0.0], 1e-12));
    // Landing on top: centres 0.5 + 0.6 + 0.4 + 0.3 apart vertically.
    let (t, n) =
        sweep_vertical_capsules([0.1, 5.0, 0.0], [0.1, 0.0, 0.0], BODY, [0.0; 3], other).unwrap();
    let bottom = 1.1 + (0.49f64 - 0.01).sqrt();
    assert!(close(t, (5.0 - bottom) / 5.0, 1e-12));
    assert!(close3(n, [0.1 / 0.7, (bottom - 1.1) / 0.7, 0.0], 1e-12));
    // Passing beside, tangent, and overlapping.
    let pass =
        |z: f64| sweep_vertical_capsules([-3.0, 0.0, z], [3.0, 0.0, z], BODY, [0.0; 3], other);
    assert!(pass(0.71).is_none());
    assert!(pass(0.7).is_none());
    assert!(pass(0.69).is_some());
    let (t, n) =
        sweep_vertical_capsules([0.5, 0.0, 0.0], [3.0, 0.0, 0.0], BODY, [0.0; 3], other).unwrap();
    assert_eq!(t, 0.0);
    assert!(close3(n, [1.0, 0.0, 0.0], 1e-12));
}

// ---- Randomised cross-check against an independent brute-force reference.

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> f64 {
        // SplitMix64.
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64
    }
    fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.next()
    }
    fn point(&mut self, extent: f64) -> Vec3 {
        std::array::from_fn(|_| self.range(-extent, extent))
    }
}

/// Distance from a point to a triangle by plane projection when it lands
/// inside, else the nearest edge; written independently of the module.
fn reference_point_distance(p: Vec3, v: [Vec3; 3]) -> f64 {
    let n = cross(sub(v[1], v[0]), sub(v[2], v[0]));
    let length = norm(n);
    let edge_distance = |a: Vec3, b: Vec3| {
        let d = sub(b, a);
        let s = (dot(sub(p, a), d) / dot(d, d)).clamp(0.0, 1.0);
        norm(sub(p, add(a, scale(d, s))))
    };
    let edges = edge_distance(v[0], v[1])
        .min(edge_distance(v[1], v[2]))
        .min(edge_distance(v[2], v[0]));
    if length < 1e-12 {
        return edges;
    }
    let n = scale(n, 1.0 / length);
    let height = dot(sub(p, v[0]), n);
    let foot = sub(p, scale(n, height));
    let inside = (0..3).all(|i| dot(cross(sub(v[(i + 1) % 3], v[i]), sub(foot, v[i])), n) >= 0.0);
    if inside { height.abs() } else { edges }
}

/// Minimum of a convex function on [0, 1] by golden-section search.
fn convex_minimum(f: impl Fn(f64) -> f64) -> (f64, f64) {
    let ratio = (5f64.sqrt() - 1.0) / 2.0;
    let (mut lo, mut hi) = (0.0, 1.0);
    for _ in 0..64 {
        let a = hi - ratio * (hi - lo);
        let b = lo + ratio * (hi - lo);
        if f(a) <= f(b) {
            hi = b;
        } else {
            lo = a;
        }
    }
    let mut best = (0.5 * (lo + hi), f(0.5 * (lo + hi)));
    for t in [0.0, 1.0] {
        if f(t) < best.1 {
            best = (t, f(t));
        }
    }
    best
}

/// Static capsule-axis to triangle distance: the point distance is convex
/// along the axis.
fn reference_distance(centre: Vec3, capsule: VerticalCapsule, v: [Vec3; 3]) -> f64 {
    let h = capsule.half_height;
    convex_minimum(|s| {
        let p = [centre[0], centre[1] - h + 2.0 * h * s, centre[2]];
        reference_point_distance(p, v)
    })
    .1
}

#[derive(Debug)]
enum Expected {
    Overlap,
    Miss,
    Hit(f64),
}

/// Distance along the motion is convex too: classify by its minimum, then
/// bisect the descending part for the first time it reaches the radius.
/// Cases within `margin` of tangency are ambiguous and skipped.
fn reference_sweep(
    start: Vec3,
    delta: Vec3,
    capsule: VerticalCapsule,
    v: [Vec3; 3],
    margin: f64,
) -> Option<Expected> {
    let r = capsule.radius;
    let distance = |t: f64| reference_distance(add(start, scale(delta, t)), capsule, v);
    let initial = distance(0.0);
    if (initial - r).abs() < margin {
        return None;
    }
    if initial < r {
        return Some(Expected::Overlap);
    }
    let (t_min, d_min) = convex_minimum(distance);
    if (d_min - r).abs() < margin {
        return None;
    }
    if d_min > r {
        return Some(Expected::Miss);
    }
    let (mut lo, mut hi) = (0.0, t_min);
    for _ in 0..56 {
        let mid = 0.5 * (lo + hi);
        if distance(mid) > r {
            lo = mid
        } else {
            hi = mid
        }
    }
    Some(Expected::Hit(hi))
}

fn random_case(rng: &mut Rng) -> (Vec3, Vec3, VerticalCapsule, [Vec3; 3]) {
    let capsule = VerticalCapsule {
        radius: rng.range(0.1, 0.6),
        half_height: if rng.next() < 0.15 {
            0.0
        } else {
            rng.range(0.05, 0.9)
        },
    };
    let mut v = [rng.point(2.0), rng.point(2.0), rng.point(2.0)];
    match (rng.next() * 4.0) as u32 {
        // Level floor and vertical wall pieces exercise the degenerate prisms.
        0 => {
            let y = v[0][1];
            v.iter_mut().for_each(|p| p[1] = y);
        }
        1 => {
            let x = v[0][0];
            v.iter_mut().for_each(|p| p[0] = x);
        }
        _ => {}
    }
    let start = rng.point(4.0);
    // Aim near a random triangle point so hits, misses and grazes all occur.
    let (a, b) = (rng.next(), rng.next());
    let (a, b) = if a + b > 1.0 {
        (1.0 - a, 1.0 - b)
    } else {
        (a, b)
    };
    let target = add(
        add(
            v[0],
            add(scale(sub(v[1], v[0]), a), scale(sub(v[2], v[0]), b)),
        ),
        rng.point(1.0),
    );
    // Some starts already touch or overlap the triangle.
    let start = if rng.next() < 0.2 {
        add(target, rng.point(0.3))
    } else {
        start
    };
    let mut delta = scale(sub(target, start), rng.range(0.3, 2.0));
    if rng.next() < 0.25 {
        delta[1] = 0.0;
    }
    (start, delta, capsule, v)
}

#[test]
fn random_sweeps_agree_with_a_brute_force_reference() {
    let mut rng = Rng(0x5eed_ca95_0001);
    let (mut hits, mut misses, mut overlaps, mut skipped) = (0, 0, 0, 0);
    for case in 0..600 {
        let (start, delta, capsule, v) = random_case(&mut rng);
        let Some(expected) = reference_sweep(start, delta, capsule, v, 1e-4) else {
            skipped += 1;
            continue;
        };
        let got = sweep_capsule_triangle(start, delta, capsule, v);
        let context =
            format!("case {case}: {start:?} {delta:?} {capsule:?} {v:?} {expected:?} {got:?}");
        match expected {
            Expected::Miss => {
                assert!(got.is_none(), "{context}");
                misses += 1;
            }
            Expected::Overlap => {
                assert_eq!(got.expect(&context).fraction, 0.0, "{context}");
                overlaps += 1;
            }
            Expected::Hit(t) => {
                let got = got.expect(&context);
                assert!(close(got.fraction, t, 1e-6), "{context}");
                // The contact lies on the triangle, one radius from the axis,
                // and the normal points from it to its nearest axis point.
                let centre = add(start, scale(delta, got.fraction));
                let (low, high) = capsule.axis(centre);
                let (axis, _) = closest_on_segment(got.contact, low, high);
                let gap = sub(axis, got.contact);
                assert!(reference_point_distance(got.contact, v) < 1e-9, "{context}");
                assert!(close(norm(gap), capsule.radius, 1e-6), "{context}");
                assert!(
                    close3(got.normal, scale(gap, 1.0 / norm(gap)), 1e-5),
                    "{context}"
                );
                assert!(close(norm(got.normal), 1.0, 1e-12), "{context}");
                hits += 1;
            }
        }
    }
    assert!(
        hits >= 100 && misses >= 100 && overlaps >= 40,
        "{hits} {misses} {overlaps} {skipped}"
    );
    assert!(skipped < 40, "{skipped}");
}

#[test]
fn random_capsule_pairs_agree_with_the_summed_segment_distance() {
    let mut rng = Rng(0x5eed_ca95_0002);
    let mut hits = 0;
    for case in 0..400 {
        let mover = VerticalCapsule {
            radius: rng.range(0.1, 0.6),
            half_height: rng.range(0.0, 0.9),
        };
        let other = VerticalCapsule {
            radius: rng.range(0.1, 0.6),
            half_height: rng.range(0.0, 0.9),
        };
        let (from, centre) = (rng.point(3.0), rng.point(1.0));
        let to = add(
            from,
            scale(sub(add(centre, rng.point(1.0)), from), rng.range(0.3, 2.0)),
        );
        let combined = mover.radius + other.radius;
        let span = mover.half_height + other.half_height;
        let distance = |t: f64| {
            let p = add(from, scale(sub(to, from), t));
            let y = (p[1] - centre[1]).clamp(-span, span);
            norm(sub(p, [centre[0], centre[1] + y, centre[2]]))
        };
        let got = sweep_vertical_capsules(from, to, mover, centre, other);
        let context = format!("case {case}: {got:?}");
        let (t_min, d_min) = convex_minimum(distance);
        if (distance(0.0) - combined).abs() < 1e-4 || (d_min - combined).abs() < 1e-4 {
            continue;
        }
        if distance(0.0) < combined {
            assert_eq!(got.expect(&context).0, 0.0);
        } else if d_min > combined {
            assert!(got.is_none(), "{context}");
        } else {
            let (mut lo, mut hi) = (0.0, t_min);
            for _ in 0..80 {
                let mid = 0.5 * (lo + hi);
                if distance(mid) > combined {
                    lo = mid
                } else {
                    hi = mid
                }
            }
            assert!(close(got.expect(&context).0, hi, 1e-6), "{context} {hi}");
            hits += 1;
        }
    }
    assert!(hits >= 50, "{hits}");
}

// ---- Cost against the box sweep on a dense synthetic mesh.

/// Run with `cargo test --release capsule_sweep_cost -- --ignored --nocapture`.
#[test]
#[ignore]
fn capsule_sweep_cost_against_the_box_sweep() {
    // 224 x 224 quads (100 352 triangles) of rolling terrain over 112 m.
    let n = 224;
    let spacing = 0.5;
    let height =
        |x: f64, z: f64| 0.4 * (x * 0.7).sin() * (z * 0.5).cos() + 0.1 * (x * 3.1 + z * 2.3).sin();
    let mut vertices = Vec::new();
    let mut triangles = Vec::new();
    for zi in 0..=n {
        for xi in 0..=n {
            let (x, z) = (xi as f64 * spacing, zi as f64 * spacing);
            vertices.push(json!([x, height(x, z), z]));
        }
    }
    let at = |x: usize, z: usize| x + z * (n + 1);
    for z in 0..n {
        for x in 0..n {
            triangles.push(json!([at(x, z), at(x, z + 1), at(x + 1, z + 1)]));
            triangles.push(json!([at(x, z), at(x + 1, z + 1), at(x + 1, z)]));
        }
    }
    let terrain = world(json!([{"id":"terrain","vertices_xyz":vertices,"triangles":triangles}]));
    let mut rng = Rng(0x5eed_ca95_0003);
    // Bot-like ticks: a 0.3 m walk step and a small fall, just above ground.
    let motions: Vec<(Vec3, Vec3)> = (0..20_000)
        .map(|_| {
            let (x, z) = (rng.range(5.0, 107.0), rng.range(5.0, 107.0));
            let heading = rng.range(0.0, std::f64::consts::TAU);
            let feet = height(x, z) + rng.range(0.0, 0.3);
            let start = [x, feet + BODY.half_extents()[1], z];
            let end = [
                x + 0.3 * heading.cos(),
                start[1] - 0.05,
                z + 0.3 * heading.sin(),
            ];
            (start, end)
        })
        .collect();
    let half = BODY.half_extents();
    let run = |capsule: bool| {
        let mut work = 0usize;
        let mut hits = 0usize;
        let began = std::time::Instant::now();
        for &(start, end) in &motions {
            let mut budget = usize::MAX;
            let hit = if capsule {
                terrain
                    .movement_sweep_capsule(start, end, BODY, &mut budget)
                    .unwrap()
                    .map(|h| h.hit)
            } else {
                terrain
                    .movement_sweep_aabb(start, end, half, &mut budget)
                    .unwrap()
            };
            hits += usize::from(hit.is_some());
            work += usize::MAX - budget;
        }
        let elapsed = began.elapsed().as_secs_f64();
        (
            elapsed * 1e6 / motions.len() as f64,
            work as f64 / motions.len() as f64,
            hits,
        )
    };
    run(false);
    run(true);
    let (box_us, box_work, box_hits) = run(false);
    let (capsule_us, capsule_work, capsule_hits) = run(true);
    println!(
        "{} triangles, {} sweeps: box {box_us:.2} us/sweep ({box_work:.0} work, {box_hits} hits), \
         capsule {capsule_us:.2} us/sweep ({capsule_work:.0} work, {capsule_hits} hits)",
        terrain.stats().triangles,
        motions.len()
    );
}
