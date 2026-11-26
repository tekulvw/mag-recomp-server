//! Synthetic engine life-cycle checks; fixtures contain no extracted tuning.
use super::*;
use crate::spawn_progression::{dispatch, engine, ready, reserve};
use std::sync::Arc;

pub(crate) fn rules() -> incapacitation::Rules {
    incapacitation::Rules {
        schema_version: 1,
        incapacitated_damage_threshold_hp: 83.0,
        incapacitated_duration_seconds: 7.0,
        excluded_character_states: vec![22],
        provenance: None,
    }
}

fn modifiers(restore: f32, health_bonus: f32) -> skill_catalog::OwnedModifiers {
    let catalog = skill_catalog::SkillCatalog::from_value(json!({
        "schema_version":2,"application_id":20231,
        "table":{"Skills":{
            "MagCharacterSkill1":{"MagCharacterSkill":{
                "Default":{"Modifier":"kCharModResuscitatePercent","ModifierValue":restore},
                "Black":{"Available":true}}},
            "MagCharacterSkill2":{"MagCharacterSkill":{
                "Default":{"Modifier":"kCharModHealth","ModifierValue":health_bonus},
                "Black":{"Available":true}}}
        }}
    }))
    .unwrap();
    catalog.resolve(1, &[6, 0, 0, 0, 0]).unwrap()
}

fn damage(attacker: u32) -> replication::CharacterDamage {
    replication::CharacterDamage {
        last_location_and_type: 0x13,
        health_quarter_points: 0,
        conditions: 0,
        last_attacker: attacker,
        last_source_object: 0x0680_0002,
        last_user_data: 19,
    }
}

fn two_actors(restore: f32) -> Engine<Vec<u8>> {
    two_actors_from_engine(
        engine().with_incapacitation_rules(rules()).unwrap(),
        restore,
    )
}

pub(crate) fn two_actors_from_engine(mut engine: Engine<Vec<u8>>, restore: f32) -> Engine<Vec<u8>> {
    reserve(&mut engine, 0);
    reserve(&mut engine, 1);
    let shared = Arc::new(engine.spawn_catalog.as_ref().unwrap().for_squad(0).unwrap());
    for actor in engine.peers.values_mut().chain(engine.active.iter_mut()) {
        ready(actor);
        actor.squad_catalog = Arc::clone(&shared);
        actor.life = CharacterLife::Alive {
            generation: 4,
            health_quarter_points: 120,
        };
        actor.owner_fields.state = Some(replication::CharacterState {
            state: 1,
            stance: 0,
            user_data: [0; 3],
        });
        actor.pending_shared.clear();
        actor.pending_spawn_deadline = None;
    }
    engine.active.as_mut().unwrap().modifiers = modifiers(restore, 0.5);
    engine
}

fn patient(engine: &Engine<Vec<u8>>) -> &Active {
    &engine.peers["synthetic-reservation-0000"]
}

fn down_patient(engine: &mut Engine<Vec<u8>>, now: Duration) {
    let (model, life) = engine
        .plan_human_damage(patient(engine), 40.0, damage(0x0380_0002), now)
        .unwrap();
    let patient = engine.peers.get_mut("synthetic-reservation-0000").unwrap();
    patient.last_damage = model;
    patient.life = life;
}

fn request(patient: u32) -> Vec<u8> {
    let mut bytes = 0x021cu16.to_be_bytes().to_vec();
    bytes.extend(patient.to_be_bytes());
    bytes
}

fn assert_downed(engine: &Engine<Vec<u8>>, deadline: Duration) {
    assert!(matches!(patient(engine).life,
        CharacterLife::Incapacitated { generation: 4, downed }
            if downed.deadline == deadline));
    assert_eq!(patient(engine).last_damage.conditions & 2, 2);
    assert_eq!(patient(engine).last_damage.health_quarter_points, 0);
}

#[test]
fn initial_lethal_hit_enters_same_generation_with_empty_separate_damage_accumulator() {
    let engine = two_actors(0.4);
    let (model, life) = engine
        .plan_human_damage(
            patient(&engine),
            40.0,
            damage(0x0380_0002),
            Duration::from_secs(3),
        )
        .unwrap();
    assert!(
        matches!(life, CharacterLife::Incapacitated {generation:4,downed}
        if downed.deadline == Duration::from_secs(10) && downed.accumulated_damage_hp == 0.0)
    );
    assert_eq!(model.health_quarter_points, 0);
    assert_eq!(model.conditions & 2, 2);
    assert_eq!(model.last_attacker, 0x0380_0002);
    assert!(patient(&engine).life.is_alive(), "planning must not commit");
}

#[test]
fn initial_threshold_equality_and_excluded_state_are_terminal() {
    let mut engine = two_actors(0.4);
    let (_, life) = engine
        .plan_human_damage(patient(&engine), 83.0, damage(0x0380_0002), Duration::ZERO)
        .unwrap();
    assert!(matches!(
        life,
        CharacterLife::AwaitingRespawn { generation: 4 }
    ));
    engine
        .peers
        .get_mut("synthetic-reservation-0000")
        .unwrap()
        .owner_fields
        .state
        .as_mut()
        .unwrap()
        .state = 22;
    let (_, life) = engine
        .plan_human_damage(patient(&engine), 40.0, damage(0x0380_0002), Duration::ZERO)
        .unwrap();
    assert!(matches!(
        life,
        CharacterLife::AwaitingRespawn { generation: 4 }
    ));
}

#[test]
fn finishing_crosses_accumulated_threshold_and_preserves_initial_attribution() {
    let mut engine = two_actors(0.4);
    down_patient(&mut engine, Duration::ZERO);
    let (model, life) = engine
        .plan_human_damage(
            patient(&engine),
            82.0,
            damage(0x0380_0003),
            Duration::from_secs(1),
        )
        .unwrap();
    assert!(
        matches!(life, CharacterLife::Incapacitated {downed,..} if downed.accumulated_damage_hp == 82.0)
    );
    let target = engine.peers.get_mut("synthetic-reservation-0000").unwrap();
    target.life = life;
    target.last_damage = model;
    let (model, life) = engine
        .plan_human_damage(
            patient(&engine),
            1.0,
            damage(0x0380_0003),
            Duration::from_secs(2),
        )
        .unwrap();
    assert!(matches!(
        life,
        CharacterLife::AwaitingRespawn { generation: 4 }
    ));
    assert_eq!(model.conditions & 2, 0);
    assert_eq!(model.last_attacker, 0x0380_0002);
    assert_eq!(model.last_source_object, 0x0680_0002);
    assert_eq!(model.last_user_data, 19);
}

