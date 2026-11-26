//! SYNTHETIC Acquisition LALO drop-zone fixtures. Invented poses and tokens only.
//!
//! A drop zone is the forward spawn (active when complete) of a passive objective
//! that the defenders' anti-aircraft structure completes when destroyed.
use super::*;
use crate::{replication::SpawnAvailability, wire_profile::WireProfile};

const HOME: u16 = 0;
const DROP: u16 = 2;
const SQUAD: u16 = 0;

fn spawns(drop_kind: &str) -> SpawnCatalog {
    let mut drop = serde_json::json!({"index":DROP,"token":102,"kind":drop_kind,"active":false,
        "position":{"x":40.,"y":60.,"z":40.}});
    if drop_kind == "parachute" {
        drop["parachute"] = serde_json::json!({"forward":[0.,0.,1.],"spacing_m":10.,"yaw":0});
    }
    SpawnCatalog::from_json(
        &serde_json::json!({
            "format":"mag_spawn_catalog","schema_version":1,"source":"SYNTHETIC drop zone",
            "active":{"map_name":"synthetic_acquisition","squad_index":SQUAD,"default_spawn":HOME,
                "available":[HOME,1]},
            "match_profile":{"game_type":1,"has_bunkers":false,"has_objectives":false},
            "maps":[{"name":"synthetic_acquisition","owner_faction":1,"spawn_count":3,
                "runtime_indices_verified":true,"runtime_indices_basis":"SYNTHETIC pool identity",
                "locations":[
                    {"index":0,"token":100,"kind":"insertion","position":{"x":10.,"y":0.,"z":10.}},
                    {"index":1,"token":101,"kind":"alternate","position":{"x":20.,"y":0.,"z":10.}},
                    drop],
                "squads":[{"index":SQUAD,"team":0,"platoon":0,"squad":0,
                    "available":[HOME,1],"default_spawn":HOME}]}]})
        .to_string(),
    )
    .unwrap()
}

/// The passive objective (index 8) whose forward spawn is the drop zone.
fn bindings() -> Bindings {
    serde_json::from_value(serde_json::json!({
        "schema":"SYNTHETIC","map":"synthetic_acquisition",
        "metadata_sha256":"0".repeat(64),"initial_objective_sha256":"1".repeat(64),
        "level_archive_sha256":"2".repeat(64),"level_clv_sha256":"3".repeat(64),
        "rows":{"8":{"token":800,"class":"secure_hold","start_deactivated":false,"hooks":{},
            "spawn":{"token":102,"index":DROP,"team":0,"user_class":0,"spawn_type":5,
                "fsp_type":1,"vehicle":false}}},
        "squads":{"0":{"points":[[HOME,100],[1,101],[DROP,102]],"category_zero":[HOME,100]}}}))
    .unwrap()
}

/// Stage the passive objective's entry as the anti-aircraft hook requests it.
fn enter(catalog: &SpawnCatalog, state: ObjectiveState) -> SpawnCatalog {
    let bindings = bindings();
    let plan = bindings
        .plan_fsp(800, bindings.rows[&8].spawn.unwrap(), state)
        .unwrap();
    let (next, ()) = fsp::stage_spawn_effects(catalog, &plan)
        .unwrap()
        .complete(|_| Ok(()))
        .unwrap();
    next
}

fn published(catalog: &SpawnCatalog) -> Vec<u8> {
    SpawnAvailability::encode_update_with_profile(1, 0, catalog, WireProfile::default()).unwrap()
}

#[test]
fn a_parachute_drop_zone_opens_when_the_anti_aircraft_falls_and_closes_on_repair() {
    let mut catalog = spawns("parachute");
    bindings().configure_spawns(&mut catalog).unwrap();
    // Mission start: the standing anti-aircraft keeps the zone closed.
    let closed = enter(&catalog, ObjectiveState::Deactivated);
    assert!(!closed.point_active(DROP));
    assert_eq!(closed.default_for_squad(SQUAD), Some(HOME));
    assert!(closed.for_squad(SQUAD).unwrap().alpha(DROP).is_none());

    // Destroyed: the zone is published, selectable and the squad's spawn.
    let open = enter(&closed, ObjectiveState::Completed);
    assert!(open.point_active(DROP));
    assert_ne!(published(&open), published(&closed));
    assert_eq!(open.default_for_squad(SQUAD), Some(DROP));
    let squad = open.for_squad(SQUAD).unwrap();
    assert_eq!(squad.default_spawn(), DROP);
    let point = squad.alpha(DROP).expect("selectable drop zone");
    assert!(matches!(
        point.kind,
        crate::spawn_catalog::SpawnKind::Parachute
    ));
    assert!(open.bot_arrival_point(SQUAD, DROP).is_some());
    // Later jumpers of one wave start further along the authored forward.
    assert_eq!(point.parachute_position(3).unwrap().z, 70.);
    assert_eq!(point.parachute_position(3).unwrap().y, 60.);

    // Repaired: closed again and the squad returns to its category-zero point.
    let repaired = enter(&open, ObjectiveState::Repaired);
    assert!(!repaired.point_active(DROP));
    assert_eq!(repaired.default_for_squad(SQUAD), Some(HOME));
    assert_eq!(published(&repaired), published(&closed));
    assert!(repaired.for_squad(SQUAD).unwrap().alpha(DROP).is_none());
}

#[test]
fn an_unsupported_drop_zone_is_never_published_even_when_opened() {
    // The catalogs before this projection: the runtime opened the zone, but
    // its availability bit stayed clear, so the client kept the icon greyed.
    let mut catalog = spawns("unsupported");
    bindings().configure_spawns(&mut catalog).unwrap();
    let closed = enter(&catalog, ObjectiveState::Deactivated);
    let open = enter(&closed, ObjectiveState::Completed);
    assert!(open.point_active(DROP));
    assert_eq!(published(&open), published(&closed));
    assert_eq!(open.locally_selectable(SQUAD, DROP), HOME);
    assert!(open.for_squad(SQUAD).unwrap().alpha(DROP).is_none());
}
