//! Independent synthetic human-collapse XP regression fixtures; no game tuning.
use super::*;
use crate::spawn_progression::{dispatch, engine, ready, reserve};
use std::sync::Arc;

fn combat_items() -> Arc<item_catalog::ItemCatalog> {
    combat_items_with_grenade(false)
}
fn combat_items_with_grenade(grenade: bool) -> Arc<item_catalog::ItemCatalog> {
    combat_fixture_items(grenade, 0.0, 0.0, None, false, false)
}

pub(crate) fn combat_items_for_ground_test(vehicle_hp: f32) -> Arc<item_catalog::ItemCatalog> {
    combat_fixture_items(false, vehicle_hp, 0.0, None, false, false)
}

pub(crate) fn grenade_items_for_ground_test(vehicle_hp: f32) -> Arc<item_catalog::ItemCatalog> {
    combat_fixture_items(true, 0.0, vehicle_hp, None, false, false)
}

pub(crate) fn grenade_items_with_cover() -> Arc<item_catalog::ItemCatalog> {
    combat_fixture_items(true, 0.0, 0.0, Some(1.25), false, false)
}

pub(crate) fn vehicle_grenade_items_with_cover(vehicle_hp: f32) -> Arc<item_catalog::ItemCatalog> {
    combat_fixture_items(true, 0.0, vehicle_hp, Some(1.25), false, false)
}

pub(crate) fn bot_firearm_items() -> Arc<item_catalog::ItemCatalog> {
    combat_fixture_items(false, 0.0, 0.0, None, true, false)
}

pub(crate) fn melee_items() -> Arc<item_catalog::ItemCatalog> {
    combat_fixture_items(true, 0.0, 0.0, None, false, true)
}

fn combat_fixture_items(
    grenade: bool,
    vehicle_hp: f32,
    grenade_vehicle_hp: f32,
    material_penetration: Option<f64>,
    bot_firearm: bool,
    melee: bool,
) -> Arc<item_catalog::ItemCatalog> {
    let (raw, metadata) = combat_fixture_documents(
        grenade,
        vehicle_hp,
        grenade_vehicle_hp,
        material_penetration,
        bot_firearm,
        melee,
    );
    finish_combat_fixture(
        item_catalog::ItemCatalog::from_value_for_application(raw, synthetic_blast_rules(), 20231)
            .unwrap(),
        &metadata,
    )
}

pub(crate) fn synthetic_blast_rules() -> blast_rules::CharacterBlastRules {
    blast_rules::CharacterBlastRules {
        armor_rating_reduction: 0.0,
        armor_prevention_reduction: 0.0,
        hit_location: 3,
        damage_type: 10,
    }
}

pub(crate) fn combat_fixture_documents(
    grenade: bool,
    vehicle_hp: f32,
    grenade_vehicle_hp: f32,
    material_penetration: Option<f64>,
    bot_firearm: bool,
    melee: bool,
) -> (Value, Value) {
    let mut weapon = json!({"magazine_capacity":9});
    if bot_firearm {
        weapon["reticule_params_stand"] = json!({
            "KnockAdd":0,"KnockFire":0.,"KnockReturn":1.,"SideKnockFire":0.,"SideKnockReturn":1.,
            "SpreadFire":0.,"SpreadReturn":1.,"SpreadMove":0.,"SpreadRotate":0.,"MoveSpreadReturn":1.,
            "cone_width_range":[0.,0.],"pitch_offset_range":[0.,0.],"yaw_offset_range":[0.,0.],
            "SpreadNormalization":0,"WidthCombineOp":"kWCO_Add",
        });
    }
    let raw = json!({
        "schema_version":2,"application_id":20231,
        "items":{
            "FactionItemList0:2":{"authored_ref":"FactionItemList0:2","faction_list":"FactionItemList0","local_ordinal":2,"wire_item_id":null,
                "name":"synthetic_rifle","item_type":"kWeapon","definition":{"group":"weapons","name":"synthetic_rifle"}},
            "FactionItemList0:3":{"authored_ref":"FactionItemList0:3","faction_list":"FactionItemList0","local_ordinal":3,"wire_item_id":null,
                "name":"synthetic_armor","item_type":"kArmor","definition":{"group":"armor","name":"synthetic_armor"}}
        },
        "definitions":{"weapons":{"synthetic_rifle":{"parameters":weapon}},"armor":{"synthetic_armor":{"parameters":{}}}},"resources":{}
    });
    let mut metadata = json!({
        "schema_version":2,"application_id":20231,"status":"synthetic",
        "authority":{"bullet_damage":true,"armor_reduction":true,"body_damage":true,"grenade_damage":false,"grenade_physics":false,"healing_schedule":false,"ammo_tokens":false},
        "provenance":{},
        "bullets":{"2":{"authored_ref":"FactionItemList0:2","name":"synthetic_rifle","definition":{},"source":{},"item_source":{},
            "ammo":{"group":"hitscan_ammo","name":"synthetic_bullet","source":{},"ammo_token_candidate":77,"ammo_token_verified_for_retail":false},
            "damage":{"character":40.0,"characterAPierce":0.0,"characterARedux":0.0,"vehicle":vehicle_hp,"vehicleAPierce":0.0,"vehicleARedux":0.0,"objective":0.0,"objectiveAPierce":0.0,"objectiveARedux":0.0,"mine":0.0},
            "effective_range":20.0,"max_range":80.0,"magazine_capacity":9,"initial_magazine_count":2}},
        "armor":{"3":{"authored_ref":"FactionItemList0:3","name":"synthetic_armor","definition":{},"source":{},"item_source":{},"armor_type":1,"rating":0.0,"prevention":0.0,"coverage":[0.0,0.0,0.0,0.0],"armor_type_source":{}}},
        "body":(0..16).map(|location|json!({"location":location,"armor_area":1,"multiplier":1.0,"incapacitated_multiplier":1.0,"source":{}})).collect::<Vec<_>>(),
        "grenades":{},"healing":{},"unsupported":{},"limitations":["Synthetic fixtures only"]
    });
    if bot_firearm {
        metadata["bullets"]["2"]["firearm"] = json!({
            "initial_fire_mode":"kSemiAuto","refire_seconds":0.1,
            "reload_seconds":1.,"infinite_ammo":false,
        });
    }
    if grenade {
        metadata["schema_version"] = json!(3);
        metadata["authority"]["grenade_damage"] = json!(true);
        metadata["authority"]["healing_schedule"] = json!(true);
        metadata["runtime_policy"] = json!({"projectile_motion":"authenticated_owner_reports",
            "grenade_token_basis":"expected_static_original_resources","projectile_expiry_grace_ms":0,
            "healing_tick_policy":"guaranteed_ticks_with_bounded_catch_up","healing_tick_seconds":1});
        metadata["grenades"]["4"] = json!({"authored_ref":"SyntheticGrenade:4","name":"synthetic_frag",
            "definition":{},"source":{},"item_source":{},
            "ammo":{"group":"projectile_ammo","name":"synthetic_frag_ammo","source":{},
                "ammo_token_candidate":91,"ammo_token_verified_for_retail":false},
            "damage":{"character":20.0,"characterAPierce":0.0,"characterARedux":0.0,
                "vehicle":grenade_vehicle_hp,"vehicleAPierce":0.0,"vehicleARedux":0.0,"objective":0.0,
                "objectiveAPierce":0.0,"objectiveARedux":0.0,"mine":0.0},
            "full_radius":8.0,"half_radius":4.0,"fuse_ms":100,"authored_lifetime_ms":2000,
            "initial_count":1,"effective_range":4.0,"max_range":8.0,
            "detonation_strategy":"kWepDetonationStrategyTimer","explosion_strategy":"kWepExplosionStrategyDebris",
            "physics":{"launch_velocity":1.0,"radius":0.1,"capsuleHeight":[0.1,0.1,0.1],
                "collisionType":"kWepProjCollisionShapeCapsule","mass":1.0,"friction":0.0,"restitution":0.0,
                "linearDamping":0.0,"linearDampingOnGround":0.0,"angularDamping":0.0,"angularDampingOnGround":0.0,
                "fBounceDampingDefault":0.0,"fBounceDampingCharacter":0.0,"m_fLoftAngle":0.0,
                "random_rotation_range":0.0,"propulsion_strategy":"kWepPropulsionStrategyThrown"}});
        if let Some(penetration) = material_penetration {
            metadata["grenades"]["4"]["material_penetration"] = json!(penetration);
        }
    }
    if melee {
        metadata["schema_version"] = json!(4);
        metadata["authority"]["melee_damage"] = json!(true);
        metadata["melee_direction"] = json!({
            "front_dot_threshold":-0.4,"back_dot_threshold":0.6,
            "front_multiplier":1.,"side_multiplier":1.,"back_multiplier":1.
        });
        metadata["melee"]["2"] = json!({
            "authored_ref":"FactionItemList0:2","name":"synthetic_knife",
            "definition":{},"source":{},"item_source":{},
            "ammo":{"group":"hitscan_ammo","name":"synthetic_swipe","source":{},
                "ammo_token_candidate":78,"ammo_token_verified_for_retail":false},
            "damage":{"character":40.,"characterAPierce":0.,"characterARedux":0.,
                "vehicle":0.,"vehicleAPierce":0.,"vehicleARedux":0.,
                "objective":0.,"objectiveAPierce":0.,"objectiveARedux":0.,"mine":0.},
            "effective_range":3.,"max_range":4.,"gear_slot":2,"fire_mode":0,
            "category":"kWepCategoryKnife","damage_type":3,
            "max_angle":1.,"max_height":1.,"refire_seconds":0.2
        });
        // This synthetic item can be equipped in either slot; metadata, not its
        // numeric ID, determines the admitted weapon kind for the event.
        metadata["bullets"].as_object_mut().unwrap().clear();
        metadata["body"][0]["armor_area"] = json!(0);
    }
    (raw, metadata)
}