#[test]
fn expiry_waits_until_deadline_then_consumes_incapacitation_once() {
    let mut engine = two_actors(0.4);
    down_patient(&mut engine, Duration::from_secs(2));
    engine
        .peers
        .get_mut("synthetic-reservation-0000")
        .unwrap()
        .last_damage
        .conditions |= 1;
    engine
        .tick_incapacitation(Duration::from_millis(8999), 1)
        .unwrap();
    assert_downed(&engine, Duration::from_secs(9));
    assert!(patient(&engine).pending_spawn_deadline.is_none());
    engine
        .tick_incapacitation(Duration::from_secs(9), 1)
        .unwrap();
    assert!(matches!(
        patient(&engine).life,
        CharacterLife::AwaitingRespawn { generation: 4 }
    ));
    assert_eq!(patient(&engine).last_damage.conditions & 2, 0);
    assert_eq!(patient(&engine).last_damage.conditions & 3, 0);
    assert_eq!(patient(&engine).last_damage.last_location_and_type >> 4, 0);
    assert_eq!(patient(&engine).last_damage.last_attacker, 0x0380_0002);
    let publications = patient(&engine).pending_shared.len();
    engine
        .tick_incapacitation(Duration::from_secs(10), 1)
        .unwrap();
    assert_eq!(patient(&engine).pending_shared.len(), publications);
}

#[test]
fn downed_patient_cannot_queue_respawn_before_expiration() {
    let mut engine = two_actors(0.4);
    down_patient(&mut engine, Duration::ZERO);
    let healer = engine.active.take().unwrap();
    engine.active = engine.peers.remove("synthetic-reservation-0000");
    engine.peers.insert(healer.reservation.id.clone(), healer);
    let _ = dispatch(&mut engine, &0x0211u16.to_be_bytes(), 1000);
    let actor = engine.active.as_ref().unwrap();
    assert!(matches!(
        actor.life,
        CharacterLife::Incapacitated { generation: 4, .. }
    ));
    assert!(actor.pending_spawn_deadline.is_none());
}

#[test]
fn authenticated_resuscitation_uses_owned_healer_maximum_and_preserves_patient_generation() {
    let mut engine = two_actors(0.4);
    down_patient(&mut engine, Duration::ZERO);
    let messages = dispatch(&mut engine, &request(0x0380_0000), 1000).unwrap();
    let notice = [
        0x01, 0xff, 0x00, 0x01, 0x00, 0x00, 0x0b, 0xff, 0xff, 0xff, 0xff, 0x00,
    ];
    let mut recipients = messages
        .iter()
        .filter(|message| protocol::decode_hex(&message.payload_hex) == notice)
        .map(|message| message.player_id)
        .collect::<Vec<_>>();
    recipients.sort_unstable();
    assert_eq!(recipients, vec![0, 1]);
    let packets = messages
        .iter()
        .map(|message| protocol::decode_hex(&message.payload_hex))
        .collect::<Vec<_>>();
    let packet = packets
        .iter()
        .find(|packet| {
            replication::ObjectUpdate::decode_with_profile(packet, engine.wire_profile).is_ok()
        })
        .unwrap();
    let update =
        replication::ObjectUpdate::decode_with_profile(packet, engine.wire_profile).unwrap();
    assert_eq!(update.header.object_type_mask, 1 << 3);
    let mut bits = replication::BitReader::with_profile(packet, engine.wire_profile);
    bits.position = engine.wire_profile.header_bits();
    assert_eq!(bits.read(1), Ok(1));
    assert_eq!(bits.read(16), Ok(0));
    for field in 0..22 {
        assert_eq!(
            bits.read(1),
            Ok(u32::from(field == 8)),
            "unexpected revived field {field}"
        );
        if field == 8 {
            assert_eq!(bits.read(8), Ok(0x73));
            assert_eq!(bits.read(16), Ok(240));
            assert_eq!(bits.read(32), Ok(0x0380_0002));
            assert_eq!(bits.read(32), Ok(0x0680_0002));
            assert_eq!(bits.read(32), Ok(19));
        }
    }
    assert_eq!(bits.read(1), Ok(0));
    assert!(matches!(
        patient(&engine).life,
        CharacterLife::Alive {
            generation: 4,
            health_quarter_points: 240
        }
    ));
    assert_eq!(patient(&engine).last_damage.health_quarter_points, 240);
    assert_eq!(patient(&engine).last_damage.conditions & 2, 0);
    assert_eq!(patient(&engine).last_damage.last_location_and_type >> 4, 7);
    assert_eq!(patient(&engine).last_damage.last_attacker, 0x0380_0002);
    assert_eq!(patient(&engine).last_damage.last_source_object, 0x0680_0002);
    assert_eq!(patient(&engine).last_damage.last_user_data, 19);
    assert!(patient(&engine).pending_spawn_deadline.is_none());
    assert!(patient(&engine).spawn_delta.is_none());
    assert_eq!(engine.active.as_ref().unwrap().reservation.player, 1);
    let before = patient(&engine).pending_shared.len();
    let _ = dispatch(&mut engine, &request(0x0380_0000), 1001);
    assert!(matches!(
        patient(&engine).life,
        CharacterLife::Alive {
            generation: 4,
            health_quarter_points: 240
        }
    ));
    assert_eq!(patient(&engine).pending_shared.len(), before);
}

#[test]
fn resuscitation_caps_restored_source_health_to_patient_maximum() {
    let mut engine = two_actors(0.8);
    down_patient(&mut engine, Duration::ZERO);
    dispatch(&mut engine, &request(0x0380_0000), 1000).unwrap();
    assert!(matches!(
        patient(&engine).life,
        CharacterLife::Alive {
            generation: 4,
            health_quarter_points: 400
        }
    ));
}

