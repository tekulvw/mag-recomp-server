//! Objective interaction range: a development point sphere or authored trigger boxes.
//!
//! Authored boxes follow the original engine's trigger construction: `axes` are
//! the box's unit axes as rows and `half_extents` are half of each authored
//! scale, so a point is inside when its offset from `centre` projects within the
//! half-extent on every axis. The original client registers characters by a
//! physics overlap whose reference point and padding are not established, so the
//! server applies its own body policy: the actor's vertical body column, from the
//! feet position to the body height, must overlap the box padded by the body
//! radius.
use crate::{bot_physics::Body, replication::CharacterPosition};
use serde::Deserialize;

pub(crate) const DEVELOPMENT_POINT_DISTANCE: &str = "development_point_distance";
pub(crate) const AUTHORED_TRIGGER_BOX: &str = "authored_trigger_box";
const AXIS_TOLERANCE: f64 = 1e-3;
const MAXIMUM_HALF_EXTENT_M: f32 = 1000.0;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TriggerBox {
    pub(crate) centre: [f32; 3],
    pub(crate) axes: [[f32; 3]; 3],
    pub(crate) half_extents: [f32; 3],
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn wide(v: [f32; 3]) -> [f64; 3] {
    v.map(f64::from)
}

impl TriggerBox {
    fn valid(&self) -> bool {
        let finite = self
            .centre
            .iter()
            .chain(self.axes.iter().flatten())
            .chain(self.half_extents.iter())
            .all(|v| v.is_finite());
        let extents = self
            .half_extents
            .iter()
            .all(|h| *h > 0.0 && *h <= MAXIMUM_HALF_EXTENT_M);
        let axes = self.axes.map(wide);
        let orthonormal = (0..3).all(|i| {
            (0..3).all(|j| {
                let expected = if i == j { 1.0 } else { 0.0 };
                (dot(axes[i], axes[j]) - expected).abs() <= AXIS_TOLERANCE
            })
        });
        finite && extents && orthonormal
    }
    /// Whether the segment from `feet` to `feet + height` meets the box grown by `pad`.
    pub(crate) fn overlaps_column(&self, feet: [f64; 3], height: f64, pad: f64) -> bool {
        let offset = std::array::from_fn::<f64, 3, _>(|i| feet[i] - f64::from(self.centre[i]));
        let (mut low, mut high) = (0.0f64, 1.0f64);
        for (axis, half) in self.axes.iter().zip(self.half_extents) {
            let axis = wide(*axis);
            let start = dot(offset, axis);
            let rise = height * axis[1];
            let limit = f64::from(half) + pad;
            if rise.abs() < 1e-12 {
                if start.abs() > limit {
                    return false;
                }
                continue;
            }
            let (a, b) = ((-limit - start) / rise, (limit - start) / rise);
            low = low.max(a.min(b));
            high = high.min(a.max(b));
            if low > high {
                return false;
            }
        }
        true
    }
    /// Vertical span of the box's world-space bounding box.
    pub(crate) fn vertical_bounds(&self) -> (f64, f64) {
        let reach: f64 = self
            .axes
            .iter()
            .zip(self.half_extents)
            .map(|(axis, half)| f64::from(axis[1]).abs() * f64::from(half))
            .sum();
        let y = f64::from(self.centre[1]);
        (y - reach, y + reach)
    }
    /// A standing candidate at fractions of the two most horizontal half-extents.
    pub(crate) fn candidate(&self, fraction: [f64; 2], body: Body) -> [f64; 3] {
        let mut order = [0usize, 1, 2];
        order.sort_by(|a, b| {
            f64::from(self.axes[*a][1])
                .abs()
                .total_cmp(&f64::from(self.axes[*b][1]).abs())
        });
        let mut point = wide(self.centre);
        for (&axis, f) in order[..2].iter().zip(fraction) {
            let usable = (f64::from(self.half_extents[axis]) - body.radius).max(0.0);
            for (p, a) in point.iter_mut().zip(self.axes[axis]) {
                *p += f * usable * f64::from(a);
            }
        }
        point
    }
}

/// Approach candidate fractions inside a box, centre first.
const BOX_CANDIDATES: [[f64; 2]; 9] = [
    [0.0, 0.0],
    [0.5, 0.0],
    [-0.5, 0.0],
    [0.0, 0.5],
    [0.0, -0.5],
    [0.5, 0.5],
    [-0.5, -0.5],
    [0.5, -0.5],
    [-0.5, 0.5],
];

#[derive(Clone, Copy, Debug)]
pub(crate) struct ObjectiveRange<'a> {
    pub(crate) position: [f32; 3],
    pub(crate) maximum_distance_m: f32,
    pub(crate) volumes: &'a [TriggerBox],
}

