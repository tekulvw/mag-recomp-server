//! SYNTHETIC downed-bot checks: generated catalogs, invented tuning and a
//! flat synthetic floor. No game data.
use super::*;
use crate::bot_teammate_healing_tests::{records, with_item, world};
use crate::spawn_progression::dispatch;
use std::sync::Arc;

/// Invented tuning with a downed time longer than the medic check delay.
fn rules() -> incapacitation::Rules {
    incapacitation::Rules {
        schema_version: 1,
        incapacitated_damage_threshold_hp: 83.0,
        incapacitated_duration_seconds: 30.0,
        excluded_character_states: vec![],
        provenance: None,
    }
}

const PATIENT: u16 = 1;

fn at(ms: u64) -> Duration {
    Duration::from_millis(ms)
}

/// The medic fixture with downs enabled and the human out of the way, so
/// only the characters a test places count as medics.
fn downs(others: &[(u16, u8, [f32; 3], u16)]) -> (Engine<Vec<u8>>, u16) {
    let (engine, medic) = world(others);
    let mut engine = engine.with_incapacitation_rules(rules()).unwrap();
    engine.active.as_mut().unwrap().life = CharacterLife::AwaitingFirstSpawn;
    // Every placed bot starts on the medic's side.
    let squad = engine.bot_registry.bots[&medic].target.squad;
    for bot in engine.bot_registry.bots.values_mut() {
        bot.target.squad = squad;
    }
    (engine, medic)
}

fn team(engine: &Engine<Vec<u8>>) -> u8 {
    engine.active.as_ref().unwrap().team()
}

fn bullet(attacker: u32, kinetic: bool) -> replication::CharacterDamage {
    replication::CharacterDamage {
        last_location_and_type: if kinetic { 0x83 } else { 0x13 },
        health_quarter_points: 0,
        conditions: 0,
        last_attacker: attacker,
        last_source_object: 0x0680_0002,
        last_user_data: 19,
    }
}

fn generation(engine: &Engine<Vec<u8>>, id: u16) -> u32 {
    engine.bot_registry.bots[&id]
        .target
        .runtime
        .unwrap()
        .generation
}

/// Plan and commit one hit of `amount` HP on bot `id`, as the damage paths do.
fn hit(engine: &mut Engine<Vec<u8>>, id: u16, amount: f32, kinetic: bool, now: Duration) {
    let damage = bullet(0x0380_0077, kinetic);
    let (model, downed) = engine.plan_bot_damage(id, amount, damage, now).unwrap();
    let generation = generation(engine, id);
    engine
        .commit_planned_bot_damage(id, generation, model, downed, now)
        .unwrap();
}

fn place(engine: &mut Engine<Vec<u8>>, id: u16, p: [f32; 3]) {
    let runtime = engine
        .bot_registry
        .bots
        .get_mut(&id)
        .unwrap()
        .target
        .runtime
        .as_mut()
        .unwrap();
    runtime.position = replication::CharacterPosition {
        x: p[0],
        y: p[1],
        z: p[2],
    };
    if let Some(motion) = runtime.physics_motion.as_mut() {
        motion.position = p.map(f64::from);
    }
}

fn is_downed(engine: &Engine<Vec<u8>>, id: u16) -> bool {
    engine.bot_registry.bots[&id].downed.is_some()
}

#[test]
fn a_lethal_bullet_downs_a_bot_and_holds_it_out_of_its_wave() {
    let (mut engine, medic) = downs(&[(PATIENT, 0, [5., 0., 0.], 120)]);
    let patient = medic + PATIENT;
    hit(&mut engine, patient, 40., false, at(1000));
    let bot = &engine.bot_registry.bots[&patient];
    assert_eq!(bot.health, 0);
    assert_eq!(
        bot.damage.conditions & 2,
        2,
        "the client's downed condition"
    );
    assert_eq!(bot.damage.last_attacker, 0x0380_0077);
    let runtime = bot.target.runtime.unwrap();
    assert!(matches!(runtime.phase, development_target::BotPhase::Dead));
    assert!(
        runtime.physics_motion.is_none(),
        "a downed body does not move"
    );
    let downed = bot.downed.unwrap();
    assert_eq!(
        downed.state.deadline,
        at(31_000),
        "the authored downed time"
    );
    assert_eq!(downed.next_medic_check, at(11_000));
    assert!(downed.motion.is_some(), "kept for a revival");
    assert_eq!(
        engine
            .due_bot_respawn(patient, at(100_000), [0, 0])
            .unwrap(),
        None,
        "a downed bot waits for a medic, not a wave"
    );
}

