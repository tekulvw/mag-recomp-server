//! Independent synthetic human-collapse XP regression fixtures; no game tuning.
use super::*;
use crate::spawn_progression::{dispatch, engine, ready, reserve};
use std::sync::Arc;

fn combat_items() -> Arc<item_catalog::ItemCatalog> {
    let items = item_catalog::ItemCatalog::from_value_for_application(json!({
        "schema_version":2,"application_id":20231,
        "items":{
            "FactionItemList0:2":{"authored_ref":"FactionItemList0:2","faction_list":"FactionItemList0","local_ordinal":2,"wire_item_id":null,
                "name":"synthetic_rifle","item_type":"kWeapon","definition":{"group":"weapons","name":"synthetic_rifle"}},
            "FactionItemList0:3":{"authored_ref":"FactionItemList0:3","faction_list":"FactionItemList0","local_ordinal":3,"wire_item_id":null,
                "name":"synthetic_armor","item_type":"kArmor","definition":{"group":"armor","name":"synthetic_armor"}}
        },
        "definitions":{"weapons":{"synthetic_rifle":{"parameters":{"magazine_capacity":9}}},"armor":{"synthetic_armor":{"parameters":{}}}},"resources":{}
    }), blast_rules::CharacterBlastRules {armor_rating_reduction:0.0,armor_prevention_reduction:0.0,hit_location:3,damage_type:10},20231).unwrap();
    let armory = retail_armory::RetailArmory::from_json(&json!({
        "schema_version":1,"application_id":20231,"source_sha256":{"synthetic":"a".repeat(64)},
        "items":[
            {"wire_id":2,"name":"synthetic_rifle","authored_ref":"FactionItemList0:2","available_bit":2,"factions":[0,1,2],"primary_ammo":9},
            {"wire_id":3,"name":"synthetic_armor","authored_ref":"FactionItemList0:3","available_bit":3,"factions":[0,1,2],"primary_ammo":null}
        ],
        "factions":(0..3).map(|faction|json!({"faction":faction,"available_item_bits":[2,3],"skill_ids":[],
            "loadouts":(0..5).map(|_|json!({"update_id":1,"items":vec![None::<u16>;22]})).collect::<Vec<_>>()
        })).collect::<Vec<_>>()
    }).to_string()).unwrap();
    let combat = retail_combat::RetailCombatCatalog::from_json(&json!({
        "schema_version":2,"application_id":20231,"status":"synthetic",
        "authority":{"bullet_damage":true,"armor_reduction":true,"body_damage":true,"grenade_damage":false,"grenade_physics":false,"healing_schedule":false,"ammo_tokens":false},
        "provenance":{},
        "bullets":{"2":{"authored_ref":"FactionItemList0:2","name":"synthetic_rifle","definition":{},"source":{},"item_source":{},
            "ammo":{"group":"hitscan_ammo","name":"synthetic_bullet","source":{},"ammo_token_candidate":77,"ammo_token_verified_for_retail":false},
            "damage":{"character":40.0,"characterAPierce":0.0,"characterARedux":0.0,"vehicle":0.0,"vehicleAPierce":0.0,"vehicleARedux":0.0,"objective":0.0,"objectiveAPierce":0.0,"objectiveARedux":0.0,"mine":0.0},
            "effective_range":20.0,"max_range":80.0,"magazine_capacity":9,"initial_magazine_count":2}},
        "armor":{"3":{"authored_ref":"FactionItemList0:3","name":"synthetic_armor","definition":{},"source":{},"item_source":{},"armor_type":1,"rating":0.0,"prevention":0.0,"coverage":[0.0,0.0,0.0,0.0],"armor_type_source":{}}},
        "body":(0..16).map(|location|json!({"location":location,"armor_area":1,"multiplier":1.0,"incapacitated_multiplier":1.0,"source":{}})).collect::<Vec<_>>(),
        "grenades":{},"healing":{},"unsupported":{},"limitations":["Synthetic fixtures only"]
    }).to_string()).unwrap();
    Arc::new(items.with_retail_armory(armory).with_retail_combat(combat))
}

