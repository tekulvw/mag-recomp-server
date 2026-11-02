//! Synthetic legacy bunker event regressions; no game assets or captures.
use super::*;
use crate::spawn_progression::{engine, ready, reserve};
use std::sync::Arc;

fn catalog() -> StructureCatalog {
    StructureCatalog::from_value(json!({
        "schema_version":1,"application_id":20230,
        "native_rules":{"health_condition_threshold_percent":[70.,40.,15.],
            "dead_below_health":1.,"minimum_rebuilt_health_percent":15.},
        "resources":{
            "settings/mag_game_settings.lua":{"MAGGameRules":{"FriendlyFireDamageMultiplier":0.25}},
            "settings/gameproperties.zpr":{"GameCreationPropertiesTable":{"m_gameCreationTypes":{
                "synthetic":{"GameCreationType":{"m_gameRules":{"FriendlyFireDamageMultiplier":0.25}}}}}},
            "vehicle_damagemodel.lua":{"ArmorTypeInfoList":{"ArmorTypes":{
                "ArmorTypeInfo0":{"ArmorTypeInfo":{"rating":6.,"prevention":0.5,"m_coverage":[1.,1.,1.,1.]}}}}},
            "vehicle_collision_info.lua":{"VehicleCollisionData":{"CollisionBodyMapping":
                (0..16).map(|i|(format!("CollisionBodyInfo{i}"),json!({"CollisionBodyInfo":{
                    "armorArea":0,"hitpointArea":0,"hitpointMult":1.}}))).collect::<serde_json::Map<_,_>>()}}
        },
        "repair":{"item_id":99,"parameters":{"activeitem_type":"kActiveItemRepairkit","cooldown_time":1.},
            "globals":{"repairkit_repair_amount":10.,"repairkit_repair_distance":3.,"repairkit_repair_radius":1.},
            "skill_modifiers":{}},
        "profiles":(0..3).map(|faction|json!({"vehicle_type":"DefensiveEmplacement","faction":faction,
            "model":"synthetic","properties_source":format!("synthetic-{faction}"),
            "damage_properties":{"MaxHitpoints":200.,"ArmorType":1,"DamageCriticalHealthLossRate":0.,
                "CanBeRepairedFromDestroyed":true,"RepairedHealthPct":50.,"TimeToRepairInSeconds":-1.}
        })).collect::<Vec<_>>()
    })).unwrap()
}

