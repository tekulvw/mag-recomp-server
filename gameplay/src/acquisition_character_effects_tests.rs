//! Independent synthetic actor/candidate regression tests. No game data or runs.
use super::*;
use std::collections::BTreeMap;

fn target(engine: &Engine<Vec<u8>>, active: bool) -> Target {
    let actor = if active {
        engine.active.as_ref().unwrap()
    } else {
        engine.peers.values().next().unwrap()
    };
    Target {
        character_id: 0x0380_0000 | u32::from(actor.reservation.player),
        generation: 4,
        world: actor.reservation.world,
    }
}
fn native(invulnerability: u8, mounted: bool) -> NativeState {
    NativeState {
        invulnerability: Some(invulnerability),
        associated_vehicle: Some(if mounted {
            protocol::VictimWasMounted::AssociatedVehicle
        } else {
            protocol::VictimWasMounted::NoAssociatedVehicle
        }),
    }
}
fn lethal() -> Request {
    Request::VolumeLethal {
        victim_vector: [7., 9., -2.],
    }
}
fn states(t: Target, state: NativeState) -> BTreeMap<u32, NativeState> {
    BTreeMap::from([(t.character_id, state)])
}
fn actor(engine: &Engine<Vec<u8>>, t: Target) -> &Active {
    engine
        .active
        .iter()
        .chain(engine.peers.values())
        .find(|a| (0x0380_0000 | u32::from(a.reservation.player)) == t.character_id)
        .unwrap()
}
fn notice(actor: &Active) -> &[u8] {
    actor
        .pending_notifications
        .iter()
        .find_map(|p| match p {
            match_flow::Publication::Application(wire) if wire.starts_with(&[1, 255]) => {
                Some(wire.as_slice())
            }
            _ => None,
        })
        .unwrap()
}

#[test]
fn volume_death_prepares_in_isolation_restores_bool_and_installs_ordinary_self_death() {
    let mut engine = crate::human_kill_reward_tests::fixture();
    let t = target(&engine, true);
    let mut native = states(t, native(2, false));
    let before_log = engine.sequence;
    let candidate = engine
        .prepare_acquisition_character_effects(&[(t, lethal())], &native, Duration::from_secs(1))
        .unwrap();
    assert!(actor(&engine, t).life.is_alive());
    assert!(actor(&engine, t).pending_notifications.is_empty());
    assert!(actor(&engine, t).reward_statistics.is_empty());
    assert_eq!(native[&t.character_id].invulnerability, Some(2));
    assert_eq!(candidate.audit()[0]["invulnerability_during"], 0);
    assert_eq!(candidate.audit()[0]["invulnerability_after"], 1);
    assert_eq!(candidate.audit()[0]["raw_amount_hp"], 1000.0);
    assert_eq!(candidate.audit()[0]["damage_source_type"], 10);
    assert_eq!(candidate.audit()[0]["from_direction"], json!([7., 9., -2.]));
    let messages = engine
        .install_acquisition_character_effects(candidate, &mut native)
        .unwrap();
    assert_eq!(
        engine.sequence, before_log,
        "outer caller owns evidence recording"
    );
    let victim = actor(&engine, t);
    assert!(matches!(
        victim.life,
        CharacterLife::AwaitingRespawn { generation: 4 }
    ));
    assert_eq!(victim.last_damage.last_location_and_type, 0x50);
    assert_eq!(victim.last_damage.last_attacker, t.character_id);
    assert_eq!(victim.reward_statistics.get(&12), Some(&1));
    assert_eq!(victim.reward_statistics.get(&13), Some(&1));
    assert_eq!(native[&t.character_id].invulnerability, Some(1));
    assert!(!victim.pending_shared.is_empty());
    assert!(!messages.is_empty());
    assert_eq!(&notice(victim)[6..11], &[10, 255, 255, 255, 255]);
    assert_eq!(notice(victim)[11] & 0x40, 0);
    assert!(engine.pending_reward_batch().is_empty());
}

