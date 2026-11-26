//! Continuous collision of an upright (Y axis) capsule against triangles and
//! against other upright capsules.
//!
//! # Geometry
//!
//! A capsule is every point within `radius` of its axis segment, the centre
//! `c` plus or minus `half_height` along +Y. It touches a triangle `T` exactly
//! when the centre lies in `M = T ⊕ S ⊕ B(radius)`, where `S` is the axis
//! segment about the origin (symmetric, so its reflection is itself) and `B` a
//! ball. A translating capsule therefore hits `T` at the first time the ray
//! `c(t) = start + t·delta`, `t ∈ [0, 1]`, enters the convex set `M`.
//!
//! `P = T ⊕ S` is the triangle extruded up and down by `half_height`: two caps
//! (`T ± half_height·Y`), three side parallelograms (each edge swept along Y),
//! nine segments (cap edges and the three vertex sweeps) and six corners.
//! Every face, edge and corner of `P`'s convex hull is among those candidates,
//! including degenerate cases (a vertical triangle makes `P` flat; a zero
//! half-height makes it the triangle itself). `M` is the union of:
//!
//! * each candidate face thickened by `radius` along its normal, entered
//!   first through one of its two offset planes (a planar test, accepted when
//!   the entry point projects inside the face);
//! * each candidate segment's solid cylinder of `radius` (accepted when the
//!   entry point projects inside the segment);
//! * a ball of `radius` at each candidate corner;
//! * `P` itself, which a ray starting outside can reach only by first crossing
//!   one of the above.
//!
//! A face slab's side walls lie inside the cylinders of the face's edges and a
//! cylinder's end discs inside its corner balls, so the earliest entry into `M`
//! is the minimum over those planar, cylinder and ball entries. Every candidate
//! is a subset of `M` (each point is within `radius` of `P`), so none can report
//! an early, false hit. The result is exact, not a conservative advancement:
//! closed-form roots, no iteration and no step tolerance. A start already
//! within `radius` of the triangle (by a static segment-to-triangle distance)
//! reports fraction zero; nothing is depenetrated.
//!
//! # Contact rule
//!
//! A contact only blocks motion that approaches it. Pure tangency, where the
//! motion's deepest approach stays within [`CONTACT_TOLERANCE`] of the
//! surface, is not a hit, so a capsule resting on a tessellated floor may slide
//! across shared edges. A start penetrating deeper than the tolerance is an
//! overlap (fraction zero) whatever the motion; a stationary query reports only
//! such penetration, as the box sweep does.
//!
//! # Contact normal
//!
//! Computed from the closest points of the capsule axis and the triangle at
//! the time of impact: the unit face normal, turned toward the axis, when the
//! closest triangle point is inside the face; otherwise the unit direction
//! from that triangle point (on an edge or corner) to the axis. A grounding
//! test can then treat a crest or a ledge lip by the direction it actually
//! supports the body from. When the axis crosses the triangle (deep initial
//! overlap) the face normal opposes the motion.
//!
//! Written from the geometry above; it shares no structure with any
//! particular physics engine.

type Vec3 = [f64; 3];

/// Distance band within which a contact counts as tangency, in metres.
pub const CONTACT_TOLERANCE: f64 = 1e-9;
/// Largest accepted radius or half-height, in metres.
pub const MAX_CAPSULE_EXTENT: f64 = 100.0;

/// An upright capsule: its axis runs `half_height` above and below the centre
/// along +Y; the body is every point within `radius` of that axis. A zero
/// half-height is a sphere.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VerticalCapsule {
    pub radius: f64,
    pub half_height: f64,
}
impl VerticalCapsule {
    pub fn valid(self) -> bool {
        self.radius.is_finite()
            && self.radius > 0.0
            && self.radius <= MAX_CAPSULE_EXTENT
            && self.half_height.is_finite()
            && self.half_height >= 0.0
            && self.half_height <= MAX_CAPSULE_EXTENT
    }
    /// Half extents of the capsule's axis-aligned bounding box.
    pub fn half_extents(self) -> Vec3 {
        [self.radius, self.half_height + self.radius, self.radius]
    }
    fn axis(self, centre: Vec3) -> (Vec3, Vec3) {
        let h = self.half_height;
        (
            [centre[0], centre[1] - h, centre[2]],
            [centre[0], centre[1] + h, centre[2]],
        )
    }
}

