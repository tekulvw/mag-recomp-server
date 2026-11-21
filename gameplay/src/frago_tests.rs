//! Source-shaped contextual kill and immutable-stat acknowledgement checks.
use super::*;

fn geometry() -> Value {
    json!({"schema_version":1,"map_name":"synthetic_arena","game_type":1,"source":{"authored_catalog_sha256":"a".repeat(64),
        "progression_catalog_sha256":"b".repeat(64),"source_image_sha256":"c".repeat(64)},
        "vehicle_range":7.,"objectives":[{"object_index":100,"position":[100.,3.,100.],"radius":5.}]})
}
fn catalog_input() -> Value {
    json!({"schema_version":1,"format":"mag_retail_objective_catalog","application_id":20231,"protocol":0x6cc,
        "game_type":1,"map_name":"synthetic_arena","entity_family":12,"network_type":12,
        "source":{"authored_catalog_sha256":"a".repeat(64),"binding_capture_sha256":"1".repeat(64),
        "startup_proof_sha256":"2".repeat(64),"source_image_sha256":"c".repeat(64),"capture_qualification":"native_source_gameplay"},
        "objects":[{"token":"0x01234567","object_index":100,"initial_state":1}],
        "squads":(0..2).flat_map(|team|(0..8).map(move|slot|json!({"team":team,"squad_index":team*16+slot,
            "initial_objective_index":100,"authored_objectives":[{"object_index":100,"order":0}]}))).collect::<Vec<_>>(),
        "frago":geometry()})
}
#[test]
fn complete_source_bound_geometry_loads_and_legacy_shape_remains_valid() {
    let mut input = catalog_input();
    assert!(objective_catalog::ObjectiveCatalog::from_json(&input.to_string()).is_ok());
    input.as_object_mut().unwrap().remove("frago");
    assert!(objective_catalog::ObjectiveCatalog::from_json(&input.to_string()).is_ok());
}

fn fixture() -> Engine<Vec<u8>> {
    let base =
        spawn_progression::tests::dynamic_bodies::acquisition_gameplay_tests::capture_fixture();
    let mut e = human_kill_reward_tests::configured_fixture(base);
    e.match_started = true;
    let map_name = e.map_name().to_owned();
    let catalog = e.objective_catalog.as_mut().unwrap();
    let input = json!({"schema_version":1,"map_name":"synthetic_arena","game_type":1,"source":{"authored_catalog_sha256":catalog.source().authored_catalog_sha256,
        "progression_catalog_sha256":"b".repeat(64),"source_image_sha256":catalog.source().source_image_sha256},
        "vehicle_range":7.,"objectives":catalog.objects.iter().map(|o|json!({"object_index":o.object_index,"position":[100.,3.,100.],"radius":5.})).collect::<Vec<_>>()});
    let mut input = input;
    input["map_name"] = json!(map_name);
    catalog.frago = Some(serde_json::from_value(input).unwrap());
    let killer = e.active.as_mut().unwrap();
    killer.reservation.character.frago_bonus_enabled = Some(true);
    killer.tracking.server_position(
        replication::CharacterPosition {
            x: 130.,
            y: 3.,
            z: 100.,
        },
        player_tracking::PositionOrigin::ServerSpawn,
        0,
    );
    e.peers
        .get_mut("synthetic-reservation-0000")
        .unwrap()
        .tracking
        .server_position(
            replication::CharacterPosition {
                x: 104.,
                y: 3.,
                z: 103.,
            },
            player_tracking::PositionOrigin::ServerSpawn,
            0,
        );
    e.acquisition_gameplay
        .as_mut()
        .unwrap()
        .state
        .squad_mut(16)
        .unwrap()
        .assignment = acquisition_objective_runtime::Assignment {
        primary_index: u16::MAX,
        tagged_object: 0x0c80_0064,
    };
    e
}
fn kill(e: &mut Engine<Vec<u8>>) -> progression::RewardEvent {
    let before: std::collections::BTreeSet<_> = e.pending_rewards.keys().cloned().collect();
    human_kill_reward_tests::hit(e, 20, 1000).unwrap();
    e.pending_rewards
        .values()
        .find(|r| r.kind == progression::RewardKind::EnemyKilled && !before.contains(&r.reward_id))
        .unwrap()
        .clone()
}
#[test]
fn qualified_kill_uses_the_victim_at_the_inclusive_three_dimensional_boundary() {
    let mut e = fixture();
    let reward = kill(&mut e);
    assert_eq!(serde_json::to_value(&reward).unwrap()["frago"], json!(true));
    // The eligibility fact is frozen in the existing immutable reward.
    e.acquisition_gameplay
        .as_mut()
        .unwrap()
        .state
        .squad_mut(16)
        .unwrap()
        .assignment
        .tagged_object = u32::MAX;
    assert_eq!(e.pending_rewards[&reward.reward_id], reward);
}
#[test]
fn vertical_separation_and_missing_admission_capability_keep_base_kill_only() {
    let mut e = fixture();
    e.peers
        .get_mut("synthetic-reservation-0000")
        .unwrap()
        .tracking
        .server_position(
            replication::CharacterPosition {
                x: 104.,
                y: 9.,
                z: 103.,
            },
            player_tracking::PositionOrigin::ServerSpawn,
            0,
        );
    assert!(kill(&mut e).frago.is_none());
    let mut legacy = fixture();
    legacy
        .active
        .as_mut()
        .unwrap()
        .reservation
        .character
        .frago_bonus_enabled = None;
    assert!(kill(&mut legacy).frago.is_none());
}

