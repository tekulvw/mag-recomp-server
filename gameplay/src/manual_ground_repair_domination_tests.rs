//! Domination Ground repair through the authenticated event and normal output paths.
use super::*;
use crate::{
    acquisition_ground_runtime as ground,
    acquisition_objective_runtime::Assignment,
    domination_gameplay_effects::OwnedAssignment,
    domination_gameplay_engine::tests::structural_repair::repair_fixture,
    manual_ground_repair_tests::repair,
    replication::{BitReader, CharacterPosition},
};
use std::sync::Arc;

const APC: u32 = 0x0100_0000;
const MOTOR_POOL: u32 = 0x0100_0008;
const GRENADE: u32 = 0x1080_0002;

fn source_catalogs(
    attacking_faction: u8,
) -> (spawn_catalog::SpawnCatalog, ground_vehicles::Catalog) {
    let defending_faction = (attacking_faction + 1) % 3;
    let mut locations = vec![
        json!({"index":0,"token":201,"kind":"insertion","position":{"x":1.,"y":2.,"z":1.}}),
        json!({"index":1,"token":202,"kind":"insertion","position":{"x":2.,"y":2.,"z":1.}}),
    ];
    let mut vehicles = Vec::new();
    let mut sources = Vec::new();
    let mut passenger_spawns = Vec::new();
    for index in 0..16 {
        let apc = index < 8;
        let team = u8::from(!apc);
        let faction = [attacking_faction, defending_faction][usize::from(team)];
        let spawn = index + 2;
        let token = 901 + index;
        let vehicle_type = 110 + u32::from(team);
        let kind = if apc {
            "vehicle_passenger"
        } else {
            "unsupported"
        };
        let position = json!({"x":50 + index,"y":2.,"z":50.});
        locations.push(json!({"index":spawn,"token":token,"kind":kind,"position":position}));
        let mut vehicle = json!({
            "index":index,"spawn_point_index":spawn,"spawn_point_token":token,
            "vehicle_type_token":vehicle_type,"team":team,"faction":faction,"modifier":0,
            "health_quarter_points":600,"position":position,"rotation":[0.,0.,0.,1.]
        });
        if apc {
            vehicle["passenger_seats"] = json!([2]);
            vehicle["maximum_occupancy"] = json!(1);
            vehicle["passenger_exit_candidates"] =
                json!([{"ranking_offset":[1.,0.,0.],"position_offset":[1.,0.,0.]}]);
            passenger_spawns.push(json!({
                "vehicle_index":index,"spawn_point_index":spawn,"spawn_point_token":token
            }));
        }
        vehicles.push(vehicle);
        sources.push(json!({
            "vehicle_index":index,"spawn_point_index":spawn,"spawn_point_token":token,
            "vehicle_type_token":vehicle_type,"team":team,"spawn_type":if apc { 3 } else { 7 },
            "user_class":team,"source_family":35,"authored_active":true
        }));
    }
    let spawns = spawn_catalog::SpawnCatalog::from_json(
        &json!({
            "format":"mag_spawn_catalog","schema_version":1,"source":"synthetic Ground points",
            "active":{"map_name":"synthetic","squad_index":0,"default_spawn":0,"available":[0]},
            "match_profile":{"game_type":2,"attacking_faction":attacking_faction,
                "has_bunkers":false,"has_objectives":true},
            "maps":[{"name":"synthetic","spawn_count":locations.len(),"locations":locations,
                "owner_faction":defending_faction,"runtime_indices_verified":true,
                "runtime_indices_basis":"synthetic Ground point identities","squads":[
                    {"index":0,"team":0,"platoon":0,"squad":0,"faction":null,
                        "available":[0],"default_spawn":0},
                    {"index":16,"team":1,"platoon":0,"squad":0,"faction":defending_faction,
                        "available":[1],"default_spawn":1}
                ]}]
        })
        .to_string(),
    )
    .unwrap();
    let catalog = ground_vehicles::Catalog::from_json(
        &json!({
            "format":"mag_ground_vehicles","schema_version":1,
            "source":{"spawn_catalog_sha256":spawns.source_sha256().unwrap(),
                "native_elf_sha256":"a".repeat(64),"native_binding_sha256":"b".repeat(64),
                "vehicle_properties_sha256":"c".repeat(64),
                "creation_scope":"native_domination_vehicle_source_points"},
            "vehicles":vehicles,"vehicle_sources":sources,"passenger_spawns":passenger_spawns
        })
        .to_string(),
        &spawns,
    )
    .unwrap();
    (spawns, catalog)
}