impl ObjectiveRange<'_> {
    pub(crate) fn validate(
        policy: &str,
        maximum: f32,
        triggers: u8,
        volumes: &[TriggerBox],
    ) -> bool {
        let maximum_valid = maximum.is_finite() && maximum > 0.0;
        match policy {
            DEVELOPMENT_POINT_DISTANCE => maximum_valid && volumes.is_empty(),
            AUTHORED_TRIGGER_BOX => {
                maximum_valid
                    && volumes.len() == usize::from(triggers)
                    && volumes.iter().all(TriggerBox::valid)
            }
            _ => false,
        }
    }
    pub(crate) fn policy(&self) -> &'static str {
        if self.volumes.is_empty() {
            DEVELOPMENT_POINT_DISTANCE
        } else {
            AUTHORED_TRIGGER_BOX
        }
    }
    fn point_distance(&self, p: [f64; 3]) -> f64 {
        (0..3)
            .map(|i| (p[i] - f64::from(self.position[i])).powi(2))
            .sum::<f64>()
            .sqrt()
    }
    /// Authoritative check for requests and holds, using the default body.
    pub(crate) fn contains(&self, p: CharacterPosition) -> bool {
        self.contains_feet([p.x, p.y, p.z].map(f64::from), 0.0)
    }
    /// Bot plans keep a margin so small motion does not leave the range.
    pub(crate) fn contains_planned(&self, feet: [f64; 3]) -> bool {
        if self.volumes.is_empty() {
            self.point_distance(feet) <= f64::from(self.maximum_distance_m) * 0.95
        } else {
            self.contains_feet(feet, -Body::default().radius)
        }
    }
    fn contains_feet(&self, feet: [f64; 3], shrink: f64) -> bool {
        if self.volumes.is_empty() {
            return self.point_distance(feet) <= f64::from(self.maximum_distance_m);
        }
        let body = Body::default();
        self.volumes
            .iter()
            .any(|v| v.overlaps_column(feet, body.height, body.radius + shrink))
    }
    /// The point bots use to rank operators: the first trigger centre, else the objective.
    pub(crate) fn anchor(&self) -> [f64; 3] {
        self.volumes
            .first()
            .map_or(wide(self.position), |v| wide(v.centre))
    }
    /// Raw approach candidate for one planning attempt (before ground support).
    pub(crate) fn approach(&self, attempt: usize) -> [f64; 3] {
        if let Some(volume) = self.volumes.get(attempt % self.volumes.len().max(1)) {
            let fraction = BOX_CANDIDATES[(attempt / self.volumes.len()) % BOX_CANDIDATES.len()];
            let mut p = volume.candidate(fraction, Body::default());
            p[1] = volume.vertical_bounds().1;
            return p;
        }
        // Generic concentric approaches; grounding is checked again against the
        // actual 3D trigger radius before accepting the physical route.
        let radius = f64::from(self.maximum_distance_m);
        let angle = (attempt % 8) as f64 * std::f64::consts::FRAC_PI_4;
        let radial = radius * [0.55, 0.25, 0.8][attempt / 8 % 3];
        let c = wide(self.position);
        [
            c[0] + radial * angle.cos(),
            c[1] + 0.5,
            c[2] + radial * angle.sin(),
        ]
    }
    /// Vertical support search window (up, down) from a raw candidate.
    pub(crate) fn support_window(&self, raw: [f64; 3]) -> (f64, f64) {
        if let Some(volume) = self.volumes.first() {
            let (bottom, top) = self
                .volumes
                .iter()
                .map(TriggerBox::vertical_bounds)
                .fold(volume.vertical_bounds(), |a, b| {
                    (a.0.min(b.0), a.1.max(b.1))
                });
            let body = Body::default();
            // Feet may sit below the box while the body column still overlaps it.
            return (
                (top - raw[1]).max(0.0),
                (raw[1] - (bottom - body.height)).max(0.0),
            );
        }
        // Restrict support discovery to this trigger's vertical sphere slice.
        // A generic +8m ray can hit a roof above the action instead of its floor.
        let c = wide(self.position);
        let radius = f64::from(self.maximum_distance_m);
        let horizontal = (raw[0] - c[0]).powi(2) + (raw[2] - c[2]).powi(2);
        let vertical = (radius.powi(2) - horizontal).max(0.).sqrt();
        (
            (c[1] + vertical - raw[1]).max(0.),
            (raw[1] - c[1] + vertical).max(0.),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Invented boxes only.
    fn yaw_box(degrees: f64) -> TriggerBox {
        let (s, c) = degrees.to_radians().sin_cos();
        TriggerBox {
            centre: [10.0, 5.0, -3.0],
            axes: [
                [c as f32, 0.0, -s as f32],
                [0.0, 1.0, 0.0],
                [s as f32, 0.0, c as f32],
            ],
            half_extents: [2.5, 1.0, 1.0],
        }
    }
    fn range(volumes: &[TriggerBox]) -> ObjectiveRange<'_> {
        ObjectiveRange {
            position: [10.0, 5.0, -3.0],
            maximum_distance_m: 5.0,
            volumes,
        }
    }
    fn at(x: f32, y: f32, z: f32) -> CharacterPosition {
        CharacterPosition { x, y, z }
    }

    #[test]
    fn box_range_rejects_points_inside_the_development_sphere() {
        let volumes = [yaw_box(0.0)];
        let r = range(&volumes);
        // 3.9 m off the 1 m half-extent: inside 5 m, outside the padded box.
        assert!(!r.contains(at(10.0, 4.5, 0.9)));
        assert!(range(&[]).contains(at(10.0, 4.5, 0.9)));
        assert!(r.contains(at(12.0, 4.5, -3.0)));
        // Body radius padding: 1.3 m off a 1 m half-extent with a 0.35 m body.
        assert!(r.contains(at(10.0, 4.5, -1.7)));
        assert!(!r.contains(at(10.0, 4.5, -1.6)));
    }

    #[test]
    fn body_column_reaches_a_box_above_the_feet() {
        let volumes = [yaw_box(0.0)];
        let r = range(&volumes);
        // Box spans y 4..6; feet 1.9 m below the bottom still overlap via the 1.8 m body.
        assert!(r.contains(at(10.0, 2.3, -3.0)));
        assert!(!r.contains(at(10.0, 1.8, -3.0)));
        assert!(!r.contains(at(10.0, 6.5, -3.0)));
    }

    #[test]
    fn rotated_box_uses_its_own_axes() {
        let volumes = [yaw_box(90.0)];
        let r = range(&volumes);
        // After a 90 degree yaw the long axis runs along world z.
        assert!(r.contains(at(10.0, 4.5, -5.0)));
        assert!(!r.contains(at(12.0, 4.5, -3.0)));
    }

    #[test]
    fn planned_candidates_are_inside_with_margin() {
        let volumes = [yaw_box(30.0), yaw_box(-60.0)];
        let r = range(&volumes);
        for attempt in 0..36 {
            let mut p = r.approach(attempt);
            p[1] = 4.5;
            assert!(r.contains_planned(p), "attempt {attempt}");
            assert!(r.contains(at(p[0] as f32, p[1] as f32, p[2] as f32)));
        }
        let (up, down) = r.support_window([10.0, 6.0, -3.0]);
        assert!(up.abs() < 1e-9 && (down - 3.8).abs() < 1e-9, "{up} {down}");
    }

    #[test]
    fn validation_requires_one_unit_box_per_trigger() {
        let good = yaw_box(15.0);
        assert!(ObjectiveRange::validate(
            AUTHORED_TRIGGER_BOX,
            5.0,
            1,
            &[good]
        ));
        assert!(!ObjectiveRange::validate(
            AUTHORED_TRIGGER_BOX,
            5.0,
            2,
            &[good]
        ));
        assert!(!ObjectiveRange::validate(
            DEVELOPMENT_POINT_DISTANCE,
            5.0,
            1,
            &[good]
        ));
        assert!(ObjectiveRange::validate(
            DEVELOPMENT_POINT_DISTANCE,
            5.0,
            1,
            &[]
        ));
        assert!(!ObjectiveRange::validate("unknown", 5.0, 1, &[]));
        let mut skew = good;
        skew.axes[0] = [1.0, 0.5, 0.0];
        assert!(!ObjectiveRange::validate(
            AUTHORED_TRIGGER_BOX,
            5.0,
            1,
            &[skew]
        ));
        let mut flat = good;
        flat.half_extents[1] = 0.0;
        assert!(!ObjectiveRange::validate(
            AUTHORED_TRIGGER_BOX,
            5.0,
            1,
            &[flat]
        ));
        let mut nan = good;
        nan.centre[0] = f32::NAN;
        assert!(!ObjectiveRange::validate(
            AUTHORED_TRIGGER_BOX,
            5.0,
            1,
            &[nan]
        ));
    }
}
