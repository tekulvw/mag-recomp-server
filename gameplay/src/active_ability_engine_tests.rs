//! Normal authenticated command-event regressions with invented map and tuning.
use super::*;
use crate::{
    active_abilities::{Ability, Authority, CommandProperty, CommandRequest, Mode, Rules, Side},
    active_ability_effects::{ClusterRules, EffectRules},
    active_ability_engine::{Boundary, SensorObjective, Setup},
    active_ability_sensor::SensorRules,
    spawn_progression::{engine, ready, reserve},
};

const ABILITIES: [Ability; 6] = [
    Ability::ClusterBomb,
    Ability::Sensor,
    Ability::PrecisionStrike,
    Ability::GasBombardment,
    Ability::StrafingRun,
    Ability::GuidedArtillery,
];

fn setup() -> Setup {
    let digest = "ab".repeat(32);
    let mut modes = Vec::new();
    let mut effects = Vec::new();
    for side in [Side::Attacker, Side::Defender] {
        for players in [64, 128, 256] {
            modes.push(Mode {
                players,
                side,
                squad: if players == 256 {
                    ABILITIES.to_vec()
                } else {
                    Vec::new()
                },
                platoon: Vec::new(),
                strategic: Vec::new(),
            });
        }
        for faction in 0..3 {
            effects.push((
                side,
                faction,
                Ability::ClusterBomb,
                EffectRules::Cluster(ClusterRules {
                    delay_ms: 12,
                    damage_delay_ms: 15,
                    interval_ms: 8,
                    count: 3,
                    scatter_radius: 0.25,
                    damage: 17.,
                    inner_radius: 2.,
                    inner_scale: 0.5,
                    outer_radius: 5.,
                    outer_scale: 0.,
                }),
            ));
            effects.push((
                side,
                faction,
                Ability::Sensor,
                EffectRules::Sensor(SensorRules {
                    delay_ms: 12,
                    duration_ms: 41,
                    attacker_radius: 29.,
                    defender_radius: 19.,
                    speed_threshold: 0.,
                }),
            ));
        }
    }
    Setup {
        rules: Rules {
            schema_version: 1,
            application_id: 20231,
            rules_sha256: digest.clone(),
            source_archive_sha256: digest.clone(),
            global_source_member: "invented-command-table".into(),
            global_source_sha256: digest.clone(),
            command_global_cooldown_ms: 10,
            strategic_global_cooldown_ms: 10,
            authority: Authority {
                effects: false,
                target_validation: false,
                ability_replication: false,
            },
            modes,
            commands: ABILITIES
                .into_iter()
                .map(|ability| CommandProperty {
                    ability,
                    source_member: format!("invented-{ability:?}"),
                    source_sha256: digest.clone(),
                    cooldown_ms: 100,
                    delay_ms: 12,
                })
                .collect(),
            strategic: Vec::new(),
        },
        map_name: "synthetic_ability_arena".into(),
        boundary: Boundary {
            min_x: 2.,
            max_x: 200.,
            min_z: 2.,
            max_z: 200.,
        },
        vertical_bounds: [1., 20.],
        aaa: Vec::new(),
        objectives: vec![SensorObjective {
            object_index: 20,
            position: [100., 3., 100.],
            category: 2,
            subtype: 0,
            team: 1,
            platoons: vec![4],
        }],
        effects,
        damage_triggers: Vec::new(),
    }
}

