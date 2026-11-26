// SYNTHETIC: every index, mesh and catalog here is authored in code; no game
// catalogs or captures.
use super::*;
use crate::bot_structure_collision::{STRUCTURE_WORK_LIMIT, StructureObstacles};
use ObjectiveState::*;
use serde_json::{Value, json};

const MAP: &str = "synthetic_arena";
/// Character movement row: collides with pack layer 9, not with layer 11.
const ROW: u32 = 1 | 1 << 9;
const CONSOLE: u16 = 4;
const SERVER: u16 = 7;

fn catalog(game_type: u8) -> ObjectiveCatalog {
    let per_team: u16 = if game_type == 0 { 4 } else { 8 };
    let object = |index: u16, initial: u8| {
        json!({"token":format!("0x{index:08x}"),"object_index":index,"initial_state":initial})
    };
    ObjectiveCatalog::from_json(
        &json!({"schema_version":1,"format":"mag_retail_objective_catalog","application_id":20231,
        "protocol":crate::protocol::ProtocolInfo::RETAIL_PROTOCOL,"game_type":game_type,
        "map_name":MAP,"entity_family":12,"network_type":12,
        "source":{"authored_catalog_sha256":"0".repeat(64),"binding_capture_sha256":"1".repeat(64),
            "startup_proof_sha256":"2".repeat(64),"source_image_sha256":"3".repeat(64),
            "capture_qualification":"bounded_concurrent_token_slot_capture"},
        "objects":[object(CONSOLE, 1), object(SERVER, 0)],
        "squads":(0..2u16).flat_map(|team| (0..per_team).map(move |slot| json!({"team":team,
            "squad_index":team*16+slot,"initial_objective_index":CONSOLE,
            "authored_objectives":[{"object_index":CONSOLE,"order":1}]}))).collect::<Vec<_>>()})
        .to_string(),
    )
    .unwrap()
}

/// An axis-aligned box mesh of half size `half` centred on its local origin.
fn block(half: [f64; 3]) -> CollisionWorld {
    let [x, y, z] = half;
    let vertices: Vec<[f64; 3]> = (0..8)
        .map(|i| {
            [
                if i & 1 == 0 { -x } else { x },
                if i & 2 == 0 { -y } else { y },
                if i & 4 == 0 { -z } else { z },
            ]
        })
        .collect();
    let faces = [
        [0, 1, 3, 2],
        [4, 6, 7, 5],
        [0, 4, 5, 1],
        [2, 3, 7, 6],
        [0, 2, 6, 4],
        [1, 5, 7, 3],
    ];
    let triangles: Vec<[usize; 3]> = faces
        .iter()
        .flat_map(|[a, b, c, d]| [[*a, *b, *c], [*a, *c, *d]])
        .collect();
    CollisionWorld::from_bytes(
        json!({"format":"mag_collision_world","schema_version":1,"coordinate_space":"world",
            "metadata":{"map_name":MAP,"build":"synthetic","coverage":"provisional",
                "provenance":["synthetic"],"unknowns":["Not a game map"]},
            "instances":[{"id":"part","vertices_xyz":vertices,"triangles":triangles}]})
        .to_string()
        .as_bytes(),
        CollisionLimits::default(),
    )
    .unwrap()
}

fn surface(neutral: char, at: [f64; 3], tag: u8) -> Value {
    json!({"object_token":"0000000a","collision_token":"0000000b",
        "packfile_sha256":neutral.to_string().repeat(64),
        "neutral_sha256":neutral.to_string().repeat(64),"collision_tag":tag,
        "material_indices":[8],"start_seconds":0.0,
        "pose":{"translation":at,"rotation":[[1.,0.,0.],[0.,1.,0.],[0.,0.,1.]]}})
}

fn slots(pairs: &[(&str, i8)]) -> Value {
    let mut map = serde_json::Map::new();
    for state in STATES {
        let slot = pairs
            .iter()
            .find(|(s, _)| *s == state)
            .map_or(-2, |(_, i)| *i);
        map.insert(state.into(), slot.into());
    }
    Value::Object(map)
}

