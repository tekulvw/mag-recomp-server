//! SYNTHETIC structural field-hospital spawn scope fixtures. No captured map data.
use super::*;

const HOME: u16 = 0;
const HOSPITAL: u16 = 2;

fn spawns(hospital_kind: &str, hospital_available: bool) -> SpawnCatalog {
    let available: Vec<u16> = if hospital_available {
        vec![HOME, 1, HOSPITAL]
    } else {
        vec![HOME, 1]
    };
    SpawnCatalog::from_json(
        &serde_json::json!({
            "format":"mag_spawn_catalog","schema_version":1,"source":"SYNTHETIC field hospital",
            "active":{"map_name":"synthetic_acquisition","squad_index":16,"default_spawn":HOME,
                "available":available},
            "match_profile":{"game_type":1,"has_bunkers":false,"has_objectives":false},
            "maps":[{"name":"synthetic_acquisition","owner_faction":0,"spawn_count":3,
                "runtime_indices_verified":true,"runtime_indices_basis":"SYNTHETIC pool identity",
                "locations":[
                    {"index":0,"token":100,"kind":"insertion","position":{"x":10.,"y":0.,"z":10.}},
                    {"index":1,"token":101,"kind":"alternate","position":{"x":20.,"y":0.,"z":10.}},
                    {"index":2,"token":102,"kind":hospital_kind,"position":{"x":40.,"y":0.,"z":40.}}],
                "squads":[{"index":16,"team":1,"platoon":0,"squad":0,"faction":0,
                    "available":available,"default_spawn":HOME}]}]})
        .to_string(),
    )
    .unwrap()
}

/// One structural objective whose forward spawn (a type-8 field hospital of
/// user class 0) belongs to squad 16, active while the structure stands.
fn bindings() -> Bindings {
    serde_json::from_value(serde_json::json!({
        "schema":"SYNTHETIC","map":"synthetic_acquisition",
        "metadata_sha256":"0".repeat(64),"initial_objective_sha256":"1".repeat(64),
        "level_archive_sha256":"2".repeat(64),"level_clv_sha256":"3".repeat(64),
        "rows":{"7":{"token":700,"class":"structural","start_deactivated":true,"hooks":{},
            "spawn":{"token":102,"index":HOSPITAL,"team":1,"user_class":0,"spawn_type":8,
                "fsp_type":0,"vehicle":false}}},
        "squads":{"16":{"points":[[HOME,100],[1,101],[HOSPITAL,102]],"category_zero":[HOME,100]}}}))
    .unwrap()
}

#[test]
fn an_unsupported_field_hospital_falls_back_locally_and_stays_the_native_default() {
    let mut catalog = spawns("unsupported", false);
    bindings().configure_spawns(&mut catalog).unwrap();
    assert_eq!(catalog.locally_selectable(16, HOSPITAL), HOME);
    catalog.set_runtime_squad_default(16, HOSPITAL).unwrap();
    assert_eq!(catalog.native_unselectable_default(16), Some(HOSPITAL));
    catalog
        .set_runtime_point_active(HOSPITAL, 102, false)
        .unwrap();
    assert_eq!(catalog.native_unselectable_default(16), None);
}

#[test]
fn a_selectable_field_hospital_needs_no_fallback_and_becomes_the_squad_default() {
    let mut catalog = spawns("alternate", true);
    bindings().configure_spawns(&mut catalog).unwrap();
    assert_eq!(catalog.locally_selectable(16, HOSPITAL), HOSPITAL);
    // The structure standing at match start routes the squad there.
    let plan = bindings()
        .plan_fsp(
            700,
            bindings().rows[&7].spawn.unwrap(),
            ObjectiveState::Deactivated,
        )
        .unwrap();
    let (catalog, ()) = fsp::stage_spawn_effects(&catalog, &plan)
        .unwrap()
        .complete(|_| Ok(()))
        .unwrap();
    assert_eq!(catalog.default_for_squad(16), Some(HOSPITAL));
    assert_eq!(catalog.native_unselectable_default(16), None);
    assert!(catalog.bot_arrival_point(16, HOSPITAL).is_some());
}
