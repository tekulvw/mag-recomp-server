//! Synthetic coverage of every character firearm class on the retail path.
//!
//! The catalogs below are invented. Weapons other than the projected starter
//! rifle are described only by the item catalog, which is how retail firearms
//! beyond the projected rows gain a bullet profile.
use super::*;
use crate::human_kill_reward_tests::{
    combat_fixture_documents, finish_combat_fixture, fixture, native_hit, victim,
};
use crate::spawn_progression::dispatch;

const WEAPON: u16 = 20;
const OTHER_WEAPON: u16 = 21;

#[derive(Clone, Copy)]
struct Firearm {
    category: &'static str,
    character: f64,
    effective: f64,
    maximum: f64,
    ammo_token: Option<u64>,
}

impl Firearm {
    fn new(category: &'static str, character: f64) -> Self {
        Self {
            category,
            character,
            effective: 50.0,
            maximum: 50.0,
            ammo_token: Some(500),
        }
    }
}

#[derive(Clone, Copy)]
struct Attachment {
    ordinal: u16,
    kind: &'static str,
    faction: &'static str,
    listed: bool,
}

const fn attachment(ordinal: u16, kind: &'static str, listed: bool) -> Attachment {
    Attachment {
        ordinal,
        kind,
        faction: "FactionTypeRed",
        listed,
    }
}

struct Built {
    items: item_catalog::ItemCatalog,
    metadata: Value,
}

