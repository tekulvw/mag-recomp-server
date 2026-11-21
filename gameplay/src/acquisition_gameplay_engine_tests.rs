//! Synthetic Engine fixture for capture, FSP activation, and callback rollback.
use super::*;
use crate::{
    acquisition_control, acquisition_gameplay,
    acquisition_gameplay_engine::Setup,
    acquisition_ground_runtime as ground,
    acquisition_objective_runtime::{Assignment, State},
    acquisition_round::Rules,
    objective_control::ObjectiveActor,
    objective_range::TriggerBox,
    replication::{ObjectiveControlDelta, ObjectiveStateActor},
};
use std::collections::BTreeMap;

const FIRST_CAPTURE: u16 = 100;
#[path = "acquisition_hold_exit_tests.rs"]
mod hold_exits;

pub(crate) fn capture_fixture() -> Engine<Vec<u8>> {
    configured_capture_fixture(None, None)
}

pub(crate) fn automatic_repair_fixture(
    catalog: crate::leadership_passives::LeadershipCatalog,
    policy: crate::leadership_runtime::RolePolicy,
) -> Engine<Vec<u8>> {
    let mut engine = capture_fixture()
        .with_development_control()
        .with_leadership_passives(catalog, policy)
        .unwrap();
    start_capture(&mut engine, 0, 100);
    engine
        .tick_acquisition_objectives(Duration::from_millis(3200), 2)
        .unwrap();
    engine.tick_acquisition_ground_registration(2).unwrap();
    let actor = engine.active.as_mut().unwrap();
    actor.ground_seat = Some(crate::ground_seats::PassengerSeat {
        vehicle_index: 4,
        seat: 0,
    });
    actor.ground_vehicle_batch = 1;
    actor.owner_fields.state = Some(crate::replication::CharacterState {
        state: 14,
        stance: 0,
        user_data: [1, 4, 0],
    });
    engine.enable_world_ticks();
    engine
}

fn configured_capture_fixture(
    ground_damage: Option<crate::acquisition_ground_damage::Policy>,
    arrival_static: Option<ground_vehicles::arrival_static::ArrivalStatic>,
) -> Engine<Vec<u8>> {
    capture_fixture_with_reinforcements(ground_damage, arrival_static, false)
}

fn capture_fixture_with_reinforcements(
    ground_damage: Option<crate::acquisition_ground_damage::Policy>,
    arrival_static: Option<ground_vehicles::arrival_static::ArrivalStatic>,
    reinforcements: bool,
) -> Engine<Vec<u8>> {
    let (engine, setup) =
        acquisition_setup_fixture(ground_damage, arrival_static, reinforcements, 4);
    engine.with_acquisition_gameplay(setup).unwrap()
}

