use super::*;
use crate::domination_fsp::Effect;
use serde_json::{Value, json};

fn inputs() -> (SpawnCatalog, ObjectiveCatalog, Value) {
    let squad_spawns: Vec<_> = (0..32).map(|index| json!({
        "index":index,"team":index/16,"platoon":index%16/4,"squad":index%4,
        "faction":if index <16 {None} else {Some(0)},"available":[index/16],"default_spawn":index/16,
    })).collect();
    let spawns = SpawnCatalog::from_json(&json!({
        "format":"mag_spawn_catalog","schema_version":1,"source":"synthetic",
        "active":{"map_name":"synthetic_fsp","squad_index":0,"default_spawn":0,"available":[0]},
        "match_profile":{"game_type":2,"auto_assign_teams":true,"attacking_faction":1,
            "has_objectives":true,"has_bunkers":false},
        "maps":[{"name":"synthetic_fsp","owner_faction":0,"spawn_count":3,
            "runtime_indices_verified":true,"runtime_indices_basis":"synthetic",
            "squads":squad_spawns,"locations":[
                {"index":0,"token":100,"kind":"insertion","position":{"x":0.,"y":0.,"z":0.}},
                {"index":1,"token":101,"kind":"insertion","position":{"x":1.,"y":0.,"z":0.}},
                {"index":2,"token":102,"kind":"on_foot","active":false,"position":{"x":2.,"y":0.,"z":0.}},
            ]}],
    }).to_string()).unwrap();
    let objectives = ObjectiveCatalog::from_json(&json!({
        "format":"mag_retail_objective_catalog","schema_version":1,"application_id":20231,
        "protocol":crate::protocol::ProtocolInfo::RETAIL_PROTOCOL,"game_type":2,"map_name":"synthetic_fsp",
        "entity_family":12,"network_type":12,
        "source":{"authored_catalog_sha256":"a".repeat(64),"binding_capture_sha256":"b".repeat(64),
            "startup_proof_sha256":"c".repeat(64),"source_image_sha256":"d".repeat(64),
            "capture_qualification":"native_initial_load","initial_load_scope":"pre_mission_start_before_dynamic_creation"},
        "objects":[{"token":"0x000000c8","object_index":0,"initial_state":0}],
        "squads":(0..32).map(|i|json!({"team":i/16,"squad_index":i,"initial_objective_index":65535,
            "authored_objectives":[{"object_index":0,"order":2}]})).collect::<Vec<_>>(),
    }).to_string()).unwrap();
    let bindings = json!({
        "format":"mag_domination_fsp_bindings_v1","metadata_sha256":"a".repeat(64),
        "spawn_catalog_sha256":spawns.source_sha256().unwrap(),"map_name":"synthetic_fsp",
        "points":[{"token":100,"index":0,"team":0,"user_class":0},
            {"token":101,"index":1,"team":1,"user_class":0},
            {"token":102,"index":2,"team":0,"user_class":0}],
        "links":[{"objective_token":200,"spawn_token":102,"fsp_type":1}],
        "squads":(0..32).map(|i|json!({"index":i,"points":if i<16 {vec![100,102]} else {vec![101]},
            "category_zero":if i<16 {100} else {101}})).collect::<Vec<_>>(),
    });
    (spawns, objectives, bindings)
}

#[test]
fn uses_full_native_membership_and_not_the_initial_ui_subset() {
    let (spawns, objectives, raw) = inputs();
    assert!(!spawns.map_pool().squads[0].available.contains(&2));
    let bindings = Bindings::from_json(raw.to_string().as_bytes(), &spawns, &objectives).unwrap();
    let candidate = bindings
        .plan_entry(200, ObjectiveState::Completed, true)
        .unwrap();
    let selected: Vec<_> = candidate
        .effects()
        .iter()
        .filter_map(|e| match e {
            Effect::SetSquadSpawn { squad_index, point } => Some((*squad_index, point.native_id)),
            _ => None,
        })
        .collect();
    assert_eq!(
        selected,
        (0..16).map(|i| (i, 0x2380_0002)).collect::<Vec<_>>()
    );
    let candidate = bindings
        .plan_entry(200, ObjectiveState::Deactivated, false)
        .unwrap();
    assert!(candidate.effects().iter().any(
        |e| matches!(e,Effect::SetSquadSpawn{squad_index:0,point} if point.native_id==0x2380_0000)
    ));
}

#[test]
fn rejects_foreign_sources_rebound_points_and_incomplete_squad_membership() {
    let (spawns, objectives, baseline) = inputs();
    for change in [
        "source",
        "map",
        "point",
        "team",
        "fallback",
        "missing_squad",
        "duplicate_link",
    ] {
        let mut raw = baseline.clone();
        match change {
            "source" => raw["spawn_catalog_sha256"] = json!("e".repeat(64)),
            "map" => raw["map_name"] = json!("foreign"),
            "point" => raw["points"][2]["index"] = json!(1),
            "team" => raw["points"][2]["team"] = json!(1),
            "fallback" => raw["squads"][0]["category_zero"] = json!(101),
            "missing_squad" => {
                raw["squads"].as_array_mut().unwrap().pop();
            }
            "duplicate_link" => {
                let link = raw["links"][0].clone();
                raw["links"].as_array_mut().unwrap().push(link);
            }
            _ => unreachable!(),
        }
        assert!(
            Bindings::from_json(raw.to_string().as_bytes(), &spawns, &objectives).is_err(),
            "{change}"
        );
    }
}