#[test]
fn kinetic_threshold_and_disabled_hits_stay_terminal() {
    for case in ["kinetic", "threshold", "policy_off", "no_rules"] {
        let (mut engine, medic) = downs(&[(PATIENT, 0, [5., 0., 0.], 120)]);
        if case == "no_rules" {
            engine.incapacitation_rules = None;
        }
        if case == "policy_off" {
            engine
                .bot_registry
                .config
                .as_mut()
                .unwrap()
                .incapacitation
                .enabled = false;
        }
        let patient = medic + PATIENT;
        let amount = if case == "threshold" { 83. } else { 40. };
        hit(&mut engine, patient, amount, case == "kinetic", at(1000));
        let bot = &engine.bot_registry.bots[&patient];
        assert_eq!(bot.health, 0, "{case}");
        assert!(bot.downed.is_none(), "{case} is a terminal death");
        assert_eq!(bot.damage.conditions & 2, 0, "{case}");
        assert!(
            engine
                .due_bot_respawn(patient, at(100_000), [0, 0])
                .unwrap()
                .is_some(),
            "{case} joins a wave"
        );
    }
}

#[test]
fn finishing_damage_accumulates_to_the_threshold_and_keeps_the_downing_attribution() {
    let (mut engine, medic) = downs(&[(PATIENT, 0, [5., 0., 0.], 120)]);
    let patient = medic + PATIENT;
    hit(&mut engine, patient, 40., false, at(1000));
    let mut later = bullet(0x0380_0099, false);
    later.last_location_and_type = 0x25;
    let (model, downed) = engine
        .plan_bot_damage(patient, 50., later, at(2000))
        .unwrap();
    assert_eq!(downed.unwrap().accumulated_damage_hp, 50.);
    assert_eq!(model.conditions & 2, 2);
    let g = generation(&engine, patient);
    engine
        .commit_planned_bot_damage(patient, g, model, downed, at(2000))
        .unwrap();
    assert!(is_downed(&engine, patient));
    let (model, downed) = engine
        .plan_bot_damage(patient, 33., later, at(3000))
        .unwrap();
    assert!(downed.is_none(), "50 + 33 reaches the 83 HP threshold");
    assert_eq!(model.conditions & 2, 0);
    assert_eq!(
        model.last_attacker, 0x0380_0077,
        "the downing hit keeps the kill"
    );
    engine
        .commit_planned_bot_damage(patient, g, model, downed, at(3000))
        .unwrap();
    let bot = &engine.bot_registry.bots[&patient];
    assert!(bot.downed.is_none());
    assert_eq!(bot.target.runtime.unwrap().since, at(3000));
    assert_eq!(
        engine.due_bot_respawn(patient, at(3000), [0, 0]).unwrap(),
        Some(at(3000)),
        "the finished bot joins the wave after its finish"
    );
}

#[test]
fn with_no_medic_in_range_a_bot_bleeds_out_at_the_check_and_requeues() {
    let (mut engine, medic) = downs(&[(PATIENT, 0, [5., 0., 0.], 120)]);
    engine.bot_registry.bots.remove(&medic);
    let patient = medic + PATIENT;
    let side = usize::from(engine.bot_registry.bots[&patient].target.squad.team());
    engine.team_waves[side] = Some(TeamWaveClock {
        first_deadline: at(2000),
        period: at(5000),
    });
    hit(&mut engine, patient, 40., false, at(1000));
    engine.tick_bot_incapacitation(at(10_999), 1).unwrap();
    assert!(
        is_downed(&engine, patient),
        "the first check is 10 s after the down"
    );
    engine.tick_bot_incapacitation(at(11_000), 1).unwrap();
    let bot = &engine.bot_registry.bots[&patient];
    assert!(bot.downed.is_none());
    assert_eq!(bot.damage.conditions & 3, 0, "published like a give-up");
    assert_eq!(bot.damage.last_attacker, 0x0380_0077);
    let bled = records(&engine, "bot_bled_out");
    assert_eq!(bled.len(), 1);
    assert_eq!(bled[0]["reason"], "no_medic_in_range");
    assert_eq!(bled[0]["downed_ms"], 10_000);
    // Waves fall at 2, 7, 12 s ...: the bleed-out at 11 s takes the 12 s wave.
    assert_eq!(
        engine.due_bot_respawn(patient, at(11_500), [0, 0]).unwrap(),
        None
    );
    assert_eq!(
        engine.due_bot_respawn(patient, at(12_000), [0, 0]).unwrap(),
        Some(at(12_000))
    );
}

