//! Invented two-platoon Acquisition fixtures, independent of native map data.
use super::*;

fn acquisition() -> Engine<Vec<u8>> {
    let mut engine = engine();
    let mut value = spawn_json();
    value["source"] = json!("SYNTHETIC Acquisition entry");
    value["active"]["available"] = json!([0]);
    value["match_profile"]["game_type"] = json!(1);
    value["match_profile"]["team_capacity"] = json!(32);
    value["match_profile"]["squad_capacity"] = json!(4);
    value["match_profile"]["has_objectives"] = json!(false);
    value["maps"][0]["forward_spawns"] = json!([]);
    value["maps"][0]["spawn_count"] = json!(16);
    value["maps"][0]["locations"] = json!(
        (0..16)
            .map(|i| json!({
                "index":i,"token":1000+i,"kind":"insertion",
                "position":{"x":100.+i as f32*2.,"y":3.,"z":100.}
            }))
            .collect::<Vec<_>>()
    );
    value["maps"][0]["squads"] = json!(
        (0..2)
            .flat_map(|team| (0..8).map(move |slot| {
                let point = team * 8 + slot;
                json!({"index":team*16+slot,"team":team,"platoon":slot/4,"squad":slot%4,
            "faction":if team==1 {Some(0)} else {None},"available":[point],"default_spawn":point})
            }))
            .collect::<Vec<_>>()
    );
    engine.spawn_catalog = Some(spawns(value).unwrap());
    engine.objective_catalog = None;
    engine.objective_world = None;
    engine
}

fn claimed(actor: &mut Active) {
    actor.claimed = Some(Claimed {
        wire: Vec::new(),
        np_identity: [actor.reservation.player as u8; 36],
        game_info: protocol::GameInfo {
            client_index: actor.reservation.player,
            level_name: "synthetic_arena".into(),
            start_time_ms: 0,
            attacking_faction: 1,
            defending_faction: 0,
            maximum_points: [100, 100],
            current_points: [100, 100],
            game_type: 1,
        },
    });
}

#[test]
fn acquisition_admission_uses_existing_64_bound_and_all_sixteen_squads() {
    let mut engine = acquisition();
    assert_eq!(engine.admission_capacity_for(20230), 8);
    assert_eq!(engine.admission_capacity_for(20231), 64);
    engine = engine.with_match_limits(1, 64).unwrap();
    for player in 0..64 {
        reserve_faction(&mut engine, player, if player < 32 { 1 } else { 0 });
    }
    let mut counts = std::collections::BTreeMap::new();
    for actor in engine.peers.values().chain(engine.active.iter()) {
        *counts.entry(actor.squad_catalog.squad_index()).or_insert(0) += 1;
        assert!(engine.actor_matches_team(actor));
    }
    assert_eq!(counts.len(), 16);
    assert!(counts.values().all(|count| *count == 4));
    for actor in actors(&mut engine) {
        claimed(actor);
    }
    for player in 0..64 {
        let mut bytes = engine
            .wire_profile
            .wire_message_type(5)
            .to_be_bytes()
            .to_vec();
        bytes.push(0x80);
        let response = engine.handle(
            Request::Event {
                v: 2,
                request_id: format!("synthetic-ready-{player}"),
                reservation_id: format!("synthetic-reservation-{player:04}"),
                world_id: 1,
                player_id: player,
                transport: Transport::Tcp,
                direction: Direction::ClientToServer,
                rt_type: 0x3d,
                plugin_id: Some(1),
                payload_hex: protocol::encode_hex(&bytes),
            },
            Duration::from_millis(10),
            1,
        );
        assert!(response.ok, "{:?}", response.error);
    }
    assert!(
        engine
            .peers
            .values()
            .chain(engine.active.iter())
            .all(|actor| actor.initial_objects.is_some() && actor.ready_for_updates)
    );
    assert!(!acquisition().with_match_limits(1, 65).is_ok());
}

#[test]
fn acquisition_second_platoon_spawn_and_late_join_preserve_squad_identity() {
    let mut engine = acquisition().with_match_limits(1, 64).unwrap();
    for player in 0..8 {
        reserve(&mut engine, player);
    }
    // Empty squads are assigned before a second member: player4 is platoon1/Alpha.
    let actor = engine
        .peers
        .values()
        .chain(engine.active.iter())
        .find(|a| a.reservation.player == 4)
        .unwrap();
    assert_eq!(actor.squad_catalog.squad_index(), 4);
    assert_eq!(actor.squad_spawn, 4);
    for actor in actors(&mut engine) {
        claimed(actor);
        ready(actor);
    }
    let squads = engine.squad_creations(None);
    assert_eq!(squads.len(), 16);
    assert_eq!(
        squads.iter().find(|s| s.object_index == 4).unwrap().members[0],
        4
    );
    engine.active.as_mut().unwrap().emplacement_creation = None;
    // The real owner update plans the configured spawn setup, not Alpha0.
    let player = engine.active.as_ref().unwrap().reservation.player;
    let selected = engine.active.as_ref().unwrap().squad_catalog.squad_index();
    assert_eq!(selected, 7);
    let frames = owner_update(&mut engine, 10, 0);
    assert!(!frames.is_empty());
    assert!(
        engine
            .active
            .as_ref()
            .unwrap()
            .emplacement_creation
            .is_some()
    );
    let packet = &engine
        .active
        .as_ref()
        .unwrap()
        .emplacement_creation
        .as_ref()
        .unwrap();
    let mut bits = BitReader::with_profile(packet, engine.wire_profile);
    bits.position = engine.wire_profile.header_bits();
    assert_eq!(bits.read(1).unwrap(), 1);
    assert_eq!(bits.read(16).unwrap(), u32::from(player));
    for field in 0..22 {
        assert_eq!(bits.read(1).unwrap(), u32::from(field == 14));
        if field == 14 {
            assert_eq!(bits.read(7).unwrap(), 7);
        }
    }
    engine.active.as_mut().unwrap().life = CharacterLife::AwaitingRespawn { generation: 1 };
    queue_respawn(&mut engine, 100);
    let deadline = engine
        .active
        .as_ref()
        .unwrap()
        .pending_spawn_deadline
        .unwrap();
    owner_update(&mut engine, deadline.as_millis() as u64 - 1, 0);
    assert!(engine.active.as_ref().unwrap().spawn_delta.is_none());
    owner_update(&mut engine, deadline.as_millis() as u64, 0);
    let actor = engine.active.as_ref().unwrap();
    assert!(actor.life.is_alive());
    assert_eq!(actor.squad_catalog.squad_index(), 7);
    assert_eq!(actor.tracking.sample.unwrap().position.x, 114.);
    let packet = physical_packet(&engine);
    let sequence = ObjectUpdate::decode_with_profile(&packet, engine.wire_profile)
        .unwrap()
        .header
        .sequence;
    let replay = owner_update(&mut engine, deadline.as_millis() as u64 + 1, sequence);
    assert_eq!(protocol::decode_hex(&replay[0].payload_hex), packet);
    // Late join gets the existing second platoon's members, not relabeled Alpha.
    reserve(&mut engine, 8);
    let squads = engine.squad_creations(Some(8));
    assert_eq!(
        squads.iter().find(|s| s.object_index == 4).unwrap().members[0],
        4
    );
    assert_eq!(
        engine
            .peers
            .values()
            .find(|a| a.reservation.player == 4)
            .unwrap()
            .squad_catalog
            .squad_index(),
        4
    );
}
