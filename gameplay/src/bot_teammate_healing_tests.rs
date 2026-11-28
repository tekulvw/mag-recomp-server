//! SYNTHETIC bot medic checks: a generated catalog with a self-only item and a
//! medkit with an authored recipient sphere. No game data.
use super::*;
use crate::spawn_progression::ready;
use std::sync::Arc;

const FIRST_AID: u16 = 5;
const MEDKIT: u16 = 6;

fn medic_items() -> item_catalog::ItemCatalog {
    let item = |id: u16, name: &str| {
        (
            format!("FactionItemList0:{id}"),
            json!({"authored_ref":format!("FactionItemList0:{id}"),"faction_list":"FactionItemList0",
                "local_ordinal":id,"wire_item_id":null,"name":name,"item_type":"kActiveItem",
                "definition":{"group":"active_items","name":name}}),
        )
    };
    let items = item_catalog::ItemCatalog::from_value_for_application(
        json!({
            "schema_version":2,"application_id":20231,
            "items":serde_json::Map::from_iter([item(FIRST_AID, "first_aid_pack"), item(MEDKIT, "medkit")]),
            "definitions":{"active_items":{"first_aid_pack":{"parameters":{}},"medkit":{"parameters":{}}}},
            "resources":{}
        }),
        blast_rules::CharacterBlastRules {
            armor_rating_reduction: 0.,
            armor_prevention_reduction: 0.,
            hit_location: 3,
            damage_type: 10,
        },
        20231,
    )
    .unwrap();
    let healing = |id: u16, name: &str, sphere: bool| {
        let mut row = json!({"authored_ref":format!("FactionItemList0:{id}"),"name":name,
            "definition":{"group":"active_items","name":name},"source":{},"item_source":{},
            "global_source":{},"cooldown_seconds":7.,"heal_percent":50.,"duration_seconds":2.});
        if sphere {
            row["recipient_sphere"] = json!({"forward_offset":2.,"radius":3.,"source":{
                "file":"active_items.lua","pointer":"/parameters/ActiveItemDataTable",
                "fields":["medkit_heal_range","medkit_heal_radius"],"sha256":"0".repeat(64)}});
        }
        (id.to_string(), row)
    };
    let combat = retail_combat::RetailCombatCatalog::from_json(&json!({
        "schema_version":3,"application_id":20231,"status":"synthetic",
        "authority":{"bullet_damage":true,"armor_reduction":true,"body_damage":true,
            "grenade_damage":true,"grenade_physics":false,"healing_schedule":true,"ammo_tokens":false},
        "runtime_policy":{"projectile_motion":"authenticated_owner_reports",
            "grenade_token_basis":"expected_static_original_resources","projectile_expiry_grace_ms":0,
            "healing_tick_policy":"guaranteed_ticks_with_bounded_catch_up","healing_tick_seconds":1},
        "provenance":{},"bullets":{},"grenades":{},"armor":{},
        "body":(0..16).map(|location|json!({"location":location,"armor_area":0,
            "multiplier":1.,"source":{}})).collect::<Vec<_>>(),
        "healing":serde_json::Map::from_iter([healing(FIRST_AID, "first_aid_pack", false), healing(MEDKIT, "medkit", true)]),
        "unsupported":{},"limitations":["Independent synthetic values; no game tuning"]
    }).to_string()).unwrap();
    items.with_retail_combat(combat)
}

fn reserve(engine: &mut Engine<Vec<u8>>, player: u16) {
    let response = engine.handle(
        Request::Reserve {
            v: BRIDGE_VERSION,
            request_id: format!("synthetic-medic-reserve-{player}"),
            reservation_id: format!("synthetic-medic-{player:04}"),
            world_id: 1,
            player_id: player,
            account_id: u32::from(player) + 1,
            session_key: "aaaaaaaaaaaaaaaa".into(),
            access_key: "bbbbbbbbbbbbbbbb".into(),
            character: serde_json::from_value(json!({"name":format!("Synthetic Medic {player}"),
                "faction":2,"head":0,"voice":0,"universe_id":26,"character_id":u32::from(player)+1}))
            .unwrap(),
        },
        Duration::ZERO,
        1,
    );
    assert!(response.ok, "{:?}", response.error);
}

