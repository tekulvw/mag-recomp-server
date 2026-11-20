//! Ordered retail Domination forward-spawn operations for committed objective entry.
use crate::replication::ObjectiveState;

pub const SQUADS_PER_TEAM: usize = 16;
pub const SQUAD_SLOTS: usize = 2 * SQUADS_PER_TEAM;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Team {
    Zero = 0,
    One = 1,
}

/// Original authored FSPType. This is independent of the point's Active field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum FspType {
    ActiveWhenIncomplete = 0,
    ActiveWhenComplete = 1,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuthoredLink {
    pub objective_token: u32,
    pub spawn_token: u32,
    pub team: Team,
    pub fsp_type: FspType,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommittedEntry {
    pub objective_token: u32,
    pub state: ObjectiveState,
    /// Result of the current objective's GetIsCompleted virtual predicate.
    pub is_completed: bool,
}

/// The engine joins the authored token to the manager's current packed ID.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpawnIdentity {
    pub token: u32,
    pub native_id: u32,
}

impl SpawnIdentity {
    fn valid(self) -> bool {
        self.token != 0
            && self.native_id & 0xffff_0000 == 0x2380_0000
            && (self.native_id as u16) < 128
    }
}

/// Evidence of the resolved runtime point's native +0x4c routing branch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SquadRouting {
    Enabled,
    Disabled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResolvedSpawn {
    pub identity: SpawnIdentity,
    pub team: Team,
    pub squad_routing: SquadRouting,
}