#[test]
fn failed_later_pair_and_discarded_candidates_preserve_all_authoritative_effect_fields() {
    let mut engine = crate::human_kill_reward_tests::fixture();
    let t = target(&engine, true);
    let native = states(t, native(0, false));
    let bad = Target {
        generation: 99,
        ..t
    };
    assert!(
        engine
            .prepare_acquisition_character_effects(
                &[(t, lethal()), (bad, lethal())],
                &native,
                Duration::from_secs(1)
            )
            .is_err()
    );
    assert!(actor(&engine, t).life.is_alive());
    assert!(actor(&engine, t).pending_shared.is_empty());
    assert!(actor(&engine, t).pending_notifications.is_empty());
    assert!(actor(&engine, t).reward_statistics.is_empty());
    assert!(engine.pending_reward_batch().is_empty());
    drop(
        engine
            .prepare_acquisition_character_effects(
                &[(t, lethal())],
                &native,
                Duration::from_secs(1),
            )
            .unwrap(),
    );
    assert!(actor(&engine, t).life.is_alive());
    assert!(actor(&engine, t).pending_shared.is_empty());
}

#[test]
fn collector_duplicates_see_candidate_death_without_double_kill_or_notice() {
    let mut engine = crate::human_kill_reward_tests::fixture();
    let t = target(&engine, true);
    let mut native = states(t, native(0, false));
    let candidate = engine
        .prepare_acquisition_character_effects(
            &[(t, lethal()), (t, lethal())],
            &native,
            Duration::from_secs(1),
        )
        .unwrap();
    assert_eq!(candidate.audit()[0]["outcome"], "Applied");
    assert_eq!(candidate.audit()[1]["outcome"], "DeadIgnored");
    engine
        .install_acquisition_character_effects(candidate, &mut native)
        .unwrap();
    let victim = actor(&engine, t);
    assert_eq!(victim.reward_statistics.get(&12), Some(&1));
    assert_eq!(victim.pending_notifications.len(), 1);
}

#[test]
fn occupant_protection_is_honored_and_unknown_native_inputs_fail_without_mutation() {
    let engine = crate::human_kill_reward_tests::enable_incoming_base(
        crate::human_kill_reward_tests::fixture(),
        0.25,
    );
    let mut engine = engine;
    let t = target(&engine, true);
    let request = Request::VehicleDeathOccupant {
        last_attacker: u32::MAX,
        vehicle_vector: [1., 2., 3.],
    };
    let mut state = native(9, true);
    let plan = engine
        .plan_acquisition_character_effect(t, request, state, Duration::from_secs(1))
        .unwrap();
    assert_eq!(plan.outcome(), Outcome::Protected);
    engine
        .commit_acquisition_character_effect(plan, &mut state)
        .unwrap();
    assert!(actor(&engine, t).life.is_alive());
    assert_eq!(state.invulnerability, Some(9));
    state.invulnerability = None;
    assert!(matches!(
        engine.plan_acquisition_character_effect(t, lethal(), state, Duration::from_secs(1)),
        Err("acquisition_character_invulnerability_missing")
    ));
    state = native(0, true);
    state.associated_vehicle = None;
    assert!(matches!(
        engine.plan_acquisition_character_effect(t, request, state, Duration::from_secs(1)),
        Err("acquisition_character_vehicle_association_missing")
    ));
    assert!(actor(&engine, t).life.is_alive());
}

#[test]
fn vehicle_enemy_kill_credits_real_peer_attacker_and_preserves_source_and_mounted_notice() {
    let mut engine = crate::human_kill_reward_tests::enable_incoming_base(
        crate::human_kill_reward_tests::fixture(),
        0.25,
    );
    let victim = target(&engine, true);
    let attacker = target(&engine, false);
    engine.active.as_mut().unwrap().mounted_seat = Some(3);
    let mut native = states(victim, native(0, true));
    let request = Request::VehicleDeathOccupant {
        last_attacker: attacker.character_id,
        vehicle_vector: [0., 5., 0.],
    };
    let candidate = engine
        .prepare_acquisition_character_effects(
            &[(victim, request)],
            &native,
            Duration::from_secs(1),
        )
        .unwrap();
    assert!(engine.pending_reward_batch().is_empty());
    engine
        .install_acquisition_character_effects(candidate, &mut native)
        .unwrap();
    assert_eq!(
        actor(&engine, attacker).reward_statistics.get(&10),
        Some(&1)
    );
    assert_eq!(actor(&engine, victim).reward_statistics.get(&12), Some(&1));
    assert!(actor(&engine, victim).mounted_seat.is_none());
    assert_eq!(
        actor(&engine, victim).last_damage.last_location_and_type,
        0x20
    );
    assert_eq!(
        actor(&engine, victim).last_damage.last_attacker,
        attacker.character_id
    );
    assert_eq!(engine.pending_reward_batch().len(), 1);
    let wire = notice(actor(&engine, victim));
    assert_eq!(&wire[2..4], &(attacker.character_id as u16).to_be_bytes());
    assert_eq!(&wire[6..11], &[5, 255, 255, 255, 255]);
    assert_eq!(wire[11] & 0x40, 0x40);
}