fn fixture() -> Engine<Vec<u8>> {
    let mut engine = engine()
        .with_incapacitation_rules(incapacitation::Rules {
            schema_version: 1,
            incapacitated_damage_threshold_hp: 83.0,
            incapacitated_duration_seconds: 7.0,
            excluded_character_states: vec![22],
        })
        .unwrap();
    reserve(&mut engine, 0);
    reserve(&mut engine, 1);
    engine.item_catalog = combat_items();
    for actor in engine.peers.values_mut().chain(engine.active.iter_mut()) {
        ready(actor);
        actor.life = CharacterLife::Alive {
            generation: 4,
            health_quarter_points: 120,
        };
        actor.owner_fields.state = Some(replication::CharacterState {
            state: 1,
            stance: 0,
            user_data: [0; 3],
        });
        actor.reservation.character.account_id = Some(format!(
            "synthetic-human-account-{}",
            actor.reservation.player
        ));
        actor
            .reservation
            .character
            .loadouts
            .as_mut()
            .unwrap()
            .inventories[0][12] = Some(3);
        actor.current_inventory_handle = Some(replication::InventoryHandle {
            update_id: 1,
            inventory_index: 0,
        });
        actor.observed_primary = Some(replication::InventoryItem {
            inventory_update_id: 1,
            gear_slot: 0,
            ammo_pool: 0,
            ammo_count: 9,
            reloading: false,
        });
        actor.primary_equipped = true;
        actor.pending_shared.clear();
    }
    // Distinct actual teams, regardless of admission order/auto-assignment.
    engine.active.as_mut().unwrap().squad_catalog = Arc::new(
        engine
            .spawn_catalog
            .as_ref()
            .unwrap()
            .for_squad(16)
            .unwrap(),
    );
    engine
        .peers
        .get_mut("synthetic-reservation-0000")
        .unwrap()
        .squad_catalog = Arc::new(engine.spawn_catalog.as_ref().unwrap().for_squad(0).unwrap());
    engine
}
fn victim(engine: &Engine<Vec<u8>>) -> &Active {
    &engine.peers["synthetic-reservation-0000"]
}
fn kills(engine: &Engine<Vec<u8>>) -> Vec<progression::RewardEvent> {
    engine
        .pending_reward_batch()
        .into_iter()
        .filter(|event| event.kind == progression::RewardKind::EnemyKilled)
        .collect()
}
fn native_hit(defender: u32, user_data: u32) -> Vec<u8> {
    let mut bytes = 0x020fu16.to_be_bytes().to_vec();
    bytes.extend([0, 0, 3]);
    bytes.extend(user_data.to_be_bytes());
    bytes.extend(defender.to_be_bytes());
    for number in [100.0_f32, 3.0, 100.0, 101.0, 3.0, 100.0] {
        bytes.extend(number.to_be_bytes());
    }
    bytes.extend(1000u16.to_be_bytes());
    bytes
}
fn hit(
    engine: &mut Engine<Vec<u8>>,
    user_data: u32,
    now_ms: u64,
) -> Result<Vec<OutboundMessage>, &'static str> {
    dispatch(engine, &native_hit(0x0380_0000, user_data), now_ms)
}
fn queue_grenade(
    engine: &mut Engine<Vec<u8>>,
    target: u16,
    attacker: u16,
    generation: u32,
    amount: f32,
    now_ms: u64,
) -> Result<Vec<OutboundMessage>, &'static str> {
    let active = engine.active.as_mut().unwrap();
    active.grenade_damage.push_back(PendingDamage {
        target_character_id: 0x0380_0000 | u32::from(target),
        target_reservation_id: Some(format!("synthetic-reservation-{target:04}")),
        id: format!("synthetic-grenade-{now_ms}"),
        generation,
        falling: false,
        damage: damage::ResolvedDamage {
            amount_hp: amount,
            kind: damage::DamageKind::Explosive,
            kinetic_response: true,
            location: 3,
            attacker_id: 0x0380_0000 | u32::from(attacker),
            source_object_id: 0x1080_0001,
            user_data: now_ms as u32,
        },
    });
    let header = replication::UpdateHeader {
        sequence: active.latest_owner_sequence + 1,
        nak_sequence: 0,
        client_index: 0,
        object_type_mask: 0,
        timestamp_ms: now_ms as u32,
    };
    let packet = replication::ObjectUpdate { header, wire: &[] }
        .encode_empty_with_profile(engine.wire_profile)
        .unwrap();
    dispatch(engine, &packet, now_ms)
}