fn ack_value(event: &progression::RewardEvent, xp: u32, points: Option<u32>) -> Value {
    let mut value = json!({"op":"progression_committed","v":BRIDGE_VERSION,"request_id":"synthetic-frago-ack",
        "reservation_id":event.reservation_id,"world_id":event.world_id,"player_id":event.player_id,"reward_id":event.reward_id,
        "progression":{"experience":xp,"level":1,"skill_points":0},"match_experience":xp});
    if let Some(points) = points {
        value["match_frago_points"] = json!(points);
    }
    value
}
fn acknowledge(e: &mut Engine<Vec<u8>>, value: Value) -> Response {
    e.handle(
        serde_json::from_value(value).unwrap(),
        Duration::from_millis(1100),
        3,
    )
}
#[test]
fn committed_bonus_publishes_absolute_stat3_and_replays_without_awarding_again() {
    let mut e = fixture();
    let reward = kill(&mut e);
    let ack = ack_value(&reward, 18, Some(9));
    let r = acknowledge(&mut e, ack.clone());
    assert!(r.ok, "{:?}", r.error);
    assert_eq!(
        e.active.as_ref().unwrap().reward_statistics.get(&3),
        Some(&9)
    );
    // Literal native0204: type, character1, two stats, XP18 and FRAGO9.
    assert!(
        r.messages
            .iter()
            .any(|m| protocol::decode_hex(&m.payload_hex) == vec![2, 4, 0, 1, 2, 2, 18, 3, 9])
    );
    assert!(acknowledge(&mut e, ack).messages.is_empty());
    assert!(e.pending_rewards.is_empty());
}

#[test]
fn a_geometry_block_cannot_move_to_a_different_map_with_the_same_global_source_hash() {
    let mut input = catalog_input();
    input["map_name"] = json!("another_arena");
    assert!(objective_catalog::ObjectiveCatalog::from_json(&input.to_string()).is_err());
}

