//! Synthetic extended objective settlement through native callbacks and durable ACKs.
use super::*;
use objective_control::{NativeClass, NativeEntry};
use progression::RewardKind as Kind;
use replication::ObjectiveState as State;

pub(crate) fn fixture(subtype: u8, category: u8) -> Engine<Vec<u8>> {
    let mut e = objective_reward_tests::acquisition_fixture();
    let rule = e
        .objective_catalog
        .as_mut()
        .unwrap()
        .objects
        .iter_mut()
        .find(|o| o.object_index == 120)
        .unwrap()
        .reward
        .as_mut()
        .unwrap();
    rule.subtype = subtype;
    rule.category = category;
    let actor = e.active.as_mut().unwrap();
    let mut value = serde_json::to_value(&actor.reservation.character).unwrap();
    value.as_object_mut().unwrap().remove("skills");
    value["extended_objective_rewards_enabled"] = json!(true);
    actor.reservation.character = serde_json::from_value(value).unwrap();
    e
}
fn entry(e: &Engine<Vec<u8>>, old: State, new: State) -> NativeEntry {
    let mut delta = e
        .objective_world
        .as_ref()
        .unwrap()
        .snapshot(1, 0)
        .objectives
        .into_iter()
        .find(|d| d.object_index == 120)
        .unwrap();
    let player = e.active.as_ref().unwrap().reservation.player;
    delta.state_actor = replication::ObjectiveStateActor {
        state: new,
        actor_index: player,
        change_time_ms: 130,
    };
    NativeEntry {
        class: NativeClass::Structural,
        previous: old,
        delta,
        actor_generation: Some(1),
        reward_actor: Some(player),
        active_trigger_index: Some(1),
        bot_plant: None,
    }
}
pub(crate) fn callback(
    e: &mut Engine<Vec<u8>>,
    old: State,
    new: State,
) -> Result<(), &'static str> {
    let entry = entry(e, old, new);
    let assignment = e.frago_assignment(e.active.as_ref().unwrap());
    e.stage_native_objective_rewards(entry, assignment, frago::Frame::default())
        .map(|_| ())
}
pub(crate) fn ack(e: &mut Engine<Vec<u8>>, r: &progression::RewardEvent) -> Response {
    let mut value = json!({"op":"progression_committed","v":BRIDGE_VERSION,"request_id":"extended-objective-ack",
        "reservation_id":r.reservation_id,"world_id":r.world_id,"player_id":r.player_id,"reward_id":r.reward_id,
        "progression":{"experience":40,"level":1,"skill_points":0},"match_experience":40});
    if r.frago == Some(true) {
        value["match_frago_points"] = json!(20);
    }
    e.handle(
        serde_json::from_value(value).unwrap(),
        Duration::from_millis(140),
        1,
    )
}
#[test]
fn bridge_recognizes_all_nine_additional_one_unit_objective_kinds() {
    for (alias, stat) in [
        ("defensive_emplacements_destroyed", 42),
        ("bridges_destroyed", 44),
        ("fortifications_destroyed", 46),
        ("gates_destroyed", 48),
        ("mortars_destroyed", 50),
        ("sensor_networks_destroyed", 52),
        ("anti_aircraft_destroyed", 54),
        ("motor_pools_destroyed", 56),
        ("escort_vehicle_delivered", 59),
    ] {
        let kind: Kind = serde_json::from_value(json!(alias)).unwrap();
        assert_eq!(kind.statistic(), Some(stat));
    }
}
#[test]
fn each_supported_subtype_pays_on_both_armed_completions_independent_of_category() {
    for (subtype, alias) in [
        (3, "defensive_emplacements_destroyed"),
        (4, "bridges_destroyed"),
        (5, "fortifications_destroyed"),
        (7, "gates_destroyed"),
        (8, "mortars_destroyed"),
        (11, "sensor_networks_destroyed"),
        (13, "anti_aircraft_destroyed"),
        (14, "motor_pools_destroyed"),
    ] {
        for old in [State::Activated, State::Deactivating] {
            let mut e = fixture(subtype, 4);
            callback(&mut e, old, State::Completed).unwrap();
            let rewards = e.pending_reward_batch();
            assert_eq!(rewards.len(), 1, "subtype {subtype}, previous {old:?}");
            assert_eq!(serde_json::to_value(rewards[0].kind).unwrap(), json!(alias));
            assert_eq!((rewards[0].units, rewards[0].frago), (1, Some(true)));
        }
    }
}
#[test]
fn subtype_reward_precedes_primary_secure_and_defuse_completion_has_no_secure_award() {
    for (old, want) in [
        (
            State::Activated,
            vec![
                "defensive_emplacements_destroyed",
                "primary_objective_secured",
            ],
        ),
        (
            State::Deactivating,
            vec!["defensive_emplacements_destroyed"],
        ),
    ] {
        let mut e = fixture(3, 0);
        callback(&mut e, old, State::Completed).unwrap();
        assert_eq!(
            e.pending_reward_batch()
                .iter()
                .map(|r| serde_json::to_value(r.kind).unwrap())
                .collect::<Vec<_>>(),
            want.iter().map(|s| json!(s)).collect::<Vec<_>>()
        );
    }
}
#[test]
fn unsupported_subtypes_and_non_armed_completions_do_not_invent_destruction_credit() {
    for subtype in [0, 1, 2, 6, 9, 10, 12, 15, 255] {
        let mut e = fixture(subtype, 4);
        callback(&mut e, State::Activated, State::Completed).unwrap();
        assert!(e.pending_rewards.is_empty());
    }
    for old in [
        State::Inactive,
        State::Deactivated,
        State::Activating,
        State::Repairing,
        State::Repaired,
        State::Completed,
    ] {
        let mut e = fixture(3, 4);
        callback(&mut e, old, State::Completed).unwrap();
        assert!(e.pending_rewards.is_empty());
    }
}
#[test]
fn destruction_ack_updates_its_counter_and_zero_xp_detonation_once() {
    let mut e = fixture(3, 4);
    callback(&mut e, State::Activated, State::Completed).unwrap();
    let r = e.pending_reward_batch()[0].clone();
    assert!(ack(&mut e, &r).ok);
    let a = e
        .peers
        .values()
        .chain(e.active.iter())
        .find(|a| a.reservation.id == r.reservation_id)
        .unwrap();
    assert_eq!(
        (a.reward_statistics.get(&42), a.reward_statistics.get(&61)),
        (Some(&1), Some(&1))
    );
    assert!(ack(&mut e, &r).ok);
    let a = e
        .peers
        .values()
        .chain(e.active.iter())
        .find(|a| a.reservation.id == r.reservation_id)
        .unwrap();
    assert_eq!(a.reward_statistics.get(&61), Some(&1));
}
#[test]
fn failed_ack_record_installs_neither_destruction_nor_detonation_counter() {
    let mut e = fixture(3, 4);
    callback(&mut e, State::Activated, State::Completed).unwrap();
    let r = e.pending_reward_batch()[0].clone();
    let limit = e.max_log;
    e.max_log = e.written;
    assert_eq!(ack(&mut e, &r).error.as_deref(), Some("log_limit_reached"));
    let a = e
        .peers
        .values()
        .chain(e.active.iter())
        .find(|a| a.reservation.id == r.reservation_id)
        .unwrap();
    assert!(!a.reward_statistics.contains_key(&42));
    assert!(!a.reward_statistics.contains_key(&61));
    e.max_log = limit;
    e.failed = false;
    assert!(ack(&mut e, &r).ok);
}