#[test]
fn native_hit_initial_human_collapse_grants_one_kill_receipt_and_finishing_does_not_repeat_it() {
    let mut engine = fixture();
    assert_eq!(
        victim(&engine).reservation.character.faction,
        engine
            .active
            .as_ref()
            .unwrap()
            .reservation
            .character
            .faction
    );
    assert_ne!(
        victim(&engine).team(),
        engine.active.as_ref().unwrap().team()
    );
    hit(&mut engine, 10, 1000).unwrap();
    assert!(matches!(
        victim(&engine).life,
        CharacterLife::Incapacitated { generation: 4, .. }
    ));
    assert_eq!(victim(&engine).reward_statistics.get(&12), Some(&1));
    assert_eq!(
        engine.active.as_ref().unwrap().reward_statistics.get(&10),
        Some(&1)
    );
    let rewards = kills(&engine);
    assert_eq!(rewards.len(), 1, "native Hit must stage human-collapse XP");
    assert_eq!(rewards[0].player_id, 1);
    assert_eq!(rewards[0].character_id, 2);
    assert_eq!(rewards[0].units, 1);
    queue_grenade(&mut engine, 0, 1, 4, 83.0, 2000).unwrap();
    assert!(matches!(
        victim(&engine).life,
        CharacterLife::AwaitingRespawn { generation: 4 }
    ));
    assert_eq!(kills(&engine).len(), 1);
    assert_eq!(victim(&engine).reward_statistics.get(&12), Some(&1));

    let mut terminal = fixture();
    terminal.incapacitation_rules = None;
    hit(&mut terminal, 12, 1000).unwrap();
    assert!(matches!(
        victim(&terminal).life,
        CharacterLife::AwaitingRespawn { generation: 4 }
    ));
    assert_eq!(kills(&terminal).len(), 1);
}

#[test]
fn queued_grenade_collapse_grants_xp_for_owned_damage_even_after_killer_dies() {
    for posthumous in [false, true] {
        let mut engine = fixture();
        if posthumous {
            engine.active.as_mut().unwrap().life = CharacterLife::AwaitingRespawn { generation: 4 };
        }
        queue_grenade(&mut engine, 0, 1, 4, 40.0, 1000).unwrap();
        assert!(matches!(
            victim(&engine).life,
            CharacterLife::Incapacitated { generation: 4, .. }
        ));
        assert_eq!(victim(&engine).reward_statistics.get(&12), Some(&1));
        let rewards = kills(&engine);
        assert_eq!(
            rewards.len(),
            1,
            "accepted queued grenade must stage human-collapse XP"
        );
        assert_eq!(rewards[0].player_id, 1);
        assert_eq!(rewards[0].units, 1);
    }
}

#[test]
fn expiry_consumes_incapacitation_without_another_ordinary_kill_receipt() {
    let mut engine = fixture();
    hit(&mut engine, 10, 1000).unwrap();
    assert_eq!(kills(&engine).len(), 1);
    engine
        .tick_incapacitation(Duration::from_secs(8), 1)
        .unwrap();
    assert!(matches!(
        victim(&engine).life,
        CharacterLife::AwaitingRespawn { generation: 4 }
    ));
    engine
        .tick_incapacitation(Duration::from_secs(9), 1)
        .unwrap();
    assert_eq!(kills(&engine).len(), 1);
    assert_eq!(victim(&engine).reward_statistics.get(&12), Some(&1));
}

#[test]
fn same_generation_revive_and_new_collapse_produce_distinct_reward_ids() {
    let mut engine = fixture();
    hit(&mut engine, 10, 1000).unwrap();
    let first = kills(&engine)
        .first()
        .expect("first collapse reward")
        .reward_id
        .clone();
    let skills=skill_catalog::SkillCatalog::from_value(json!({"schema_version":2,"application_id":20231,"table":{"Skills":{
        "MagCharacterSkill1":{"MagCharacterSkill":{"Default":{"Modifier":"kCharModResuscitatePercent","ModifierValue":0.2},"Black":{"Available":true}}}
    }}})).unwrap();
    engine.active.as_mut().unwrap().modifiers = skills.resolve(1, &[2, 0, 0, 0, 0]).unwrap();
    engine.active.as_mut().unwrap().squad_catalog = Arc::clone(&victim(&engine).squad_catalog);
    let mut revive = 0x021cu16.to_be_bytes().to_vec();
    revive.extend(0x0380_0000u32.to_be_bytes());
    dispatch(&mut engine, &revive, 2000).unwrap();
    assert!(matches!(
        victim(&engine).life,
        CharacterLife::Alive {
            generation: 4,
            health_quarter_points: 80
        }
    ));
    engine.active.as_mut().unwrap().squad_catalog = Arc::new(
        engine
            .spawn_catalog
            .as_ref()
            .unwrap()
            .for_squad(16)
            .unwrap(),
    );
    hit(&mut engine, 11, 3000).unwrap();
    let rewards = kills(&engine);
    assert_eq!(rewards.len(), 2);
    assert_ne!(rewards[0].reward_id, rewards[1].reward_id);
    assert!(rewards.iter().any(|event| event.reward_id == first));
    assert!(matches!(
        victim(&engine).life,
        CharacterLife::Incapacitated { generation: 4, .. }
    ));
    assert_eq!(victim(&engine).reward_statistics.get(&12), Some(&2));
}

