//! SYNTHETIC inputs authored from Rust schema constraints, never game catalogs or captures.
use mag_gameplay::{
    blast_rules::CharacterBlastRules,
    fall_damage::Rules,
    parse_request,
    replication::{ObjectUpdate, UpdateHeader},
    spawn_catalog::SpawnCatalog,
    wire_profile::WireProfileCatalog,
};
use serde_json::{Value, json};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

struct SyntheticFile(PathBuf);
impl SyntheticFile {
    fn new(value: &Value) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "mag-synthetic-schema-{}-{}.json",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(&path, serde_json::to_vec(value).unwrap()).unwrap();
        Self(path)
    }
}
impl Drop for SyntheticFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
fn reservation() -> Value {
    json!({"v":2,"request_id":"synthetic-request","op":"reserve",
        "reservation_id":"synthetic-reservation-0001","world_id":1,"player_id":2,
        "account_id":3,"session_key":"aaaaaaaaaaaaaaaa","access_key":"bbbbbbbbbbbbbbbb",
        "character":{"name":"Synthetic Player","faction":0,"head":0,"voice":0,"universe_id":1}})
}
#[test]
fn admission_rejects_invalid_identity_credentials_and_unknown_fields() {
    assert!(parse_request(reservation().to_string().as_bytes()).is_ok());
    for (field, value, code) in [
        ("account_id", json!(0), "invalid_account_id"),
        ("session_key", json!("short"), "invalid_credentials"),
        (
            "reservation_id",
            json!("bad/path"),
            "invalid_reservation_id",
        ),
        ("v", json!(1), "unsupported_version"),
        ("request_id", json!("bad\nrequest"), "invalid_request_id"),
    ] {
        let mut request = reservation();
        request[field] = value;
        assert_eq!(
            parse_request(request.to_string().as_bytes())
                .err()
                .unwrap()
                .code,
            code
        );
    }
    let mut request = reservation();
    request["unexpected"] = json!(true);
    assert_eq!(
        parse_request(request.to_string().as_bytes())
            .err()
            .unwrap()
            .code,
        "invalid_json_or_schema"
    );
}
#[test]
fn admission_bounds_payload_and_correlation_recovery() {
    let mut request = reservation();
    request["session_key"] = json!("SYNTHETIC_BAD");
    let error = parse_request(request.to_string().as_bytes()).err().unwrap();
    assert_eq!(error.request_id, "synthetic-request");
    assert!(!format!("{error:?}").contains("SYNTHETIC_BAD"));
    assert_eq!(
        parse_request(&vec![b' '; mag_gameplay::MAX_LINE + 1])
            .err()
            .unwrap()
            .code,
        "line_too_long"
    );
    let event = json!({"v":2,"request_id":"synthetic-event","op":"event",
        "reservation_id":"synthetic-reservation-0001","world_id":1,"player_id":2,
        "transport":"udp","direction":"client_to_server","rt_type":61,"plugin_id":1,
        "payload_hex":"zz"});
    assert!(parse_request(event.to_string().as_bytes()).is_err());
}
#[test]
fn synthetic_wire_profile_schema_rejects_missing_or_duplicate_profiles() {
    let mut value: Value =
        serde_json::from_str(include_str!("../data/synthetic-wire-profiles.json")).unwrap();
    let catalog = WireProfileCatalog::from_json(&value.to_string()).unwrap();
    assert_eq!(catalog.profile(20230).unwrap().header_bits(), 147);
    assert_eq!(catalog.profile(20231).unwrap().header_bits(), 148);
    assert!(catalog.profile(1).is_err());
    value["profiles"][1] = value["profiles"][0].clone();
    assert!(WireProfileCatalog::from_json(&value.to_string()).is_err());
    value["profiles"] = json!([]);
    assert!(WireProfileCatalog::from_json(&value.to_string()).is_err());
}
#[test]
fn synthetic_empty_replication_roundtrips_both_profiles_and_rejects_truncation() {
    let catalog =
        WireProfileCatalog::from_json(include_str!("../data/synthetic-wire-profiles.json"))
            .unwrap();
    for id in [20230, 20231] {
        let profile = catalog.profile(id).unwrap();
        let header = UpdateHeader {
            sequence: 19,
            nak_sequence: 7,
            client_index: 3,
            object_type_mask: 0,
            timestamp_ms: 123,
        };
        let packet = ObjectUpdate { header, wire: &[] }
            .encode_empty_with_profile(profile)
            .unwrap();
        assert_eq!(
            ObjectUpdate::decode_with_profile(&packet, profile)
                .unwrap()
                .header,
            header
        );
        for len in 0..packet.len() {
            assert!(ObjectUpdate::decode_with_profile(&packet[..len], profile).is_err());
        }
        let mut invalid = header;
        invalid.object_type_mask = 1;
        assert!(
            ObjectUpdate {
                header: invalid,
                wire: &[]
            }
            .encode_empty_with_profile(profile)
            .is_err()
        );
    }
}
#[test]
fn synthetic_blast_schema_enforces_application_and_damage_bounds() {
    let mut value = json!({"schema_version":1,"applications":[20230,20231],
        "character_damage":{"armor_rating_reduction":0.125,"armor_prevention_reduction":0.375,
        "hit_location":2,"damage_type":10}});
    assert_eq!(
        CharacterBlastRules::from_value(value.clone())
            .unwrap()
            .hit_location,
        2
    );
    value["character_damage"]["hit_location"] = json!(16);
    assert!(CharacterBlastRules::from_value(value.clone()).is_err());
    value["character_damage"]["hit_location"] = json!(2);
    value["applications"] = json!([1, 2]);
    assert!(CharacterBlastRules::from_value(value).is_err());
}
#[test]
fn synthetic_fall_rules_load_and_validate_quadratic_damage() {
    let mut value = json!({"schema_version":1,"source_build":"retail","application_id":20231,
        "min_height":3.,"damage_factor":4.,"minimum_damage_exclusive":2.,
        "jump_fall_state":1,"damage_type":5,"damage_source_type":1});
    let file = SyntheticFile::new(&value);
    let rules = Rules::load(&file.0).unwrap();
    assert!(rules.damage_for_drop(2.).unwrap().is_none());
    assert_eq!(rules.damage_for_drop(5.).unwrap().unwrap().amount, 16.);
    assert_eq!(
        rules
            .predict_drop(5., 400)
            .unwrap()
            .remaining_health_quarter_points,
        336
    );
    assert!(rules.damage_for_drop(f32::NAN).is_err());
    value["damage_factor"] = json!(0);
    assert!(Rules::load(&SyntheticFile::new(&value).0).is_err());
}
#[test]
fn synthetic_spawn_schema_preserves_identity_and_rejects_broken_links() {
    let mut value = json!({"format":"mag_spawn_catalog","schema_version":1,"source":"SYNTHETIC schema example",
    "active":{"map_name":"synthetic_arena","squad_index":0,"default_spawn":0,"available":[0]},
    "maps":[{"name":"synthetic_arena","spawn_count":1,"locations":[
        {"index":0,"token":1,"kind":"on_foot","position":{"x":10.,"y":2.,"z":12.}}
    ]}]});
    let catalog = SpawnCatalog::load(&SyntheticFile::new(&value).0).unwrap();
    assert_eq!(catalog.map_name(), "synthetic_arena");
    assert_eq!(catalog.point(0).unwrap().position.x, 10.);
    value["active"]["available"] = json!([1]);
    assert!(SpawnCatalog::load(&SyntheticFile::new(&value).0).is_err());
}
