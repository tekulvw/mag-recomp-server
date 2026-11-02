//! Invented controller data shared by integration scenarios.
use mag_gameplay::spawn_catalog::SpawnCatalog;
use serde_json::{Value, json};

pub fn fixture(faction: u8) -> (SpawnCatalog, Value) {
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
