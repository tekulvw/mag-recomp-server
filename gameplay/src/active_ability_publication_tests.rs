//! Reliable native ability state under a full synthetic roster and queue pressure.
use super::*;
use crate::active_abilities::{Ability, CommandRequest};

fn configured_roster(players: u16) -> Engine<Vec<u8>> {
    let engine = active_ability_engine_tests::configured();
    let name = engine.map_name();
    let spawns = spawn_catalog::SpawnCatalog::from_json(
        &json!({
            "format":"mag_spawn_catalog","schema_version":1,"source":"synthetic full roster",
            "active":{"map_name":name,"squad_index":16,"default_spawn":0,"available":[0]},
            "match_profile":{"game_type":2,"auto_assign_teams":true,"team_capacity":32,
                "squad_capacity":4,"has_bunkers":false,"has_objectives":false,"attacking_faction":0},
            "maps":[{"name":name,"owner_faction":1,"spawn_count":1,
                "locations":[{"index":0,"token":71,"kind":"on_foot","position":{"x":100.,"y":3.,"z":100.}}],
                "squads":(0..32).map(|index| json!({
                    "index":index,"team":index/16,"platoon":index%16/4,"squad":index%4,
                    "faction":if index>=16 {Some(1)} else {None},"available":[0],"default_spawn":0
                })).collect::<Vec<_>>() }]
        }).to_string(),
    ).unwrap();
    let mut engine = engine
        .with_spawn_catalog(spawns)
        .with_match_limits(1, 64)
        .unwrap();
    let mut loadouts = loadouts::SavedLoadouts {
        layout_revision: 0,
        selected_index: 0,
        update_ids: [1; 5],
        inventories: [[None; 22]; 5],
    };
    loadouts.inventories[0][0] = Some(2);
    for player in 0..players {
        let response = engine.handle(
            Request::Reserve {
                v: BRIDGE_VERSION,
                request_id: format!("reserve-{player}"),
                reservation_id: format!("synth-ability-pub-{player:02}"),
                world_id: 1,
                player_id: player,
                account_id: u32::from(player) + 1,
                session_key: "aaaaaaaaaaaaaaaa".into(),
                access_key: "bbbbbbbbbbbbbbbb".into(),
                character: serde_json::from_value(json!({
                    "name":format!("Synthetic Player {player}"),"faction":1-player%2,
                    "head":0,"voice":0,"universe_id":26,"loadouts":loadouts,
                    "character_id":u32::from(player)+1,
                }))
                .unwrap(),
            },
            Duration::ZERO,
            1,
        );
        assert!(response.ok, "{:?}", response.error);
    }
    for actor in engine.active.iter_mut().chain(engine.peers.values_mut()) {
        spawn_progression::ready(actor);
        actor.nominations = 1;
        assert!(actor.pending_shared.is_empty());
    }
    engine.reconcile_squad_leaders().unwrap();
    engine
}

fn activate_defender_sensor(engine: &mut Engine<Vec<u8>>) {
    let leader = &engine.peers["synth-ability-pub-00"];
    let (candidate, decision, denial) = engine
        .plan_command_ability(
            leader,
            CommandRequest {
                squad_index: 16,
                ability: Ability::Sensor,
                position: [100., 3., 100.],
                request_time: 0,
                target_id: u32::MAX,
            },
            Duration::from_millis(20),
        )
        .unwrap();
    assert_eq!(denial, None);
    assert_eq!(decision["accepted"], true, "{decision}");
    engine.active_abilities = Some(candidate.unwrap());
}

fn update_mask(packet: &[u8], profile: wire_profile::WireProfile) -> u32 {
    replication::ObjectUpdate::decode_with_profile(packet, profile)
        .unwrap()
        .header
        .object_type_mask
}

#[test]
fn eight_fully_populated_squad_updates_fit_one_bridge_payload() {
    use crate::active_ability_state::{NativeAbilityData, NativeAbilityScope, NativeAbilityUpdate};

    let profile = active_ability_engine_tests::configured().wire_profile;
    let updates: Vec<_> = (0..8)
        .map(|squad| {
            let mut update = NativeAbilityUpdate::new(NativeAbilityScope::Squad(squad));
            update.active = Some(true);
            update.owner_index = Some(0x105);
            update.leader_index = Some(squad);
            update.data = [Some(NativeAbilityData {
                squad_index: squad,
                ..Default::default()
            }); 6];
            update.active_mask = Some(0x3f);
            update.available_mask = Some(0x3f);
            update.shadow_modifier_percent = Some(0);
            update
        })
        .collect();
    let packet = NativeAbilityUpdate::encode_batch(&updates, 2, 21, profile).unwrap();
    assert_eq!(update_mask(&packet, profile), 1 << 8);
    assert!(packet.len() * 8 >= 8 * 6 * NativeAbilityData::WIRE_BITS);
    assert!(packet.len() <= 1_397);
}

