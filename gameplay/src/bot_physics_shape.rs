//! The shape a bot body moves with, and the external body profile that
//! selects it.
//!
//! `Box` is the conservative upright box every existing artifact and
//! certificate was traced with; it stays the default. `Capsule` is an upright
//! capsule whose centre sits `centre_height` above the body's feet point. The
//! shape is an identity: navmesh artifacts and drop certificates record the
//! shape they were traced with, and a mesh loads only for the same shape
//! (see `navmesh::NavMesh::load_with_body`). Capsule dimensions are external
//! data; nothing here supplies game values.
use crate::capsule_sweep::VerticalCapsule;
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{self, Read},
    path::Path,
};

/// Format string of a body profile document.
pub const BODY_PROFILE_FORMAT: &str = "mag_bot_body_profile_v1";
const MAX_PROFILE_BYTES: u64 = 4096;

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "snake_case", try_from = "RawShape")]
pub enum BodyShape {
    /// The upright box of the body's `radius` and `height`, feet at its base.
    #[default]
    Box,
    /// An upright capsule: every point within `radius` of the vertical axis
    /// segment `half_height` above and below its centre, which is
    /// `centre_height` above the feet point.
    Capsule {
        radius: f64,
        half_height: f64,
        centre_height: f64,
    },
}

/// The recorded form, read strictly: a box takes no dimensions and a
/// capsule needs all three.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawShape {
    shape: RawKind,
    radius: Option<f64>,
    half_height: Option<f64>,
    centre_height: Option<f64>,
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum RawKind {
    Box,
    Capsule,
}
impl TryFrom<RawShape> for BodyShape {
    type Error = &'static str;
    fn try_from(raw: RawShape) -> Result<Self, Self::Error> {
        match (raw.shape, raw.radius, raw.half_height, raw.centre_height) {
            (RawKind::Box, None, None, None) => Ok(Self::Box),
            (RawKind::Capsule, Some(radius), Some(half_height), Some(centre_height)) => {
                Ok(Self::Capsule {
                    radius,
                    half_height,
                    centre_height,
                })
            }
            _ => Err("body shape dimensions do not match its kind"),
        }
    }
}

impl BodyShape {
    /// Dimensions within the body limits (`Body::validate`): radius 0.05 to
    /// 2 m, top 0.2 to 4 m above the feet, and the capsule's lowest point
    /// within half a metre of the feet.
    pub fn valid(self) -> bool {
        match self {
            Self::Box => true,
            Self::Capsule {
                radius,
                half_height,
                centre_height,
            } => {
                [radius, half_height, centre_height]
                    .iter()
                    .all(|v| v.is_finite())
                    && (0.05..=2.0).contains(&radius)
                    && (0.0..=2.0).contains(&half_height)
                    && (0.2..=4.0).contains(&(centre_height + half_height + radius))
                    && (-0.5..=0.5).contains(&(centre_height - half_height - radius))
            }
        }
    }
    /// How artifacts and certificates record the shape: absent for the box,
    /// so box-certified data serializes exactly as before shapes existed.
    pub fn stamp(self) -> Option<Self> {
        (self != Self::Box).then_some(self)
    }
    /// The shape a recorded stamp names; absent means the box.
    pub fn from_stamp(stamp: Option<Self>) -> Self {
        stamp.unwrap_or_default()
    }
    pub(crate) fn capsule(self) -> Option<(VerticalCapsule, f64)> {
        match self {
            Self::Box => None,
            Self::Capsule {
                radius,
                half_height,
                centre_height,
            } => Some((
                VerticalCapsule {
                    radius,
                    half_height,
                },
                centre_height,
            )),
        }
    }

    /// Load an external body profile document:
    /// `{"format":"mag_bot_body_profile_v1","body":{"shape":"capsule",...}}`.
    pub fn load_profile(path: impl AsRef<Path>) -> io::Result<Self> {
        let mut bytes = Vec::new();
        File::open(path)?
            .take(MAX_PROFILE_BYTES + 1)
            .read_to_end(&mut bytes)?;
        Self::profile_from_bytes(&bytes).map_err(io::Error::other)
    }

    pub fn profile_from_bytes(bytes: &[u8]) -> Result<Self, &'static str> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Document {
            format: String,
            body: BodyShape,
        }
        if bytes.len() as u64 > MAX_PROFILE_BYTES {
            return Err("bot_body_profile_too_large");
        }
        let document: Document =
            serde_json::from_slice(bytes).map_err(|_| "bot_body_profile_invalid_document")?;
        if document.format != BODY_PROFILE_FORMAT {
            return Err("bot_body_profile_invalid_document");
        }
        if !document.body.valid() {
            return Err("bot_body_profile_invalid_shape");
        }
        Ok(document.body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_body_profile_selects_a_validated_shape_and_refuses_bad_input() {
        let capsule = br#"{"format":"mag_bot_body_profile_v1","body":{"shape":"capsule","radius":0.4,"half_height":0.5,"centre_height":0.9}}"#;
        assert_eq!(
            BodyShape::profile_from_bytes(capsule),
            Ok(BodyShape::Capsule {
                radius: 0.4,
                half_height: 0.5,
                centre_height: 0.9
            })
        );
        let boxed = br#"{"format":"mag_bot_body_profile_v1","body":{"shape":"box"}}"#;
        assert_eq!(BodyShape::profile_from_bytes(boxed), Ok(BodyShape::Box));
        for (bytes, error) in [
            (
                &br#"{"format":"mag_bot_body_profile_v2","body":{"shape":"box"}}"#[..],
                "bot_body_profile_invalid_document",
            ),
            (
                br#"{"format":"mag_bot_body_profile_v1","body":{"shape":"sphere"}}"#,
                "bot_body_profile_invalid_document",
            ),
            (
                br#"{"format":"mag_bot_body_profile_v1","body":{"shape":"capsule","radius":0.4,"half_height":0.5}}"#,
                "bot_body_profile_invalid_document",
            ),
            (
                br#"{"format":"mag_bot_body_profile_v1","body":{"shape":"capsule","radius":0.4,"half_height":0.5,"centre_height":0.9,"extra":1}}"#,
                "bot_body_profile_invalid_document",
            ),
            (
                br#"{"format":"mag_bot_body_profile_v1","body":{"shape":"box","radius":0.4}}"#,
                "bot_body_profile_invalid_document",
            ),
            (
                br#"{"format":"mag_bot_body_profile_v1","body":{"shape":"capsule","radius":0.0,"half_height":0.5,"centre_height":0.9}}"#,
                "bot_body_profile_invalid_shape",
            ),
            // Lowest point 1.5 m above the feet.
            (
                br#"{"format":"mag_bot_body_profile_v1","body":{"shape":"capsule","radius":0.4,"half_height":0.5,"centre_height":2.4}}"#,
                "bot_body_profile_invalid_shape",
            ),
        ] {
            assert_eq!(BodyShape::profile_from_bytes(bytes), Err(error));
        }
        assert_eq!(
            BodyShape::profile_from_bytes(&[b' '; 5000]),
            Err("bot_body_profile_too_large")
        );
    }

    #[test]
    fn the_box_is_recorded_as_absent() {
        assert_eq!(BodyShape::Box.stamp(), None);
        assert_eq!(BodyShape::from_stamp(None), BodyShape::Box);
        let capsule = BodyShape::Capsule {
            radius: 0.4,
            half_height: 0.5,
            centre_height: 0.9,
        };
        assert_eq!(BodyShape::from_stamp(capsule.stamp()), capsule);
    }
}