#[test]
fn a_bot_medic_at_25_m_keeps_it_waiting_until_it_walks_away() {
    let (mut engine, medic) = downs(&[(PATIENT, 0, [25., 0., 0.], 120)]);
    let patient = medic + PATIENT;
    hit(&mut engine, patient, 40., false, at(0));
    engine.tick_bot_incapacitation(at(10_000), 1).unwrap();
    assert!(is_downed(&engine, patient), "a medic 25 m away");
    let waiting = records(&engine, "bot_downed_waiting_for_medic");
    assert_eq!(waiting.len(), 1);
    assert_eq!(waiting[0]["medic"]["object_index"], medic);
    engine.tick_bot_incapacitation(at(10_900), 1).unwrap();
    engine.tick_bot_incapacitation(at(11_000), 1).unwrap();
    assert!(
        is_downed(&engine, patient),
        "still in range at the 1 s recheck"
    );
    assert_eq!(records(&engine, "bot_downed_waiting_for_medic").len(), 1);
    place(&mut engine, medic, [-10., 0., 0.]);
    engine.tick_bot_incapacitation(at(11_500), 1).unwrap();
    assert!(is_downed(&engine, patient), "between rechecks");
    engine.tick_bot_incapacitation(at(12_000), 1).unwrap();
    assert!(!is_downed(&engine, patient), "35 m away is out of range");
    assert_eq!(
        records(&engine, "bot_bled_out")[0]["reason"],
        "no_medic_in_range"
    );
}

#[test]
fn a_medic_in_range_never_extends_the_authored_downed_time() {
    let (mut engine, medic) = downs(&[(PATIENT, 0, [20., 0., 0.], 120)]);
    let patient = medic + PATIENT;
    let side = engine.bot_registry.bots[&medic].target.squad.team();
    assert_eq!(engine.bot_registry.bots[&patient].target.squad.team(), side);
    hit(&mut engine, patient, 40., false, at(0));
    for second in 10..30 {
        engine
            .tick_bot_incapacitation(at(second * 1000), 1)
            .unwrap();
        assert!(is_downed(&engine, patient), "{second} s");
    }
    engine.tick_bot_incapacitation(at(30_000), 1).unwrap();
    assert!(!is_downed(&engine, patient));
    assert_eq!(records(&engine, "bot_bled_out")[0]["reason"], "expired");
}

#[test]
fn enemy_and_downed_medics_do_not_count() {
    for case in ["enemy", "downed"] {
        let (mut engine, medic) = downs(&[(PATIENT, 0, [10., 0., 0.], 120)]);
        let patient = medic + PATIENT;
        if case == "enemy" {
            let other = 1 - engine.bot_registry.bots[&patient].target.squad.team();
            engine
                .bot_registry
                .bots
                .get_mut(&medic)
                .unwrap()
                .target
                .squad = replication::SquadAddress::from_index(16 * u16::from(other)).unwrap();
        } else {
            hit(&mut engine, medic, 40., false, at(0));
            assert!(is_downed(&engine, medic));
        }
        hit(&mut engine, patient, 40., false, at(0));
        engine.tick_bot_incapacitation(at(10_000), 1).unwrap();
        assert!(
            !is_downed(&engine, patient),
            "an {case} medic does not count"
        );
    }
}

/// Give the fixture's human a kit holding the medkit, at `position`.
fn arm_human_medic(engine: &mut Engine<Vec<u8>>, medic: u16, restore: f32, position: [f32; 3]) {
    let kit = engine.bot_registry.bots[&medic].target.loadout.clone();
    let human = engine.active.as_mut().unwrap();
    human.life = CharacterLife::Alive {
        generation: 2,
        health_quarter_points: 400,
    };
    human.current_inventory_handle = Some(kit.handle);
    human.spawned_loadout = Some(kit);
    human.modifiers = crate::incapacitation_tests::modifiers(restore, 0.);
    human.reservation.character.account_id = Some("synthetic-medic-account".into());
    human.reservation.character.character_id = Some(1);
    human.tracking.server_position(
        replication::CharacterPosition {
            x: position[0],
            y: position[1],
            z: position[2],
        },
        player_tracking::PositionOrigin::ServerSpawn,
        0,
    );
}