pub(crate) fn configured() -> Engine<Vec<u8>> {
    let abilities = setup();
    let name = &abilities.map_name;
    let spawns = spawn_catalog::SpawnCatalog::from_json(
        &json!({
            "format":"mag_spawn_catalog","schema_version":1,"source":"synthetic ability arena",
            "active":{"map_name":name,"squad_index":16,"default_spawn":0,"available":[0]},
            "match_profile":{"game_type":2,"auto_assign_teams":true,"team_capacity":8,
                "squad_capacity":4,"has_bunkers":false,"has_objectives":false,"attacking_faction":0},
            "maps":[{"name":name,"owner_faction":1,"spawn_count":1,
                "locations":[{"index":0,"token":71,"kind":"on_foot","position":{"x":100.,"y":3.,"z":100.}}],
                "squads":(0..2).flat_map(|team| (0..2).map(move |squad| json!({
                    "index":team*16+squad,"team":team,"platoon":0,"squad":squad,
                    "faction":if team==1 { Some(1) } else { None },"available":[0],"default_spawn":0
                }))).collect::<Vec<_>>() }]
        })
        .to_string(),
    )
    .unwrap();
    let passives = leadership_passives::LeadershipCatalog::from_json(
        &json!({
            "schema_version":1,"application_id":20231,
            "source":{"member":"scripts/character_leadership_abilities.zpr","member_sha256":"a1".repeat(32),
                "archive_sha256":"b2".repeat(32),"extractor":"synthetic","projector":"synthetic"},
            "max_separation_range":10.0,"refresh_hold_ms":100,"removal_grace_ms":200,
            "levels":[{"level":0,"name":"none","modifiers":[]},
                {"level":1,"name":"squad","modifiers":[{"kind":"kCharModReloadTime","value":0.1}]},
                {"level":2,"name":"platoon","modifiers":[{"kind":"kCharModReloadTime","value":0.1}]},
                {"level":3,"name":"officer","modifiers":[{"kind":"kCharModReloadTime","value":0.1}]}]
        })
        .to_string(),
    )
    .unwrap();
    let policy = leadership_runtime::RolePolicy::from_json(
        &json!({
            "schema_version":2,"application_id":20231,"squad_leader":"native","development_bindings":[],
            "native_election":{"source_sha256":"c3".repeat(32),"squad_level_weight":1.0,
                "squad_mslc_weight":1.0,"default_record":{
                    "leadership_level":1,"nominations":1,"matches_since_last_command":0}}
        })
        .to_string(),
    )
    .unwrap();
    let mut engine = engine()
        .with_spawn_catalog(spawns)
        .with_leadership_passives(passives, policy)
        .unwrap()
        .with_active_abilities(abilities)
        .unwrap();
    engine.objective_catalog = None;
    engine.objective_world = None;
    engine
}

pub(crate) fn add_ready(engine: &mut Engine<Vec<u8>>, slot: u16) {
    reserve(engine, slot);
    let actor = engine
        .active
        .iter_mut()
        .chain(engine.peers.values_mut())
        .find(|actor| actor.reservation.player == slot)
        .unwrap();
    ready(actor);
    actor.life = CharacterLife::Alive {
        generation: 1,
        health_quarter_points: 400,
    };
    actor.nominations = 1;
    assert_eq!(actor.team(), 1);
}

pub(crate) fn activate(
    engine: &mut Engine<Vec<u8>>,
    request: CommandRequest,
    actor: u16,
    now: u64,
) -> Response {
    engine.handle(
        Request::Event {
            v: BRIDGE_VERSION,
            request_id: format!("synthetic-command-{actor}"),
            reservation_id: format!("synthetic-reservation-{actor:04}"),
            world_id: 1,
            player_id: actor,
            transport: Transport::Tcp,
            direction: Direction::ClientToServer,
            rt_type: 0x3d,
            plugin_id: Some(1),
            payload_hex: protocol::encode_hex(&request.to_bytes().unwrap()),
        },
        Duration::from_millis(now),
        1,
    )
}

pub(crate) fn request(squad_index: u16, ability: Ability) -> CommandRequest {
    CommandRequest {
        squad_index,
        ability,
        position: [20., 3., 23.],
        request_time: u32::MAX,
        target_id: u32::MAX,
    }
}

fn last_command(engine: &Engine<Vec<u8>>) -> Value {
    std::str::from_utf8(&engine.writer)
        .unwrap()
        .lines()
        .rev()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .find(|event| event["decoded"]["type"] == "command_ability")
        .unwrap()
}