fn acquisition_setup_fixture(
    ground_damage: Option<crate::acquisition_ground_damage::Policy>,
    arrival_static: Option<ground_vehicles::arrival_static::ArrivalStatic>,
    reinforcements: bool,
    qualified_drivers: u16,
) -> (Engine<Vec<u8>>, Setup) {
    let mut e = acquisition_entry::acquisition();
    let mut spawn_input = acquisition_entry::acquisition_spawns();
    spawn_input["match_profile"]["has_objectives"] = json!(true);
    spawn_input["match_profile"]["maximum_points"] = json!([1200, 1000]);
    spawn_input["maps"][0]["spawn_count"] = json!(if reinforcements { 22 } else { 20 });
    if reinforcements {
        spawn_input["active"]["available"]
            .as_array_mut()
            .unwrap()
            .push(json!(20));
        spawn_input["maps"][0]["squads"][0]["available"]
            .as_array_mut()
            .unwrap()
            .push(json!(20));
    }
    let locations = spawn_input["maps"][0]["locations"].as_array_mut().unwrap();
    let mut vehicles = Vec::new();
    let mut vehicle_sources = Vec::new();
    let mut driver_seats = Vec::new();
    let mut chains = Vec::new();
    let mut groups = Vec::new();
    let mut volumes = BTreeMap::new();
    let mut arrival_triggers = BTreeMap::new();
    for chain in 0..4_u16 {
        let point = 16 + chain;
        let spawn_token = 301 + u32::from(chain);
        let index = 4 + chain;
        let capture = FIRST_CAPTURE + chain * 2;
        let arrival = capture + 1;
        let volume = 200 + u32::from(chain);
        let x = 1000. + f32::from(chain) * 30.;
        let position = json!({"x":x,"y":3.,"z":1000.});
        locations.push(
            json!({"index":point,"token":spawn_token,"kind":"unsupported",
            "active":false,"position":position}),
        );
        vehicles.push(
            json!({"index":index,"spawn_point_index":point,"spawn_point_token":spawn_token,
            "vehicle_type_token":12345,"faction":1,"team":0,"modifier":0,"initial_state":0,
            "position":position,"rotation":[0.,0.,0.,1.],"health_quarter_points":600}),
        );
        if chain < qualified_drivers {
            let vehicle = vehicles.last_mut().unwrap();
            vehicle["passenger_seats"] = json!([2, 3]);
            vehicle["maximum_occupancy"] = json!(2);
            vehicle["maximum_mount_distance_m"] = json!(8.);
            vehicle["maximum_mount_position_age_ms"] = json!(750);
            vehicle["passenger_exit_candidates"] = json!([
                {"ranking_offset":[3.,0.,0.],"position_offset":[3.,-1.,0.]},
                {"ranking_offset":[-3.,0.,0.],"position_offset":[-3.,-1.,0.]}
            ]);
            driver_seats.push(json!({"vehicle_index":index,"seat":0,"maximum_speed_mps":25.}));
        }
        vehicle_sources.push(json!({"vehicle_index":index,"spawn_point_index":point,
            "spawn_point_token":spawn_token,"vehicle_type_token":12345,"spawn_type":9,"user_class":1,
            "source_family":35,"authored_active":false,"team":0}));
        chains.push(
            json!({"platoon":chain/2,"stage":if chain%2==0 {"a"} else {"b"},
            "capture_token":capture,"arrival_token":arrival,"vehicle_spawn_token":spawn_token,
            "volume_token":volume}),
        );
        let mut deactivation =
            vec![json!({"kind":"objective","target":arrival,"operation":"disable"})];
        let mut completion = vec![
            json!({"kind":"objective","target":capture,"operation":"lock"}),
            json!({"kind":"objective","target":arrival,"operation":"lock"}),
        ];
        if chain % 2 == 0 {
            deactivation
                .push(json!({"kind":"objective","target":capture + 2,"operation":"disable"}));
            completion.push(json!({"kind":"objective","target":capture + 2,"operation":"enable"}));
        }
        groups.push(json!({"branches":[
            {"condition":{"objective_token":arrival,"state":"completed"},"commands":completion},
            {"condition":{"objective_token":capture,"state":"completed"},"commands":[
                {"kind":"objective","target":arrival,"operation":"enable"},
                {"kind":"damage_volume","target":volume,"arguments":[0,0,0,0],
                    "damage_type_symbol":"DamageType.ExternalKinetic",
                    "damage_source_symbol":"DamageSourceType.eDamageSourceTypeCrushed"}]},
            {"condition":{"objective_token":capture,"state":"deactivated"},"commands":deactivation}
        ]}));
        volumes.insert(
            volume,
            QueryBox {
                pose: Pose::at([500., 5., 500.]),
                half_extents: [1.; 3],
                radius: f64::from(0.05_f32),
            },
        );
        arrival_triggers.insert(
            u32::from(arrival),
            TriggerBox {
                centre: [x, 3., 1040.],
                axes: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
                half_extents: [3.; 3],
            },
        );
    }
    let mut passenger_spawns = Vec::new();
    if reinforcements {
        for (index, point, token, spawn_type, user_class) in
            [(0, 20, 320, 3, 0), (8, 21, 321, 7, 1)]
        {
            let apc = spawn_type == 3;
            let position = json!({"x":1200. + f32::from(point as u16)*10.,"y":3.,"z":1000.});
            locations.push(json!({"index":point,"token":token,
                "kind":if apc { "vehicle_passenger" } else { "unsupported" },"position":position}));
            let mut creation = json!({"index":index,"spawn_point_index":point,"spawn_point_token":token,
                "vehicle_type_token":12345,"faction":1,"team":0,"modifier":0,
                "position":position,"rotation":[0.,0.,0.,1.],"health_quarter_points":600});
            if apc {
                creation["passenger_seats"] = json!([2, 3]);
                creation["maximum_occupancy"] = json!(2);
                creation["passenger_exit_candidates"] = json!([
                    {"ranking_offset":[3.,0.,0.],"position_offset":[3.,-1.,0.]}]);
                passenger_spawns.push(json!({"vehicle_index":index,
                    "spawn_point_index":point,"spawn_point_token":token}));
            }
            vehicles.push(creation);
            vehicle_sources.push(json!({"vehicle_index":index,"spawn_point_index":point,
                "spawn_point_token":token,"vehicle_type_token":12345,"spawn_type":spawn_type,
                "user_class":user_class,"source_family":35,"authored_active":true,"team":0}));
        }
    }
    let spawns = super::super::spawns(spawn_input).unwrap();
    let spawn_sha256 = spawns.source_sha256().unwrap();
    let ground_catalog = crate::ground_vehicles::Catalog::from_json(
        &json!({
            "format":"mag_ground_vehicles","schema_version":1,
            "source":{"spawn_catalog_sha256":spawn_sha256,"native_elf_sha256":"a".repeat(64),
                "native_binding_sha256":"b".repeat(64),"vehicle_properties_sha256":"c".repeat(64),
                "creation_scope":"native_vehicle_source_points"},
            "vehicles":vehicles,"vehicle_sources":vehicle_sources,"driver_seats":driver_seats,
            "passenger_spawns":passenger_spawns
        })
        .to_string(),
        &spawns,
    )
    .unwrap();
    let graph = acquisition_control::Catalog::from_json(&json!({
        "format":"mag_acquisition_control","schema_version":1,"controller_token":999,
        "source":{"map_name":"synthetic_arena","spawn_catalog_sha256":spawn_sha256,
            "game_archive_sha256":"a".repeat(64),"script_sha256":"b".repeat(64),"graph_proof_sha256":"c".repeat(64)},
        "chains":chains,"groups":groups,"unbound_calls":[{"symbol":"Game_SetScoreBleed","arguments":[8,1,0]}]
    }).to_string(), &spawns).unwrap();
    e.spawn_catalog = Some(spawns);
    e = e
        .with_dynamic_collision_bodies(
            Catalog::from_json(&support::fixture().to_string()).unwrap(),
            RuntimeBindings {
                character_template: "invented-character".into(),
                vehicles: vec![crate::dynamic_collision_runtime::VehicleBinding {
                    vehicle_type_token: 12345,
                    faction: 1,
                    template: "invented-vehicle".into(),
                }],
            },
        )
        .unwrap()
        .with_ground_vehicles(ground_catalog)
        .unwrap()
        .with_acquisition_static_partitions(vec![static_partition("synthetic_arena")])
        .unwrap();

    let captures: BTreeMap<_, _> = graph.capture_spawn_bindings().collect();
    let mut objectives = BTreeMap::new();
    let mut objective_rows = Vec::new();
    for index in FIRST_CAPTURE..FIRST_CAPTURE + 8 {
        let token = u32::from(index);
        let fsp = captures.get(&token).map(|&spawn_point_token| {
            let vehicle = e
                .ground_runtime
                .as_ref()
                .unwrap()
                .get(4 + (index - FIRST_CAPTURE) / 2)
                .unwrap();
            let current = e.current_ground_vehicle(vehicle.key.index).unwrap();
            acquisition_gameplay::Fsp {
                vehicle: vehicle.key,
                source: ground::QualifiedRespawn {
                    spawn_point_index: current.spawn_point_index,
                    spawn_point_token,
                    active: false,
                    pose: ground::Pose {
                        position: current.position,
                        rotation: current.rotation,
                    },
                },
            }
        });
        let activation_score_value = if fsp.is_some() {
            0
        } else if index % 4 == 1 {
            600
        } else {
            400
        };
        objectives.insert(
            token,
            acquisition_gameplay::Objective {
                state: State {
                    current: 0,
                    target: 0,
                },
                owner_team: 1,
                activation_score_value,
                current_attacker: u32::MAX,
                current_defender: u32::MAX,
                replication: ObjectiveControlDelta {
                    object_index: index,
                    state_actor: ObjectiveStateActor {
                        state: ObjectiveState::Inactive,
                        actor_index: u16::MAX,
                        change_time_ms: 0,
                    },
                    expiration_time_ms: 0,
                    deactivation_time_ms: 0,
                    health_percent: 100,
                },
                fsp,
            },
        );
        let mut row = json!({"token":format!("0x{token:08x}"),"object_index":index,"initial_state":0,
            "scoring":{"owner_team":1,"activation_score":activation_score_value,"deactivation_score":0}});
        if fsp.is_some() {
            row["interaction"] = json!({"owner_team":1,"activate_ms":3000,"complete_ms":0,
                "deactivate_ms":3000,"can_deactivate":true,"position":[100.,3.,100.],
                "trigger_count":1,"maximum_distance_m":2.,"distance_policy":"development_point_distance"});
        }
        objective_rows.push(row);
    }

    let mut squads = BTreeMap::new();
    let mut squad_rows = Vec::new();
    for team in 0..2_u8 {
        for slot in 0..8_u16 {
            let squad_index = u16::from(team) * 16 + slot;
            let first = FIRST_CAPTURE + 4 * (slot / 4);
            let orders: Vec<_> = (first..first + 4)
                .map(|index| acquisition_gameplay::OrderSource {
                    objective_token: Some(u32::from(index)),
                    order: i32::from(team),
                })
                .collect();
            squad_rows.push(
                json!({"team":team,"squad_index":squad_index,"initial_objective_index":u16::MAX,
                "authored_objectives":orders.iter().map(|order| json!({
                    "object_index":order.objective_token.unwrap(),"order":order.order
                })).collect::<Vec<_>>()}),
            );
            squads.insert(
                squad_index,
                acquisition_gameplay::Squad {
                    team,
                    gate_value: 0,
                    spawn_resolved: false,
                    normal: Some(orders.clone()),
                    frago: orders,
                    manual_tag: u32::MAX,
                    assignment: Assignment {
                        primary_index: u16::MAX,
                        tagged_object: u32::MAX,
                    },
                },
            );
        }
    }
    objective_rows.push(json!({"token":"0x000000fe","object_index":120,"initial_state":1}));
    let objective_catalog = ObjectiveCatalog::from_json(&json!({
        "schema_version":1,"format":"mag_retail_objective_catalog","application_id":20231,
        "protocol":protocol::ProtocolInfo::RETAIL_PROTOCOL,"game_type":1,"map_name":"synthetic_arena",
        "entity_family":12,"network_type":12,"objects":objective_rows,"squads":squad_rows,
        "source":{"authored_catalog_sha256":"0".repeat(64),"binding_capture_sha256":"1".repeat(64),
            "startup_proof_sha256":"2".repeat(64),"source_image_sha256":"3".repeat(64),
            "capture_qualification":"native_source_gameplay"}
    }).to_string()).unwrap();
    let state = acquisition_gameplay::Gameplay::new(&graph, objectives, squads).unwrap();
    let mut filter_rows = [0; 32];
    filter_rows[6] = (1 << 6) | (1 << 21) | (1 << 22);
    filter_rows[21] = 1 << 6;
    filter_rows[22] = 1 << 6;
    filter_rows[24] = 1 << 9;
    let ground_capabilities = if arrival_static.is_some() {
        (4..8)
            .map(|index| {
                (
                    index,
                    ground::Capabilities {
                        delivered_inert_seconds: Some(12.),
                        ..Default::default()
                    },
                )
            })
            .collect()
    } else {
        Vec::new()
    };
    let vehicle_reinforcement_intervals_ms = e.match_profile().spawn_wave_ms;
    (
        e.with_objective_catalog(objective_catalog).unwrap(),
        Setup {
            controller: graph,
            state,
            rules: Rules {
                starting_points: [1200, 1000],
                major_percent: 50,
                end_delay_ms: 2000,
            },
            filter_rows,
            physics: Default::default(),
            volumes,
            initial_active: captures
                .keys()
                .copied()
                .filter(|idx| idx % 4 == u32::from(FIRST_CAPTURE) % 4)
                .collect(),
            arrival_triggers,
            arrival_static,
            ground_capabilities,
            vehicle_reinforcement_intervals_ms,
            structural: None,
            ground_damage,
            frag_character_blast: false,
        },
    )
}

