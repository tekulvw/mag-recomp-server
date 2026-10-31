//! Bunker-relative placement using explicit operator-supplied transforms and offsets.
//! This module contains no map positions, rotations, or model exit catalog defaults.
use crate::replication::CharacterPosition;
use serde::Deserialize;
use std::{io, path::Path};

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BunkerTransform {
    pub position: CharacterPosition,
    pub rotation_xyzw: [f32; 4],
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExitCandidate {
    /// Model-local direction used to choose the exit facing the requested camera.
    pub ranking_offset: [f32; 3],
    /// Model-local settled position; the operator owns terrain/clearance validation.
    pub position_offset: [f32; 3],
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BunkerExitPolicy {
    pub transform: BunkerTransform,
    pub candidates: Vec<ExitCandidate>,
}

#[derive(Clone, Copy, Debug)]
pub struct BunkerExit {
    pub candidate_index: usize,
    pub position: CharacterPosition,
    pub packed_yaw: u16,
}

impl BunkerTransform {
    pub(crate) fn rotate(&self, [x, y, z]: [f32; 3]) -> [f32; 3] {
        let [qx, qy, qz, qw] = self.rotation_xyzw;
        let t = [
            2.0 * (qy * z - qz * y),
            2.0 * (qz * x - qx * z),
            2.0 * (qx * y - qy * x),
        ];
        [
            x + qw * t[0] + qy * t[2] - qz * t[1],
            y + qw * t[1] + qz * t[0] - qx * t[2],
            z + qw * t[2] + qx * t[1] - qy * t[0],
        ]
    }
}

impl BunkerExitPolicy {
    pub fn load(path: impl AsRef<Path>) -> io::Result<Self> {
        let value: Self = serde_json::from_reader(std::fs::File::open(path)?)?;
        value.validate().map_err(io::Error::other)?;
        Ok(value)
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        self.transform.position.validate()?;
        let quaternion = self.transform.rotation_xyzw;
        let norm = quaternion.iter().map(|v| v * v).sum::<f32>();
        if quaternion.iter().any(|v| !v.is_finite())
            || (norm - 1.).abs() > 0.001
            || self.candidates.is_empty()
            || self.candidates.len() > 32
        {
            return Err("invalid_bunker_exit_policy");
        }
        for candidate in &self.candidates {
            if candidate
                .ranking_offset
                .iter()
                .chain(&candidate.position_offset)
                .any(|v| !v.is_finite() || v.abs() >= 2048.)
                || candidate.ranking_offset.iter().map(|v| v * v).sum::<f32>() <= f32::EPSILON
            {
                return Err("invalid_bunker_exit_candidate");
            }
            self.position(candidate.position_offset).validate()?;
        }
        Ok(())
    }
    fn position(&self, local: [f32; 3]) -> CharacterPosition {
        let offset = self.transform.rotate(local);
        CharacterPosition {
            x: self.transform.position.x + offset[0],
            y: self.transform.position.y + offset[1],
            z: self.transform.position.z + offset[2],
        }
    }
    /// Rank supplied candidates using camera facing, then transform its supplied
    /// settled offset. An invalid camera selects the first explicit candidate.
    pub fn exit(&self, camera_degrees: u32) -> Result<BunkerExit, &'static str> {
        self.validate()?;
        let mut selected = 0;
        let mut best = f32::INFINITY;
        if camera_degrees <= 360 {
            for (index, candidate) in self.candidates.iter().enumerate() {
                let d = self.transform.rotate(candidate.ranking_offset);
                let length = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
                let mut angle = (-d[2] / length).clamp(-1.0, 1.0).acos();
                if d[0] > 0.0 {
                    angle = -angle;
                }
                let degrees = (angle * 57.295_78_f32).rem_euclid(360.0);
                let delta = (camera_degrees as f32 - degrees).abs();
                let distance = delta.min(360.0 - delta);
                if distance < best {
                    best = distance;
                    selected = index;
                }
            }
        }
        let [x, y, z, w] = self.transform.rotation_xyzw;
        let radians = if camera_degrees <= 360 {
            camera_degrees as f32 * 0.017_453_292_f32
        } else {
            (2.0 * (w * y - x * z))
                .atan2(1.0 - 2.0 * (x * x + y * y))
                .rem_euclid(std::f32::consts::TAU)
        };
        Ok(BunkerExit {
            candidate_index: selected,
            position: self.position(self.candidates[selected].position_offset),
            packed_yaw: ((radians / std::f32::consts::TAU * 1024.0) as u16) & 1023,
        })
    }
}