#[test]
fn wrong_team_dead_unskilled_source_or_expired_patient_cannot_revive() {
    for denial in 0..4 {
        let mut engine = two_actors(0.4);
        down_patient(&mut engine, Duration::ZERO);
        match denial {
            0 => {
                engine.active.as_mut().unwrap().squad_catalog = Arc::new(
                    engine
                        .spawn_catalog
                        .as_ref()
                        .unwrap()
                        .for_squad(16)
                        .unwrap(),
                )
            }
            1 => {
                engine.active.as_mut().unwrap().life =
                    CharacterLife::AwaitingRespawn { generation: 4 }
            }
            2 => {
                engine.active.as_mut().unwrap().modifiers = skill_catalog::OwnedModifiers::default()
            }
            _ => (),
        }
        let _ = dispatch(
            &mut engine,
            &request(0x0380_0000),
            if denial == 3 { 7000 } else { 1000 },
        );
        assert_downed(&engine, Duration::from_secs(7));
        assert!(patient(&engine).pending_shared.is_empty());
    }
}

#[test]
fn malformed_patient_handle_and_appended_healer_cannot_choose_source_identity() {
    for bytes in [request(0x0480_0000), request(0x0380_ffff), {
        let mut bytes = request(0x0380_0000);
        bytes.extend(0x0380_0002u32.to_be_bytes());
        bytes
    }] {
        let mut engine = two_actors(0.4);
        down_patient(&mut engine, Duration::ZERO);
        let _ = dispatch(&mut engine, &bytes, 1000);
        assert_downed(&engine, Duration::from_secs(7));
        assert!(patient(&engine).pending_shared.is_empty());
    }
}

#[test]
fn failed_revive_audit_does_not_commit_patient_or_replication_changes() {
    let mut engine = two_actors(0.4);
    down_patient(&mut engine, Duration::ZERO);
    engine.max_log = engine.written;
    assert_eq!(
        dispatch(&mut engine, &request(0x0380_0000), 1000).err(),
        Some("log_limit_reached")
    );
    assert_downed(&engine, Duration::from_secs(7));
    assert!(patient(&engine).pending_shared.is_empty());
    assert!(patient(&engine).spawn_delta.is_none());
    assert_eq!(patient(&engine).last_damage.last_attacker, 0x0380_0002);
}

fn dispatch_pending_damage(engine: &mut Engine<Vec<u8>>, amount_hp: f32, now_ms: u64) {
    let source = engine.active.as_mut().unwrap();
    source.pending_damage = Some(PendingDamage {
        target_character_id: 0x0380_0000,
        target_reservation_id: Some("synthetic-reservation-0000".into()),
        id: format!("synthetic-peer-hit-{now_ms}"),
        generation: 4,
        damage: damage::ResolvedDamage {
            amount_hp,
            kind: damage::DamageKind::Bullet,
            kinetic_response: false,
            location: 3,
            attacker_id: 0x0380_0001,
            source_object_id: 0x0680_0001,
            user_data: now_ms as u32,
        },
        source: PendingDamageSource::Other,
    });
    let header = replication::UpdateHeader {
        sequence: source.latest_owner_sequence + 1,
        nak_sequence: 0,
        client_index: 0,
        object_type_mask: 0,
        timestamp_ms: now_ms as u32,
    };
    let bytes = replication::ObjectUpdate { header, wire: &[] }
        .encode_empty_with_profile(engine.wire_profile)
        .unwrap();
    dispatch(engine, &bytes, now_ms).unwrap();
}

#[test]
fn owner_update_commits_peer_downed_and_finishing_lives_through_normal_event_transaction() {
    let mut engine = two_actors(0.4);
    dispatch_pending_damage(&mut engine, 40.0, 1000);
    assert_downed(&engine, Duration::from_secs(8));
    assert_eq!(patient(&engine).last_damage.last_user_data, 1000);
    dispatch_pending_damage(&mut engine, 82.0, 2000);
    assert!(matches!(patient(&engine).life,
        CharacterLife::Incapacitated {generation:4,downed}
        if downed.accumulated_damage_hp == 82.0));
    dispatch_pending_damage(&mut engine, 1.0, 3000);
    assert!(matches!(
        patient(&engine).life,
        CharacterLife::AwaitingRespawn { generation: 4 }
    ));
    assert_eq!(patient(&engine).last_damage.conditions & 2, 0);
    assert_eq!(patient(&engine).last_damage.last_attacker, 0x0380_0001);
    assert_eq!(patient(&engine).last_damage.last_source_object, 0x0680_0001);
    assert_eq!(patient(&engine).last_damage.last_user_data, 1000);
}

#[test]
fn failed_expiry_audit_leaves_the_downed_life_and_pending_publications_unchanged() {
    let mut engine = two_actors(0.4);
    down_patient(&mut engine, Duration::ZERO);
    engine.max_log = engine.written;
    assert_eq!(
        engine.tick_incapacitation(Duration::from_secs(7), 1).err(),
        Some("log_limit_reached")
    );
    assert_downed(&engine, Duration::from_secs(7));
    assert!(patient(&engine).pending_shared.is_empty());
}