fn configure_targets(e: &mut Engine<Vec<u8>>, targets: &[(u16, [f32; 3], f32)]) {
    let map_name = e.map_name().to_owned();
    let c = e.objective_catalog.as_mut().unwrap();
    let mut value = geometry();
    value["map_name"] = json!(map_name);
    value["source"]["authored_catalog_sha256"] = json!(c.source().authored_catalog_sha256);
    value["source"]["source_image_sha256"] = json!(c.source().source_image_sha256);
    value["objectives"] = json!(
        c.objects
            .iter()
            .map(|o| {
                let (_, p, r) = targets
                    .iter()
                    .find(|(i, _, _)| *i == o.object_index)
                    .copied()
                    .unwrap_or((o.object_index, [1000., 3., 1000.], 1.));
                json!({"object_index":o.object_index,"position":p,"radius":r})
            })
            .collect::<Vec<_>>()
    );
    c.frago = Some(serde_json::from_value(value).unwrap());
}
#[test]
fn primary_assignment_can_qualify_when_a_distinct_manual_tag_is_outside_range() {
    let mut e = fixture();
    configure_targets(
        &mut e,
        &[(100, [1000., 3., 1000.], 5.), (101, [100., 3., 100.], 5.)],
    );
    e.acquisition_gameplay
        .as_mut()
        .unwrap()
        .state
        .squad_mut(16)
        .unwrap()
        .assignment
        .primary_index = 101;
    assert_eq!(kill(&mut e).frago, Some(true));
}
#[test]
fn explicit_zero_radius_qualifies_only_the_exact_current_coordinate_even_when_inactive() {
    let mut e = fixture();
    configure_targets(&mut e, &[(100, [104., 3., 103.], 0.)]);
    assert_eq!(
        e.objective_world.as_ref().unwrap().state(100),
        Some(replication::ObjectiveState::Inactive)
    );
    assert_eq!(kill(&mut e).frago, Some(true));
    let mut outside = fixture();
    configure_targets(&mut outside, &[(100, [104., 3., 103.001], 0.)]);
    assert_eq!(kill(&mut outside).frago, None);
}
#[test]
fn killer_position_can_qualify_without_a_nearby_context_and_unknown_target_does_not() {
    let mut e = fixture();
    configure_targets(&mut e, &[(100, [130., 3., 100.], 0.)]);
    assert_eq!(kill(&mut e).frago, Some(true));
    let mut e = fixture();
    e.acquisition_gameplay
        .as_mut()
        .unwrap()
        .state
        .squad_mut(16)
        .unwrap()
        .assignment
        .tagged_object = 0x0c80_007f;
    assert_eq!(kill(&mut e).frago, None);
}
#[test]
fn failed_kill_record_installs_neither_bonus_fact_nor_damage_then_retry_is_once() {
    let mut e = fixture();
    let max = e.max_log;
    e.max_log = e.written;
    assert_eq!(
        human_kill_reward_tests::hit(&mut e, 20, 1000).unwrap_err(),
        "log_limit_reached"
    );
    assert!(e.pending_rewards.is_empty());
    assert!(matches!(
        e.peers["synthetic-reservation-0000"].life,
        CharacterLife::Alive {
            health_quarter_points: 120,
            ..
        }
    ));
    e.max_log = max;
    e.failed = false;
    assert_eq!(kill(&mut e).frago, Some(true));
    assert_eq!(e.pending_rewards.len(), 1);
}
#[test]
fn missing_or_invalid_bonus_ack_keeps_the_receipt_pending() {
    for (points, want) in [
        (None, "frago_snapshot_required"),
        (Some(19), "invalid_match_frago_points"),
    ] {
        let mut e = fixture();
        let reward = kill(&mut e);
        let r = acknowledge(&mut e, ack_value(&reward, 18, points));
        assert_eq!(r.error.as_deref(), Some(want));
        assert_eq!(e.pending_rewards.len(), 1);
        assert!(
            !e.active
                .as_ref()
                .unwrap()
                .reward_statistics
                .contains_key(&3)
        );
    }
}
#[test]
fn failed_bonus_ack_record_preserves_stats_and_outbox_then_retries_exactly() {
    let mut e = fixture();
    let reward = kill(&mut e);
    let value = ack_value(&reward, 18, Some(9));
    let max = e.max_log;
    e.max_log = e.written;
    let r = acknowledge(&mut e, value.clone());
    assert_eq!(r.error.as_deref(), Some("log_limit_reached"));
    assert_eq!(e.pending_rewards.len(), 1);
    assert!(
        !e.active
            .as_ref()
            .unwrap()
            .reward_statistics
            .contains_key(&3)
    );
    e.max_log = max;
    e.failed = false;
    assert!(acknowledge(&mut e, value).ok);
    let conflict = acknowledge(&mut e, ack_value(&reward, 18, Some(8)));
    assert_eq!(conflict.error.as_deref(), Some("reward_ack_conflict"));
    assert_eq!(
        e.active.as_ref().unwrap().reward_statistics.get(&3),
        Some(&9)
    );
}
#[test]
fn qualified_zero_is_explicit_and_old_round_ack_cannot_credit_the_new_round() {
    let mut zero = fixture();
    let reward = kill(&mut zero);
    let r = acknowledge(&mut zero, ack_value(&reward, 0, Some(0)));
    assert!(r.ok);
    assert!(
        r.messages
            .iter()
            .any(|m| protocol::decode_hex(&m.payload_hex) == vec![2, 4, 0, 1, 1, 3, 0])
    );
    let mut e = fixture();
    let reward = kill(&mut e);
    e.reward_match_id = Some("synthetic-new-round".into());
    let a = e.active.as_mut().unwrap();
    a.reward_match_id = "synthetic-new-round".into();
    a.match_experience = 2;
    a.reward_statistics.insert(3, 1);
    assert!(acknowledge(&mut e, ack_value(&reward, 18, Some(9))).ok);
    let a = e.active.as_ref().unwrap();
    assert_eq!(a.match_experience, 2);
    assert_eq!(a.reward_statistics.get(&3), Some(&1));
    assert_eq!(a.reservation.character.progression.unwrap().experience, 18);
}
#[test]
fn replacement_character_does_not_receive_an_old_bonus_snapshot() {
    let mut e = fixture();
    let reward = kill(&mut e);
    e.active
        .as_mut()
        .unwrap()
        .reservation
        .character
        .character_id = Some(99);
    assert!(acknowledge(&mut e, ack_value(&reward, 18, Some(9))).ok);
    assert!(
        !e.active
            .as_ref()
            .unwrap()
            .reward_statistics
            .contains_key(&3)
    );
    assert_eq!(e.active.as_ref().unwrap().match_experience, 0);
}
#[test]
fn malformed_context_and_incomplete_geometry_reject_without_invented_fallback() {
    let mut e = fixture();
    let mut reward = kill(&mut e);
    reward.frago = Some(false);
    assert_eq!(
        e.check_reward_capacity(&[reward.clone()]),
        Err("invalid_frago_reward_context")
    );
    reward.frago = Some(true);
    reward.units = 2;
    assert_eq!(
        e.check_reward_capacity(&[reward]),
        Err("invalid_frago_reward_context")
    );
    for mutation in 0..4 {
        let mut input = catalog_input();
        match mutation {
            0 => input["frago"]["objectives"] = json!([]),
            1 => input["frago"]["objectives"][0]["radius"] = json!(-1),
            2 => input["frago"]["source"]["source_image_sha256"] = json!("d".repeat(64)),
            _ => input["frago"]["objectives"][0]["object_index"] = json!(101),
        }
        assert!(objective_catalog::ObjectiveCatalog::from_json(&input.to_string()).is_err());
    }
}