/// An occupied native squad slot. The engine resolves its actual point list
/// and GetSpawnPointByCategory(0) result from the same staged world.
#[derive(Clone, Copy, Debug)]
pub struct SquadMembership<'a> {
    pub points: &'a [SpawnIdentity],
    pub category_zero: Option<SpawnIdentity>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VehicleHook {
    OnFspActivate,
    OnFspDeactivate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Invoke the point/manager active operation at this position in the sequence.
    SetPointActive { point: SpawnIdentity, active: bool },
    SetSquadSpawn {
        squad_index: u8,
        point: SpawnIdentity,
    },
    /// The engine must resolve the current linked vehicle and invoke its hook
    /// when present. The resolved absence of a vehicle has no vehicle effect.
    ResolveVehicleHook {
        point: SpawnIdentity,
        objective_token: u32,
        hook: VehicleHook,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanError {
    InvalidObjective,
    UnsupportedEntryState,
    UnresolvedSpawnBinding,
    SpawnBindingMismatch,
    SquadMembershipMismatch { squad_index: u8 },
    MissingCategoryZero { squad_index: u8 },
}

#[must_use = "the caller must stage and consume the complete ordered FSP candidate"]
#[derive(Debug, PartialEq, Eq)]
pub struct Candidate {
    effects: Vec<Effect>,
}

impl Candidate {
    pub fn effects(&self) -> &[Effect] {
        &self.effects
    }

    pub fn into_effects(self) -> Vec<Effect> {
        self.effects
    }
}

/// Stage the common native FSP portion only. The caller selects a committed
/// class entry and later applies the whole sequence to its candidate world.
/// Each slot is an observed squad membership or a confirmed vacant slot.
pub fn plan_entry(
    entry: CommittedEntry,
    authored: Option<AuthoredLink>,
    resolved: Option<ResolvedSpawn>,
    squads: &[Option<SquadMembership<'_>>; SQUAD_SLOTS],
) -> Result<Candidate, PlanError> {
    if entry.objective_token == 0 {
        return Err(PlanError::InvalidObjective);
    }
    let completed_entry = match entry.state {
        ObjectiveState::Completed => true,
        ObjectiveState::Deactivated | ObjectiveState::Repaired => false,
        _ => return Err(PlanError::UnsupportedEntryState),
    };
    let (link, spawn) = match (authored, resolved) {
        (None, None) => {
            return Ok(Candidate {
                effects: Vec::new(),
            });
        }
        (Some(_), None) => return Err(PlanError::UnresolvedSpawnBinding),
        (None, Some(_)) => return Err(PlanError::SpawnBindingMismatch),
        (Some(link), Some(spawn)) => (link, spawn),
    };
    let point = spawn.identity;
    if !point.valid()
        || entry.objective_token != link.objective_token
        || point.token != link.spawn_token
        || spawn.team != link.team
    {
        return Err(PlanError::SpawnBindingMismatch);
    }

    let normally_active_when_complete = link.fsp_type == FspType::ActiveWhenComplete;
    let routing_active = completed_entry == normally_active_when_complete;
    let mut effects = vec![Effect::SetPointActive {
        point,
        active: routing_active,
    }];
    if spawn.squad_routing == SquadRouting::Enabled {
        let first = spawn.team as usize * SQUADS_PER_TEAM;
        for (slot, membership) in squads.iter().enumerate().skip(first).take(SQUADS_PER_TEAM) {
            let Some(membership) = membership else {
                continue;
            };
            let squad_index = slot as u8;
            if membership.points.iter().any(|candidate| {
                *candidate != point
                    && (candidate.native_id == point.native_id || candidate.token == point.token)
            }) {
                return Err(PlanError::SquadMembershipMismatch { squad_index });
            }
            if !membership.points.contains(&point) {
                continue;
            }
            let selected = if routing_active {
                point
            } else {
                let fallback = membership
                    .category_zero
                    .ok_or(PlanError::MissingCategoryZero { squad_index })?;
                if !fallback.valid() || !membership.points.contains(&fallback) {
                    return Err(PlanError::SquadMembershipMismatch { squad_index });
                }
                fallback
            };
            effects.push(Effect::SetSquadSpawn {
                squad_index,
                point: selected,
            });
        }
    }

    let active = entry.is_completed == normally_active_when_complete;
    effects.push(Effect::SetPointActive { point, active });
    let hook = if active {
        VehicleHook::OnFspActivate
    } else {
        VehicleHook::OnFspDeactivate
    };
    effects.push(Effect::ResolveVehicleHook {
        point,
        objective_token: entry.objective_token,
        hook,
    });
    Ok(Candidate { effects })
}

/// Required vehicle work retained after the point and squad portion is staged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VehicleRequest {
    pub point: SpawnIdentity,
    pub objective_token: u32,
    pub hook: VehicleHook,
}

#[must_use = "vehicle hooks must be prepared before installing the staged spawn catalog"]
pub struct SpawnStage {
    catalog: crate::spawn_catalog::SpawnCatalog,
    vehicle_requests: Vec<VehicleRequest>,
}

impl SpawnStage {
    /// The hook consumer prepares owned vehicle state. Installation remains with
    /// the caller after recording the complete objective/world transaction.
    pub fn complete<V>(
        self,
        prepare_vehicles: impl FnOnce(&[VehicleRequest]) -> Result<V, &'static str>,
    ) -> Result<(crate::spawn_catalog::SpawnCatalog, V), &'static str> {
        let vehicles = prepare_vehicles(&self.vehicle_requests)?;
        Ok((self.catalog, vehicles))
    }
}

fn catalog_point(
    catalog: &crate::spawn_catalog::SpawnCatalog,
    identity: SpawnIdentity,
) -> Result<u16, &'static str> {
    let index = identity.native_id as u16;
    if !identity.valid()
        || catalog
            .point(index)
            .is_none_or(|p| p.token != identity.token)
    {
        return Err("domination_fsp_catalog_binding_mismatch");
    }
    Ok(index)
}

