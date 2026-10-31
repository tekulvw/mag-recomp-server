//! SYNTHETIC fixtures composed from schema constraints. No captured map data.
use super::*;
use crate::objective_catalog::ObjectiveCatalog;
use crate::replication::{
    BitReader, CharacterPosition, ObjectUpdate, ObjectiveState, UpdateHeader,
};
use crate::spawn_catalog::SpawnCatalog;
use std::sync::Arc;

fn spawn_json() -> Value {
    let mut locations = vec![
        json!({"index":0,"token":100,"kind":"on_foot","position":{"x":100.,"y":3.,"z":100.}}),
        json!({"index":1,"token":101,"kind":"on_foot","position":{"x":120.,"y":3.,"z":100.}}),
    ];
    locations.extend((0..4).map(|slot| {
        json!({
            "index":slot+2,"token":102+slot,"kind":"parachute","active":false,
            "position":{"x":200.+slot as f32*20.,"y":80.,"z":300.},
            "parachute":{"forward":[0.6,0.,0.8],"spacing_m":5.,"yaw":377}
        })
    }));
    json!({"format":"mag_spawn_catalog","schema_version":1,"source":"SYNTHETIC progression test",
    "active":{"map_name":"synthetic_arena","squad_index":0,"default_spawn":0,"available":[0,1]},
    "match_profile":{"game_type":0,"auto_assign_teams":true,"team_capacity":8,"squad_capacity":8,
        "has_bunkers":false,"has_objectives":true,"attacking_faction":1,"spawn_wave_ms":[100,100],
        "initial_spawn_delay_ms":[100,100]},
    "maps":[{"name":"synthetic_arena","owner_faction":0,"spawn_count":6,
        "runtime_indices_verified":true,"runtime_indices_basis":"SYNTHETIC pool identity assertion",
        "locations":locations,
        "squads":(0..2).flat_map(|team| (0..4).map(move |slot| json!({
            "index":team*16+slot,"team":team,"platoon":0,"squad":slot,
            "faction":if team==1 {Some(0)} else {None},"available":[0,1],"default_spawn":0
        }))).collect::<Vec<_>>(),
        "forward_spawns":(0..4).map(|slot| json!({
            "objective_index":20+slot,"squad_index":slot,"spawn_point_index":slot+2
        })).collect::<Vec<_>>()
    }]})
}
fn spawns(value: Value) -> std::io::Result<SpawnCatalog> {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "mag-synthetic-forward-{}-{}.json",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::write(&path, value.to_string()).unwrap();
    let catalog = SpawnCatalog::load(&path);
    std::fs::remove_file(&path).unwrap();
    catalog
}
fn objective_json() -> Value {
    let mut objects: Vec<_> = [10, 11]
        .into_iter()
        .map(|index| {
            json!({
                "token":format!("0x{index:08x}"),"object_index":index,"initial_state":1,
                "interaction":{"owner_team":1,"activate_ms":10,"complete_ms":20,"deactivate_ms":10,
                    "can_deactivate":true,"position":[100.,3.,100.],"trigger_count":1,
                    "maximum_distance_m":2.,"distance_policy":"development_point_distance"}
            })
        })
        .collect();
    objects.extend((20..24).map(|index| {
        json!({
            "token":format!("0x{index:08x}"),"object_index":index,"initial_state":0,
            "scoring":{"owner_team":1,"activation_score":0,"deactivation_score":0}
        })
    }));
    objects.push(json!({"token":"0x0000001e","object_index":30,"initial_state":9}));
    json!({"schema_version":1,"format":"mag_retail_objective_catalog","application_id":20231,
        "protocol":protocol::ProtocolInfo::RETAIL_PROTOCOL,"game_type":0,"map_name":"synthetic_arena",
        "entity_family":12,"network_type":12,
        "source":{"authored_catalog_sha256":"0".repeat(64),"binding_capture_sha256":"1".repeat(64),
            "startup_proof_sha256":"2".repeat(64),"source_image_sha256":"3".repeat(64),
            "capture_qualification":"bounded_concurrent_token_slot_capture"},
        "objects":objects,
        "squads":(0..2).flat_map(|team| (0..4).map(move |slot| json!({
            "team":team,"squad_index":team*16+slot,"initial_objective_index":10,
            "authored_objectives":[{"object_index":10,"order":1},{"object_index":11,"order":2},
                {"object_index":30,"order":3}]
        }))).collect::<Vec<_>>(),
        "progression":{"prerequisites":[10,11],"lock":[10,11],"complete":[20,21,22,23],
            "delayed_objective":30,"delay_ms":1000}})
}
fn objectives() -> ObjectiveCatalog {
    ObjectiveCatalog::from_json(&objective_json().to_string()).unwrap()
}
fn items() -> Arc<item_catalog::ItemCatalog> {
    // A minimal independently authored weapon/armory supplies real inventory fields.
    let raw = json!({"schema_version":2,"application_id":20231,
        "items":{"FactionItemList0:2":{"authored_ref":"FactionItemList0:2",
            "faction_list":"FactionItemList0","local_ordinal":2,"wire_item_id":null,
            "name":"synthetic_weapon","item_type":"kWeapon",
            "definition":{"group":"weapons","name":"synthetic_weapon"}}},
        "definitions":{"weapons":{"synthetic_weapon":{"parameters":{"magazine_capacity":7}}}},"resources":{}});
    let catalog = item_catalog::ItemCatalog::from_value_for_application(
        raw,
        blast_rules::CharacterBlastRules {
            armor_rating_reduction: 0.25,
            armor_prevention_reduction: 0.75,
            hit_location: 3,
            damage_type: 10,
        },
        20231,
    )
    .unwrap();
    let armory = retail_armory::RetailArmory::from_json(&json!({
        "schema_version":1,"application_id":20231,"source_sha256":{"synthetic":"a".repeat(64)},
        "items":[{"wire_id":2,"name":"synthetic_weapon","authored_ref":"FactionItemList0:2",
            "available_bit":2,"factions":[0,1,2],"primary_ammo":7}],
        "factions":(0..3).map(|faction| json!({"faction":faction,"available_item_bits":[2],"skill_ids":[],
            "loadouts":(0..5).map(|_|json!({"update_id":1,"items":vec![None::<u16>;22]})).collect::<Vec<_>>()
        })).collect::<Vec<_>>()
    }).to_string()).unwrap();
    Arc::new(catalog.with_retail_armory(armory))
}
fn engine() -> Engine<Vec<u8>> {
    let resources = EngineResources::for_application(
        items(),
        spawns(spawn_json()).unwrap(),
        None,
        wire_profile::WireProfileCatalog::embedded(),
        20231,
    )
    .unwrap();
    Engine::from_resources(Vec::new(), MAX_LOG, resources)
        .with_objective_catalog(objectives())
        .unwrap()
        .with_match_limits(1, 16)
        .unwrap()
}
fn reserve(engine: &mut Engine<Vec<u8>>, player: u16) {
    let mut saved = loadouts::SavedLoadouts {
        selected_index: 0,
        update_ids: [1; 5],
        inventories: [[None; 22]; 5],
    };
    saved.inventories[0][0] = Some(2);
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
            character: serde_json::from_value(
                json!({"name":format!("Synthetic {player}"),"faction":1,"head":0,
            "voice":0,"universe_id":26,"character_id":u32::from(player)+1,"loadouts":saved}),
            )
            .unwrap(),
        },
        Duration::ZERO,
        1,
    );
    assert!(response.ok, "{:?}", response.error);
}
fn actors(engine: &mut Engine<Vec<u8>>) -> impl Iterator<Item = &mut Active> {
    engine.peers.values_mut().chain(engine.active.iter_mut())
}
fn ready(actor: &mut Active) {
    actor.ready_for_updates = true;
    actor.entered_game = true;
    actor.initial_objects = Some(Vec::new());
    actor.emplacement_creation = Some(Vec::new());
    actor.objective_delta = Some(Vec::new());
    actor.objective_assigned = true;
    actor.objective_needs_clear = false;
    actor.outbound_sequence = 1;
    actor.tracking.server_position(
        CharacterPosition {
            x: 100.,
            y: 3.,
            z: 100.,
        },
        player_tracking::PositionOrigin::ServerSpawn,
        0,
    );
}
fn start_objective(engine: &mut Engine<Vec<u8>>, index: u16, now_ms: u64) {
    let actor = objective_control::ObjectiveActor {
        player: 0,
        team: 0,
        generation: 1,
        position: CharacterPosition {
            x: 100.,
            y: 3.,
            z: 100.,
        },
    };
    let result = engine.objective_world.as_mut().unwrap().interact(
        protocol::ObjectiveInteraction {
            target_object_id: 0x0c80_0000 | u32::from(index),
            kind: protocol::ObjectiveInteractionKind::Activate,
            actor_index: 0,
            trigger_index: 0,
        },
        actor,
        Duration::from_millis(now_ms),
    );
    assert!(result.accepted, "{}", result.reason);
}
fn complete_stage(engine: &mut Engine<Vec<u8>>) {
    start_objective(engine, 10, 0);
    engine
        .tick_objectives(Duration::from_millis(30), 1)
        .unwrap();
    start_objective(engine, 11, 40);
    engine
        .tick_objectives(Duration::from_millis(70), 1)
        .unwrap();
}
fn make_operator(engine: &mut Engine<Vec<u8>>) {
    reserve(engine, 0);
    let actor = engine.active.as_mut().unwrap();
    ready(actor);
    actor.life = CharacterLife::Alive {
        generation: 1,
        health_quarter_points: 400,
    };
}
fn dispatch(
    engine: &mut Engine<Vec<u8>>,
    bytes: &[u8],
    now_ms: u64,
) -> Result<Vec<OutboundMessage>, &'static str> {
    let actor = engine.active.as_ref().unwrap();
    let input = events::EventInput {
        reservation_id: actor.reservation.id.clone(),
        world_id: 1,
        player_id: actor.reservation.player,
        transport: Transport::Udp,
        direction: Direction::ClientToServer,
        rt_type: 0x3d,
        plugin_id: Some(1),
        payload_hex: protocol::encode_hex(bytes),
    };
    let decoded = DecodedEvent::with_profile(bytes, actor.reservation.player, engine.wire_profile);
    engine.handle_event(input, &decoded, Duration::from_millis(now_ms), 1)
}
fn owner_update(engine: &mut Engine<Vec<u8>>, now_ms: u64, nak: u32) -> Vec<OutboundMessage> {
    let header = UpdateHeader {
        sequence: engine.active.as_ref().unwrap().latest_owner_sequence + 1,
        nak_sequence: nak,
        client_index: 0,
        object_type_mask: 0,
        timestamp_ms: now_ms as u32,
    };
    let bytes = ObjectUpdate { header, wire: &[] }
        .encode_empty_with_profile(engine.wire_profile)
        .unwrap();
    dispatch(engine, &bytes, now_ms).unwrap()
}
fn select(engine: &mut Engine<Vec<u8>>, point: u16, now_ms: u64) {
    let mut bytes = 0x0219u16.to_be_bytes().to_vec();
    bytes.extend(point.to_be_bytes());
    dispatch(engine, &bytes, now_ms).unwrap();
}
fn queue_respawn(engine: &mut Engine<Vec<u8>>, now_ms: u64) {
    dispatch(engine, &0x0211u16.to_be_bytes(), now_ms).unwrap();
}
fn read_availability(packet: &[u8], profile: wire_profile::WireProfile) -> Vec<bool> {
    let header = ObjectUpdate::decode_with_profile(packet, profile)
        .unwrap()
        .header;
    assert_eq!(header.object_type_mask, 1 << 7);
    let mut bits = BitReader::with_profile(packet, profile);
    bits.position = profile.header_bits();
    assert_eq!(bits.read(1), Ok(1));
    assert_eq!(bits.read(16), Ok(0));
    assert_eq!(bits.read(1), Ok(0));
    assert_eq!(bits.read(1), Ok(0));
    assert_eq!(bits.read(1), Ok(1));
    assert_eq!(bits.read(8), Ok(6));
    assert_eq!(bits.read(1), Ok(1));
    let available = (0..128).map(|_| bits.read(1).unwrap() != 0).collect();
    assert_eq!(bits.read(6), Ok(0));
    assert_eq!(bits.read(1), Ok(0));
    available
}

