//! Synthetic bot kit catalogs: every item, name and value below is invented.
use super::*;
use crate::item_catalog::ItemCatalog;
use serde_json::json;
use std::sync::Arc;

const RIFLE: u16 = 20;
const SMG: u16 = 21;
const PISTOL: u16 = 22;
const KNIFE: u16 = 23;
const LIGHT: u16 = 30;
const MEDIUM: u16 = 31;
const LIGHT_UNIFORM: u16 = 40;
const MEDIUM_UNIFORM: u16 = 41;
const HELMET: u16 = 42;
const GOGGLES: u16 = 43;
const LIGHT_UNIFORM_B: u16 = 44;
const SIGHT: u16 = 50;
const SUPPRESSOR: u16 = 51;
const GRIP: u16 = 52;
const PISTOL_SIGHT: u16 = 53;
const LAUNCHER_SIGHT: u16 = 54;
const FIRST_AID: u16 = 60;
const MEDKIT: u16 = 61;
const REPAIR: u16 = 62;
const FRAG: u16 = 63;
const LAUNCHER: u16 = 64;

fn item(raw: &mut Value, id: u16, name: &str, item_type: &str, group: &str, parameters: Value) {
    let reference = format!("FactionItemList0:{id}");
    raw["items"][&reference] = json!({"authored_ref":reference,"faction_list":"FactionItemList0",
        "local_ordinal":id,"wire_item_id":null,"name":name,"item_type":item_type,
        "definition":{"group":group,"name":name}});
    raw["definitions"][group][name] = json!({ "parameters": parameters });
}

fn firearm(raw: &mut Value, id: u16, name: &str, category: &str, attachments: &[&str]) {
    let list: serde_json::Map<String, Value> = attachments
        .iter()
        .enumerate()
        .map(|(i, a)| {
            (
                format!("WeaponAttachment{i}"),
                json!({"WeaponAttachment":{"name":a}}),
            )
        })
        .collect();
    let ammo = format!("{name}_round");
    item(
        raw,
        id,
        name,
        "kWeapon",
        "weapons",
        json!({
        "id":name,"category":category,"ammo_name":ammo,"faction":"FactionTypeGreen",
        "effective_range":30.,"max_range":60.,"magazine_capacity":12,"initial_magazine_count":3,
        "infinite_ammo":false,"initial_fire_mode":"kSemiAuto",
        "menu_selection_slot":"kSelectionMenuSlotWeapons",
        "fire_mode":{"refire_time":0.2,"reload_time":2.0,"fire_modes":{
            "FireModeTypeClass0":{"FireModeTypeClass":{"fire_mode_value":"kSemiAuto"}}}},
        "attachment_list":list,
        "reticule_params_stand":{
            "KnockAdd":0,"KnockFire":0.,"KnockReturn":1.,"SideKnockFire":0.,"SideKnockReturn":1.,
            "SpreadFire":0.,"SpreadReturn":1.,"SpreadMove":0.,"SpreadRotate":0.,"MoveSpreadReturn":1.,
            "cone_width_range":[0.,0.],"pitch_offset_range":[0.,0.],"yaw_offset_range":[0.,0.],
            "SpreadNormalization":0,"WidthCombineOp":"kWCO_Add"}}),
    );
    raw["definitions"]["hitscan_ammo"][&ammo] = json!({"parameters":{"damage":{
        "character":20.,"characterAPierce":0.,"characterARedux":0.,"vehicle":0.,"vehicleAPierce":0.,
        "vehicleARedux":0.,"objective":0.,"objectiveAPierce":0.,"objectiveARedux":0.,"mine":0.}},
        "native_token_u32":500 + u64::from(id)});
}

fn attachment(raw: &mut Value, id: u16, name: &str, kind: &str) {
    item(
        raw,
        id,
        name,
        "kAttachmentItem",
        "attachments",
        json!({"id":name,"attachment_type":kind,
        "weapon_name":"","character_attachment_type":"kCharAttachTypePrimary","faction":"FactionTypeGreen"}),
    );
}