#[test]
fn downed_owner_gives_up_once_without_a_second_death_and_cannot_be_revived_after() {
    let mut engine = two_actors(0.4);
    dispatch_pending_damage(&mut engine, 40.0, 1000);
    assert!(matches!(
        patient(&engine).life,
        CharacterLife::Incapacitated { generation: 4, .. }
    ));
    assert_eq!(patient(&engine).reward_statistics.get(&12), Some(&1));
    // The patient owns the request; its healer becomes an ordinary peer.
    let healer = engine.active.take().unwrap();
    engine.active = engine.peers.remove("synthetic-reservation-0000");
    engine.peers.insert(healer.reservation.id.clone(), healer);
    let mut give_up = 0x0215u16.to_be_bytes().to_vec();
    give_up.extend(0x0380_0000u32.to_be_bytes());
    dispatch(&mut engine, &give_up, 2000).unwrap();
    let actor = engine.active.as_ref().unwrap();
    assert!(matches!(
        actor.life,
        CharacterLife::AwaitingRespawn { generation: 4 }
    ));
    assert_eq!(actor.last_damage.health_quarter_points, 0);
    assert_eq!(actor.last_damage.conditions & 2, 0);
    assert_eq!(actor.reward_statistics.get(&12), Some(&1));
    // Back to the healer: a give-up cannot be undone by a late revive.
    let patient_actor = engine.active.take().unwrap();
    let healer_id = engine.peers.keys().next().unwrap().clone();
    engine.active = engine.peers.remove(&healer_id);
    engine
        .peers
        .insert(patient_actor.reservation.id.clone(), patient_actor);
    let _ = dispatch(&mut engine, &request(0x0380_0000), 3000);
    assert!(matches!(
        patient(&engine).life,
        CharacterLife::AwaitingRespawn { generation: 4 }
    ));
}

#[test]
fn initial_collapse_death_counter_survives_finish_and_counts_a_new_collapse_after_revive() {
    let mut finished = two_actors(0.4);
    dispatch_pending_damage(&mut finished, 40.0, 1000);
    assert_eq!(patient(&finished).reward_statistics.get(&12), Some(&1));
    dispatch_pending_damage(&mut finished, 83.0, 2000);
    assert_eq!(patient(&finished).reward_statistics.get(&12), Some(&1));

    let mut revived = two_actors(0.4);
    dispatch_pending_damage(&mut revived, 40.0, 1000);
    assert_eq!(patient(&revived).reward_statistics.get(&12), Some(&1));
    dispatch(&mut revived, &request(0x0380_0000), 2000).unwrap();
    assert_eq!(patient(&revived).reward_statistics.get(&12), Some(&1));
    dispatch_pending_damage(&mut revived, 70.0, 3000);
    assert!(matches!(
        patient(&revived).life,
        CharacterLife::Incapacitated { generation: 4, .. }
    ));
    assert_eq!(patient(&revived).reward_statistics.get(&12), Some(&2));
    dispatch_pending_damage(&mut revived, 83.0, 4000);
    assert_eq!(patient(&revived).reward_statistics.get(&12), Some(&2));
}

#[test]
fn firearm_assists_discard_credit_from_another_damage_path_before_same_life_revive() {
    use firearm_assists::{Fighter, Plan, Rules};

    let mut engine = two_actors(0.4)
        .with_firearm_assist_rules(Rules {
            schema_version: 1,
            application_id: 20231,
            assist_timeout_ms: 8000,
        })
        .unwrap();
    engine.match_started = true;
    engine.maximum_players = 4;
    reserve(&mut engine, 2);
    let opposing = engine
        .spawn_catalog
        .as_ref()
        .unwrap()
        .for_squad(16)
        .unwrap();
    let mut contributor = engine.active.take().unwrap();
    ready(&mut contributor);
    contributor.squad_catalog = Arc::new(opposing);
    contributor.life = CharacterLife::Alive {
        generation: 4,
        health_quarter_points: 400,
    };
    let contributor_id = contributor.reservation.id.clone();
    engine.peers.insert(contributor_id.clone(), contributor);
    engine.active = engine.peers.remove("synthetic-reservation-0001");
    let contribution = Plan::prepare(
        &engine,
        1,
        Duration::from_millis(1),
        Fighter::human(&engine.peers[&contributor_id]),
        Fighter::human(patient(&engine)),
        false,
    )
    .unwrap();
    contribution.install(&mut engine).unwrap();
    let preview = |engine: &Engine<Vec<u8>>| {
        Plan::prepare(
            engine,
            1,
            Duration::from_millis(2001),
            Fighter::human(engine.active.as_ref().unwrap()),
            Fighter::human(patient(engine)),
            true,
        )
        .unwrap()
        .audit()
    };
    assert_eq!(preview(&engine).len(), 1);

    // A queued hit uses the normal damage transaction without entering the
    // ordinary-firearm contribution route. The original victim life is revived.
    dispatch_pending_damage(&mut engine, 40.0, 1000);
    assert_downed(&engine, Duration::from_secs(8));
    dispatch(&mut engine, &request(0x03800000), 2000).unwrap();
    assert!(matches!(
        patient(&engine).life,
        CharacterLife::Alive { generation: 4, .. }
    ));
    assert!(preview(&engine).is_empty());
    assert!(
        !engine.peers[&contributor_id]
            .reward_statistics
            .contains_key(&11)
    );

    let stale = Plan::prepare(
        &engine,
        1,
        Duration::from_millis(2002),
        Fighter::human(&engine.peers[&contributor_id]),
        Fighter::human(patient(&engine)),
        false,
    )
    .unwrap();
    dispatch_pending_damage(&mut engine, 70.0, 3000);
    dispatch(&mut engine, &request(0x03800000), 4000).unwrap();
    assert!(matches!(
        patient(&engine).life,
        CharacterLife::Alive { generation: 4, .. }
    ));
    assert_eq!(stale.install(&mut engine), Err("stale_firearm_assist_plan"));
}