#[test]
fn compensated_occupant_damage_exceeds_command_limit_without_forcing_health() {
    let mut engine = crate::human_kill_reward_tests::enable_incoming_base(
        crate::human_kill_reward_tests::fixture(),
        0.25,
    );
    let t = target(&engine, true);
    engine.active.as_mut().unwrap().life = CharacterLife::Alive {
        generation: 4,
        health_quarter_points: 4095,
    };
    let mut state = native(0, true);
    let plan = engine
        .plan_acquisition_character_effect(
            t,
            Request::VehicleDeathOccupant {
                last_attacker: u32::MAX,
                vehicle_vector: [0.; 3],
            },
            state,
            Duration::from_secs(1),
        )
        .unwrap();
    assert_eq!(plan.audit()["raw_amount_hp"], 4000.0);
    assert_eq!(plan.audit()["attacker"], t.character_id);
    engine
        .commit_acquisition_character_effect(plan, &mut state)
        .unwrap();
    assert_eq!(actor(&engine, t).last_damage.health_quarter_points, 0);
}

#[test]
fn volume_raw1000_respects_resistance_and_can_incapacitate_then_finish() {
    let mut engine = crate::human_kill_reward_tests::fixture();
    let t = target(&engine, true);
    let skills =
        skill_catalog::SkillCatalog::from_value(json!({"schema_version":2,"application_id":20231,
        "table":{"Skills":{"MagCharacterSkill0":{"MagCharacterSkill":{
            "Default":{"Modifier":"kCharModImpactDamageTaken","ModifierValue":-0.99},
            "Green":{"Available":true},"Black":{"Available":true},"Red":{"Available":true}}}}}}))
        .unwrap();
    engine.active.as_mut().unwrap().modifiers = skills.resolve(1, &[1, 0, 0, 0, 0]).unwrap();
    let mut state = native(1, false);
    let plan = engine
        .plan_acquisition_character_effect(t, lethal(), state, Duration::from_secs(1))
        .unwrap();
    engine
        .commit_acquisition_character_effect(plan, &mut state)
        .unwrap();
    assert!(
        actor(&engine, t).life.is_alive(),
        "1000 raw isn't unconditional death"
    );
    assert!(actor(&engine, t).last_damage.health_quarter_points > 0);
    assert!(actor(&engine, t).last_damage.health_quarter_points < 120);
    // Independent threshold admits a reduced lethal hit as downed.
    engine.active.as_mut().unwrap().life = CharacterLife::Alive {
        generation: 4,
        health_quarter_points: 4,
    };
    let plan = engine
        .plan_acquisition_character_effect(t, lethal(), state, Duration::from_secs(2))
        .unwrap();
    engine
        .commit_acquisition_character_effect(plan, &mut state)
        .unwrap();
    assert!(matches!(
        actor(&engine, t).life,
        CharacterLife::Incapacitated { .. }
    ));
    assert_eq!(actor(&engine, t).reward_statistics.get(&12), Some(&1));
    engine.active.as_mut().unwrap().modifiers = Default::default();
    let plan = engine
        .plan_acquisition_character_effect(t, lethal(), state, Duration::from_secs(3))
        .unwrap();
    engine
        .commit_acquisition_character_effect(plan, &mut state)
        .unwrap();
    assert!(matches!(
        actor(&engine, t).life,
        CharacterLife::AwaitingRespawn { .. }
    ));
    assert_eq!(actor(&engine, t).reward_statistics.get(&12), Some(&1));
}