#[test]
fn acquisition_rejects_missing_escort_driver_bindings_before_setup_installation() {
    for count in [0, 3] {
        let (engine, setup) = acquisition_setup_fixture(None, None, false, count);
        let vehicles = engine.ground_vehicles.as_ref().unwrap();
        assert_eq!(
            setup.validate_ground_drivers(vehicles),
            Err("acquisition_escort_driver_policy_required")
        );
        assert!(matches!(
            engine.with_acquisition_gameplay(setup),
            Err("acquisition_escort_driver_policy_required")
        ));
    }
    let (engine, setup) = acquisition_setup_fixture(None, None, false, 4);
    assert!(
        setup
            .validate_ground_drivers(engine.ground_vehicles.as_ref().unwrap())
            .is_ok()
    );
    assert!(engine.with_acquisition_gameplay(setup).is_ok());
}

#[test]
fn acquisition_rejects_native_structure_without_its_effect_provider() {
    let (mut engine, setup) = acquisition_setup_fixture(None, None, false, 4);
    let catalog = engine.objective_catalog.as_mut().unwrap();
    let structure = catalog
        .objects
        .iter_mut()
        .find(|o| o.object_index == 120)
        .unwrap();
    structure.native_class = Some(crate::objective_catalog::NativeClass::Structural);

    assert!(matches!(
        engine.with_acquisition_gameplay(setup),
        Err("acquisition_native_structural_provider_required")
    ));
}

fn prepare_capture_actor(e: &mut Engine<Vec<u8>>, player: u16, epoch_ms: u64) {
    reserve(e, player);
    let active = e.active.as_mut().unwrap();
    ready(active);
    active.life = CharacterLife::Alive {
        generation: 1,
        health_quarter_points: 400,
    };
    e.match_started = true;
    e.match_epoch = Some(Duration::from_millis(epoch_ms));
    e.tick_acquisition_objectives(Duration::from_millis(epoch_ms), 1)
        .unwrap();
    let world = e.objective_world.as_mut().unwrap();
    assert_eq!(
        world.state(FIRST_CAPTURE),
        Some(ObjectiveState::Deactivated)
    );
    assert_eq!(primary_assignment(e), FIRST_CAPTURE);
}

fn start_capture(e: &mut Engine<Vec<u8>>, player: u16, epoch_ms: u64) {
    prepare_capture_actor(e, player, epoch_ms);
    let position = active_position(e);
    let outcome = e.objective_world.as_mut().unwrap().interact(
        protocol::ObjectiveInteraction {
            kind: protocol::ObjectiveInteractionKind::Activate,
            target_object_id: 0x0c80_0000 | u32::from(FIRST_CAPTURE),
            actor_index: player,
            trigger_index: 0,
        },
        ObjectiveActor {
            player,
            team: 0,
            generation: 1,
            position,
        },
        Duration::from_millis(epoch_ms + 100),
    );
    assert!(outcome.accepted, "{}", outcome.reason);
}

fn primary_assignment(e: &Engine<Vec<u8>>) -> u16 {
    let squad = e.active.as_ref().unwrap().squad_catalog.squad_index();
    e.acquisition_gameplay
        .as_ref()
        .unwrap()
        .state
        .squad(squad)
        .unwrap()
        .assignment
        .primary_index
}

fn active_position(e: &Engine<Vec<u8>>) -> CharacterPosition {
    e.active.as_ref().unwrap().tracking.sample.unwrap().position
}

#[test]
fn precision_completes_an_unhooked_acquisition_structure_only_after_recording() {
    let mut engine = capture_fixture();
    start_capture(&mut engine, 0, 100);
    let world = engine.objective_world.as_mut().unwrap();
    let source = crate::structure_damage::Source {
        player: 0,
        generation: 1,
    };
    assert!(world.stage_precision_strike(120, Some(source)).unwrap());
    let scores = engine.acquisition_round.as_ref().unwrap().points();
    let max_log = engine.max_log;
    engine.max_log = engine.written;
    assert_eq!(
        engine
            .tick_acquisition_objectives(Duration::from_millis(400), 2)
            .err(),
        Some("log_limit_reached")
    );
    assert_eq!(
        engine.objective_world.as_ref().unwrap().state(120),
        Some(ObjectiveState::Deactivated)
    );
    engine.max_log = max_log;
    engine.failed = false;
    engine
        .tick_acquisition_objectives(Duration::from_millis(400), 2)
        .unwrap();

    let world = engine.objective_world.as_ref().unwrap();
    let row = world
        .snapshot(1, 400)
        .objectives
        .into_iter()
        .find(|row| row.object_index == 120)
        .unwrap();
    assert_eq!(row.state_actor.state, ObjectiveState::Completed);
    assert_eq!(row.state_actor.actor_index, 0);
    assert_eq!(row.health_percent, 0);
    assert_eq!(world.state(FIRST_CAPTURE), Some(ObjectiveState::Activating));
    assert_eq!(engine.acquisition_round.as_ref().unwrap().points(), scores);
}

#[test]
fn engine_capture_callback_activates_existing_escort_and_starts_score_rate() {
    let mut e = capture_fixture();
    start_capture(&mut e, 0, 100);
    let hidden = e.ground_runtime.as_ref().unwrap().get(4).unwrap().key;
    assert_eq!(e.ground_runtime.as_ref().unwrap().get(4).unwrap().state, 0);
    e.tick_acquisition_objectives(Duration::from_millis(3200), 2)
        .unwrap();
    assert_eq!(
        e.objective_world.as_ref().unwrap().state(FIRST_CAPTURE),
        Some(ObjectiveState::Completed)
    );
    assert_eq!(primary_assignment(&e), FIRST_CAPTURE + 1);
    e.tick_acquisition_ground_registration(2).unwrap();
    let truck = e.ground_runtime.as_ref().unwrap().get(4).unwrap();
    assert_eq!(truck.key, hidden);
    assert_eq!(truck.state, 2);
    assert_eq!(truck.body, ground::Body::Operational);
    let mut round = e.acquisition_round.clone().unwrap();
    assert_eq!(round.points(), [1200, 1000]);
    assert_eq!(round.update_bleed(8100), [None, None]);
    assert_eq!(round.update_bleed(8101), [Some(1199), None]);
}

#[test]
fn engine_capture_callback_log_failure_preserves_objective_and_escort_state() {
    let mut e = capture_fixture();
    start_capture(&mut e, 0, 100);
    let ground = e.ground_runtime.clone();
    let round = e.acquisition_round.clone();
    e.max_log = e.written;
    assert_eq!(
        e.tick_acquisition_objectives(Duration::from_millis(3200), 2)
            .err(),
        Some("log_limit_reached")
    );
    assert_eq!(e.ground_runtime, ground);
    assert_eq!(e.acquisition_round, round);
    assert_eq!(
        e.objective_world.as_ref().unwrap().state(FIRST_CAPTURE),
        Some(ObjectiveState::Activating)
    );
}