fn build(firearms: &[(u16, Firearm)], attachments: &[Attachment], matching_source: bool) -> Built {
    use sha2::{Digest, Sha256};
    let (mut raw, mut metadata) = combat_fixture_documents(false, 0.0, 0.0, None, false, false);
    for attachment in attachments {
        let name = format!("invented_attachment_{}", attachment.ordinal);
        let reference = format!("FactionItemList0:{}", attachment.ordinal);
        raw["items"][&reference] = json!({
            "authored_ref":reference,"faction_list":"FactionItemList0",
            "local_ordinal":attachment.ordinal,"wire_item_id":null,"name":name,
            "item_type":"kAttachmentItem","definition":{"group":"attachments","name":name}
        });
        raw["definitions"]["attachments"][&name]["parameters"] = json!({
            "id":name,"attachment_type":attachment.kind,"weapon_name":"",
            "character_attachment_type":"kCharAttachTypePrimary","faction":attachment.faction
        });
    }
    for &(ordinal, firearm) in firearms {
        let name = format!("invented_firearm_{ordinal}");
        let ammo = format!("invented_round_{ordinal}");
        let reference = format!("FactionItemList0:{ordinal}");
        raw["items"][&reference] = json!({
            "authored_ref":reference,"faction_list":"FactionItemList0","local_ordinal":ordinal,
            "wire_item_id":null,"name":name,"item_type":"kWeapon",
            "definition":{"group":"weapons","name":name}
        });
        let list: serde_json::Map<String, Value> = attachments
            .iter()
            .filter(|attachment| attachment.listed)
            .enumerate()
            .map(|(row, attachment)| {
                (
                    format!("WeaponAttachment{row}"),
                    json!({"WeaponAttachment":{"name":format!("invented_attachment_{}", attachment.ordinal)}}),
                )
            })
            .collect();
        raw["definitions"]["weapons"][&name] = json!({"parameters":{
            "id":name,"category":firearm.category,"ammo_name":ammo,"faction":"FactionTypeRed",
            "effective_range":firearm.effective,"max_range":firearm.maximum,
            "magazine_capacity":8,"initial_magazine_count":3,"infinite_ammo":false,
            "initial_fire_mode":"kSemiAuto",
            "fire_mode":{"refire_time":0.25,"reload_time":2.0,"fire_modes":{
                "FireModeTypeClass0":{"FireModeTypeClass":{"fire_mode_value":"kSemiAuto"}}
            }},
            "attachment_list":list
        }});
        raw["definitions"]["hitscan_ammo"][&ammo] = json!({
            "parameters":{"damage":{
                "character":firearm.character,"characterAPierce":0.0,"characterARedux":0.0,
                "vehicle":0.0,"vehicleAPierce":0.0,"vehicleARedux":0.0,
                "objective":0.0,"objectiveAPierce":0.0,"objectiveARedux":0.0,"mine":0.0
            }}
        });
        if let Some(token) = firearm.ammo_token {
            raw["definitions"]["hitscan_ammo"][&ammo]["native_token_u32"] = json!(token);
        }
    }
    let bytes = serde_json::to_vec(&raw).unwrap();
    metadata["provenance"]["catalog_sha256"] = json!(if matching_source {
        format!("{:x}", Sha256::digest(&bytes))
    } else {
        "0".repeat(64)
    });
    let path = std::env::temp_dir().join(format!(
        "mag-synthetic-firearm-coverage-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&path).unwrap();
    std::fs::write(
        path.join("blast.json"),
        json!({"schema_version":1,"applications":[20230,20231],"character_damage":{
            "armor_rating_reduction":0.0,"armor_prevention_reduction":0.0,
            "hit_location":3,"damage_type":10}})
        .to_string(),
    )
    .unwrap();
    std::fs::write(path.join("items.json"), &bytes).unwrap();
    let items = item_catalog::ItemCatalog::load_for_application(
        path.join("items.json"),
        path.join("blast.json"),
        20231,
    );
    std::fs::remove_dir_all(&path).unwrap();
    Built {
        items: items.unwrap(),
        metadata,
    }
}

/// Equip `weapon` in `gear` and fill attachment slots; the victim starts at 100 HP.
fn engine_with(built: Built, gear: u8, weapon: u16, slots: &[(usize, u16)]) -> Engine<Vec<u8>> {
    let mut engine = fixture();
    engine.item_catalog = finish_combat_fixture(built.items, &built.metadata);
    let actor = engine.active.as_mut().unwrap();
    let inventory = &mut actor.spawned_loadout.as_mut().unwrap().inventories[0];
    inventory.slots[usize::from(gear)] = Some(weapon);
    for &(slot, item) in slots {
        inventory.slots[slot] = Some(item);
    }
    if gear == 1 {
        actor.primary_equipped = false;
        actor.observed_secondary = Some(replication::InventoryItem {
            gear_slot: 1,
            ..actor.observed_primary.unwrap()
        });
    }
    engine
        .peers
        .get_mut("synthetic-reservation-0000")
        .unwrap()
        .life = CharacterLife::Alive {
        generation: 4,
        health_quarter_points: 400,
    };
    engine
}

fn shoot(engine: &mut Engine<Vec<u8>>, gear: u8, distance: f32, user_data: u32) {
    let mut wire = native_hit(0x0380_0000, user_data);
    wire[2] = gear;
    wire[25..29].copy_from_slice(&(100.0_f32 + distance).to_be_bytes());
    dispatch(engine, &wire, 1000 + u64::from(user_data)).unwrap();
}

fn health(engine: &Engine<Vec<u8>>) -> Option<u16> {
    victim(engine).life.damage_health().map(|(_, hp)| hp)
}

fn audits(engine: &Engine<Vec<u8>>) -> Vec<Value> {
    std::str::from_utf8(&engine.writer)
        .unwrap()
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .collect()
}

fn denial(engine: &Engine<Vec<u8>>) -> Option<(String, Value)> {
    audits(engine).into_iter().find_map(|entry| {
        let code = entry["gameplay_denial"].as_str()?.to_owned();
        let equipment = entry
            .pointer("/decoded/refused_equipment")
            .or_else(|| entry.get("refused_equipment"))
            .cloned()
            .unwrap_or(Value::Null);
        Some((code, equipment))
    })
}

#[test]
fn every_character_firearm_class_deals_its_authored_damage() {
    for (category, gear, damage) in [
        ("kWepCategoryAssaultRifle", 0, 30.0),
        ("kWepCategorySubMachineGun", 0, 24.0),
        ("kWepCategoryMachineGun", 0, 36.0),
        ("kWepCategorySniperRifle", 0, 70.0),
        ("kWepCategoryShotgun", 0, 10.0),
        ("kWepCategorySideArm", 1, 20.0),
    ] {
        let built = build(&[(WEAPON, Firearm::new(category, damage))], &[], true);
        let mut engine = engine_with(built, gear, WEAPON, &[]);
        shoot(&mut engine, gear, 1.0, 60);
        assert_eq!(
            health(&engine),
            Some(400 - (damage as u16) * 4),
            "{category}: {:?}",
            denial(&engine)
        );
    }
}

#[test]
fn every_shotgun_pellet_report_deals_the_authored_per_pellet_damage() {
    let built = build(
        &[(WEAPON, Firearm::new("kWepCategoryShotgun", 10.0))],
        &[],
        true,
    );
    let mut engine = engine_with(built, 0, WEAPON, &[]);
    for pellet in 0..3 {
        shoot(&mut engine, 0, 1.0, 70 + pellet);
    }
    assert_eq!(health(&engine), Some(400 - 3 * 40));
}

#[test]
fn item_catalog_firearm_uses_its_authored_range_falloff() {
    let firearm = Firearm {
        effective: 10.0,
        maximum: 30.0,
        ..Firearm::new("kWepCategoryAssaultRifle", 40.0)
    };
    let built = build(&[(WEAPON, firearm)], &[], true);
    let mut engine = engine_with(built, 0, WEAPON, &[]);
    // Halfway from effective to maximum range keeps three quarters: 30 HP.
    shoot(&mut engine, 0, 20.0, 80);
    assert_eq!(health(&engine), Some(400 - 120), "{:?}", denial(&engine));
}

#[test]
fn sights_suppressors_and_grips_keep_the_host_damage() {
    let firearm = Firearm::new("kWepCategorySniperRifle", 70.0);
    let attachments = [
        attachment(40, "kAttachTypeSuppressor", true),
        // Sights and grips the item list offers but no weapon list names.
        attachment(41, "kAttachTypeSight", false),
        attachment(42, "kAttachTypeForwardGrip", false),
        attachment(43, "kAttachTypeSight", true),
    ];
    for slots in [
        vec![],
        vec![(16, 40)],
        vec![(15, 41), (16, 40), (17, 42)],
        vec![(14, 43), (15, 41), (16, 40), (17, 42)],
    ] {
        let built = build(&[(WEAPON, firearm)], &attachments, true);
        let mut engine = engine_with(built, 0, WEAPON, &slots);
        shoot(&mut engine, 0, 1.0, 90);
        assert_eq!(
            health(&engine),
            Some(400 - 280),
            "{slots:?}: {:?}",
            denial(&engine)
        );
    }
}

#[test]
fn sidearm_suppressor_and_sight_keep_the_host_damage() {
    let firearm = Firearm::new("kWepCategorySideArm", 20.0);
    let attachments = [
        attachment(40, "kAttachTypeSuppressor", true),
        attachment(41, "kAttachTypeSight", true),
    ];
    let built = build(&[(WEAPON, firearm)], &attachments, true);
    let mut engine = engine_with(built, 1, WEAPON, &[(19, 41), (20, 40)]);
    shoot(&mut engine, 1, 1.0, 95);
    assert_eq!(health(&engine), Some(400 - 80), "{:?}", denial(&engine));
}

#[test]
fn unknown_or_foreign_attachments_are_refused_with_the_slot_and_items() {
    let firearm = Firearm::new("kWepCategoryAssaultRifle", 30.0);
    let foreign = Attachment {
        faction: "FactionTypeBlack",
        ..attachment(44, "kAttachTypeForwardGrip", false)
    };
    let attachments = [
        attachment(40, "kAttachTypeSuppressor", true),
        attachment(45, "kAttachTypeLaser", true),
        foreign,
    ];
    for (slots, expected) in [
        (vec![(17, 44)], "unsupported_bullet_attachment_incompatible"),
        (vec![(16, 2)], "unsupported_bullet_attachment_unresolved"),
        (vec![(16, 3)], "unsupported_bullet_attachment_unresolved"),
        (vec![(17, 40)], "unsupported_bullet_attachment_slot"),
        (vec![(15, 40)], "unsupported_bullet_attachment_slot"),
        (vec![(16, 45)], "unsupported_bullet_attachment_type"),
    ] {
        let built = build(&[(WEAPON, firearm)], &attachments, true);
        let mut engine = engine_with(built, 0, WEAPON, &slots);
        shoot(&mut engine, 0, 1.0, 100);
        assert_eq!(health(&engine), Some(400), "{slots:?}");
        let (code, equipment) = denial(&engine).expect("refusal audit");
        assert_eq!(code, expected, "{slots:?}");
        let (slot, item) = slots[0];
        assert_eq!(equipment["gear"], 0);
        assert_eq!(equipment["weapon"], WEAPON);
        assert_eq!(equipment["attachments"][slot.to_string()], item);
    }
}

#[test]
fn firearms_must_occupy_their_own_gear() {
    let built = build(
        &[
            (WEAPON, Firearm::new("kWepCategorySniperRifle", 70.0)),
            (OTHER_WEAPON, Firearm::new("kWepCategorySideArm", 20.0)),
        ],
        &[],
        true,
    );
    let mut engine = engine_with(built, 1, WEAPON, &[]);
    shoot(&mut engine, 1, 1.0, 110);
    assert_eq!(health(&engine), Some(400));
    assert_eq!(denial(&engine).unwrap().0, "unsupported_bullet_weapon_gear");
}

#[test]
fn incomplete_firearms_stay_refused_and_are_named_at_load() {
    let incomplete = Firearm {
        ammo_token: None,
        ..Firearm::new("kWepCategoryMachineGun", 36.0)
    };
    let reversed = Firearm {
        effective: 40.0,
        maximum: 20.0,
        ..Firearm::new("kWepCategorySubMachineGun", 24.0)
    };
    let built = build(&[(WEAPON, incomplete), (OTHER_WEAPON, reversed)], &[], true);
    let items = finish_combat_fixture(built.items, &built.metadata);
    assert_eq!(
        items.derived_bullet_skips(),
        [
            item_catalog::SkippedRetailFirearm {
                item_id: WEAPON,
                reason: "derived_bullet_missing_ammo",
            },
            item_catalog::SkippedRetailFirearm {
                item_id: OTHER_WEAPON,
                reason: "derived_bullet_invalid_range",
            },
        ]
    );
    assert!(
        items
            .retail_combat()
            .unwrap()
            .derived_bullet_ids()
            .is_empty()
    );

    let built = build(&[(WEAPON, incomplete)], &[], true);
    let mut engine = engine_with(built, 0, WEAPON, &[]);
    shoot(&mut engine, 0, 1.0, 120);
    assert_eq!(health(&engine), Some(400));
    let (code, equipment) = denial(&engine).unwrap();
    assert_eq!(code, "unsupported_retail_bullet_weapon");
    assert_eq!(equipment["weapon"], WEAPON);
}

#[test]
fn projected_rows_win_and_another_catalog_source_derives_nothing() {
    let firearms = [(WEAPON, Firearm::new("kWepCategoryAssaultRifle", 30.0))];
    let built = build(&firearms, &[], true);
    let items = finish_combat_fixture(built.items, &built.metadata);
    let combat = items.retail_combat().unwrap();
    assert_eq!(combat.derived_bullet_ids(), [WEAPON]);
    // The projected starter keeps its own row and authored damage.
    assert_eq!(combat.bullet_for_damage(2).unwrap().damage.character, 40.0);
    let derived = combat.bullet_for_damage(WEAPON).unwrap();
    assert_eq!(derived.damage.character, 30.0);
    assert_eq!(derived.ammo.ammo_token_candidate, 500);
    assert!(!derived.ammo.ammo_token_verified_for_retail);
    assert_eq!(derived.magazine_capacity, 8);

    let built = build(&firearms, &[], false);
    let items = finish_combat_fixture(built.items, &built.metadata);
    assert!(
        items
            .retail_combat()
            .unwrap()
            .derived_bullet_ids()
            .is_empty()
    );
    assert!(items.derived_bullet_skips().is_empty());
}

#[test]
fn item_catalog_firearms_keep_the_ordinary_ammo_pool() {
    let built = build(
        &[(WEAPON, Firearm::new("kWepCategoryAssaultRifle", 30.0))],
        &[],
        true,
    );
    let mut engine = engine_with(built, 0, WEAPON, &[]);
    engine
        .active
        .as_mut()
        .unwrap()
        .observed_primary
        .as_mut()
        .unwrap()
        .ammo_pool = 1;
    shoot(&mut engine, 0, 1.0, 130);
    assert_eq!(health(&engine), Some(400));
    assert_eq!(denial(&engine).unwrap().0, "unsupported_bullet_magazine");
}