#[test]
fn bot_death_candidate_cleans_bot_state_and_rewards_source_once_without_live_mutation() {
    let mut bot_engine =
        crate::bot_registry::identity_tests::populated_engine("Synthetic Acquisition");
    let bot_id = *bot_engine.bot_registry.bots.keys().next().unwrap();
    let owned_bot = bot_engine.bot_registry.bots.remove(&bot_id).unwrap();
    let mut engine = crate::human_kill_reward_tests::enable_incoming_base(
        crate::human_kill_reward_tests::fixture(),
        0.25,
    );
    let source = target(&engine, true);
    let loadout = engine.active.as_ref().unwrap().equipped_loadout().unwrap();
    let source_team = engine.active.as_ref().unwrap().team();
    engine.bot_registry.bots.insert(bot_id, owned_bot);
    let bot = engine.bot_registry.bots.get_mut(&bot_id).unwrap();
    bot.target.loadout = loadout;
    bot.target.squad =
        replication::SquadAddress::from_index(u16::from(1 - source_team) * 16).unwrap();
    bot.health = 120;
    bot.damage.health_quarter_points = 120;
    let generation = bot.target.runtime.unwrap().generation;
    let t = Target {
        character_id: 0x0380_0000 | u32::from(bot_id),
        generation,
        world: source.world,
    };
    let mut native = states(t, native(0, false));
    let request = Request::VehicleDeathOccupant {
        last_attacker: source.character_id,
        vehicle_vector: [0.; 3],
    };
    let candidate = engine
        .prepare_acquisition_character_effects(
            &[(t, request), (t, request)],
            &native,
            Duration::from_secs(1),
        )
        .unwrap();
    assert_eq!(engine.bot_registry.bots[&bot_id].health, 120);
    assert!(engine.pending_reward_batch().is_empty());
    assert_eq!(candidate.audit()[1]["outcome"], "DeadIgnored");
    engine
        .install_acquisition_character_effects(candidate, &mut native)
        .unwrap();
    let bot = &engine.bot_registry.bots[&bot_id];
    assert_eq!(bot.health, 0);
    assert!(matches!(
        bot.target.runtime.unwrap().phase,
        development_target::BotPhase::Dead
    ));
    assert!(bot.target.pending_move.is_none());
    assert_eq!(actor(&engine, source).reward_statistics.get(&10), Some(&1));
    assert_eq!(engine.pending_reward_batch().len(), 1);
    assert_eq!(
        &notice(actor(&engine, source))[6..11],
        &[5, 255, 255, 255, 255]
    );
}

#[test]
fn world_tick_without_active_human_still_credits_the_qualified_peer_source() {
    let mut engine = crate::human_kill_reward_tests::enable_incoming_base(
        crate::human_kill_reward_tests::fixture(),
        0.25,
    );
    let victim = target(&engine, true);
    let source = target(&engine, false);
    let active = engine.active.take().unwrap();
    engine.peers.insert(active.reservation.id.clone(), active);
    let mut native = states(victim, native(0, false));
    let request = Request::VehicleDeathOccupant {
        last_attacker: source.character_id,
        vehicle_vector: [0.; 3],
    };
    let candidate = engine
        .prepare_acquisition_character_effects(
            &[(victim, request)],
            &native,
            Duration::from_secs(1),
        )
        .unwrap();
    engine
        .install_acquisition_character_effects(candidate, &mut native)
        .unwrap();
    assert_eq!(actor(&engine, source).reward_statistics.get(&10), Some(&1));
    assert_eq!(actor(&engine, victim).reward_statistics.get(&12), Some(&1));
    assert_eq!(engine.pending_reward_batch().len(), 1);
}

#[test]
fn candidates_reject_foreign_engine_stale_revision_round_and_changed_native_inputs() {
    let mut engine = crate::human_kill_reward_tests::fixture();
    let mut foreign = crate::human_kill_reward_tests::fixture();
    let t = target(&engine, true);
    let mut native = states(t, native(0, false));
    let candidate = engine
        .prepare_acquisition_character_effects(&[(t, lethal())], &native, Duration::from_secs(1))
        .unwrap();
    assert_eq!(
        foreign.check_acquisition_character_effects(&candidate, &native),
        Err("stale_acquisition_character_candidate")
    );
    assert!(
        foreign
            .install_acquisition_character_effects(candidate, &mut native)
            .is_err()
    );
    assert!(actor(&foreign, t).life.is_alive());
    let candidate = engine
        .prepare_acquisition_character_effects(&[(t, lethal())], &native, Duration::from_secs(1))
        .unwrap();
    engine
        .record(json!({"op":"synthetic_accepted_revision"}), 1)
        .unwrap();
    let revision = engine.sequence;
    assert!(
        engine
            .record_acquisition_character_effects(candidate, &mut native, 1)
            .is_err()
    );
    assert_eq!(
        engine.sequence, revision,
        "stale candidate cannot write another record"
    );
    assert!(actor(&engine, t).life.is_alive());
    let candidate = engine
        .prepare_acquisition_character_effects(&[(t, lethal())], &native, Duration::from_secs(1))
        .unwrap();
    engine.round_number += 1;
    assert_eq!(
        engine.check_acquisition_character_effects(&candidate, &native),
        Err("stale_acquisition_character_candidate")
    );
    engine.round_number -= 1;
    native.get_mut(&t.character_id).unwrap().invulnerability = Some(1);
    assert_eq!(
        engine.check_acquisition_character_effects(&candidate, &native),
        Err("stale_acquisition_character_candidate")
    );
}