/// Which part of the triangle the capsule touches. Edge `i` joins vertices
/// `i` and `(i + 1) % 3`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TriangleFeature {
    Face,
    Edge(u8),
    Vertex(u8),
}

#[derive(Clone, Copy, Debug)]
pub struct TriangleContact {
    /// Fraction of the requested motion, zero for an initial overlap.
    pub fraction: f64,
    /// Closest triangle point to the capsule axis at that time.
    pub contact: Vec3,
    /// Unit contact normal pointing from the triangle toward the capsule.
    pub normal: Vec3,
    pub feature: TriangleFeature,
}

/// Earliest contact of a capsule whose centre moves from `start` by `delta`
/// with a double-sided triangle. Caller validates finiteness and the capsule.
pub fn sweep_capsule_triangle(
    start: Vec3,
    delta: Vec3,
    capsule: VerticalCapsule,
    vertices: [Vec3; 3],
) -> Option<TriangleContact> {
    let r = capsule.radius;
    let (bottom, top) = capsule.axis(start);
    let initial = closest_segment_triangle(bottom, top, vertices);
    let limit = r - CONTACT_TOLERANCE;
    if initial.distance_squared < limit * limit {
        return Some(contact(initial, 0.0, delta, start, vertices));
    }
    if delta == [0.0; 3] {
        return None;
    }
    let fraction = first_entry(start, delta, capsule, vertices)?;
    let centre = add(start, scale(delta, fraction));
    let (bottom, top) = capsule.axis(centre);
    let closest = closest_segment_triangle(bottom, top, vertices);
    Some(contact(closest, fraction, delta, centre, vertices))
}

/// Earliest contact of capsule `mover` translating from `from` to `to` with
/// a stationary upright capsule `other` centred at `other_centre`. Returns the
/// fraction and the unit normal from `other`'s axis toward the mover. Both
/// axes are vertical, so this is the mover's centre ray against one capsule of
/// the summed radius and half-height.
pub fn sweep_vertical_capsules(
    from: Vec3,
    to: Vec3,
    mover: VerticalCapsule,
    other_centre: Vec3,
    other: VerticalCapsule,
) -> Option<(f64, Vec3)> {
    let combined = VerticalCapsule {
        radius: mover.radius + other.radius,
        half_height: mover.half_height + other.half_height,
    };
    let r = combined.radius;
    let delta = sub(to, from);
    let (low, high) = combined.axis(other_centre);
    let normal_at = |centre: Vec3| {
        let axis = closest_on_segment(centre, low, high).0;
        let away = sub(centre, axis);
        let length = norm(away);
        if length > 1e-12 {
            scale(away, 1.0 / length)
        } else if norm(delta) > 0.0 {
            scale(delta, -1.0 / norm(delta))
        } else {
            [1.0, 0.0, 0.0]
        }
    };
    let (point, _) = closest_on_segment(from, low, high);
    let limit = r - CONTACT_TOLERANCE;
    if dot(sub(from, point), sub(from, point)) < limit * limit {
        return Some((0.0, normal_at(from)));
    }
    if delta == [0.0; 3] {
        return None;
    }
    let mut best = cylinder_entry(from, delta, low, high, r);
    for corner in [low, high] {
        best = earlier(best, ball_entry(from, delta, corner, r));
    }
    best.map(|t| (t, normal_at(add(from, scale(delta, t)))))
}

fn earlier(a: Option<f64>, b: Option<f64>) -> Option<f64> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, None) => a,
        (None, b) => b,
    }
}

fn first_entry(start: Vec3, delta: Vec3, capsule: VerticalCapsule, v: [Vec3; 3]) -> Option<f64> {
    let r = capsule.radius;
    let h = capsule.half_height;
    let up = [0.0, h, 0.0];
    let low = v.map(|p| sub(p, up));
    let high = v.map(|p| add(p, up));
    let mut best = None;
    // Caps (one when the extrusion is zero).
    let face = cross(sub(v[1], v[0]), sub(v[2], v[0]));
    if let Some(n) = unit(face) {
        best = earlier(best, face_entry(start, delta, n, &low, r));
        if h > 0.0 {
            best = earlier(best, face_entry(start, delta, n, &high, r));
        }
    }
    for i in 0..3 {
        let j = (i + 1) % 3;
        if h > 0.0 {
            // Side parallelogram, ordered so its normal is edge × Y.
            if let Some(n) = unit(cross(sub(v[j], v[i]), [0.0, 1.0, 0.0])) {
                let side = [low[i], low[j], high[j], high[i]];
                best = earlier(best, face_entry(start, delta, n, &side, r));
            }
            best = earlier(best, cylinder_entry(start, delta, high[i], high[j], r));
            best = earlier(best, cylinder_entry(start, delta, low[i], high[i], r));
            best = earlier(best, ball_entry(start, delta, high[i], r));
        }
        best = earlier(best, cylinder_entry(start, delta, low[i], low[j], r));
        best = earlier(best, ball_entry(start, delta, low[i], r));
    }
    best
}

