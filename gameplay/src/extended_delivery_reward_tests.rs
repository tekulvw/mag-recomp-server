//! Native operator-only delivery settlement in the connected Acquisition engine.
use super::*;
use spawn_progression::tests::dynamic_bodies::acquisition_gameplay_tests::delivery;
fn configure(a: &mut Active) {
    let mut value = serde_json::to_value(&a.reservation.character).unwrap();
    value.as_object_mut().unwrap().remove("skills");
    value["extended_objective_rewards_enabled"] = json!(true);
    value["frago_objective_bonus_enabled"] = json!(true);
    a.reservation.character = serde_json::from_value(value).unwrap();
    a.reservation.character.account_id = Some(format!("delivery-account-{}", a.reservation.player));
    a.reservation.character.progression = Some(Default::default());
    a.claimed = Some(Claimed {
        wire: vec![],
        np_identity: [0; 36],
        game_info: protocol::GameInfo {
            client_index: a.reservation.player,
            level_name: "synthetic_arena".into(),
            start_time_ms: 0,
            attacking_faction: 1,
            defending_faction: 0,
            maximum_points: [1200, 1000],
            current_points: [1200, 1000],
            game_type: 1,
        },
    });
}
fn fixture() -> Engine<Vec<u8>> {
    let mut e = delivery::delivery_fixture();
    configure(e.active.as_mut().unwrap());
    let catalog = e.objective_catalog.as_mut().unwrap();
    let geometry = json!({"schema_version":1,"map_name":"synthetic_arena","game_type":1,
        "source":{"authored_catalog_sha256":catalog.source().authored_catalog_sha256,
            "source_image_sha256":catalog.source().source_image_sha256,"progression_catalog_sha256":"b".repeat(64)},
        "vehicle_range":1.,"objectives":catalog.objects.iter().map(|o|json!({
            "object_index":o.object_index,"position":if o.object_index==100 {[1000.,3.,1040.]} else {[5000.,3.,5000.]},"radius":0.})).collect::<Vec<_>>()});
    catalog.frago = Some(serde_json::from_value(geometry).unwrap());
    e.acquisition_gameplay
        .as_mut()
        .unwrap()
        .state
        .squad_mut(0)
        .unwrap()
        .assignment = acquisition_objective_runtime::Assignment {
        primary_index: u16::MAX,
        tagged_object: 0x0c80_0064,
    };
    delivery::position(&mut e, 1000., 1040.);
    e
}
fn tick(e: &mut Engine<Vec<u8>>) -> Result<Vec<OutboundMessage>, &'static str> {
    e.tick_acquisition_objectives(Duration::from_millis(3400), 1)
}
#[test]
fn delivery_credits_only_operator_and_uses_its_mounted_pose_before_exit_and_reassignment() {
    let mut e = fixture();
    spawn_progression::reserve(&mut e, 1);
    let a = e.active.as_mut().unwrap();
    spawn_progression::ready(a);
    a.life = CharacterLife::Alive {
        generation: 1,
        health_quarter_points: 400,
    };
    a.ground_seat = Some(ground_seats::PassengerSeat {
        vehicle_index: 4,
        seat: 2,
    });
    a.ground_vehicle_batch = 1;
    configure(a);
    tick(&mut e).unwrap();
    let rewards = e.pending_reward_batch();
    assert_eq!(rewards.len(), 1);
    assert_eq!(
        serde_json::to_value(rewards[0].kind).unwrap(),
        json!("escort_vehicle_delivered")
    );
    assert_eq!(
        (rewards[0].player_id, rewards[0].units, rewards[0].frago),
        (0, 1, Some(true))
    );
    assert!(e.active.as_ref().unwrap().ground_seat.is_none());
    assert!(e.peers["synthetic-reservation-0000"].ground_seat.is_none());
    tick(&mut e).unwrap();
    assert_eq!(e.pending_reward_batch(), rewards);
}
#[test]
fn delivery_record_failure_keeps_occupants_vehicle_objective_and_reward_retryable() {
    let mut e = fixture();
    let cap = e.max_log;
    e.max_log = e.written;
    assert_eq!(tick(&mut e).unwrap_err(), "log_limit_reached");
    assert!(e.pending_rewards.is_empty());
    assert!(e.active.as_ref().unwrap().ground_seat.is_some());
    assert_eq!(e.ground_runtime.as_ref().unwrap().get(4).unwrap().state, 2);
    e.max_log = cap;
    e.failed = false;
    tick(&mut e).unwrap();
    assert_eq!(e.pending_reward_batch().len(), 1);
}
#[test]
fn absent_extended_admission_preserves_delivery_gameplay_without_new_aliases() {
    let mut e = fixture();
    e.active
        .as_mut()
        .unwrap()
        .reservation
        .character
        .extended_objective_rewards_enabled = None;
    tick(&mut e).unwrap();
    assert_eq!(e.ground_runtime.as_ref().unwrap().get(4).unwrap().state, 8);
    assert!(e.pending_rewards.is_empty());
}

#[test]
fn staging_then_superseding_a_delivery_does_not_award_an_unentered_callback() {
    let mut e = fixture();
    let mut runtime = e.acquisition_gameplay.clone().unwrap();
    let mut ground = e.ground_runtime.clone().unwrap();
    let mut changes = std::collections::BTreeMap::new();
    let mut actors = std::collections::BTreeMap::new();
    let mut witnesses = std::collections::BTreeMap::new();
    e.stage_acquisition_deliveries(
        &mut runtime,
        &mut ground,
        Duration::from_millis(3400),
        &mut changes,
        &mut actors,
        &mut witnesses,
    )
    .unwrap();
    let token = *actors.keys().next().unwrap();
    runtime
        .state
        .request_state(token, replication::ObjectiveState::Deactivated)
        .unwrap();
    runtime
        .state
        .request_state(token, replication::ObjectiveState::Completed)
        .unwrap();
    assert!(
        e.pending_rewards.is_empty(),
        "planning is not an accepted Completion callback"
    );
    e.stage_acquisition_delivery_reward(
        &runtime,
        &ground,
        token,
        witnesses.remove(&token).unwrap(),
    )
    .unwrap();
    assert!(
        e.pending_rewards.is_empty(),
        "a superseded pending request must not reuse its mounted witness"
    );
}

#[test]
fn accepted_delivery_callback_uses_current_assignment_and_its_frozen_mounted_pose() {
    for (current_tag, expected) in [(u32::MAX, None), (0x0c80_0064, Some(true))] {
        let mut e = fixture();
        let mut runtime = e.acquisition_gameplay.clone().unwrap();
        let mut ground = e.ground_runtime.clone().unwrap();
        let mut changes = std::collections::BTreeMap::new();
        let mut actors = std::collections::BTreeMap::new();
        let mut witnesses = std::collections::BTreeMap::new();
        e.stage_acquisition_deliveries(
            &mut runtime,
            &mut ground,
            Duration::from_millis(3400),
            &mut changes,
            &mut actors,
            &mut witnesses,
        )
        .unwrap();
        assert!(e.pending_rewards.is_empty());
        let token = *actors.keys().next().unwrap();
        runtime.state.squad_mut(0).unwrap().assignment =
            acquisition_objective_runtime::Assignment {
                primary_index: u16::MAX,
                tagged_object: current_tag,
            };
        e.stage_acquisition_delivery_reward(
            &runtime,
            &ground,
            token,
            witnesses.remove(&token).unwrap(),
        )
        .unwrap();
        assert_eq!(e.pending_reward_batch()[0].frago, expected);
    }
}
