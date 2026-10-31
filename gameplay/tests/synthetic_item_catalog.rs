//! Independently constructed schema fixtures; no extracted item parameters.
use mag_gameplay::{blast_rules::CharacterBlastRules, item_catalog::ItemCatalog};
use serde_json::{Value, json};

fn rules() -> CharacterBlastRules {
    CharacterBlastRules {
        armor_rating_reduction: 0.25,
        armor_prevention_reduction: 0.75,
        hit_location: 3,
        damage_type: 10,
    }
}
fn catalog() -> Value {
    json!({"schema_version":2,"application_id":20231,
        "items":{
            "FactionItemList0:2":{"authored_ref":"FactionItemList0:2","faction_list":"FactionItemList0","local_ordinal":2,"wire_item_id":null,"name":"synthetic_tool","item_type":"kActiveItem","definition":{"group":"active_items","name":"synthetic_tool"}},
            "FactionItemList2:17":{"authored_ref":"FactionItemList2:17","faction_list":"FactionItemList2","local_ordinal":17,"wire_item_id":null,"name":"synthetic_rifle","item_type":"kWeapon","definition":{"group":"weapons","name":"synthetic_rifle"}},
            "FactionItemList3:23":{"authored_ref":"FactionItemList3:23","faction_list":"FactionItemList3","local_ordinal":23,"wire_item_id":null,"name":null,"item_type":null,"definition":null}
        },
        "definitions":{"active_items":{"synthetic_tool":{"parameters":{}}},"weapons":{"synthetic_rifle":{"parameters":{"magazine_capacity":7}}}},
        "resources":{}})
}
fn parse(raw: Value) -> std::io::Result<ItemCatalog> {
    ItemCatalog::from_value_for_application(raw, rules(), 20231)
}
#[test]
fn retail_authored_coordinates_resolve_native_wire_ids() {
    let items = parse(catalog()).unwrap();
    assert_eq!(items.application_id(), 20231);
    assert_eq!(items.validate_item(2), Ok(()));
    assert_eq!(items.validate_item(2115), Ok(()));
    assert_eq!(items.primary_magazine_capacity(2115), Ok(7));
    assert_eq!(items.validate_item(3145), Err("reserved_saved_item"));
    assert_eq!(items.validate_item(17), Err("unknown_saved_item"));
    assert_eq!(
        items.validate_item(4096),
        Err("saved_item_out_of_wire_range")
    );
}
#[test]
fn retail_has_no_prototype_equipment_or_defaults() {
    let items = parse(catalog()).unwrap();
    assert!(items.valor_loadout(0).is_none());
    assert!(items.bullet(13).is_err());
    assert!(items.bullet(2115).is_err());
    assert!(items.vehicle_bullet(13).is_err());
    assert!(items.armor(116).is_err());
    assert!(items.healing(125).is_err());
    assert!(items.grenade(42).is_err());
    assert!(items.default_grenade_item().is_err());
}
#[test]
fn retail_rejects_wrong_profile_schema_and_empty_items() {
    for (key, value) in [
        ("application_id", json!(20230)),
        ("schema_version", json!(1)),
        ("items", json!({})),
    ] {
        let mut raw = catalog();
        raw[key] = value;
        assert!(parse(raw).is_err(), "{key}");
    }
    assert!(ItemCatalog::from_value_for_application(catalog(), rules(), 20230).is_err());
    assert!(ItemCatalog::from_value_for_application(catalog(), rules(), 99).is_err());
}
#[test]
fn retail_rejects_inconsistent_coordinates_and_embedded_wire_ids() {
    for (key, value) in [
        ("authored_ref", json!("FactionItemList2:18")),
        ("faction_list", json!("FactionItemList1")),
        ("local_ordinal", json!(18)),
        ("wire_item_id", json!(2115)),
    ] {
        let mut raw = catalog();
        raw["items"]["FactionItemList2:17"][key] = value;
        assert!(parse(raw).is_err(), "{key}");
    }
}
#[test]
fn retail_rejects_out_of_domain_and_noncanonical_references() {
    for reference in [
        "FactionItemList4:0",
        "FactionItemList3:1023",
        "FactionItemList1:1024",
        "FactionItemList0:02",
        "FactionItemList2:+17",
    ] {
        let mut raw = catalog();
        let row = raw["items"]
            .as_object_mut()
            .unwrap()
            .remove("FactionItemList2:17")
            .unwrap();
        raw["items"][reference] = row;
        assert!(parse(raw).is_err(), "{reference}");
    }
}
#[test]
fn retail_rejects_unresolved_definition_and_malformed_reserved_row() {
    let mut raw = catalog();
    raw["definitions"]["weapons"] = json!({});
    assert!(parse(raw).is_err());
    let mut raw = catalog();
    raw["items"]["FactionItemList3:23"]["name"] = json!("synthetic_reserved");
    assert!(parse(raw).is_err());
}

#[test]
fn retail_preserves_assigned_unresolved_items_without_claiming_support() {
    let mut raw = catalog();
    raw["items"]["FactionItemList0:2"]["definition"] = Value::Null;
    let items = parse(raw).unwrap();
    assert_eq!(items.validate_item(2), Err("unresolved_saved_item"));
}

#[test]
fn retail_disk_loader_needs_only_explicit_catalog_and_rules() {
    let directory = std::env::temp_dir().join(format!(
        "mag-synthetic-retail-loader-{}",
        std::process::id()
    ));
    std::fs::create_dir(&directory).unwrap();
    let items = directory.join("items.json");
    let blast = directory.join("blast.json");
    std::fs::write(&items, catalog().to_string()).unwrap();
    std::fs::write(&blast, json!({"schema_version":1,"applications":[20230,20231],"character_damage":{
        "armor_rating_reduction":0.25,"armor_prevention_reduction":0.75,"hit_location":3,"damage_type":10
    }}).to_string()).unwrap();
    let result = ItemCatalog::load_for_application(&items, &blast, 20231);
    let wrong_application = ItemCatalog::load_for_application(&items, &blast, 20230);
    std::fs::remove_dir_all(&directory).unwrap();
    assert_eq!(result.unwrap().primary_magazine_capacity(2115), Ok(7));
    assert!(wrong_application.is_err());
}