#[test]
fn native_command_checks_held_role_and_exact_squad_before_spending_timers() {
    let mut engine = configured();
    add_ready(&mut engine, 0);
    add_ready(&mut engine, 1);
    engine
        .active
        .iter_mut()
        .chain(engine.peers.values_mut())
        .find(|actor| actor.reservation.player == 1)
        .unwrap()
        .nominations = 0;
    engine.match_started = true;
    engine.reconcile_squad_leaders().unwrap();
    let command = request(16, Ability::Sensor);

    let denied = activate(&mut engine, command, 1, 20);
    assert!(denied.ok, "{:?}", denied.error);
    assert_eq!(
        last_command(&engine)["gameplay_denial"],
        "command_requires_native_squad_leader"
    );

    let wrong_squad = request(17, Ability::Sensor);
    let denied = activate(&mut engine, wrong_squad, 0, 20);
    assert!(denied.ok, "{:?}", denied.error);
    assert_eq!(
        last_command(&engine)["gameplay_denial"],
        "command_wrong_squad"
    );

    let accepted = activate(&mut engine, command, 0, 20);
    assert!(accepted.ok, "{:?}", accepted.error);
    let event = last_command(&engine);
    assert_eq!(event["decoded"]["accepted"], true);
    assert_eq!(event["decoded"]["activate_time_ms"], 20);
    assert_eq!(event["decoded"]["request_time"], u32::MAX);
    assert_eq!(event["gameplay_denial"], Value::Null);
}

#[test]
fn command_replay_and_another_squad_share_platoon_timer_but_keep_individual_timer() {
    let mut engine = configured();
    add_ready(&mut engine, 0);
    add_ready(&mut engine, 1);
    let sibling = engine
        .active
        .iter_mut()
        .chain(engine.peers.values_mut())
        .find(|actor| actor.reservation.player == 1)
        .unwrap();
    sibling.squad_catalog = std::sync::Arc::new(sibling.squad_catalog.for_squad(17).unwrap());
    engine.match_started = true;
    engine.reconcile_squad_leaders().unwrap();
    let first = request(16, Ability::Sensor);
    let sibling_command = request(17, Ability::ClusterBomb);

    assert!(activate(&mut engine, first, 0, 20).ok);
    assert_eq!(last_command(&engine)["decoded"]["accepted"], true);
    assert!(activate(&mut engine, first, 0, 20).ok);
    assert_eq!(last_command(&engine)["gameplay_denial"], "command_cooldown");
    assert!(activate(&mut engine, sibling_command, 1, 29).ok);
    assert_eq!(last_command(&engine)["gameplay_denial"], "command_cooldown");
    assert!(activate(&mut engine, sibling_command, 1, 30).ok);
    assert_eq!(last_command(&engine)["decoded"]["accepted"], true);
    assert!(activate(&mut engine, first, 0, 40).ok);
    assert_eq!(last_command(&engine)["gameplay_denial"], "command_cooldown");
    assert!(activate(&mut engine, first, 0, 120).ok);
    assert_eq!(last_command(&engine)["decoded"]["accepted"], true);
}

#[test]
fn command_record_failure_releases_no_effect_and_spends_no_cooldown() {
    let mut engine = configured();
    add_ready(&mut engine, 0);
    engine.match_started = true;
    engine.reconcile_squad_leaders().unwrap();
    let command = request(16, Ability::Sensor);
    let now = Duration::from_millis(20);
    let actor = engine.active.as_ref().unwrap();
    let (prepared, _, _) = engine.plan_command_ability(actor, command, now).unwrap();
    assert!(prepared.is_some());
    let written = engine.writer.len();
    engine.max_log = engine.written;
    let failed = activate(&mut engine, command, 0, 20);
    assert!(!failed.ok);
    assert_eq!(failed.error.as_deref(), Some("log_limit_reached"));
    assert!(failed.messages.is_empty());
    assert_eq!(engine.writer.len(), written);
    let (after, _, _) = engine
        .plan_command_ability(engine.active.as_ref().unwrap(), command, now)
        .unwrap();
    assert!(after.is_some());
}

