//! Synthetic committed desired-kit persistence, without accounts or network I/O.
use super::*;
use crate::spawn_progression::{engine, ready, reserve};

fn fixture() -> Engine<Vec<u8>> {
    let mut engine = engine();
    reserve(&mut engine, 0);
    let actor = engine.active.as_mut().unwrap();
    ready(actor);
    actor.claimed = Some(Claimed {
        wire: Vec::new(),
        np_identity: [0; 36],
        game_info: protocol::GameInfo {
            client_index: 0,
            level_name: "synthetic_arena".into(),
            start_time_ms: 0,
            attacking_faction: 1,
            defending_faction: 0,
            maximum_points: [20, 20],
            current_points: [20, 20],
            game_type: 0,
        },
    });
    actor.reservation.character.loadout_account_id = Some("synthetic-account".into());
    actor
        .reservation
        .character
        .loadouts
        .as_mut()
        .unwrap()
        .inventories[1][0] = Some(2);
    actor.current_inventory_handle = Some(replication::InventoryHandle {
        update_id: 1,
        inventory_index: 0,
    });
    actor.observed_primary = Some(replication::InventoryItem {
        inventory_update_id: 1,
        gear_slot: 0,
        ammo_pool: 0,
        ammo_count: 5,
        reloading: false,
    });
    engine
}
fn select(engine: &mut Engine<Vec<u8>>, index: u8, now: u64) -> Response {
    engine.handle(
        Request::Event {
            v: 2,
            request_id: format!("synthetic-select-{now}"),
            reservation_id: "synthetic-reservation-0000".into(),
            world_id: 1,
            player_id: 0,
            transport: Transport::Udp,
            direction: Direction::ClientToServer,
            rt_type: 0x3d,
            plugin_id: Some(1),
            payload_hex: protocol::encode_hex(&[0x02, 0x52, index]),
        },
        Duration::from_millis(now),
        1,
    )
}
fn hello(engine: &mut Engine<Vec<u8>>, now: u64) -> Value {
    serde_json::to_value(engine.handle(
        Request::Hello {
            v: 2,
            request_id: "synthetic-hello".into(),
        },
        Duration::from_millis(now),
        1,
    ))
    .unwrap()
}
fn actions(response: &Response) -> Vec<Value> {
    serde_json::to_value(response).unwrap()["loadout_selections"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}
fn ack(engine: &mut Engine<Vec<u8>>, receipt: &Value, status: &str) -> Response {
    let request = json!({"op":"loadout_selection_settled","v":2,"request_id":"synthetic-settled",
        "reservation_id":receipt["reservation_id"],"world_id":receipt["world_id"],"player_id":receipt["player_id"],
        "selection_id":receipt["selection_id"],"revision":receipt["revision"],"status":status});
    engine.handle(
        parse_request(request.to_string().as_bytes()).unwrap(),
        Duration::from_secs(1),
        1,
    )
}
#[test]
fn hello_advertises_committed_selection_capability_and_empty_actions_are_omitted() {
    let mut empty = crate::spawn_progression::engine();
    assert_eq!(
        hello(&mut empty, 0)["capabilities"],
        json!(["committed_loadout_selection_v1"])
    );
    let mut engine = fixture();
    let response = hello(&mut engine, 0);
    assert_eq!(response["v"], 2);
    assert!(
        response["capabilities"]
            .as_array()
            .unwrap()
            .contains(&json!("committed_loadout_selection_v1"))
    );
    assert!(response.get("loadout_selections").is_none());
    let response = select(&mut engine, 2, 1);
    assert!(
        serde_json::to_value(response)
            .unwrap()
            .get("capabilities")
            .is_none()
    );
}
#[test]
fn signed_native_selection_emits_only_committed_owned_desired_slot_without_equipping() {
    let mut engine = fixture();
    let response = select(&mut engine, 0xf9, 1);
    assert!(response.ok);
    let receipts = actions(&response);
    assert_eq!(receipts.len(), 1);
    let receipt = &receipts[0];
    assert_eq!(
        receipt
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>(),
        [
            "selection_id",
            "reservation_id",
            "world_id",
            "player_id",
            "account_id",
            "character_id",
            "revision",
            "inventory_index"
        ]
        .map(str::to_owned)
        .into_iter()
        .collect()
    );
    assert_eq!(receipt["inventory_index"], 1);
    assert_eq!(receipt["revision"], 1);
    assert_eq!(receipt["account_id"], "synthetic-account");
    assert_eq!(receipt["character_id"], 1);
    assert_eq!(receipt["reservation_id"], "synthetic-reservation-0000");
    assert_eq!(receipt["world_id"], 1);
    assert_eq!(receipt["player_id"], 0);
    let actor = engine.active.as_ref().unwrap();
    assert_eq!(actor.selected_loadout.unwrap().index, -7);
    assert_eq!(actor.current_inventory_handle.unwrap().inventory_index, 0);
    assert_eq!(actor.observed_primary.unwrap().ammo_count, 5);
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
}
#[test]
fn invalid_unowned_or_unpersistable_slots_keep_native_desire_but_emit_no_action() {
    for case in 0..7 {
        let mut engine = fixture();
        let actor = engine.active.as_mut().unwrap();
        let index = match case {
            0 => 7,
            1 => 2,
            2 => {
                actor.reservation.character.loadouts = None;
                1
            }
            3 => {
                actor.reservation.character.loadout_account_id = None;
                actor.reservation.character.account_id = None;
                1
            }
            4 => {
                actor.reservation.character.character_id = None;
                1
            }
            5 => {
                actor
                    .reservation
                    .character
                    .loadouts
                    .as_mut()
                    .unwrap()
                    .inventories[1][0] = Some(999);
                1
            }
            _ => {
                actor.reservation.character.faction = 3;
                1
            }
        };
        let response = select(&mut engine, index, 1);
        assert!(response.ok);
        assert!(actions(&response).is_empty(), "case {case}");
        assert_eq!(
            engine
                .active
                .as_ref()
                .unwrap()
                .selected_loadout
                .unwrap()
                .desired_inventory_index(),
            index & 7
        );
    }
    let mut denied = fixture();
    denied.active.as_mut().unwrap().claimed = None;
    let response = select(&mut denied, 1, 1);
    assert!(actions(&response).is_empty());
    assert!(denied.active.as_ref().unwrap().selected_loadout.is_none());
}
#[test]
fn latest_valid_selection_coalesces_and_retries_until_its_exact_outcome_settles() {
    let mut engine = fixture();
    let first = actions(&select(&mut engine, 1, 1))[0].clone();
    let second = actions(&select(&mut engine, 0, 2))[0].clone();
    assert_eq!(second["revision"], 2);
    assert_ne!(first["selection_id"], second["selection_id"]);
    assert_eq!(
        hello(&mut engine, 3)["loadout_selections"],
        json!([second.clone()])
    );
    assert!(ack(&mut engine, &first, "persisted").ok);
    assert_eq!(
        hello(&mut engine, 4)["loadout_selections"],
        json!([second.clone()])
    );
    let mut wrong = second.clone();
    wrong["world_id"] = json!(2);
    let _ = ack(&mut engine, &wrong, "rejected");
    assert_eq!(
        hello(&mut engine, 5)["loadout_selections"],
        json!([second.clone()])
    );
    assert!(ack(&mut engine, &second, "rejected").ok);
    assert!(hello(&mut engine, 6).get("loadout_selections").is_none());
    assert!(ack(&mut engine, &second, "persisted").ok);
    let third = actions(&select(&mut engine, 1, 7))[0].clone();
    assert_eq!(third["revision"], 3);
    assert!(ack(&mut engine, &third, "persisted").ok);
    assert!(hello(&mut engine, 8).get("loadout_selections").is_none());
}
#[test]
fn audit_failure_cannot_publish_or_consume_a_selection_and_world_response_retries_committed_work() {
    let mut engine = fixture();
    engine.max_log = engine.written;
    let response = select(&mut engine, 1, 1);
    assert!(!response.ok);
    assert!(actions(&response).is_empty());
    assert!(engine.active.as_ref().unwrap().selected_loadout.is_none());
    assert_eq!(
        engine.active.as_ref().unwrap().loadout_selection_revision,
        0
    );
    assert!(
        engine
            .active
            .as_ref()
            .unwrap()
            .pending_loadout_selection
            .is_none()
    );
    let mut engine = fixture();
    let receipt = actions(&select(&mut engine, 1, 2))[0].clone();
    assert_eq!(receipt["revision"], 1);
    engine.max_log = engine.written;
    let response = ack(&mut engine, &receipt, "persisted");
    assert!(!response.ok);
    let retry = engine.tick_world(Duration::from_secs(1), 1, "synthetic-world".into());
    assert_eq!(actions(&retry), vec![receipt]);
}
#[test]
fn owner_release_expiry_and_round_replacement_remove_pending_selections() {
    let mut released = fixture();
    select(&mut released, 1, 1);
    let response = released.handle(
        Request::Release {
            v: 2,
            request_id: "synthetic-release".into(),
            reservation_id: "synthetic-reservation-0000".into(),
        },
        Duration::from_millis(2),
        1,
    );
    assert!(response.ok);
    assert!(actions(&response).is_empty());
    let mut expired = fixture();
    select(&mut expired, 1, 1);
    assert!(
        hello(&mut expired, 121_000)
            .get("loadout_selections")
            .is_none()
    );
    let mut replaced = fixture();
    select(&mut replaced, 1, 1);
    replaced.reset_objective_world();
    assert!(hello(&mut replaced, 2).get("loadout_selections").is_none());
    assert_eq!(actions(&select(&mut replaced, 0, 3))[0]["revision"], 2);
}
#[test]
fn settlement_codec_requires_exact_v2_identity_revision_and_typed_outcome() {
    let base = json!({"op":"loadout_selection_settled","v":2,"request_id":"synthetic-ack","reservation_id":"synthetic-reservation-0000",
        "world_id":1,"player_id":0,"selection_id":"a".repeat(64),"revision":1,"status":"persisted"});
    assert!(parse_request(base.to_string().as_bytes()).is_ok());
    for (field, value) in [
        ("revision", json!(0)),
        ("revision", json!(-1)),
        ("status", json!("retry")),
        ("reason", json!("extra")),
        ("v", json!(3)),
    ] {
        let mut bad = base.clone();
        bad[field] = value;
        assert!(
            parse_request(bad.to_string().as_bytes()).is_err(),
            "{field}"
        );
    }
}

#[test]
fn exhausted_revision_preserves_native_desire_without_wrapping_or_publishing() {
    let mut engine = fixture();
    engine.active.as_mut().unwrap().loadout_selection_revision = u64::MAX;
    let response = select(&mut engine, 1, 1);
    assert!(response.ok);
    assert!(actions(&response).is_empty());
    let actor = engine.active.as_ref().unwrap();
    assert_eq!(actor.loadout_selection_revision, u64::MAX);
    assert_eq!(actor.selected_loadout.unwrap().desired_inventory_index(), 1);
}

#[test]
fn response_decoder_rejects_oversized_persistence_batches_and_unknown_action_fields() {
    let mut engine = fixture();
    let response = select(&mut engine, 1, 1);
    let mut value = serde_json::to_value(response).unwrap();
    let receipt = value["loadout_selections"][0].clone();
    value["loadout_selections"] = json!(vec![receipt.clone(); 33]);
    assert!(serde_json::from_value::<Response>(value.clone()).is_err());
    let mut extra = receipt;
    extra["reason"] = json!("unsupported");
    value["loadout_selections"] = json!([extra]);
    assert!(serde_json::from_value::<Response>(value).is_err());
}

#[test]
fn loadout_account_binding_does_not_enable_xp_and_must_agree_with_optional_progression_identity() {
    let mut engine = fixture();
    let actor = engine.active.as_ref().unwrap();
    assert!(actor.reservation.character.account_id.is_none());
    assert!(progression::RewardRecipient::from_active(actor).is_none());
    let character = actor.reservation.character.clone();
    let reserve = |character| Request::Reserve {
        v: 2,
        request_id: "synthetic-bound-account".into(),
        reservation_id: "synthetic-new-reservation".into(),
        world_id: 1,
        player_id: 1,
        account_id: 2,
        session_key: "aaaaaaaaaaaaaaaa".into(),
        access_key: "bbbbbbbbbbbbbbbb".into(),
        character,
    };
    assert!(reserve(character.clone()).validate().is_ok());
    let mut mismatched = character.clone();
    mismatched.account_id = Some("different-account".into());
    mismatched.progression = Some(progression::ProgressionSnapshot::default());
    assert_eq!(
        reserve(mismatched).validate(),
        Err("loadout_account_identity_mismatch")
    );
    let mut missing_character = character.clone();
    missing_character.character_id = None;
    assert_eq!(
        reserve(missing_character).validate(),
        Err("invalid_loadout_persistence_identity")
    );
    let mut invalid = character;
    invalid.loadout_account_id = Some("bad!".into());
    assert_eq!(
        reserve(invalid).validate(),
        Err("invalid_loadout_persistence_identity")
    );
    let receipt = actions(&select(&mut engine, 1, 1))[0].clone();
    assert_eq!(receipt["account_id"], "synthetic-account");
    assert!(engine.pending_reward_batch().is_empty());
    let actor = engine.active.as_mut().unwrap();
    actor.reservation.character.loadout_account_id = None;
    actor.reservation.character.account_id = Some("synthetic-account".into());
    actor.reservation.character.progression = Some(progression::ProgressionSnapshot::default());
    assert_eq!(
        actions(&select(&mut engine, 0, 2))[0]["account_id"],
        "synthetic-account"
    );
}

#[test]
fn saved_primary_with_valid_raw_binding_still_requires_owned_faction_eligibility() {
    let mut engine = fixture();
    let armory=retail_armory::RetailArmory::from_json(&json!({
        "schema_version":1,"application_id":20231,"source_sha256":{"synthetic":"a".repeat(64)},
        "items":[{"wire_id":2,"name":"synthetic_weapon","authored_ref":"FactionItemList0:2","available_bit":2,"factions":[0],"primary_ammo":7}],
        "factions":(0..3).map(|faction|json!({"faction":faction,"available_item_bits":[2],"skill_ids":[],
            "loadouts":(0..5).map(|_|json!({"update_id":1,"items":vec![None::<u16>;22]})).collect::<Vec<_>>()
        })).collect::<Vec<_>>()
    }).to_string()).unwrap();
    let catalog = std::sync::Arc::try_unwrap(engine.item_catalog).unwrap();
    engine.item_catalog = std::sync::Arc::new(catalog.with_retail_armory(armory));
    let response = select(&mut engine, 1, 1);
    assert!(response.ok);
    assert!(actions(&response).is_empty());
    assert_eq!(
        engine
            .active
            .as_ref()
            .unwrap()
            .selected_loadout
            .unwrap()
            .desired_inventory_index(),
        1
    );
}