#[test]
fn objective_completion_advances_every_squad_preserves_overrides_and_new_join_defaults() {
    let mut engine = engine();
    for player in 0..4 {
        reserve(&mut engine, player);
    }
    for actor in actors(&mut engine) {
        ready(actor);
        actor.life = CharacterLife::Alive {
            generation: 1,
            health_quarter_points: 400,
        };
        if actor.reservation.player == 0 {
            actor.selected_spawn = Some(protocol::CharacterSpawnSelection {
                spawn_point_index: 1,
            });
        }
        actor.pending_shared.clear();
    }
    start_objective(&mut engine, 10, 0);
    engine
        .tick_objectives(Duration::from_millis(10), 1)
        .unwrap();
    assert_eq!(
        engine.objective_world.as_ref().unwrap().state(10),
        Some(ObjectiveState::Activated)
    );
    for actor in actors(&mut engine) {
        assert!(
            actor
                .squad_catalog
                .alpha(actor.squad_catalog.squad_index() + 2)
                .is_none()
        );
    }
    engine
        .tick_objectives(Duration::from_millis(30), 1)
        .unwrap();
    assert_eq!(
        engine.objective_world.as_ref().unwrap().state(10),
        Some(ObjectiveState::Completed)
    );
    assert_eq!(
        engine.spawn_catalog.as_ref().unwrap().default_for_squad(0),
        Some(0)
    );
    start_objective(&mut engine, 11, 40);
    engine
        .tick_objectives(Duration::from_millis(70), 1)
        .unwrap();
    assert_eq!(
        engine.objective_world.as_ref().unwrap().state(30),
        Some(ObjectiveState::Locked)
    );
    let profile = engine.wire_profile;
    for actor in actors(&mut engine) {
        let point = actor.squad_catalog.squad_index() + 2;
        assert_eq!(actor.squad_spawn, point);
        assert_eq!(actor.squad_catalog.default_spawn(), point);
        assert!(actor.squad_catalog.alpha(point).is_some());
        assert_eq!(
            actor.selected_spawn.unwrap().spawn_point_index,
            if actor.reservation.player == 0 {
                1
            } else {
                point
            }
        );
        assert!(actor.life.is_alive());
        assert_eq!(actor.tracking.sample.unwrap().position.y, 3.);
        assert!(actor.spawn_delta.is_none());
        let availability = actor
            .pending_shared
            .iter()
            .find(|packet| {
                ObjectUpdate::decode_with_profile(packet, profile)
                    .unwrap()
                    .header
                    .object_type_mask
                    == 1 << 7
            })
            .unwrap();
        let flags = read_availability(availability, profile);
        assert!(flags[..6].iter().all(|v| *v));
        assert!(flags[6..].iter().all(|v| !*v));
        assert_eq!(
            actor
                .pending_shared
                .iter()
                .filter(|p| ObjectUpdate::decode_with_profile(p, profile)
                    .unwrap()
                    .header
                    .object_type_mask
                    == 1 << 8)
                .count(),
            4
        );
        assert_eq!(
            actor
                .pending_shared
                .iter()
                .filter(|p| ObjectUpdate::decode_with_profile(p, profile)
                    .unwrap()
                    .header
                    .object_type_mask
                    == 1 << 3)
                .count(),
            usize::from(actor.reservation.player != 0)
        );
        actor.pending_shared.clear();
    }
    engine
        .advance_forward_spawns(Duration::from_millis(80))
        .unwrap();
    assert!(actors(&mut engine).all(|a| a.pending_shared.is_empty()));
    reserve(&mut engine, 4);
    assert_eq!(
        engine.active.as_ref().unwrap().squad_catalog.squad_index(),
        0
    );
    assert_eq!(engine.active.as_ref().unwrap().squad_spawn, 2);
    assert!(
        engine
            .active
            .as_ref()
            .unwrap()
            .squad_catalog
            .alpha(2)
            .is_some()
    );
    // The authored source is unchanged; progression lives only in the runtime overlay.
    assert!(
        !engine
            .spawn_catalog
            .as_ref()
            .unwrap()
            .point(2)
            .unwrap()
            .active
    );
    assert_eq!(
        engine
            .spawn_catalog
            .as_ref()
            .unwrap()
            .selected_squad(0)
            .unwrap()
            .default_spawn,
        0
    );
}

