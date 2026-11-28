//! SYNTHETIC medic approach checks on the generated armed-objective floor and
//! navmesh, with a generated medkit catalog. No game data.
use super::*;
use crate::bot_armed_objectives::tests::{bot_at, engine, plan_tick, records};
use crate::bot_teammate_healing_tests::{MEDKIT, medic_combat, with_item};

/// Medics and patients defend, so the attackers' demolition duties (a
/// higher duty than a medic approach) never take them.
const A: u8 = 1;
const SA: u16 = 16;
const B: u8 = 0;
const SB: u16 = 0;

fn rules() -> incapacitation::Rules {
    incapacitation::Rules {
        schema_version: 1,
        incapacitated_damage_threshold_hp: 83.0,
        incapacitated_duration_seconds: 30.0,
        excluded_character_states: vec![],
        provenance: None,
    }
}

fn at(ms: u64) -> Duration {
    Duration::from_millis(ms)
}

/// The armed-objective floor with `slots` (team, squad, x, z), downs on and
/// the medkit catalog. Bots standing at `medics` carry the medkit. A ready
/// human on the medics' side stands at `human`, so the world ticks run.
fn world_with(
    slots: &[(u8, u16, f32, f32)],
    medics: &[(f32, f32)],
    human: Option<(f32, f32)>,
) -> Engine<Vec<u8>> {
    // The population needs as many slots on each side: pad the shorter side
    // with bots far from the scene.
    let mut slots = slots.to_vec();
    for team in 0..2u8 {
        let other = slots.iter().filter(|s| s.0 != team).count();
        let mine = slots.iter().filter(|s| s.0 == team).count();
        for i in mine..other {
            slots.push((team, u16::from(team) * 16 + 3, 112., 10. + 4. * i as f32));
        }
    }
    let mut engine = engine(&slots, false)
        .with_incapacitation_rules(rules())
        .unwrap();
    if let Some((x, z)) = human {
        add_human(&mut engine, x, z);
    }
    // The bootstrap kits stay, so later lives still spawn; the combat catalog
    // is the generated one with the medkit.
    engine.item_catalog = std::sync::Arc::new(
        crate::bot_registry::identity_tests::bootstrap_items().with_retail_combat(medic_combat()),
    );
    for &(x, z) in medics {
        let id = bot_at(&engine, x, z);
        let bot = engine.bot_registry.bots.get_mut(&id).unwrap();
        bot.target.loadout = with_item(bot.target.loadout.clone(), MEDKIT);
    }
    engine
}

fn world(slots: &[(u8, u16, f32, f32)], medics: &[(f32, f32)]) -> Engine<Vec<u8>> {
    world_with(slots, medics, None)
}

fn down(engine: &mut Engine<Vec<u8>>, id: u16, now: Duration) {
    let damage = replication::CharacterDamage {
        last_location_and_type: 0x13,
        health_quarter_points: 0,
        conditions: 0,
        last_attacker: 0x0380_0077,
        last_source_object: 0x0680_0002,
        last_user_data: 0,
    };
    let (model, downed) = engine.plan_bot_damage(id, 40., damage, now).unwrap();
    assert!(downed.is_some());
    let generation = engine.bot_registry.bots[&id]
        .target
        .runtime
        .unwrap()
        .generation;
    engine
        .commit_planned_bot_damage(id, generation, model, downed, now)
        .unwrap();
}

fn approach(engine: &Engine<Vec<u8>>, id: u16) -> Option<Approach> {
    engine.bot_registry.bots[&id].medic_approach
}

fn place(engine: &mut Engine<Vec<u8>>, id: u16, x: f32, z: f32) {
    let r = engine
        .bot_registry
        .bots
        .get_mut(&id)
        .unwrap()
        .target
        .runtime
        .as_mut()
        .unwrap();
    r.position.x = x;
    r.position.z = z;
    if let Some(m) = r.physics_motion.as_mut() {
        m.position[0] = x.into();
        m.position[2] = z.into();
    }
}