#[test]
fn older_distinct_receipt_cannot_regress_cumulative_match_bonus() {
    let mut e = fixture();
    let first = kill(&mut e);
    let victim = e.peers.get_mut("synthetic-reservation-0000").unwrap();
    victim.life = CharacterLife::Alive {
        generation: 4,
        health_quarter_points: 120,
    };
    victim.last_damage.health_quarter_points = 120;
    let second = kill(&mut e);
    assert_ne!(first.reward_id, second.reward_id);
    assert!(acknowledge(&mut e, ack_value(&second, 36, Some(18))).ok);
    assert!(acknowledge(&mut e, ack_value(&first, 18, Some(9))).ok);
    assert_eq!(
        e.active.as_ref().unwrap().reward_statistics.get(&3),
        Some(&18)
    );
    assert_eq!(e.active.as_ref().unwrap().match_experience, 36);
}
#[test]
fn friendly_collapse_does_not_create_a_contextual_enemy_reward() {
    let mut e = fixture();
    e.active.as_mut().unwrap().squad_catalog =
        std::sync::Arc::clone(&e.peers["synthetic-reservation-0000"].squad_catalog);
    human_kill_reward_tests::hit(&mut e, 20, 1000).unwrap();
    assert!(e.pending_rewards.is_empty());
}
#[test]
fn source_world_damage_uses_the_credited_killers_squad_even_with_victim_active() {
    let mut e = fixture();
    let victim = e.peers.remove("synthetic-reservation-0000").unwrap();
    let killer = e.active.replace(victim).unwrap();
    e.peers.insert(killer.reservation.id.clone(), killer);
    let damage = replication::CharacterDamage {
        health_quarter_points: 0,
        last_location_and_type: 0xa3,
        conditions: 2,
        last_attacker: 0x0380_0001,
        last_source_object: 0x1080_0002,
        last_user_data: 0,
    };
    let plan = events::plan_character_effect_consequences(
        &e,
        events::CharacterEffectInput {
            world: 1,
            victim_id: 0,
            generation: 4,
            attacker: 0x0380_0001,
            damage,
            life: Some(CharacterLife::AwaitingRespawn { generation: 4 }),
            now: Duration::from_millis(1000),
        },
    )
    .unwrap();
    plan.validate(&e).unwrap();
    plan.commit(&mut e);
    assert_eq!(
        e.pending_rewards
            .values()
            .find(|r| r.kind == progression::RewardKind::EnemyKilled)
            .unwrap()
            .frago,
        Some(true)
    );
}

#[test]
fn hello_negotiates_frago_admission_without_requiring_a_geometry_catalog() {
    let mut e = fixture();
    e.objective_catalog.as_mut().unwrap().frago = None;
    let r = e.handle(
        Request::Hello {
            v: BRIDGE_VERSION,
            request_id: "synthetic-frago-hello".into(),
        },
        Duration::ZERO,
        3,
    );
    assert!(r.ok);
    assert!(r.capabilities.iter().any(|c| c == "frago_kill_bonus_v1"));
}