pub(crate) fn with_item(
    mut kit: replication::CharacterLoadout,
    item: u16,
) -> replication::CharacterLoadout {
    kit.inventories[usize::from(kit.handle.inventory_index)].slots[5] = Some(item);
    kit
}

/// One human, a medic bot at the origin and bots placed by `others`
/// (index offset, team, position, health), all standing on the floor.
pub(crate) fn world(others: &[(u16, u8, [f32; 3], u16)]) -> (Engine<Vec<u8>>, u16) {
    let mut engine = crate::bot_registry::identity_tests::engine("Synthetic Medic");
    reserve(&mut engine, 0);
    ready(engine.active.as_mut().unwrap());
    engine
        .reconcile_bot_population(Duration::ZERO, 1, &mut Default::default())
        .unwrap();
    let medic = *engine.bot_registry.bots.keys().next().unwrap();
    engine.item_catalog = Arc::new(medic_items());
    engine.match_started = true;
    let team = engine.active.as_ref().unwrap().team();
    let squad = replication::SquadAddress::from_index(16 * u16::from(team)).unwrap();
    let template = engine.bot_registry.bots[&medic].clone();
    let place = |b: &mut crate::bot_registry::WorldBot, p: [f32; 3], health: u16| {
        let runtime = b.target.runtime.as_mut().unwrap();
        runtime.position = replication::CharacterPosition {
            x: p[0],
            y: p[1],
            z: p[2],
        };
        let mut motion = bot_physics::Motion::new([p[0].into(), p[1].into(), p[2].into()]);
        motion.grounded = true;
        runtime.physics_motion = Some(motion);
        b.health = health;
        b.damage.health_quarter_points = health;
    };
    let m = engine.bot_registry.bots.get_mut(&medic).unwrap();
    m.target.squad = squad;
    m.target.loadout = with_item(m.target.loadout.clone(), MEDKIT);
    place(m, [0., 0., 0.], 400);
    for &(offset, team, p, health) in others {
        let mut b = template.clone();
        b.target.object_index = medic + offset;
        b.target.squad = replication::SquadAddress::from_index(16 * u16::from(team)).unwrap();
        // No healing item of their own, so only the medic treats them.
        let mut kit = b.target.loadout.clone();
        kit.inventories[usize::from(kit.handle.inventory_index)].slots[5] = None;
        b.target.loadout = kit;
        place(&mut b, p, health);
        engine.bot_registry.bots.insert(medic + offset, b);
    }
    (engine, medic)
}

pub(crate) fn records(engine: &Engine<Vec<u8>>, op: &str) -> Vec<Value> {
    String::from_utf8_lossy(&engine.writer)
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|row| row["op"] == op)
        .collect()
}

#[test]
fn medic_slots_carry_the_medkit_in_place_of_the_self_heal_item() {
    let items = medic_items();
    let engine = crate::bot_registry::identity_tests::engine("Synthetic Kit");
    let kit = with_item(
        engine.item_catalog.bootstrap_loadout(2, 0).unwrap(),
        FIRST_AID,
    );
    let policy = crate::bot_healing::Policy::default();
    let medic = crate::bot_healing::medic_loadout(&policy, 1, &items, kit.clone());
    let slots = |k: &replication::CharacterLoadout| {
        k.inventories[usize::from(k.handle.inventory_index)].slots
    };
    assert_eq!(slots(&medic)[5], Some(MEDKIT));
    assert_eq!(slots(&medic)[0], slots(&kit)[0], "the weapon is unchanged");
    let rifleman = crate::bot_healing::medic_loadout(&policy, 0, &items, kit.clone());
    assert_eq!(slots(&rifleman)[5], Some(FIRST_AID));
    let off = crate::bot_healing::Policy {
        teammates: false,
        ..Default::default()
    };
    assert_eq!(
        slots(&crate::bot_healing::medic_loadout(&off, 1, &items, kit))[5],
        Some(FIRST_AID)
    );
}