pub(crate) fn finish_combat_fixture(
    items: item_catalog::ItemCatalog,
    metadata: &Value,
) -> Arc<item_catalog::ItemCatalog> {
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
    let combat = retail_combat::RetailCombatCatalog::from_json(&metadata.to_string()).unwrap();
    Arc::new(items.with_retail_armory(armory).with_retail_combat(combat))
}

pub(crate) fn enable_incoming_base(engine: Engine<Vec<u8>>, base: f32) -> Engine<Vec<u8>> {
    let digest = engine
        .spawn_catalog
        .as_ref()
        .unwrap()
        .source_sha256()
        .unwrap();
    let rules = crate::character_damage_rules::Rules::from_json(&json!({
        "schema_version":1,"application_id":20231,"map_name":"synthetic_arena","game_type":0,
        "friendly_fire_multiplier":base,"provenance":{"spawn_catalog_sha256":digest,"source":"synthetic"}
    }).to_string()).unwrap();
    engine.with_character_damage_rules(rules).unwrap()
}

#[test]
fn native_hit_uses_relation_base_with_same_faction_and_preserves_audit_rollback() {
    let mut friendly = fixture();
    friendly.active.as_mut().unwrap().squad_catalog = Arc::clone(&victim(&friendly).squad_catalog);
    let mut friendly = enable_incoming_base(friendly, 0.25);
    assert_eq!(
        friendly.character_incoming_base(0x03800001, 0x03800001, 1),
        1.0
    );
    assert_eq!(
        friendly.character_incoming_base(0x03800001, 0x0380003f, 1),
        1.0
    );
    hit(&mut friendly, 19, 1000).unwrap();
    assert!(matches!(
        victim(&friendly).life,
        CharacterLife::Alive {
            health_quarter_points: 80,
            ..
        }
    ));
    assert_eq!(victim(&friendly).last_damage.last_location_and_type, 0x13);
    assert!(kills(&friendly).is_empty());
    let mut opposing = enable_incoming_base(fixture(), 0.25);
    assert_eq!(
        victim(&opposing).reservation.character.faction,
        opposing
            .active
            .as_ref()
            .unwrap()
            .reservation
            .character
            .faction
    );
    hit(&mut opposing, 20, 1000).unwrap();
    assert!(matches!(
        victim(&opposing).life,
        CharacterLife::Incapacitated { .. }
    ));
    assert_eq!(kills(&opposing).len(), 1);
    let mut capped = fixture();
    capped.active.as_mut().unwrap().squad_catalog = Arc::clone(&victim(&capped).squad_catalog);
    let mut capped = enable_incoming_base(capped, 100.0);
    hit(&mut capped, 22, 1000).unwrap();
    assert!(matches!(
        victim(&capped).life,
        CharacterLife::Alive {
            health_quarter_points: 120,
            ..
        }
    ));
    assert!(
        std::str::from_utf8(&capped.writer)
            .unwrap()
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .any(|entry| entry["gameplay_denial"] == "invalid_damage_amount")
    );
    let mut failed = fixture();
    failed.active.as_mut().unwrap().squad_catalog = Arc::clone(&victim(&failed).squad_catalog);
    let mut failed = enable_incoming_base(failed, 0.25);
    failed.max_log = failed.written;
    assert_eq!(hit(&mut failed, 21, 1000).err(), Some("log_limit_reached"));
    assert!(matches!(
        victim(&failed).life,
        CharacterLife::Alive {
            health_quarter_points: 120,
            ..
        }
    ));
    assert!(kills(&failed).is_empty());
}

