use mag_gameplay::active_ability_config;
use std::io;
use std::path::Path;

#[test]
fn active_ability_setup_requires_an_explicit_absolute_manifest_path() {
    let error = active_ability_config::load(Path::new("relative-setup.json")).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert!(error.to_string().contains("absolute"));
}

#[test]
fn an_absolute_manifest_cannot_supply_a_relative_rules_path() {
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    let name = format!(
        "mag-synthetic-ability-config-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let path = std::env::temp_dir().join(name);
    fs::write(
        &path,
        serde_json::to_vec(&serde_json::json!({
            "format": "mag_active_abilities_v1",
            "rules": "relative-rules.json",
            "effects": "/example/effects.json",
            "map_bindings": "/example/objectives.json",
            "static_ray_receipt": "/example/scene.json"
        }))
        .unwrap(),
    )
    .unwrap();
    let result = active_ability_config::load(&path);
    fs::remove_file(path).unwrap();
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidInput);
}
