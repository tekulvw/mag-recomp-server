//! Synthetic native owner reports exercise falling through the real event path.
use super::*;
use crate::spawn_progression::{dispatch, engine, ready, reserve};

fn fixture(general: f32, impact: f32) -> Engine<Vec<u8>> {
    let mut engine = engine().with_fall_damage_rules(fall_damage::Rules {
        schema_version: 1,
        source_build: "retail".into(),
        application_id: 20231,
        min_height: 1.,
        damage_factor: 2.,
        minimum_damage_exclusive: 1.,
        jump_fall_state: 11,
        damage_type: 5,
        damage_source_type: 1,
    });
    reserve(&mut engine, 0);
    let actor = engine.active.as_mut().unwrap();
    ready(actor);
    actor.life = CharacterLife::Alive {
        generation: 1,
        health_quarter_points: 400,
    };
    actor.fall_tracker.reset(1, 0, 3.).unwrap();
    actor.owner_fields.state = Some(replication::CharacterState {
        state: 1,
        stance: 0,
        user_data: [0; 3],
    });
    let skills = skill_catalog::SkillCatalog::from_value(json!({
        "schema_version":2,"application_id":20231,"table":{"Skills":{
            "MagCharacterSkill0":{"MagCharacterSkill":{
                "Default":{"Modifier":"kCharModDamageTaken","ModifierValue":general},
                "Green":{"Available":true},"Black":{"Available":true},"Red":{"Available":true}}},
            "MagCharacterSkill1":{"MagCharacterSkill":{
                "Default":{"Modifier":"kCharModImpactDamageTaken","ModifierValue":impact},
                "Green":{"Available":true},"Black":{"Available":true},"Red":{"Available":true}}},
            "MagCharacterSkill2":{"MagCharacterSkill":{
                "Default":{"Modifier":"kCharModExplosiveDamageTaken","ModifierValue":-0.9},
                "Green":{"Available":true},"Black":{"Available":true},"Red":{"Available":true}}}
        }}
    }))
    .unwrap();
    actor.modifiers = skills
        .resolve(actor.reservation.character.faction, &[7, 0, 0, 0, 0])
        .unwrap();
    assert_eq!(actor.modifiers.value("kCharModDamageTaken"), general);
    assert_eq!(actor.modifiers.value("kCharModImpactDamageTaken"), impact);
    engine
}
fn report(
    engine: &mut Engine<Vec<u8>>,
    sequence: u32,
    state: u8,
    height: f32,
) -> Result<Vec<OutboundMessage>, &'static str> {
    let fields = player_tracking::ClientCharacterFields {
        state: Some(replication::CharacterState {
            state,
            stance: 0,
            user_data: [0; 3],
        }),
        position: Some(replication::CharacterPosition {
            x: 100.,
            y: height,
            z: 100.,
        }),
        ..Default::default()
    };
    let wire = replication::multiplayer::CharacterOwnerDelta {
        sequence,
        timestamp_ms: sequence * 100,
        player: 0,
        fields: &fields,
    }
    .encode_with_profile(engine.wire_profile)
    .unwrap();
    dispatch(engine, &wire, u64::from(sequence) * 100)
}
fn health(engine: &Engine<Vec<u8>>) -> u16 {
    match engine.active.as_ref().unwrap().life {
        CharacterLife::Alive {
            health_quarter_points,
            ..
        } => health_quarter_points,
        _ => 0,
    }
}
fn fall(engine: &mut Engine<Vec<u8>>) {
    report(engine, 2, 11, 14.).unwrap();
    report(engine, 3, 1, 10.).unwrap();
}
#[test]
fn falling_applies_general_and_impact_additively_once() {
    for (general, impact, expected_hp) in [
        (0., 0., 328),
        (-0.25, -0.25, 364),
        (-2., 1.5, 364),
        (0.25, 0.5, 274),
    ] {
        let mut engine = fixture(general, impact);
        fall(&mut engine);
        // Independent synthetic raw amount: (4 - 1)^2 * 2 = 18 HP.
        assert_eq!(
            health(&engine),
            expected_hp,
            "general {general}, impact {impact}"
        );
        assert_eq!(
            engine
                .active
                .as_ref()
                .unwrap()
                .last_damage
                .last_location_and_type,
            0x50
        );
        report(&mut engine, 4, 1, 10.).unwrap();
        assert_eq!(
            health(&engine),
            expected_hp,
            "same landing must not apply twice"
        );
    }
}
#[test]
fn falling_zero_scale_consumes_the_fall_without_damage_or_rewards() {
    let mut engine = fixture(-0.75, -0.75);
    fall(&mut engine);
    assert_eq!(health(&engine), 400);
    assert!(engine.active.as_ref().unwrap().grenade_damage.is_empty());
    assert!(engine.pending_reward_batch().is_empty());
    engine.active.as_mut().unwrap().modifiers = Default::default();
    report(&mut engine, 4, 1, 10.).unwrap();
    assert_eq!(health(&engine), 400);
}
#[test]
fn falling_keeps_raw_threshold_before_modifiers_and_allows_small_adjusted_hits() {
    let mut engine = fixture(-0.5, -0.25);
    report(&mut engine, 2, 11, 12.).unwrap();
    report(&mut engine, 3, 1, 10.).unwrap();
    // Raw (2 - 1)^2 * 2 = 2; adjusted0.5 survives the raw >1 gate.
    assert_eq!(health(&engine), 398);
    let mut culled = fixture(10., 10.);
    report(&mut culled, 2, 11, 11.5).unwrap();
    report(&mut culled, 3, 1, 10.).unwrap();
    assert_eq!(
        health(&culled),
        400,
        "raw0.5 must be culled before amplification"
    );
}
#[test]
fn falling_retains_large_internal_amounts_and_transactional_recording() {
    let mut lethal = fixture(-0.25, -0.25);
    report(&mut lethal, 2, 11, 50.).unwrap();
    report(&mut lethal, 3, 1, 0.).unwrap();
    assert!(matches!(
        lethal.active.as_ref().unwrap().life,
        CharacterLife::AwaitingRespawn { .. }
    ));
    let mut failed = fixture(-0.25, -0.25);
    report(&mut failed, 2, 11, 14.).unwrap();
    failed.max_log = failed.written;
    assert_eq!(
        report(&mut failed, 3, 1, 10.).err(),
        Some("log_limit_reached")
    );
    assert_eq!(health(&failed), 400);
    assert!(failed.active.as_ref().unwrap().grenade_damage.is_empty());
}

