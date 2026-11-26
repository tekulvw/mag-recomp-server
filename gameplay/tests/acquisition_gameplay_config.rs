//! Invented configuration structure, without original assets or file reads.
use mag_gameplay::acquisition_gameplay_config::Configuration;
use serde_json::{Value, json};

fn configuration() -> Value {
    json!({
        "format":"mag_acquisition_gameplay","schema_version":1,
        "source":{"map_name":"invented","spawn_catalog_sha256":"a".repeat(64),"controller_sha256":"b".repeat(64),"objective_binding_sha256":"c".repeat(64),"startup_proof_sha256":"d".repeat(64),"native_rules_sha256":"e".repeat(64)},
        "controller_path":"/invented/controller.json",
        "rules":{"starting_points":[1200,1000],"major_percent":50,"end_delay_ms":2000},
        "vehicle_reinforcement_intervals_ms":[20000,20000],
        "filter_rows":vec![0_u32;32],
        "objectives":(0..8).map(|i|json!({"token":100+i,"object_index":i,"owner_team":1,"class":"arrival","initial_active":false,"activation_score_value":0,"fsp":null})).collect::<Vec<_>>(),
        "squads":(0..2).flat_map(|team|(0..8).map(move|i|json!({"squad_index":team*16+i,"team":team,"gate_value":0,"normal":null,"frago":[]}))).collect::<Vec<_>>(),
        "volumes":(0..4).map(|i|json!({"token":200+i,"pose":{"translation":[0,0,0],"rotation":[[1,0,0],[0,1,0],[0,0,1]]},"half_extents":[1,1,1],"radius":0.05})).collect::<Vec<_>>(),
        "arrival_triggers":(0..4).map(|i|json!({"objective_token":100+i,"volume":{"centre":[0,0,0],"axes":[[1,0,0],[0,1,0],[0,0,1]],"half_extents":[1,1,1]}})).collect::<Vec<_>>(),
        "arrival_static":{"path":"/invented/world.json","sha256":"f".repeat(64)},
        "ground_capabilities":(0..4).map(|i|json!({"vehicle_index":4+i,"capabilities":{"destruction":{"damage":100,"full_radius":8,"half_radius":4},"hulk_template":"invented","delivered_inert_seconds":12}})).collect::<Vec<_>>(),
        "ground_damage":{"friendly_fire_multiplier":0.5,"profiles":[{"vehicle_type_token":321,"faction":0,"max_health":200,"critical_loss_percent_per_second":3,"armor":{"rating":1,"prevention":0.5,"coverage":[0.5,0.5,0.5,0.5]}}],"areas":vec![json!({"armor_area":0,"multiplier":1});16]}
    })
}

#[test]
fn enabled_gameplay_configuration_requires_damage_and_arrival_resources() {
    let complete = configuration();
    assert!(Configuration::from_json(&complete.to_string()).is_ok());
    for field in ["ground_damage", "arrival_static"] {
        let mut missing = complete.clone();
        missing[field] = Value::Null;
        assert!(
            Configuration::from_json(&missing.to_string()).is_err(),
            "missing {field} was admitted"
        );
    }
}

#[test]
fn portable_acquisition_paths_require_a_declaring_file() {
    let dir = std::env::temp_dir().join(format!("mag-portable-acquisition-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let path = dir.join("gameplay.json");
    let mut config = configuration();
    config["controller_path"] = json!("resources/controller.json");
    config["arrival_static"]["path"] = json!("resources/arrival.json");
    std::fs::write(&path, config.to_string()).unwrap();
    let loaded = Configuration::load(&path);
    assert!(Configuration::from_json(&config.to_string()).is_err());
    for rejected in ["", "../controller.json"] {
        config["controller_path"] = json!(rejected);
        std::fs::write(&path, config.to_string()).unwrap();
        assert!(Configuration::load(&path).is_err());
    }
    config["controller_path"] = json!("resources/controller.json");
    config["arrival_static"]["path"] = json!("../arrival.json");
    std::fs::write(&path, config.to_string()).unwrap();
    assert!(Configuration::load(&path).is_err());
    std::fs::remove_dir_all(dir).unwrap();
    match loaded {
        Ok(_) => (),
        Err(error) => panic!("portable configuration rejected: {error}"),
    }
}

#[test]
fn retired_smoke_field_points_operators_to_the_worker_flag() {
    for value in [json!("smoke.json"), json!("")] {
        let mut config = configuration();
        config["smoke_grenades"] = value;
        let error = Configuration::from_json(&config.to_string())
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("--smoke-grenade-rules"), "{error}");
    }
}

#[test]
fn field_hospital_turret_rules_need_a_portable_nonempty_path() {
    let mut config = configuration();
    config["hospital_turrets"] = json!("");
    assert!(Configuration::from_json(&config.to_string()).is_err());
    let dir = std::env::temp_dir().join(format!(
        "mag-acquisition-hospital-turrets-{}",
        std::process::id()
    ));
    std::fs::create_dir(&dir).unwrap();
    let path = dir.join("gameplay.json");
    for (value, admitted) in [("resources/turrets.json", true), ("../turrets.json", false)] {
        config["hospital_turrets"] = json!(value);
        std::fs::write(&path, config.to_string()).unwrap();
        assert_eq!(Configuration::load(&path).is_ok(), admitted, "{value}");
    }
    std::fs::remove_dir_all(dir).unwrap();
}
