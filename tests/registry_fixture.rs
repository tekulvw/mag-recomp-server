//! Build-only neutral runtime inputs for the Horizon/worker integration check.
#[path = "../tests/support/synthetic_runtime_catalogs.rs"]
mod synthetic_runtime_catalogs;

use serde_json::json;
use std::{fs, path::PathBuf};
use synthetic_runtime_catalogs::SyntheticCatalogs;

fn main() {
    let output = PathBuf::from(std::env::var_os("MAG_REGISTRY_TEST_CATALOGS").unwrap());
    assert!(output.is_absolute());
    fs::create_dir(&output).unwrap();
    let base = SyntheticCatalogs::new();
    for entry in fs::read_dir(&base.0).unwrap() {
        let entry = entry.unwrap();
        fs::copy(entry.path(), output.join(entry.file_name())).unwrap();
    }

    let mut profiles = Vec::new();
    for attacking in 0..3 {
        for defending in 0..3 {
            if attacking == defending {
                continue;
            }
            let name = format!("synthetic-{attacking}-{defending}");
            let spawns = output.join(format!("{name}.json"));
            let catalog = json!({
                "format":"mag_spawn_catalog","schema_version":1,"source":"SYNTHETIC registry integration",
                "active":{"map_name":name,"squad_index":0,"default_spawn":0,"available":[0]},
                "match_profile":{"game_type":0,"auto_assign_teams":true,"team_capacity":1,"squad_capacity":1,
                    "attacking_faction":attacking,"has_bunkers":false,"has_objectives":false},
                "maps":[{"name":name,"owner_faction":defending,"spawn_count":1,
                    "locations":[{"index":0,"token":1,"kind":"on_foot","position":{"x":10.,"y":2.,"z":12.}}],
                    "squads":[{"index":0,"team":0,"platoon":0,"squad":0,"faction":null,"available":[0],"default_spawn":0},
                        {"index":16,"team":1,"platoon":0,"squad":0,"faction":defending,"available":[0],"default_spawn":0}]}]
            });
            fs::write(&spawns, serde_json::to_vec(&catalog).unwrap()).unwrap();
            profiles.push(json!({"profile_id":name,"spawn_catalog":spawns}));
        }
    }
    let manifest = json!({"format":"mag_match_profiles_v1","profiles":profiles});
    fs::write(
        output.join("profiles.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
}
