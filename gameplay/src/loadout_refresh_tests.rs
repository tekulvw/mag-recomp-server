//! Synthetic typed armory refreshes; no durable accounts, sockets or game data.
use super::*;
use crate::spawn_progression::dispatch;

fn fixture() -> Engine<Vec<u8>> {
    let mut engine = crate::human_kill_reward_tests::fixture();
    for actor in engine.active.iter_mut().chain(engine.peers.values_mut()) {
        actor.reservation.character.loadout_account_id =
            actor.reservation.character.account_id.clone();
    }
    let source_squad =
        std::sync::Arc::new(engine.spawn_catalog.as_ref().unwrap().for_squad(0).unwrap());
    let actor = engine.active.as_mut().unwrap();
    actor.squad_catalog = source_squad;
    actor
        .reservation
        .character
        .loadouts
        .as_mut()
        .unwrap()
        .inventories[1][0] = Some(2);
    actor
        .reservation
        .character
        .loadouts
        .as_mut()
        .unwrap()
        .inventories[1][12] = Some(3);
    actor.claimed = Some(Claimed {
        wire: Vec::new(),
        np_identity: [1; 36],
        game_info: protocol::GameInfo {
            client_index: 1,
            level_name: "synthetic_arena".into(),
            start_time_ms: 0,
            attacking_faction: 1,
            defending_faction: 0,
            maximum_points: [20; 2],
            current_points: [20; 2],
            game_type: 0,
        },
    });
    engine
}
fn snapshot(engine: &Engine<Vec<u8>>, player: u16) -> loadouts::SavedLoadouts {
    (**engine
        .active
        .iter()
        .chain(engine.peers.values())
        .find(|a| a.reservation.player == player)
        .unwrap()
        .reservation
        .character
        .loadouts
        .as_ref()
        .unwrap())
    .clone()
}
fn changed(engine: &Engine<Vec<u8>>, player: u16) -> loadouts::SavedLoadouts {
    let mut saved = snapshot(engine, player);
    saved.update_ids[0] = 2;
    saved.inventories[0][21] = Some(2);
    saved
}
fn request(player: u16, revision: u64, saved: &loadouts::SavedLoadouts) -> Value {
    json!({"op":"refresh_loadouts","v":2,"request_id":"synthetic-refresh",
        "reservation_id":format!("synthetic-reservation-{player:04}"),"world_id":1,"player_id":player,
        "account_id":format!("synthetic-human-account-{player}"),"character_id":u32::from(player)+1,
        "revision":revision,"loadouts":saved})
}
fn refresh(engine: &mut Engine<Vec<u8>>, value: &Value, now: u64) -> Response {
    engine.handle(
        parse_request(value.to_string().as_bytes()).expect("typed refresh request"),
        Duration::from_millis(now),
        1,
    )
}
fn assert_error(response: Response, want: &str) {
    assert!(!response.ok);
    assert_eq!(response.error.as_deref(), Some(want));
}