#[test]
fn duration_admission_uses_inherited_leadership_snapshot_without_mutating_owned_modifiers() {
    let catalog = leadership_passives::LeadershipCatalog::from_json(&json!({
        "schema_version":1,"application_id":20231,
        "source":{"member":"scripts/character_leadership_abilities.zpr",
            "member_sha256":"a".repeat(64),"archive_sha256":"b".repeat(64),
            "extractor":"synthetic","projector":"synthetic"},
        "max_separation_range":15.0,"refresh_hold_ms":400,"removal_grace_ms":200,
        "levels":[
            {"level":0,"name":"none","modifiers":[]},
            {"level":1,"name":"synthetic squad","modifiers":[{"kind":"kCharModIncapacitatedTime","value":0.5}]},
            {"level":2,"name":"synthetic platoon","modifiers":[{"kind":"kCharModReloadTime","value":0.1}]},
            {"level":3,"name":"synthetic officer","modifiers":[{"kind":"kCharModAutoHeal","value":0.1}]}
        ]
    }).to_string()).unwrap();
    let policy = leadership_runtime::RolePolicy::from_json(
        &json!({
            "schema_version":1,"application_id":20231,
            "first_member_squad_leader":true,"development_bindings":[]
        })
        .to_string(),
    )
    .unwrap();
    let configured = engine()
        .with_incapacitation_rules(rules())
        .unwrap()
        .with_leadership_passives(catalog, policy)
        .unwrap();
    let engine = two_actors_from_engine(configured, 0.4);
    let victim = engine.active.as_ref().unwrap(); // Member1 inherits from member0.
    assert_eq!(victim.reservation.player, 1);
    assert_eq!(victim.modifiers.value("kCharModIncapacitatedTime"), 0.0);
    let (_, life) = engine
        .plan_human_damage(victim, 40.0, damage(0x0380_0002), Duration::from_secs(3))
        .unwrap();
    assert!(
        matches!(life, CharacterLife::Incapacitated {generation:4,downed}
        if downed.deadline == Duration::from_millis(13_500))
    );
    assert_eq!(victim.modifiers.value("kCharModIncapacitatedTime"), 0.0);
}

#[test]
fn progression_receipts_do_not_increment_committed_finishing_or_resuscitation_counters_again() {
    for (kind, statistic) in [
        (progression::RewardKind::EnemyFinished, 24),
        (progression::RewardKind::TeammateResuscitated, 34),
    ] {
        let mut engine = two_actors(0.4);
        engine
            .active
            .as_mut()
            .unwrap()
            .reservation
            .character
            .account_id = Some("synthetic-reward-account".into());
        if kind == progression::RewardKind::EnemyFinished {
            engine.active.as_mut().unwrap().squad_catalog = Arc::new(
                engine
                    .spawn_catalog
                    .as_ref()
                    .unwrap()
                    .for_squad(16)
                    .unwrap(),
            );
        }
        dispatch_pending_damage(&mut engine, 40.0, 1000);
        if kind == progression::RewardKind::EnemyFinished {
            dispatch_pending_damage(&mut engine, 83.0, 2000);
        } else {
            // Exercise earned revival credit: this damage came from outside
            // the healer's actual team, not the synthetic damage sender above.
            engine
                .peers
                .get_mut("synthetic-reservation-0000")
                .unwrap()
                .last_damage
                .last_attacker = 0x0380_0002;
            dispatch(&mut engine, &request(0x0380_0000), 2000).unwrap();
        }
        assert_eq!(
            engine
                .active
                .as_ref()
                .unwrap()
                .reward_statistics
                .get(&statistic),
            Some(&1)
        );
        let receipt = engine
            .pending_reward_batch()
            .into_iter()
            .find(|event| event.kind == kind)
            .unwrap();
        for attempt in 0..2 {
            let response = engine.handle(
                Request::ProgressionCommitted {
                    v: BRIDGE_VERSION,
                    request_id: format!("synthetic-xp-ack-{attempt}"),
                    reservation_id: receipt.reservation_id.clone(),
                    world_id: receipt.world_id,
                    player_id: receipt.player_id,
                    reward_id: receipt.reward_id.clone(),
                    progression: progression::ProgressionSnapshot {
                        experience: 47,
                        level: 1,
                        skill_points: 0,
                    },
                    match_experience: 47,
                    leadership: None,
                    match_leadership_points: None,
                    match_frago_points: None,
                },
                Duration::from_secs(3),
                1,
            );
            assert!(response.ok, "{:?}", response.error);
            assert_eq!(
                engine
                    .active
                    .as_ref()
                    .unwrap()
                    .reward_statistics
                    .get(&statistic),
                Some(&1)
            );
            assert_eq!(patient(&engine).reward_statistics.get(&12), Some(&1));
        }
    }
}

fn full_actor_event(
    engine: &mut Engine<Vec<u8>>,
    player: u16,
    bytes: &[u8],
    now_ms: u64,
) -> Response {
    engine.handle(
        Request::Event {
            v: BRIDGE_VERSION,
            request_id: format!("synthetic-owner-{player}-{now_ms}"),
            reservation_id: format!("synthetic-reservation-{player:04}"),
            world_id: 1,
            player_id: player,
            transport: Transport::Udp,
            direction: Direction::ClientToServer,
            rt_type: 0x3d,
            plugin_id: Some(1),
            payload_hex: protocol::encode_hex(bytes),
        },
        Duration::from_millis(now_ms),
        1,
    )
}

fn actor(engine: &Engine<Vec<u8>>, player: u16) -> &Active {
    engine
        .active
        .iter()
        .chain(engine.peers.values())
        .find(|actor| actor.reservation.player == player)
        .unwrap()
}

fn owner_packet(
    engine: &Engine<Vec<u8>>,
    sequence: u32,
    fields: &player_tracking::ClientCharacterFields,
) -> Vec<u8> {
    replication::multiplayer::CharacterOwnerDelta {
        sequence,
        timestamp_ms: sequence * 100,
        player: 0,
        fields,
    }
    .encode_with_profile(engine.wire_profile)
    .unwrap()
}

fn relayed_patient_fields(
    engine: &Engine<Vec<u8>>,
    response: &Response,
) -> Vec<player_tracking::ClientCharacterFields> {
    response
        .messages
        .iter()
        .filter(|message| message.player_id == 1)
        .filter_map(|message| {
            let bytes = protocol::decode_hex(&message.payload_hex);
            let update =
                replication::ObjectUpdate::decode_with_profile(&bytes, engine.wire_profile).ok()?;
            if update.header.object_type_mask != 1 << 3 {
                return None;
            }
            player_tracking::ClientCharacterFields::decode_for_owner_with_profile(
                &update,
                0,
                engine.wire_profile,
            )
            .ok()
        })
        .collect()
}