#[test]
fn late_join_queues_platoon_creation_before_active_squad_ability_snapshot() {
    let mut engine = configured();
    add_ready(&mut engine, 0);
    engine.match_started = true;
    engine.reconcile_squad_leaders().unwrap();
    assert!(activate(&mut engine, request(16, Ability::Sensor), 0, 20).ok);
    engine
        .queue_active_ability_state(Duration::from_millis(21), 1)
        .unwrap();

    add_ready(&mut engine, 1);
    engine
        .queue_active_ability_state(Duration::from_millis(22), 1)
        .unwrap();
    let joined = engine
        .active
        .iter()
        .chain(engine.peers.values())
        .find(|actor| actor.reservation.player == 1)
        .unwrap();
    let updates = joined
        .pending_shared
        .iter()
        .filter_map(|bytes| {
            replication::ObjectUpdate::decode_with_profile(bytes, engine.wire_profile).ok()
        })
        .filter(|update| matches!(update.header.object_type_mask, 0x100 | 0x400))
        .collect::<Vec<_>>();
    assert_eq!(updates.len(), 2);
    let platoons = &updates[0];
    assert_eq!(platoons.header.object_type_mask, 1 << 10);
    let mut bits = replication::BitReader::with_profile(platoons.wire, engine.wire_profile);
    bits.position = engine.wire_profile.header_bits();
    for platoon in 0..8 {
        assert_eq!(bits.read(1).unwrap(), 1);
        assert_eq!(bits.read(16).unwrap(), platoon);
        assert_eq!(bits.read(2).unwrap(), 3); // active present and true
        assert_eq!(bits.read(1).unwrap(), 1); // owner present
        assert_eq!(bits.read(16).unwrap(), 0x105);
        assert_eq!(bits.read(6).unwrap(), 0); // no activation data on platoon
        for width in [16, 16, 16, 16, 8] {
            // leader, enabled, active, available, shadow
            assert_eq!(bits.read(1).unwrap(), 1);
            bits.read(width).unwrap();
        }
    }
    assert_eq!(bits.read(1).unwrap(), 0);
    let squad = &updates[1];
    assert_eq!(squad.header.object_type_mask, 1 << 8);
    let mut bits = replication::BitReader::with_profile(squad.wire, engine.wire_profile);
    bits.position = engine.wire_profile.header_bits();
    assert_eq!(bits.read(1).unwrap(), 1);
    assert_eq!(bits.read(16).unwrap(), 16);
    assert_eq!(bits.read(18).unwrap(), 0); // absent fields preceding Sensor
    assert_eq!(bits.read(1).unwrap(), 1);
    for axis in [20.0f32, 3.0, 23.0] {
        assert_eq!(bits.read(32).unwrap(), axis.to_bits());
    }
    assert_eq!(bits.read(16).unwrap(), 16);
    assert_eq!(bits.read(32).unwrap(), 20); // retained server activation time
    assert_eq!(bits.read(1).unwrap(), 1); // accepted and enabled
    assert_eq!(bits.read(32).unwrap(), u32::MAX);
    assert_eq!(bits.read(3).unwrap(), 0);
    assert_eq!(bits.read(1).unwrap(), 1);
    assert_eq!(bits.read(16).unwrap(), 4); // Sensor flag
}

use crate::synthetic_dynamic_collision as dynamic_fixture;

pub(crate) fn strike_floor(map_name: &str, qualified: bool) -> collision::CollisionWorld {
    let mut floor = json!({
        "id":"invented_floor",
        "vertices_xyz":[[80.,1.,80.],[120.,1.,80.],[120.,1.,120.],[80.,1.,120.]],
        "triangles":[[0,1,2],[0,2,3]]
    });
    if qualified {
        floor["collision"] = json!({"filter_info":9,"ray_child_filter_info":0,
            "convex_radius":0.,"child_filter_policy":"uniform","provenance":"synthetic"});
    }
    collision::CollisionWorld::from_bytes(
        &serde_json::to_vec(&json!({
            "format":"mag_collision_world","schema_version":1,"coordinate_space":"world",
            "metadata":{"map_name":map_name,"build":"synthetic","coverage":"complete",
                "provenance":["synthetic"],"unknowns":[]},
            "instances":[floor]
        }))
        .unwrap(),
        Default::default(),
    )
    .unwrap()
}

