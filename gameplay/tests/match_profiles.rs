//! Invented registry input; no native assets are needed.
use mag_gameplay::match_profiles::Registry;
#[test]
fn registry_requires_absolute_manifest_and_fails_closed() {
    assert!(Registry::load("relative.json").is_err());
}

#[test]
fn profile_selection_has_exact_versioned_and_bounded_identity_schema() {
    use mag_gameplay::{Request, parse_request};
    let raw = serde_json::json!({"v":2,"request_id":"synthetic-request","op":"select_match_profile","formation_id":"synthetic-formation","profile_id":"synthetic-profile"});
    assert!(matches!(
        parse_request(&serde_json::to_vec(&raw).unwrap()).unwrap(),
        Request::SelectMatchProfile { .. }
    ));
    for (field, value) in [
        ("v", serde_json::json!(1)),
        ("formation_id", serde_json::json!("../wrong")),
        ("profile_id", serde_json::json!("x".repeat(65))),
        ("unknown_field", serde_json::json!(true)),
    ] {
        let mut invalid = raw.clone();
        invalid[field] = value;
        assert!(parse_request(&serde_json::to_vec(&invalid).unwrap()).is_err());
    }
}
