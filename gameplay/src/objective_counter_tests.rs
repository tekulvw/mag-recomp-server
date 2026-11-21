//! Synthetic zero-XP objective counters, isolated from account settlement.
use super::*;
use acquisition_objective_runtime::Assignment;

fn fixture() -> Engine<Vec<u8>> {
    extended_objective_reward_tests::fixture(3, 4)
}
fn tag(squad: u16, target: u32) -> (u16, Assignment) {
    (
        squad,
        Assignment {
            primary_index: 120,
            tagged_object: target,
        },
    )
}
#[test]
fn zero_xp_counters_do_not_require_an_account_or_xp_admission() {
    let mut e = fixture();
    let a = e.active.as_mut().unwrap();
    a.reservation.character.account_id = None;
    a.reservation.character.character_id = None;
    a.reservation.character.extended_objective_rewards_enabled = None;
    a.reservation.character.frago_objective_bonus_enabled = None;
    let player = a.reservation.player;
    let effects = e
        .stage_objective_counters(player, 120, &[43, 58], true, &[tag(0, 0x0c80_0078)])
        .unwrap();
    let a = e.active.as_ref().unwrap();
    assert_eq!(
        a.reward_statistics,
        [(41, 1), (43, 1), (58, 1)].into_iter().collect()
    );
    assert_eq!(a.match_experience, 0);
    assert!(e.pending_reward_batch().is_empty());
    assert!(!effects.audit.is_empty());
    assert_eq!(effects.messages.len(), 1);
    assert_eq!(
        protocol::decode_hex(&effects.messages[0].payload_hex),
        protocol::StatUpdate {
            character_index: player,
            stats: vec![
                protocol::StatValue { stat: 41, value: 1 },
                protocol::StatValue { stat: 43, value: 1 },
                protocol::StatValue { stat: 58, value: 1 }
            ]
        }
        .encode_with_profile(e.wire_profile)
        .unwrap()
    );
}
#[test]
fn tagged_completion_uses_exact_family_slot_not_primary_or_range() {
    for (target, count) in [
        (0x0c00_0078, 1),
        (0x0cfe_0078, 1),
        (0x0180_0078, 0),
        (0x0c80_0079, 0),
        (u32::MAX, 0),
    ] {
        let mut e = fixture();
        let a = e.active.as_mut().unwrap();
        a.life = CharacterLife::AwaitingRespawn { generation: 1 };
        a.tracking = Default::default();
        let player = a.reservation.player;
        e.stage_objective_counters(
            player,
            120,
            &[],
            true,
            &[tag(0, target), tag(16, 0x0c80_0078)],
        )
        .unwrap();
        assert_eq!(
            e.active
                .as_ref()
                .unwrap()
                .reward_statistics
                .get(&41)
                .copied()
                .unwrap_or(0),
            count,
            "tag {target:08x}"
        );
    }
}
#[test]
fn counter_and_tag_updates_roll_back_with_the_outer_record_and_retry_once() {
    let mut e = fixture();
    let player = e.active.as_ref().unwrap().reservation.player;
    let native = e.owned_acquisition_characters();
    let plan = e
        .prepare_acquisition_character_callback(&native, Duration::ZERO, |engine, _| {
            let effects = engine.stage_objective_counters(
                player,
                120,
                &[43],
                true,
                &[tag(0, 0x0c80_0078)],
            )?;
            Ok((effects.audit, effects.messages))
        })
        .unwrap();
    assert!(e.active.as_ref().unwrap().reward_statistics.is_empty());
    let cap = e.max_log;
    e.max_log = e.written;
    let mut owned = native.clone();
    assert_eq!(
        e.record_acquisition_character_effects(plan, &mut owned, 1)
            .unwrap_err(),
        "log_limit_reached"
    );
    assert!(e.active.as_ref().unwrap().reward_statistics.is_empty());
    e.max_log = cap;
    e.failed = false;
    let plan = e
        .prepare_acquisition_character_callback(&native, Duration::ZERO, |engine, _| {
            let effects = engine.stage_objective_counters(
                player,
                120,
                &[43],
                true,
                &[tag(0, 0x0c80_0078)],
            )?;
            Ok((effects.audit, effects.messages))
        })
        .unwrap();
    let messages = e
        .record_acquisition_character_effects(plan, &mut owned, 1)
        .unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(
        e.active.as_ref().unwrap().reward_statistics,
        [(41, 1), (43, 1)].into_iter().collect()
    );
}
#[test]
fn callback_counts_saturate_and_missing_actor_or_leader_cannot_receive_credit() {
    let mut e = fixture();
    let a = e.active.as_mut().unwrap();
    let player = a.reservation.player;
    a.reward_statistics.insert(43, i32::MAX);
    e.stage_objective_counters(player, 120, &[43], true, &[tag(1, 0x0c80_0078)])
        .unwrap();
    assert_eq!(
        e.active.as_ref().unwrap().reward_statistics,
        [(43, i32::MAX)].into_iter().collect()
    );
    let effects = e
        .stage_objective_counters(u16::MAX, 120, &[58], true, &[tag(0, 0x0c80_0078)])
        .unwrap();
    assert!(effects.messages.is_empty());
    assert!(
        !e.active
            .as_ref()
            .unwrap()
            .reward_statistics
            .contains_key(&58)
    );
}
#[test]
fn rebuilt_defuse_uses_existing_xp_reward_without_repair_or_primary_awards() {
    use replication::ObjectiveState as S;
    let mut e = fixture();
    extended_objective_reward_tests::callback(&mut e, S::Deactivating, S::Repaired).unwrap();
    let rewards = e.pending_reward_batch();
    assert_eq!(rewards.len(), 1);
    assert_eq!(rewards[0].kind, progression::RewardKind::ChargeDefused);
    assert_eq!(rewards[0].frago, Some(true));
    assert!(
        !e.active
            .as_ref()
            .unwrap()
            .reward_statistics
            .contains_key(&43)
    );
}