#[test]
fn full_owner_route_relays_downed_presentation_without_authority_then_filters_stale_revive_state() {
    let mut engine = two_actors(0.4);
    down_patient(&mut engine, Duration::ZERO);
    let old_position = actor(&engine, 0).tracking.sample.unwrap().position;
    let old_ammo = actor(&engine, 0)
        .observed_primary
        .map(|item| item.ammo_count);
    let downed_report = player_tracking::ClientCharacterFields {
        state: Some(replication::CharacterState {
            state: 7,
            stance: 2,
            user_data: [4, 5, 6],
        }),
        position: Some(replication::CharacterPosition {
            x: 130.0,
            y: 9.0,
            z: 140.0,
        }),
        pitch: Some(41),
        yaw: Some(321),
        primary: Some(replication::InventoryItem {
            inventory_update_id: 1,
            gear_slot: 0,
            ammo_pool: 0,
            ammo_count: 99,
            reloading: false,
        }),
        primary_equipped: Some(false),
        ..Default::default()
    };
    let packet = owner_packet(&engine, 2, &downed_report);
    let response = full_actor_event(&mut engine, 0, &packet, 1000);
    assert!(response.ok, "{:?}", response.error);
    let relayed = relayed_patient_fields(&engine, &response);
    assert_eq!(relayed.len(), 1);
    let fields = &relayed[0];
    assert_eq!(fields.state.unwrap().state, 7);
    assert_eq!(fields.state.unwrap().stance, 2);
    assert_eq!(fields.state.unwrap().user_data, [4, 5, 6]);
    assert_eq!(fields.pitch, Some(41));
    assert_eq!(fields.yaw, Some(321));
    assert!(fields.position.is_none());
    assert!(fields.primary.is_none());
    assert!(fields.primary_equipped.is_none());
    let patient = actor(&engine, 0);
    assert!(matches!(
        patient.life,
        CharacterLife::Incapacitated { generation: 4, .. }
    ));
    let position = patient.tracking.sample.unwrap().position;
    assert_eq!(
        (position.x, position.y, position.z),
        (old_position.x, old_position.y, old_position.z)
    );
    assert!(patient.owner_fields.position.is_none());
    assert_eq!(
        patient.observed_primary.map(|item| item.ammo_count),
        old_ammo
    );
    assert_eq!(patient.owner_fields.state.unwrap().user_data, [4, 5, 6]);

    let response = full_actor_event(&mut engine, 1, &request(0x0380_0000), 2000);
    assert!(response.ok, "{:?}", response.error);
    assert!(actor(&engine, 0).revive_state_pending);
    assert!(matches!(
        actor(&engine, 0).life,
        CharacterLife::Alive { generation: 4, .. }
    ));
    let stale = player_tracking::ClientCharacterFields {
        state: Some(replication::CharacterState {
            state: 7,
            stance: 0,
            user_data: [9, 9, 9],
        }),
        ..Default::default()
    };
    let packet = owner_packet(&engine, 3, &stale);
    let response = full_actor_event(&mut engine, 0, &packet, 2500);
    assert!(response.ok, "{:?}", response.error);
    assert!(
        relayed_patient_fields(&engine, &response)
            .iter()
            .all(|fields| fields.state.is_none())
    );
    assert!(actor(&engine, 0).revive_state_pending);
    assert_eq!(
        actor(&engine, 0).owner_fields.state.unwrap().user_data,
        [4, 5, 6]
    );

    let recovery = player_tracking::ClientCharacterFields {
        state: Some(replication::CharacterState {
            state: 31,
            stance: 0,
            user_data: [0; 3],
        }),
        ..Default::default()
    };
    let packet = owner_packet(&engine, 4, &recovery);
    let response = full_actor_event(&mut engine, 0, &packet, 3000);
    assert!(response.ok, "{:?}", response.error);
    assert!(
        relayed_patient_fields(&engine, &response)
            .iter()
            .any(|fields| fields.state.is_some_and(|state| state.state == 31))
    );
    assert!(!actor(&engine, 0).revive_state_pending);
    assert_eq!(actor(&engine, 0).owner_fields.state.unwrap().state, 31);

    let basic = player_tracking::ClientCharacterFields {
        state: Some(replication::CharacterState {
            state: 1,
            stance: 0,
            user_data: [0; 3],
        }),
        ..Default::default()
    };
    let packet = owner_packet(&engine, 5, &basic);
    let response = full_actor_event(&mut engine, 0, &packet, 3500);
    assert!(response.ok, "{:?}", response.error);
    assert!(
        relayed_patient_fields(&engine, &response)
            .iter()
            .any(|fields| fields.state.is_some_and(|state| state.state == 1))
    );
    assert_eq!(actor(&engine, 0).owner_fields.state.unwrap().state, 1);
}

#[test]
fn revive_accepts_both_authenticated_transports_but_never_server_originated_requests() {
    for transport in [Transport::Tcp, Transport::Udp] {
        let mut engine = two_actors(0.4);
        down_patient(&mut engine, Duration::ZERO);
        let bytes = request(0x0380_0000);
        let input = events::EventInput {
            reservation_id: "synthetic-reservation-0001".into(),
            world_id: 1,
            player_id: 1,
            transport,
            direction: Direction::ClientToServer,
            rt_type: 0x3d,
            plugin_id: Some(1),
            payload_hex: protocol::encode_hex(&bytes),
        };
        let decoded = DecodedEvent::with_profile(&bytes, 1, engine.wire_profile, false);
        engine
            .handle_event(input, &decoded, Duration::from_secs(1), 1)
            .unwrap();
        assert!(
            patient(&engine).life.is_alive(),
            "authenticated transport {transport:?} was denied"
        );
    }
    let invalid_direction = json!({"v":BRIDGE_VERSION,"op":"event","request_id":"invalid-direction",
        "reservation_id":"synthetic-reservation-0001","world_id":1,"player_id":1,
        "transport":"tcp","direction":"server_to_client","rt_type":61,"plugin_id":1,
        "payload_hex":"021c03800000"});
    assert!(parse_request(invalid_direction.to_string().as_bytes()).is_err());
}