fn claim_capture_actor(engine: &mut Engine<Vec<u8>>) {
    let actor = engine.active.as_mut().unwrap();
    actor.claimed = Some(Claimed {
        wire: Vec::new(),
        np_identity: [0; 36],
        game_info: protocol::GameInfo {
            client_index: actor.reservation.player,
            level_name: "synthetic_arena".into(),
            start_time_ms: 0,
            attacking_faction: 1,
            defending_faction: 0,
            maximum_points: [1200, 1000],
            current_points: [1200, 1000],
            game_type: 1,
        },
    });
}

#[test]
fn late_ordinary_capture_increments_vehicles_captured_once_after_recording() {
    let mut engine = capture_fixture();
    prepare_capture_actor(&mut engine, 0, 100);
    claim_capture_actor(&mut engine);
    assert!(progression::RewardRecipient::from_active(engine.active.as_ref().unwrap()).is_none());
    crate::objective_reward_tests::interact(
        &mut engine,
        FIRST_CAPTURE,
        protocol::ObjectiveInteractionKind::Activate,
        200,
    )
    .unwrap();
    let limit = engine.max_log;
    engine.max_log = engine.written;
    let due = Duration::from_millis(3200);
    assert_eq!(
        engine.tick_acquisition_objectives(due, 2).unwrap_err(),
        "log_limit_reached"
    );
    assert!(
        !engine
            .active
            .as_ref()
            .unwrap()
            .reward_statistics
            .contains_key(&58)
    );

    engine.failed = false;
    engine.max_log = limit;
    engine.tick_acquisition_objectives(due, 2).unwrap();
    engine.tick_acquisition_objectives(due, 3).unwrap();
    assert_eq!(
        engine.active.as_ref().unwrap().reward_statistics.get(&58),
        Some(&1)
    );
}

#[test]
fn accountless_capture_cannot_credit_a_reused_player_slot() {
    let mut engine = capture_fixture();
    let catalog = engine.objective_catalog.as_mut().unwrap();
    catalog
        .objects
        .iter_mut()
        .find(|object| object.object_index == FIRST_CAPTURE)
        .unwrap()
        .interaction
        .as_mut()
        .unwrap()
        .complete_ms = 1000;
    engine.objective_world = Some(objective_control::ObjectiveWorld::new(catalog));
    prepare_capture_actor(&mut engine, 0, 100);
    claim_capture_actor(&mut engine);
    let original = engine.active.as_ref().unwrap();
    assert!(progression::RewardRecipient::from_active(original).is_none());
    let original_id = original.reservation.id.clone();
    crate::objective_reward_tests::interact(
        &mut engine,
        FIRST_CAPTURE,
        protocol::ObjectiveInteractionKind::Activate,
        200,
    )
    .unwrap();
    engine
        .tick_acquisition_objectives(Duration::from_millis(3200), 2)
        .unwrap();
    assert_eq!(
        engine
            .objective_world
            .as_ref()
            .unwrap()
            .state(FIRST_CAPTURE),
        Some(ObjectiveState::Activated)
    );

    super::super::reserve_faction(&mut engine, 1, 0);
    ready(engine.active.as_mut().unwrap());
    claim_capture_actor(&mut engine);
    let released = engine.handle(
        Request::Release {
            v: BRIDGE_VERSION,
            request_id: "release-accountless-capture".into(),
            reservation_id: original_id,
        },
        Duration::from_millis(3300),
        2,
    );
    assert!(released.ok, "{:?}", released.error);
    assert_eq!(
        engine
            .objective_world
            .as_ref()
            .unwrap()
            .state(FIRST_CAPTURE),
        Some(ObjectiveState::Activated)
    );

    let id = "capture-replacement-slot";
    let reserved = super::super::reserve_identity(&mut engine, 0, 1, id);
    assert!(reserved.ok, "{:?}", reserved.error);
    ready(engine.active.as_mut().unwrap());
    claim_capture_actor(&mut engine);
    let squad = engine.active.as_ref().unwrap().squad_catalog.squad_index();
    assert_eq!(
        engine
            .objective_counter_leader(squad)
            .unwrap()
            .reservation
            .id,
        id
    );
    engine
        .acquisition_gameplay
        .as_mut()
        .unwrap()
        .state
        .squad_mut(squad)
        .unwrap()
        .assignment
        .tagged_object = 0x0c80_0000 | u32::from(FIRST_CAPTURE);

    engine
        .tick_acquisition_objectives(Duration::from_millis(4200), 3)
        .unwrap();
    assert_eq!(
        engine
            .objective_world
            .as_ref()
            .unwrap()
            .state(FIRST_CAPTURE),
        Some(ObjectiveState::Completed)
    );
    let statistics = &engine.active.as_ref().unwrap().reward_statistics;
    assert_eq!((statistics.get(&58), statistics.get(&41)), (None, None));
}

#[test]
fn generic_escort_completion_does_not_increment_vehicles_captured() {
    let mut engine = capture_fixture();
    start_capture(&mut engine, 0, 100);
    claim_capture_actor(&mut engine);
    engine
        .acquisition_gameplay
        .as_mut()
        .unwrap()
        .state
        .request_state(u32::from(FIRST_CAPTURE), ObjectiveState::Completed)
        .unwrap();
    engine
        .tick_acquisition_objectives(Duration::from_millis(110), 2)
        .unwrap();
    assert_eq!(
        engine
            .objective_world
            .as_ref()
            .unwrap()
            .state(FIRST_CAPTURE),
        Some(ObjectiveState::Completed)
    );
    assert!(
        engine
            .ground_runtime
            .as_ref()
            .unwrap()
            .get(4)
            .unwrap()
            .spawn_cycle_enabled
    );
    assert!(
        !engine
            .active
            .as_ref()
            .unwrap()
            .reward_statistics
            .contains_key(&58)
    );
}

#[test]
fn engine_late_join_receives_current_capture_and_scores() {
    let mut e = capture_fixture();
    start_capture(&mut e, 0, 100);
    e.tick_acquisition_objectives(Duration::from_millis(3200), 2)
        .unwrap();
    e.tick_acquisition_ground_registration(2).unwrap();
    e.tick_match_clock(Duration::from_millis(8101), 3).unwrap();

    reserve(&mut e, 1);
    let mut claim = e.wire_profile.wire_message_type(3).to_be_bytes().to_vec();
    // 36-byte NP identity, four empty length-prefixed strings and zero IP.
    claim.extend([0; 48]);
    dispatch(&mut e, &claim, 8200).unwrap();
    let late = e.active.as_mut().unwrap();
    assert_eq!(late.reservation.player, 1);
    assert_eq!(
        late.claimed.as_ref().unwrap().game_info.current_points,
        [1199, 1000]
    );
    ready(late);
    late.objective_delta = None;

    // An owner update first replays current Escort creation, then snapshots the
    // live objective world for this new recipient.
    for now_ms in 8201..8210 {
        owner_update(&mut e, now_ms, 0);
        if e.active.as_ref().unwrap().objective_delta.is_some() {
            break;
        }
    }
    let wire = e.active.as_ref().unwrap().objective_delta.as_ref().unwrap();
    let mut bits = BitReader::with_profile(wire, e.wire_profile);
    bits.position = e.wire_profile.header_bits();
    assert_eq!(bits.read(1).unwrap(), 1);
    assert_eq!(bits.read(16).unwrap(), u32::from(FIRST_CAPTURE));
    assert_eq!(bits.read(1).unwrap(), 0); // field 0
    assert_eq!(bits.read(1).unwrap(), 0); // field 1
    assert_eq!(bits.read(1).unwrap(), 1); // state/actor field
    assert_eq!(bits.read(8).unwrap(), ObjectiveState::Completed as u32);
}

