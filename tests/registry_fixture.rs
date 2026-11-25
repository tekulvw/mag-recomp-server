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
    let mut party_profiles = Vec::new();
    let mut large_profiles = Vec::new();
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

            let mut party_catalog = catalog;
            party_catalog["match_profile"]["team_capacity"] = json!(4);
            party_catalog["match_profile"]["squad_capacity"] = json!(2);
            party_catalog["maps"][0]["squads"]
                .as_array_mut()
                .unwrap()
                .extend([
                    json!({"index":1,"team":0,"platoon":0,"squad":1,"faction":null,"available":[0],"default_spawn":0}),
                    json!({"index":17,"team":1,"platoon":0,"squad":1,"faction":defending,"available":[0],"default_spawn":0}),
                ]);
            let party_spawns = output.join(format!("party-{name}.json"));
            fs::write(&party_spawns, serde_json::to_vec(&party_catalog).unwrap()).unwrap();
            party_profiles.push(json!({"profile_id":name,"spawn_catalog":party_spawns}));

            // A 128-Character world (world_capacity_v1): 64 per side over eight
            // eight-member squads on each side.
            let mut large_catalog = party_catalog;
            large_catalog["match_profile"]["team_capacity"] = json!(64);
            large_catalog["match_profile"]["squad_capacity"] = json!(8);
            large_catalog["maps"][0]["squads"] = json!((0..2)
                .flat_map(|team| (0..8).map(move |k| (team, k)))
                .map(|(team, k)| {
                    let index = team * 16 + k;
                    let faction = if team == 0 { json!(null) } else { json!(defending) };
                    json!({"index":index,"team":team,"platoon":k / 4,"squad":k % 4,"faction":faction,
                        "available":[0],"default_spawn":0})
                })
                .collect::<Vec<_>>());
            let large_spawns = output.join(format!("large-{name}.json"));
            fs::write(&large_spawns, serde_json::to_vec(&large_catalog).unwrap()).unwrap();
            large_profiles.push(json!({"profile_id":name,"spawn_catalog":large_spawns}));
        }
    }
    let manifest = json!({"format":"mag_match_profiles_v1","profiles":profiles});
    fs::write(
        output.join("profiles.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    let manifest = json!({"format":"mag_match_profiles_v1","profiles":party_profiles});
    fs::write(
        output.join("party-profiles.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    let manifest = json!({"format":"mag_match_profiles_v1","profiles":large_profiles});
    fs::write(
        output.join("large-profiles.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
}