fn ended(engine: &Engine<Vec<u8>>) -> Vec<String> {
    records(engine, "bot_medic_approach_ended")
        .into_iter()
        .map(|r| r["reason"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn the_nearest_free_medic_walks_to_a_downed_bot() {
    let slots = [
        (A, SA, 40., 60.),
        (A, SA, 50., 60.),
        (A, SA, 62., 60.),
        (B, SB, 100., 20.),
    ];
    let mut engine = world(&slots, &[(40., 60.), (50., 60.)]);
    let (far, near, patient) = (
        bot_at(&engine, 40., 60.),
        bot_at(&engine, 50., 60.),
        bot_at(&engine, 62., 60.),
    );
    down(&mut engine, patient, at(0));
    plan_tick(&mut engine, 1000);
    let a = approach(&engine, near).expect("the 12 m medic is sent");
    assert_eq!(a.patient, Patient::Bot(patient));
    assert!(!a.arrived);
    let r = engine.bot_registry.bots[&near].target.runtime.unwrap();
    assert!(matches!(r.phase, BotPhase::Relocating), "it walks");
    assert!(approach(&engine, far).is_none(), "one medic per patient");
    let started = records(&engine, "bot_medic_approach_started");
    assert_eq!(started.len(), 1);
    assert_eq!(started[0]["object_index"], near);
    // The next tick keeps the same medic and does not route it again.
    plan_tick(&mut engine, 1050);
    assert_eq!(records(&engine, "bot_medic_approach_started").len(), 1);
    assert!(approach(&engine, far).is_none());
}

#[test]
fn each_patient_gets_its_own_medic_and_one_medic_serves_one_patient() {
    let slots = [
        (A, SA, 40., 60.),
        (A, SA, 44., 60.),
        (A, SA, 52., 60.),
        (A, SA, 56., 60.),
    ];
    let mut engine = world(&slots, &[(40., 60.)]);
    let medic = bot_at(&engine, 40., 60.);
    let (first, second) = (bot_at(&engine, 52., 60.), bot_at(&engine, 56., 60.));
    down(&mut engine, first, at(0));
    down(&mut engine, second, at(0));
    plan_tick(&mut engine, 1000);
    assert_eq!(
        approach(&engine, medic).unwrap().patient,
        Patient::Bot(first)
    );
    assert_eq!(records(&engine, "bot_medic_approach_started").len(), 1);
    // A second medic takes the other patient.
    let other = bot_at(&engine, 44., 60.);
    let bot = engine.bot_registry.bots.get_mut(&other).unwrap();
    bot.target.loadout = with_item(bot.target.loadout.clone(), MEDKIT);
    plan_tick(&mut engine, 1050);
    assert_eq!(
        approach(&engine, other).unwrap().patient,
        Patient::Bot(second)
    );
    assert_eq!(
        approach(&engine, medic).unwrap().patient,
        Patient::Bot(first)
    );
}

#[test]
fn medics_beyond_30_m_enemy_medics_and_downed_medics_are_not_sent() {
    let slots = [
        (A, SA, 20., 60.),
        (B, SB, 56., 60.),
        (A, SA, 58., 60.),
        (A, SA, 60., 60.),
    ];
    let mut engine = world(&slots, &[(20., 60.), (56., 60.), (58., 60.)]);
    let (far, enemy, downed_medic, patient) = (
        bot_at(&engine, 20., 60.),
        bot_at(&engine, 56., 60.),
        bot_at(&engine, 58., 60.),
        bot_at(&engine, 60., 60.),
    );
    down(&mut engine, patient, at(0));
    down(&mut engine, downed_medic, at(0));
    plan_tick(&mut engine, 1000);
    for id in [far, enemy, downed_medic] {
        assert!(approach(&engine, id).is_none(), "{id}");
    }
    assert!(records(&engine, "bot_medic_approach_started").is_empty());
}

#[test]
fn an_arrived_medic_stops_revives_and_returns_to_its_objective() {
    let slots = [(A, SA, 45., 60.), (A, SA, 60., 60.)];
    let mut engine = world_with(&slots, &[(45., 60.)], Some((110., 110.)));
    let (medic, patient) = (bot_at(&engine, 45., 60.), bot_at(&engine, 60., 60.));
    down(&mut engine, patient, at(0));
    plan_tick(&mut engine, 1000);
    assert!(approach(&engine, medic).is_some());
    // As if it had walked there.
    place(&mut engine, medic, 58., 60.);
    plan_tick(&mut engine, 4000);
    assert!(approach(&engine, medic).unwrap().arrived);
    let r = engine.bot_registry.bots[&medic].target.runtime.unwrap();
    assert!(
        matches!(r.phase, BotPhase::Complete),
        "it stops by the patient"
    );
    assert_eq!(records(&engine, "bot_medic_approach_arrived").len(), 1);
    engine.tick_bot_healing(at(4050), 1).unwrap();
    assert!(
        engine.bot_registry.bots[&patient].downed.is_none(),
        "revived"
    );
    assert!(engine.bot_registry.bots[&patient].health > 0);
    plan_tick(&mut engine, 4100);
    assert!(approach(&engine, medic).is_none());
    assert_eq!(ended(&engine), ["patient_revived"]);
    assert!(
        engine.bot_registry.bots[&medic].task.objective.is_some(),
        "back on its objective"
    );
}

#[test]
fn an_approach_ends_on_bleed_out_medic_down_higher_duty_or_no_progress() {
    let slots = [(A, SA, 45., 60.), (A, SA, 60., 60.)];
    for case in ["bleed_out", "medic_down", "duty", "stall"] {
        let mut engine = world(&slots, &[(45., 60.)]);
        let (medic, patient) = (bot_at(&engine, 45., 60.), bot_at(&engine, 60., 60.));
        down(&mut engine, patient, at(0));
        plan_tick(&mut engine, 1000);
        assert!(approach(&engine, medic).is_some(), "{case}");
        let expected = match case {
            "bleed_out" => {
                engine
                    .bleed_out_bot(patient, "expired", at(2000), 1)
                    .unwrap();
                plan_tick(&mut engine, 2050);
                "patient_lost"
            }
            "medic_down" => {
                down(&mut engine, medic, at(2000));
                plan_tick(&mut engine, 2050);
                "medic_down"
            }
            "duty" => {
                let mut receipts = Vec::new();
                let busy = std::collections::BTreeSet::from([medic]);
                engine
                    .tick_bot_medic_approaches(at(2000), &busy, &mut receipts)
                    .unwrap();
                assert_eq!(receipts[0]["reason"], "higher_duty");
                assert!(approach(&engine, medic).is_none());
                continue;
            }
            _ => {
                // It never moves: six seconds without getting closer.
                plan_tick(&mut engine, 6900);
                assert!(approach(&engine, medic).is_some(), "not yet");
                plan_tick(&mut engine, 7000);
                "no_progress"
            }
        };
        assert!(approach(&engine, medic).is_none(), "{case}");
        assert_eq!(ended(&engine), [expected], "{case}");
    }
}

#[test]
fn approaches_need_the_policy() {
    let slots = [(A, SA, 45., 60.), (A, SA, 60., 60.)];
    for case in ["approach_off", "revive_off"] {
        let mut engine = world(&slots, &[(45., 60.)]);
        let (medic, patient) = (bot_at(&engine, 45., 60.), bot_at(&engine, 60., 60.));
        down(&mut engine, patient, at(0));
        let healing = &mut engine.bot_registry.config.as_mut().unwrap().healing;
        if case == "approach_off" {
            healing.approach_downed = false;
        } else {
            healing.revive = false;
        }
        plan_tick(&mut engine, 1000);
        assert!(approach(&engine, medic).is_none(), "{case}");
    }
}

/// A ready human player 63 defending (the medics' side) at (x, z).
fn add_human(engine: &mut Engine<Vec<u8>>, x: f32, z: f32) -> u16 {
    let total = engine.bot_registry.bots.len() + 1;
    engine.bot_registry.set_total_for_test(total);
    let player = 63;
    let response = engine.handle(
        Request::Reserve {
            v: BRIDGE_VERSION,
            request_id: "synthetic-medic-approach-reserve".into(),
            reservation_id: "synthetic-medic-approach-human".into(),
            world_id: 1,
            player_id: player,
            account_id: 64,
            session_key: "aaaaaaaaaaaaaaaa".into(),
            access_key: "bbbbbbbbbbbbbbbb".into(),
            character: serde_json::from_value(json!({"name":"Synthetic Approach Human",
                "faction":1,"head":0,"voice":0,"universe_id":26,"character_id":64}))
            .unwrap(),
        },
        Duration::ZERO,
        1,
    );
    assert!(response.ok, "{:?}", response.error);
    let catalog = std::sync::Arc::new(
        engine
            .spawn_catalog
            .as_ref()
            .unwrap()
            .for_squad(SA)
            .unwrap(),
    );
    let human = engine.active.as_mut().unwrap();
    crate::spawn_progression::ready(human);
    human.squad_catalog = catalog;
    human.life = CharacterLife::Alive {
        generation: 1,
        health_quarter_points: 400,
    };
    human.tracking.server_position(
        replication::CharacterPosition { x, y: 0., z },
        player_tracking::PositionOrigin::ServerSpawn,
        0,
    );
    player
}

#[test]
fn a_downed_human_gets_a_bot_medic_too() {
    let slots = [(A, SA, 45., 60.), (A, SA, 90., 90.)];
    let mut engine = world_with(&slots, &[(45., 60.)], Some((60., 60.)));
    let medic = bot_at(&engine, 45., 60.);
    let player = 63;
    let side = engine.bot_registry.bots[&medic].target.squad.team();
    let human = engine.active.as_mut().unwrap();
    assert_eq!(human.team(), side);
    human.life = CharacterLife::Incapacitated {
        generation: 1,
        downed: incapacitation::Downed {
            deadline: at(30_000),
            accumulated_damage_hp: 0.,
        },
    };
    plan_tick(&mut engine, 1000);
    assert_eq!(
        approach(&engine, medic).unwrap().patient,
        Patient::Human(player)
    );
}

#[test]
fn a_medic_walks_over_on_the_world_tick_and_revives_the_downed_bot() {
    let slots = [(A, SA, 45., 60.), (A, SA, 60., 60.)];
    let mut engine = world_with(&slots, &[(45., 60.)], Some((110., 110.)));
    let (medic, patient) = (bot_at(&engine, 45., 60.), bot_at(&engine, 60., 60.));
    down(&mut engine, patient, at(0));
    let mut revived_at = None;
    for step in 1..=200 {
        let now = at(step * 50);
        engine.tick_bots(now, 1).unwrap();
        engine.tick_bot_healing(now, 1).unwrap();
        if engine.bot_registry.bots[&patient].downed.is_none() {
            revived_at = Some(now);
            break;
        }
    }
    let revived_at = revived_at.expect("revived inside 10 s");
    assert!(engine.bot_registry.bots[&patient].health > 0);
    assert!(revived_at < at(10_000), "before the 10 s medic check");
    // It walked: the medkit reaches 5 m and the medic started 15 m away.
    assert_eq!(records(&engine, "bot_medic_approach_started").len(), 1);
    let patient_feet = engine.bot_registry.bots[&patient]
        .target
        .runtime
        .unwrap()
        .physical_position();
    let me = engine.bot_registry.bots[&medic]
        .target
        .runtime
        .unwrap()
        .physical_position();
    assert!((me[0] - patient_feet[0]).hypot(me[2] - patient_feet[2]) <= 5.);
    engine.tick_bots(revived_at + at(50), 1).unwrap();
    assert_eq!(ended(&engine), ["patient_revived"]);
}

#[test]
fn a_patient_off_the_medics_walking_component_gets_no_approach_and_no_retry() {
    // The generated navmesh covers 1..119 m; the patient lies beyond it.
    let slots = [(A, SA, 110., 60.), (A, SA, 115., 60.)];
    let mut engine = world(&slots, &[(110., 60.)]);
    let (medic, patient) = (bot_at(&engine, 110., 60.), bot_at(&engine, 115., 60.));
    down(&mut engine, patient, at(0));
    place(&mut engine, patient, 125., 60.);
    plan_tick(&mut engine, 1000);
    assert!(approach(&engine, medic).is_none());
    let refused = records(&engine, "bot_medic_approach_unroutable");
    assert_eq!(refused.len(), 1);
    assert_eq!(refused[0]["reason"], "navmesh_component");
    // The pair is not tried again during this downed life.
    plan_tick(&mut engine, 1050);
    plan_tick(&mut engine, 9000);
    assert_eq!(records(&engine, "bot_medic_approach_unroutable").len(), 1);
}

#[test]
fn the_medic_and_the_revived_bot_resume_their_objective_tasks() {
    let slots = [(A, SA, 45., 60.), (A, SA, 60., 60.)];
    let mut engine = world_with(&slots, &[(45., 60.)], Some((110., 110.)));
    let (medic, patient) = (bot_at(&engine, 45., 60.), bot_at(&engine, 60., 60.));
    plan_tick(&mut engine, 500);
    let objective = |engine: &Engine<Vec<u8>>, id: u16| {
        let task = &engine.bot_registry.bots[&id].task;
        (task.objective, task.role_goal)
    };
    let (medic_task, patient_task) = (objective(&engine, medic), objective(&engine, patient));
    assert!(medic_task.0.is_some() && patient_task.0.is_some());
    down(&mut engine, patient, at(600));
    plan_tick(&mut engine, 1000);
    assert!(approach(&engine, medic).is_some());
    assert_eq!(
        objective(&engine, medic),
        medic_task,
        "kept while approaching"
    );
    place(&mut engine, medic, 58., 60.);
    plan_tick(&mut engine, 4000);
    engine.tick_bot_healing(at(4050), 1).unwrap();
    assert!(engine.bot_registry.bots[&patient].downed.is_none());
    assert_eq!(
        objective(&engine, patient).0,
        patient_task.0,
        "the revived bot resumes"
    );
    plan_tick(&mut engine, 4100);
    assert_eq!(ended(&engine), ["patient_revived"]);
    assert_eq!(
        objective(&engine, medic).0,
        medic_task.0,
        "the medic resumes"
    );
}