pub(crate) fn prepare_effect_tick(
    engine: &mut Engine<Vec<u8>>,
    native: &std::collections::BTreeMap<u32, acquisition_character_effects::NativeState>,
    millis: u64,
) -> Result<acquisition_character_effects::Candidate, &'static str> {
    let mut rows = [0; 32];
    rows[6] = (1 << 6) | (1 << 21);
    rows[19] = 1 | (1 << 9);
    rows[24] = 1 | (1 << 9);
    let now = Duration::from_millis(millis);
    engine.prepare_acquisition_character_callback(native, now, |engine, native| {
        let result = engine.stage_active_ability_update(
            now,
            native,
            domination_structural_world::BlastCollision {
                filter_rows: &rows,
                limits: Default::default(),
                sequences: &Default::default(),
                material_costs: &Default::default(),
                terrain_materials: None,
            },
            None,
        )?;
        Ok((result.audit, result.messages))
    })
}

fn actor(engine: &Engine<Vec<u8>>, slot: u16) -> &Active {
    engine
        .active
        .iter()
        .chain(engine.peers.values())
        .find(|actor| actor.reservation.player == slot)
        .unwrap()
}

pub(crate) fn actor_mut(engine: &mut Engine<Vec<u8>>, slot: u16) -> &mut Active {
    engine
        .active
        .iter_mut()
        .chain(engine.peers.values_mut())
        .find(|actor| actor.reservation.player == slot)
        .unwrap()
}

#[test]
fn timed_cluster_damage_retries_unqualified_geometry_and_credit_is_fenced_to_invoker() {
    let mut engine = configured()
        .with_dynamic_collision_bodies(
            dynamic_collision::Catalog::from_json(&dynamic_fixture::fixture().to_string()).unwrap(),
            dynamic_collision_runtime::RuntimeBindings {
                character_template: "invented-character".into(),
                vehicles: Vec::new(),
            },
        )
        .unwrap();
    engine.collision_world = Some(strike_floor(engine.map_name(), false));
    engine.item_catalog = crate::human_kill_reward_tests::combat_items_for_ground_test(0.);
    let rules = character_damage_rules::Rules::from_json(
        &json!({"schema_version":1,"application_id":20231,"map_name":engine.map_name(),
            "game_type":2,"friendly_fire_multiplier":0.5,"provenance":{
                "spawn_catalog_sha256":engine.spawn_catalog.as_ref().unwrap().source_sha256().unwrap(),
                "source":"synthetic"}})
        .to_string(),
    )
    .unwrap();
    engine = engine.with_character_damage_rules(rules).unwrap();
    add_ready(&mut engine, 0);
    let items = std::sync::Arc::clone(&engine.item_catalog);
    let leader = actor_mut(&mut engine, 0);
    leader
        .reservation
        .character
        .loadouts
        .as_mut()
        .unwrap()
        .inventories[0][12] = Some(3);
    leader.current_inventory_handle = Some(replication::InventoryHandle {
        update_id: 1,
        inventory_index: 0,
    });
    leader.spawned_loadout = Some(
        leader
            .reservation
            .character
            .replication_loadout(&items, 0, leader.wire_profile)
            .unwrap(),
    );
    engine.match_started = true;
    engine.reconcile_squad_leaders().unwrap();

    let mut command = request(16, Ability::ClusterBomb);
    command.position = [100., 1., 100.];
    assert!(activate(&mut engine, command, 0, 20).ok);
    assert_eq!(last_command(&engine)["decoded"]["accepted"], true);

    let id = 0x0380_0000;
    let mut native = std::collections::BTreeMap::from([(
        id,
        acquisition_character_effects::NativeState {
            invulnerability: Some(0),
            associated_vehicle: Some(protocol::VictimWasMounted::NoAssociatedVehicle),
        },
    )]);
    let early = prepare_effect_tick(&mut engine, &native, 46).unwrap();
    assert!(
        early
            .audit()
            .iter()
            .all(|entry| entry["on_character_hit"].is_null())
    );
    assert!(matches!(
        prepare_effect_tick(&mut engine, &native, 47),
        Err("air_strike_static_impact_ray_failed")
    ));
    assert_eq!(actor(&engine, 0).life.damage_health(), Some((1, 400)));

    engine.collision_world = Some(strike_floor(engine.map_name(), true));
    let impact = prepare_effect_tick(&mut engine, &native, 47).unwrap();
    let damage = impact
        .audit()
        .iter()
        .find(|entry| entry["on_character_hit"] == true)
        .unwrap();
    assert_eq!(damage["damage_source_type"], 4);
    assert_eq!(damage["attacker"], id);
    assert_eq!(damage["outcome"], "Applied");
    assert_eq!(actor(&engine, 0).life.damage_health(), Some((1, 400)));
    engine
        .record_acquisition_character_effects(impact, &mut native, 1)
        .unwrap();
    assert!(actor(&engine, 0).life.damage_health().unwrap().1 < 400);
    assert_eq!(actor(&engine, 0).last_damage.last_location_and_type, 0xa1);

    actor_mut(&mut engine, 0).reservation.id = "synthetic-replacement".into();
    let next = prepare_effect_tick(&mut engine, &native, 55).unwrap();
    let damage = next
        .audit()
        .iter()
        .find(|entry| entry["on_character_hit"] == true)
        .unwrap();
    assert_eq!(damage["damage_source_type"], 4);
    assert_eq!(damage["attacker"], u32::MAX);
    assert_eq!(damage["calculation"]["incoming_damage_base"], 0.5);
}

