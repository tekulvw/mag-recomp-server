//! Invented qualified bindings; no original map, objective IDs or packets.
use mag_gameplay::{
    objective_catalog::ObjectiveCatalog, replication::ObjectiveState,
    wire_profile::WireProfileCatalog,
};
use serde_json::{Value, json};

fn fixture(mode: u8, inactive: bool) -> Value {
    let slots = if mode == 1 { 8 } else { 4 };
    let squads:Vec<_>=(0..2).flat_map(|team| (0..slots).map(move |slot| json!({
        "team":team,"squad_index":team*16+slot,
        "initial_objective_index":if inactive {65535} else {63},
        "authored_objectives":[{"object_index":17,"order":9},{"object_index":63,"order":2},{"object_index":17,"order":9}]
    }))).collect();
    json!({"schema_version":1,"format":"mag_retail_objective_catalog","application_id":20231,"protocol":mag_gameplay::protocol::ProtocolInfo::RETAIL_PROTOCOL,
        "game_type":mode,"map_name":"invented_acquisition","entity_family":12,"network_type":12,
        "source":{"authored_catalog_sha256":"a".repeat(64),"binding_capture_sha256":"b".repeat(64),
            "startup_proof_sha256":"c".repeat(64),"source_image_sha256":"d".repeat(64),
            "capture_qualification":"bounded_concurrent_token_slot_capture"},
        "objects":[{"token":"0x00001001","object_index":17,"initial_state":0},
                   {"token":"0x00001002","object_index":63,"initial_state":if inactive {0} else {1}}],
        "squads":squads})
}

#[test]
fn acquisition_keeps_all_sixteen_ordered_squad_bindings_and_native_indices() {
    let catalog = ObjectiveCatalog::from_json(&fixture(1, false).to_string()).unwrap();
    for team in 0..2 {
        for slot in 0..8 {
            assert_eq!(catalog.initial_assignment(team * 16 + slot), Ok(63));
        }
    }
    assert!(catalog.initial_assignment(8).is_err());
    assert!(catalog.initial_assignment(24).is_err());
    let snapshot = catalog.snapshot(9, 100);
    assert_eq!(
        snapshot
            .objectives
            .iter()
            .map(|x| (x.object_index, x.state_actor.state))
            .collect::<Vec<_>>(),
        [
            (17, ObjectiveState::Inactive),
            (63, ObjectiveState::Deactivated)
        ]
    );
    let profile =
        WireProfileCatalog::from_json(include_str!("../data/synthetic-wire-profiles.json"))
            .unwrap()
            .profile(20231)
            .unwrap();
    assert!(snapshot.encode_with_profile(profile).is_ok());
}

#[test]
fn inactive_acquisition_startup_preserves_explicit_no_assignment() {
    let catalog = ObjectiveCatalog::from_json(&fixture(1, true).to_string()).unwrap();
    assert_eq!(catalog.initial_assignment(7), Ok(u16::MAX));
    assert_eq!(catalog.initial_assignment(23), Ok(u16::MAX));
    assert!(
        catalog
            .snapshot(9, 100)
            .objectives
            .iter()
            .all(|o| o.state_actor.state == ObjectiveState::Inactive)
    );
}

#[test]
fn missing_platoons_false_empty_assignments_and_wrong_modes_fail() {
    let good = fixture(1, false);
    for mutate in 0..4 {
        let mut bad = good.clone();
        match mutate {
            0 => {
                bad["squads"].as_array_mut().unwrap().pop();
            }
            1 => bad["squads"][15]["squad_index"] = json!(24),
            2 => bad["squads"][0]["initial_objective_index"] = json!(65535),
            _ => bad["game_type"] = json!(3),
        }
        assert!(
            ObjectiveCatalog::from_json(&bad.to_string()).is_err(),
            "case{mutate}"
        );
    }
    assert!(ObjectiveCatalog::from_json(&fixture(0, false).to_string()).is_ok());
    assert!(ObjectiveCatalog::from_json(&fixture(0, true).to_string()).is_err());
}

#[test]
fn initial_load_evidence_remains_pre_mission_start_and_noninteractive() {
    let mut source_model = fixture(1, true);
    source_model["source"]["capture_qualification"] = json!("native_initial_load");
    source_model["source"]["initial_load_scope"] =
        json!("pre_mission_start_before_dynamic_creation");
    assert!(ObjectiveCatalog::from_json(&source_model.to_string()).is_ok());
    let mut bad = source_model.clone();
    bad["objects"][1]["initial_state"] = json!(1);
    assert!(ObjectiveCatalog::from_json(&bad.to_string()).is_err());
    let mut bad = source_model.clone();
    bad["source"]
        .as_object_mut()
        .unwrap()
        .remove("initial_load_scope");
    assert!(ObjectiveCatalog::from_json(&bad.to_string()).is_err());
    let mut bad = source_model;
    bad["score_bleed"] =
        json!({"team":0,"interval_ms":1000,"points":1,"pause_objective":17,"pause_state":7});
    assert!(ObjectiveCatalog::from_json(&bad.to_string()).is_err());
}