#[test]
fn friendly_damage_revive_restores_health_and_notice_without_earned_credit() {
    let mut engine = two_actors(0.4);
    engine
        .active
        .as_mut()
        .unwrap()
        .reservation
        .character
        .account_id = Some("synthetic-healer".into());
    down_patient(&mut engine, Duration::ZERO);
    engine
        .peers
        .get_mut("synthetic-reservation-0000")
        .unwrap()
        .last_damage
        .last_attacker = 0x0380_0001;
    let messages = dispatch(&mut engine, &request(0x0380_0000), 1000).unwrap();
    assert!(matches!(
        patient(&engine).life,
        CharacterLife::Alive {
            health_quarter_points: 240,
            ..
        }
    ));
    assert!(
        messages
            .iter()
            .any(|m| protocol::decode_hex(&m.payload_hex).starts_with(&[0x01, 0xff]))
    );
    assert!(engine.pending_reward_batch().is_empty());
    assert!(
        !engine
            .active
            .as_ref()
            .unwrap()
            .reward_statistics
            .contains_key(&34)
    );
    assert_eq!(patient(&engine).last_damage.last_attacker, 0x0380_0001);
}

#[test]
fn a_bled_out_patient_rejoins_the_next_wave_at_the_point_chosen_while_downed() {
    use crate::replication::{ObjectUpdate, UpdateHeader};
    let mut engine = two_actors(0.4);
    down_patient(&mut engine, Duration::ZERO);
    let healer = engine.active.take().unwrap();
    engine.active = engine.peers.remove("synthetic-reservation-0000");
    engine.peers.insert(healer.reservation.id.clone(), healer);
    // The map is open while downed: the choice is stored without queueing.
    // (A confirmation would give up the downed life; see the tests below.)
    let mut select = 0x0219u16.to_be_bytes().to_vec();
    select.extend(1u16.to_be_bytes());
    dispatch(&mut engine, &select, 1000).unwrap();
    let actor = engine.active.as_ref().unwrap();
    assert_eq!(actor.selected_spawn.unwrap().spawn_point_index, 1);
    assert!(actor.pending_spawn_deadline.is_none());
    engine
        .tick_incapacitation(Duration::from_secs(7), 1)
        .unwrap();
    assert!(matches!(
        engine.active.as_ref().unwrap().life,
        CharacterLife::AwaitingRespawn { generation: 4 }
    ));
    // The client sends nothing more; its ordinary updates carry it through.
    let update = |engine: &mut Engine<Vec<u8>>, now_ms: u64| {
        let header = UpdateHeader {
            sequence: engine.active.as_ref().unwrap().latest_owner_sequence + 1,
            nak_sequence: 0,
            client_index: 0,
            object_type_mask: 0,
            timestamp_ms: now_ms as u32,
        };
        let bytes = ObjectUpdate { header, wire: &[] }
            .encode_empty_with_profile(engine.wire_profile)
            .unwrap();
        dispatch(engine, &bytes, now_ms).unwrap();
    };
    update(&mut engine, 7050);
    let deadline = engine
        .active
        .as_ref()
        .unwrap()
        .pending_spawn_deadline
        .expect("bled-out character rejoins the queue");
    update(&mut engine, deadline.as_millis() as u64);
    let actor = engine.active.as_ref().unwrap();
    assert!(matches!(
        actor.life,
        CharacterLife::Alive { generation: 5, .. }
    ));
    let stored = actor.tracking.sample.unwrap().position;
    assert_eq!([stored.x, stored.y, stored.z], [120., 3., 100.]);
}

#[test]
fn a_bleed_out_just_before_a_wave_spawns_on_that_wave() {
    use crate::replication::{ObjectUpdate, UpdateHeader};
    let mut engine = two_actors(0.4);
    engine.enable_world_ticks();
    engine.match_started = true;
    down_patient(&mut engine, Duration::ZERO);
    // The patient lives on a running team clock whose wave falls 100 ms
    // after the 7 s bleed-out, well inside a dead client's update interval.
    let team = usize::from(patient(&engine).team());
    engine.team_waves[team] = Some(TeamWaveClock {
        first_deadline: Duration::from_millis(7100),
        period: Duration::from_secs(20),
    });
    let response = engine.tick_world(Duration::from_secs(7), 1, "synthetic-tick".into());
    assert!(response.ok, "{:?}", response.error);
    let queued = patient(&engine);
    assert!(matches!(
        queued.life,
        CharacterLife::AwaitingRespawn { generation: 4 }
    ));
    assert_eq!(
        queued.pending_spawn_deadline,
        Some(Duration::from_millis(7100))
    );
    let healer = engine.active.take().unwrap();
    engine.active = engine.peers.remove("synthetic-reservation-0000");
    engine.peers.insert(healer.reservation.id.clone(), healer);
    let header = UpdateHeader {
        sequence: engine.active.as_ref().unwrap().latest_owner_sequence + 1,
        nak_sequence: 0,
        client_index: 0,
        object_type_mask: 0,
        timestamp_ms: 7800,
    };
    let bytes = ObjectUpdate { header, wire: &[] }
        .encode_empty_with_profile(engine.wire_profile)
        .unwrap();
    dispatch(&mut engine, &bytes, 7800).unwrap();
    assert!(matches!(
        engine.active.as_ref().unwrap().life,
        CharacterLife::Alive { generation: 5, .. }
    ));
}

/// Make the downed patient the event owner; the healer becomes a peer.
fn patient_owns_events(engine: &mut Engine<Vec<u8>>) {
    let healer = engine.active.take().unwrap();
    engine.active = engine.peers.remove("synthetic-reservation-0000");
    engine.peers.insert(healer.reservation.id.clone(), healer);
}

/// Make the healer the event owner again.
fn healer_owns_events(engine: &mut Engine<Vec<u8>>) {
    let patient = engine.active.take().unwrap();
    engine.active = engine.peers.remove("synthetic-reservation-0001");
    engine.peers.insert(patient.reservation.id.clone(), patient);
}