fn fixture(actor_team: u8, bunker_team: u8, same_faction: bool) -> Engine<Vec<u8>> {
    let mut engine = engine();
    reserve(&mut engine, 0);
    engine.wire_profile = engine.wire_profiles.profile(20230).unwrap();
    engine.item_catalog = item_catalog::bunker_test_items::catalog();
    let actor = engine.active.as_mut().unwrap();
    ready(actor);
    actor.wire_profile = engine.wire_profile;
    actor.squad_catalog = Arc::new(
        actor
            .squad_catalog
            .for_squad(u16::from(actor_team) * 16)
            .unwrap(),
    );
    actor.reservation.character.faction = if same_faction { 0 } else { 2 };
    actor.life = CharacterLife::Alive {
        generation: 1,
        health_quarter_points: 400,
    };
    let handle = replication::InventoryHandle {
        update_id: 1,
        inventory_index: 0,
    };
    let equipped = replication::InventoryItem {
        inventory_update_id: 1,
        gear_slot: 0,
        ammo_pool: 0,
        ammo_count: 7,
        reloading: false,
    };
    let mut slots = [None; 22];
    slots[0] = Some(13);
    slots[3] = Some(99);
    actor.spawned_loadout = Some(replication::CharacterLoadout {
        inventories: std::array::from_fn(|_| replication::CharacterInventory {
            update_id: 1,
            slots,
        }),
        handle,
        primary: equipped,
        primary_equipped: true,
    });
    actor.current_inventory_handle = Some(handle);
    actor.observed_primary = Some(equipped);
    actor.primary_equipped = true;
    let catalog = catalog();
    let profile = catalog.profile(0).unwrap().clone();
    let health = BunkerHealth::for_profile(&profile, &catalog).unwrap();
    engine.bunker_world = Some(BunkerWorld {
        catalog,
        bunkers: BTreeMap::from([(
            0,
            Bunker {
                id: "synthetic-bunker".into(),
                spawn: 0,
                faction: 0,
                team: bunker_team,
                last_health_update: Duration::ZERO,
                repair_points: vec![],
                profile,
                health,
                damage: VehicleDamage {
                    last_location_and_type: 0,
                    health_quarter_points: 800,
                    conditions: 0,
                    last_attacker: u32::MAX,
                    last_source_object: u32::MAX,
                    last_user_data: 0,
                },
            },
        )]),
        vehicle_token: 1,
        pending_placements: 0,
        commands: BTreeMap::new(),
    });
    engine
}
fn health(engine: &Engine<Vec<u8>>) -> f32 {
    engine.bunker_world.as_ref().unwrap().bunkers[&0]
        .health
        .hp()
}
fn repair(engine: &mut Engine<Vec<u8>>) -> Vec<OutboundMessage> {
    engine
        .structure_event(
            &Ok(protocol::GameMessage::Repair(protocol::Repair {
                target_object_id: 0x0d80_0000,
            })),
            Duration::from_secs(1),
            1,
        )
        .unwrap()
        .unwrap()
}
fn equip_repair(engine: &mut Engine<Vec<u8>>) {
    let actor = engine.active.as_mut().unwrap();
    actor.observed_primary.as_mut().unwrap().gear_slot = 3;
    engine
        .bunker_world
        .as_mut()
        .unwrap()
        .bunkers
        .get_mut(&0)
        .unwrap()
        .health
        .damage(40.)
        .unwrap();
}
fn bullet(engine: &mut Engine<Vec<u8>>, penetration_milli: u16) -> Vec<OutboundMessage> {
    engine
        .structure_event(
            &Ok(protocol::GameMessage::Hit(bullet_hit::NetMessageHit {
                gear_index: 0,
                fire_mode: 0,
                hit_location: 0,
                client_data: 0,
                defender: 0x0d80_0000,
                hit_origin: [0.; 3],
                hit_dest: [1.; 3],
                penetration_milli,
            })),
            Duration::from_secs(1),
            1,
        )
        .unwrap()
        .unwrap()
}