fn published_detection(engine: &Engine<Vec<u8>>, recipient: u16, target: u16) -> Option<bool> {
    actor(engine, recipient)
        .pending_shared
        .iter()
        .rev()
        .find_map(|wire| {
            let update =
                replication::ObjectUpdate::decode_with_profile(wire, engine.wire_profile).ok()?;
            if update.header.object_type_mask != 1 << 3 {
                return None;
            }
            let mut bits = replication::BitReader::with_profile(wire, engine.wire_profile);
            bits.position = engine.wire_profile.header_bits();
            if bits.read(1).ok()? != 1
                || bits.read(16).ok()? != u32::from(target)
                || bits.read(10).ok()? != 0
                || bits.read(1).ok()? != 1
            {
                return None;
            }
            Some(bits.read(1).ok()? != 0)
        })
}

#[test]
fn defender_sensor_marks_verified_moving_target_and_publishes_expiry() {
    let mut engine = configured();
    add_ready(&mut engine, 0);
    add_ready(&mut engine, 1);
    let target = actor_mut(&mut engine, 1);
    target.squad_catalog = std::sync::Arc::new(target.squad_catalog.for_squad(0).unwrap());
    target.reservation.character.faction = 0;
    target.nominations = 0;
    engine.match_started = true;
    engine.reconcile_squad_leaders().unwrap();
    assert!(activate(&mut engine, request(16, Ability::Sensor), 0, 20).ok);
    assert_eq!(last_command(&engine)["decoded"]["accepted"], true);
    let mut native = Default::default();
    let first = prepare_effect_tick(&mut engine, &native, 21).unwrap();
    engine
        .record_acquisition_character_effects(first, &mut native, 1)
        .unwrap();
    engine
        .queue_active_ability_state(Duration::from_millis(21), 1)
        .unwrap();
    assert_eq!(published_detection(&engine, 0, 1), None);

    let repeated = prepare_effect_tick(&mut engine, &native, 22).unwrap();
    engine
        .record_acquisition_character_effects(repeated, &mut native, 1)
        .unwrap();
    engine
        .queue_active_ability_state(Duration::from_millis(22), 1)
        .unwrap();
    assert_eq!(published_detection(&engine, 0, 1), None);

    actor_mut(&mut engine, 1).tracking.server_position(
        replication::CharacterPosition {
            x: 101.,
            y: 3.,
            z: 100.,
        },
        player_tracking::PositionOrigin::ServerSpawn,
        23,
    );
    let marked = prepare_effect_tick(&mut engine, &native, 23).unwrap();
    engine
        .record_acquisition_character_effects(marked, &mut native, 1)
        .unwrap();
    engine
        .queue_active_ability_state(Duration::from_millis(23), 1)
        .unwrap();
    assert_eq!(published_detection(&engine, 0, 1), Some(true));

    let expired = prepare_effect_tick(&mut engine, &native, 5023).unwrap();
    engine
        .record_acquisition_character_effects(expired, &mut native, 1)
        .unwrap();
    engine
        .queue_active_ability_state(Duration::from_millis(5023), 1)
        .unwrap();
    assert_eq!(published_detection(&engine, 0, 1), Some(false));
}

