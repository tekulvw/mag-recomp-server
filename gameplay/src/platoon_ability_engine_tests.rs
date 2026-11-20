//! Authenticated scope and replication checks with invented ability availability.
use super::*;
use crate::{
    active_abilities::{Ability, CommandRole, Side},
    active_ability_effects::{
        BlastFalloff, EffectRules, GasBombardmentRules, GuidedArtilleryRules, PrecisionStrikeRules,
        StrafingRunRules,
    },
    active_ability_engine_tests::{activate, add_ready, configured, request},
};

pub(crate) fn platoon_leadership() -> (
    leadership_passives::LeadershipCatalog,
    leadership_runtime::RolePolicy,
) {
    let catalog = leadership_passives::LeadershipCatalog::from_json(
        &json!({
            "schema_version":1,"application_id":20231,
            "source":{"member":"scripts/character_leadership_abilities.zpr","member_sha256":"a1".repeat(32),
                "archive_sha256":"b2".repeat(32),"extractor":"synthetic","projector":"synthetic"},
            "max_separation_range":10.0,"refresh_hold_ms":100,"removal_grace_ms":200,
            "levels":(0..4).map(|level|json!({"level":level,"name":format!("synthetic-{level}"),"modifiers":if level==0 {vec![]} else {vec![json!({"kind":"kCharModReloadTime","value":0.1})]}})).collect::<Vec<_>>()
        }).to_string(),
    ).unwrap();
    let policy = leadership_runtime::RolePolicy::from_json(&json!({
        "schema_version":3,"application_id":20231,"squad_leader":"native","development_bindings":[],
        "native_election":{"source_sha256":"c3".repeat(32),"squad_level_weight":1.,"squad_mslc_weight":1.,
            "default_record":{"leadership_level":2,"nominations":3,"matches_since_last_command":0}},
        "platoon_election":{"level_weight":2.,"mslc_weight":1.}
    }).to_string()).unwrap();
    (catalog, policy)
}

#[test]
fn legacy_active_abilities_reject_platoon_election_in_both_builder_orders() {
    const ERROR: &str = "platoon_leadership_requires_complete_active_abilities";
    let (catalog, policy) = platoon_leadership();
    assert_eq!(
        configured()
            .with_leadership_passives(catalog.clone(), policy.clone())
            .err(),
        Some(ERROR)
    );

    let mut engine = configured();
    let setup = engine
        .active_abilities
        .take()
        .unwrap()
        .setup
        .as_ref()
        .clone();
    assert_eq!(
        engine
            .with_leadership_passives(catalog, policy)
            .unwrap()
            .with_active_abilities(setup)
            .err(),
        Some(ERROR)
    );
}

pub(crate) fn configured_platoon_engine() -> Engine<Vec<u8>> {
    let mut engine = configured();
    let (catalog, policy) = platoon_leadership();
    let mut setup = engine
        .active_abilities
        .take()
        .unwrap()
        .setup
        .as_ref()
        .clone();
    let falloff = BlastFalloff {
        outer_radius: 5.,
        outer_scale: 0.,
        inner_radius: 2.,
        inner_scale: 0.5,
    };
    for faction in 0..3 {
        setup.effects.push((
            Side::Attacker,
            faction,
            Ability::PrecisionStrike,
            EffectRules::PrecisionStrike(PrecisionStrikeRules { delay_ms: 12 }),
        ));
        for side in [Side::Attacker, Side::Defender] {
            for rules in [
                EffectRules::GasBombardment(GasBombardmentRules {
                    delay_ms: 12,
                    damage_delay_ms: 5,
                    duration_ms: 50,
                    interval_ms: 10,
                    num_bomblets: 3,
                    bomblet_damage: 10.,
                    bomblet: falloff,
                    gas_damage_per_second: 5.,
                    gas: falloff,
                }),
                EffectRules::GuidedArtillery(GuidedArtilleryRules {
                    delay_ms: 12,
                    damage_delay_ms: 5,
                    effective_radius: 5.,
                }),
                EffectRules::StrafingRun(StrafingRunRules {
                    delay_ms: 12,
                    damage_delay_ms: 5,
                    duration_ms: 50,
                    interval_ms: 10,
                    strafe_length: 100.,
                    deviation_radius: 10.,
                    bomblet_damage: 10.,
                    bomblet: falloff,
                }),
            ] {
                setup.effects.push((side, faction, rules.ability(), rules));
            }
        }
    }
    engine = engine
        .with_leadership_passives(catalog, policy)
        .unwrap()
        .with_active_abilities(setup)
        .unwrap();
    // Sensor serves as a small synthetic effect to isolate native scope routing;
    // shipped mode availability remains entirely external and unchanged here.
    let setup = std::sync::Arc::make_mut(&mut engine.active_abilities.as_mut().unwrap().setup);
    for mode in &mut setup.rules.modes {
        if mode.players == 256 {
            mode.platoon = vec![Ability::Sensor];
        }
    }
    engine
}

