use mag_gameplay::static_collision_partition::Partition;
use mag_gameplay::{
    acquisition_atomic::WorldKey,
    acquisition_spawn_blockers::BlockerQueries,
    acquisition_volume_damage::{Target, TargetKey, TargetKind},
    acquisition_world::{Filters, RootKey, Settings, World},
    dynamic_collision::{Catalog, Pose, Snapshot},
};
use serde_json::json;
#[path = "support/dynamic_collision_fixture.rs"]
mod support;
fn value() -> serde_json::Value {
    json!({"format":"mag_static_filter_partition","schema_version":1,"map_name":"invented","scope":"initial_compiled_collision_and_type1_terrain","source":{"native_binding_sha256":"a".repeat(64),"root_inventory_sha256":"b".repeat(64),"level_archive_sha256":"c".repeat(64),"compiled_catalog_sha256":"d".repeat(64)},"root_count":2,"root_filters":[{"filter_info":9,"root_count":1},{"filter_info":29,"root_count":1}]})
}
#[test]
fn only_actual_current_root_filter_rejection_authorizes_skipping() {
    let p = Partition::from_json(&value().to_string()).unwrap();
    let mut rows = [0; 32];
    rows[21] = 1 << 6;
    assert!(p.require_excluded(21 | (7 << 16), &rows).is_ok());
    rows[21] |= 1 << 9;
    assert!(p.require_excluded(21 | (7 << 16), &rows).is_err());
    assert_eq!(p.map_name(), "invented");
    assert_eq!(p.root_count(), 2);
}
#[test]
fn summary_rejects_unknown_scope_incomplete_counts_duplicate_filters_and_group_claims() {
    for (key, bad) in [
        ("scope", json!("whole_native_world")),
        ("root_count", json!(3)),
        ("root_filters", json!([])),
    ] {
        let mut v = value();
        v[key] = bad;
        assert!(Partition::from_json(&v.to_string()).is_err());
    }
    let mut v = value();
    v["root_filters"][1]["filter_info"] = json!(9);
    assert!(Partition::from_json(&v.to_string()).is_err());
    let mut v = value();
    v["root_filters"][1]["filter_info"] = json!(9 | (1 << 16));
    assert!(Partition::from_json(&v.to_string()).is_err());
    let mut v = value();
    v["source"]["root_inventory_sha256"] = json!("missing");
    assert!(Partition::from_json(&v.to_string()).is_err());
}
#[test]
fn excluded_static_partition_keeps_dynamic_sweeps_and_rechecks_changed_caster_table() {
    let partition = Partition::from_json(&value().to_string()).unwrap();
    let mut fixture = support::fixture();
    fixture["templates"][0]["roots"][1]["translation"] = json!([1.9, 0., 0.]);
    fixture["templates"][0]["roots"][1]["shape"]["half_extents"] = json!([0.1, 0.1, 0.1]);
    let c = Catalog::from_json(&fixture.to_string()).unwrap();
    let actor = Target {
        key: TargetKey {
            id: 3,
            generation: 1,
        },
        kind: TargetKind::Character,
        team: 0,
    };
    let source = Target {
        key: TargetKey {
            id: 1,
            generation: 1,
        },
        kind: TargetKind::GroundVehicle,
        team: 0,
    };
    let snapshot = Snapshot::capture(
        WorldKey {
            world_id: 1,
            incarnation: 1,
        },
        0,
        &[actor, source],
        vec![
            c.instantiate("invented-character", actor, Pose::at([1.45, 0., 0.]), false)
                .unwrap(),
            c.instantiate("invented-vehicle", source, Pose::at([0.; 3]), false)
                .unwrap(),
        ],
        u32::MAX,
    )
    .unwrap();
    let mut rows = [0; 32];
    rows[6] = (1 << 21) | (1 << 22);
    rows[21] = 1 << 22;
    let keys = || {
        (
            RootKey {
                entity: 3,
                generation: 1,
                root: "capsule".into(),
            },
            RootKey {
                entity: 1,
                generation: 1,
                root: "chassis".into(),
            },
        )
    };
    let filters = Filters::owned_entities_for_partition(&snapshot, &partition, rows).unwrap();
    let mut w =
        World::with_excluded_statics(&snapshot, &partition, filters, Settings::default()).unwrap();
    let (a, b) = keys();
    assert_eq!(
        w.character(a, b)
            .unwrap()
            .cast_world([1.5, 0., 0.])
            .unwrap()
            .unwrap()
            .fraction,
        0.
    );
    rows[21] |= 1 << 9;
    let filters = Filters::owned_entities_for_partition(&snapshot, &partition, rows).unwrap();
    let mut w =
        World::with_excluded_statics(&snapshot, &partition, filters, Settings::default()).unwrap();
    let (a, b) = keys();
    assert!(matches!(
        w.character(a, b).unwrap().cast_world([1.5, 0., 0.]),
        Err("static_partition_not_excluded")
    ));
    rows[6] |= 1 << 29;
    assert!(Filters::owned_entities_for_partition(&snapshot, &partition, rows).is_err());
}