#[test]
fn queued_grenade_uses_the_accepted_owner_position_at_the_kill_in_both_directions() {
    for (prior, current, want) in [(106., 104., Some(true)), (104., 106., None)] {
        let mut e = fixture();
        e.active.as_mut().unwrap().tracking.server_position(
            replication::CharacterPosition {
                x: prior,
                y: 3.,
                z: 100.,
            },
            player_tracking::PositionOrigin::ServerSpawn,
            0,
        );
        e.peers
            .get_mut("synthetic-reservation-0000")
            .unwrap()
            .tracking
            .server_position(
                replication::CharacterPosition {
                    x: 120.,
                    y: 3.,
                    z: 100.,
                },
                player_tracking::PositionOrigin::ServerSpawn,
                0,
            );
        let batches = e.ground_vehicles.as_ref().unwrap().batch_count();
        let active = e.active.as_mut().unwrap();
        active.ground_vehicle_batch = batches;
        active.grenade_damage.push_back(PendingDamage {
            target_character_id: 0x0380_0000,
            target_reservation_id: Some("synthetic-reservation-0000".into()),
            id: "synthetic-moved-grenade".into(),
            generation: 4,
            falling: false,
            damage: damage::ResolvedDamage {
                amount_hp: 40.,
                kind: damage::DamageKind::Explosive,
                kinetic_response: true,
                location: 3,
                attacker_id: 0x0380_0001,
                source_object_id: 0x1080_0001,
                user_data: 0,
            },
        });
        let fields = player_tracking::ClientCharacterFields {
            position: Some(replication::CharacterPosition {
                x: current,
                y: 3.,
                z: 100.,
            }),
            ..Default::default()
        };
        let wire = replication::multiplayer::CharacterOwnerDelta {
            sequence: 1,
            timestamp_ms: 1000,
            player: 1,
            fields: &fields,
        }
        .encode_with_profile(e.wire_profile)
        .unwrap();
        spawn_progression::tests::dispatch(&mut e, &wire, 1000).unwrap();
        assert_eq!(
            e.active
                .as_ref()
                .unwrap()
                .tracking
                .sample
                .unwrap()
                .position
                .x,
            current
        );
        let reward = e
            .pending_rewards
            .values()
            .find(|r| r.kind == progression::RewardKind::EnemyKilled)
            .unwrap();
        assert_eq!(reward.frago, want);
    }
}

#[test]
fn accepted_ground_candidate_supplies_both_tagged_vehicle_and_mounted_killer_roots() {
    for mounted in [false, true] {
        let mut e = fixture();
        e.peers
            .get_mut("synthetic-reservation-0000")
            .unwrap()
            .tracking
            .server_position(
                replication::CharacterPosition {
                    x: 120.,
                    y: 3.,
                    z: 100.,
                },
                player_tracking::PositionOrigin::ServerSpawn,
                0,
            );
        let key = e.ground_runtime.as_ref().unwrap().get(4).unwrap().key;
        let pose = |x| acquisition_ground_runtime::Pose {
            position: replication::CharacterPosition { x, y: 3., z: 100. },
            rotation: [0., 0., 0., 1.],
        };
        e.ground_runtime
            .as_mut()
            .unwrap()
            .force_pose(key, pose(150.))
            .unwrap();
        let mut candidate = e.ground_runtime.clone().unwrap();
        candidate
            .force_pose(key, pose(if mounted { 100. } else { 130. }))
            .unwrap();
        if mounted {
            e.active.as_mut().unwrap().ground_seat = Some(ground_seats::PassengerSeat {
                vehicle_index: 4,
                seat: 0,
            });
        } else {
            e.acquisition_gameplay
                .as_mut()
                .unwrap()
                .state
                .squad_mut(16)
                .unwrap()
                .assignment
                .tagged_object = 0x0100_0004;
        }
        let killer = e.active.as_ref().unwrap();
        let victim = &e.peers["synthetic-reservation-0000"];
        let reward = progression::RewardRecipient::from_active(killer)
            .unwrap()
            .human_killed(&victim.reservation, 4, 1);
        let context = e.frago_character_position(victim, frago::Frame::default());
        assert_eq!(
            e.frago_kill_reward(reward.clone(), killer, context, frago::Frame::default())
                .frago,
            None
        );
        assert_eq!(
            e.frago_kill_reward(
                reward,
                killer,
                context,
                frago::Frame {
                    ground: Some(&candidate),
                    ..Default::default()
                }
            )
            .frago,
            Some(true)
        );
    }
}