#[test]
fn new_base_admission_and_objective_frago_are_independent() {
    let mut e = fixture(3, 4);
    e.active
        .as_mut()
        .unwrap()
        .reservation
        .character
        .frago_objective_bonus_enabled = Some(false);
    callback(&mut e, State::Activated, State::Completed).unwrap();
    assert_eq!(e.pending_reward_batch().len(), 1);
    assert_eq!(e.pending_reward_batch()[0].frago, None);
    let mut old = fixture(3, 0);
    old.active
        .as_mut()
        .unwrap()
        .reservation
        .character
        .extended_objective_rewards_enabled = None;
    callback(&mut old, State::Activated, State::Completed).unwrap();
    assert_eq!(
        old.pending_reward_batch()
            .iter()
            .map(|r| r.kind)
            .collect::<Vec<_>>(),
        [Kind::PrimaryObjectiveSecured]
    );
    assert_eq!(old.pending_reward_batch()[0].frago, Some(true));
}

#[test]
fn fuse_during_another_characters_defuse_credits_only_the_planter() {
    let mut e = fixture(3, 4);
    objective_reward_tests::interact(
        &mut e,
        120,
        protocol::ObjectiveInteractionKind::Activate,
        100,
    )
    .unwrap();
    e.tick_acquisition_objectives(Duration::from_millis(110), 1)
        .unwrap();
    let mut defender = fixture(3, 4).active.take().unwrap();
    defender.reservation.id = "extended-defender".into();
    defender.reservation.player = 1;
    defender.reservation.character.account_id = Some("extended-defender-account".into());
    defender.reservation.character.character_id = Some(2);
    defender.squad_catalog =
        std::sync::Arc::new(e.spawn_catalog.as_ref().unwrap().for_squad(16).unwrap());
    let original = e.active.replace(defender).unwrap();
    let original_id = original.reservation.id.clone();
    e.peers.insert(original_id.clone(), original);
    objective_reward_tests::interact(
        &mut e,
        120,
        protocol::ObjectiveInteractionKind::Deactivate,
        120,
    )
    .unwrap();
    let limit = e.max_log;
    e.max_log = e.written;
    assert_eq!(
        e.tick_acquisition_objectives(Duration::from_millis(130), 1)
            .unwrap_err(),
        "log_limit_reached"
    );
    assert_eq!(e.pending_reward_batch().len(), 1);
    assert_eq!(
        e.objective_world.as_ref().unwrap().state(120),
        Some(State::Deactivating)
    );
    e.max_log = limit;
    e.failed = false;
    e.tick_acquisition_objectives(Duration::from_millis(130), 1)
        .unwrap();
    let rewards = e.pending_reward_batch();
    assert_eq!(rewards.len(), 2);
    assert_eq!(
        (rewards[1].kind, rewards[1].player_id),
        (Kind::DefensiveEmplacementsDestroyed, 0)
    );
    assert_eq!(rewards[1].reservation_id, original_id);
    assert!(
        !rewards
            .iter()
            .any(|r| r.kind == Kind::PrimaryObjectiveSecured)
    );
}

#[test]
fn source_classified_passive_precision_pays_only_its_final_callback() {
    let mut e = fixture(3, 4);
    let object = e
        .objective_catalog
        .as_mut()
        .unwrap()
        .objects
        .iter_mut()
        .find(|o| o.object_index == 120)
        .unwrap();
    object.native_class = None;
    object.repair = None;
    object.demolition = None;
    e.objective_world = Some(objective_control::ObjectiveWorld::new(
        e.objective_catalog.as_ref().unwrap(),
    ));
    acquisition_gameplay_engine::test_support::disable_structural_fixture(
        e.acquisition_gameplay.as_mut().unwrap(),
    );
    let source = structure_damage::Source {
        player: 0,
        generation: 1,
    };
    e.objective_world
        .as_mut()
        .unwrap()
        .stage_precision_strike_with_actor(
            120,
            Some(source),
            Some(objective_control::PrecisionRewardActor::Attacker(0)),
        )
        .unwrap();
    e.tick_acquisition_objectives(Duration::from_millis(130), 1)
        .unwrap();
    let rewards = e.pending_reward_batch();
    assert_eq!(rewards.len(), 1);
    assert_eq!(rewards[0].kind, Kind::DefensiveEmplacementsDestroyed);
}