#[test]
fn engine_release_requeue_restarts_native_capture_and_escort_cycle() {
    let mut e = capture_fixture();
    start_capture(&mut e, 0, 100);
    e.tick_acquisition_objectives(Duration::from_millis(3200), 2)
        .unwrap();
    e.tick_acquisition_ground_registration(2).unwrap();
    assert!(e.ground_vehicle_is_operational(4));

    // Synthetic depletion isolates requeue from the separate delivery trigger
    // path. The normal match-clock and release paths own the remaining state.
    e.acquisition_round
        .as_mut()
        .unwrap()
        .apply_score(crate::acquisition_round::ScoreIntent {
            team: 1,
            delta: -1000,
        })
        .unwrap();
    e.tick_match_clock(Duration::from_millis(3300), 3).unwrap();
    e.tick_match_clock(Duration::from_millis(5300), 4).unwrap();
    assert!(e.round.outcome.as_ref().unwrap().finalized);
    let released = e.handle(
        Request::Release {
            v: BRIDGE_VERSION,
            request_id: "synthetic-acquisition-round-release".into(),
            reservation_id: "synthetic-reservation-0000".into(),
        },
        Duration::from_millis(5301),
        4,
    );
    assert!(released.ok, "{:?}", released.error);
    reserve(&mut e, 1); // Rollover commits with successor admission.
    assert!(e.match_terminal.is_none());
    assert_eq!(e.acquisition_round.as_ref().unwrap().points(), [1200, 1000]);
    assert_eq!(
        e.objective_world.as_ref().unwrap().state(FIRST_CAPTURE),
        Some(ObjectiveState::Inactive)
    );
    assert_eq!(e.ground_runtime.as_ref().unwrap().get(4).unwrap().state, 0);
    assert!(!e.ground_vehicle_is_operational(4));

    start_capture(&mut e, 1, 6000);
    e.tick_acquisition_objectives(Duration::from_millis(9100), 5)
        .unwrap();
    e.tick_acquisition_ground_registration(5).unwrap();
    assert_eq!(
        e.objective_world.as_ref().unwrap().state(FIRST_CAPTURE),
        Some(ObjectiveState::Completed)
    );
    assert!(e.ground_vehicle_is_operational(4));
    assert_eq!(primary_assignment(&e), FIRST_CAPTURE + 1);
}

pub(crate) fn ground_firearm_fixture(amount_hp: f32, friendly: bool) -> Engine<Vec<u8>> {
    let policy = serde_json::from_value(json!({
        "friendly_fire_multiplier": 0.5,
        "profiles":[{
            "vehicle_type_token":12345,"faction":1,"max_health":150.0,
            "critical_loss_percent_per_second":3.0,
            "armor":{"rating":0.0,"prevention":0.0,"coverage":[0.0,0.0,0.0,0.0]}
        }],
        "areas": vec![json!({"armor_area":0,"multiplier":1.0});16]
    }))
    .unwrap();
    let mut e = configured_capture_fixture(Some(policy), None);
    e.item_catalog = crate::human_kill_reward_tests::combat_items_for_ground_test(amount_hp);
    start_capture(&mut e, 0, 100);
    e.tick_acquisition_objectives(Duration::from_millis(3200), 2)
        .unwrap();
    e.tick_acquisition_ground_registration(2).unwrap();
    if !friendly {
        super::super::reserve_faction(&mut e, 1, 0);
    }
    let actor = e.active.as_mut().unwrap();
    assert_eq!(actor.team(), u8::from(!friendly));
    ready(actor);
    actor.life = CharacterLife::Alive {
        generation: 1,
        health_quarter_points: 400,
    };
    actor.current_inventory_handle = Some(replication::InventoryHandle {
        update_id: 1,
        inventory_index: 0,
    });
    actor.spawned_loadout =
        actor
            .reservation
            .character
            .replication_loadout(&e.item_catalog, 0, actor.wire_profile);
    actor.observed_primary = Some(replication::InventoryItem {
        inventory_update_id: 1,
        gear_slot: 0,
        ammo_pool: 0,
        ammo_count: 9,
        reloading: false,
    });
    actor.primary_equipped = true;
    e
}

fn ground_hit(e: &mut Engine<Vec<u8>>, now_ms: u64) -> Result<Vec<OutboundMessage>, &'static str> {
    let mut bytes = [0; bullet_hit::WIRE_LEN];
    bytes[..2].copy_from_slice(&bullet_hit::MESSAGE_ID.to_be_bytes());
    bytes[4] = 2;
    bytes[5..9].copy_from_slice(&7u32.to_be_bytes());
    bytes[9..13].copy_from_slice(&0x0100_0004u32.to_be_bytes());
    bytes[37..39].copy_from_slice(&1000u16.to_be_bytes());
    dispatch(e, &bytes, now_ms)
}

#[test]
fn engine_firearm_reports_damage_escort_and_pending_death_exactly_once() {
    let mut e = ground_firearm_fixture(100.125, false);
    ground_hit(&mut e, 3300).unwrap();
    let vehicle = e.ground_runtime.as_ref().unwrap().get(4).unwrap();
    let damage = vehicle.damage_wire();
    assert_eq!(damage.health_quarter_points, 199);
    assert_eq!(damage.conditions, 3);
    assert_eq!(damage.last_location_and_type, 0x12);
    assert_eq!(damage.last_attacker, 0x0380_0001);
    assert_eq!(damage.last_user_data, 7);
    assert!(!vehicle.pending_death);

    // Native hit reports carry no shot nonce. Identical authenticated hit
    // bytes can represent a later shot while the target remains operational.
    ground_hit(&mut e, 3400).unwrap();
    let dead = e.ground_runtime.clone();
    let vehicle = dead.as_ref().unwrap().get(4).unwrap();
    assert_eq!(vehicle.damage_wire().conditions, 15);
    assert_eq!(vehicle.health_quarter_points, 0);
    assert!(vehicle.pending_death);
    assert!(!vehicle.vaporize_on_death);
    ground_hit(&mut e, 3500).unwrap();
    assert_eq!(e.ground_runtime, dead);
}

#[test]
fn engine_firearm_applies_team_multiplier_and_retains_subquarter_health() {
    let mut e = ground_firearm_fixture(0.25, true);
    for (now, quarters) in [(3300, 599), (3400, 599), (3500, 598)] {
        ground_hit(&mut e, now).unwrap();
        let vehicle = e.ground_runtime.as_ref().unwrap().get(4).unwrap();
        assert_eq!(vehicle.health_quarter_points, quarters);
    }
}

#[test]
fn engine_firearm_log_failure_does_not_install_ground_damage() {
    let mut e = ground_firearm_fixture(149.125, false);
    let before = e.ground_runtime.clone();
    let queued = e.active.as_ref().unwrap().pending_shared.len();
    e.max_log = e.written;
    assert_eq!(ground_hit(&mut e, 3300).err(), Some("log_limit_reached"));
    assert_eq!(e.ground_runtime, before);
    assert_eq!(e.active.as_ref().unwrap().pending_shared.len(), queued);
}

#[test]
fn engine_firearm_uses_native_sub_one_hp_death_floor() {
    let mut e = ground_firearm_fixture(149.125, false);
    ground_hit(&mut e, 3300).unwrap();
    let vehicle = e.ground_runtime.as_ref().unwrap().get(4).unwrap();
    assert_eq!(vehicle.health_quarter_points, 0);
    assert!(vehicle.pending_death);
}

#[path = "acquisition_delivery_tests.rs"]
pub(crate) mod delivery;