#[test]
fn activation_does_not_require_a_human_squad_leader_or_an_occupied_squad() {
    let mut engine = engine();
    make_operator(&mut engine);
    complete_stage(&mut engine);
    for squad in 0..4 {
        let selected = engine
            .spawn_catalog
            .as_ref()
            .unwrap()
            .for_squad(squad)
            .unwrap();
        assert_eq!(selected.default_spawn(), squad + 2);
        assert!(selected.alpha(squad + 2).is_some());
    }
    reserve(&mut engine, 1);
    assert_eq!(
        engine.active.as_ref().unwrap().squad_catalog.squad_index(),
        1
    );
    assert_eq!(engine.active.as_ref().unwrap().squad_spawn, 3);
}

#[test]
fn assigned_final_objective_does_not_activate_uncompleted_forward_objectives() {
    let mut engine = engine();
    let mut raw = objective_json();
    raw["objects"][0]["initial_state"] = json!(9);
    raw["objects"][1]["initial_state"] = json!(9);
    raw["objects"][6]["initial_state"] = json!(3);
    for squad in raw["squads"].as_array_mut().unwrap() {
        squad["initial_objective_index"] = json!(30);
    }
    engine = engine
        .with_objective_catalog(ObjectiveCatalog::from_json(&raw.to_string()).unwrap())
        .unwrap();
    assert_eq!(engine.initial_objective_index(0), Ok(30));
    engine.advance_forward_spawns(Duration::ZERO).unwrap();
    assert!(engine.spawn_catalog.as_ref().unwrap().alpha(2).is_none());
    assert_eq!(
        engine.spawn_catalog.as_ref().unwrap().default_for_squad(0),
        Some(0)
    );
}