#[test]
fn a_medic_treats_injured_teammates_in_its_sphere_only() {
    let own = world(&[]).0.active.as_ref().unwrap().team();
    let (mut engine, medic) = world(&[
        (1, own, [2., 0., 0.], 160),     // injured teammate in reach
        (2, 1 - own, [2., 0., 1.], 160), // injured enemy in reach
        (3, own, [9., 0., 0.], 160),     // injured teammate out of reach
        (4, own, [1., 0., 1.], 380),     // nearly healthy teammate
    ]);
    // The human teammate stands inside the sphere too.
    let human = engine.active.as_mut().unwrap();
    human.life = CharacterLife::Alive {
        generation: 3,
        health_quarter_points: 120,
    };
    human.tracking.server_position(
        replication::CharacterPosition {
            x: 2.,
            y: 0.,
            z: -1.,
        },
        player_tracking::PositionOrigin::ServerSpawn,
        0,
    );
    engine
        .tick_bot_healing(Duration::from_millis(1000), 1)
        .unwrap();
    let started = records(&engine, "bot_teammate_healing_started");
    assert_eq!(started.len(), 1, "{started:?}");
    assert_eq!(started[0]["object_index"], medic);
    let bots = &engine.bot_registry.bots;
    assert!(bots[&(medic + 1)].received_healing.is_some());
    assert!(bots[&(medic + 2)].received_healing.is_none(), "enemy");
    assert!(
        bots[&(medic + 3)].received_healing.is_none(),
        "out of reach"
    );
    assert!(
        bots[&(medic + 4)].received_healing.is_none(),
        "above threshold"
    );
    assert!(
        engine.active.as_ref().unwrap().healing.is_some(),
        "human teammate"
    );
    // The treatment ticks the teammate's health up, then the medkit cools down.
    let before = engine.bot_registry.bots[&(medic + 1)].health;
    engine
        .tick_bot_healing(Duration::from_millis(2100), 1)
        .unwrap();
    assert!(engine.bot_registry.bots[&(medic + 1)].health > before);
    engine
        .tick_bot_healing(Duration::from_millis(3000), 1)
        .unwrap();
    assert_eq!(records(&engine, "bot_teammate_healing_started").len(), 1);
}

#[test]
fn a_fighting_medic_only_treats_a_badly_hurt_teammate() {
    let own = world(&[]).0.active.as_ref().unwrap().team();
    let (mut engine, medic) = world(&[(1, own, [2., 0., 0.], 250)]);
    engine.active.as_mut().unwrap().life = CharacterLife::AwaitingFirstSpawn;
    let enemy = medic + 1;
    engine
        .bot_registry
        .bots
        .get_mut(&medic)
        .unwrap()
        .firearm
        .set_target_for_test(enemy);
    engine
        .tick_bot_healing(Duration::from_millis(1000), 1)
        .unwrap();
    assert!(records(&engine, "bot_teammate_healing_started").is_empty());
    let b = engine.bot_registry.bots.get_mut(&(medic + 1)).unwrap();
    b.health = 120;
    b.damage.health_quarter_points = 120;
    engine
        .tick_bot_healing(Duration::from_millis(1050), 1)
        .unwrap();
    assert_eq!(records(&engine, "bot_teammate_healing_started").len(), 1);
}

#[test]
fn a_medic_revives_a_downed_human_teammate() {
    let (mut engine, medic) = world(&[]);
    engine = engine
        .with_incapacitation_rules(crate::incapacitation_tests::rules())
        .unwrap();
    let human = engine.active.as_mut().unwrap();
    human.life = CharacterLife::Incapacitated {
        generation: 3,
        downed: incapacitation::Downed {
            deadline: Duration::from_secs(10),
            accumulated_damage_hp: 0.,
        },
    };
    human.tracking.server_position(
        replication::CharacterPosition {
            x: 2.,
            y: 0.,
            z: 0.,
        },
        player_tracking::PositionOrigin::ServerSpawn,
        0,
    );
    let messages = engine
        .tick_bot_healing(Duration::from_millis(1000), 1)
        .unwrap();
    let human = engine.active.as_ref().unwrap();
    assert!(
        matches!(
            human.life,
            CharacterLife::Alive {
                generation: 3,
                health_quarter_points: 100
            }
        ),
        "a quarter of the reviver's 100 HP"
    );
    assert_eq!(messages.len(), 1, "the squad hears the resuscitation");
    let started = records(&engine, "bot_teammate_healing_started");
    assert_eq!(started[0]["patients"][0]["kind"], "revived");
    assert_eq!(started[0]["object_index"], medic);
}