#[test]
fn failed_durable_record_installs_neither_health_xp_notifications_nor_native_state() {
    let mut engine = crate::human_kill_reward_tests::enable_incoming_base(
        crate::human_kill_reward_tests::fixture(),
        0.25,
    );
    let victim = target(&engine, true);
    let source = target(&engine, false);
    let mut native = states(victim, native(0, true));
    let before_native = native.clone();
    let request = Request::VehicleDeathOccupant {
        last_attacker: source.character_id,
        vehicle_vector: [0.; 3],
    };
    let candidate = engine
        .prepare_acquisition_character_effects(
            &[(victim, request)],
            &native,
            Duration::from_secs(1),
        )
        .unwrap();
    engine.max_log = engine.written;
    assert!(matches!(
        engine.record_acquisition_character_effects(candidate, &mut native, 1),
        Err("log_limit_reached")
    ));
    assert!(actor(&engine, victim).life.is_alive());
    assert!(actor(&engine, victim).pending_notifications.is_empty());
    assert!(actor(&engine, victim).pending_shared.is_empty());
    assert!(actor(&engine, source).reward_statistics.is_empty());
    assert!(engine.pending_reward_batch().is_empty());
    assert_eq!(native, before_native);
}

#[test]
fn successful_character_transaction_records_once_before_installing_effects() {
    let mut engine = crate::human_kill_reward_tests::fixture();
    let t = target(&engine, true);
    let mut native = states(t, native(2, false));
    let candidate = engine
        .prepare_acquisition_character_effects(&[(t, lethal())], &native, Duration::from_secs(1))
        .unwrap();
    let before = engine.sequence;
    let messages = engine
        .record_acquisition_character_effects(candidate, &mut native, 7)
        .unwrap();
    assert_eq!(engine.sequence, before + 1);
    assert!(!actor(&engine, t).life.is_alive());
    assert_eq!(native[&t.character_id].invulnerability, Some(1));
    assert!(!messages.is_empty());
    let record: Value = serde_json::from_slice(
        engine
            .writer
            .split(|b| *b == b'\n')
            .rev()
            .find(|s| !s.is_empty())
            .unwrap(),
    )
    .unwrap();
    assert_eq!(record["op"], "acquisition_character_effects");
    assert_eq!(record["effects"][0]["damage_source_type"], 10);
    assert!(
        record["effects"][0]["damage_publication_hex"]
            .as_str()
            .is_some()
    );
}

#[test]
fn human_force_position_stages_field5_for_owner_and_peers_and_rolls_back_tracking_lock() {
    let mut engine = crate::human_kill_reward_tests::fixture();
    let t = target(&engine, true);
    let position = replication::CharacterPosition {
        x: 40.,
        y: 6.,
        z: 44.,
    };
    let before = serde_json::to_value(actor(&engine, t).tracking.sample).unwrap();
    let mut native = BTreeMap::new();
    drop(
        engine
            .prepare_acquisition_character_effects_with_moves(
                &[(t, position)],
                &[],
                &native,
                Duration::from_secs(1),
            )
            .unwrap(),
    );
    assert_eq!(
        serde_json::to_value(actor(&engine, t).tracking.sample).unwrap(),
        before
    );
    assert!(!actor(&engine, t).tracking.correction_pending());
    assert!(actor(&engine, t).pending_shared.is_empty());
    let candidate = engine
        .prepare_acquisition_character_effects_with_moves(
            &[(t, position)],
            &[],
            &native,
            Duration::from_secs(1),
        )
        .unwrap();
    engine
        .record_acquisition_character_effects(candidate, &mut native, 7)
        .unwrap();
    assert_eq!(
        actor(&engine, t).tracking.sample.unwrap().position,
        position
    );
    assert!(matches!(
        actor(&engine, t).tracking.sample.unwrap().origin,
        player_tracking::PositionOrigin::ServerCorrection
    ));
    assert!(actor(&engine, t).tracking.correction_pending());
    assert_eq!(actor(&engine, t).owner_fields.position, Some(position));
    for recipient in engine.active.iter().chain(engine.peers.values()) {
        let wire = recipient.pending_shared.back().unwrap();
        let update =
            replication::ObjectUpdate::decode_with_profile(wire, engine.wire_profile).unwrap();
        let decoded = player_tracking::ClientCharacterFields::decode_for_owner_with_profile(
            &update,
            t.character_id as u16,
            engine.wire_profile,
        )
        .unwrap();
        assert_eq!(decoded.position, Some(position));
        assert!(
            decoded.state.is_none(),
            "position correction must not synthesize respawn/state"
        );
    }
}

