//! Synthetic indexed spawn catalog testing the assembled FSP staging boundary.
use super::*;
use crate::{
    replication::{AlphaSpawnSetup, BitReader, ObjectUpdate, ObjectiveState, SpawnAvailability},
    spawn_catalog::SpawnCatalog,
    wire_profile::{WireProfile, WireProfileCatalog},
};
use serde_json::json;

const OBJECTIVE: u32 = 0x1234;
const BASE: SpawnIdentity = SpawnIdentity {
    native_id: 0x2380_0000,
    token: 0x9870,
};
const POINT: SpawnIdentity = SpawnIdentity {
    native_id: 0x2380_0001,
    token: 0x9871,
};
const ROSTER: &[SpawnIdentity] = &[BASE, POINT];

fn catalog(verified: bool) -> SpawnCatalog {
    catalog_with_point(verified, true)
}

fn catalog_with_point(verified: bool, active: bool) -> SpawnCatalog {
    let available: &[u16] = if active { &[0, 1] } else { &[0] };
    let default = u16::from(active);
    let mut point = json!({"index":1,"token":POINT.token,"kind":"on_foot","active":active,
        "position":{"x":20.,"y":2.,"z":1.}});
    if !active {
        point["kind"] = json!("parachute");
        point["parachute"] = json!({"forward":[1.,0.,0.],"spacing_m":1.,"yaw":0});
    }
    let squads = (0..32)
        .map(|slot| {
            json!({
                "index":slot,"team":slot/16,"platoon":slot%16/4,"squad":slot%4,
                "faction":if slot < 16 { None } else { Some(0) },
                "available":available,"default_spawn":if slot < 16 { 0 } else { default },
            })
        })
        .collect::<Vec<_>>();
    SpawnCatalog::from_json(&json!({
        "format":"mag_spawn_catalog","schema_version":1,"source":"SYNTHETIC FSP staging map",
        "active":{"map_name":"synthetic_fsp","squad_index":16,"default_spawn":default,"available":available},
        "match_profile":{"game_type":2,"auto_assign_teams":true,"team_capacity":16,"squad_capacity":1,
            "has_bunkers":false,"has_objectives":true,"attacking_faction":1},
        "maps":[{"name":"synthetic_fsp","owner_faction":0,"spawn_count":3,"squads":squads,
            "runtime_indices_verified":verified,"runtime_indices_basis":"SYNTHETIC fixed family-35 slots",
            "locations":[
                {"index":0,"token":BASE.token,"kind":"insertion","position":{"x":10.,"y":2.,"z":1.}},
                point,
                {"index":2,"token":0x9872,"kind":"unsupported","position":{"x":30.,"y":2.,"z":1.}},
            ]
        }],
    }).to_string()).unwrap()
}

fn membership() -> [Option<SquadMembership<'static>>; SQUAD_SLOTS] {
    [Some(SquadMembership {
        points: ROSTER,
        category_zero: Some(BASE),
    }); SQUAD_SLOTS]
}