/// The console stands while Deactivated or Activated and is gone once
/// Completed (slot -1). The server's intact sequence starts the intact mast;
/// its destroyed sequence starts the wreck, which stops the mast.
fn index() -> Value {
    json!({"schema":SCHEMA,"map":MAP,"level_archive_sha256":"a".repeat(64),
        "level_clv_sha256":"b".repeat(64),"global_data_sha256":"c".repeat(64),
        "physics_sha256":"d".repeat(64),
        "objectives":[
            {"objective_token":format!("0x{CONSOLE:08x}"),"class":"ObjectiveSecureAndHoldProperties",
             "definition_sha256":"e".repeat(64),
             "state_sequence_indices":slots(&[("Deactivated",0),("Activated",1),("Completed",-1)]),
             "sequences":[
                {"source_sha256":"f".repeat(64),"surfaces":[surface('1',[10.,1.,10.],9)],"starts":[]},
                {"source_sha256":"f".repeat(64),"surfaces":[surface('1',[10.,1.,10.],9)],"starts":[]}]},
            {"objective_token":format!("0x{SERVER:08x}"),"class":"ObjectiveDemolitionStructuralProperties",
             "definition_sha256":"e".repeat(64),
             "state_sequence_indices":slots(&[("Inactive",0),("Deactivated",0),("Locked",0),("Completed",1)]),
             "sequences":[
                {"source_sha256":"f".repeat(64),"surfaces":[surface('2',[30.,1.,10.],9)],"starts":["0000a5a5"]},
                {"source_sha256":"f".repeat(64),
                 "surfaces":[surface('3',[30.,1.,10.],9),surface('3',[31.,1.,10.],11)],
                 "starts":["0000b6b6"]}]}],
        "dependents":{
            "0000a5a5":{"surfaces":[surface('2',[30.,9.,10.],9)],"starts":[],"stops":[]},
            "0000b6b6":{"surfaces":[surface('3',[30.,8.,10.],9)],"starts":[],"stops":["0000a5a5"]}}})
}

fn assets_for(raw: Value, catalog: &ObjectiveCatalog) -> Result<Assets, &'static str> {
    let index: Index = serde_json::from_value(raw).map_err(|_| "parse")?;
    Assets::build(index, MAP, catalog, |neutral, packfile| {
        assert_eq!(neutral, packfile);
        Ok(block([1., 1., 1.]))
    })
}

fn assets(raw: Value) -> Result<Assets, &'static str> {
    assets_for(raw, &catalog(0))
}

fn runtime() -> Runtime {
    Runtime::new(Arc::new(assets(index()).unwrap()), ROW)
}

fn labels(runtime: &Runtime) -> Vec<String> {
    let (collision, row) = runtime.collision();
    collision
        .movement_obstacles(row)
        .into_iter()
        .map(|(label, ..)| label)
        .collect()
}

fn heights(runtime: &Runtime) -> Vec<f64> {
    let (collision, row) = runtime.collision();
    let mut out: Vec<f64> = collision
        .movement_obstacles(row)
        .into_iter()
        .map(|(_, pose, _)| pose.translation[1])
        .collect();
    out.sort_by(f64::total_cmp);
    out
}

fn states(
    console: ObjectiveState,
    server: ObjectiveState,
) -> impl Fn(u16) -> Option<ObjectiveState> {
    move |index| match index {
        CONSOLE => Some(console),
        SERVER => Some(server),
        _ => None,
    }
}

#[test]
fn objectives_place_their_initial_sequences_and_dependents() {
    let mut runtime = runtime();
    let changes = runtime
        .sync_states(true, states(Deactivated, Inactive))
        .unwrap();
    assert_eq!(
        changes,
        vec![
            Change {
                object_index: CONSOLE,
                state: Deactivated as u8,
                sequence: 0,
                packs: 1
            },
            Change {
                object_index: SERVER,
                state: Inactive as u8,
                sequence: 0,
                packs: 2
            },
        ]
    );
    assert_eq!(
        labels(&runtime),
        ["objective-4-0-0", "objective-7-0-0", "objective-7-1-0"]
    );
    assert_eq!(heights(&runtime), [1., 1., 9.]);
    // Nothing changed: no new changes and the same standing geometry.
    let revision = runtime.collision().0.revision();
    assert!(
        runtime
            .sync_states(true, states(Deactivated, Inactive))
            .unwrap()
            .is_empty()
    );
    assert_eq!(runtime.collision().0.revision(), revision);
}

#[test]
fn a_kept_slot_holds_and_an_empty_slot_removes_the_objects() {
    let mut runtime = runtime();
    runtime
        .sync_states(true, states(Deactivated, Inactive))
        .unwrap();
    // Activating keeps (-2) what Deactivated placed; Locked shares slot 0.
    assert!(
        runtime
            .sync_states(false, states(Activating, Locked))
            .unwrap()
            .is_empty()
    );
    assert_eq!(heights(&runtime), [1., 1., 9.]);
    let changed = runtime
        .sync_states(false, states(Activated, Locked))
        .unwrap();
    assert_eq!(changed.len(), 1);
    assert_eq!(changed[0].sequence, 1);
    let changed = runtime
        .sync_states(false, states(Completed, Locked))
        .unwrap();
    assert_eq!(changed[0].packs, 0);
    assert_eq!(labels(&runtime), ["objective-7-0-0", "objective-7-1-0"]);
}