#[test]
fn repair_completion_selects_only_supported_subtypes_and_previous_repairing() {
    use objective_catalog::NativeClass;
    use replication::ObjectiveState as S;
    let mut e = fixture();
    for (subtype, stat) in [
        (3, 43),
        (4, 45),
        (5, 47),
        (7, 49),
        (8, 51),
        (11, 53),
        (13, 55),
        (14, 57),
        (0, 0),
        (12, 0),
    ] {
        for old in [S::Repairing, S::Deactivating, S::Activating] {
            e.objective_catalog
                .as_mut()
                .unwrap()
                .objects
                .iter_mut()
                .find(|o| o.object_index == 120)
                .unwrap()
                .reward
                .as_mut()
                .unwrap()
                .subtype = subtype;
            e.active.as_mut().unwrap().reward_statistics.clear();
            e.pending_rewards.clear();
            e.reward_order.clear();
            let player = e.active.as_ref().unwrap().reservation.player;
            let mut delta = e
                .objective_world
                .as_ref()
                .unwrap()
                .snapshot(1, 130)
                .objectives
                .into_iter()
                .find(|o| o.object_index == 120)
                .unwrap();
            delta.state_actor = replication::ObjectiveStateActor {
                state: S::Repaired,
                actor_index: player,
                change_time_ms: 130,
            };
            let entry = objective_control::NativeEntry {
                class: NativeClass::Structural,
                previous: old,
                delta,
                actor_generation: Some(1),
                reward_actor: Some(player),
                active_trigger_index: None,
            };
            e.stage_native_objective_callbacks(entry, None, &[], frago::Frame::default())
                .unwrap();
            let stats = &e.active.as_ref().unwrap().reward_statistics;
            assert_eq!(
                stats.get(&stat).copied(),
                (old == S::Repairing && stat != 0).then_some(1),
                "subtype {subtype} old {old:?}"
            );
            assert_eq!(stats.len(), usize::from(old == S::Repairing && stat != 0));
            assert_eq!(
                e.pending_reward_batch().len(),
                usize::from(old == S::Deactivating)
            );
        }
    }
}

#[test]
fn tagged_completion_credits_each_current_same_side_squad_leader() {
    let mut e = fixture();
    let source = e.active.as_ref().unwrap().reservation.player;
    let info = e
        .active
        .as_ref()
        .unwrap()
        .claimed
        .as_ref()
        .unwrap()
        .game_info
        .clone();
    for (player, squad) in [(1, 1), (2, 16)] {
        let mut character = e.active.as_ref().unwrap().reservation.character.clone();
        character.loadouts = None;
        character.account_id = Some(format!("synthetic-counter-account-{player}"));
        character.character_id = Some(u32::from(player) + 1);
        let response = e.handle(
            Request::Reserve {
                v: BRIDGE_VERSION,
                request_id: format!("counter-reserve-{player}"),
                reservation_id: format!("counter-reservation-{player}"),
                world_id: 1,
                player_id: player,
                account_id: u32::from(player) + 1,
                session_key: "aaaaaaaaaaaaaaaa".into(),
                access_key: "bbbbbbbbbbbbbbbb".into(),
                character,
            },
            Duration::ZERO,
            1,
        );
        assert!(response.ok, "{:?}", response.error);
        let catalog =
            std::sync::Arc::new(e.spawn_catalog.as_ref().unwrap().for_squad(squad).unwrap());
        let actor = e.active.as_mut().unwrap();
        spawn_progression::ready(actor);
        actor.squad_catalog = catalog;
        actor.claimed = Some(Claimed {
            wire: vec![],
            game_info: info.clone(),
            np_identity: [0; 36],
        });
        actor.life = CharacterLife::AwaitingRespawn { generation: 1 };
    }
    let effects = e
        .stage_objective_counters(
            source,
            120,
            &[43],
            true,
            &[
                tag(0, 0x0c80_0078),
                tag(1, 0x0c80_0078),
                tag(16, 0x0c80_0078),
            ],
        )
        .unwrap();
    let actor = |player| {
        e.active
            .iter()
            .chain(e.peers.values())
            .find(|a| a.reservation.player == player)
            .unwrap()
    };
    assert_eq!(actor(source).reward_statistics.get(&41), Some(&1));
    assert_eq!(actor(1).reward_statistics.get(&41), Some(&1));
    assert!(actor(2).reward_statistics.is_empty());
    assert_eq!(actor(source).reward_statistics.get(&43), Some(&1));
    assert_eq!(effects.messages.len(), 6);
}
