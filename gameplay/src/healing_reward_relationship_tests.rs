//! Synthetic healing ticks: actual attacker relationships affect credit, never HP.
use super::*;
use crate::spawn_progression::{dispatch, ready};
use std::sync::Arc;

fn reserve(engine: &mut Engine<Vec<u8>>, player: u16) {
    let response = engine.handle(
        Request::Reserve {
            v: BRIDGE_VERSION,
            request_id: format!("synthetic-reserve-{player}"),
            reservation_id: format!("synthetic-reservation-{player:04}"),
            world_id: 1,
            player_id: player,
            account_id: u32::from(player) + 1,
            session_key: "aaaaaaaaaaaaaaaa".into(),
            access_key: "bbbbbbbbbbbbbbbb".into(),
            character: serde_json::from_value(json!({
                "name":format!("Synthetic {player}"),"faction":2,"head":0,
                "voice":0,"universe_id":26,"character_id":u32::from(player)+1
            }))
            .unwrap(),
        },
        Duration::ZERO,
        1,
    );
    assert!(response.ok, "{:?}", response.error);
}

fn effect() -> healing::HealingEffect {
    let items = item_catalog::ItemCatalog::from_value_for_application(
        json!({
            "schema_version":2,"application_id":20231,
            "items":{"FactionItemList0:2":{"authored_ref":"FactionItemList0:2",
                "faction_list":"FactionItemList0","local_ordinal":2,"wire_item_id":null,
                "name":"synthetic_heal","item_type":"kActiveItem",
                "definition":{"group":"active_items","name":"synthetic_heal"}}},
            "definitions":{"active_items":{"synthetic_heal":{"parameters":{}}}},"resources":{}
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
        "healing":{"2":{"authored_ref":"FactionItemList0:2","name":"synthetic_heal",
            "definition":{},"source":{},"item_source":{},"global_source":{},
            "cooldown_seconds":1.,"heal_percent":10.,"duration_seconds":1.}},
        "unsupported":{},"limitations":["Independent synthetic values; no game tuning"]
    }).to_string()).unwrap();
    healing::HealingEffect::new_with_catalog(
        healing::HealItem::Retail(2),
        4,
        Duration::ZERO,
        &items.with_retail_combat(combat),
    )
    .unwrap()
}

fn fixture() -> (Engine<Vec<u8>>, u16) {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut engine = super::identity_tests::engine(&format!("Synthetic Healing {id}"));
    engine.maximum_players = 4;
    reserve(&mut engine, 0);
    ready(engine.active.as_mut().unwrap());
    engine
        .reconcile_bot_population(Duration::ZERO, 1, &mut Default::default())
        .unwrap();
    let bot = *engine.bot_registry.bots.keys().next().unwrap();
    // Retain the allocated bot while exercising the human healing event path.
    reserve(&mut engine, 2);
    reserve(&mut engine, 3);
    let recipient = engine.peers.remove("synthetic-reservation-0002").unwrap();
    let attacker = engine.active.replace(recipient).unwrap();
    engine
        .peers
        .insert(attacker.reservation.id.clone(), attacker);
    let squad = Arc::new(engine.spawn_catalog.as_ref().unwrap().for_squad(0).unwrap());
    for actor in engine.peers.values_mut().chain(engine.active.iter_mut()) {
        ready(actor);
        actor.squad_catalog = Arc::clone(&squad);
        actor.life = CharacterLife::Alive {
            generation: 4,
            health_quarter_points: 160,
        };
        actor.pending_shared.clear();
    }
    let healer = engine.peers.get_mut("synthetic-reservation-0000").unwrap();
    healer.reservation.character.account_id = Some("synthetic-healer".into());
    let reward = progression::RewardRecipient::from_active(healer).unwrap();
    engine.active.as_mut().unwrap().healing = Some(progression::HealingSession {
        effect: effect(),
        healer: Some(reward),
        effect_id: "synthetic-heal-tick".into(),
    });
    (engine, bot)
}

fn remembered(engine: &mut Engine<Vec<u8>>, attacker: u32, source: u32) {
    engine.active.as_mut().unwrap().last_damage = replication::CharacterDamage {
        health_quarter_points: 160,
        conditions: 0,
        last_location_and_type: 0,
        last_attacker: attacker,
        last_source_object: source,
        last_user_data: 0,
    };
}

fn tick(engine: &mut Engine<Vec<u8>>) -> Result<Vec<OutboundMessage>, &'static str> {
    let wire = replication::ObjectUpdate {
        header: replication::UpdateHeader {
            sequence: engine.active.as_ref().unwrap().latest_owner_sequence + 1,
            nak_sequence: 0,
            client_index: 0,
            object_type_mask: 0,
            timestamp_ms: 1000,
        },
        wire: &[],
    }
    .encode_empty_with_profile(engine.wire_profile)
    .unwrap();
    dispatch(engine, &wire, 1000)
}