pub(crate) fn fixture(attacking_faction: u8, team: u8) -> Engine<Vec<u8>> {
    let mut engine = repair_fixture();
    let (spawns, catalog) = source_catalogs(attacking_faction);
    let squad = u16::from(team) * 16;
    let actor = engine.active.as_mut().unwrap();
    actor.squad_catalog = Arc::new(spawns.for_squad(squad).unwrap());
    actor.reservation.character.faction = (attacking_faction + team) % 3;
    actor.ground_vehicle_batch = catalog.batch_count();
    actor.pending_shared.clear();
    engine.ground_runtime = Some(ground::Runtime::from_catalog(&catalog).unwrap());
    engine.ground_vehicles = Some(catalog);
    engine.spawn_catalog = Some(spawns);
    engine
        .domination_gameplay
        .as_mut()
        .unwrap()
        .state
        .assignments[usize::from(squad)] = Some(OwnedAssignment {
        assignment: Assignment {
            primary_index: 1,
            tagged_object: u32::MAX,
        },
        manual_tag: u32::MAX,
    });
    engine
}

fn add_peer(engine: &mut Engine<Vec<u8>>, player: u16, team: u8, faction: u8) -> String {
    let mut peer = repair_fixture().active.take().unwrap();
    peer.reservation.id = format!("domination-repair-peer-{player}");
    peer.reservation.player = player;
    peer.reservation.account = u32::from(player) + 1;
    peer.reservation.character.account_id = Some(peer.reservation.id.clone());
    peer.reservation.character.character_id = Some(u32::from(player) + 1);
    peer.reservation.character.faction = faction;
    peer.squad_catalog = Arc::new(
        engine
            .spawn_catalog
            .as_ref()
            .unwrap()
            .for_squad(u16::from(team) * 16)
            .unwrap(),
    );
    peer.ground_vehicle_batch = engine.ground_vehicles.as_ref().unwrap().batch_count();
    let id = peer.reservation.id.clone();
    engine.peers.insert(id.clone(), peer);
    id
}

fn vehicle(engine: &Engine<Vec<u8>>, id: u32) -> &ground::Vehicle {
    engine
        .ground_runtime
        .as_ref()
        .unwrap()
        .get(id as u16)
        .unwrap()
}

fn damage(engine: &mut Engine<Vec<u8>>, id: u32, last_attacker: u32) {
    let key = vehicle(engine, id).key;
    engine
        .ground_runtime
        .as_mut()
        .unwrap()
        .apply_grenade_damage(key, 40., last_attacker, GRENADE)
        .unwrap();
}

#[test]
fn domination_apc_repair_qualifies_frago_at_the_live_target_for_each_attacking_faction() {
    for faction in 0..3 {
        let mut engine = fixture(faction, 0);
        let former_attacker = add_peer(&mut engine, 2, 1, faction);
        assert_eq!(engine.peers[&former_attacker].team(), 1);
        let passenger = add_peer(&mut engine, 1, 0, faction);
        let seat = Some(ground_seats::PassengerSeat {
            vehicle_index: 0,
            seat: 2,
        });
        engine.peers.get_mut(&passenger).unwrap().ground_seat = seat;
        damage(&mut engine, APC, 0x0380_0002);
        let key = vehicle(&engine, APC).key;
        engine
            .ground_runtime
            .as_mut()
            .unwrap()
            .force_pose(
                key,
                ground::Pose {
                    position: CharacterPosition {
                        x: 2.,
                        y: 2.,
                        z: 1.,
                    },
                    rotation: [0., 0., 0., 1.],
                },
            )
            .unwrap();
        let before = vehicle(&engine, APC).clone();
        let messages = repair(&mut engine, APC, 2000, Transport::Tcp).unwrap();
        let after = vehicle(&engine, APC);
        assert_eq!(
            after.health_quarter_points, 508,
            "attacking faction {faction}"
        );
        assert_eq!(
            (after.key, after.pose, after.state, &after.body),
            (before.key, before.pose, before.state, &before.body)
        );
        assert_eq!(after.damage_wire().last_attacker, 0x0380_0002);
        assert_eq!(after.damage_wire().last_source_object, GRENADE);
        assert_eq!(after.damage_wire().last_location_and_type, 0xa0);
        assert_eq!(engine.peers[&passenger].ground_seat, seat);
        let active = engine.active.as_ref().unwrap();
        assert_eq!(active.reward_statistics.get(&35), Some(&1));
        assert_eq!(messages.len(), 3);
        assert!(
            messages
                .iter()
                .all(|packet| packet.payload_hex == "02040000012301")
        );
        let rewards = engine.pending_reward_batch();
        assert_eq!(rewards.len(), 1);
        assert_eq!(rewards[0].kind, progression::RewardKind::RepairUnits);
        assert_eq!(
            (rewards[0].player_id, rewards[0].units, rewards[0].frago),
            (0, 1, Some(true))
        );
        assert_eq!(active.pending_shared.len(), 1);
        let mut bits = BitReader::with_profile(&active.pending_shared[0], engine.wire_profile);
        bits.position = engine.wire_profile.header_bits();
        assert_eq!(bits.read(1), Ok(1));
        assert_eq!(bits.read(16), Ok(0));
        for _ in 0..18 {
            assert_eq!(bits.read(1), Ok(0));
        }
        assert_eq!(bits.read(1), Ok(1));
        assert_eq!(bits.read(8), Ok(0xa0));
        assert_eq!(bits.read(16), Ok(508));
        assert_eq!(bits.read(32), Ok(0x0380_0002));
        assert_eq!(bits.read(32), Ok(GRENADE));
        assert_eq!(bits.read(32), Ok(0));
        for _ in 19..30 {
            assert_eq!(bits.read(1), Ok(0));
        }
        assert_eq!(bits.read(1), Ok(0));
    }
}