/// The owner's report of its downed state (7) with a death-state substate.
fn downed_state_report(engine: &mut Engine<Vec<u8>>, substate: u8, now_ms: u64) {
    let actor = engine.active.as_ref().unwrap();
    let mut bits = replication::BitWriter::with_profile(engine.wire_profile);
    replication::UpdateHeader {
        sequence: actor.latest_owner_sequence + 1,
        nak_sequence: 0,
        client_index: 0,
        object_type_mask: 8,
        timestamp_ms: now_ms as u32,
    }
    .encode(&mut bits)
    .unwrap();
    bits.write(1, 1).unwrap();
    bits.write(actor.reservation.player.into(), 16).unwrap();
    for field in 0..22 {
        bits.write(u32::from(field == 4), 1).unwrap();
        if field == 4 {
            replication::CharacterState {
                state: 7,
                stance: 0,
                user_data: [substate, 2, 0],
            }
            .encode(&mut bits)
            .unwrap();
        }
    }
    bits.write(0, 1).unwrap();
    let wire = bits.finish();
    dispatch(engine, &wire, now_ms).unwrap();
}

fn empty_update(engine: &mut Engine<Vec<u8>>, now_ms: u64) {
    use crate::replication::{ObjectUpdate, UpdateHeader};
    let header = UpdateHeader {
        sequence: engine.active.as_ref().unwrap().latest_owner_sequence + 1,
        nak_sequence: 0,
        client_index: 0,
        object_type_mask: 0,
        timestamp_ms: now_ms as u32,
    };
    let bytes = ObjectUpdate { header, wire: &[] }
        .encode_empty_with_profile(engine.wire_profile)
        .unwrap();
    dispatch(engine, &bytes, now_ms).unwrap();
}

/// Downed at 0 with a 7 s bleed-out; the team's waves fall at 3 s, 23 s, ...
fn downed_owner_with_waves() -> Engine<Vec<u8>> {
    let mut engine = two_actors(0.4);
    engine.enable_world_ticks();
    engine.match_started = true;
    down_patient(&mut engine, Duration::ZERO);
    let team = usize::from(patient(&engine).team());
    engine.team_waves[team] = Some(TeamWaveClock {
        first_deadline: Duration::from_secs(3),
        period: Duration::from_secs(20),
    });
    patient_owns_events(&mut engine);
    engine
}

fn assert_gave_up(actor: &Active) {
    assert!(matches!(
        actor.life,
        CharacterLife::AwaitingRespawn { generation: 4 }
    ));
    assert_eq!(actor.last_damage.health_quarter_points, 0);
    assert_eq!(actor.last_damage.conditions & 3, 0);
    // The downed hit keeps the death: attribution is that of a bleed-out.
    assert_eq!(actor.last_damage.last_attacker, 0x0380_0002);
    assert_eq!(actor.last_damage.last_source_object, 0x0680_0002);
}

#[test]
fn a_bleed_out_report_gives_up_at_once_and_spawns_on_the_next_wave() {
    let mut engine = downed_owner_with_waves();
    downed_state_report(&mut engine, 3, 1000);
    assert_gave_up(engine.active.as_ref().unwrap());
    // The world tick queues the dead character for the wave at 3 s.
    let response = engine.tick_world(Duration::from_millis(1050), 1, "tick".into());
    assert!(response.ok, "{:?}", response.error);
    assert_eq!(
        engine.active.as_ref().unwrap().pending_spawn_deadline,
        Some(Duration::from_secs(3))
    );
    empty_update(&mut engine, 3100);
    assert!(matches!(
        engine.active.as_ref().unwrap().life,
        CharacterLife::Alive { generation: 5, .. }
    ));
}

#[test]
fn other_downed_substates_keep_the_full_bleed_out() {
    let mut engine = downed_owner_with_waves();
    downed_state_report(&mut engine, 1, 1000);
    let actor = engine.active.as_ref().unwrap();
    assert!(matches!(
        actor.life,
        CharacterLife::Incapacitated { generation: 4, downed }
            if downed.deadline == Duration::from_secs(7)
    ));
    assert_eq!(actor.last_damage.conditions & 2, 2);
    engine
        .tick_incapacitation(Duration::from_millis(6999), 1)
        .unwrap();
    assert!(matches!(
        engine.active.as_ref().unwrap().life,
        CharacterLife::Incapacitated { .. }
    ));
    engine
        .tick_incapacitation(Duration::from_secs(7), 1)
        .unwrap();
    assert_gave_up(engine.active.as_ref().unwrap());
}

#[test]
fn a_downed_confirmation_gives_up_queues_and_cannot_be_revived() {
    let mut engine = downed_owner_with_waves();
    dispatch(&mut engine, &0x0211u16.to_be_bytes(), 1000).unwrap();
    let actor = engine.active.as_ref().unwrap();
    assert_gave_up(actor);
    assert_eq!(actor.pending_spawn_deadline, Some(Duration::from_secs(3)));
    // A medic arriving after the give-up finds nobody to revive.
    healer_owns_events(&mut engine);
    let _ = dispatch(&mut engine, &request(0x0380_0000), 1500);
    assert_gave_up(patient(&engine));
    patient_owns_events(&mut engine);
    empty_update(&mut engine, 3100);
    assert!(matches!(
        engine.active.as_ref().unwrap().life,
        CharacterLife::Alive { generation: 5, .. }
    ));
}

#[test]
fn a_cancel_after_a_downed_confirmation_is_an_ordinary_dead_cancel() {
    let mut engine = downed_owner_with_waves();
    dispatch(&mut engine, &0x0211u16.to_be_bytes(), 1000).unwrap();
    let messages = dispatch(&mut engine, &0x0213u16.to_be_bytes(), 1100).unwrap();
    assert!(messages.iter().any(
        |m| m.payload_hex.eq_ignore_ascii_case(&protocol::encode_hex(
            &protocol::CancelRespawnResult { success: true }.encode()
        ))
    ));
    let actor = engine.active.as_ref().unwrap();
    assert_gave_up(actor);
    assert!(actor.pending_spawn_deadline.is_none());
    // Held out of the queue like any dead player until it confirms again.
    let response = engine.tick_world(Duration::from_millis(1200), 1, "tick".into());
    assert!(response.ok, "{:?}", response.error);
    assert!(
        engine
            .active
            .as_ref()
            .unwrap()
            .pending_spawn_deadline
            .is_none()
    );
    dispatch(&mut engine, &0x0211u16.to_be_bytes(), 1300).unwrap();
    assert_eq!(
        engine.active.as_ref().unwrap().pending_spawn_deadline,
        Some(Duration::from_secs(3))
    );
}
