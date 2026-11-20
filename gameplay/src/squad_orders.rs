//! Authored squad-order timing and round-owned waypoint state.

use std::{io, path::Path};

use serde::Deserialize;

use crate::replication::CharacterPosition;

const CHARACTER_GAMEPLAY_MEMBER: &str = "/run/mag/ps3/scripts/character_gameplay.zpr";
const SQUAD_COUNT: usize = 32;
const ORDER_ALLOWANCE_MS: u32 = 500;

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rules {
    pub squad_order_cooldown_ms: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    archive_sha256: String,
    member: String,
    member_sha256: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Projection {
    schema_version: u32,
    application_id: u32,
    source: Source,
    settings: Rules,
}

impl Rules {
    pub fn load(path: impl AsRef<Path>) -> io::Result<Self> {
        Self::from_json(&std::fs::read(path)?)
    }

    pub fn from_json(bytes: &[u8]) -> io::Result<Self> {
        let projection: Projection = serde_json::from_slice(bytes)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let valid_sha256 = |digest: &str| {
            digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
        };
        if projection.schema_version != 1
            || projection.application_id != 20231
            || projection.source.member != CHARACTER_GAMEPLAY_MEMBER
            || !valid_sha256(&projection.source.archive_sha256)
            || !valid_sha256(&projection.source.member_sha256)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid retail squad order rules projection",
            ));
        }
        Ok(projection.settings)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Waypoint {
    pub position: CharacterPosition,
    pub active: bool,
}

#[derive(Clone, Copy, Debug, Default)]
struct Squad {
    last_set_ms: u32,
    waypoint: Option<Waypoint>,
}

#[derive(Clone, Debug, Default)]
pub struct State {
    squads: [Squad; SQUAD_COUNT],
}

impl State {
    pub fn can_set(&self, squad: u16, now_ms: u32, rules: &Rules) -> bool {
        self.squads.get(usize::from(squad)).is_some_and(|squad| {
            let last = squad.last_set_ms;
            now_ms <= last
                || now_ms.wrapping_sub(last).wrapping_add(ORDER_ALLOWANCE_MS)
                    > rules.squad_order_cooldown_ms
        })
    }

    pub fn stamp_set(&mut self, squad: u16, now_ms: u32) {
        if let Some(squad) = self.squads.get_mut(usize::from(squad)) {
            squad.last_set_ms = now_ms;
        }
    }

    pub fn waypoint(&self, squad: u16) -> Option<Waypoint> {
        self.squads.get(usize::from(squad))?.waypoint
    }

    pub fn set_waypoint(&mut self, squad: u16, position: CharacterPosition, now_ms: u32) {
        if let Some(squad) = self.squads.get_mut(usize::from(squad)) {
            squad.last_set_ms = now_ms;
            squad.waypoint = Some(Waypoint {
                position,
                active: true,
            });
        }
    }

    pub fn clear_waypoint(&mut self, squad: u16) {
        if let Some(waypoint) = self
            .squads
            .get_mut(usize::from(squad))
            .and_then(|squad| squad.waypoint.as_mut())
        {
            waypoint.active = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_the_source_bound_projection() {
        let mut raw = serde_json::json!({
            "schema_version": 1,
            "application_id": 20231,
            "source": {
                "archive_sha256": "a".repeat(64),
                "member": CHARACTER_GAMEPLAY_MEMBER,
                "member_sha256": "b".repeat(64),
            },
            "settings": {"squad_order_cooldown_ms": 1800},
        });
        let parse =
            |value: &serde_json::Value| Rules::from_json(&serde_json::to_vec(value).unwrap());
        assert_eq!(parse(&raw).unwrap().squad_order_cooldown_ms, 1800);
        raw["source"]["member_sha256"] = serde_json::json!("g".repeat(64));
        assert_eq!(parse(&raw).unwrap_err().kind(), io::ErrorKind::InvalidData);
        raw["source"]["member_sha256"] = serde_json::json!("b".repeat(64));
        raw["source"]["member"] = serde_json::json!("wrong-member");
        assert_eq!(parse(&raw).unwrap_err().kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn set_gate_matches_native_unsigned_boundaries() {
        let rules = Rules {
            squad_order_cooldown_ms: 1800,
        };
        let mut state = State::default();
        assert!(!state.can_set(0, 1300, &rules));
        assert!(state.can_set(0, 1301, &rules));
        state.stamp_set(0, 2000);
        assert!(state.can_set(0, 2000, &rules));
        assert!(state.can_set(0, 1999, &rules));
        assert!(!state.can_set(0, 3300, &rules));
        assert!(state.can_set(0, 3301, &rules));
        assert!(state.can_set(1, 3300, &rules));
        state.stamp_set(0, 1);
        assert!(!state.can_set(0, u32::MAX, &rules));
        assert!(!state.can_set(32, 2000, &rules));
    }

    #[test]
    fn clear_retains_waypoint_position_and_shared_stamp() {
        let rules = Rules {
            squad_order_cooldown_ms: 1800,
        };
        let mut state = State::default();
        let position = crate::replication::CharacterPosition {
            x: 1.0,
            y: -2.0,
            z: 3.0,
        };
        state.set_waypoint(17, position, 4000);
        assert_eq!(
            state.waypoint(17),
            Some(Waypoint {
                position,
                active: true
            })
        );
        state.clear_waypoint(17);
        assert_eq!(
            state.waypoint(17),
            Some(Waypoint {
                position,
                active: false
            })
        );
        assert!(!state.can_set(17, 4001, &rules));
        assert!(state.can_set(16, 4001, &rules));
        assert!(state.waypoint(32).is_none());
    }
}