#[test]
fn domination_repair_respects_actual_target_team_and_friendly_damage_credit_policy() {
    let mut engine = fixture(1, 1);
    engine
        .active
        .as_mut()
        .unwrap()
        .reservation
        .character
        .faction = 1;
    add_peer(&mut engine, 2, 1, 2);
    damage(&mut engine, APC, 0x0380_0002);
    damage(&mut engine, MOTOR_POOL, 0x0380_0002);
    assert!(
        repair(&mut engine, APC, 2000, Transport::Tcp)
            .unwrap_or_default()
            .is_empty()
    );
    assert_eq!(vehicle(&engine, APC).health_quarter_points, 440);
    assert!(engine.next_repair.is_empty());
    assert!(
        repair(&mut engine, MOTOR_POOL, 2000, Transport::Tcp)
            .unwrap()
            .is_empty()
    );
    assert_eq!(vehicle(&engine, MOTOR_POOL).health_quarter_points, 508);
    let active = engine.active.as_ref().unwrap();
    assert_eq!(active.repair_credit, repair_rewards::State::default());
    assert_eq!(active.reward_statistics.get(&35), None);
    assert!(engine.pending_reward_batch().is_empty());
    assert_eq!(active.pending_shared.len(), 1);
}

#[test]
fn domination_ground_repair_record_failure_preserves_hp_cooldown_and_publications() {
    let mut engine = fixture(1, 0);
    damage(&mut engine, APC, u32::MAX);
    let ground_before = engine.ground_runtime.clone();
    let limit = engine.max_log;
    engine.max_log = engine.written;
    assert_eq!(
        repair(&mut engine, APC, 2000, Transport::Tcp).err(),
        Some("log_limit_reached")
    );
    assert_eq!(engine.ground_runtime, ground_before);
    assert!(engine.next_repair.is_empty());
    assert!(engine.pending_reward_batch().is_empty());
    let active = engine.active.as_ref().unwrap();
    assert_eq!(active.repair_credit, repair_rewards::State::default());
    assert_eq!(active.reward_statistics.get(&35), None);
    assert!(active.pending_shared.is_empty());
    engine.failed = false;
    engine.max_log = limit;

    assert_eq!(
        repair(&mut engine, APC, 2000, Transport::Tcp)
            .unwrap()
            .len(),
        1
    );
    assert!(
        repair(&mut engine, APC, 3249, Transport::Tcp)
            .unwrap_or_default()
            .is_empty()
    );
    assert_eq!(vehicle(&engine, APC).health_quarter_points, 508);
    assert_eq!(engine.pending_reward_batch().len(), 1);
    assert_eq!(
        repair(&mut engine, APC, 3250, Transport::Tcp)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(vehicle(&engine, APC).health_quarter_points, 576);
    assert_eq!(engine.pending_reward_batch().len(), 2);
    assert_eq!(
        engine.active.as_ref().unwrap().reward_statistics.get(&35),
        Some(&2)
    );
}
