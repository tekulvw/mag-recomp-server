//! Invented capture objectives and ground vehicles exercising the callback backend.
use mag_gameplay::{
    acquisition_atomic, acquisition_control, acquisition_gameplay, acquisition_ground_runtime,
    acquisition_objective_runtime, acquisition_score_rate, replication,
};
#[path = "support/acquisition_fixture.rs"]
mod support;

use acquisition_atomic::{EnteredState, Entry};
use acquisition_control::Catalog;
use acquisition_gameplay::{Effect, Fsp, Gameplay, Objective, OrderSource, Squad};
use acquisition_ground_runtime::{Pose, QualifiedRespawn, Runtime};
use acquisition_objective_runtime::{Assignment, State};
use acquisition_score_rate::Rates;
use replication::{CharacterPosition, ObjectiveControlDelta, ObjectiveState, ObjectiveStateActor};
use std::collections::BTreeMap;

const CAPTURE: u32 = 100;
const ARRIVAL: u32 = 101;

fn fixture() -> (Catalog, Gameplay, Runtime) {
    let (spawns, mut graph) = support::fixture(0);
    graph["unbound_calls"] =
        serde_json::json!([{"symbol":"Game_SetScoreBleed","arguments":[8,9,0]}]);
    let catalog = Catalog::from_json(&graph.to_string(), &spawns).unwrap();
    let ground = Runtime::from_creation(
        &serde_json::from_value(serde_json::json!({
            "index":4,"spawn_point_index":12,"spawn_point_token":301,
            "vehicle_type_token":5,"faction":0,"team":0,"modifier":0,
            "position":{"x":2.,"y":4.,"z":8.},"rotation":[0.,0.,0.,1.],
            "health_quarter_points":800,"initial_state":0
        }))
        .unwrap(),
        Some(9),
    )
    .unwrap();
    let mut objectives = BTreeMap::new();
    let captures: BTreeMap<_, _> = catalog.capture_spawn_bindings().collect();
    for token in catalog.objective_tokens() {
        let state = match token {
            CAPTURE => ObjectiveState::Activated,
            102 => ObjectiveState::Completed,
            _ => ObjectiveState::Inactive,
        };
        objectives.insert(
            token,
            Objective {
                state: State {
                    current: state as i32,
                    target: state as i32,
                },
                owner_team: 1,
                activation_score_value: if token == ARRIVAL { 600 } else { 0 },
                current_attacker: u32::MAX,
                current_defender: u32::MAX,
                replication: ObjectiveControlDelta {
                    object_index: token as u16,
                    state_actor: ObjectiveStateActor {
                        state,
                        actor_index: u16::MAX,
                        change_time_ms: 4,
                    },
                    expiration_time_ms: 120,
                    deactivation_time_ms: 50,
                    health_percent: 50,
                },
                fsp: captures.get(&token).map(|&spawn_token| Fsp {
                    vehicle: acquisition_ground_runtime::Key {
                        index: 4 + (token - CAPTURE) as u16 / 2,
                        generation: ground.get(4).unwrap().key.generation,
                    },
                    source: QualifiedRespawn {
                        spawn_point_index: 12 + (token - CAPTURE) as u16 / 2,
                        spawn_point_token: spawn_token,
                        active: false,
                        pose: Pose {
                            position: CharacterPosition {
                                x: 20.,
                                y: 4.,
                                z: 8.,
                            },
                            rotation: [0., 0., 0., 1.],
                        },
                    },
                }),
            },
        );
    }
    let squads = BTreeMap::from([(
        0,
        Squad {
            team: 0,
            gate_value: 0,
            spawn_resolved: true,
            normal: Some(vec![
                OrderSource {
                    objective_token: Some(ARRIVAL),
                    order: 0,
                },
                OrderSource {
                    objective_token: Some(CAPTURE),
                    order: 1,
                },
            ]),
            frago: vec![OrderSource {
                objective_token: Some(102),
                order: 1,
            }],
            manual_tag: 0x1234_5678,
            assignment: Assignment {
                primary_index: u16::MAX,
                tagged_object: u32::MAX,
            },
        },
    )]);
    let gameplay = Gameplay::new(&catalog, objectives, squads).unwrap();
    (catalog, gameplay, ground)
}