fn focus(engine: &mut Engine<Vec<u8>>, player: u16) {
    if let Some(actor) = engine.active.take() {
        engine.peers.insert(actor.reservation.id.clone(), actor);
    }
    engine.active = Some(
        engine
            .peers
            .remove(&format!("synthetic-reservation-{player:04}"))
            .unwrap(),
    );
}
fn physical_packet(engine: &Engine<Vec<u8>>) -> Vec<u8> {
    engine.active.as_ref().unwrap().spawn_delta.clone().unwrap()
}
fn assert_physical(
    engine: &Engine<Vec<u8>>,
    expected_state: u8,
    expected_yaw: u16,
    position: [f32; 3],
) {
    let packet = physical_packet(engine);
    let update = ObjectUpdate::decode_with_profile(&packet, engine.wire_profile).unwrap();
    assert_eq!(update.header.object_type_mask, 1 << 3); // No vehicle object or mounting.
    let mut bits = BitReader::with_profile(&packet, engine.wire_profile);
    bits.position = engine.wire_profile.header_bits();
    assert_eq!(bits.read(1), Ok(1));
    assert_eq!(
        bits.read(16),
        Ok(u32::from(
            engine.active.as_ref().unwrap().reservation.player
        ))
    );
    let mut present = std::collections::BTreeSet::new();
    for field in 0..22 {
        if bits.read(1).unwrap() == 0 {
            continue;
        }
        present.insert(field);
        match field {
            4 => {
                assert_eq!(bits.read(8), Ok(u32::from(expected_state)));
                assert_eq!(bits.read(24), Ok(0));
            }
            5 => {
                assert_eq!(bits.read(16).unwrap() as f32 / 32., position[0]);
                assert_eq!(bits.read(1), Ok(u32::from(position[1] < 0.)));
                assert_eq!(bits.read(16).unwrap() as f32 / 32., position[1].abs());
                assert_eq!(bits.read(16).unwrap() as f32 / 32., position[2]);
            }
            6 => assert_eq!(bits.read(8), Ok(127)),
            7 => assert_eq!(bits.read(10), Ok(u32::from(expected_yaw))),
            8 => {
                bits.read(8).unwrap();
                assert_eq!(bits.read(16), Ok(400));
                bits.read(32).unwrap();
                bits.read(32).unwrap();
                bits.read(32).unwrap();
            }
            9 => (), // Null respawn trigger.
            11 => {
                assert_eq!(bits.read(8), Ok(1));
                assert_eq!(bits.read(5), Ok(0));
                bits.read(2).unwrap();
                assert_eq!(bits.read(8), Ok(7));
                bits.read(1).unwrap();
            }
            13 => {
                bits.read(1).unwrap();
            }
            15 => {
                for inventory in 0..5 {
                    assert_eq!(bits.read(8), Ok(1));
                    for slot in 0..22 {
                        let occupied = bits.read(1).unwrap() != 0;
                        assert_eq!(occupied, inventory == 0 && slot == 0);
                        if occupied {
                            assert_eq!(bits.read(engine.wire_profile.inventory_item_bits()), Ok(2));
                        }
                    }
                }
            }
            17 => {
                assert_eq!(bits.read(8), Ok(1));
                assert_eq!(bits.read(8), Ok(0));
            }
            _ => panic!("unexpected respawn field {field}"),
        }
    }
    assert_eq!(
        present,
        [4, 5, 6, 7, 8, 9, 11, 13, 15, 17].into_iter().collect()
    );
    assert_eq!(bits.read(1), Ok(0));
    let actor = engine.active.as_ref().unwrap();
    assert!(actor.mounted_seat.is_none());
    assert_eq!(actor.owner_fields.state.unwrap().state, expected_state);
    assert_eq!(actor.owner_fields.yaw, Some(expected_yaw));
    let stored = actor.tracking.sample.unwrap().position;
    assert_eq!([stored.x, stored.y, stored.z], position);
    assert!(actor.current_inventory_handle.is_some());
    assert!(actor.observed_primary.is_some());
}