#[test]
fn valid_refresh_preserves_current_kit_pending_preference_and_live_hit_until_new_spawn() {
    let mut engine = fixture();
    dispatch(&mut engine, &[0x02, 0x52, 0xf9], 100).unwrap();
    let actor = engine.active.as_ref().unwrap();
    let pending = actor.pending_loadout_selection.clone();
    assert!(pending.is_some());
    let old_equipped = actor.equipped_loadout().unwrap();
    let mut saved = changed(&engine, 1);
    saved.selected_index = 1;
    saved.update_ids[0] = 0;
    saved.update_ids[1] = 3;
    saved.update_ids[3] = 255;
    saved.inventories[1][21] = Some(2);
    let response = refresh(&mut engine, &request(1, 1, &saved), 1000);
    assert!(response.ok, "{:?}", response.error);
    let actor = engine.active.as_ref().unwrap();
    assert_eq!(
        actor
            .reservation
            .character
            .loadouts
            .as_ref()
            .unwrap()
            .selected_index,
        0
    );
    assert_eq!(
        actor
            .reservation
            .character
            .loadouts
            .as_ref()
            .unwrap()
            .update_ids[0],
        0
    );
    assert_eq!(
        actor
            .reservation
            .character
            .loadouts
            .as_ref()
            .unwrap()
            .update_ids[3],
        255
    );
    assert_eq!(actor.selected_loadout.unwrap().index, -7);
    assert_eq!(actor.pending_loadout_selection, pending);
    assert_eq!(actor.current_inventory_handle.unwrap().update_id, 1);
    assert_eq!(actor.current_inventory_handle.unwrap().inventory_index, 0);
    assert_eq!(actor.observed_primary.unwrap().ammo_count, 9);
    assert_eq!(
        actor.equipped_loadout().unwrap().inventories[0].slots,
        old_equipped.inventories[0].slots
    );
    assert!(matches!(
        actor.life,
        CharacterLife::Alive { generation: 4, .. }
    ));
    crate::human_kill_reward_tests::hit(&mut engine, 17, 1100).unwrap();
    assert!(matches!(
        engine.peers["synthetic-reservation-0000"].life,
        CharacterLife::Incapacitated { .. }
    ));
    let actor = engine.active.as_mut().unwrap();
    actor.life = CharacterLife::AwaitingRespawn { generation: 4 };
    actor.objective_assigned = true;
    actor.objective_needs_clear = false;
    dispatch(&mut engine, &0x0211u16.to_be_bytes(), 1200).unwrap();
    let bytes = replication::ObjectUpdate {
        header: replication::UpdateHeader {
            sequence: 1,
            nak_sequence: 0,
            client_index: 0,
            object_type_mask: 0,
            timestamp_ms: 1300,
        },
        wire: &[],
    }
    .encode_empty_with_profile(engine.wire_profile)
    .unwrap();
    dispatch(&mut engine, &bytes, 1300).unwrap();
    let actor = engine.active.as_ref().unwrap();
    assert!(
        matches!(actor.life, CharacterLife::Alive { generation: 5, .. }),
        "life {:?}, pending {:?}, delta {}, entered {}, assigned {}, clear {}",
        actor.life,
        actor.pending_spawn_deadline,
        actor.spawn_delta.is_some(),
        actor.entered_game,
        actor.objective_assigned,
        actor.objective_needs_clear
    );
    assert_eq!(actor.current_inventory_handle.unwrap().inventory_index, 1);
    // The live inventory handle follows native slot-switch versioning; the
    // refreshed per-kit byte version belongs to the inventory row itself.
    assert_eq!(actor.current_inventory_handle.unwrap().update_id, 2);
    assert_eq!(
        actor.equipped_loadout().unwrap().inventories[1].update_id,
        3
    );
    assert_eq!(
        actor.equipped_loadout().unwrap().inventories[1].slots[21],
        Some(2)
    );
}
#[test]
fn refresh_retries_are_audited_exact_and_stale_or_conflicting_revisions_do_not_apply() {
    let mut engine = fixture();
    let saved = changed(&engine, 1);
    let req = request(1, 2, &saved);
    assert!(refresh(&mut engine, &req, 1000).ok);
    let records = engine.sequence;
    assert!(refresh(&mut engine, &req, 1001).ok);
    assert_eq!(engine.sequence, records + 1);
    assert_error(
        refresh(&mut engine, &request(1, 1, &saved), 1002),
        "refresh_loadouts_stale_revision",
    );
    let mut conflict = saved.clone();
    conflict.update_ids[0] = 3;
    assert_error(
        refresh(&mut engine, &request(1, 2, &conflict), 1003),
        "refresh_loadouts_revision_conflict",
    );
    assert_eq!(snapshot(&engine, 1), saved);
    engine.reset_objective_world();
    assert_error(
        refresh(&mut engine, &request(1, 1, &saved), 1004),
        "refresh_loadouts_stale_revision",
    );
    assert!(refresh(&mut engine, &request(1, 3, &conflict), 1005).ok);
}
#[test]
fn refresh_requires_current_owned_lifetime_and_complete_valid_saved_kit() {
    let mut engine = fixture();
    let before = snapshot(&engine, 1);
    let saved = changed(&engine, 1);
    for (field, value) in [
        ("world_id", json!(2)),
        ("player_id", json!(0)),
        ("account_id", json!("wrong-account")),
        ("character_id", json!(7)),
    ] {
        let mut req = request(1, 1, &saved);
        req[field] = value;
        assert_error(
            refresh(&mut engine, &req, 1000),
            "refresh_loadouts_identity_mismatch",
        );
        assert_eq!(snapshot(&engine, 1), before);
    }
    let mut bad = saved.clone();
    bad.inventories[0][0] = None;
    assert!(!refresh(&mut engine, &request(1, 1, &bad), 1001).ok);
    let mut bad = saved.clone();
    bad.inventories[1][21] = Some(999);
    assert!(!refresh(&mut engine, &request(1, 1, &bad), 1002).ok);
    assert_eq!(snapshot(&engine, 1), before);
    assert_error(
        refresh(&mut engine, &request(1, 1, &saved), 121_000),
        "refresh_loadouts_reservation_expired",
    );
    assert_eq!(snapshot(&engine, 1), before);
}
#[test]
fn failed_refresh_record_cannot_mutate_saved_or_equipped_snapshots() {
    let mut engine = fixture();
    let before = snapshot(&engine, 1);
    let saved = changed(&engine, 1);
    let equipped = engine.active.as_ref().unwrap().equipped_loadout().unwrap();
    engine.max_log = engine.written;
    assert_error(
        refresh(&mut engine, &request(1, 1, &saved), 1000),
        "log_limit_reached",
    );
    assert_eq!(snapshot(&engine, 1), before);
    assert_eq!(
        engine
            .active
            .as_ref()
            .unwrap()
            .equipped_loadout()
            .unwrap()
            .inventories[0]
            .slots,
        equipped.inventories[0].slots
    );
}
#[test]
fn peer_refresh_does_not_swap_current_actor_and_old_owner_release_cancels_it() {
    let mut engine = fixture();
    let owner = snapshot(&engine, 1);
    let saved = changed(&engine, 0);
    assert!(refresh(&mut engine, &request(0, 1, &saved), 1000).ok);
    assert_eq!(engine.active.as_ref().unwrap().reservation.player, 1);
    assert_eq!(snapshot(&engine, 1), owner);
    assert_eq!(snapshot(&engine, 0), saved);
    let mut replacement_character = engine.peers["synthetic-reservation-0000"]
        .reservation
        .character
        .clone();
    replacement_character.account_id = None;
    replacement_character.progression = None;
    let response = engine.handle(
        Request::Release {
            v: 2,
            request_id: "synthetic-release".into(),
            reservation_id: "synthetic-reservation-0000".into(),
        },
        Duration::from_millis(1001),
        1,
    );
    assert!(response.ok);
    assert_error(
        refresh(&mut engine, &request(0, 2, &saved), 1002),
        "refresh_loadouts_owner_unavailable",
    );
    let response = engine.handle(
        Request::Reserve {
            v: 2,
            request_id: "synthetic-requeue".into(),
            reservation_id: "synthetic-replacement-0000".into(),
            world_id: 1,
            player_id: 0,
            account_id: 1,
            session_key: "aaaaaaaaaaaaaaaa".into(),
            access_key: "bbbbbbbbbbbbbbbb".into(),
            character: replacement_character,
        },
        Duration::from_millis(1003),
        1,
    );
    assert!(response.ok, "{:?}", response.error);
    let replacement = snapshot(&engine, 0);
    assert_error(
        refresh(&mut engine, &request(0, 2, &saved), 1004),
        "refresh_loadouts_owner_unavailable",
    );
    assert_eq!(
        engine.active.as_ref().unwrap().reservation.id,
        "synthetic-replacement-0000"
    );
    assert_eq!(engine.active.as_ref().unwrap().reservation.account, 1);
    assert!(
        engine
            .active
            .as_ref()
            .unwrap()
            .last_loadout_refresh
            .is_none()
    );
    assert_eq!(snapshot(&engine, 0), replacement);
}
#[test]
fn refresh_codec_is_strict_and_hello_advertises_both_armory_capabilities() {
    let mut engine = fixture();
    let saved = changed(&engine, 1);
    let req = request(1, 1, &saved);
    assert!(parse_request(req.to_string().as_bytes()).is_ok());
    for (field, value) in [
        ("revision", json!(0)),
        ("character_id", json!(0)),
        ("unknown", json!(true)),
    ] {
        let mut bad = req.clone();
        bad[field] = value;
        assert!(parse_request(bad.to_string().as_bytes()).is_err());
    }
    let response = engine.handle(
        Request::Hello {
            v: 2,
            request_id: "synthetic-cap".into(),
        },
        Duration::ZERO,
        1,
    );
    assert!(
        response
            .capabilities
            .contains(&"committed_loadout_selection_v1".to_owned())
    );
    assert!(
        response
            .capabilities
            .contains(&"active_loadout_refresh_v1".to_owned())
    );
}