#[test]
fn a_human_medic_counts_only_with_the_medkit_and_the_revive_skill() {
    for (restore, waits) in [(0.4, true), (0., false)] {
        let (mut engine, medic) = downs(&[(PATIENT, 0, [0., 0., 0.], 120)]);
        let patient = medic + PATIENT;
        arm_human_medic(&mut engine, medic, restore, [0., 0., 20.]);
        // The bot medic is not a medic for this check.
        let bot = engine.bot_registry.bots.get_mut(&medic).unwrap();
        bot.target.loadout = with_item(bot.target.loadout.clone(), 0);
        assert_eq!(bot.target.squad.team(), team(&engine));
        hit(&mut engine, patient, 40., false, at(0));
        engine.tick_bot_incapacitation(at(10_000), 1).unwrap();
        assert_eq!(is_downed(&engine, patient), waits, "restore {restore}");
        if waits {
            let waiting = records(&engine, "bot_downed_waiting_for_medic");
            assert_eq!(waiting[0]["medic"]["kind"], "human");
        }
    }
}

#[test]
fn a_human_medic_revives_a_downed_bot_with_the_human_credit() {
    let (mut engine, medic) = downs(&[(PATIENT, 0, [3., 0., 0.], 120)]);
    let patient = medic + PATIENT;
    arm_human_medic(&mut engine, medic, 0.4, [2., 0., 0.]);
    hit(&mut engine, patient, 40., false, at(0));
    let healer = engine.active.as_ref().unwrap().reservation.id.clone();
    engine.bot_registry.known_for_test(&healer).insert(patient);
    engine.active.as_mut().unwrap().pending_shared.clear();
    let messages = dispatch(
        &mut engine,
        &crate::incapacitation_tests::request(0x0380_0000 | u32::from(patient)),
        2000,
    )
    .unwrap();
    let bot = &engine.bot_registry.bots[&patient];
    assert!(bot.downed.is_none());
    assert_eq!(bot.health, 160, "40% of the healer's 100 HP");
    assert_eq!(bot.damage.conditions & 2, 0);
    assert_eq!(bot.damage.last_location_and_type >> 4, 7);
    assert_eq!(
        bot.damage.last_attacker, 0x0380_0077,
        "the downing attribution"
    );
    assert_eq!(bot.revivals, 1);
    let runtime = bot.target.runtime.unwrap();
    assert!(matches!(
        runtime.phase,
        development_target::BotPhase::Complete
    ));
    assert!(runtime.physics_motion.is_some(), "it can move again");
    assert_eq!(runtime.generation, generation(&engine, patient));
    let notice = [
        0x01,
        0xff,
        0x00,
        engine.active.as_ref().unwrap().reservation.player as u8,
        0x00,
        patient as u8,
        0x0b,
        0xff,
        0xff,
        0xff,
        0xff,
        0x00,
    ];
    assert!(
        messages
            .iter()
            .any(|m| protocol::decode_hex(&m.payload_hex) == notice),
        "the squad hears the resuscitation"
    );
    let revives: Vec<_> = engine
        .pending_reward_batch()
        .into_iter()
        .filter(|event| event.kind == progression::RewardKind::TeammateResuscitated)
        .collect();
    assert_eq!(revives.len(), 1, "the same revive XP as for a human");
    let human = engine.active.as_ref().unwrap();
    assert!(
        !human.pending_shared.is_empty(),
        "the bot's recovery is published"
    );
    // A second request finds no downed bot.
    let again = dispatch(
        &mut engine,
        &crate::incapacitation_tests::request(0x0380_0000 | u32::from(patient)),
        2100,
    )
    .unwrap();
    assert!(again.is_empty());
    assert_eq!(engine.bot_registry.bots[&patient].health, 160);
}

