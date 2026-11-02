//! Source-bound scalar parameters for the native temporary DamageVolume box.
//! No overlap query, matrix/trig approximation or current body/filter proof.
#[derive(Clone, Copy, Debug)]
pub struct Transform {
    pub translation: [f32; 3],
    pub rotation_degrees: [f32; 3],
    pub scale: [f32; 3],
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Parameters {
    pub center: [f32; 3],
    pub half_extents: [f32; 3],
    /// Native rotation acts as Rz * Ry * Rx on column vectors. Constructing its
    /// actual float32 matrix still needs the qualified native arithmetic path.
    pub rotation_radians: [f32; 3],
    pub margin: f32,
}
impl Transform {
    /// Reproduce scalar float32 setup only. Native lifts world Y by half scaleY
    /// before installing translation; it does not rotate this center adjustment.
    /// Negative/zero extents are retained, not claimed to be valid query shapes.
    pub fn parameters(self) -> Result<Parameters, &'static str> {
        if self
            .translation
            .into_iter()
            .chain(self.rotation_degrees)
            .chain(self.scale)
            .any(|v| !v.is_finite())
        {
            return Err("unsupported_volume_transform");
        }
        let half_extents = self.scale.map(|v| v * 0.5_f32);
        let center = [
            self.translation[0],
            self.translation[1] + half_extents[1],
            self.translation[2],
        ];
        let rotation_radians = self
            .rotation_degrees
            .map(|v| v * f32::from_bits(0x3c8efa35));
        if center
            .into_iter()
            .chain(rotation_radians)
            .any(|v| !v.is_finite())
        {
            return Err("unsupported_volume_transform_arithmetic");
        }
        Ok(Parameters {
            center,
            half_extents,
            rotation_radians,
            margin: f32::from_bits(0x3d4ccccd),
        })
    }
}

/// The temporary query has raw filter6 (system group0). Its pair-level group
/// filter checks the target's low5-bit layer against the CURRENT row6 mask;
/// target subsystem/group bits do not select the same-system-group exception.
/// Shape-level filters, live body coverage and actual penetration remain separate.
pub fn filter6_allows(current_row6: u32, target_filter: u32) -> bool {
    current_row6 & (1_u32 << (target_filter & 31)) != 0
}