#[test]
fn engine_dying_escort_blocks_further_hits_and_decays_with_original_attacker() {
    let mut e = ground_firearm_fixture(130.0, false);
    ground_hit(&mut e, 3300).unwrap();
    let runtime = e.ground_runtime.as_ref().unwrap();
    let key = runtime.get(4).unwrap().key;
    assert_eq!(runtime.get(4).unwrap().health_quarter_points, 80);
    let before = runtime.clone();
    ground_hit(&mut e, 3400).unwrap();
    assert_eq!(e.ground_runtime, Some(before));

    let policy = e
        .acquisition_gameplay
        .as_ref()
        .unwrap()
        .setup
        .ground_damage
        .as_ref()
        .unwrap();
    let runtime = e.ground_runtime.as_mut().unwrap();
    let first = policy.advance_critical(runtime, key, 1.0).unwrap();
    assert_eq!(first.len(), 1);
    let state = runtime.update(key, 1.0, 0).unwrap();
    assert!(
        state
            .iter()
            .any(|event| event.effect == ground::Effect::StateChanged(3))
    );
    assert_eq!(runtime.get(4).unwrap().health_quarter_points, 62);
    let last = policy.advance_critical(runtime, key, 4.0).unwrap();
    assert_eq!(last.len(), 1);
    let vehicle = runtime.get(4).unwrap();
    assert_eq!(vehicle.health_quarter_points, 0);
    assert_eq!(vehicle.last_attacker, 0x0380_0001);
    assert!(vehicle.pending_death);
    assert!(
        policy
            .advance_critical(runtime, key, 1.0)
            .unwrap()
            .is_empty()
    );
}

#[path = "acquisition_ground_effects_tests.rs"]
mod ground_effects;

fn ground_grenade_fixture(blocked: bool, vehicle_damage: f32) -> Engine<Vec<u8>> {
    ground_grenade_fixture_for_map(blocked, vehicle_damage, "synthetic_arena")
}

pub(crate) fn ground_grenade_fixture_for_map(
    blocked: bool,
    vehicle_damage: f32,
    map_name: &str,
) -> Engine<Vec<u8>> {
    let mut e = ground_firearm_fixture(0.0, false);
    e.item_catalog = crate::human_kill_reward_tests::grenade_items_for_ground_test(vehicle_damage);
    let p = e
        .ground_runtime
        .as_ref()
        .unwrap()
        .get(4)
        .unwrap()
        .pose
        .position;
    let x = p.x - if blocked { 1.0 } else { 20.0 };
    let raw = json!({"format":"mag_collision_world","schema_version":1,"coordinate_space":"world",
        "metadata":{"map_name":map_name,"build":"synthetic","coverage":"complete",
            "provenance":["synthetic grenade wall"],"unknowns":[]},
        "instances":[{"id":"wall","vertices_xyz":[[x,p.y-2.,p.z-3.],[x,p.y+4.,p.z-3.],
            [x,p.y+4.,p.z+3.],[x,p.y-2.,p.z+3.]],"triangles":[[0,1,2],[0,2,3]]}]});
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "synthetic-acq-grenade-{}-{}.json",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::write(&path, raw.to_string()).unwrap();
    e.collision_world =
        Some(crate::collision::CollisionWorld::load(&path, Default::default()).unwrap());
    std::fs::remove_file(path).unwrap();
    let actor = e.active.as_mut().unwrap();
    actor
        .projectiles
        .admit_item_with_catalog(0, 1, 3200, 0, &[0; 57], 4, &e.item_catalog)
        .unwrap();
    actor
        .projectiles
        .accept_owner(
            0,
            1,
            1,
            3300,
            projectile::ProjectileOwnerFields {
                state: Some(2),
                position: Some(CharacterPosition { x: p.x - 2., ..p }),
                rotation_xyzw: None,
            },
        )
        .unwrap();
    e
}

fn ground_grenade_update(
    e: &mut Engine<Vec<u8>>,
    now: u64,
) -> Result<Vec<OutboundMessage>, &'static str> {
    let wire = ObjectUpdate {
        header: UpdateHeader {
            sequence: e.active.as_ref().unwrap().latest_owner_sequence + 1,
            nak_sequence: 0,
            client_index: 0,
            object_type_mask: 0,
            timestamp_ms: now as u32,
        },
        wire: &[],
    }
    .encode_empty_with_profile(e.wire_profile)
    .unwrap();
    dispatch(e, &wire, now)
}

#[test]
fn owned_grenade_terminal_damages_escort_once() {
    for (amount, expected_quarters) in [(100.0, 300), (300.0, 0)] {
        let mut e = ground_grenade_fixture(false, amount);
        ground_grenade_update(&mut e, 3400).unwrap();
        let after = e.ground_runtime.clone();
        let truck = after.as_ref().unwrap().get(4).unwrap();
        assert_eq!(truck.health_quarter_points, expected_quarters);
        assert_eq!(truck.pending_death, expected_quarters == 0);
        assert_eq!(truck.damage_wire().last_source_object, 0x1080_0001);
        assert_eq!(truck.damage_wire().last_location_and_type, 0xa0);
        assert_eq!(truck.last_attacker, 0x0380_0001);
        ground_grenade_update(&mut e, 3500).unwrap();
        assert_eq!(e.ground_runtime, after);
    }
}

#[test]
fn ground_grenade_checks_static_cover_and_rolls_back_failed_record() {
    let mut covered = ground_grenade_fixture(true, 300.0);
    let before = covered.ground_runtime.clone();
    ground_grenade_update(&mut covered, 3400).unwrap();
    assert_eq!(covered.ground_runtime, before);

    let mut failed = ground_grenade_fixture(false, 300.0);
    let before = failed.ground_runtime.clone();
    failed.max_log = failed.written;
    assert_eq!(
        ground_grenade_update(&mut failed, 3400).err(),
        Some("log_limit_reached")
    );
    assert_eq!(failed.ground_runtime, before);
    assert!(
        !failed
            .active
            .as_ref()
            .unwrap()
            .projectiles
            .get(0)
            .unwrap()
            .completed
    );
}

pub(crate) fn ground_grenade_driver_fixture() -> Engine<Vec<u8>> {
    let mut e = delivery::delivery_fixture();
    let policy = ground_firearm_fixture(0.0, true)
        .acquisition_gameplay
        .unwrap()
        .setup
        .ground_damage
        .clone();
    std::sync::Arc::get_mut(&mut e.acquisition_gameplay.as_mut().unwrap().setup)
        .unwrap()
        .ground_damage = policy;
    e.item_catalog = crate::human_kill_reward_tests::grenade_items_for_ground_test(500.0);
    e
}

#[path = "acquisition_vehicle_wave_effects_tests.rs"]
mod vehicle_wave_effects;

fn grenade_report(e: &Engine<Vec<u8>>, sequence: u32, position: CharacterPosition) -> Vec<u8> {
    grenade_report_for_slot(e, sequence, position, 0)
}

fn grenade_report_for_slot(
    e: &Engine<Vec<u8>>,
    sequence: u32,
    position: CharacterPosition,
    slot: u8,
) -> Vec<u8> {
    projectile_replication::ProjectileOwnerDelta {
        header: UpdateHeader {
            sequence,
            nak_sequence: 0,
            client_index: 0,
            object_type_mask: 1 << (16 + slot),
            timestamp_ms: 0,
        },
        update: &projectile::ProjectileOwnerUpdate {
            slot,
            object_index: e.active.as_ref().unwrap().reservation.player,
            fields: projectile::ProjectileOwnerFields {
                state: Some(2),
                position: Some(position),
                rotation_xyzw: None,
            },
        },
    }
    .encode_with_profile(e.wire_profile)
    .unwrap()
}

fn grenade_packet_on_route(
    e: &mut Engine<Vec<u8>>,
    wire: &[u8],
    transport: Transport,
    now_ms: u64,
) -> Result<Vec<OutboundMessage>, &'static str> {
    let actor = e.active.as_ref().unwrap();
    let input = events::EventInput {
        reservation_id: actor.reservation.id.clone(),
        world_id: actor.reservation.world,
        player_id: actor.reservation.player,
        transport,
        direction: Direction::ClientToServer,
        rt_type: 0x3d,
        plugin_id: Some(1),
        payload_hex: protocol::encode_hex(wire),
    };
    let decoded = DecodedEvent::with_profile(wire, input.player_id, e.wire_profile, false);
    e.handle_event(input, &decoded, Duration::from_millis(now_ms), 1)
}