#[test]
fn a_revive_is_refused_for_an_enemy_bot_or_without_the_skill() {
    for case in ["enemy", "unskilled"] {
        let (mut engine, medic) = downs(&[(PATIENT, 0, [3., 0., 0.], 120)]);
        let patient = medic + PATIENT;
        arm_human_medic(
            &mut engine,
            medic,
            if case == "unskilled" { 0. } else { 0.4 },
            [2., 0., 0.],
        );
        if case == "enemy" {
            let other = 1 - team(&engine);
            engine
                .bot_registry
                .bots
                .get_mut(&patient)
                .unwrap()
                .target
                .squad = replication::SquadAddress::from_index(16 * u16::from(other)).unwrap();
        }
        hit(&mut engine, patient, 40., false, at(0));
        let _ = dispatch(
            &mut engine,
            &crate::incapacitation_tests::request(0x0380_0000 | u32::from(patient)),
            2000,
        );
        assert!(is_downed(&engine, patient), "{case}");
    }
}

#[test]
fn a_bot_medic_revives_a_downed_bot_teammate_in_reach() {
    let (mut engine, medic) = downs(&[(PATIENT, 0, [2., 0., 0.], 120)]);
    let patient = medic + PATIENT;
    hit(&mut engine, patient, 40., false, at(0));
    let human = engine.active.as_mut().unwrap();
    human.life = CharacterLife::Alive {
        generation: 1,
        health_quarter_points: 400,
    };
    engine.tick_bot_healing(at(1000), 1).unwrap();
    let bot = &engine.bot_registry.bots[&patient];
    assert!(bot.downed.is_none());
    assert_eq!(
        bot.health, 100,
        "the policy's quarter of the reviver's 100 HP"
    );
    let started = records(&engine, "bot_teammate_healing_started");
    assert_eq!(started[0]["patients"][0]["kind"], "revived_bot");
}

#[test]
fn a_collapse_after_a_revival_is_a_distinct_kill() {
    let (engine, _) = shooting_range();
    let recipient =
        progression::RewardRecipient::from_active(engine.active.as_ref().unwrap()).unwrap();
    let first = recipient.bot_killed_collapse(3, 9, 0);
    assert_eq!(first.reward_id, recipient.bot_killed(3, 9).reward_id);
    assert_ne!(
        first.reward_id,
        recipient.bot_killed_collapse(3, 9, 1).reward_id
    );
}

/// One human with the synthetic rifle against one enemy bot at 30 HP.
fn shooting_range() -> (Engine<Vec<u8>>, u16) {
    let mut armed = crate::human_kill_reward_tests::fixture();
    let equipped = armed.active.take().unwrap();
    let mut engine = crate::bot_registry::identity_tests::engine("Synthetic Downed Range");
    engine = engine.with_incapacitation_rules(rules()).unwrap();
    let response = engine.handle(
        Request::Reserve {
            v: BRIDGE_VERSION,
            request_id: "synthetic-range-reserve".into(),
            reservation_id: "synthetic-range-0000".into(),
            world_id: 1,
            player_id: 0,
            account_id: 1,
            session_key: "aaaaaaaaaaaaaaaa".into(),
            access_key: "bbbbbbbbbbbbbbbb".into(),
            character: serde_json::from_value(json!({"name":"Synthetic Range","faction":2,
                "head":0,"voice":0,"universe_id":26,"character_id":7}))
            .unwrap(),
        },
        Duration::ZERO,
        1,
    );
    assert!(response.ok, "{:?}", response.error);
    let source = engine.active.as_mut().unwrap();
    crate::spawn_progression::ready(source);
    source.life = CharacterLife::Alive {
        generation: 4,
        health_quarter_points: 400,
    };
    source.reservation.character.account_id = Some("synthetic-range-owner".into());
    source.reservation.character.character_id = Some(7);
    engine
        .reconcile_bot_population(Duration::ZERO, 1, &mut Default::default())
        .unwrap();
    let target = engine.bot_combat_snapshots()[0].object_index;
    engine.item_catalog = Arc::clone(&armed.item_catalog);
    let source = engine.active.as_mut().unwrap();
    source.current_inventory_handle = equipped.current_inventory_handle;
    source.spawned_loadout = equipped.spawned_loadout.clone();
    source.observed_primary = equipped.observed_primary;
    source.primary_equipped = true;
    let kit = source.equipped_loadout().unwrap();
    let side = source.team();
    let faction = source.reservation.character.faction;
    let bot = engine.bot_registry.bots.get_mut(&target).unwrap();
    bot.target.squad = replication::SquadAddress::from_index(u16::from(1 - side) * 16).unwrap();
    bot.target.faction = faction;
    bot.target.loadout = kit;
    bot.health = 120;
    bot.damage.health_quarter_points = 120;
    (engine, target)
}