#[test]
fn physical_parachute_spawn_uses_committed_wave_ordinals_and_replays_do_not_consume() {
    let mut engine = engine();
    make_operator(&mut engine);
    // Two real synthetic reservations share one authored squad/point.
    reserve(&mut engine, 1);
    let shared = engine.spawn_catalog.as_ref().unwrap().for_squad(0).unwrap();
    let actor = engine.active.as_mut().unwrap();
    ready(actor);
    actor.squad_catalog = Arc::new(shared);
    actor.squad_spawn = 0;
    focus(&mut engine, 0);
    complete_stage(&mut engine);
    for actor in actors(&mut engine) {
        actor.objective_needs_clear = false;
        actor.objective_assigned = true;
        actor.life = CharacterLife::AwaitingRespawn { generation: 1 };
    }
    select(&mut engine, 2, 100);
    queue_respawn(&mut engine, 100);
    queue_respawn(&mut engine, 101);
    assert_eq!(
        engine.active.as_ref().unwrap().pending_spawn_deadline,
        Some(Duration::from_millis(200))
    );
    focus(&mut engine, 1);
    queue_respawn(&mut engine, 100);
    assert!(engine.parachute_waves.is_empty());
    focus(&mut engine, 0);
    owner_update(&mut engine, 199, 0);
    assert!(engine.active.as_ref().unwrap().spawn_delta.is_none());
    assert!(engine.parachute_waves.is_empty());
    owner_update(&mut engine, 200, 0);
    assert_physical(&engine, 17, 377, [200., 80., 300.]);
    assert_eq!(engine.parachute_waves[&(2, Duration::from_millis(200))], 1);
    focus(&mut engine, 1);
    owner_update(&mut engine, 200, 0);
    assert_physical(&engine, 17, 377, [203., 80., 304.]);
    assert_eq!(engine.parachute_waves[&(2, Duration::from_millis(200))], 2);
    let packet = physical_packet(&engine);
    let sequence = ObjectUpdate::decode_with_profile(&packet, engine.wire_profile)
        .unwrap()
        .header
        .sequence;
    let replay = owner_update(&mut engine, 201, sequence);
    assert_eq!(protocol::decode_hex(&replay[0].payload_hex), packet);
    queue_respawn(&mut engine, 201);
    assert!(
        engine
            .active
            .as_ref()
            .unwrap()
            .pending_spawn_deadline
            .is_none()
    );
    assert_eq!(engine.parachute_waves[&(2, Duration::from_millis(200))], 2);
    focus(&mut engine, 0);
    let actor = engine.active.as_mut().unwrap();
    actor.life = CharacterLife::AwaitingRespawn { generation: 2 };
    actor.objective_needs_clear = false;
    actor.objective_assigned = true;
    queue_respawn(&mut engine, 201);
    owner_update(&mut engine, 300, 0);
    assert_physical(&engine, 17, 377, [200., 80., 300.]);
    assert_eq!(engine.parachute_waves[&(2, Duration::from_millis(300))], 1);
    assert_eq!(engine.parachute_waves.len(), 1);
}