#[test]
fn ground_grenade_tcp_owner_report_can_neither_seed_nor_consume_a_detonation() {
    let mut e = ground_grenade_fixture(false, 300.0);
    let active = e.active.as_mut().unwrap();
    active.projectiles = Default::default();
    active
        .projectiles
        .admit_item_with_catalog(0, 1, 3200, 0, &[0; 57], 4, &e.item_catalog)
        .unwrap();
    let before = e.ground_runtime.clone();
    let truck = before.as_ref().unwrap().get(4).unwrap().pose.position;
    let target = CharacterPosition {
        x: truck.x - 2.0,
        ..truck
    };
    let forged = grenade_report(&e, 99, target);
    grenade_packet_on_route(&mut e, &forged, Transport::Tcp, 3250).unwrap();
    let flight = e.active.as_ref().unwrap().projectiles.get(0).unwrap();
    assert!(!flight.terminal_reported);
    assert!(flight.last_position.is_none());
    assert_eq!(flight.accepted_owner_sequence, 0);
    assert_eq!(e.active.as_ref().unwrap().latest_owner_sequence, 0);

    let genuine = grenade_report(&e, 1, target);
    grenade_packet_on_route(&mut e, &genuine, Transport::Udp, 3290).unwrap();
    let far = CharacterPosition {
        x: truck.x - 20.0,
        ..truck
    };
    let forged = grenade_report(&e, 100, far);
    grenade_packet_on_route(&mut e, &forged, Transport::Tcp, 3300).unwrap();
    let flight = e.active.as_ref().unwrap().projectiles.get(0).unwrap();
    assert!(!flight.completed);
    assert!(flight.terminal_reported);
    assert_eq!(flight.last_position, Some(target));
    assert_eq!(e.ground_runtime, before);

    ground_grenade_update(&mut e, 3301).unwrap();
    assert!(
        e.ground_runtime
            .as_ref()
            .unwrap()
            .get(4)
            .unwrap()
            .pending_death
    );
    assert!(
        e.active
            .as_ref()
            .unwrap()
            .projectiles
            .get(0)
            .unwrap()
            .completed
    );
}

fn grenade_launch_wire(e: &Engine<Vec<u8>>, slot: u16, x: f32) -> Vec<u8> {
    let player = e.active.as_ref().unwrap().reservation.player;
    let mut bits = replication::BitWriter::with_profile(e.wire_profile);
    bits.write(0x0210, 16).unwrap();
    for value in [x, 3., 100., 0., 0., 0., 1., 0., 0., 0.] {
        bits.write(value.to_bits(), 32).unwrap();
    }
    for (value, width) in [
        (4, 8),
        (100, 10),
        (u32::MAX, 32),
        (0x0380_0000 | u32::from(player), 32),
        (u32::from(player), 16),
        (u32::from(slot), 16),
        (0, 6),
    ] {
        bits.write(value, width).unwrap();
    }
    bits.finish()
}

fn prepare_ordinary_grenade_launch(e: &mut Engine<Vec<u8>>) {
    let active = e.active.as_mut().unwrap();
    active.projectiles = Default::default();
    active.grenade_remaining = 2;
    active.spawned_loadout.as_mut().unwrap().inventories[0].slots[4] = Some(4);
    // The fixture has already completed its authored Ground registration.
    let batches = e.ground_vehicles.as_ref().unwrap().batch_count();
    for actor in e.active.iter_mut().chain(e.peers.values_mut()) {
        actor.ground_vehicle_batch = batches;
    }
}

#[test]
fn retail_fourth_frag_launch_fuse_ground_damage_and_cleanup() {
    let mut e = ground_grenade_fixture(false, 100.);
    prepare_ordinary_grenade_launch(&mut e);
    let player = e.active.as_ref().unwrap().reservation.player;
    let slot = 3;
    for invalid_slot in [4, 259] {
        let invalid = grenade_launch_wire(&e, invalid_slot, 100.);
        assert!(
            grenade_packet_on_route(&mut e, &invalid, Transport::Tcp, 3249)
                .unwrap()
                .is_empty()
        );
    }
    assert_eq!(e.active.as_ref().unwrap().grenade_remaining, 2);
    let wire = grenade_launch_wire(&e, slot, 100.);
    assert_eq!(wire.len(), 57);
    let created = grenade_packet_on_route(&mut e, &wire, Transport::Tcp, 3250).unwrap();
    assert_eq!(created.len(), 1, "accepted family-19 frag creation");
    let packet = protocol::decode_hex(&created[0].payload_hex);
    let creation = ObjectUpdate::decode_with_profile(&packet, e.wire_profile).unwrap();
    assert_eq!(creation.header.object_type_mask, 1 << 19);
    let actor = e.active.as_ref().unwrap();
    assert_eq!(actor.grenade_remaining, 1);
    assert!(actor.projectiles.get(3).is_some());
    assert_eq!(
        actor
            .projectile_replicas
            .snapshot_packets(3250)
            .unwrap()
            .len(),
        2
    );

    assert!(
        grenade_packet_on_route(&mut e, &wire, Transport::Tcp, 3251)
            .unwrap()
            .is_empty()
    );
    assert_eq!(e.active.as_ref().unwrap().grenade_remaining, 1);
    let before = e.ground_runtime.clone();
    let truck_pos = before.as_ref().unwrap().get(4).unwrap().pose.position;
    let target = CharacterPosition {
        x: truck_pos.x - 2.,
        ..truck_pos
    };
    let forged = grenade_report_for_slot(&e, 99, target, 3);
    grenade_packet_on_route(&mut e, &forged, Transport::Tcp, 3280).unwrap();
    assert!(
        !e.active
            .as_ref()
            .unwrap()
            .projectiles
            .get(3)
            .unwrap()
            .terminal_reported
    );

    let genuine = grenade_report_for_slot(&e, 1, target, 3);
    grenade_packet_on_route(&mut e, &genuine, Transport::Udp, 3300).unwrap();
    let flight = e.active.as_ref().unwrap().projectiles.get(3).unwrap();
    assert!(flight.terminal_reported);
    assert!(!flight.completed);
    assert_eq!(e.ground_runtime, before);

    ground_grenade_update(&mut e, 3350).unwrap();
    ground_grenade_update(&mut e, 3351).unwrap();
    let actor = e.active.as_ref().unwrap();
    assert!(actor.projectiles.get(3).unwrap().completed);
    assert!(
        actor
            .projectile_replicas
            .snapshot_packets(3350)
            .unwrap()
            .is_empty()
    );
    let ground = e.ground_runtime.clone();
    let damage = ground.as_ref().unwrap().get(4).unwrap().damage_wire();
    assert_eq!(damage.health_quarter_points, 300);
    assert_eq!(damage.last_source_object, 0x1380_0000 | u32::from(player));

    let next = grenade_launch_wire(&e, slot, 101.);
    assert!(
        grenade_packet_on_route(&mut e, &next, Transport::Tcp, 3351)
            .unwrap()
            .is_empty()
    );
    e.queue_projectile_publications(Duration::from_millis(3351))
        .unwrap();
    let terminal = e
        .drain_shared(Vec::new(), 1)
        .unwrap()
        .into_iter()
        .filter_map(|message| {
            let bytes = protocol::decode_hex(&message.payload_hex);
            let decoded = ObjectUpdate::decode_with_profile(&bytes, e.wire_profile).ok()?;
            projectile::decode_projectile_updates_with_profile(&decoded, e.wire_profile).ok()
        })
        .flatten()
        .filter(|row| row.slot == 3 && row.fields.state == Some(2))
        .count();
    assert_eq!(
        terminal,
        e.active
            .iter()
            .chain(e.peers.values())
            .filter(|actor| actor.initial_objects.is_some())
            .count()
    );

    e.queue_projectile_publications(Duration::from_millis(3849))
        .unwrap();
    assert!(e.projectile_cleanup_pending(e.active.as_ref().unwrap(), 3));
    let at = Duration::from_millis(3850);
    e.queue_projectile_publications(at).unwrap();
    let removal = projectile_replication::ProjectileRemoval {
        slot: 3,
        object_index: player,
    };
    let queued = removal
        .encode_with_profile(1, 3850, e.wire_profile)
        .unwrap();
    assert!(e.active.as_ref().unwrap().pending_shared.contains(&queued));
    assert!(
        grenade_packet_on_route(&mut e, &next, Transport::Tcp, 3851)
            .unwrap()
            .is_empty()
    );
    assert_eq!(e.active.as_ref().unwrap().grenade_remaining, 1);
    assert!(!e.drain_shared(Vec::new(), 1).unwrap().is_empty());
    assert!(!e.projectile_cleanup_pending(e.active.as_ref().unwrap(), 3));
    assert!(
        !grenade_packet_on_route(&mut e, &next, Transport::Tcp, 3852)
            .unwrap()
            .is_empty()
    );
    assert_eq!(e.active.as_ref().unwrap().grenade_remaining, 0);
    assert_eq!(e.ground_runtime, ground);
}