fn shoot(engine: &mut Engine<Vec<u8>>, target: u16, now_ms: u64) {
    let wire = crate::human_kill_reward_tests::native_hit(0x0380_0000 | u32::from(target), 17);
    dispatch(engine, &wire, now_ms).unwrap();
}

fn rewards(engine: &Engine<Vec<u8>>, kind: progression::RewardKind) -> usize {
    engine
        .pending_reward_batch()
        .into_iter()
        .filter(|event| event.kind == kind)
        .count()
}

#[test]
fn shooting_a_downed_bot_finishes_it_with_one_kill_at_the_down() {
    let (mut engine, target) = shooting_range();
    shoot(&mut engine, target, 1000);
    assert!(
        is_downed(&engine, target),
        "the 40 HP bullet downs the 30 HP bot"
    );
    assert_eq!(rewards(&engine, progression::RewardKind::EnemyKilled), 1);
    assert_eq!(
        engine.active.as_ref().unwrap().pending_notifications.len(),
        1,
        "the kill feed names the downing hit"
    );
    assert_eq!(engine.bot_registry.bots[&target].firearm_kd.deaths, 1);
    shoot(&mut engine, target, 1100);
    shoot(&mut engine, target, 1200);
    assert!(is_downed(&engine, target), "80 HP of finishing damage");
    shoot(&mut engine, target, 1300);
    assert!(!is_downed(&engine, target), "120 HP reaches the threshold");
    assert_eq!(engine.bot_registry.bots[&target].health, 0);
    assert_eq!(rewards(&engine, progression::RewardKind::EnemyKilled), 1);
    assert_eq!(rewards(&engine, progression::RewardKind::EnemyFinished), 1);
    assert_eq!(
        engine.active.as_ref().unwrap().pending_notifications.len(),
        1
    );
    assert_eq!(engine.bot_registry.bots[&target].firearm_kd.deaths, 1);
    let since = engine.bot_registry.bots[&target]
        .target
        .runtime
        .unwrap()
        .since;
    assert_eq!(since, at(1300), "the death is final at the finish");
}

fn queue_grenade(engine: &mut Engine<Vec<u8>>, target: u16, amount_hp: f32, kinetic: bool) {
    let generation = generation(engine, target);
    let attacker = 0x0380_0000 | u32::from(engine.active.as_ref().unwrap().reservation.player);
    engine
        .active
        .as_mut()
        .unwrap()
        .grenade_damage
        .push_back(PendingDamage {
            target_character_id: 0x0380_0000 | u32::from(target),
            target_reservation_id: None,
            id: format!("synthetic-downed-grenade-{amount_hp}"),
            generation,
            source: PendingDamageSource::Other,
            damage: damage::ResolvedDamage {
                amount_hp,
                kind: damage::DamageKind::Explosive,
                kinetic_response: kinetic,
                location: 3,
                attacker_id: attacker,
                source_object_id: 0x1080_0000,
                user_data: 0,
            },
        });
}

fn owner_update(engine: &mut Engine<Vec<u8>>, now_ms: u64) {
    let sequence = engine.active.as_ref().unwrap().latest_owner_sequence + 1;
    let wire = replication::ObjectUpdate {
        header: replication::UpdateHeader {
            sequence,
            nak_sequence: 0,
            client_index: 0,
            object_type_mask: 0,
            timestamp_ms: now_ms as u32,
        },
        wire: &[],
    }
    .encode_empty_with_profile(engine.wire_profile)
    .unwrap();
    dispatch(engine, &wire, now_ms).unwrap();
}

#[test]
fn a_kinetic_grenade_kills_outright_and_a_grenade_finishes_a_downed_bot() {
    let (mut engine, target) = shooting_range();
    queue_grenade(&mut engine, target, 40., true);
    owner_update(&mut engine, 1000);
    let bot = &engine.bot_registry.bots[&target];
    assert_eq!(bot.health, 0);
    assert!(
        bot.downed.is_none(),
        "KineticResponse is terminal below the threshold"
    );

    let (mut engine, target) = shooting_range();
    shoot(&mut engine, target, 1000);
    assert!(is_downed(&engine, target));
    queue_grenade(&mut engine, target, 90., true);
    owner_update(&mut engine, 1100);
    assert!(
        !is_downed(&engine, target),
        "the blast finishes the downed bot"
    );
    assert_eq!(rewards(&engine, progression::RewardKind::EnemyKilled), 1);
    assert_eq!(rewards(&engine, progression::RewardKind::EnemyFinished), 1);
}