#[test]
fn friendly_self_stale_and_another_actors_queued_attribution_grant_no_kill_xp() {
    let mut friendly = fixture();
    friendly.active.as_mut().unwrap().squad_catalog = Arc::clone(&victim(&friendly).squad_catalog);
    hit(&mut friendly, 10, 1000).unwrap();
    assert_eq!(victim(&friendly).reward_statistics.get(&12), Some(&1));
    assert!(kills(&friendly).is_empty());

    let mut cross_world = fixture();
    cross_world
        .peers
        .get_mut("synthetic-reservation-0000")
        .unwrap()
        .reservation
        .world = 2;
    hit(&mut cross_world, 10, 1000).unwrap();
    assert!(kills(&cross_world).is_empty());
    assert_eq!(
        cross_world
            .active
            .as_ref()
            .unwrap()
            .reward_statistics
            .get(&10),
        None
    );

    let mut suicide = fixture();
    queue_grenade(&mut suicide, 1, 1, 4, 40.0, 1000).unwrap();
    assert_eq!(
        suicide.active.as_ref().unwrap().reward_statistics.get(&12),
        Some(&1)
    );
    assert!(kills(&suicide).is_empty());

    let mut stale = fixture();
    queue_grenade(&mut stale, 0, 1, 3, 40.0, 1000).unwrap();
    assert!(victim(&stale).life.is_alive());
    assert!(kills(&stale).is_empty());

    // A real third actor makes spoofed queue attribution distinguishable from
    // an unknown handle. Current actor1 must not mint actor2's XP receipt.
    let mut forged = fixture();
    reserve(&mut forged, 2);
    let mut attacker = forged.peers.remove("synthetic-reservation-0001").unwrap();
    let mut other = forged.active.take().unwrap();
    ready(&mut other);
    other.life = CharacterLife::Alive {
        generation: 4,
        health_quarter_points: 120,
    };
    other.reservation.character.account_id = Some("synthetic-human-account-2".into());
    other.squad_catalog = Arc::clone(&attacker.squad_catalog);
    forged.peers.insert(other.reservation.id.clone(), other);
    attacker.pending_shared.clear();
    forged.active = Some(attacker);
    queue_grenade(&mut forged, 0, 2, 4, 40.0, 1000).unwrap();
    assert!(!victim(&forged).life.is_alive());
    assert_eq!(victim(&forged).reward_statistics.get(&12), Some(&1));
    assert_eq!(
        forged.peers["synthetic-reservation-0002"]
            .reward_statistics
            .get(&10)
            .copied()
            .unwrap_or(0),
        0
    );
    assert!(kills(&forged).is_empty());
}

#[test]
fn failed_audit_releases_neither_native_hit_nor_queued_grenade_reward() {
    let mut bullet = fixture();
    bullet.max_log = bullet.written;
    assert_eq!(hit(&mut bullet, 10, 1000).err(), Some("log_limit_reached"));
    assert!(victim(&bullet).life.is_alive());
    assert!(kills(&bullet).is_empty());
    let mut grenade = fixture();
    grenade.max_log = grenade.written;
    assert_eq!(
        queue_grenade(&mut grenade, 0, 1, 4, 40.0, 1000).err(),
        Some("log_limit_reached")
    );
    assert!(victim(&grenade).life.is_alive());
    assert!(kills(&grenade).is_empty());
}

#[test]
fn exhausted_collapse_ordinal_keeps_damage_and_death_but_denies_an_ambiguous_xp_receipt() {
    let mut engine = fixture();
    engine
        .peers
        .get_mut("synthetic-reservation-0000")
        .unwrap()
        .reward_statistics
        .insert(12, i32::MAX);
    hit(&mut engine, 10, 1000).unwrap();
    assert!(matches!(
        victim(&engine).life,
        CharacterLife::Incapacitated { generation: 4, .. }
    ));
    assert_eq!(victim(&engine).last_damage.health_quarter_points, 0);
    assert_eq!(victim(&engine).reward_statistics.get(&12), Some(&i32::MAX));
    assert!(kills(&engine).is_empty());
    assert!(
        std::str::from_utf8(&engine.writer)
            .unwrap()
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .any(|entry| entry["gameplay_denial"] == "human_kill_reward_invalid_collapse_ordinal")
    );
}