fn assert_tick(engine: &mut Engine<Vec<u8>>, credited: bool) {
    tick(engine).unwrap();
    assert_eq!(
        engine
            .active
            .as_ref()
            .unwrap()
            .last_damage
            .health_quarter_points,
        200
    );
    assert!(matches!(
        engine.active.as_ref().unwrap().life,
        CharacterLife::Alive {
            generation: 4,
            health_quarter_points: 200
        }
    ));
    let rewards = engine.pending_reward_batch();
    assert_eq!(rewards.len(), usize::from(credited));
    if credited {
        assert_eq!(rewards[0].kind, progression::RewardKind::TeammateHealed);
        assert_eq!(rewards[0].units, 10);
        assert_eq!(rewards[0].account_id, "synthetic-healer");
    }
    tick(engine).unwrap();
    assert_eq!(
        engine.pending_reward_batch(),
        rewards,
        "a repeated tick cannot credit twice"
    );
    assert_eq!(
        engine
            .active
            .as_ref()
            .unwrap()
            .last_damage
            .health_quarter_points,
        200
    );
}

#[test]
fn healing_credit_uses_human_attacker_team_and_world_not_faction_or_source_object() {
    for (team, world, credited) in [(0, 1, false), (1, 1, true), (0, 2, true)] {
        let (mut engine, _) = fixture();
        let squad = Arc::new(
            engine
                .spawn_catalog
                .as_ref()
                .unwrap()
                .for_squad(team * 16)
                .unwrap(),
        );
        let attacker = engine.peers.get_mut("synthetic-reservation-0003").unwrap();
        attacker.squad_catalog = squad;
        attacker.reservation.world = world;
        assert_eq!(attacker.reservation.character.faction, 2);
        remembered(&mut engine, 0x0380_0003, 0x0380_0000);
        assert_tick(&mut engine, credited);
    }
}

#[test]
fn healing_credit_resolves_retained_dead_registry_attacker_actual_team() {
    for (team, credited) in [(0, false), (1, true)] {
        let (mut engine, id) = fixture();
        let bot = engine.bot_registry.bots.get_mut(&id).unwrap();
        bot.target.squad = replication::SquadAddress::from_index(team * 16).unwrap();
        bot.health = 0;
        bot.damage.health_quarter_points = 0;
        remembered(&mut engine, 0x0380_0000 | u32::from(id), 0x0380_0000);
        assert_tick(&mut engine, credited);
    }
}

#[test]
fn unknown_removed_and_noncharacter_attacker_handles_do_not_suppress_healing_credit() {
    for attacker in [u32::MAX, 0x0380_0003, 0x0680_0000] {
        let (mut engine, _) = fixture();
        engine.peers.remove("synthetic-reservation-0003");
        remembered(&mut engine, attacker, 0x0380_0000);
        assert_tick(&mut engine, true);
    }
    let (mut engine, id) = fixture();
    engine.bot_registry.bots.remove(&id);
    remembered(&mut engine, 0x0380_0000 | u32::from(id), 0x0380_0000);
    assert_tick(&mut engine, true);
}

#[test]
fn represented_development_attacker_is_resolved_only_in_its_owners_world() {
    for (world, credited) in [(1, false), (2, true)] {
        let (mut engine, id) = fixture();
        let mut target = engine.bot_registry.bots.remove(&id).unwrap().target;
        target.squad = replication::SquadAddress::from_index(0).unwrap();
        let owner = engine.peers.get_mut("synthetic-reservation-0003").unwrap();
        owner.reservation.world = world;
        owner.development_target = Some(target);
        owner.dummy_created = true;
        owner.dummy_health = Some(0);
        remembered(&mut engine, 0x0380_0000 | u32::from(id), 0x0380_0000);
        assert_tick(&mut engine, credited);
    }
}

#[test]
fn failed_healing_audit_commits_neither_restored_health_nor_credit() {
    let (mut engine, _) = fixture();
    remembered(&mut engine, u32::MAX, 0x0380_0000);
    engine.max_log = engine.written;
    assert_eq!(tick(&mut engine).err(), Some("log_limit_reached"));
    assert_eq!(
        engine
            .active
            .as_ref()
            .unwrap()
            .last_damage
            .health_quarter_points,
        160
    );
    assert!(engine.active.as_ref().unwrap().healing.is_some());
    assert!(engine.pending_reward_batch().is_empty());
}
