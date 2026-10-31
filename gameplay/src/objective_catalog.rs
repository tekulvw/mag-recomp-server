//! Reviewed retail objective bindings and authored initial orders, loaded from disk.
use crate::{protocol, replication, spawn_catalog, wire_profile};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    io::{self, Read},
    path::Path,
};

const MAX_BYTES: usize = 65_536;
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectiveSource {
    pub authored_catalog_sha256: String,
    pub binding_capture_sha256: String,
    pub startup_proof_sha256: String,
    pub source_image_sha256: String,
    pub capture_qualification: String,
    #[serde(default)]
    pub rules_catalog_sha256: Option<String>,
    #[serde(default)]
    pub game_rules_sha256: Option<String>,
    #[serde(default)]
    pub game_rules_json_pointer: Option<String>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Objective {
    token: String,
    pub(crate) object_index: u16,
    pub(crate) initial_state: u8,
    #[serde(default)]
    pub(crate) scoring: Option<ObjectiveScoring>,
    #[serde(default)]
    pub(crate) reward: Option<ObjectiveRewardRule>,
    #[serde(default)]
    pub(crate) interaction: Option<ObjectiveInteractionRule>,
    #[serde(default)]
    pub(crate) demolition: Option<ObjectiveDemolitionRule>,
}
/// Authored classification, independent of XP values in the progression catalog.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ObjectiveRewardRule {
    pub(crate) category: u8,
    // Retained for future subtype-specific destruction stats; current generic actions use category/class.
    #[allow(dead_code)]
    pub(crate) subtype: u8,
    /// Verified ObjectiveDemolitionStructuralProperties native virtual type 1.
    pub(crate) demolition: bool,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ObjectiveScoring {
    pub(crate) owner_team: u8,
    pub(crate) activation_score: u32,
    pub(crate) deactivation_score: u32,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MatchRules {
    pub(crate) end_game_delay_ms: u32,
    #[serde(default)]
    pub(crate) local_end_game_delay_ms: Option<u32>,
    pub(crate) major_victory_percent: u8,
}
/// Authored timing/position and explicit development range policy, loaded from disk.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ObjectiveInteractionRule {
    pub(crate) owner_team: u8,
    pub(crate) activate_ms: u32,
    pub(crate) complete_ms: u32,
    pub(crate) deactivate_ms: u32,
    pub(crate) can_deactivate: bool,
    pub(crate) position: [f32; 3],
    pub(crate) trigger_count: u8,
    pub(crate) maximum_distance_m: f32,
    pub(crate) distance_policy: String,
}
/// Structural demolition timing and capability from authored retail properties.
/// Score values remain configuration; this type alone does not apply score effects.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ObjectiveDemolitionRule {
    pub(crate) owner_team: u8,
    pub(crate) arm_ms: u32,
    pub(crate) detonate_ms: u32,
    pub(crate) defuse_ms: u32,
    pub(crate) can_defuse: bool,
    pub(crate) activation_score: u32,
    pub(crate) deactivation_score: u32,
    pub(crate) position: [f32; 3],
    pub(crate) trigger_count: u8,
    pub(crate) maximum_distance_m: f32,
    pub(crate) distance_policy: String,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ObjectiveProgressionRule {
    pub(crate) prerequisites: Vec<u16>,
    pub(crate) lock: Vec<u16>,
    pub(crate) complete: Vec<u16>,
    pub(crate) delayed_objective: u16,
    pub(crate) delay_ms: u32,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ObjectiveScoreBleed {
    pub(crate) team: u8,
    pub(crate) interval_ms: u32,
    pub(crate) points: i16,
    pub(crate) pause_objective: u16,
    pub(crate) pause_state: u8,
    #[serde(default)]
    pub(crate) pause_states: Vec<u8>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthoredObjective {
    object_index: u16,
    // Authored metadata only; the first available source-list entry determines the initial order.
    order: u32,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Squad {
    team: u8,
    squad_index: u16,
    initial_objective_index: u16,
    authored_objectives: Vec<AuthoredObjective>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectiveCatalog {
    schema_version: u8,
    format: String,
    application_id: u32,
    protocol: u32,
    game_type: i8,
    map_name: String,
    entity_family: u8,
    network_type: u8,
    source: ObjectiveSource,
    pub(crate) objects: Vec<Objective>,
    squads: Vec<Squad>,
    #[serde(default)]
    pub(crate) progression: Option<ObjectiveProgressionRule>,
    #[serde(default)]
    pub(crate) score_bleed: Option<ObjectiveScoreBleed>,
    #[serde(default)]
    pub(crate) match_rules: Option<MatchRules>,
}
impl ObjectiveCatalog {
    pub fn load(path: impl AsRef<Path>) -> io::Result<Self> {
        let mut bytes = Vec::new();
        std::fs::File::open(path)?
            .take((MAX_BYTES + 1) as u64)
            .read_to_end(&mut bytes)?;
        let text = std::str::from_utf8(&bytes).map_err(io::Error::other)?;
        Self::from_json(text)
    }
    pub fn from_json(text: &str) -> io::Result<Self> {
        if text.len() > MAX_BYTES {
            return Err(io::Error::other("objective_catalog_too_large"));
        }
        let value: Self = serde_json::from_str(text).map_err(io::Error::other)?;
        value.validate().map_err(io::Error::other)?;
        Ok(value)
    }
    fn validate(&self) -> Result<(), &'static str> {
        if self.schema_version != 1
            || self.format != "mag_retail_objective_catalog"
            || self.application_id != 20231
            || self.protocol != protocol::ProtocolInfo::RETAIL_PROTOCOL
            || self.game_type != 0
            || self.entity_family != 12
            || self.network_type != 12
            || self.map_name.is_empty()
            || self.map_name.len() > 128
            || !self
                .map_name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
            || self.source.capture_qualification != "bounded_concurrent_token_slot_capture"
            || [
                &self.source.authored_catalog_sha256,
                &self.source.binding_capture_sha256,
                &self.source.startup_proof_sha256,
                &self.source.source_image_sha256,
            ]
            .iter()
            .any(|hash| hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            return Err("invalid_objective_catalog_identity");
        }
        if self.match_rules.as_ref().is_some_and(|r| {
            r.end_game_delay_ms > 60_000
                || r.major_victory_percent > 100
                || r.local_end_game_delay_ms
                    .is_some_and(|v| v == 0 || v > 60_000)
        }) || self
            .objects
            .iter()
            .filter_map(|o| o.scoring.as_ref())
            .any(|s| {
                s.owner_team > 1
                    || s.activation_score > i16::MAX as u32
                    || s.deactivation_score > i16::MAX as u32
            })
            || [
                &self.source.rules_catalog_sha256,
                &self.source.game_rules_sha256,
            ]
            .iter()
            .filter_map(|v| v.as_ref())
            .any(|s| s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            return Err("invalid_objective_match_rules");
        }
        if self.objects.is_empty() || self.objects.len() > 128 || self.squads.len() != 8 {
            return Err("invalid_objective_catalog_count");
        }
        let mut tokens = BTreeSet::new();
        let mut indices = BTreeSet::new();
        let mut previous = None;
        for object in &self.objects {
            let token = object
                .token
                .strip_prefix("0x")
                .filter(|s| s.len() == 8)
                .and_then(|s| u32::from_str_radix(s, 16).ok())
                .ok_or("invalid_objective_token")?;
            if object.object_index >= 128
                || previous.is_some_and(|p| object.object_index <= p)
                || !tokens.insert(token)
                || !indices.insert(object.object_index)
                || native_state(object.initial_state).is_none()
            {
                return Err("invalid_objective_binding");
            }
            if object.reward.as_ref().is_some_and(|rule| {
                rule.category > 4
                    || (object.demolition.is_some() && !rule.demolition)
                    || (object.interaction.is_some() && rule.demolition)
            }) {
                return Err("invalid_objective_reward_rule");
            }
            if let Some(rule) = &object.interaction {
                if rule.owner_team > 1
                    || rule.activate_ms == 0
                    || rule.complete_ms == 0
                    || rule.deactivate_ms == 0
                    || rule.trigger_count == 0
                    || rule.position.iter().any(|value| !value.is_finite())
                    || !rule.maximum_distance_m.is_finite()
                    || rule.maximum_distance_m <= 0.0
                    || rule.distance_policy != "development_point_distance"
                {
                    return Err("invalid_objective_interaction_rule");
                }
                // Retained authored capability; defender handling follows separately.
                let _can_deactivate = rule.can_deactivate;
            }
            if let Some(rule) = &object.demolition {
                if object.interaction.is_some()
                    || rule.owner_team > 1
                    || [rule.arm_ms, rule.detonate_ms, rule.defuse_ms]
                        .iter()
                        .any(|duration| *duration == 0 || *duration > 3_600_000)
                    || rule.trigger_count == 0
                    || rule.position.iter().any(|value| !value.is_finite())
                    || !rule.maximum_distance_m.is_finite()
                    || rule.maximum_distance_m <= 0.0
                    || rule.distance_policy != "development_point_distance"
                {
                    return Err("invalid_objective_demolition_rule");
                }
                // Retain authored scores until their native application is proved.
                let _scores = (rule.activation_score, rule.deactivation_score);
            }
            previous = Some(object.object_index);
        }
        if let Some(rule) = &self.progression {
            let valid_refs = |refs: &[u16]| {
                !refs.is_empty()
                    && refs.len() <= self.objects.len()
                    && refs.iter().all(|index| indices.contains(index))
                    && refs.iter().copied().collect::<BTreeSet<_>>().len() == refs.len()
            };
            if !valid_refs(&rule.prerequisites)
                || !valid_refs(&rule.lock)
                || !valid_refs(&rule.complete)
                || !rule
                    .prerequisites
                    .iter()
                    .all(|index| rule.lock.contains(index))
                || rule.lock.iter().any(|index| rule.complete.contains(index))
                || !indices.contains(&rule.delayed_objective)
                || rule.lock.contains(&rule.delayed_objective)
                || rule.complete.contains(&rule.delayed_objective)
                || rule.delay_ms == 0
                || rule.delay_ms > 3_600_000
            {
                return Err("invalid_objective_progression_rule");
            }
        }
        if let Some(rule) = &self.score_bleed
            && (rule.team > 1
                || rule.interval_ms == 0
                || rule.interval_ms > 3_600_000
                || rule.points <= 0
                || !indices.contains(&rule.pause_objective)
                || native_state(rule.pause_state).is_none()
                || rule.pause_states.len() > 13
                || rule
                    .pause_states
                    .iter()
                    .any(|state| native_state(*state).is_none())
                || rule
                    .pause_states
                    .iter()
                    .copied()
                    .collect::<BTreeSet<_>>()
                    .len()
                    != rule.pause_states.len()
                || (!rule.pause_states.is_empty()
                    && !rule.pause_states.contains(&rule.pause_state))
                || self.progression.as_ref().is_some_and(|progression| {
                    progression.delayed_objective != rule.pause_objective
                }))
        {
            return Err("invalid_objective_score_bleed");
        }
        let mut squads = BTreeSet::new();
        for squad in &self.squads {
            if ![0, 1, 2, 3, 16, 17, 18, 19].contains(&squad.squad_index)
                || u16::from(squad.team) != squad.squad_index / 16
                || !squads.insert(squad.squad_index)
                || squad.authored_objectives.is_empty()
                || squad.authored_objectives.len() > 128
            {
                return Err("invalid_objective_squad");
            }
            for authored in &squad.authored_objectives {
                // Duplicate orders are intentional authored metadata, not sorting keys.
                let _order = authored.order;
                if !indices.contains(&authored.object_index) {
                    return Err("invalid_objective_reference");
                }
            }
            // Preserve every source-list entry, including repeated objects/orders.
            // A deactivated/activated objective is available for initial orders;
            // inactive/completed/locked startup states are not HUD targets.
            let first_available = squad
                .authored_objectives
                .iter()
                .find(|authored| {
                    self.objects.iter().any(|object| {
                        object.object_index == authored.object_index
                            && matches!(object.initial_state, 1 | 3)
                    })
                })
                .map(|authored| authored.object_index);
            if first_available != Some(squad.initial_objective_index) {
                return Err("invalid_initial_objective_assignment");
            }
        }
        Ok(())
    }
    /// Validate match policy before construction; wire identity is known at ConfigureMatch.
    pub fn validate_match(
        &self,
        map: &str,
        policy: spawn_catalog::MatchProfile,
    ) -> Result<(), &'static str> {
        self.validate()?;
        if self.map_name != map || self.game_type != policy.game_type || !policy.has_objectives {
            return Err("objective_catalog_match_mismatch");
        }
        Ok(())
    }
    pub fn validate_profile(&self, profile: wire_profile::WireProfile) -> Result<(), &'static str> {
        if profile.application_id() != self.application_id
            || profile.object_type_bits() != 20
            || self.protocol != protocol::ProtocolInfo::RETAIL_PROTOCOL
        {
            return Err("objective_catalog_wire_mismatch");
        }
        Ok(())
    }
    pub fn source(&self) -> &ObjectiveSource {
        &self.source
    }
    pub fn object_count(&self) -> usize {
        self.objects.len()
    }
    pub fn initial_assignment(&self, squad_index: u16) -> Result<u16, &'static str> {
        self.squads
            .iter()
            .find(|s| s.squad_index == squad_index)
            .map(|s| s.initial_objective_index)
            .ok_or("objective_squad_not_configured")
    }
    /// Preserve the native per-squad objective vector's order and repetitions.
    /// Legacy catalogs without classification contribute no completion awards.
    pub(crate) fn end_reward_objectives(
        &self,
        squad_index: u16,
    ) -> Result<Vec<(u16, u8)>, &'static str> {
        let squad = self
            .squads
            .iter()
            .find(|s| s.squad_index == squad_index)
            .ok_or("objective_squad_not_configured")?;
        Ok(squad
            .authored_objectives
            .iter()
            .filter_map(|authored| {
                self.objects
                    .iter()
                    .find(|object| object.object_index == authored.object_index)
                    .and_then(|object| object.reward.as_ref())
                    .map(|rule| (authored.object_index, rule.category))
            })
            .collect())
    }
    pub(crate) fn dynamic_assignment(
        &self,
        squad_index: u16,
        state: impl Fn(u16) -> Option<replication::ObjectiveState>,
    ) -> Result<u16, &'static str> {
        use replication::ObjectiveState::{Completed, Inactive, Locked};
        let squad = self
            .squads
            .iter()
            .find(|s| s.squad_index == squad_index)
            .ok_or("objective_squad_not_configured")?;
        Ok(squad
            .authored_objectives
            .iter()
            .find(|objective| {
                state(objective.object_index)
                    .is_some_and(|value| !matches!(value, Inactive | Completed | Locked))
            })
            .map_or(u16::MAX, |objective| objective.object_index))
    }
    pub fn snapshot(
        &self,
        sequence: u32,
        timestamp_ms: u32,
    ) -> replication::ObjectiveStateSnapshot {
        replication::ObjectiveStateSnapshot {
            sequence,
            timestamp_ms,
            objectives: self
                .objects
                .iter()
                .map(|object| replication::ObjectiveStateDelta {
                    object_index: object.object_index,
                    state_actor: replication::ObjectiveStateActor {
                        state: native_state(object.initial_state)
                            .expect("validated native objective state"),
                        change_time_ms: timestamp_ms,
                        actor_index: u16::MAX,
                    },
                })
                .collect(),
        }
    }
}
pub(crate) fn native_state(state: u8) -> Option<replication::ObjectiveState> {
    use replication::ObjectiveState::*;
    Some(match state {
        0 => Inactive,
        1 => Deactivated,
        2 => Activating,
        3 => Activated,
        4 => Deactivating,
        5 => Repairing,
        6 => Repaired,
        7 => Completed,
        8 => Failed,
        9 => Locked,
        10 => Retaking,
        11 => Retaken,
        12 => CancelRetaken,
        _ => return None,
    })
}