#[test]
fn personal_onfoot_selection_and_inactive_parachute_selection_keep_normal_state() {
    let mut engine = engine();
    make_operator(&mut engine);
    engine.active.as_mut().unwrap().life = CharacterLife::AwaitingRespawn { generation: 1 };
    select(&mut engine, 2, 0);
    assert!(engine.active.as_ref().unwrap().selected_spawn.is_none());
    assert!(engine.parachute_waves.is_empty());
    engine.active.as_mut().unwrap().life = CharacterLife::Alive {
        generation: 1,
        health_quarter_points: 400,
    };
    complete_stage(&mut engine);
    let actor = engine.active.as_mut().unwrap();
    actor.life = CharacterLife::AwaitingRespawn { generation: 1 };
    actor.objective_needs_clear = false;
    actor.objective_assigned = true;
    select(&mut engine, 1, 100);
    queue_respawn(&mut engine, 100);
    owner_update(&mut engine, 200, 0);
    assert_physical(&engine, 1, 0, [120., 3., 100.]);
    assert!(engine.parachute_waves.is_empty());
}

#[test]
fn recording_failure_and_full_wave_never_consume_or_physically_spawn() {
    let mut engine = engine();
    make_operator(&mut engine);
    complete_stage(&mut engine);
    let actor = engine.active.as_mut().unwrap();
    actor.life = CharacterLife::AwaitingRespawn { generation: 1 };
    actor.objective_needs_clear = false;
    actor.objective_assigned = true;
    queue_respawn(&mut engine, 100);
    engine
        .parachute_waves
        .insert((2, Duration::from_millis(200)), 8);
    owner_update(&mut engine, 200, 0);
    assert!(!engine.active.as_ref().unwrap().life.is_alive());
    assert!(engine.active.as_ref().unwrap().spawn_delta.is_none());
    assert_eq!(engine.parachute_waves[&(2, Duration::from_millis(200))], 8);
    assert!(
        engine
            .active
            .as_ref()
            .unwrap()
            .pending_spawn_deadline
            .is_some()
    );
    engine.parachute_waves.clear();
    let header = UpdateHeader {
        sequence: 100,
        nak_sequence: 0,
        client_index: 0,
        object_type_mask: 0,
        timestamp_ms: 200,
    };
    let bytes = ObjectUpdate { header, wire: &[] }
        .encode_empty_with_profile(engine.wire_profile)
        .unwrap();
    engine.max_log = engine.written;
    assert!(dispatch(&mut engine, &bytes, 200).is_err());
    assert!(engine.parachute_waves.is_empty());
    assert!(engine.active.as_ref().unwrap().spawn_delta.is_none());
    assert!(!engine.active.as_ref().unwrap().life.is_alive());
}

