//! Invented controller graph and observations; no original map/script records.
use mag_gameplay::{acquisition_control::*, spawn_catalog::SpawnCatalog};
use serde_json::{Value, json};
use std::collections::BTreeMap;

fn fixture(faction: u8) -> (SpawnCatalog, Value) {
    let spawn = SpawnCatalog::from_json(&json!({
        "format":"mag_spawn_catalog","schema_version":1,"source":"invented",
        "match_profile":{"game_type":1,"attacking_faction":faction},
        "active":{"map_name":"invented","squad_index":0,"default_spawn":0,"available":[0]},
        "maps":[{"name":"invented","owner_faction":(faction+1)%3,"spawn_count":5,
            "locations":(0..5).map(|i|json!({"index":i,"token":300+i,"kind":"insertion","position":{"x":100.,"y":1.,"z":100.}})).collect::<Vec<_>>()}]
    }).to_string()).unwrap();
    let chains:Vec<_>=(0..4).map(|i|json!({"platoon":i/2,"stage":if i%2==0{"a"}else{"b"},
        "capture_token":100+i*2,"arrival_token":101+i*2,"vehicle_spawn_token":301+i,"volume_token":200+i})).collect();
    let graph = json!({"format":"mag_acquisition_control","schema_version":1,"controller_token":999,
        "source":{"map_name":"invented","spawn_catalog_sha256":spawn.source_sha256().unwrap(),
            "game_archive_sha256":"a".repeat(64),"script_sha256":"b".repeat(64),"graph_proof_sha256":"c".repeat(64)},
        "chains":chains,"groups":[
            {"branches":[
                {"condition":{"objective_token":101,"state":"completed"},"commands":[{"kind":"objective","target":100,"operation":"lock"},{"kind":"objective","target":102,"operation":"enable"}]},
                {"condition":{"objective_token":100,"state":"completed"},"commands":[{"kind":"objective","target":101,"operation":"enable"}]},
                {"condition":{"objective_token":100,"state":"deactivated"},"commands":[{"kind":"objective","target":101,"operation":"disable"}]}]},
            {"branches":[{"condition":{"objective_token":102,"state":"completed"},"commands":[{"kind":"damage_volume","target":200,"arguments":[0,0,0,0],"damage_type_symbol":"InventedDamage","damage_source_symbol":"InventedSource"}]}]},
            {"branches":[{"condition":{"objective_token":104,"state":"deactivated"},"commands":[{"kind":"objective","target":105,"operation":"disable"}]}]},
            {"branches":[{"condition":{"objective_token":106,"state":"completed"},"commands":[{"kind":"objective","target":107,"operation":"enable"}]}]}
        ],"unbound_calls":[{"symbol":"UnboundClock","arguments":[8,9]}]});
    (spawn, graph)
}
fn states() -> BTreeMap<u32, ObservedState> {
    (100..108)
        .map(|token| (token, ObservedState::Other))
        .collect()
}
fn load(spawn: &SpawnCatalog, graph: &Value) -> Catalog {
    Catalog::from_json(&graph.to_string(), spawn).unwrap()
}

#[test]
fn snapshot_priority_and_all_group_order_are_preserved_for_every_faction() {
    for faction in 0..3 {
        let (spawn, graph) = fixture(faction);
        let catalog = load(&spawn, &graph);
        let mut state = states();
        state.insert(100, ObservedState::Completed);
        state.insert(101, ObservedState::Completed);
        state.insert(102, ObservedState::Completed);
        state.insert(104, ObservedState::Deactivated);
        state.insert(106, ObservedState::Completed);
        let plan = catalog.plan(&state).unwrap();
        assert_eq!(
            serde_json::to_value(&plan.commands).unwrap(),
            json!([
                {"kind":"objective","target":100,"operation":"lock"},
                {"kind":"objective","target":102,"operation":"enable"},
                {"kind":"damage_volume","target":200,"arguments":[0,0,0,0],"damage_type_symbol":"InventedDamage","damage_source_symbol":"InventedSource"},
                {"kind":"objective","target":105,"operation":"disable"},
                {"kind":"objective","target":107,"operation":"enable"}
            ])
        );
        assert_eq!(plan.unbound_calls[0].symbol, "UnboundClock");
        assert_eq!(plan.unbound_calls[0].arguments, [8, 9]);
        assert_eq!(catalog.plan(&state).unwrap(), plan);
        assert_eq!(state[&100], ObservedState::Completed);
    }
}

#[test]
fn missing_observations_fail_and_other_states_do_not_match() {
    let (spawn, graph) = fixture(0);
    let catalog = load(&spawn, &graph);
    let mut state = states();
    assert!(catalog.plan(&state).unwrap().commands.is_empty());
    state.remove(&107);
    assert_eq!(
        catalog.plan(&state).unwrap_err(),
        "incomplete_acquisition_observations"
    );
}

#[test]
fn wrong_sources_duplicate_chains_and_cross_family_targets_are_refused() {
    let (spawn, graph) = fixture(0);
    for (pointer, value) in [
        ("/source/map_name", json!("foreign")),
        ("/source/spawn_catalog_sha256", json!("f".repeat(64))),
        ("/source/script_sha256", json!("not-a-digest")),
        ("/chains/1/platoon", json!(1)),
        ("/chains/1/capture_token", json!(100)),
        ("/chains/0/vehicle_spawn_token", json!(999)),
        ("/chains/0/volume_token", json!(100)),
        ("/groups/0/branches/0/condition/objective_token", json!(999)),
        ("/groups/0/branches/0/commands/0/target", json!(200)),
        ("/groups/1/branches/0/commands/0/target", json!(100)),
        (
            "/groups/1/branches/0/commands/0/damage_type_symbol",
            json!("x();"),
        ),
    ] {
        let mut bad = graph.clone();
        *bad.pointer_mut(pointer).unwrap() = value;
        assert!(
            Catalog::from_json(&bad.to_string(), &spawn).is_err(),
            "{pointer}"
        );
    }
    let mut bad = graph.clone();
    bad["groups"].as_array_mut().unwrap().pop();
    assert!(Catalog::from_json(&bad.to_string(), &spawn).is_err());
    let mut bad = graph.clone();
    bad["groups"][0]["branches"][0]["condition"]["state"] = json!("other");
    assert!(Catalog::from_json(&bad.to_string(), &spawn).is_err());
    let mut bad = graph;
    bad["groups"][0]["branches"][0]["commands"] = json!([]);
    assert!(Catalog::from_json(&bad.to_string(), &spawn).is_err());
}