/// Objective-enabled matches require explicit operator-supplied bindings for every
/// wire profile. A missing catalog never selects a built-in map or objective ID.
pub(crate) fn validate_configuration(
    catalog: Option<&ObjectiveCatalog>,
    map: &str,
    policy: spawn_catalog::MatchProfile,
    profile: wire_profile::WireProfile,
) -> Result<(), &'static str> {
    if let Some(catalog) = catalog {
        catalog.validate_match(map, policy)?;
        catalog.validate_profile(profile)?;
    } else if policy.has_objectives {
        return Err("retail_objective_catalog_required");
    }
    Ok(())
}

#[cfg(test)]
mod configuration_tests {
    use super::*;

    #[test]
    fn synthetic_catalog_drives_initial_states_and_assignments() {
        // Schema-authored values; placeholder hashes are not provenance claims.
        let squads: Vec<_> = (0..2).flat_map(|team| (0..4).map(move |slot| {
            serde_json::json!({
                "team": team, "squad_index": team * 16 + slot,
                "initial_objective_index": 117,
                "authored_objectives": [{"object_index": 61, "order": 1}, {"object_index": 117, "order": 2}]
            })
        })).collect();
        let input = serde_json::json!({
            "schema_version": 1, "format": "mag_retail_objective_catalog",
            "application_id": 20231, "protocol": protocol::ProtocolInfo::RETAIL_PROTOCOL,
            "game_type": 0, "map_name": "synthetic_arena", "entity_family": 12, "network_type": 12,
            "source": {
                "authored_catalog_sha256": "0".repeat(64), "binding_capture_sha256": "1".repeat(64),
                "startup_proof_sha256": "2".repeat(64), "source_image_sha256": "3".repeat(64),
                "capture_qualification": "bounded_concurrent_token_slot_capture"
            },
            "objects": [
                {"token": "0x01234567", "object_index": 61, "initial_state": 9},
                {"token": "0x76543210", "object_index": 117, "initial_state": 1}
            ],
            "squads": squads
        });
        let catalog = ObjectiveCatalog::from_json(&input.to_string()).unwrap();
        let policy = spawn_catalog::MatchProfile {
            game_type: 0,
            has_objectives: true,
            ..Default::default()
        };
        let profile = wire_profile::WireProfileCatalog::embedded()
            .profile(20231)
            .unwrap();
        assert_eq!(
            validate_configuration(Some(&catalog), "synthetic_arena", policy, profile),
            Ok(())
        );
        let snapshot = catalog.snapshot(37, 900);
        assert_eq!(snapshot.objectives.len(), 2);
        for (objective, (index, state)) in snapshot.objectives.iter().zip([
            (61, replication::ObjectiveState::Locked),
            (117, replication::ObjectiveState::Deactivated),
        ]) {
            assert_eq!(objective.object_index, index);
            assert_eq!(objective.state_actor.state, state);
            assert_eq!(objective.state_actor.change_time_ms, 900);
        }
        assert_eq!(catalog.initial_assignment(0), Ok(117));
        assert_eq!(catalog.initial_assignment(19), Ok(117));
        assert!(snapshot.encode_with_profile(profile).is_ok());
    }

    #[test]
    fn missing_catalog_denies_objective_matches_for_both_profiles() {
        let profiles = wire_profile::WireProfileCatalog::embedded();
        let policy = spawn_catalog::MatchProfile {
            has_objectives: true,
            ..Default::default()
        };
        for id in [20230, 20231] {
            assert_eq!(
                validate_configuration(
                    None,
                    "synthetic_arena",
                    policy,
                    profiles.profile(id).unwrap()
                ),
                Err("retail_objective_catalog_required")
            );
        }
    }

    #[test]
    fn missing_catalog_allows_matches_without_objectives() {
        let profiles = wire_profile::WireProfileCatalog::embedded();
        let policy = spawn_catalog::MatchProfile {
            has_objectives: false,
            ..Default::default()
        };
        for id in [20230, 20231] {
            assert_eq!(
                validate_configuration(
                    None,
                    "synthetic_arena",
                    policy,
                    profiles.profile(id).unwrap()
                ),
                Ok(())
            );
        }
    }
}