/// The audit of the last bullet hit the engine planned.
fn last_bullet_hit(engine: &Engine<Vec<u8>>) -> Value {
    records(engine, "event")
        .into_iter()
        .rev()
        .find(|row| row["decoded"]["type"] == "bullet_hit")
        .expect("a bullet hit audit")
}

#[test]
fn the_next_round_of_a_burst_finishes_the_bot_it_downed() {
    // The live burst class: one round takes the bot to 0 HP and the next
    // round of the same burst lands 20 ms later. On a downed body it is
    // finishing damage, not a refused hit.
    let (mut engine, target) = shooting_range();
    shoot(&mut engine, target, 1000);
    assert!(is_downed(&engine, target));
    shoot(&mut engine, target, 1020);
    let row = last_bullet_hit(&engine);
    assert_eq!(row["gameplay_denial"], Value::Null);
    assert!(row["decoded"]["refusal"].is_null());
    assert_eq!(
        row["decoded"]["applied_damage"]["health_after_quarter_points"],
        0
    );
    assert_eq!(
        engine.bot_registry.bots[&target]
            .downed
            .unwrap()
            .state
            .accumulated_damage_hp,
        40.
    );
}

#[test]
fn a_late_round_on_a_finished_bot_is_refused_and_says_how_long_it_has_been_dead() {
    let (mut engine, target) = shooting_range();
    for ms in [1000, 1100, 1200, 1300] {
        shoot(&mut engine, target, ms);
    }
    assert!(!is_downed(&engine, target), "finished at 1300 ms");
    let killed = rewards(&engine, progression::RewardKind::EnemyKilled);
    let finished = rewards(&engine, progression::RewardKind::EnemyFinished);
    shoot(&mut engine, target, 1350);
    let row = last_bullet_hit(&engine);
    assert_eq!(row["gameplay_denial"], "bullet_target_not_alive");
    assert!(row["decoded"]["applied_damage"].is_null());
    let refusal = &row["decoded"]["refusal"];
    assert_eq!(refusal["code"], "bullet_target_not_alive");
    assert_eq!(refusal["target"]["kind"], "bot");
    assert_eq!(refusal["target"]["life"]["state"], "dead");
    assert_eq!(refusal["target"]["life"]["dead_for_ms"], 50);
    assert_eq!(refusal["shooter"]["life"]["state"], "alive");
    assert_eq!(engine.bot_registry.bots[&target].health, 0);
    assert_eq!(
        rewards(&engine, progression::RewardKind::EnemyKilled),
        killed
    );
    assert_eq!(
        rewards(&engine, progression::RewardKind::EnemyFinished),
        finished
    );
}

#[test]
fn a_downed_shooter_is_refused_and_the_audit_names_its_downed_life() {
    // The live desync class: the server holds the shooter down while its
    // client keeps firing. The hit stays refused (a downed character cannot
    // fire) and the audit says so.
    let (mut engine, target) = shooting_range();
    engine.active.as_mut().unwrap().life = CharacterLife::Incapacitated {
        generation: 4,
        downed: incapacitation::Downed {
            deadline: at(31_000),
            accumulated_damage_hp: 0.,
        },
    };
    shoot(&mut engine, target, 1000);
    let row = last_bullet_hit(&engine);
    assert_eq!(
        row["gameplay_denial"],
        "bullet_requires_living_armed_shooter"
    );
    let refusal = &row["decoded"]["refusal"];
    assert_eq!(refusal["shooter"]["life"]["state"], "downed");
    assert_eq!(refusal["shooter"]["life"]["bleeds_out_in_ms"], 30_000);
    assert_eq!(refusal["shooter"]["life"]["expired"], false);
    assert_eq!(refusal["shooter"]["mounted_seat"], Value::Null);
    assert_eq!(refusal["target"]["life"]["state"], "alive");
    assert_eq!(engine.bot_registry.bots[&target].health, 120);
    assert!(!is_downed(&engine, target));
}

#[test]
fn an_accepted_hit_carries_no_refusal() {
    let (mut engine, target) = shooting_range();
    shoot(&mut engine, target, 1000);
    let row = last_bullet_hit(&engine);
    assert_eq!(row["gameplay_denial"], Value::Null);
    assert!(row["decoded"].get("refusal").is_none());
}