#[test]
fn queued_falling_damage_keeps_its_landing_modifier_snapshot() {
    let mut engine = fixture(-0.25, -0.25);
    report(&mut engine, 2, 11, 14.).unwrap();
    let actor = engine.active.as_mut().unwrap();
    actor.pending_damage = Some(PendingDamage {
        target_character_id: 0x0380_0000,
        target_reservation_id: Some(actor.reservation.id.clone()),
        id: "synthetic-prior-hit".into(),
        generation: 1,
        falling: false,
        damage: damage::ResolvedDamage {
            amount_hp: 1.,
            kind: damage::DamageKind::Bullet,
            kinetic_response: false,
            location: 0,
            attacker_id: 0x0380_0000,
            source_object_id: u32::MAX,
            user_data: 0,
        },
    });
    report(&mut engine, 3, 1, 10.).unwrap();
    assert_eq!(
        health(&engine),
        396,
        "prior resolved1HP hit is released first"
    );
    assert_eq!(
        engine
            .active
            .as_ref()
            .unwrap()
            .grenade_damage
            .front()
            .unwrap()
            .damage
            .amount_hp,
        9.
    );
    engine.active.as_mut().unwrap().modifiers = fixture(-0.75, -0.75).active.unwrap().modifiers;
    report(&mut engine, 4, 1, 10.).unwrap();
    assert_eq!(
        health(&engine),
        360,
        "queued9HP must survive a later zero-scale modifier"
    );
    assert!(engine.active.as_ref().unwrap().grenade_damage.is_empty());
}

#[test]
fn nonfinite_effective_falling_amount_does_not_commit_the_landing() {
    let mut engine = fixture(f32::MAX, f32::MAX);
    report(&mut engine, 2, 11, 14.).unwrap();
    assert_eq!(
        report(&mut engine, 3, 1, 10.).err(),
        Some("invalid_falling_damage")
    );
    assert_eq!(health(&engine), 400);
    assert_eq!(engine.active.as_ref().unwrap().latest_owner_sequence, 2);
    assert!(engine.active.as_ref().unwrap().grenade_damage.is_empty());
}

#[test]
fn falling_self_damage_uses_neutral_base_with_friendly_fire_rules_present() {
    let mut engine =
        crate::human_kill_reward_tests::enable_incoming_base(fixture(-0.25, -0.25), 0.25);
    fall(&mut engine);
    assert_eq!(health(&engine), 364);
}