#[test]
fn interleaved_callback_observes_move_then_death_before_next_pair_and_bounds_the_batch() {
    let mut engine = crate::human_kill_reward_tests::fixture();
    let t = target(&engine, true);
    let mut native = states(t, native(0, false));
    let position = replication::CharacterPosition {
        x: 41.,
        y: 6.,
        z: 45.,
    };
    let candidate = engine
        .prepare_acquisition_character_callback(
            &native,
            Duration::from_secs(1),
            |candidate, native| {
                let moved = candidate.apply_acquisition_character_move(
                    t,
                    position,
                    Duration::from_secs(1),
                )?;
                assert_eq!(
                    actor(candidate, t).tracking.sample.unwrap().position,
                    position
                );
                let state = native.get_mut(&t.character_id).unwrap();
                let plan = candidate.plan_acquisition_character_effect(
                    t,
                    lethal(),
                    *state,
                    Duration::from_secs(1),
                )?;
                let damage = plan.audit().clone();
                let messages = candidate.commit_acquisition_character_effect(plan, state)?;
                assert!(
                    !actor(candidate, t).life.is_alive(),
                    "later world query can observe immediate death layer"
                );
                Ok((vec![moved, damage], messages))
            },
        )
        .unwrap();
    assert!(actor(&engine, t).life.is_alive());
    assert!(!actor(&engine, t).tracking.correction_pending());
    engine
        .record_acquisition_character_effects(candidate, &mut native, 7)
        .unwrap();
    assert!(!actor(&engine, t).life.is_alive());
    let too_many = vec![(t, lethal()); MAX_CHARACTER_EFFECTS + 1];
    assert!(matches!(
        engine.prepare_acquisition_character_effects(&too_many, &native, Duration::from_secs(1)),
        Err("acquisition_character_effect_limit")
    ));
}

#[test]
fn bot_force_position_updates_physical_body_and_cancels_obsolete_navigation_in_candidate() {
    let mut bot_engine =
        crate::bot_registry::identity_tests::populated_engine("Synthetic Acquisition Move");
    let bot_id = *bot_engine.bot_registry.bots.keys().next().unwrap();
    let bot = bot_engine.bot_registry.bots.remove(&bot_id).unwrap();
    let mut engine = crate::human_kill_reward_tests::fixture();
    let world = engine.active.as_ref().unwrap().reservation.world;
    engine.bot_registry.bots.insert(bot_id, bot);
    let bot = engine.bot_registry.bots.get_mut(&bot_id).unwrap();
    bot.task.operator = true;
    bot.drop_waiting = Some(3);
    bot.target.runtime.as_mut().unwrap().phase = development_target::BotPhase::Relocating;
    let generation = bot.target.runtime.unwrap().generation;
    let before = bot.target.runtime.unwrap().physical_position();
    let target = Target {
        character_id: 0x0380_0000 | u32::from(bot_id),
        generation,
        world,
    };
    let position = replication::CharacterPosition {
        x: 30.,
        y: 4.,
        z: 31.,
    };
    let mut native = BTreeMap::new();
    let candidate = engine
        .prepare_acquisition_character_effects_with_moves(
            &[(target, position)],
            &[],
            &native,
            Duration::from_secs(1),
        )
        .unwrap();
    assert_eq!(
        engine.bot_registry.bots[&bot_id]
            .target
            .runtime
            .unwrap()
            .physical_position(),
        before
    );
    assert!(engine.bot_registry.bots[&bot_id].task.operator);
    engine
        .record_acquisition_character_effects(candidate, &mut native, 7)
        .unwrap();
    let bot = &engine.bot_registry.bots[&bot_id];
    assert_eq!(bot.target.position, position);
    assert_eq!(
        bot.target.runtime.unwrap().physical_position(),
        [30., 4., 31.]
    );
    assert!(matches!(
        bot.target.runtime.unwrap().phase,
        development_target::BotPhase::Complete
    ));
    assert!(!bot.target.runtime.unwrap().physics_motion.unwrap().grounded);
    assert!(!bot.task.operator);
    assert!(bot.drop_waiting.is_none());
    assert!(bot.target.pending_move.is_none());
}