fn friendly_blast_fixture() -> Engine<Vec<u8>> {
    let mut engine = fixture();
    engine.item_catalog = combat_items_with_grenade(true);
    engine.active.as_mut().unwrap().squad_catalog = Arc::clone(&victim(&engine).squad_catalog);
    for actor in engine.active.iter_mut().chain(engine.peers.values_mut()) {
        actor.life = CharacterLife::Alive {
            generation: 4,
            health_quarter_points: 400,
        };
    }
    let floor = json!({"format":"mag_collision_world","schema_version":1,"coordinate_space":"world",
        "metadata":{"map_name":"synthetic_arena","build":"synthetic","coverage":"provisional",
            "provenance":["Synthetic floor"],"unknowns":["Synthetic only"],
            "physics_regions":[{"min":[0.,-2.,0.],"max":[128.,10.,128.],"ground_verified":true,
                "obstacles_verified":true,"provenance":["Synthetic floor"]}]},
        "instances":[{"id":"floor","vertices_xyz":[[0.,-1.,0.],[128.,-1.,0.],[128.,-1.,128.],[0.,-1.,128.]],
            "triangles":[[0,2,1],[0,3,2]]}]});
    let path = std::env::temp_dir().join(format!(
        "mag-synthetic-ff-floor-{}.json",
        std::process::id()
    ));
    std::fs::write(&path, floor.to_string()).unwrap();
    let world =
        collision::CollisionWorld::load(&path, collision::CollisionLimits::default()).unwrap();
    std::fs::remove_file(path).unwrap();
    engine = engine
        .with_collision_world(world)
        .with_development_grenade_radius();
    let source = engine.active.as_mut().unwrap();
    source
        .projectiles
        .admit_item_with_catalog(0, 4, 0, 0, &[0; 57], 4, &engine.item_catalog)
        .unwrap();
    source
        .projectiles
        .accept_owner(
            0,
            4,
            1,
            10,
            projectile::ProjectileOwnerFields {
                state: Some(2),
                position: Some(replication::CharacterPosition {
                    x: 100.,
                    y: 3.,
                    z: 100.,
                }),
                rotation_xyzw: None,
            },
        )
        .unwrap();
    enable_incoming_base(engine, 0.25)
}
fn idle_update(
    engine: &mut Engine<Vec<u8>>,
    now: u64,
) -> Result<Vec<OutboundMessage>, &'static str> {
    let wire = replication::ObjectUpdate {
        header: replication::UpdateHeader {
            sequence: engine.active.as_ref().unwrap().latest_owner_sequence + 1,
            nak_sequence: 0,
            client_index: 0,
            object_type_mask: 0,
            timestamp_ms: now as u32,
        },
        wire: &[],
    }
    .encode_empty_with_profile(engine.wire_profile)
    .unwrap();
    dispatch(engine, &wire, now)
}

#[test]
fn grenade_captures_relation_before_armor_and_queue_does_not_rescale_or_commit_failed_record() {
    let mut failed = friendly_blast_fixture();
    failed.max_log = failed.written;
    assert_eq!(
        idle_update(&mut failed, 1000).err(),
        Some("log_limit_reached")
    );
    assert!(failed.active.as_ref().unwrap().grenade_damage.is_empty());
    assert!(
        !failed
            .active
            .as_ref()
            .unwrap()
            .projectiles
            .get(0)
            .unwrap()
            .completed
    );
    assert!(matches!(
        victim(&failed).life,
        CharacterLife::Alive {
            health_quarter_points: 400,
            ..
        }
    ));
    let mut engine = friendly_blast_fixture();
    idle_update(&mut engine, 1000).unwrap();
    // Self base stays1; the distinct same-side recipient is captured at0.25.
    assert!(matches!(
        engine.active.as_ref().unwrap().life,
        CharacterLife::Alive {
            health_quarter_points: 320,
            ..
        }
    ));
    let queued = &engine.active.as_ref().unwrap().grenade_damage;
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].damage.amount_hp, 5.0);
    assert_eq!(queued[0].damage.kind, damage::DamageKind::Explosive);
    assert!(queued[0].damage.kinetic_response);
    let other_side = Arc::new(
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
        .squad_catalog = other_side;
    idle_update(&mut engine, 1001).unwrap();
    assert!(matches!(
        victim(&engine).life,
        CharacterLife::Alive {
            health_quarter_points: 380,
            ..
        }
    ));
    assert_eq!(victim(&engine).last_damage.last_location_and_type, 0xa3);
    assert!(engine.active.as_ref().unwrap().grenade_damage.is_empty());
}

pub(crate) fn fixture() -> Engine<Vec<u8>> {
    configured_fixture(engine())
}

