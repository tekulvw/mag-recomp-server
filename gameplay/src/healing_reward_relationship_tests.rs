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

fn healing_items(name: &str) -> item_catalog::ItemCatalog {
    let items = item_catalog::ItemCatalog::from_value_for_application(
        json!({
            "schema_version":2,"application_id":20231,
            "items":{"FactionItemList0:2":{"authored_ref":"FactionItemList0:2",
                "faction_list":"FactionItemList0","local_ordinal":2,"wire_item_id":null,
                "name":name,"item_type":"kActiveItem",
                "definition":{"group":"active_items","name":name}}},
            "definitions":{"active_items":{(name):{"parameters":{}}}},"resources":{}
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
        "healing":{"2":{"authored_ref":"FactionItemList0:2","name":name,
            "definition":{"group":"active_items","name":name},"source":{},"item_source":{},"global_source":{},
            "cooldown_seconds":1.,"heal_percent":10.,"duration_seconds":1.}},
        "unsupported":{},"limitations":["Independent synthetic values; no game tuning"]
    }).to_string()).unwrap();
    items.with_retail_combat(combat)
}

pub(crate) fn effect() -> healing::HealingEffect {
    healing::HealingEffect::new_with_catalog(
        healing::HealItem::Retail(2),
        4,
        Duration::ZERO,
        &healing_items("synthetic_heal"),
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

fn medkit_request(ids: &[u32]) -> Vec<u8> {
    let mut request = 0x021bu16.to_be_bytes().to_vec();
    request.extend((ids.len() as u16).to_be_bytes());
    for id in ids {
        request.extend(id.to_be_bytes());
    }
    request.push(0);
    request
}

fn medkit_bot_fixture() -> (Engine<Vec<u8>>, u16) {
    let (mut engine, patient) = fixture();
    let bot = engine.bot_registry.bots.get_mut(&patient).unwrap();
    bot.target.squad = replication::SquadAddress::from_index(0).unwrap();
    bot.target.runtime = Some(development_target::BotRuntime::spawned(
        Duration::ZERO,
        bot.target.position,
    ));
    bot.health = 160;
    bot.damage.health_quarter_points = 160;
    bot.damage.last_attacker = u32::MAX;
    let mut kit = bot.target.loadout.clone();
    kit.inventories[usize::from(kit.handle.inventory_index)].slots[0] = Some(2);
    let healer = engine.active.as_mut().unwrap();
    healer.healing = None;
    healer.reservation.character.account_id = Some("synthetic-live-healer".into());
    healer.current_inventory_handle = Some(kit.handle);
    healer.spawned_loadout = Some(kit);
    engine.item_catalog = Arc::new(healing_items("medkit"));
    engine.match_started = true;
    engine.bot_registry.config.as_mut().unwrap().healing.enabled = false;
    (engine, patient)
}

#[test]
fn human_medkit_native_request_heals_living_registry_bot_with_ai_healing_off() {
    let (mut engine, bot) = medkit_bot_fixture();
    let request = medkit_request(&[
        0x0380_0000 | u32::from(bot),
        0x0380_0003,
        0x0380_0000 | u32::from(bot),
    ]);
    dispatch(&mut engine, &request, 100).unwrap();
    assert!(engine.peers["synthetic-reservation-0003"].healing.is_some());
    assert_eq!(engine.bot_registry.bots[&bot].health, 160);
    let creation = engine.bot_registry.bots[&bot]
        .snapshot(Duration::from_millis(101), engine.wire_profile)
        .unwrap();
    let healer = engine.active.as_mut().unwrap();
    let before_frames = healer.pending_shared.len();
    healer.pending_shared.push_back(creation.clone());
    engine
        .bot_registry
        .known
        .entry(healer.reservation.id.clone())
        .or_default()
        .insert(bot);
    dispatch(&mut engine, &request, 200).unwrap();
    assert_eq!(engine.active.as_ref().unwrap().healing_sequence, 1);
    engine
        .tick_bot_healing(Duration::from_millis(1100), 1)
        .unwrap();
    assert_eq!(engine.bot_registry.bots[&bot].health, 200);
    let queue = &engine.active.as_ref().unwrap().pending_shared;
    assert_eq!(queue.len(), before_frames + 2);
    assert_eq!(queue[before_frames], creation);
    let expected = replication::CharacterDamageUpdate {
        sequence: 2,
        timestamp_ms: 1100,
        object_index: bot,
        damage: engine.bot_registry.bots[&bot].damage,
        forced_state: None,
    }
    .encode_with_profile(engine.wire_profile)
    .unwrap();
    assert_eq!(queue[before_frames + 1], expected);
    let rewards = engine.pending_reward_batch();
    assert_eq!(rewards.len(), 1);
    assert_eq!(rewards[0].kind, progression::RewardKind::TeammateHealed);
    assert_eq!(rewards[0].units, 10);
    assert_eq!(rewards[0].account_id, "synthetic-live-healer");
    engine
        .tick_bot_healing(Duration::from_millis(1200), 1)
        .unwrap();
    assert_eq!(engine.bot_registry.bots[&bot].health, 200);
    assert_eq!(engine.pending_reward_batch(), rewards);
}

#[test]
fn human_medkit_registry_bot_refuses_ineligible_and_stale_recipients() {
    for (health, team, dead) in [(400, 0, false), (160, 1, false), (160, 0, true)] {
        let (mut engine, id) = medkit_bot_fixture();
        let bot = engine.bot_registry.bots.get_mut(&id).unwrap();
        bot.health = health;
        bot.damage.health_quarter_points = health;
        bot.target.squad = replication::SquadAddress::from_index(team * 16).unwrap();
        if dead {
            bot.target.runtime.as_mut().unwrap().phase = development_target::BotPhase::Dead;
        }
        let request = medkit_request(&[0x0380_0000 | u32::from(id)]);
        dispatch(&mut engine, &request, 100).unwrap();
        assert_eq!(engine.active.as_ref().unwrap().healing_sequence, 0);
        engine
            .tick_bot_healing(Duration::from_millis(1100), 1)
            .unwrap();
        assert_eq!(engine.bot_registry.bots[&id].health, health);
    }
    for replace_allocation in [false, true] {
        let (mut engine, id) = medkit_bot_fixture();
        let request = medkit_request(&[0x0380_0000 | u32::from(id)]);
        dispatch(&mut engine, &request, 100).unwrap();
        assert_eq!(engine.active.as_ref().unwrap().healing_sequence, 1);
        let bot = engine.bot_registry.bots.get_mut(&id).unwrap();
        if replace_allocation {
            bot.allocation_generation += 1;
        } else {
            bot.target.runtime.as_mut().unwrap().generation += 1;
        }
        engine
            .tick_bot_healing(Duration::from_millis(1100), 1)
            .unwrap();
        assert_eq!(engine.bot_registry.bots[&id].health, 160);
        assert!(engine.pending_reward_batch().is_empty());
    }
}

#[test]
fn human_medkit_record_refusal_preserves_session_health_credit_and_fifo() {
    let (mut denied_start, id) = medkit_bot_fixture();
    let request = medkit_request(&[0x0380_0000 | u32::from(id), 0x0380_0003]);
    denied_start.max_log = denied_start.written;
    assert_eq!(
        dispatch(&mut denied_start, &request, 100).err(),
        Some("log_limit_reached")
    );
    assert_eq!(denied_start.active.as_ref().unwrap().healing_sequence, 0);
    assert!(
        denied_start.peers["synthetic-reservation-0003"]
            .healing
            .is_none()
    );
    assert_eq!(denied_start.bot_registry.bots[&id].health, 160);
    assert!(
        denied_start.bot_registry.bots[&id]
            .received_healing
            .is_none()
    );

    let (mut denied_tick, id) = medkit_bot_fixture();
    let request = medkit_request(&[0x0380_0000 | u32::from(id)]);
    dispatch(&mut denied_tick, &request, 100).unwrap();
    let queue = denied_tick.active.as_ref().unwrap().pending_shared.clone();
    denied_tick.max_log = denied_tick.written;
    assert_eq!(
        denied_tick
            .tick_bot_healing(Duration::from_millis(1100), 1)
            .err(),
        Some("log_limit_reached")
    );
    assert_eq!(denied_tick.bot_registry.bots[&id].health, 160);
    assert!(
        denied_tick.bot_registry.bots[&id]
            .received_healing
            .is_some()
    );
    assert_eq!(denied_tick.active.as_ref().unwrap().pending_shared, queue);
    assert!(denied_tick.pending_reward_batch().is_empty());
}

#[test]
fn human_medkit_and_bot_self_healing_never_overlap() {
    for human_first in [false, true] {
        let (mut engine, id) = medkit_bot_fixture();
        engine.bot_registry.config.as_mut().unwrap().healing.enabled = true;
        let bot = engine.bot_registry.bots.get_mut(&id).unwrap();
        let runtime = bot.target.runtime.as_mut().unwrap();
        let mut motion = bot_physics::Motion::new(
            [runtime.position.x, runtime.position.y, runtime.position.z].map(f64::from),
        );
        motion.grounded = true;
        runtime.physics_motion = Some(motion);
        let kit = &mut bot.target.loadout;
        kit.inventories[usize::from(kit.handle.inventory_index)].slots[0] = Some(2);
        let request = medkit_request(&[0x0380_0000 | u32::from(id)]);
        if human_first {
            dispatch(&mut engine, &request, 100).unwrap();
        }
        engine
            .tick_bot_healing(Duration::from_millis(101), 1)
            .unwrap();
        if !human_first {
            dispatch(&mut engine, &request, 102).unwrap();
        }
        assert_eq!(
            engine.bot_registry.bots[&id].received_healing.is_some(),
            human_first
        );
        let audit = std::str::from_utf8(&engine.writer).unwrap();
        assert_eq!(
            audit.matches("\"op\":\"bot_healing_started\"").count(),
            usize::from(!human_first)
        );
    }
}