/// An invented retail-shaped item catalog with Armory and combat metadata.
pub(crate) fn items() -> Arc<ItemCatalog> {
    use sha2::{Digest, Sha256};
    let mut raw = json!({"schema_version":2,"application_id":20231,"items":{},"definitions":{
        "weapons":{},"hitscan_ammo":{},"attachments":{},"active_items":{},"armor_items":{},
        "uniform_items":{},"simple_items":{}},"resources":{}});
    firearm(
        &mut raw,
        RIFLE,
        "invented_rifle",
        "kWepCategoryAssaultRifle",
        &["invented_sight", "invented_suppressor", "invented_grip"],
    );
    firearm(
        &mut raw,
        SMG,
        "invented_smg",
        "kWepCategorySubMachineGun",
        &["invented_sight"],
    );
    firearm(
        &mut raw,
        PISTOL,
        "invented_pistol",
        "kWepCategorySideArm",
        &["invented_pistol_sight"],
    );
    item(
        &mut raw,
        KNIFE,
        "invented_knife",
        "kWeapon",
        "weapons",
        json!({"id":"invented_knife","category":"kWepCategoryKnife"}),
    );
    item(
        &mut raw,
        LAUNCHER,
        "invented_launcher",
        "kWeapon",
        "weapons",
        json!({"id":"invented_launcher","category":"kWepCategoryRocketLauncher",
            "required_sight_attachment":"invented_launcher_sight","magazine_capacity":1}),
    );
    attachment(&mut raw, SIGHT, "invented_sight", "kAttachTypeSight");
    attachment(
        &mut raw,
        SUPPRESSOR,
        "invented_suppressor",
        "kAttachTypeSuppressor",
    );
    attachment(&mut raw, GRIP, "invented_grip", "kAttachTypeForwardGrip");
    attachment(
        &mut raw,
        PISTOL_SIGHT,
        "invented_pistol_sight",
        "kAttachTypeSight",
    );
    attachment(
        &mut raw,
        LAUNCHER_SIGHT,
        "invented_launcher_sight",
        "kAttachTypeSight",
    );
    for (id, name) in [(LIGHT, "invented_light"), (MEDIUM, "invented_medium")] {
        item(
            &mut raw,
            id,
            name,
            "kArmorItem",
            "armor_items",
            json!({"id":name}),
        );
    }
    for (id, name, armor) in [
        (LIGHT_UNIFORM, "invented_light_uniform", "invented_light"),
        (
            LIGHT_UNIFORM_B,
            "invented_light_uniform_b",
            "invented_light",
        ),
        (MEDIUM_UNIFORM, "invented_medium_uniform", "invented_medium"),
    ] {
        item(
            &mut raw,
            id,
            name,
            "kUniformItem",
            "uniform_items",
            json!({"id":name,"armor_dependency":armor}),
        );
    }
    for (id, name, kind) in [
        (HELMET, "invented_helmet", "kCharAttachTypeApparelHead"),
        (GOGGLES, "invented_goggles", "kCharAttachTypeApparelFace"),
    ] {
        item(
            &mut raw,
            id,
            name,
            "kSimpleItem",
            "simple_items",
            json!({"id":name,
            "simpleitem_type":"kSimpleItemApparel","character_attachment_type":kind,
            "valid_armor_list":[{"name":"invented_medium"}]}),
        );
    }
    for (id, name) in [
        (FIRST_AID, "first_aid_pack"),
        (MEDKIT, "medkit"),
        (REPAIR, "repairkit"),
    ] {
        item(
            &mut raw,
            id,
            name,
            "kActiveItem",
            "active_items",
            json!({"id":name,"menu_selection_slot":"kSelectionMenuSlotEquipment"}),
        );
    }
    item(
        &mut raw,
        FRAG,
        "invented_frag",
        "kWeapon",
        "weapons",
        json!({"id":"invented_frag",
        "category":"kWepCategoryGrenade","menu_selection_slot":"kSelectionMenuSlotEquipment"}),
    );
    let bytes = serde_json::to_vec(&raw).unwrap();
    let dir = std::env::temp_dir().join(format!(
        "mag-synthetic-bot-kits-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("blast.json"),
        json!({"schema_version":1,"applications":[20230,20231],
        "character_damage":{"armor_rating_reduction":0.0,"armor_prevention_reduction":0.0,
        "hit_location":3,"damage_type":10}})
        .to_string(),
    )
    .unwrap();
    std::fs::write(dir.join("items.json"), &bytes).unwrap();
    let catalog =
        ItemCatalog::load_for_application(dir.join("items.json"), dir.join("blast.json"), 20231);
    std::fs::remove_dir_all(&dir).unwrap();
    let catalog = catalog.unwrap();
    let healing = |id: u16, name: &str, sphere: bool| {
        let mut row = json!({"authored_ref":format!("FactionItemList0:{id}"),"name":name,
            "definition":{"group":"active_items","name":name},"source":{},"item_source":{},
            "global_source":{},"cooldown_seconds":5.,"heal_percent":40.,"duration_seconds":2.});
        if sphere {
            row["recipient_sphere"] = json!({"forward_offset":1.,"radius":2.,"source":{
                "file":"active_items.lua","pointer":"/parameters/ActiveItemDataTable",
                "fields":["medkit_heal_range","medkit_heal_radius"],"sha256":"0".repeat(64)}});
        }
        (id.to_string(), row)
    };
    let armor = |id: u16, name: &str| {
        (
            id.to_string(),
            json!({"authored_ref":format!("FactionItemList0:{id}"),"name":name,
            "definition":{},"source":{},"item_source":{},"armor_type":1,"rating":0.1,
            "prevention":0.1,"coverage":[0.5,0.5,0.5,0.5],"armor_type_source":{}}),
        )
    };
    let combat = crate::retail_combat::RetailCombatCatalog::from_json(&json!({
        "schema_version":3,"application_id":20231,"status":"synthetic",
        "authority":{"bullet_damage":true,"armor_reduction":true,"body_damage":true,
            "grenade_damage":true,"grenade_physics":false,"healing_schedule":true,"ammo_tokens":false},
        "runtime_policy":{"projectile_motion":"authenticated_owner_reports",
            "grenade_token_basis":"expected_static_original_resources","projectile_expiry_grace_ms":0,
            "healing_tick_policy":"guaranteed_ticks_with_bounded_catch_up","healing_tick_seconds":1},
        "provenance":{"catalog_sha256":format!("{:x}", Sha256::digest(&bytes))},
        "bullets":{},"grenades":{},
        "armor":serde_json::Map::from_iter([armor(LIGHT, "invented_light"), armor(MEDIUM, "invented_medium")]),
        "body":(0..16).map(|location|json!({"location":location,"armor_area":location % 4,
            "multiplier":1.,"source":{}})).collect::<Vec<_>>(),
        "healing":serde_json::Map::from_iter([healing(FIRST_AID, "first_aid_pack", false), healing(MEDKIT, "medkit", true)]),
        "unsupported":{},"limitations":["Invented synthetic values"]
    }).to_string()).unwrap();
    let names: BTreeMap<u16, (&str, Option<u8>)> = BTreeMap::from([
        (RIFLE, ("invented_rifle", Some(12))),
        (SMG, ("invented_smg", Some(12))),
        (PISTOL, ("invented_pistol", Some(12))),
        (KNIFE, ("invented_knife", Some(1))),
        (LAUNCHER, ("invented_launcher", Some(1))),
        (LIGHT, ("invented_light", None)),
        (MEDIUM, ("invented_medium", None)),
        (LIGHT_UNIFORM, ("invented_light_uniform", None)),
        (LIGHT_UNIFORM_B, ("invented_light_uniform_b", None)),
        (MEDIUM_UNIFORM, ("invented_medium_uniform", None)),
        (HELMET, ("invented_helmet", None)),
        (GOGGLES, ("invented_goggles", None)),
        (SIGHT, ("invented_sight", None)),
        (SUPPRESSOR, ("invented_suppressor", None)),
        (GRIP, ("invented_grip", None)),
        (PISTOL_SIGHT, ("invented_pistol_sight", None)),
        (LAUNCHER_SIGHT, ("invented_launcher_sight", None)),
        (FIRST_AID, ("first_aid_pack", None)),
        (MEDKIT, ("medkit", None)),
        (REPAIR, ("repairkit", None)),
        (FRAG, ("invented_frag", Some(1))),
    ]);
    let armory = crate::retail_armory::RetailArmory::from_json(&json!({
        "schema_version":1,"application_id":20231,"source_sha256":{"synthetic":"a".repeat(64)},
        "items":names.iter().map(|(&id, &(name, ammo))| json!({"wire_id":id,"name":name,
            "authored_ref":format!("FactionItemList0:{id}"),"available_bit":id,"factions":[0,1,2],
            "primary_ammo":ammo})).collect::<Vec<_>>(),
        "factions":(0..3).map(|faction|json!({"faction":faction,"available_item_bits":[],"skill_ids":[],
            "loadouts":(0..5).map(|_|json!({"update_id":1,"items":vec![None::<u16>;22]})).collect::<Vec<_>>()
        })).collect::<Vec<_>>()
    }).to_string()).unwrap();
    Arc::new(
        catalog
            .with_retail_armory(armory)
            .with_retail_combat(combat),
    )
}

fn kit(id: &str, role: &str, primary: u16, armor: u16, gear: [Option<u16>; 2]) -> Value {
    let mut items = vec![None::<u16>; 22];
    items[slot::PRIMARY] = Some(primary);
    items[slot::BACKUP] = Some(PISTOL);
    items[slot::SWIPE] = Some(KNIFE);
    items[slot::GEAR[0]] = gear[0];
    items[slot::GEAR[1]] = gear[1];
    items[slot::ARMOR] = Some(armor);
    items[slot::PRIMARY_SIGHT] = Some(SIGHT);
    items[slot::BACKUP_SIGHT] = Some(PISTOL_SIGHT);
    json!({"id":id,"role":role,"items":items})
}

pub(crate) fn document() -> Value {
    let mut suppressed = kit(
        "rifle-quiet",
        "rifleman",
        RIFLE,
        MEDIUM,
        [Some(FRAG), Some(FIRST_AID)],
    );
    suppressed["items"][slot::PRIMARY_MUZZLE] = json!(SUPPRESSOR);
    suppressed["items"][slot::PRIMARY_UNDERBARREL] = json!(GRIP);
    let mut rocket = kit(
        "rocket",
        "anti_armor",
        SMG,
        MEDIUM,
        [Some(FRAG), Some(FIRST_AID)],
    );
    rocket["items"][slot::HEAVY] = json!(LAUNCHER);
    rocket["items"][slot::HEAVY_SIGHT] = json!(LAUNCHER_SIGHT);
    let kits = vec![
        kit(
            "rifle",
            "rifleman",
            RIFLE,
            MEDIUM,
            [Some(FRAG), Some(FIRST_AID)],
        ),
        suppressed,
        kit(
            "rifle-light",
            "rifleman",
            RIFLE,
            LIGHT,
            [Some(FIRST_AID), None],
        ),
        kit(
            "smg",
            "close_quarters",
            SMG,
            LIGHT,
            [Some(FRAG), Some(FIRST_AID)],
        ),
        rocket,
        kit("medic", "medic", SMG, MEDIUM, [Some(MEDKIT), Some(FRAG)]),
        kit(
            "engineer",
            "engineer",
            RIFLE,
            MEDIUM,
            [Some(REPAIR), Some(FIRST_AID)],
        ),
    ];
    json!({"format":FORMAT,"application_id":20231,"provenance":{"source":"synthetic"},
        "heads":(0..15).collect::<Vec<u8>>(),"voices":(0..9).collect::<Vec<u8>>(),
        "apparel_head_per_mille":500,"apparel_face_per_mille":500,
        "roles":[{"role":"rifleman","weight":6},{"role":"close_quarters","weight":3,"max_per_squad":1},
            {"role":"anti_armor","weight":2,"max_per_squad":1}],
        "factions":(0..3).map(|faction| json!({"faction":faction,"kits":kits,"apparel":[
            {"armor":LIGHT,"uniforms":[LIGHT_UNIFORM, LIGHT_UNIFORM_B]},
            {"armor":MEDIUM,"uniforms":[MEDIUM_UNIFORM],"head":[HELMET],"face":[GOGGLES]}]}))
            .collect::<Vec<_>>()})
}

pub(crate) fn catalog() -> Catalog {
    Catalog::from_json(&serde_json::to_vec(&document()).unwrap()).unwrap()
}

type Edit = Box<dyn FnOnce(&mut Value)>;

fn refusal(edit: impl FnOnce(&mut Value)) -> String {
    let mut doc = document();
    edit(&mut doc);
    match Catalog::from_json(&serde_json::to_vec(&doc).unwrap()) {
        Err(e) => e.to_string(),
        Ok(catalog) => match catalog.validate_items(&items()) {
            Err(e) => e.to_string(),
            Ok(()) => "accepted".into(),
        },
    }
}

fn request<'a>(name_index: u16, salt: &'a str, squad_roles: &'a [Role]) -> Request<'a> {
    Request {
        faction: 0,
        name_index,
        salt,
        assigned: None,
        squad_roles,
    }
}

#[test]
fn synthetic_catalog_validates_against_items_and_combat_rules() {
    let catalog = catalog();
    assert_eq!(catalog.kit_count(), 21);
    catalog.validate_items(&items()).unwrap();
}

#[test]
fn refuses_kits_the_armory_or_bots_could_not_use() {
    let k = |doc: &mut Value, index: usize| doc["factions"][0]["kits"][index].clone();
    let cases: Vec<(&str, Edit)> = vec![
        (
            "invalid_bot_kit_heads",
            Box::new(|d| d["heads"] = json!([0, 15])),
        ),
        (
            "invalid_bot_kit_voices",
            Box::new(|d| d["voices"] = json!([9])),
        ),
        (
            "invalid_bot_kit_voices",
            Box::new(|d| d["voices"] = json!([1, 1])),
        ),
        (
            "invalid_bot_kit_role_weights",
            Box::new(|d| d["roles"][0]["role"] = json!("medic")),
        ),
        (
            "invalid_bot_kit_reserved_slot",
            Box::new(|d| d["factions"][0]["kits"][0]["items"][13] = json!(MEDIUM_UNIFORM)),
        ),
        (
            "bot_kit_faction_missing_roles",
            Box::new(|d| {
                d["factions"][0]["kits"]
                    .as_array_mut()
                    .unwrap()
                    .retain(|k| k["role"] != "engineer")
            }),
        ),
        (
            "bot_kit_uniform_armor_mismatch",
            Box::new(|d| d["factions"][1]["apparel"][1]["uniforms"] = json!([LIGHT_UNIFORM])),
        ),
        (
            "bot_kit_apparel_armor_mismatch",
            Box::new(|d| d["factions"][1]["apparel"][0]["head"] = json!([HELMET])),
        ),
        (
            "bot_kit_apparel_armor_mismatch",
            Box::new(|d| d["factions"][1]["apparel"][1]["head"] = json!([GOGGLES])),
        ),
        (
            "bot_kit_medic_item_role",
            Box::new(|d| d["factions"][2]["kits"][0]["items"][4] = json!(MEDKIT)),
        ),
        (
            "bot_kit_repair_item_role",
            Box::new(|d| d["factions"][2]["kits"][0]["items"][4] = json!(REPAIR)),
        ),
        (
            "bot_kit_without_healing",
            Box::new(|d| d["factions"][2]["kits"][0]["items"][5] = Value::Null),
        ),
        (
            "bot_kit_sight_slot",
            Box::new(|d| d["factions"][0]["kits"][0]["items"][14] = json!(SUPPRESSOR)),
        ),
        (
            "bot_kit_muzzle_slot",
            Box::new(|d| d["factions"][0]["kits"][0]["items"][16] = json!(GRIP)),
        ),
        (
            "bot_kit_heavy_slot",
            Box::new(|d| d["factions"][0]["kits"][4]["items"][21] = Value::Null),
        ),
        (
            "bot_kit_heavy_slot",
            Box::new(|d| d["factions"][0]["kits"][4]["items"][21] = json!(SIGHT)),
        ),
        (
            "bot_kit_gear_slot",
            Box::new(|d| d["factions"][0]["kits"][0]["items"][4] = json!(SIGHT)),
        ),
        (
            "bot_kit_swipe_not_knife",
            Box::new(|d| d["factions"][0]["kits"][0]["items"][2] = json!(FRAG)),
        ),
        (
            "bot_kit_item_not_offered",
            Box::new(|d| d["factions"][0]["kits"][0]["items"][4] = json!(99)),
        ),
        (
            "unsupported_bullet_weapon_gear",
            Box::new(|d| d["factions"][0]["kits"][0]["items"][0] = json!(PISTOL)),
        ),
        (
            "invalid_bot_kit_id",
            Box::new(move |d| {
                let first = k(d, 0);
                d["factions"][0]["kits"].as_array_mut().unwrap().push(first)
            }),
        ),
    ];
    for (expected, edit) in cases {
        assert_eq!(refusal(edit), expected);
    }
    assert_eq!(refusal(|_| {}), "accepted");
}

#[test]
fn bots_draw_varied_valid_kits_and_looks_deterministically() {
    let catalog = catalog();
    let items = items();
    let mut kits = BTreeSet::new();
    let mut heads = BTreeSet::new();
    let mut voices = BTreeSet::new();
    let mut uniforms = BTreeSet::new();
    let mut apparel = 0;
    for name in 0..64 {
        let a = catalog
            .choose(&items, request(name, "formation-a", &[]))
            .unwrap();
        let again = catalog
            .choose(&items, request(name, "formation-a", &[]))
            .unwrap();
        assert_eq!(a.kit_id, again.kit_id);
        assert_eq!(
            a.loadout.inventories[0].slots,
            again.loadout.inventories[0].slots
        );
        assert_eq!((a.head, a.voice), (again.head, again.voice));
        // Face and voice follow the bot, not the match.
        let other = catalog
            .choose(&items, request(name, "formation-b", &[]))
            .unwrap();
        assert_eq!((a.head, a.voice), (other.head, other.voice));
        let row = a.loadout.inventories[0].slots;
        assert!(!a.role.assigned());
        assert_eq!(a.loadout.primary.ammo_count, 12);
        assert!(row[slot::UNIFORM].is_some());
        apparel += usize::from(row[slot::APPAREL_HEAD].is_some());
        uniforms.insert(row[slot::UNIFORM]);
        kits.insert(a.kit_id);
        heads.insert(a.head);
        voices.insert(a.voice);
        crate::bot_firearms::validate_weapon_recoil(&items, &a.loadout).unwrap();
    }
    assert_eq!(kits.len(), 5, "every drawn kit appears: {kits:?}");
    assert!(
        heads.len() >= 10 && voices.len() >= 7,
        "{heads:?} {voices:?}"
    );
    assert_eq!(uniforms.len(), 3);
    assert!(apparel > 0);
    let changed = (0..64)
        .filter(|&name| {
            catalog
                .choose(&items, request(name, "formation-a", &[]))
                .unwrap()
                .kit_id
                != catalog
                    .choose(&items, request(name, "formation-b", &[]))
                    .unwrap()
                    .kit_id
        })
        .count();
    assert!(changed > 0, "another match reshuffles kits");
}

#[test]
fn role_slots_get_their_duty_kits_and_squad_caps_hold() {
    let catalog = catalog();
    let items = items();
    for name in 0..16 {
        for (role, tool) in [(Role::Medic, MEDKIT), (Role::Engineer, REPAIR)] {
            let chosen = catalog
                .choose(
                    &items,
                    Request {
                        assigned: Some(role),
                        ..request(name, "m", &[])
                    },
                )
                .unwrap();
            assert_eq!(chosen.role, role);
            assert!(chosen.loadout.inventories[0].slots.contains(&Some(tool)));
        }
        let full = [Role::CloseQuarters, Role::AntiArmor];
        let capped = catalog.choose(&items, request(name, "m", &full)).unwrap();
        assert_eq!(capped.role, Role::Rifleman);
    }
}

#[test]
fn bot_population_spawns_carry_kits_and_appearance() {
    let mut engine = crate::bot_registry::identity_tests::engine("Synthetic Kits");
    engine.item_catalog = items();
    engine.install_bot_kits(Some(Arc::new(catalog()))).unwrap();
    engine
        .reconcile_bot_population(std::time::Duration::ZERO, 1, &mut Default::default())
        .unwrap();
    assert_eq!(engine.bot_registry.bots.len(), 2);
    let log = String::from_utf8(engine.writer.clone()).unwrap();
    let spawns: Vec<Value> = log
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|e| e["op"] == "bot_population_spawn")
        .collect();
    assert_eq!(spawns.len(), 2);
    for spawn in &spawns {
        let bot = &engine.bot_registry.bots[&(spawn["object_index"].as_u64().unwrap() as u16)];
        let kit = bot.kit.as_ref().unwrap();
        assert_eq!(spawn["kit"], json!(kit.id));
        assert_eq!(spawn["head"], json!(kit.head));
        assert_eq!(spawn["voice"], json!(kit.voice));
        let row = bot.target.loadout.inventories[0].slots;
        assert!([RIFLE, SMG].contains(&row[0].unwrap()));
        assert!(row[slot::UNIFORM].is_some());
        assert_eq!(
            bot.appearance(),
            crate::dummy::Appearance {
                head: kit.head,
                voice: kit.voice
            }
        );
    }
}

#[test]
fn bots_without_a_kit_catalog_keep_the_bootstrap_kit() {
    let mut engine = crate::bot_registry::identity_tests::engine("Synthetic Plain");
    engine
        .reconcile_bot_population(std::time::Duration::ZERO, 1, &mut Default::default())
        .unwrap();
    let log = String::from_utf8(engine.writer.clone()).unwrap();
    assert!(log.contains("bot_population_spawn") && !log.contains("\"kit\""));
    for bot in engine.bot_registry.bots.values() {
        assert!(bot.kit.is_none());
        assert_eq!(bot.appearance(), crate::dummy::Appearance::default());
        assert_eq!(bot.target.loadout.inventories[0].slots[0], Some(2));
    }
}