/// Entry through the offset plane, at distance `r` on the side the centre
/// starts on, of a convex planar polygon wound counterclockwise about `n`.
fn face_entry(start: Vec3, delta: Vec3, n: Vec3, polygon: &[Vec3], r: f64) -> Option<f64> {
    let origin = polygon[0];
    let mut s0 = dot(sub(start, origin), n);
    let mut approach = -dot(delta, n);
    if s0 < 0.0 {
        s0 = -s0;
        approach = -approach;
    }
    let s1 = s0 - approach;
    // Already past the offset plane without overlapping the face: any entry is
    // through a side wall, which the edge cylinders cover.
    if s0 < r - CONTACT_TOLERANCE || approach <= CONTACT_TOLERANCE || s1 > r {
        return None;
    }
    let t = ((s0 - r) / approach).clamp(0.0, 1.0);
    let centre = add(start, scale(delta, t));
    let foot = sub(centre, scale(n, dot(sub(centre, origin), n)));
    let inside = (0..polygon.len()).all(|i| {
        let a = polygon[i];
        let b = polygon[(i + 1) % polygon.len()];
        let edge = sub(b, a);
        dot(cross(edge, sub(foot, a)), n) >= -1e-12 * dot(edge, edge).max(1.0)
    });
    inside.then_some(t)
}

/// Entry into the solid cylinder of radius `r` about segment `a`-`b`, when
/// the entry point projects inside the segment.
fn cylinder_entry(start: Vec3, delta: Vec3, a: Vec3, b: Vec3, r: f64) -> Option<f64> {
    let d = sub(b, a);
    let m = sub(start, a);
    let dd = dot(d, d);
    if dd < 1e-18 {
        return None;
    }
    let (nd, md, nn) = (dot(delta, d), dot(m, d), dot(delta, delta));
    let qa = dd * nn - nd * nd;
    // Motion along the axis enters only through an end ball.
    if qa <= 1e-12 * dd * nn {
        return None;
    }
    let qb = dd * dot(m, delta) - nd * md;
    let qc = dd * (dot(m, m) - r * r) - md * md;
    let s = entry_root(qa, qb, qc, r, dd)?;
    let axial = (md + s * nd) / dd;
    (0.0..=1.0).contains(&axial).then_some(s)
}

fn ball_entry(start: Vec3, delta: Vec3, centre: Vec3, r: f64) -> Option<f64> {
    let m = sub(start, centre);
    let nn = dot(delta, delta);
    entry_root(nn, dot(m, delta), dot(m, m) - r * r, r, 1.0)
}