#[test]
fn attacker_interception_spends_timers_but_invalid_target_does_not() {
    let mut engine = configured();
    add_ready(&mut engine, 0);
    let squad = std::sync::Arc::new(engine.spawn_catalog.as_ref().unwrap().for_squad(0).unwrap());
    let actor = actor_mut(&mut engine, 0);
    actor.squad_catalog = squad;
    actor.reservation.character.faction = 0;
    engine.reconcile_squad_leaders().unwrap();
    engine.match_started = true;
    let hash = "bb".repeat(32);
    let catalog = objective_catalog::ObjectiveCatalog::from_json(&json!({
        "format":"mag_retail_objective_catalog","schema_version":1,"application_id":20231,
        "protocol":protocol::ProtocolInfo::RETAIL_PROTOCOL,"game_type":2,"map_name":engine.map_name(),
        "entity_family":12,"network_type":12,
        "source":{"authored_catalog_sha256":hash,"binding_capture_sha256":hash,
            "startup_proof_sha256":hash,"source_image_sha256":hash,"capture_qualification":"native_source_gameplay"},
        "objects":[{"token":"0x0000000a","object_index":0,"initial_state":1}],
        "squads":(0..32).map(|index|json!({"squad_index":index,"team":index/16,
            "initial_objective_index":0,"authored_objectives":[{"object_index":0,"order":1}]})).collect::<Vec<_>>()
    }).to_string()).unwrap();
    engine.objective_world = Some(objective_control::ObjectiveWorld::new(&catalog));
    let mut bindings = (*engine.active_abilities.as_ref().unwrap().setup).clone();
    bindings.aaa = vec![active_ability_engine::AntiAir {
        object_index: 0,
        position: [100., 3., 100.],
    }];
    engine.active_abilities = Some(std::sync::Arc::new(bindings).runtime().unwrap());
    let mut command = request(0, Ability::ClusterBomb);
    command.position = [1., 3., 100.];
    assert!(activate(&mut engine, command, 0, 20).ok);
    assert_eq!(
        last_command(&engine)["gameplay_denial"],
        "command_target_outside_level"
    );
    command.position = [100., f32::MAX, 100.];
    assert!(activate(&mut engine, command, 0, 20).ok);
    assert_eq!(
        last_command(&engine)["gameplay_denial"],
        "command_anti_air_unavailable"
    );
    command.position = [100., 3., 100.];
    assert!(activate(&mut engine, command, 0, 20).ok);
    let intercepted = last_command(&engine);
    assert_eq!(intercepted["decoded"]["accepted"], true);
    assert_eq!(intercepted["decoded"]["enabled_at_start"], false);
    assert_eq!(intercepted["decoded"]["anti_air"], 0);
    assert!(activate(&mut engine, command, 0, 30).ok);
    assert_eq!(last_command(&engine)["gameplay_denial"], "command_cooldown");
    let world = engine.objective_world.as_mut().unwrap();
    let mut delta = world.snapshot(2, 120).objectives[0];
    delta.state_actor.state = replication::ObjectiveState::Completed;
    world.install_acquisition_delta(delta).unwrap();
    assert!(activate(&mut engine, command, 0, 120).ok);
    assert_eq!(last_command(&engine)["decoded"]["enabled_at_start"], true);
}