pub(crate) fn platoon_engine() -> Engine<Vec<u8>> {
    let mut engine = configured_platoon_engine();
    add_ready(&mut engine, 0);
    add_ready(&mut engine, 1);
    for actor in engine.active.iter_mut().chain(engine.peers.values_mut()) {
        actor.nominations = 3;
    }
    engine.match_started = true;
    engine.reconcile_squad_leaders().unwrap();
    assert_eq!(engine.platoon_leader_index(4), 0);
    engine
}

pub(crate) fn last_decision(engine: &Engine<Vec<u8>>) -> Value {
    std::str::from_utf8(&engine.writer)
        .unwrap()
        .lines()
        .rev()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .find(|event| event["decoded"]["type"] == "command_ability")
        .unwrap()
}

#[test]
fn platoon_command_uses_held_scope_and_distinct_unit_timer_with_shared_gate() {
    let mut engine = platoon_engine();
    let leader = engine
        .active
        .iter()
        .chain(engine.peers.values())
        .find(|a| a.reservation.player == 0)
        .unwrap();
    assert_eq!(
        engine.command_leadership_role(leader),
        Some(CommandRole::PlatoonLeader)
    );
    assert!(activate(&mut engine, request(16, Ability::Sensor), 0, 20).ok);
    assert_eq!(last_decision(&engine)["decoded"]["accepted"], true);
    assert!(activate(&mut engine, request(17, Ability::Sensor), 1, 21).ok);
    assert_eq!(
        last_decision(&engine)["gameplay_denial"],
        "command_cooldown"
    );
    // The platoon individual timer must not become the squad individual timer.
    assert!(activate(&mut engine, request(17, Ability::Sensor), 1, 31).ok);
    assert_eq!(last_decision(&engine)["decoded"]["accepted"], true);
    assert!(activate(&mut engine, request(16, Ability::Sensor), 0, 42).ok);
    assert_eq!(
        last_decision(&engine)["gameplay_denial"],
        "command_cooldown"
    );
}

#[test]
fn platoon_command_cannot_address_another_platoon() {
    let mut engine = platoon_engine();
    assert!(activate(&mut engine, request(20, Ability::Sensor), 0, 20).ok);
    assert_eq!(
        last_decision(&engine)["gameplay_denial"],
        "command_wrong_platoon"
    );
    assert!(activate(&mut engine, request(16, Ability::Sensor), 0, 20).ok);
    assert_eq!(last_decision(&engine)["decoded"]["accepted"], true);
}

#[test]
fn platoon_activation_and_leader_are_published_on_type_ten_for_late_join() {
    let mut engine = platoon_engine();
    assert!(activate(&mut engine, request(16, Ability::Sensor), 0, 20).ok);
    assert_eq!(last_decision(&engine)["decoded"]["accepted"], true);
    engine
        .queue_active_ability_state(Duration::from_millis(21), 1)
        .unwrap();
    let actor = engine
        .active
        .iter()
        .chain(engine.peers.values())
        .find(|a| a.reservation.player == 1)
        .unwrap();
    let update = actor
        .pending_shared
        .iter()
        .filter_map(|p| replication::ObjectUpdate::decode_with_profile(p, engine.wire_profile).ok())
        .find(|u| u.header.object_type_mask == 1 << 10)
        .unwrap();
    let mut bits = replication::BitReader::with_profile(update.wire, engine.wire_profile);
    bits.position = engine.wire_profile.header_bits();
    for index in 0..8 {
        assert_eq!(bits.read(1).unwrap(), 1);
        assert_eq!(bits.read(16).unwrap(), index);
        assert_eq!(bits.read(2).unwrap(), 3);
        assert_eq!(bits.read(1).unwrap(), 1);
        assert_eq!(bits.read(16).unwrap(), 0x105);
        for ability in 1..=6 {
            let present = bits.read(1).unwrap();
            if index == 4 && ability == 3 {
                assert_eq!(present, 1);
                for axis in [20.0f32, 3., 23.] {
                    assert_eq!(bits.read(32).unwrap(), axis.to_bits());
                }
                assert_eq!(bits.read(16).unwrap(), 16);
                assert_eq!(bits.read(32).unwrap(), 20);
                assert_eq!(bits.read(1).unwrap(), 1);
                assert_eq!(bits.read(32).unwrap(), u32::MAX);
            } else {
                assert_eq!(present, 0);
            }
        }
        assert_eq!(bits.read(1).unwrap(), 1);
        assert_eq!(
            bits.read(16).unwrap(),
            u32::from(if index == 4 { 0 } else { u16::MAX })
        );
        for expected in [u16::MAX, if index == 4 { 4 } else { 0 }, 4] {
            assert_eq!(bits.read(1).unwrap(), 1);
            assert_eq!(bits.read(16).unwrap(), u32::from(expected));
        }
        assert_eq!(bits.read(1).unwrap(), 1);
        assert_eq!(bits.read(8).unwrap(), 0);
    }
    assert_eq!(bits.read(1).unwrap(), 0);
}