#[test]
fn out_of_order_pending_wave_retains_its_ordinal_and_round_reset_restores_source() {
    let mut engine = engine();
    make_operator(&mut engine);
    complete_stage(&mut engine);
    reserve(&mut engine, 1);
    engine.active.as_mut().unwrap().pending_spawn_deadline = Some(Duration::from_millis(200));
    engine
        .parachute_waves
        .insert((2, Duration::from_millis(200)), 1);
    engine
        .parachute_waves
        .insert((2, Duration::from_millis(300)), 1);
    engine.prune_parachute_waves();
    assert_eq!(
        engine.parachute_ordinal(2, Duration::from_millis(200)),
        Ok(1)
    );
    assert_eq!(
        engine.parachute_ordinal(2, Duration::from_millis(300)),
        Ok(1)
    );
    assert_eq!(
        engine.parachute_ordinal(2, Duration::from_millis(400)),
        Ok(0)
    );
    // A personal override on a still-supported old point survives the round reset.
    engine.active.as_mut().unwrap().selected_spawn = Some(protocol::CharacterSpawnSelection {
        spawn_point_index: 1,
    });
    engine.reset_objective_world();
    assert!(engine.parachute_waves.is_empty());
    for squad in 0..4 {
        let catalog = engine
            .spawn_catalog
            .as_ref()
            .unwrap()
            .for_squad(squad)
            .unwrap();
        assert_eq!(catalog.default_spawn(), 0);
        assert!(catalog.alpha(squad + 2).is_none());
    }
    for actor in actors(&mut engine) {
        assert_eq!(actor.squad_spawn, 0);
        assert_eq!(actor.squad_catalog.default_spawn(), 0);
        assert!(actor.selected_spawn.is_none_or(|s| s.spawn_point_index < 2));
    }
    assert_eq!(
        engine
            .active
            .as_ref()
            .unwrap()
            .selected_spawn
            .unwrap()
            .spawn_point_index,
        1
    );
    assert_eq!(
        engine.objective_world.as_ref().unwrap().state(20),
        Some(ObjectiveState::Inactive)
    );
    assert_eq!(
        engine
            .spawn_catalog
            .as_ref()
            .unwrap()
            .selected_squad(0)
            .unwrap()
            .available,
        vec![0, 1]
    );
}

#[test]
fn forward_schema_rejects_bad_bindings_poses_and_unverified_pool_order() {
    let base = spawn_json();
    for (pointer, value) in [
        ("/maps/0/runtime_indices_verified", json!(false)),
        ("/maps/0/runtime_indices_basis", Value::Null),
        ("/maps/0/forward_spawns/0/squad_index", json!(31)),
        ("/maps/0/forward_spawns/0/spawn_point_index", json!(99)),
        ("/maps/0/forward_spawns/0/objective_index", json!(128)),
        ("/maps/0/locations/2/active", json!(true)),
        ("/maps/0/locations/2/parachute", Value::Null),
        ("/maps/0/locations/2/kind", json!("on_foot")),
        ("/maps/0/locations/2/parachute/forward", json!([0., 0., 0.])),
        ("/maps/0/locations/2/parachute/forward", json!([1., 0., 1.])),
        ("/maps/0/locations/2/parachute/spacing_m", json!(0.)),
        ("/maps/0/locations/2/parachute/spacing_m", json!(-1.)),
        ("/maps/0/locations/2/parachute/spacing_m", json!(65.)),
        ("/maps/0/locations/2/parachute/yaw", json!(1024)),
        ("/maps/0/locations/2/position/x", json!(2047.)),
    ] {
        let mut raw = base.clone();
        *raw.pointer_mut(pointer).unwrap() = value;
        assert!(spawns(raw).is_err(), "{pointer}");
    }
    let mut raw = base.clone();
    raw["maps"][0]["forward_spawns"]
        .as_array_mut()
        .unwrap()
        .push(base["maps"][0]["forward_spawns"][0].clone());
    assert!(spawns(raw).is_err());
    let mut raw = base.clone();
    raw["maps"][0]["forward_spawns"]
        .as_array_mut()
        .unwrap()
        .push(json!({
        "objective_index":20,"squad_index":16,"spawn_point_index":2}));
    assert!(
        spawns(raw)
            .unwrap_err()
            .to_string()
            .contains("incompatible_forward_spawn_factions")
    );
    let mut raw = base.clone();
    raw["maps"][0]["squads"][0]["available"] = json!([0, 1, 2]);
    assert!(spawns(raw).is_err());
    assert!(spawns(base).is_ok());
}