#[test]
fn repair_rejects_same_faction_on_opposing_actual_teams() {
    for (actor_team, bunker_team) in [(0, 1), (1, 0)] {
        let mut engine = fixture(actor_team, bunker_team, true);
        equip_repair(&mut engine);
        assert!(repair(&mut engine).is_empty());
        assert_eq!(health(&engine), 160.);
        assert!(
            std::str::from_utf8(&engine.writer)
                .unwrap()
                .contains("repair_requires_allied_onfoot_actor")
        );
    }
}
#[test]
fn repair_admits_different_factions_on_the_same_actual_team() {
    for team in [0, 1] {
        let mut engine = fixture(team, team, false);
        equip_repair(&mut engine);
        assert_eq!(repair(&mut engine).len(), 1);
        assert_eq!(health(&engine), 170.);
        assert_eq!(
            engine.next_repair["synthetic-reservation-0000"],
            Duration::from_secs(2)
        );
    }
}
#[test]
fn bullets_scale_allied_damage_on_either_actual_team_before_vehicle_armor() {
    for team in [0, 1] {
        for same_faction in [false, true] {
            let mut engine = fixture(team, team, same_faction);
            assert_eq!(bullet(&mut engine, 500).len(), 1);
            // 80 * .5 penetration * .25 friendly scale, then (10 - 6) * .5 armor = 2.
            assert_eq!(health(&engine), 198.);
        }
    }
}
#[test]
fn bullets_do_full_raw_damage_to_opposing_actual_teams_in_both_directions() {
    for (actor_team, bunker_team) in [(0, 1), (1, 0)] {
        for same_faction in [false, true] {
            let mut engine = fixture(actor_team, bunker_team, same_faction);
            assert_eq!(bullet(&mut engine, 500).len(), 1);
            // Full raw 80 * .5 penetration, then (40 - 6) * .5 armor = 17.
            assert_eq!(health(&engine), 183.);
        }
    }
}
#[test]
fn pending_already_resolved_damage_is_not_scaled_or_armored_again() {
    for team in [0, 1] {
        let mut engine = fixture(team, team, true);
        engine
            .pending_bunker_damage
            .push_back((0, 17., "synthetic-resolved".into()));
        let update = replication::ObjectUpdate {
            header: UpdateHeader {
                sequence: 1,
                nak_sequence: 0,
                client_index: 0,
                object_type_mask: 0,
                timestamp_ms: 1,
            },
            wire: &[],
        };
        assert_eq!(
            engine
                .structure_event(
                    &Ok(protocol::GameMessage::ObjectUpdate(update)),
                    Duration::from_secs(1),
                    1
                )
                .unwrap()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(health(&engine), 183.);
        assert!(engine.pending_bunker_damage.is_empty());
    }
}
#[test]
fn allied_repair_still_requires_onfoot_equipment_ready_life_and_cooldown() {
    for guard in [
        "mounted", "unready", "dead", "tool", "cooldown", "healthy", "critical", "rebuild",
    ] {
        let mut engine = fixture(0, 0, true);
        equip_repair(&mut engine);
        match guard {
            "mounted" => engine.active.as_mut().unwrap().mounted_seat = Some(0),
            "unready" => engine.active.as_mut().unwrap().ready_for_updates = false,
            "dead" => {
                engine.active.as_mut().unwrap().life =
                    CharacterLife::AwaitingRespawn { generation: 1 }
            }
            "tool" => {
                engine
                    .active
                    .as_mut()
                    .unwrap()
                    .observed_primary
                    .as_mut()
                    .unwrap()
                    .gear_slot = 0
            }
            "cooldown" => {
                engine
                    .next_repair
                    .insert("synthetic-reservation-0000".into(), Duration::from_secs(2));
            }
            "healthy" => engine
                .bunker_world
                .as_mut()
                .unwrap()
                .bunkers
                .get_mut(&0)
                .unwrap()
                .health
                .repair(40.)
                .unwrap(),
            "critical" => engine
                .bunker_world
                .as_mut()
                .unwrap()
                .bunkers
                .get_mut(&0)
                .unwrap()
                .health
                .damage(140.)
                .unwrap(),
            "rebuild" => {
                let b = engine
                    .bunker_world
                    .as_mut()
                    .unwrap()
                    .bunkers
                    .get_mut(&0)
                    .unwrap();
                b.health.damage(200.).unwrap();
                b.profile.can_repair_destroyed = false;
            }
            _ => unreachable!(),
        }
        let before = health(&engine);
        assert!(repair(&mut engine).is_empty(), "{guard}");
        assert_eq!(health(&engine), before, "{guard}");
    }
}

#[test]
fn retail_profile_does_not_gain_legacy_repair_authority() {
    let mut engine = fixture(0, 0, true);
    equip_repair(&mut engine);
    engine.wire_profile = engine.wire_profiles.profile(20231).unwrap();
    engine.active.as_mut().unwrap().wire_profile = engine.wire_profile;
    assert!(repair(&mut engine).is_empty());
    assert_eq!(health(&engine), 160.);
    assert!(
        std::str::from_utf8(&engine.writer)
            .unwrap()
            .contains("repair_inventory_unavailable")
    );
}

#[test]
fn bunker_hits_preserve_weapon_mount_and_life_admission() {
    for guard in ["wrong_gear", "passenger", "unready", "dead", "critical"] {
        let mut engine = fixture(0, 0, true);
        match guard {
            "wrong_gear" => {
                engine
                    .active
                    .as_mut()
                    .unwrap()
                    .observed_primary
                    .as_mut()
                    .unwrap()
                    .gear_slot = 1
            }
            "passenger" => engine.active.as_mut().unwrap().mounted_seat = Some(1),
            "unready" => engine.active.as_mut().unwrap().ready_for_updates = false,
            "dead" => {
                engine.active.as_mut().unwrap().life =
                    CharacterLife::AwaitingRespawn { generation: 1 }
            }
            "critical" => engine
                .bunker_world
                .as_mut()
                .unwrap()
                .bunkers
                .get_mut(&0)
                .unwrap()
                .health
                .damage(180.)
                .unwrap(),
            _ => unreachable!(),
        }
        let before = health(&engine);
        assert!(bullet(&mut engine, 1000).is_empty(), "{guard}");
        assert_eq!(health(&engine), before, "{guard}");
    }
}
