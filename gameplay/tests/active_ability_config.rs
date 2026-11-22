use mag_gameplay::active_ability_config;
use std::io;
use std::path::Path;

#[test]
fn source_gameplay_resolves_a_distinct_sidecar_for_each_map() {
    let directory = std::env::temp_dir();
    let provider = directory.join("retail-gameplay.json");
    for (map, name) in [
        (
            "synthetic_home_a",
            "synthetic_home_a--active-abilities.json",
        ),
        (
            "synthetic_home_b",
            "synthetic_home_b--active-abilities.json",
        ),
    ] {
        assert_eq!(
            active_ability_config::manifest_path(None, Some(&provider), map).unwrap(),
            Some(directory.join(name)),
        );
    }
}

#[test]
fn explicit_manifest_relocates_the_required_sidecar() {
    let directory = std::env::temp_dir();
    let provider = directory.join("retail-gameplay.json");
    let chosen = directory.join("custom/selected.json");
    assert_eq!(
        active_ability_config::manifest_path(Some(chosen.clone()), Some(&provider), "map").unwrap(),
        Some(chosen),
    );
}

#[test]
fn default_sidecar_cannot_escape_the_provider_directory() {
    let provider = std::env::temp_dir().join("retail-gameplay.json");
    for invalid in ["../map", r"..\map", "", "C:map"] {
        assert!(active_ability_config::manifest_path(None, Some(&provider), invalid).is_err());
    }
}

#[test]
fn missing_required_sidecar_reports_its_path() {
    let provider = std::env::temp_dir()
        .join(format!("mag-absent-ability-sidecar-{}", std::process::id()))
        .join("retail-gameplay.json");
    let manifest = active_ability_config::manifest_path(None, Some(&provider), "synthetic_map")
        .unwrap()
        .unwrap();
    let error = active_ability_config::load(&manifest).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::NotFound);
    assert!(error.to_string().contains(manifest.to_str().unwrap()));
}
#[test]
fn active_ability_setup_requires_an_explicit_absolute_manifest_path() {
    let error = active_ability_config::load(Path::new("relative-setup.json")).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert!(error.to_string().contains("absolute"));
}

#[test]
fn an_absolute_manifest_cannot_supply_a_parent_relative_rules_path() {
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
            "rules": "../relative-rules.json",
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