#[test]
fn prototype_frag_allows_slot_two_and_rejects_slot_three() {
    let mut e = ground_grenade_fixture(false, 100.);
    prepare_ordinary_grenade_launch(&mut e);
    let profile = wire_profile::WireProfile::prototype();
    e.wire_profile = profile;
    let actor = e.active.as_mut().unwrap();
    actor.wire_profile = profile;
    actor.projectile_replicas = projectile_replication::ProjectileReplicas::with_profile(profile);
    let invalid = grenade_launch_wire(&e, 3, 100.);
    assert!(
        grenade_packet_on_route(&mut e, &invalid, Transport::Tcp, 3250)
            .unwrap()
            .is_empty()
    );
    assert_eq!(e.active.as_ref().unwrap().grenade_remaining, 2);
    let valid = grenade_launch_wire(&e, 2, 100.);
    assert!(
        !grenade_packet_on_route(&mut e, &valid, Transport::Tcp, 3251)
            .unwrap()
            .is_empty()
    );
    assert_eq!(e.active.as_ref().unwrap().grenade_remaining, 1);
}

#[test]
fn ground_grenade_allows_client_tcp_launch_and_rejects_server_direction_at_ingress() {
    let mut e = ground_grenade_fixture(false, 300.0);
    let active = e.active.as_mut().unwrap();
    active.projectiles = Default::default();
    active.grenade_remaining = 1;
    active.spawned_loadout.as_mut().unwrap().inventories[0].slots[4] = Some(4);
    let player = active.reservation.player;

    let wire = grenade_launch_wire(&e, 0, 100.);
    let actor = e.active.as_ref().unwrap();
    let invalid = json!({
        "v":BRIDGE_VERSION,"op":"event","request_id":"wrong-direction",
        "reservation_id":actor.reservation.id,"world_id":actor.reservation.world,
        "player_id":player,"transport":"tcp","direction":"server_to_client",
        "rt_type":0x3d,"plugin_id":1,"payload_hex":protocol::encode_hex(&wire),
    });
    assert!(parse_request(invalid.to_string().as_bytes()).is_err());
    assert_eq!(e.active.as_ref().unwrap().grenade_remaining, 1);
    assert!(e.active.as_ref().unwrap().projectiles.get(0).is_none());

    grenade_packet_on_route(&mut e, &wire, Transport::Tcp, 3250).unwrap();
    assert_eq!(e.active.as_ref().unwrap().grenade_remaining, 0);
    assert!(e.active.as_ref().unwrap().projectiles.get(0).is_some());
}

#[test]
fn acquisition_precision_produces_original_structural_entries_in_order() {
    let mut engine = capture_fixture();
    let world = engine.objective_world.as_mut().unwrap();
    assert!(world.stage_precision_strike(120, None).unwrap());
    let mut entries = Vec::new();
    let updates = world
        .advance_acquisition_precision_entries(Duration::from_millis(400), |entry, _| {
            entries.push((
                entry.previous,
                entry.delta.state_actor.state,
                entry.requires_authored_explosion(),
            ));
            Ok(())
        })
        .unwrap();
    assert_eq!(updates.len(), 1);
    assert_eq!(
        entries,
        [
            (
                ObjectiveState::Deactivated,
                ObjectiveState::Activating,
                false
            ),
            (ObjectiveState::Activating, ObjectiveState::Activated, false),
            (ObjectiveState::Activated, ObjectiveState::Completed, true),
        ],
    );
}

#[test]
fn acquisition_structural_spawn_candidate_updates_actor_and_shared_availability() {
    let mut engine = capture_fixture();
    reserve(&mut engine, 0);
    let actor = engine.active.as_ref().unwrap();
    let squad = actor.squad_catalog.squad_index();
    let old_default = actor.squad_spawn;
    let current = engine.spawn_catalog.as_ref().unwrap();
    let mut next = current.clone();
    let destination = (0..16).find(|&index| index != old_default).unwrap();
    next.set_runtime_squad_default(squad, destination).unwrap();
    next.set_runtime_point_active(
        old_default,
        current.point(old_default).unwrap().token,
        false,
    )
    .unwrap();
    let mut shared = Vec::new();
    let writes = engine
        .prepare_runtime_spawn_publication(current, &next, 400, &mut shared)
        .unwrap();
    assert_eq!(engine.active.as_ref().unwrap().squad_spawn, old_default);
    assert_eq!(shared.len(), 2);
    engine.install_runtime_spawn_publication(&writes);
    let actor = engine.active.as_ref().unwrap();
    assert_eq!(actor.squad_spawn, destination);
    assert!(actor.squad_catalog.alpha(destination).is_some());
    assert!(actor.squad_catalog.alpha(old_default).is_none());
}

#[test]
fn acquisition_structural_native_spawn_retains_supported_local_category_zero() {
    let mut engine = capture_fixture();
    reserve(&mut engine, 0);
    let actor = engine.active.as_ref().unwrap();
    let squad = actor.squad_catalog.squad_index();
    let fallback = actor.squad_spawn;
    let current = engine.spawn_catalog.as_ref().unwrap();
    let mut next = current.clone();
    let mut fallbacks = [u16::MAX; 32];
    for (index, fallback) in fallbacks.iter_mut().enumerate() {
        if let Some(binding) = current.selected_squad(index as u16) {
            *fallback = binding.default_spawn;
        }
    }
    let native_point = 16;
    assert!(next.set_runtime_squad_default(squad, native_point).is_err());
    next.configure_acquisition_structural_spawn_scope([native_point].into(), fallbacks)
        .unwrap();
    next.set_runtime_point_active(native_point, next.point(native_point).unwrap().token, true)
        .unwrap();
    next.set_runtime_squad_default(squad, native_point).unwrap();
    let mut shared = Vec::new();
    let writes = engine
        .prepare_runtime_spawn_publication(current, &next, 400, &mut shared)
        .unwrap();
    assert_eq!(next.default_for_squad(squad), Some(native_point));
    assert!(next.point_active(native_point));
    assert_eq!(shared.len(), 1);
    engine.install_runtime_spawn_publication(&writes);
    let actor = engine.active.as_ref().unwrap();
    assert_eq!(actor.squad_spawn, fallback);
    assert_eq!(actor.selected_spawn.unwrap().spawn_point_index, fallback);
    assert!(actor.squad_catalog.alpha(native_point).is_none());
}
