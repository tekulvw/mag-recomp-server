//! Source-bound full squad membership for retail Domination forward spawns.
//! Initial UI availability is a filtered subset and cannot replace native
//! HasSpawnPointByID membership or the category-zero fallback.
use crate::{
    domination_fsp::{
        self as fsp, AuthoredLink, Candidate, CommittedEntry, FspType, ResolvedSpawn,
        SpawnIdentity, SquadMembership, SquadRouting, Team,
    },
    objective_catalog::ObjectiveCatalog,
    replication::ObjectiveState,
    spawn_catalog::SpawnCatalog,
};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    format: String,
    metadata_sha256: String,
    spawn_catalog_sha256: String,
    map_name: String,
    points: Vec<Point>,
    links: Vec<Link>,
    squads: Vec<Squad>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Point {
    token: u32,
    index: u16,
    team: u8,
    user_class: u8,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Link {
    objective_token: u32,
    spawn_token: u32,
    fsp_type: u8,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Squad {
    index: u8,
    points: Vec<u32>,
    category_zero: Option<u32>,
}
#[derive(Clone)]
struct OwnedSquad {
    points: Vec<SpawnIdentity>,
    category_zero: Option<SpawnIdentity>,
}
#[derive(Clone)]
pub struct Bindings {
    links: BTreeMap<u32, (AuthoredLink, ResolvedSpawn)>,
    squads: [OwnedSquad; 32],
    objectives: BTreeMap<u32, u16>,
}
fn team(value: u8) -> Result<Team, &'static str> {
    match value {
        0 => Ok(Team::Zero),
        1 => Ok(Team::One),
        _ => Err("invalid_domination_fsp_team"),
    }
}
impl Bindings {
    /// The catalog hash binds remapped slots to their exact source profile.
    /// Full membership and category order come from the metadata projector.
    pub fn from_json(
        raw: &[u8],
        spawns: &SpawnCatalog,
        objectives: &ObjectiveCatalog,
    ) -> Result<Self, &'static str> {
        if raw.len() > 262_144 {
            return Err("domination_fsp_bindings_too_large");
        }
        let file: File =
            serde_json::from_slice(raw).map_err(|_| "invalid_domination_fsp_bindings")?;
        let pool = spawns.map_pool();
        if file.format != "mag_domination_fsp_bindings_v1"
            || file.metadata_sha256 != objectives.source().authored_catalog_sha256
            || Some(file.spawn_catalog_sha256.as_str()) != spawns.source_sha256()
            || file.map_name != spawns.map_name()
            || !pool.runtime_indices_verified
            || spawns.match_profile().game_type != 2
            || file.points.len() != pool.locations.len()
            || file.squads.len() != 32
            || pool.squads.len() != 32
            || file.links.len() > 128
        {
            return Err("domination_fsp_source_mismatch");
        }
        objectives.validate_match(spawns.map_name(), spawns.match_profile())?;
        let mut points = BTreeMap::new();
        for point in file.points {
            if point.token == 0
                || point.index >= 128
                || point.user_class > 1
                || spawns
                    .point(point.index)
                    .is_none_or(|p| p.token != point.token)
            {
                return Err("domination_fsp_point_mismatch");
            }
            let resolved = ResolvedSpawn {
                identity: SpawnIdentity {
                    token: point.token,
                    native_id: 0x2380_0000 | u32::from(point.index),
                },
                team: team(point.team)?,
                squad_routing: if point.user_class == 0 {
                    SquadRouting::Enabled
                } else {
                    SquadRouting::Disabled
                },
            };
            if points.insert(point.token, resolved).is_some() {
                return Err("duplicate_domination_fsp_point");
            }
        }
        let mut squads: BTreeMap<u8, OwnedSquad> = BTreeMap::new();
        for squad in file.squads {
            if squad.index >= 32 || squad.points.is_empty() || squad.points.len() > 32 {
                return Err("invalid_domination_fsp_squad");
            }
            let source = pool
                .squads
                .iter()
                .find(|s| s.index == u16::from(squad.index))
                .ok_or("domination_fsp_squad_mismatch")?;
            let members = squad
                .points
                .iter()
                .map(|token| {
                    let point = points
                        .get(token)
                        .ok_or("domination_fsp_membership_mismatch")?;
                    if point.team as u8 != source.team {
                        return Err("domination_fsp_membership_team_mismatch");
                    }
                    Ok(point.identity)
                })
                .collect::<Result<Vec<_>, _>>()?;
            let category_zero = squad
                .category_zero
                .map(|token| {
                    members
                        .iter()
                        .copied()
                        .find(|p| p.token == token)
                        .ok_or("domination_fsp_fallback_mismatch")
                })
                .transpose()?;
            if source.team != squad.index / 16
                || squads
                    .insert(
                        squad.index,
                        OwnedSquad {
                            points: members,
                            category_zero,
                        },
                    )
                    .is_some()
            {
                return Err("duplicate_domination_fsp_squad");
            }
        }
        let mut links = BTreeMap::new();
        for link in file.links {
            if objectives.binding(link.objective_token).is_none() {
                return Err("domination_fsp_objective_mismatch");
            }
            let point = *points
                .get(&link.spawn_token)
                .ok_or("domination_fsp_link_mismatch")?;
            let fsp_type = match link.fsp_type {
                0 => FspType::ActiveWhenIncomplete,
                1 => FspType::ActiveWhenComplete,
                _ => return Err("invalid_domination_fsp_type"),
            };
            let authored = AuthoredLink {
                objective_token: link.objective_token,
                spawn_token: link.spawn_token,
                team: point.team,
                fsp_type,
            };
            if links
                .insert(link.objective_token, (authored, point))
                .is_some()
            {
                return Err("duplicate_domination_fsp_link");
            }
        }
        let squads = std::array::from_fn(|i| squads.remove(&(i as u8)).unwrap());
        let objectives = objectives.bound_objects().collect();
        Ok(Self {
            links,
            squads,
            objectives,
        })
    }
    /// Each objective linked to a forward spawn, by catalog index: the
    /// spawn point index, the point's team, whether the point is open while the objective is not
    /// completed, and whether squads route their lives to it (an infantry
    /// spawn rather than a vehicle source).
    pub(crate) fn linked_spawns(
        &self,
    ) -> impl Iterator<Item = (u16, u16, Team, FspType, SquadRouting)> + '_ {
        self.links.iter().filter_map(|(token, (link, spawn))| {
            Some((
                *self.objectives.get(token)?,
                spawn.identity.native_id as u16,
                spawn.team,
                link.fsp_type,
                spawn.squad_routing,
            ))
        })
    }
    pub(crate) fn category_zero_indices(&self) -> Result<[u16; 32], &'static str> {
        let mut result = [0; 32];
        for (index, squad) in self.squads.iter().enumerate() {
            result[index] = squad
                .category_zero
                .ok_or("domination_fsp_category_zero_missing")?
                .native_id as u16;
        }
        Ok(result)
    }

    pub fn plan_entry(
        &self,
        objective_token: u32,
        state: ObjectiveState,
        is_completed: bool,
    ) -> Result<Candidate, &'static str> {
        if !self.objectives.contains_key(&objective_token) {
            return Err("unknown_domination_fsp_objective");
        }
        let (authored, resolved) = self
            .links
            .get(&objective_token)
            .map_or((None, None), |(a, r)| (Some(*a), Some(*r)));
        let squads = std::array::from_fn(|i| {
            Some(SquadMembership {
                points: &self.squads[i].points,
                category_zero: self.squads[i].category_zero,
            })
        });
        fsp::plan_entry(
            CommittedEntry {
                objective_token,
                state,
                is_completed,
            },
            authored,
            resolved,
            &squads,
        )
        .map_err(|_| "domination_fsp_entry_unresolved")
    }
}
#[cfg(test)]
#[path = "domination_fsp_bindings_tests.rs"]
mod tests;