/// First root in [0, 1] of `qa t² + 2 qb t + qc = 0`, where the quadratic is
/// `scale · (distance² − r²)` along the ray. Rejects tangency (closest
/// approach within the tolerance of `r`) and a start inside that is not
/// moving deeper.
fn entry_root(qa: f64, qb: f64, qc: f64, r: f64, scale: f64) -> Option<f64> {
    if qa <= 0.0 {
        return None;
    }
    let disc = qb * qb - qa * qc;
    if disc <= 0.0 {
        return None;
    }
    let closest_squared = r * r - disc / (qa * scale);
    let limit = r - CONTACT_TOLERANCE;
    if closest_squared >= limit * limit {
        return None;
    }
    let root = disc.sqrt();
    // Numerically stable pair of roots.
    let q = -(qb + qb.signum() * root);
    let (mut t1, mut t2) = if q == 0.0 {
        (0.0, 0.0)
    } else {
        (q / qa, qc / q)
    };
    if t1 > t2 {
        std::mem::swap(&mut t1, &mut t2);
    }
    if t1 > 1.0 || t2 < 0.0 {
        return None;
    }
    if t1 >= 0.0 {
        return Some(t1);
    }
    // Starting inside (within the tolerance band): block only approach.
    (qb < 0.0).then_some(0.0)
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Closest {
    pub distance_squared: f64,
    /// Point on the axis segment.
    pub axis: Vec3,
    /// Point on the triangle.
    pub triangle: Vec3,
    pub feature: TriangleFeature,
}

/// Closest points between segment `p`-`q` and a triangle. Either the segment
/// crosses the triangle, or a closest pair has a segment endpoint or a point
/// on a triangle edge, so those candidates are exhaustive.
pub(crate) fn closest_segment_triangle(p: Vec3, q: Vec3, v: [Vec3; 3]) -> Closest {
    if let Some(point) = segment_crosses_triangle(p, q, v) {
        return Closest {
            distance_squared: 0.0,
            axis: point,
            triangle: point,
            feature: TriangleFeature::Face,
        };
    }
    let mut best: Option<Closest> = None;
    let mut consider = |candidate: Closest| {
        if best.is_none_or(|b| candidate.distance_squared < b.distance_squared) {
            best = Some(candidate);
        }
    };
    for endpoint in [p, q] {
        let (point, feature) = closest_on_triangle(endpoint, v);
        let gap = sub(endpoint, point);
        consider(Closest {
            distance_squared: dot(gap, gap),
            axis: endpoint,
            triangle: point,
            feature,
        });
        if p == q {
            break;
        }
    }
    for i in 0..3 {
        let j = (i + 1) % 3;
        let (on_axis, on_edge, s) = closest_segments(p, q, v[i], v[j]);
        let gap = sub(on_axis, on_edge);
        let feature = if s <= 0.0 {
            TriangleFeature::Vertex(i as u8)
        } else if s >= 1.0 {
            TriangleFeature::Vertex(j as u8)
        } else {
            TriangleFeature::Edge(i as u8)
        };
        consider(Closest {
            distance_squared: dot(gap, gap),
            axis: on_axis,
            triangle: on_edge,
            feature,
        });
    }
    best.expect("candidates are never empty")
}

fn contact(
    closest: Closest,
    fraction: f64,
    delta: Vec3,
    centre: Vec3,
    v: [Vec3; 3],
) -> TriangleContact {
    let face = unit(cross(sub(v[1], v[0]), sub(v[2], v[0])));
    let gap = sub(closest.axis, closest.triangle);
    let length = norm(gap);
    let normal = match (closest.feature, face) {
        (TriangleFeature::Face, Some(n)) => {
            let side = if length > 1e-12 {
                dot(gap, n)
            } else if delta != [0.0; 3] {
                -dot(delta, n)
            } else {
                dot(sub(centre, v[0]), n)
            };
            if side < 0.0 { scale(n, -1.0) } else { n }
        }
        _ if length > 1e-12 => scale(gap, 1.0 / length),
        (_, Some(n)) => {
            if dot(delta, n) > 0.0 {
                scale(n, -1.0)
            } else {
                n
            }
        }
        (_, None) => [0.0, 1.0, 0.0],
    };
    TriangleContact {
        fraction,
        contact: closest.triangle,
        normal,
        feature: closest.feature,
    }
}

fn segment_crosses_triangle(p: Vec3, q: Vec3, v: [Vec3; 3]) -> Option<Vec3> {
    let d = sub(q, p);
    let e1 = sub(v[1], v[0]);
    let e2 = sub(v[2], v[0]);
    let pv = cross(d, e2);
    let det = dot(e1, pv);
    if det.abs() < 1e-18 {
        return None;
    }
    let s = sub(p, v[0]);
    let u = dot(s, pv) / det;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let qv = cross(s, e1);
    let w = dot(d, qv) / det;
    if w < 0.0 || u + w > 1.0 {
        return None;
    }
    let t = dot(e2, qv) / det;
    (0.0..=1.0).contains(&t).then(|| add(p, scale(d, t)))
}

/// Closest point of a triangle to `p`, by which Voronoi region `p` is in.
fn closest_on_triangle(p: Vec3, v: [Vec3; 3]) -> (Vec3, TriangleFeature) {
    let [a, b, c] = v;
    let ab = sub(b, a);
    let ac = sub(c, a);
    let ap = sub(p, a);
    let d1 = dot(ab, ap);
    let d2 = dot(ac, ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return (a, TriangleFeature::Vertex(0));
    }
    let bp = sub(p, b);
    let d3 = dot(ab, bp);
    let d4 = dot(ac, bp);
    if d3 >= 0.0 && d4 <= d3 {
        return (b, TriangleFeature::Vertex(1));
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let s = d1 / (d1 - d3);
        return (add(a, scale(ab, s)), TriangleFeature::Edge(0));
    }
    let cp = sub(p, c);
    let d5 = dot(ab, cp);
    let d6 = dot(ac, cp);
    if d6 >= 0.0 && d5 <= d6 {
        return (c, TriangleFeature::Vertex(2));
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let s = d2 / (d2 - d6);
        return (add(a, scale(ac, s)), TriangleFeature::Edge(2));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let s = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return (add(b, scale(sub(c, b), s)), TriangleFeature::Edge(1));
    }
    let total = va + vb + vc;
    if total.abs() < 1e-300 {
        // Degenerate triangle with p over it: nearest of its edges.
        let candidates = [(a, b, 0u8), (b, c, 1), (c, a, 2)].map(|(x, y, e)| {
            let (point, s) = closest_on_segment(p, x, y);
            (point, s, e)
        });
        let (point, _, edge) = candidates
            .into_iter()
            .min_by(|l, r| {
                let dl = dot(sub(p, l.0), sub(p, l.0));
                let dr = dot(sub(p, r.0), sub(p, r.0));
                dl.total_cmp(&dr)
            })
            .expect("three edges");
        return (point, TriangleFeature::Edge(edge));
    }
    let s = vb / total;
    let w = vc / total;
    (
        add(a, add(scale(ab, s), scale(ac, w))),
        TriangleFeature::Face,
    )
}

/// Closest point on segment `a`-`b` to `p` and its parameter.
fn closest_on_segment(p: Vec3, a: Vec3, b: Vec3) -> (Vec3, f64) {
    let d = sub(b, a);
    let dd = dot(d, d);
    if dd <= 0.0 {
        return (a, 0.0);
    }
    let s = (dot(sub(p, a), d) / dd).clamp(0.0, 1.0);
    (add(a, scale(d, s)), s)
}

/// Closest points of segments `p1`-`q1` and `p2`-`q2`; returns both points and
/// the second segment's parameter.
fn closest_segments(p1: Vec3, q1: Vec3, p2: Vec3, q2: Vec3) -> (Vec3, Vec3, f64) {
    let d1 = sub(q1, p1);
    let d2 = sub(q2, p2);
    let r = sub(p1, p2);
    let a = dot(d1, d1);
    let e = dot(d2, d2);
    let f = dot(d2, r);
    let tiny = 1e-18;
    let (s, t);
    if a <= tiny && e <= tiny {
        return (p1, p2, 0.0);
    }
    if a <= tiny {
        s = 0.0;
        t = (f / e).clamp(0.0, 1.0);
    } else {
        let c = dot(d1, r);
        if e <= tiny {
            t = 0.0;
            s = (-c / a).clamp(0.0, 1.0);
        } else {
            let b = dot(d1, d2);
            let denom = a * e - b * b;
            let mut s0 = if denom > tiny * a * e {
                ((b * f - c * e) / denom).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let mut t0 = (b * s0 + f) / e;
            if t0 < 0.0 {
                t0 = 0.0;
                s0 = (-c / a).clamp(0.0, 1.0);
            } else if t0 > 1.0 {
                t0 = 1.0;
                s0 = ((b - c) / a).clamp(0.0, 1.0);
            }
            s = s0;
            t = t0;
        }
    }
    (add(p1, scale(d1, s)), add(p2, scale(d2, t)), t)
}

fn add(a: Vec3, b: Vec3) -> Vec3 {
    std::array::from_fn(|i| a[i] + b[i])
}
fn sub(a: Vec3, b: Vec3) -> Vec3 {
    std::array::from_fn(|i| a[i] - b[i])
}
fn scale(a: Vec3, s: f64) -> Vec3 {
    a.map(|v| v * s)
}
fn dot(a: Vec3, b: Vec3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn norm(a: Vec3) -> f64 {
    dot(a, a).sqrt()
}
fn unit(a: Vec3) -> Option<Vec3> {
    let length = norm(a);
    (length > 1e-12).then(|| scale(a, 1.0 / length))
}

#[cfg(test)]
#[path = "capsule_sweep_tests.rs"]
mod tests;