#[test]
fn forward_objective_links_fail_before_configuration_and_legacy_catalogs_are_unchanged() {
    let catalog = spawns(spawn_json()).unwrap();
    assert_eq!(
        catalog.validate_objectives(None),
        Err("forward_spawns_require_objective_catalog")
    );
    assert_eq!(catalog.validate_objectives(Some(&objectives())), Ok(()));
    let mut raw = spawn_json();
    raw["maps"][0]["forward_spawns"][0]["objective_index"] = json!(99);
    let invalid = spawns(raw).unwrap();
    assert_eq!(
        invalid.validate_objectives(Some(&objectives())),
        Err("unknown_forward_spawn_objective")
    );
    let mut engine = engine().with_spawn_catalog(invalid);
    let response = engine.handle(
        Request::ConfigureMatch {
            v: BRIDGE_VERSION,
            request_id: "synthetic-config".into(),
            minimum_players: 1,
            maximum_players: 8,
            application_id: 20231,
        },
        Duration::ZERO,
        1,
    );
    assert_eq!(
        response.error.as_deref(),
        Some("unknown_forward_spawn_objective")
    );
    assert!(engine.active.is_none());
    let mut legacy = spawn_json();
    legacy["maps"][0]
        .as_object_mut()
        .unwrap()
        .remove("forward_spawns");
    legacy["maps"][0]["locations"]
        .as_array_mut()
        .unwrap()
        .truncate(2);
    legacy["maps"][0]["spawn_count"] = json!(2);
    let mut legacy = spawns(legacy).unwrap();
    assert_eq!(legacy.validate_objectives(None), Ok(()));
    legacy.set_runtime_default(1).unwrap();
    assert_eq!(legacy.default_spawn(), 1);
    legacy.reset_runtime();
    assert_eq!(legacy.default_spawn(), 0);
}

#[test]
fn leader_selection_after_progression_preserves_followers_and_personal_overrides() {
    let mut engine = engine();
    make_operator(&mut engine);
    for player in 1..3 {
        reserve(&mut engine, player);
        let shared = engine.spawn_catalog.as_ref().unwrap().for_squad(0).unwrap();
        let actor = engine.active.as_mut().unwrap();
        ready(actor);
        actor.squad_catalog = Arc::new(shared);
        actor.squad_spawn = 0;
        actor.life = CharacterLife::Alive {
            generation: 1,
            health_quarter_points: 400,
        };
        if player == 2 {
            actor.selected_spawn = Some(protocol::CharacterSpawnSelection {
                spawn_point_index: 1,
            });
        }
    }
    focus(&mut engine, 0);
    complete_stage(&mut engine);
    let mut request = 0x0218u16.to_be_bytes().to_vec();
    request.extend(0u16.to_be_bytes());
    request.push(0);
    dispatch(&mut engine, &request, 100).unwrap();
    assert_eq!(engine.squad_spawn_for(0), Some(0));
    for actor in actors(&mut engine) {
        assert_eq!(actor.squad_spawn, 0);
        assert_eq!(actor.squad_catalog.default_spawn(), 0);
        assert_eq!(
            actor.selected_spawn.unwrap().spawn_point_index,
            u16::from(actor.reservation.player == 2)
        );
        assert!(actor.squad_catalog.alpha(2).is_some());
    }
    // Re-scanning completed objectives must not undo a later leader choice.
    engine
        .advance_forward_spawns(Duration::from_millis(110))
        .unwrap();
    assert_eq!(engine.squad_spawn_for(0), Some(0));
}

#[test]
fn parachute_without_resolved_inventory_keeps_the_pending_wave_without_consumption() {
    let mut engine = engine();
    make_operator(&mut engine);
    complete_stage(&mut engine);
    let actor = engine.active.as_mut().unwrap();
    actor.reservation.character.loadouts = None;
    actor.life = CharacterLife::AwaitingRespawn { generation: 1 };
    actor.objective_needs_clear = false;
    actor.objective_assigned = true;
    queue_respawn(&mut engine, 100);
    owner_update(&mut engine, 200, 0);
    assert!(engine.parachute_waves.is_empty());
    let actor = engine.active.as_ref().unwrap();
    assert!(actor.spawn_delta.is_none());
    assert_eq!(
        actor.pending_spawn_deadline,
        Some(Duration::from_millis(200))
    );
    assert!(!actor.life.is_alive());
}