/// Apply the point/squad operations to an isolated catalog and retain every
/// vehicle hook for a required consumer. An error exposes no partial catalog.
pub fn stage_spawn_effects(
    catalog: &crate::spawn_catalog::SpawnCatalog,
    candidate: &Candidate,
) -> Result<SpawnStage, &'static str> {
    if !catalog.map_pool().runtime_indices_verified {
        return Err("domination_fsp_requires_bound_spawn_indices");
    }
    let mut next = catalog.clone();
    let mut vehicle_requests = Vec::new();
    for effect in candidate.effects() {
        match *effect {
            Effect::SetPointActive { point, active } => {
                let index = catalog_point(&next, point)?;
                next.set_runtime_point_active(index, point.token, active)?;
            }
            Effect::SetSquadSpawn { squad_index, point } => {
                let index = catalog_point(&next, point)?;
                next.set_runtime_squad_default(squad_index.into(), index)?;
            }
            Effect::ResolveVehicleHook {
                point,
                objective_token,
                hook,
            } => {
                catalog_point(&next, point)?;
                vehicle_requests.push(VehicleRequest {
                    point,
                    objective_token,
                    hook,
                });
            }
        }
    }
    Ok(SpawnStage {
        catalog: next,
        vehicle_requests,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::replication::ObjectiveState;

    const OBJECTIVE: u32 = 0xee00;
    const POINT: SpawnIdentity = SpawnIdentity {
        token: 0xaa00,
        native_id: 0x2380_0004,
    };
    const BASE: SpawnIdentity = SpawnIdentity {
        token: 0xbb00,
        native_id: 0x2380_0014,
    };
    const ROSTER: &[SpawnIdentity] = &[POINT, BASE];

    fn link(fsp_type: FspType) -> AuthoredLink {
        AuthoredLink {
            objective_token: OBJECTIVE,
            spawn_token: POINT.token,
            team: Team::One,
            fsp_type,
        }
    }

    fn resolved(routing: SquadRouting) -> ResolvedSpawn {
        ResolvedSpawn {
            identity: POINT,
            team: Team::One,
            squad_routing: routing,
        }
    }

    fn committed(state: ObjectiveState, completed: bool) -> CommittedEntry {
        CommittedEntry {
            objective_token: OBJECTIVE,
            state,
            is_completed: completed,
        }
    }

    fn squad(fallback: Option<SpawnIdentity>) -> SquadMembership<'static> {
        SquadMembership {
            points: ROSTER,
            category_zero: fallback,
        }
    }

    #[test]
    fn completed_structural_point_falls_back_only_on_own_team_in_native_order() {
        let mut squads = [None; 32];
        squads[1] = Some(squad(None));
        squads[16] = Some(squad(Some(BASE)));
        squads[31] = Some(squad(Some(BASE)));
        let candidate = plan_entry(
            committed(ObjectiveState::Completed, true),
            Some(link(FspType::ActiveWhenIncomplete)),
            Some(resolved(SquadRouting::Enabled)),
            &squads,
        )
        .unwrap();
        assert_eq!(
            candidate.effects(),
            &[
                Effect::SetPointActive {
                    point: POINT,
                    active: false
                },
                Effect::SetSquadSpawn {
                    squad_index: 16,
                    point: BASE
                },
                Effect::SetSquadSpawn {
                    squad_index: 31,
                    point: BASE
                },
                Effect::SetPointActive {
                    point: POINT,
                    active: false
                },
                Effect::ResolveVehicleHook {
                    point: POINT,
                    objective_token: OBJECTIVE,
                    hook: VehicleHook::OnFspDeactivate
                },
            ]
        );
    }

    #[test]
    fn repaired_routes_by_entry_then_rechecks_current_completed_predicate() {
        let mut squads = [None; 32];
        squads[19] = Some(squad(None));
        let candidate = plan_entry(
            committed(ObjectiveState::Repaired, true),
            Some(link(FspType::ActiveWhenIncomplete)),
            Some(resolved(SquadRouting::Enabled)),
            &squads,
        )
        .unwrap();
        assert_eq!(
            candidate.effects(),
            &[
                Effect::SetPointActive {
                    point: POINT,
                    active: true
                },
                Effect::SetSquadSpawn {
                    squad_index: 19,
                    point: POINT
                },
                Effect::SetPointActive {
                    point: POINT,
                    active: false
                },
                Effect::ResolveVehicleHook {
                    point: POINT,
                    objective_token: OBJECTIVE,
                    hook: VehicleHook::OnFspDeactivate
                },
            ]
        );
    }

    #[test]
    fn disabled_squad_routing_still_stages_exact_setactive_calls_and_vehicle_hook() {
        let mut squads = [None; 32];
        squads[16] = Some(squad(None));
        let candidate = plan_entry(
            committed(ObjectiveState::Deactivated, false),
            Some(link(FspType::ActiveWhenComplete)),
            Some(resolved(SquadRouting::Disabled)),
            &squads,
        )
        .unwrap();
        assert_eq!(
            candidate.effects(),
            &[
                Effect::SetPointActive {
                    point: POINT,
                    active: false
                },
                Effect::SetPointActive {
                    point: POINT,
                    active: false
                },
                Effect::ResolveVehicleHook {
                    point: POINT,
                    objective_token: OBJECTIVE,
                    hook: VehicleHook::OnFspDeactivate
                },
            ]
        );
    }

    #[test]
    fn late_unresolved_fallback_cannot_publish_earlier_candidate_operations() {
        let mut squads = [None; 32];
        squads[16] = Some(squad(Some(BASE)));
        squads[31] = Some(squad(None));
        let result = plan_entry(
            committed(ObjectiveState::Completed, true),
            Some(link(FspType::ActiveWhenIncomplete)),
            Some(resolved(SquadRouting::Enabled)),
            &squads,
        );
        assert_eq!(
            result,
            Err(PlanError::MissingCategoryZero { squad_index: 31 })
        );
        assert_eq!(squads[16].unwrap().category_zero, Some(BASE));
        assert_eq!(squads[31].unwrap().category_zero, None);

        // A subsequent fully resolved attempt starts with exactly one first
        // SetActive and one selection per eligible squad.
        squads[31] = Some(squad(Some(BASE)));
        let result = plan_entry(
            committed(ObjectiveState::Completed, true),
            Some(link(FspType::ActiveWhenIncomplete)),
            Some(resolved(SquadRouting::Enabled)),
            &squads,
        )
        .unwrap();
        assert_eq!(result.effects().len(), 5);
    }

    #[test]
    fn rejects_unresolved_and_rebound_points() {
        let squads = [None; 32];
        let entry = committed(ObjectiveState::Completed, true);
        assert_eq!(
            plan_entry(
                entry,
                Some(link(FspType::ActiveWhenComplete)),
                None,
                &squads
            ),
            Err(PlanError::UnresolvedSpawnBinding)
        );
        for native_id in [0x0880_0004, 0x2380_0080] {
            let mut wrong = resolved(SquadRouting::Enabled);
            wrong.identity.native_id = native_id;
            assert_eq!(
                plan_entry(
                    entry,
                    Some(link(FspType::ActiveWhenComplete)),
                    Some(wrong),
                    &squads
                ),
                Err(PlanError::SpawnBindingMismatch)
            );
        }
        let mut wrong = resolved(SquadRouting::Enabled);
        wrong.identity.token = BASE.token;
        assert_eq!(
            plan_entry(
                entry,
                Some(link(FspType::ActiveWhenComplete)),
                Some(wrong),
                &squads
            ),
            Err(PlanError::SpawnBindingMismatch)
        );
        assert_eq!(
            plan_entry(entry, None, Some(resolved(SquadRouting::Enabled)), &squads),
            Err(PlanError::SpawnBindingMismatch)
        );
        assert!(
            plan_entry(entry, None, None, &squads)
                .unwrap()
                .effects()
                .is_empty()
        );
    }

    #[test]
    fn rejects_rebound_squad_membership_and_unqualified_fallback() {
        let mut squads = [None; 32];
        let entry = committed(ObjectiveState::Completed, true);
        let bad_membership = [SpawnIdentity {
            token: POINT.token,
            native_id: BASE.native_id,
        }];
        squads[16] = Some(SquadMembership {
            points: &bad_membership,
            category_zero: None,
        });
        assert_eq!(
            plan_entry(
                entry,
                Some(link(FspType::ActiveWhenIncomplete)),
                Some(resolved(SquadRouting::Enabled)),
                &squads
            ),
            Err(PlanError::SquadMembershipMismatch { squad_index: 16 })
        );
        squads[16] = Some(squad(Some(SpawnIdentity {
            token: 3,
            native_id: 99,
        })));
        assert_eq!(
            plan_entry(
                entry,
                Some(link(FspType::ActiveWhenIncomplete)),
                Some(resolved(SquadRouting::Enabled)),
                &squads
            ),
            Err(PlanError::SquadMembershipMismatch { squad_index: 16 })
        );
    }
}

#[cfg(test)]
#[path = "domination_fsp_backend_tests.rs"]
mod backend_tests;