fn plan(
    state: ObjectiveState,
    is_completed: bool,
    squads: &[Option<SquadMembership<'_>>; SQUAD_SLOTS],
) -> Candidate {
    plan_entry(
        CommittedEntry {
            objective_token: OBJECTIVE,
            state,
            is_completed,
        },
        Some(AuthoredLink {
            objective_token: OBJECTIVE,
            spawn_token: POINT.token,
            team: Team::One,
            fsp_type: FspType::ActiveWhenIncomplete,
        }),
        Some(ResolvedSpawn {
            identity: POINT,
            team: Team::One,
            squad_routing: SquadRouting::Enabled,
        }),
        squads,
    )
    .unwrap()
}

fn complete(catalog: &SpawnCatalog, candidate: &Candidate, hook: VehicleHook) -> SpawnCatalog {
    let requests = [VehicleRequest {
        point: POINT,
        objective_token: OBJECTIVE,
        hook,
    }];
    let (next, prepared) = stage_spawn_effects(catalog, candidate)
        .unwrap()
        .complete(|commands| {
            assert_eq!(commands, &requests);
            Ok(commands.to_vec())
        })
        .unwrap();
    assert_eq!(prepared, requests);
    next
}

fn initial_or_delta_flags(
    packet: &[u8],
    profile: WireProfile,
    late_join_player: Option<u16>,
) -> Vec<u32> {
    let header = ObjectUpdate::decode_with_profile(packet, profile)
        .unwrap()
        .header;
    let mut bits = BitReader::with_profile(packet, profile);
    bits.position = profile.header_bits();
    if let Some(player) = late_join_player {
        assert_eq!(header.object_type_mask, (1 << 3) | (1 << 7) | (1 << 8));
        assert_eq!(bits.read(1), Ok(1));
        assert_eq!(bits.read(16), Ok(u32::from(player)));
        for field in 0..22 {
            assert_eq!(bits.read(1), Ok(u32::from(field == 14)));
            if field == 14 {
                bits.read(7).unwrap();
            }
        }
        assert_eq!(bits.read(1), Ok(0));
    } else {
        assert_eq!(header.object_type_mask, 1 << 7);
    }
    assert_eq!(bits.read(1), Ok(1));
    assert_eq!(bits.read(16), Ok(0));
    assert_eq!(bits.read(2), Ok(0));
    assert_eq!(bits.read(1), Ok(1));
    assert_eq!(bits.read(8), Ok(3));
    assert_eq!(bits.read(1), Ok(1));
    let flags = (0..128).map(|_| bits.read(1).unwrap()).collect();
    assert_eq!(bits.read(7), Ok(0));
    flags
}

fn assert_wire_availability(catalog: &SpawnCatalog, base: u32, fsp: u32) {
    let profiles = WireProfileCatalog::embedded();
    let profile = profiles.profile(20231).unwrap();
    let delta = SpawnAvailability::encode_update_with_profile(3, 200, catalog, profile).unwrap();
    let late_join = catalog.for_squad(31).unwrap();
    let setup = AlphaSpawnSetup {
        sequence: 3,
        timestamp_ms: 200,
    }
    .encode_with_bunkers_with_profile(9, None, &late_join, &[], profile)
    .unwrap();
    for flags in [
        initial_or_delta_flags(&delta, profile, None),
        initial_or_delta_flags(&setup, profile, Some(9)),
    ] {
        assert_eq!(&flags[..3], &[base, fsp, 0]);
        assert!(flags[3..].iter().all(|&active| active == 0));
    }
}

#[test]
fn stages_completed_deactivated_and_repaired_with_wire_availability_and_reset() {
    let original = catalog(true);
    let squads = membership();
    assert_wire_availability(&original, 1, 1);
    let knocked_out = complete(
        &original,
        &plan(ObjectiveState::Completed, true, &squads),
        VehicleHook::OnFspDeactivate,
    );
    for slot in 0..32 {
        assert_eq!(knocked_out.default_for_squad(slot), Some(0));
        assert_eq!(
            original.default_for_squad(slot),
            Some(u16::from(slot >= 16))
        );
    }
    assert_eq!(knocked_out.default_spawn(), 0);
    assert!(!knocked_out.point_active(1));
    assert_wire_availability(&knocked_out, 1, 0);

    for restore in [ObjectiveState::Deactivated, ObjectiveState::Repaired] {
        let restored = complete(
            &knocked_out,
            &plan(restore, false, &squads),
            VehicleHook::OnFspActivate,
        );
        assert_eq!(restored.default_spawn(), 1);
        for slot in 0..32 {
            assert_eq!(
                restored.default_for_squad(slot),
                Some(u16::from(slot >= 16))
            );
        }
        let late_join = restored.for_squad(31).unwrap();
        assert_eq!(late_join.default_spawn(), 1);
        assert!(late_join.alpha(1).is_some());
        assert_wire_availability(&restored, 1, 1);
    }
    let mut reset = knocked_out.for_squad(31).unwrap();
    reset.reset_runtime();
    assert_eq!(reset.squad_index(), 31);
    assert_eq!(reset.default_spawn(), 1);
    assert!(reset.alpha(1).is_some());
    assert_wire_availability(&reset, 1, 1);
}

#[test]
fn native_fallback_accepts_an_inactive_category_zero_but_selection_rejects_it() {
    let mut original = catalog(true);
    original
        .set_runtime_point_active(0, BASE.token, false)
        .unwrap();
    let next = complete(
        &original,
        &plan(ObjectiveState::Completed, true, &membership()),
        VehicleHook::OnFspDeactivate,
    );
    let late_join = next.for_squad(31).unwrap();
    assert_eq!(late_join.default_spawn(), 0);
    assert!(late_join.alpha(0).is_none());
    assert!(late_join.alpha(1).is_none());
    assert_wire_availability(&next, 0, 0);
}

#[test]
fn failed_late_catalog_identity_and_failed_vehicle_preparation_preserve_original() {
    let original = catalog(true);
    let wrong = SpawnIdentity {
        token: BASE.token.wrapping_add(100),
        native_id: BASE.native_id,
    };
    let wrong_roster = [POINT, wrong];
    let mut squads = membership();
    squads[31] = Some(SquadMembership {
        points: &wrong_roster,
        category_zero: Some(wrong),
    });
    let candidate = plan(ObjectiveState::Completed, true, &squads);
    assert_eq!(
        stage_spawn_effects(&original, &candidate).err(),
        Some("domination_fsp_catalog_binding_mismatch")
    );
    let candidate = plan(ObjectiveState::Completed, true, &membership());
    assert_eq!(
        stage_spawn_effects(&catalog(false), &candidate).err(),
        Some("domination_fsp_requires_bound_spawn_indices")
    );
    let result = stage_spawn_effects(&original, &candidate)
        .unwrap()
        .complete::<()>(|requests| {
            assert_eq!(requests.len(), 1);
            assert_eq!(requests[0].hook, VehicleHook::OnFspDeactivate);
            Err("test_vehicle_prepare_failed")
        });
    assert_eq!(result.err(), Some("test_vehicle_prepare_failed"));
    assert!(original.point_active(1));
    assert_eq!(original.default_for_squad(16), Some(1));
    assert_eq!(original.default_for_squad(31), Some(1));
    assert_wire_availability(&original, 1, 1);
}

#[test]
fn qualified_native_membership_adds_supported_inactive_at_load_point_to_runtime_choices() {
    let original = catalog_with_point(true, false);
    assert_wire_availability(&original, 1, 0);
    assert_eq!(original.available_for_squad(31), Some(&[0][..]));
    let activated = complete(
        &original,
        &plan(ObjectiveState::Deactivated, false, &membership()),
        VehicleHook::OnFspActivate,
    );
    for squad in 0..16 {
        assert_eq!(activated.available_for_squad(squad), Some(&[0][..]));
        assert_eq!(activated.default_for_squad(squad), Some(0));
    }
    for squad in 16..32 {
        assert_eq!(activated.available_for_squad(squad), Some(&[0, 1][..]));
        let bound = activated.for_squad(squad).unwrap();
        assert_eq!(bound.default_spawn(), 1);
        assert!(bound.alpha(1).is_some());
    }
    assert_wire_availability(&activated, 1, 1);
    let mut reset = activated;
    reset.reset_runtime();
    assert_eq!(reset.available_for_squad(31), Some(&[0][..]));
    assert_eq!(reset.default_for_squad(31), Some(0));
    assert_wire_availability(&reset, 1, 0);
}

#[test]
fn unsupported_native_member_cannot_be_promoted_to_worker_spawn_choice() {
    let original = catalog(true);
    let unsupported = SpawnIdentity {
        token: 0x9872,
        native_id: 0x2380_0002,
    };
    let roster = [BASE, unsupported];
    let squads = [Some(SquadMembership {
        points: &roster,
        category_zero: Some(BASE),
    }); SQUAD_SLOTS];
    let candidate = plan_entry(
        CommittedEntry {
            objective_token: OBJECTIVE,
            state: ObjectiveState::Deactivated,
            is_completed: false,
        },
        Some(AuthoredLink {
            objective_token: OBJECTIVE,
            spawn_token: unsupported.token,
            team: Team::One,
            fsp_type: FspType::ActiveWhenIncomplete,
        }),
        Some(ResolvedSpawn {
            identity: unsupported,
            team: Team::One,
            squad_routing: SquadRouting::Enabled,
        }),
        &squads,
    )
    .unwrap();
    assert_eq!(
        stage_spawn_effects(&original, &candidate).err(),
        Some("unsupported_runtime_squad_spawn")
    );
    assert_eq!(original.available_for_squad(31), Some(&[0, 1][..]));
    assert_wire_availability(&original, 1, 1);
}

#[test]
fn scoped_helo_keeps_native_default_with_playable_category_zero_fallback() {
    let squads = (0..32)
        .map(|index| {
            json!({
                "index":index,"team":index/16,"platoon":index%16/4,"squad":index%4,
                "faction":if index < 16 { None } else { Some(0) },
                "available":[0],"default_spawn":0,
            })
        })
        .collect::<Vec<_>>();
    let locations = (0..5).map(|index| json!({
        "index":index,"token":100+index,"kind":if index == 0 {"insertion"} else {"unsupported"},
        "active":index == 0,"position":{"x":10.*f64::from(index),"y":2.,"z":1.},
    })).collect::<Vec<_>>();
    let mut spawns = SpawnCatalog::from_json(&json!({
        "format":"mag_spawn_catalog","schema_version":1,"source":"synthetic",
        "active":{"map_name":"synthetic_fsp","squad_index":16,"default_spawn":0,"available":[0]},
        "match_profile":{"game_type":2,"auto_assign_teams":true,"team_capacity":16,"squad_capacity":1,
            "has_bunkers":false,"has_objectives":true,"attacking_faction":1},
        "maps":[{"name":"synthetic_fsp","owner_faction":0,"spawn_count":5,"squads":squads,
            "runtime_indices_verified":true,"runtime_indices_basis":"synthetic source","locations":locations}],
    }).to_string()).unwrap();

    assert_eq!(
        spawns.set_runtime_squad_default(16, 1),
        Err("unsupported_runtime_squad_spawn")
    );
    spawns
        .configure_domination_helo_scope([1, 2, 3, 4].into(), [0; 32])
        .unwrap();
    spawns.set_runtime_point_active(1, 101, true).unwrap();
    spawns.set_runtime_squad_default(16, 1).unwrap();
    assert_eq!(spawns.default_for_squad(16), Some(1));
    assert_eq!(spawns.default_spawn(), 0);
    let late_join = spawns.for_squad(16).unwrap();
    assert_eq!(late_join.default_spawn(), 0);
    assert!(late_join.alpha(0).is_some());
    assert!(late_join.alpha(1).is_none());
    spawns.reset_runtime();
    assert_eq!(spawns.default_for_squad(16), Some(0));
}