#[test]
fn completed_entry_respawns_before_volume_and_assigns_from_current_state() {
    let (catalog, mut gameplay, mut ground) = fixture();
    let key = ground.get(4).unwrap().key;
    let mut rates = Rates::default();
    let mut effects = Vec::new();
    let publication = gameplay
        .apply_entry(
            &catalog,
            &Entry {
                objective_token: CAPTURE,
                state: EnteredState::Completed,
                actor_index: 2,
                timestamp_ms: 100,
            },
            &mut ground,
            &mut rates,
            |ground, effect| {
                assert_eq!(ground.get(4).unwrap().state, 2);
                effects.push(match effect {
                    Effect::Blockers(spawned) => {
                        assert_eq!(spawned, key);
                        "blockers"
                    }
                    Effect::Volume {
                        target,
                        arguments,
                        damage_type_symbol,
                        damage_source_symbol,
                    } => {
                        assert_eq!(target, 200);
                        assert_eq!(arguments, [0, 0, 0, 0]);
                        assert_eq!(damage_type_symbol, "InventedDamage");
                        assert_eq!(damage_source_symbol, "InventedSource");
                        "volume"
                    }
                    unexpected => panic!("unexpected effect {unexpected:?}"),
                });
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(effects, ["blockers", "volume"]);
    let capture = gameplay.objective(CAPTURE).unwrap();
    assert!(capture.fsp.unwrap().source.active);
    assert_eq!(capture.replication.state_actor.actor_index, 2);
    assert_eq!(capture.replication.health_percent, 0);
    assert_eq!(capture.replication.deactivation_time_ms, 0);
    assert_eq!(capture.replication.expiration_time_ms, 120);
    assert_eq!(
        gameplay.objective(ARRIVAL).unwrap().state,
        State {
            current: 0,
            target: 1
        }
    );
    assert_eq!(
        gameplay.squad(0).unwrap().assignment,
        Assignment {
            primary_index: CAPTURE as u16,
            tagged_object: 0x1234_5678,
        }
    );
    assert_eq!(publication.assignments.len(), 1);
    assert_eq!(rates.rates()[0].deadline_ms, 8100);
}

#[test]
fn escort_death_requests_deactivation_without_committing_current_state() {
    let (catalog, mut gameplay, mut ground) = fixture();
    let mut rates = Rates::default();
    let mut entry = Entry {
        objective_token: CAPTURE,
        state: EnteredState::Completed,
        actor_index: 2,
        timestamp_ms: 100,
    };
    gameplay
        .apply_entry(&catalog, &entry, &mut ground, &mut rates, |_, _| Ok(()))
        .unwrap();
    let objective_id = gameplay.objective(CAPTURE).unwrap().object_id();
    let defender = 0x0380_0005;
    gameplay
        .request_forced_fsp_deactivation(objective_id, defender, 1)
        .unwrap();
    let objective = gameplay.objective(CAPTURE).unwrap();
    assert_eq!(
        objective.state,
        State {
            current: 7,
            target: 1
        }
    );
    assert_eq!(objective.current_defender, defender);
    assert!(ground.get(4).unwrap().spawn_cycle_enabled);

    entry.state = EnteredState::Deactivated;
    entry.actor_index = 5;
    entry.timestamp_ms = 110;
    let publication = gameplay
        .apply_entry(
            &catalog,
            &entry,
            &mut ground,
            &mut rates,
            |ground, effect| {
                assert!(!ground.get(4).unwrap().spawn_cycle_enabled);
                assert!(matches!(effect, Effect::Volume { .. }));
                Ok(())
            },
        )
        .unwrap();
    let objective = gameplay.objective(CAPTURE).unwrap();
    assert_eq!(objective.current_attacker, u32::MAX);
    assert_eq!(objective.current_defender, defender);
    assert!(!objective.fsp.unwrap().source.active);
    assert_eq!(publication.fsp_active, Some((12, false)));
    assert_eq!(publication.objective.health_percent, 100);
    assert_eq!(publication.objective.expiration_time_ms, 0);
    assert_eq!(
        ground.get(4).unwrap().associated_objective,
        Some(objective_id)
    );
}

#[test]
fn arrival_deducts_authored_score_before_graph_effects_and_locks_capture_target() {
    let (catalog, mut gameplay, mut ground) = fixture();
    let mut rates = Rates::default();
    let mut effects = Vec::new();
    gameplay
        .request_state(ARRIVAL, ObjectiveState::Completed)
        .unwrap();
    let publication = gameplay
        .apply_entry(
            &catalog,
            &Entry {
                objective_token: ARRIVAL,
                state: EnteredState::Completed,
                actor_index: 9,
                timestamp_ms: 77,
            },
            &mut ground,
            &mut rates,
            |_, effect| {
                effects.push(match effect {
                    Effect::ScoreDeduction {
                        team: 1,
                        amount: 600,
                    } => "score",
                    Effect::Volume { target: 200, .. } => "volume",
                    unexpected => panic!("unexpected effect {unexpected:?}"),
                });
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(effects, ["score", "volume"]);
    assert_eq!(
        publication.objective.state_actor.state,
        ObjectiveState::Completed
    );
    assert_eq!(
        gameplay.objective(CAPTURE).unwrap().state,
        State {
            current: 3,
            target: 9
        }
    );
}

#[test]
fn manual_tag_uses_the_normal_acquisition_assignment_refresh() {
    let (_, mut game, _) = fixture();
    game.squad_mut(0).unwrap().normal = Some(vec![OrderSource {
        objective_token: Some(CAPTURE),
        order: 0,
    }]);
    let authored = game.set_manual_tag(0, u32::MAX).unwrap();
    assert_eq!(authored.tagged_object, 0x0c80_0066);
    assert_eq!(authored.primary_index, CAPTURE as u16);
    const TARGET: u32 = 0x0100_0004;
    let manual = game.set_manual_tag(0, TARGET).unwrap();
    assert_eq!(manual.primary_index, authored.primary_index);
    assert_eq!(manual.tagged_object, TARGET);
    assert_eq!(game.set_manual_tag(0, u32::MAX).unwrap(), authored);

    game.squad_mut(0).unwrap().spawn_resolved = false;
    assert_eq!(game.set_manual_tag(0, TARGET).unwrap(), authored);
    assert_eq!(game.squad(0).unwrap().manual_tag, TARGET);
    game.squad_mut(0).unwrap().spawn_resolved = true;
    game.refresh_assignments().unwrap();
    assert_eq!(game.squad(0).unwrap().assignment, manual);

    game.squad_mut(0).unwrap().frago[0].objective_token = Some(u32::MAX);
    assert_eq!(
        game.set_manual_tag(0, u32::MAX),
        Err("unknown_acquisition_gameplay_order_objective")
    );
    let squad = game.squad(0).unwrap();
    assert_eq!(squad.manual_tag, TARGET);
    assert_eq!(squad.assignment, manual);
}
