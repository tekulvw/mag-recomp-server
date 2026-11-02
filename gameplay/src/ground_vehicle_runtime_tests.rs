//! Synthetic source-bound catalog and real replication planner checks.
use super::*;
use crate::ground_vehicles::Catalog;

fn catalog(engine: &Engine<Vec<u8>>) -> Value {
    json!({"format":"mag_ground_vehicles","schema_version":1,
        "source":{"spawn_catalog_sha256":engine.spawn_catalog.as_ref().unwrap().source_sha256().unwrap(),
            "native_elf_sha256":"a".repeat(64),"native_binding_sha256":"b".repeat(64),
            "vehicle_properties_sha256":"c".repeat(64)},
        "vehicles":[{"index":0,"spawn_point_index":0,"spawn_point_token":1000,
            "vehicle_type_token":12345,"faction":1,"team":0,"modifier":0,
            "position":{"x":100.,"y":3.,"z":100.},"rotation":[0.,0.,0.,1.],
            "health_quarter_points":600}]})
}
fn configured() -> Engine<Vec<u8>> {
    let engine = acquisition_entry::acquisition()
        .with_match_limits(1, 64)
        .unwrap();
    let vehicles = Catalog::from_json(
        &catalog(&engine).to_string(),
        engine.spawn_catalog.as_ref().unwrap(),
    )
    .unwrap();
    engine.with_ground_vehicles(vehicles).unwrap()
}

#[test]
fn ground_vehicle_catalog_requires_exact_native_spawn_binding_and_match_side() {
    let engine = acquisition_entry::acquisition();
    let spawns = engine.spawn_catalog.as_ref().unwrap();
    let value = catalog(&engine);
    assert!(Catalog::from_json(&value.to_string(), spawns).is_ok());
    for (field, bad) in [
        ("spawn_point_token", json!(9999)),
        ("faction", json!(0)),
        ("spawn_point_index", json!(99)),
        ("team", json!(1)),
    ] {
        let mut bad_value = value.clone();
        bad_value["vehicles"][0][field] = bad;
        assert!(
            Catalog::from_json(&bad_value.to_string(), spawns).is_err(),
            "{field}"
        );
    }
    let mut bad = value.clone();
    bad["source"]["spawn_catalog_sha256"] = json!("0".repeat(64));
    assert!(Catalog::from_json(&bad.to_string(), spawns).is_err());
    let mut bad = value.clone();
    bad["vehicles"]
        .as_array_mut()
        .unwrap()
        .push(value["vehicles"][0].clone());
    assert!(Catalog::from_json(&bad.to_string(), spawns).is_err());
    let mut unverified = acquisition_entry::acquisition();
    let mut source = spawn_json();
    source["maps"][0]["runtime_indices_verified"] = json!(false);
    source["maps"][0]["forward_spawns"] = json!([]);
    unverified.spawn_catalog = Some(spawns_from_value(source));
    assert!(
        Catalog::from_json(
            &catalog(&unverified).to_string(),
            unverified.spawn_catalog.as_ref().unwrap()
        )
        .is_err()
    );
}
fn spawns_from_value(value: Value) -> SpawnCatalog {
    spawns(value).unwrap()
}

#[test]
fn ground_vehicle_creation_replays_exactly_and_late_join_gets_its_own_snapshot() {
    let mut engine = configured();
    reserve(&mut engine, 0);
    ready(engine.active.as_mut().unwrap());
    let frames = owner_update(&mut engine, 10, 0);
    let packet = protocol::decode_hex(&frames[0].payload_hex);
    let header = ObjectUpdate::decode_with_profile(&packet, engine.wire_profile)
        .unwrap()
        .header;
    assert_eq!(header.object_type_mask, 1 << 1);
    assert_eq!(engine.active.as_ref().unwrap().ground_vehicle_batch, 1);
    let replay = owner_update(&mut engine, 20, header.sequence);
    assert_eq!(protocol::decode_hex(&replay[0].payload_hex), packet);
    assert_eq!(engine.active.as_ref().unwrap().ground_vehicle_batch, 1);
    reserve(&mut engine, 1);
    ready(engine.active.as_mut().unwrap());
    let frames = owner_update(&mut engine, 30, 0);
    let packet = protocol::decode_hex(&frames[0].payload_hex);
    assert_eq!(
        ObjectUpdate::decode_with_profile(&packet, engine.wire_profile)
            .unwrap()
            .header
            .object_type_mask,
        1 << 1
    );
    assert_eq!(engine.active.as_ref().unwrap().ground_vehicle_batch, 1);
}

#[test]
fn ground_vehicle_creation_does_not_commit_when_evidence_recording_fails() {
    let mut engine = configured();
    reserve(&mut engine, 0);
    ready(engine.active.as_mut().unwrap());
    engine.max_log = engine.written;
    let header = UpdateHeader {
        sequence: 1,
        nak_sequence: 0,
        client_index: 0,
        object_type_mask: 0,
        timestamp_ms: 10,
    };
    let packet = ObjectUpdate { header, wire: &[] }
        .encode_empty_with_profile(engine.wire_profile)
        .unwrap();
    assert!(dispatch(&mut engine, &packet, 10).is_err());
    assert_eq!(engine.active.as_ref().unwrap().ground_vehicle_batch, 0);
    assert!(
        engine
            .active
            .as_ref()
            .unwrap()
            .replication_history
            .is_empty()
    );
}