pub(crate) fn configured_fixture(engine: Engine<Vec<u8>>) -> Engine<Vec<u8>> {
    let mut engine = engine
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
        actor.spawned_loadout = Some(
            actor
                .reservation
                .character
                .replication_loadout(&engine.item_catalog, 0, actor.wire_profile)
                .unwrap(),
        );
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
pub(crate) fn victim(engine: &Engine<Vec<u8>>) -> &Active {
    &engine.peers["synthetic-reservation-0000"]
}
fn kills(engine: &Engine<Vec<u8>>) -> Vec<progression::RewardEvent> {
    engine
        .pending_reward_batch()
        .into_iter()
        .filter(|event| event.kind == progression::RewardKind::EnemyKilled)
        .collect()
}
pub(crate) fn native_hit(defender: u32, user_data: u32) -> Vec<u8> {
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
pub(crate) fn hit(
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
        source: PendingDamageSource::Other,
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

#[test]
fn saved_armor_edit_does_not_remove_current_life_damage_authority() {
    let mut engine = fixture();
    let target = engine.peers.values_mut().next().unwrap();
    let saved = target.reservation.character.loadouts.as_mut().unwrap();
    // Removing armor is a structurally valid saved edit. It must not make a
    // living target with already published armor immune to otherwise valid Hit.
    saved.inventories[0][12] = None;
    engine
        .item_catalog
        .validate_saved_item_bindings(saved)
        .unwrap();
    hit(&mut engine, 91, 1000).unwrap();
    assert!(matches!(
        victim(&engine).life,
        CharacterLife::Incapacitated { generation: 4, .. }
    ));
    assert_eq!(kills(&engine).len(), 1);
}

#[derive(Clone, Copy, Debug)]
enum PrimaryAttachment {
    Compatible,
    Incompatible,
    Replacement,
    WrongType,
    Unbound,
    MismatchedSource,
    MissingReplacement,
    MissingLauncherWeapon,
    HostAsLauncherWeapon,
    WrongLauncherCategory,
    WrongLauncherAmmo,
    MismatchedHostAmmo,
    Optional,
    OtherRequiredBipod,
    WrongBipodRole,
}

fn machine_gun_hit_fixture(slot: Option<usize>, attachment: PrimaryAttachment) -> Engine<Vec<u8>> {
    primary_attachment_hit_fixture(
        slot,
        attachment,
        "kWepCategoryMachineGun",
        "kAttachTypeBipod",
    )
}

fn primary_attachment_hit_fixture(
    slot: Option<usize>,
    attachment: PrimaryAttachment,
    category: &str,
    kind: &str,
) -> Engine<Vec<u8>> {
    attachment_hit_fixture(
        slot.map(|slot| (slot, kind)).as_slice(),
        attachment,
        category,
    )
}

fn attachment_hit_fixture(
    attachments: &[(usize, &str)],
    attachment: PrimaryAttachment,
    category: &str,
) -> Engine<Vec<u8>> {
    use sha2::{Digest, Sha256};

    let (mut raw, mut metadata) = combat_fixture_documents(false, 0.0, 0.0, None, false, false);
    let host = &mut raw["definitions"]["weapons"]["synthetic_rifle"]["parameters"];
    host["id"] = json!("synthetic_rifle");
    host["ammo_name"] = json!(
        if matches!(attachment, PrimaryAttachment::MismatchedHostAmmo) {
            "synthetic_other_bullet"
        } else {
            "synthetic_bullet"
        }
    );
    host["category"] = json!(category);
    host["initial_fire_mode"] = json!("kSemiAuto");
    host["fire_mode"] = json!({
        "fire_modes": {
            "FireModeTypeClass0": {"FireModeTypeClass": {"fire_mode_value": "kSemiAuto"}},
            "FireModeTypeClass1": {"FireModeTypeClass": {"fire_mode_value": "kDual"}},
            "FireModeTypeClass2": {"FireModeTypeClass": {"fire_mode_value": "kAuto"}},
        }
    });
    if category == "kWepCategorySniperRifle" {
        host["fire_mode"]["fire_modes"]
            .as_object_mut()
            .unwrap()
            .retain(|key, _| key == "FireModeTypeClass0");
        host["required_sight_attachment"] = json!("synthetic_attachment_0");
        host["required_underbarrel_attachment"] = json!(match attachment {
            PrimaryAttachment::Optional => "",
            PrimaryAttachment::OtherRequiredBipod => "different_required_bipod",
            _ => "synthetic_attachment_1",
        });
    }
    host["attachment_list"] = json!({});
    for (ordinal, &(_, kind)) in attachments.iter().enumerate() {
        let name = format!("synthetic_attachment_{ordinal}");
        let item_id = 7 + ordinal;
        let item = format!("FactionItemList0:{item_id}");
        if !matches!(attachment, PrimaryAttachment::Incompatible) {
            let row = format!("WeaponAttachment{ordinal}");
            raw["definitions"]["weapons"]["synthetic_rifle"]["parameters"]["attachment_list"]
                [row] = json!({"WeaponAttachment":{"name":name}});
        }
        raw["items"][&item] = json!({
            "authored_ref":item,"faction_list":"FactionItemList0","local_ordinal":item_id,
            "wire_item_id":null,"name":name,"item_type":"kAttachmentItem",
            "definition":{"group":"attachments","name":name}
        });
        let parameters = &mut raw["definitions"]["attachments"][&name]["parameters"];
        *parameters = json!({
            "id":name,
            "attachment_type":if matches!(attachment, PrimaryAttachment::WrongType) {
                "kAttachTypeScope"
            } else {
                kind
            },
            "weapon_name":if matches!(attachment, PrimaryAttachment::Replacement) {
                "synthetic_replacement"
            } else {
                ""
            }
        });
        if matches!(
            kind,
            "kAttachTypeBipod" | "kAttachTypeForwardGrip" | "kAttachTypeSuppressor"
        ) {
            parameters["character_attachment_type"] =
                json!(if matches!(attachment, PrimaryAttachment::WrongBipodRole) {
                    "kCharAttachTypeBackup"
                } else {
                    "kCharAttachTypePrimary"
                });
        }
        if kind == "kAttachTypeLauncher" {
            parameters["character_attachment_type"] = json!("kCharAttachTypePrimary");
            parameters["weapon_name"] = json!(if matches!(
                attachment,
                PrimaryAttachment::HostAsLauncherWeapon
            ) {
                "synthetic_rifle"
            } else {
                &name
            });
            if !matches!(attachment, PrimaryAttachment::MissingLauncherWeapon) {
                raw["definitions"]["weapons"][&name]["parameters"] = json!({
                    "id":name,
                    "category":if matches!(attachment, PrimaryAttachment::WrongLauncherCategory) {
                        "kWepCategoryAssaultRifle"
                    } else {
                        "kWepCategoryGrenadeLauncher"
                    },
                    "ammo_name":if matches!(attachment, PrimaryAttachment::WrongLauncherAmmo) {
                        "synthetic_missing_projectile"
                    } else {
                        "synthetic_impact_grenade"
                    },
                    "magazine_capacity":1,
                    "initial_magazine_count":3,
                    "infinite_ammo":false
                });
            }
            raw["definitions"]["projectile_ammo"]["synthetic_impact_grenade"]["parameters"] = json!({
                "id":"synthetic_impact_grenade",
                "collisionType":"kWepProjCollisionShapeCapsule",
                "propulsion_strategy":"kWepPropulsionStrategyThrown",
                "detonation_strategy":"kWepDetonationStrategyImpact",
                "explosion_strategy":"kWepExplosionStrategyDebris"
            });
        }
        if matches!(attachment, PrimaryAttachment::MissingReplacement) {
            raw["definitions"]["attachments"][&name]["parameters"]
                .as_object_mut()
                .unwrap()
                .remove("weapon_name");
        }
    }
    let bytes = serde_json::to_vec(&raw).unwrap();
    metadata["provenance"]["catalog_sha256"] = json!(if matches!(
        attachment,
        PrimaryAttachment::MismatchedSource
    ) {
        "0".repeat(64)
    } else {
        format!("{:x}", Sha256::digest(&bytes))
    });
    let items = if matches!(attachment, PrimaryAttachment::Unbound) {
        item_catalog::ItemCatalog::from_value_for_application(raw, synthetic_blast_rules(), 20231)
            .unwrap()
    } else {
        let path = std::env::temp_dir().join(format!(
            "mag-synthetic-primary-attachment-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir(&path).unwrap();
        let rules = json!({
            "schema_version":1,"applications":[20230,20231],"character_damage": {"armor_rating_reduction":0.0,"armor_prevention_reduction":0.0,"hit_location":3,"damage_type":10}
        });
        std::fs::write(path.join("blast.json"), rules.to_string()).unwrap();
        std::fs::write(path.join("items.json"), &bytes).unwrap();
        let items = item_catalog::ItemCatalog::load_for_application(
            path.join("items.json"),
            path.join("blast.json"),
            20231,
        );
        std::fs::remove_dir_all(&path).unwrap();
        items.unwrap()
    };

    let mut engine = fixture();
    engine.item_catalog = finish_combat_fixture(items, &metadata);
    let kit = engine
        .active
        .as_mut()
        .unwrap()
        .spawned_loadout
        .as_mut()
        .unwrap();
    for (ordinal, &(slot, _)) in attachments.iter().enumerate() {
        kit.inventories[0].slots[slot] = Some(7 + u16::try_from(ordinal).unwrap());
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

#[test]
fn native_retail_firearm_mode_index_uses_the_current_host_ammo_pool() {
    for mode in 0..=2 {
        let mut engine = primary_attachment_hit_fixture(
            None,
            PrimaryAttachment::Compatible,
            "kWepCategoryAssaultRifle",
            "kAttachTypeSuppressor",
        );
        let mut wire = native_hit(0x0380_0000, 55);
        wire[3] = mode;
        dispatch(&mut engine, &wire, 1000).unwrap();
        assert_eq!(
            victim(&engine).life.damage_health(),
            Some((4, 240)),
            "mode {mode}"
        );
        assert_eq!(victim(&engine).last_damage.last_location_and_type, 0x13);
    }
}

#[test]
fn native_retail_firearm_mode_index_requires_matching_current_source_and_pool() {
    use PrimaryAttachment::*;
    for (mode, attachment, pool, denied) in [
        (3, Compatible, 0, "unsupported_retail_bullet_fire_mode"),
        (255, Compatible, 0, "unsupported_retail_bullet_fire_mode"),
        (1, Unbound, 0, "unsupported_retail_bullet_fire_mode"),
        (
            1,
            MismatchedSource,
            0,
            "unsupported_retail_bullet_fire_mode",
        ),
        (
            1,
            MismatchedHostAmmo,
            0,
            "unsupported_retail_bullet_fire_mode",
        ),
        (1, Compatible, 1, "unsupported_bullet_magazine"),
    ] {
        let mut engine = primary_attachment_hit_fixture(
            None,
            attachment,
            "kWepCategoryAssaultRifle",
            "kAttachTypeSuppressor",
        );
        engine
            .active
            .as_mut()
            .unwrap()
            .observed_primary
            .as_mut()
            .unwrap()
            .ammo_pool = pool;
        let mut wire = native_hit(0x0380_0000, 55);
        wire[3] = mode;
        dispatch(&mut engine, &wire, 1000).unwrap();
        assert_eq!(victim(&engine).life.damage_health(), Some((4, 400)));
        assert!(
            std::str::from_utf8(&engine.writer)
                .unwrap()
                .lines()
                .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                .any(|entry| entry["gameplay_denial"] == denied),
            "mode={mode} pool={pool} source={attachment:?}"
        );
    }
}

fn sniper_bipod_fixture(slot: usize, attachment: PrimaryAttachment) -> Engine<Vec<u8>> {
    attachment_hit_fixture(
        &[(14, "kAttachTypeSight"), (slot, "kAttachTypeBipod")],
        attachment,
        "kWepCategorySniperRifle",
    )
}

#[test]
fn native_heavy_sniper_hit_with_required_bipod_uses_host_ammo() {
    let mut engine = sniper_bipod_fixture(17, PrimaryAttachment::Compatible);
    let mut packet = native_hit(0x0380_0000, 55);
    packet[3] = 1;
    dispatch(&mut engine, &packet, 900).unwrap();
    assert_eq!(victim(&engine).life.damage_health(), Some((4, 400)));
    assert!(
        String::from_utf8_lossy(&engine.writer).contains("unsupported_retail_bullet_fire_mode")
    );

    hit(&mut engine, 56, 1000).unwrap();
    assert_eq!(
        victim(&engine).life.damage_health(),
        Some((4, 240)),
        "{}",
        String::from_utf8_lossy(&engine.writer)
    );
    assert_eq!(victim(&engine).last_damage.last_location_and_type, 0x13);
}

#[test]
fn native_heavy_sniper_hit_rejects_unqualified_bipods() {
    use PrimaryAttachment::*;
    for (slot, attachment, mounted) in [
        (16, Compatible, false),
        (17, OtherRequiredBipod, false),
        (17, WrongBipodRole, false),
        (17, Incompatible, false),
        (17, Unbound, false),
        (17, MismatchedSource, false),
        (17, Replacement, false),
        (17, WrongType, false),
        (17, Compatible, true),
    ] {
        let mut engine = sniper_bipod_fixture(slot, attachment);
        if mounted {
            let seat = domination_helo::PassengerSeat {
                object_index: 0,
                seat: 0,
            };
            let active = engine.active.as_mut().unwrap();
            active.helo_seat = Some(seat);
            active.owner_fields.state = Some(seat.character_state());
        }
        hit(&mut engine, 55, 1000).unwrap();
        assert_eq!(victim(&engine).life.damage_health(), Some((4, 400)));
        assert!(
            String::from_utf8_lossy(&engine.writer).contains("unsupported_bullet_attachment"),
            "slot={slot} attachment={attachment:?} mounted={mounted}"
        );
    }
}

#[test]
fn native_ordinary_hits_accept_qualified_optional_attachments() {
    for (category, kind, slot) in [
        ("kWepCategorySniperRifle", "kAttachTypeBipod", 17),
        ("kWepCategorySniperRifle", "kAttachTypeSuppressor", 16),
        ("kWepCategoryAssaultRifle", "kAttachTypeBipod", 17),
        ("kWepCategoryMachineGun", "kAttachTypeForwardGrip", 17),
        ("kWepCategorySubMachineGun", "kAttachTypeSuppressor", 16),
        ("kWepCategorySniperRifle", "kAttachTypeForwardGrip", 17),
        // Any listed damage-neutral attachment in its own slot keeps the host
        // bullet, whatever the host firearm's category.
        ("kWepCategorySubMachineGun", "kAttachTypeForwardGrip", 17),
        ("kWepCategoryShotgun", "kAttachTypeSuppressor", 16),
        ("kWepCategoryShotgun", "kAttachTypeSight", 15),
        ("kWepCategoryMachineGun", "kAttachTypeSight", 15),
    ] {
        let mut engine =
            primary_attachment_hit_fixture(Some(slot), PrimaryAttachment::Optional, category, kind);
        hit(&mut engine, 55, 1000).unwrap();
        assert_eq!(
            victim(&engine).life.damage_health(),
            Some((4, 240)),
            "{category} {kind} in slot {slot}"
        );
    }
}

#[test]
fn native_ordinary_hits_reject_wrong_optional_roles_or_missing_required_bipod() {
    use PrimaryAttachment::*;
    for (category, kind, slot, attachment) in [
        (
            "kWepCategoryShotgun",
            "kAttachTypeSuppressor",
            Some(17),
            Optional,
        ),
        (
            "kWepCategoryShotgun",
            "kAttachTypeSight",
            Some(16),
            Optional,
        ),
        (
            "kWepCategoryMachineGun",
            "kAttachTypeForwardGrip",
            Some(16),
            Optional,
        ),
        (
            "kWepCategorySubMachineGun",
            "kAttachTypeSuppressor",
            Some(16),
            WrongBipodRole,
        ),
        (
            "kWepCategorySniperRifle",
            "kAttachTypeForwardGrip",
            Some(17),
            OtherRequiredBipod,
        ),
        (
            "kWepCategorySniperRifle",
            "kAttachTypeBipod",
            None,
            Compatible,
        ),
    ] {
        let mut engine = primary_attachment_hit_fixture(slot, attachment, category, kind);
        hit(&mut engine, 55, 1000).unwrap();
        assert_eq!(
            victim(&engine).life.damage_health(),
            Some((4, 400)),
            "{category} {kind} in slot {slot:?}"
        );
        assert!(String::from_utf8_lossy(&engine.writer).contains("unsupported_bullet_attachment"));
    }
}

#[test]
fn native_machine_gun_hit_with_compatible_bipod_uses_host_ammo() {
    let mut unmodified = machine_gun_hit_fixture(None, PrimaryAttachment::Compatible);
    let mut attached = machine_gun_hit_fixture(Some(17), PrimaryAttachment::Compatible);
    for engine in [&mut unmodified, &mut attached] {
        hit(engine, 55, 1000).unwrap();
        assert!(matches!(
            victim(engine).life,
            CharacterLife::Alive {
                health_quarter_points: 240,
                ..
            }
        ));
        assert_eq!(victim(engine).last_damage.last_location_and_type, 0x13);
        assert!(kills(engine).is_empty());
    }
}

#[test]
fn native_machine_gun_hit_rejects_unqualified_attachments() {
    use PrimaryAttachment::*;
    for (slot, attachment) in [
        (16, Compatible),
        (17, Incompatible),
        (17, Replacement),
        (17, WrongType),
        (17, Unbound),
        (17, MismatchedSource),
        (17, MissingReplacement),
    ] {
        let mut engine = machine_gun_hit_fixture(Some(slot), attachment);
        hit(&mut engine, 55, 1000).unwrap();
        assert!(
            matches!(
                victim(&engine).life,
                CharacterLife::Alive {
                    health_quarter_points: 400,
                    ..
                }
            ),
            "{slot} {attachment:?}"
        );
        assert!(
            std::str::from_utf8(&engine.writer)
                .unwrap()
                .lines()
                .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                .any(|entry| entry["gameplay_denial"]
                    .as_str()
                    .is_some_and(|code| code.starts_with("unsupported_bullet_attachment"))),
            "{slot} {attachment:?}"
        );
    }
}

#[test]
fn native_machine_gun_hit_does_not_admit_a_bipod_for_a_mounted_actor() {
    let mut engine = machine_gun_hit_fixture(Some(17), PrimaryAttachment::Compatible);
    let seat = domination_helo::PassengerSeat {
        object_index: 0,
        seat: 0,
    };
    let active = engine.active.as_mut().unwrap();
    active.helo_seat = Some(seat);
    active.owner_fields.state = Some(seat.character_state());
    hit(&mut engine, 55, 1000).unwrap();
    assert!(matches!(
        victim(&engine).life,
        CharacterLife::Alive {
            health_quarter_points: 400,
            ..
        }
    ));
    assert!(
        std::str::from_utf8(&engine.writer)
            .unwrap()
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .any(|entry| entry["gameplay_denial"]
                .as_str()
                .is_some_and(|code| code.starts_with("unsupported_bullet_attachment")))
    );
}

#[test]
fn native_assault_rifle_hit_with_compatible_suppressor_uses_host_ammo() {
    for slot in [None, Some(16)] {
        let mut engine = primary_attachment_hit_fixture(
            slot,
            PrimaryAttachment::Compatible,
            "kWepCategoryAssaultRifle",
            "kAttachTypeSuppressor",
        );
        hit(&mut engine, 55, 1000).unwrap();
        assert_eq!(
            victim(&engine).life.damage_health(),
            Some((4, 240)),
            "{slot:?}"
        );
        assert_eq!(victim(&engine).last_damage.last_location_and_type, 0x13);
        assert!(kills(&engine).is_empty());
    }
}

#[test]
fn native_assault_rifle_hit_preserves_unqualified_firepoint_denials() {
    assert_unqualified_assault_attachments_denied("kAttachTypeSuppressor", 16, 17);
}

fn assert_unqualified_assault_attachments_denied(kind: &str, allowed: usize, other: usize) {
    use PrimaryAttachment::*;
    for (slot, attachment, category, mounted) in [
        (15, Compatible, "kWepCategoryAssaultRifle", false),
        (other, Compatible, "kWepCategoryAssaultRifle", false),
        (allowed, Compatible, "kWepCategorySideArm", false),
        (allowed, Compatible, "kWepCategoryAssaultRifle", true),
        (allowed, Incompatible, "kWepCategoryAssaultRifle", false),
        (allowed, Replacement, "kWepCategoryAssaultRifle", false),
        (allowed, WrongType, "kWepCategoryAssaultRifle", false),
        (allowed, Unbound, "kWepCategoryAssaultRifle", false),
        (allowed, MismatchedSource, "kWepCategoryAssaultRifle", false),
        (
            allowed,
            MissingReplacement,
            "kWepCategoryAssaultRifle",
            false,
        ),
    ] {
        let mut engine = primary_attachment_hit_fixture(Some(slot), attachment, category, kind);
        if mounted {
            let seat = domination_helo::PassengerSeat {
                object_index: 0,
                seat: 0,
            };
            let active = engine.active.as_mut().unwrap();
            active.helo_seat = Some(seat);
            active.owner_fields.state = Some(seat.character_state());
        }
        hit(&mut engine, 55, 1000).unwrap();
        assert_eq!(
            victim(&engine).life.damage_health(),
            Some((4, 400)),
            "{slot} {attachment:?} {category} mounted={mounted}"
        );
        assert!(
            std::str::from_utf8(&engine.writer)
                .unwrap()
                .lines()
                .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                .any(|entry| entry["gameplay_denial"]
                    .as_str()
                    .is_some_and(|code| code.starts_with("unsupported_bullet_attachment"))),
            "{slot} {attachment:?} {category} mounted={mounted}"
        );
    }
}

#[test]
fn native_assault_rifle_hit_keeps_host_ammo_with_forward_grip() {
    let cases: [&[(usize, &str)]; 3] = [
        &[],
        &[(17, "kAttachTypeForwardGrip")],
        &[
            (16, "kAttachTypeSuppressor"),
            (17, "kAttachTypeForwardGrip"),
        ],
    ];
    let results = cases.map(|attachments| {
        let mut engine = attachment_hit_fixture(
            attachments,
            PrimaryAttachment::Compatible,
            "kWepCategoryAssaultRifle",
        );
        hit(&mut engine, 55, 1000).unwrap();
        (
            victim(&engine).life.damage_health(),
            victim(&engine).last_damage.last_location_and_type,
        )
    });
    assert_eq!(results, [(Some((4, 240)), 0x13); 3]);
}

#[test]
fn native_assault_rifle_hit_rejects_unqualified_forward_grips() {
    assert_unqualified_assault_attachments_denied("kAttachTypeForwardGrip", 17, 16);
}

fn equip_synthetic_sidearm(engine: &mut Engine<Vec<u8>>) {
    let actor = engine.active.as_mut().unwrap();
    let inventory = &mut actor.spawned_loadout.as_mut().unwrap().inventories[0];
    inventory.slots[1] = inventory.slots[0];
    actor.primary_equipped = false;
    actor.observed_secondary = Some(replication::InventoryItem {
        gear_slot: 1,
        ..actor.observed_primary.unwrap()
    });
}

fn secondary_hit(engine: &mut Engine<Vec<u8>>) -> Result<Vec<OutboundMessage>, &'static str> {
    let mut wire = native_hit(0x0380_0000, 55);
    wire[2] = 1;
    dispatch(engine, &wire, 1000)
}

#[test]
fn native_sidearm_hit_with_compatible_suppressor_uses_host_ammo() {
    for slot in [None, Some(20)] {
        let mut engine = primary_attachment_hit_fixture(
            slot,
            PrimaryAttachment::Compatible,
            "kWepCategorySideArm",
            "kAttachTypeSuppressor",
        );
        equip_synthetic_sidearm(&mut engine);
        secondary_hit(&mut engine).unwrap();
        assert_eq!(
            victim(&engine).life.damage_health(),
            Some((4, 240)),
            "{slot:?}"
        );
        assert_eq!(victim(&engine).last_damage.last_location_and_type, 0x13);
    }
}

#[test]
fn native_sidearm_hit_keeps_unqualified_attachments_denied() {
    use PrimaryAttachment::*;
    for (gear, slot, category, attachment, mounted) in [
        (0, 16, "kWepCategorySideArm", Compatible, false),
        (1, 20, "kWepCategoryAssaultRifle", Compatible, false),
        (1, 19, "kWepCategorySideArm", Compatible, false),
        (1, 20, "kWepCategorySideArm", Compatible, true),
        (1, 20, "kWepCategorySideArm", Incompatible, false),
        (1, 20, "kWepCategorySideArm", Replacement, false),
        (1, 20, "kWepCategorySideArm", MissingReplacement, false),
        (1, 20, "kWepCategorySideArm", WrongType, false),
        (1, 20, "kWepCategorySideArm", Unbound, false),
        (1, 20, "kWepCategorySideArm", MismatchedSource, false),
    ] {
        let mut engine = primary_attachment_hit_fixture(
            Some(slot),
            attachment,
            category,
            "kAttachTypeSuppressor",
        );
        if mounted {
            let seat = domination_helo::PassengerSeat {
                object_index: 0,
                seat: 0,
            };
            let active = engine.active.as_mut().unwrap();
            active.helo_seat = Some(seat);
            active.owner_fields.state = Some(seat.character_state());
        }
        if gear == 1 {
            equip_synthetic_sidearm(&mut engine);
            secondary_hit(&mut engine).unwrap();
        } else {
            hit(&mut engine, 55, 1000).unwrap();
        }
        assert_eq!(
            victim(&engine).life.damage_health(),
            Some((4, 400)),
            "gear={gear} slot={slot} host={category} {attachment:?} mounted={mounted}",
        );
        assert!(
            std::str::from_utf8(&engine.writer)
                .unwrap()
                .lines()
                .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                .any(|entry| entry["gameplay_denial"]
                    .as_str()
                    .is_some_and(|code| code.starts_with("unsupported_bullet_attachment"))),
            "gear={gear} slot={slot} host={category} {attachment:?} mounted={mounted}",
        );
    }
}

#[test]
fn native_rifle_hit_keeps_host_ammo_with_separate_underbarrel_launcher() {
    for attachments in [
        vec![],
        vec![(17, "kAttachTypeLauncher")],
        vec![(16, "kAttachTypeSuppressor"), (17, "kAttachTypeLauncher")],
    ] {
        let mut engine = attachment_hit_fixture(
            &attachments,
            PrimaryAttachment::Compatible,
            "kWepCategoryAssaultRifle",
        );
        hit(&mut engine, 55, 1000).unwrap();
        assert_eq!(
            victim(&engine).life.damage_health(),
            Some((4, 240)),
            "{attachments:?}"
        );
    }
}

#[test]
fn native_rifle_hit_keeps_unqualified_underbarrel_launchers_denied() {
    use PrimaryAttachment::*;
    for (slot, category, attachment, mounted) in [
        (17, "kWepCategoryAssaultRifle", MissingLauncherWeapon, false),
        (17, "kWepCategoryAssaultRifle", HostAsLauncherWeapon, false),
        (17, "kWepCategoryAssaultRifle", WrongLauncherCategory, false),
        (17, "kWepCategoryAssaultRifle", WrongLauncherAmmo, false),
        (17, "kWepCategoryAssaultRifle", MissingReplacement, false),
        (17, "kWepCategoryAssaultRifle", Incompatible, false),
        (17, "kWepCategoryAssaultRifle", WrongType, false),
        (17, "kWepCategoryAssaultRifle", Unbound, false),
        (17, "kWepCategoryAssaultRifle", MismatchedSource, false),
        (17, "kWepCategoryAssaultRifle", Compatible, true),
        (16, "kWepCategoryAssaultRifle", Compatible, false),
        (17, "kWepCategoryMachineGun", Compatible, false),
        (20, "kWepCategorySideArm", Compatible, false),
    ] {
        let mut engine =
            attachment_hit_fixture(&[(slot, "kAttachTypeLauncher")], attachment, category);
        if mounted {
            let seat = domination_helo::PassengerSeat {
                object_index: 0,
                seat: 0,
            };
            let active = engine.active.as_mut().unwrap();
            active.helo_seat = Some(seat);
            active.owner_fields.state = Some(seat.character_state());
        }
        if slot == 20 {
            equip_synthetic_sidearm(&mut engine);
            secondary_hit(&mut engine).unwrap();
        } else {
            hit(&mut engine, 55, 1000).unwrap();
        }
        assert_eq!(
            victim(&engine).life.damage_health(),
            Some((4, 400)),
            "slot={slot} host={category} {attachment:?} mounted={mounted}",
        );
        assert!(
            std::str::from_utf8(&engine.writer)
                .unwrap()
                .lines()
                .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                .any(|entry| entry["gameplay_denial"]
                    .as_str()
                    .is_some_and(|code| code.starts_with("unsupported_bullet_attachment")))
        );
    }
}

#[test]
fn reconnect_without_progression_restores_only_the_same_characters_round_statistics() {
    let mut engine = fixture();
    let original_id = engine.active.as_ref().unwrap().reservation.id.clone();
    let account = engine.active.as_ref().unwrap().reservation.account;
    let original_character = {
        let actor = engine.active.as_mut().unwrap();
        actor.reservation.character.loadout_account_id =
            actor.reservation.character.account_id.take();
        assert!(actor.reservation.character.progression.is_none());
        actor.reservation.character.clone()
    };

    hit(&mut engine, 10, 1_000).unwrap();
    assert_eq!(
        engine.active.as_ref().unwrap().reward_statistics.get(&10),
        Some(&1)
    );
    assert_eq!(victim(&engine).reward_statistics.get(&12), Some(&1));
    assert!(engine.pending_reward_batch().is_empty());
    // Represent counters already earned by this character earlier in the round.
    engine
        .active
        .as_mut()
        .unwrap()
        .reward_statistics
        .extend([(11, 2), (12, 3), (24, 4)]);
    let round = engine.reward_match_id.clone();

    let release = |engine: &mut Engine<Vec<u8>>, id: &str, time| {
        engine.handle(
            Request::Release {
                v: BRIDGE_VERSION,
                request_id: format!("release-{id}"),
                reservation_id: id.into(),
            },
            Duration::from_millis(time),
            1,
        )
    };
    let reserve = |engine: &mut Engine<Vec<u8>>, id: &str, player, account, character, time| {
        let reply = engine.handle(
            Request::Reserve {
                v: BRIDGE_VERSION,
                request_id: format!("reserve-{id}"),
                reservation_id: id.into(),
                world_id: 1,
                player_id: player,
                account_id: account,
                session_key: "aaaaaaaaaaaaaaaa".into(),
                access_key: "bbbbbbbbbbbbbbbb".into(),
                character,
            },
            Duration::from_millis(time),
            1,
        );
        assert!(reply.ok, "{:?}", reply.error);
    };
    assert!(release(&mut engine, &original_id, 1_101).ok);

    let mut different_character = original_character.clone();
    different_character.character_id = Some(17);
    let replacement_id = "synthetic-other-character";
    reserve(
        &mut engine,
        replacement_id,
        1,
        17,
        different_character,
        1_200,
    );
    assert!(engine.active.as_ref().unwrap().reward_statistics.is_empty());

    let returning_id = "synthetic-same-character";
    reserve(
        &mut engine,
        returning_id,
        2,
        account,
        original_character.clone(),
        1_300,
    );
    assert_eq!(engine.reward_match_id, round);
    let actor = engine.active.as_mut().unwrap();
    assert_eq!(actor.reservation.id, returning_id);
    assert!(progression::RewardRecipient::from_active(actor).is_none());
    assert_eq!(actor.match_experience, 0);
    actor.claimed = Some(Claimed {
        wire: Vec::new(),
        np_identity: [0; 36],
        game_info: protocol::GameInfo {
            client_index: actor.reservation.player,
            level_name: "synthetic_arena".into(),
            start_time_ms: 0,
            attacking_faction: 1,
            defending_faction: 1,
            maximum_points: [100; 2],
            current_points: [100; 2],
            game_type: 0,
        },
    });
    let mut ready = engine
        .wire_profile
        .wire_message_type(5)
        .to_be_bytes()
        .to_vec();
    ready.push(0x80);
    let messages = dispatch(&mut engine, &ready, 1_400).unwrap();
    assert!(
        messages.iter().any(|message| {
            message.reservation_id == returning_id
                && message.payload_hex == "02040002040a010b020c031804"
        }),
        "{messages:?}"
    );

    assert!(release(&mut engine, returning_id, 1_500).ok);
    assert!(release(&mut engine, replacement_id, 1_501).ok);
    assert!(release(&mut engine, "synthetic-reservation-0000", 1_502).ok);
    assert!(engine.round_participants.is_empty());
    reserve(
        &mut engine,
        "synthetic-new-round",
        1,
        account,
        original_character,
        1_600,
    );
    assert_ne!(engine.reward_match_id, round);
    assert!(engine.active.as_ref().unwrap().reward_statistics.is_empty());
    assert!(engine.pending_reward_batch().is_empty());
    let actor = engine.active.as_mut().unwrap();
    actor.reward_statistics.insert(10, 1);
    engine.max_log = engine.written;
    let reply = release(&mut engine, "synthetic-new-round", 1_601);
    assert_eq!(reply.error.as_deref(), Some("log_limit_reached"));
    assert_eq!(
        engine.active.as_ref().unwrap().reward_statistics.get(&10),
        Some(&1)
    );
    assert!(engine.round_participants.is_empty());
}