#[test]
fn full_roster_bootstraps_in_five_packets_and_receives_only_changed_squad() {
    let mut engine = configured_roster(64);
    let actors = || engine.active.iter().chain(engine.peers.values());
    assert_eq!(actors().count(), 64);
    assert_eq!(
        actors()
            .map(|actor| actor.squad_catalog.squad_index())
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        32
    );
    engine
        .queue_active_ability_state(Duration::from_millis(1), 1)
        .unwrap();
    let profile = engine.wire_profile;
    for actor in engine.active.iter_mut().chain(engine.peers.values_mut()) {
        let masks: Vec<_> = actor
            .pending_shared
            .drain(..)
            .map(|packet| update_mask(&packet, profile))
            .collect();
        assert_eq!(masks, [1 << 10, 1 << 8, 1 << 8, 1 << 8, 1 << 8]);
        assert!(!actor.replication_backlogged);
    }

    activate_defender_sensor(&mut engine);
    engine
        .queue_active_ability_state(Duration::from_millis(21), 1)
        .unwrap();
    for actor in engine.active.iter().chain(engine.peers.values()) {
        assert_eq!(actor.pending_shared.len(), 1);
        let bytes = &actor.pending_shared[0];
        assert_eq!(update_mask(bytes, profile), 1 << 8);
        let mut bits = replication::BitReader::with_profile(bytes, profile);
        bits.position = profile.header_bits();
        assert_eq!(bits.read(1).unwrap(), 1);
        assert_eq!(bits.read(16).unwrap(), 16);
        assert_eq!(bits.read(18).unwrap(), 0);
        assert_eq!(bits.read(1).unwrap(), 1); // Sensor AbilityData present.
        for width in [32, 32, 32, 16, 32, 1, 32] {
            bits.read(width).unwrap();
        }
        assert_eq!(bits.read(3).unwrap(), 0);
        assert_eq!(bits.read(1).unwrap(), 1); // Active mask present.
        assert_eq!(bits.read(16).unwrap(), 1 << 2);
        assert_eq!(bits.read(4).unwrap(), 0); // No availability/shadow/tail/next object.
    }
}

#[test]
fn full_queue_defers_recipient_without_dropping_pending_delta() {
    let mut engine = configured_roster(2);
    engine
        .queue_active_ability_state(Duration::from_millis(1), 1)
        .unwrap();
    let deferring = "synth-ability-pub-00";
    let peer = engine.peers.get_mut(deferring).unwrap();
    let placeholder = peer.pending_shared[0].clone();
    peer.pending_shared
        .resize(multiplayer::MAX_SHARED_QUEUE, placeholder);
    engine.active.as_mut().unwrap().pending_shared.clear();
    activate_defender_sensor(&mut engine);
    engine
        .queue_active_ability_state(Duration::from_millis(21), 1)
        .unwrap();
    assert_eq!(
        engine.peers[deferring].pending_shared.len(),
        multiplayer::MAX_SHARED_QUEUE
    );
    assert!(!engine.peers[deferring].replication_backlogged);
    assert_eq!(engine.active.as_ref().unwrap().pending_shared.len(), 1);

    let queue = &mut engine.peers.get_mut(deferring).unwrap().pending_shared;
    queue.pop_front().unwrap();
    engine
        .queue_active_ability_state(Duration::from_millis(22), 1)
        .unwrap();
    assert_eq!(
        engine.peers[deferring].pending_shared.len(),
        multiplayer::MAX_SHARED_QUEUE
    );
    assert_eq!(
        update_mask(
            engine.peers[deferring].pending_shared.back().unwrap(),
            engine.wire_profile
        ),
        1 << 8
    );
    assert_eq!(engine.active.as_ref().unwrap().pending_shared.len(), 1);
    assert!(!engine.peers[deferring].replication_backlogged);
}

#[test]
fn failed_publication_record_leaves_fifos_and_recipient_baselines_untouched() {
    let mut engine = configured_roster(2);
    let maximum_log = engine.max_log;
    engine.max_log = engine.written;
    assert_eq!(
        engine.queue_active_ability_state(Duration::from_millis(1), 1),
        Err("log_limit_reached")
    );
    assert!(
        engine
            .active
            .iter()
            .chain(engine.peers.values())
            .all(|actor| actor.pending_shared.is_empty())
    );

    // Reopen the synthetic recorder to test that the full bootstrap retries.
    engine.max_log = maximum_log;
    engine.failed = false;
    engine
        .queue_active_ability_state(Duration::from_millis(2), 1)
        .unwrap();
    for actor in engine.active.iter().chain(engine.peers.values()) {
        assert_eq!(actor.pending_shared.len(), 2);
    }
}