#[test]
fn destruction_swaps_the_server_and_its_wreck_stops_the_mast() {
    let mut runtime = runtime();
    runtime
        .sync_states(true, states(Deactivated, Inactive))
        .unwrap();
    let changed = runtime
        .sync_states(false, states(Deactivated, Completed))
        .unwrap();
    assert_eq!(changed[0].packs, 3);
    // The destroyed racks (one pack on vehicle-only layer 11, which the
    // character row passes) and the wreck replace the racks and the mast.
    assert_eq!(heights(&runtime), [1., 1., 8.]);
    assert_eq!(
        labels(&runtime),
        ["objective-4-0-0", "objective-7-0-0", "objective-7-2-0"]
    );
}

#[test]
fn a_restarted_objective_world_replaces_everything_placed() {
    let mut runtime = runtime();
    runtime
        .sync_states(true, states(Deactivated, Inactive))
        .unwrap();
    runtime
        .sync_states(false, states(Completed, Completed))
        .unwrap();
    assert_eq!(heights(&runtime), [1., 8.]);
    // A fresh round: the wreck goes too, though no sequence stops it.
    runtime
        .sync_states(true, states(Deactivated, Inactive))
        .unwrap();
    assert_eq!(heights(&runtime), [1., 1., 9.]);
}

#[test]
fn placed_packs_block_bot_movement_through_the_structure_snapshot() {
    let sweep = |runtime: &Runtime| {
        let (collision, row) = runtime.collision();
        let mut work = STRUCTURE_WORK_LIMIT;
        StructureObstacles::new(collision, row)
            .sweep_aabb([0., 1., 10.], [20., 1., 10.], [0.4, 0.9, 0.4], &mut work)
            .unwrap()
            .map(|(fraction, normal, label)| (fraction, normal, label.to_owned()))
    };
    let mut runtime = runtime();
    runtime
        .sync_states(true, states(Deactivated, Inactive))
        .unwrap();
    // A body walking along x at the console's height stops at its face.
    let (fraction, normal, label) = sweep(&runtime).expect("the console blocks");
    assert!((fraction - (10. - 1. - 0.4) / 20.).abs() < 1e-3, "{fraction}");
    assert!(normal[0] < -0.9);
    assert_eq!(label, "objective-4-0-0");
    runtime
        .sync_states(false, states(Completed, Inactive))
        .unwrap();
    assert_eq!(sweep(&runtime), None);
}

#[test]
fn the_index_must_bind_to_a_sabotage_catalog_map_and_its_own_dependents() {
    assert!(assets(index()).is_ok());
    let cases: [(&str, fn(&mut Value)); 6] = [
        ("sabotage_structures_source_mismatch", |v| {
            v["map"] = json!("other_map")
        }),
        ("sabotage_structures_source_mismatch", |v| {
            v["schema"] = json!("other")
        }),
        ("sabotage_structures_objective_unbound", |v| {
            v["objectives"][0]["objective_token"] = json!("0x000000ff")
        }),
        ("sabotage_structures_dependent_unbound", |v| {
            v["dependents"]["0000b6b6"]["stops"] = json!(["00000bad"])
        }),
        ("sabotage_structures_objective_invalid", |v| {
            v["objectives"][0]["state_sequence_indices"]["Completed"] = json!(2)
        }),
        ("sabotage_structures_surface_invalid", |v| {
            v["objectives"][0]["sequences"][0]["surfaces"][0]["collision_tag"] = json!(32)
        }),
    ];
    for (code, mutate) in cases {
        let mut raw = index();
        mutate(&mut raw);
        assert_eq!(assets(raw).err(), Some(code));
    }
    assert_eq!(
        assets_for(index(), &catalog(1)).err(),
        Some("sabotage_structures_source_mismatch")
    );
}

#[test]
fn standing_packs_stand_all_match_and_survive_a_restart() {
    let mut raw = index();
    raw["standing"] = json!([surface('4', [50., 5., 10.], 9), surface('4', [50., 1., 10.], 29)]);
    let mut runtime = Runtime::new(Arc::new(assets(raw).unwrap()), ROW | 1 << 29);
    assert_eq!(heights(&runtime), [1., 5.]);
    runtime
        .sync_states(true, states(Deactivated, Inactive))
        .unwrap();
    runtime
        .sync_states(false, states(Completed, Completed))
        .unwrap();
    runtime
        .sync_states(true, states(Deactivated, Inactive))
        .unwrap();
    assert_eq!(heights(&runtime), [1., 1., 1., 5., 9.]);
}
