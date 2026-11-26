//! Vehicle Update consumers through the same transaction used by the world tick.
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

fn destruction_scene(wall: bool) -> collision::CollisionWorld {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "mag-destruction-{}-{}.json",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let mut instances = vec![json!({
        "id":"invented-floor", "vertices_xyz":[[950.,2.,980.],[1120.,2.,980.],[950.,2.,1080.],[1120.,2.,1080.]],
        "triangles":[[0,1,2],[1,2,3]]
    })];
    if wall {
        instances.push(json!({
            "id":"invented-blocking-wall", "vertices_xyz":[[1001.5,2.,995.],[1001.5,9.,995.],
                [1001.5,2.,1005.],[1001.5,9.,1005.]],"triangles":[[0,1,2],[1,2,3]]
        }));
    }
    std::fs::write(
        &path,
        json!({
            "format":"mag_collision_world", "schema_version":1, "coordinate_space":"world",
            "metadata":{"map_name":"synthetic_arena","build":"synthetic","provenance":["synthetic"],
                "unknowns":[],"coverage":"provisional"},
            "instances":instances,
        })
        .to_string(),
    )
    .unwrap();
    let world = collision::CollisionWorld::load(&path, Default::default()).unwrap();
    std::fs::remove_file(path).unwrap();
    world
}

fn destruction_fixture(wall: bool) -> Engine<Vec<u8>> {
    destruction_fixture_with_bot_origin(wall, false)
}

fn destruction_fixture_with_bot_origin(wall: bool, detached: bool) -> Engine<Vec<u8>> {
    let mut engine =
        ground_firearm_fixture(135., false).with_collision_world(destruction_scene(wall));
    let sha = engine
        .spawn_catalog
        .as_ref()
        .unwrap()
        .source_sha256()
        .unwrap();
    let rules = character_damage_rules::Rules::from_json(&json!({
        "schema_version":1,"application_id":20231,"map_name":"synthetic_arena","game_type":1,
        "friendly_fire_multiplier":0.5,"provenance":{"spawn_catalog_sha256":sha,"source":"synthetic"}
    }).to_string()).unwrap();
    engine = engine.with_character_damage_rules(rules).unwrap();
    let runtime = engine.ground_runtime.as_mut().unwrap();
    let key = runtime.get(4).unwrap().key;
    runtime
        .configure(
            key,
            ground::Capabilities {
                destruction: Some(ground::Destruction {
                    damage: 70.,
                    full_radius: 8.,
                    half_radius: 4.,
                }),
                ..Default::default()
            },
        )
        .unwrap();
    let shooter = engine.active.as_mut().unwrap();
    let occupant = engine.peers.get_mut("synthetic-reservation-0000").unwrap();
    occupant.ground_seat = Some(ground_seats::PassengerSeat {
        vehicle_index: 4,
        seat: 0,
    });
    for actor in [occupant, &mut *shooter] {
        actor.reservation.character.account_id = Some(format!(
            "invented-ground-actor-{}",
            actor.reservation.player
        ));
        actor
            .reservation
            .character
            .loadouts
            .as_mut()
            .unwrap()
            .inventories[0][12] = Some(3);
        actor.current_inventory_handle = Some(replication::InventoryHandle {
            update_id: 1,
            inventory_index: 0,
        });
        actor.spawned_loadout = actor.reservation.character.replication_loadout(
            &engine.item_catalog,
            0,
            actor.wire_profile,
        );
    }
    shooter.tracking.server_position(
        replication::CharacterPosition {
            x: 1003.,
            y: 3.,
            z: 1000.,
        },
        player_tracking::PositionOrigin::ServerExit,
        3300,
    );
    if detached {
        let source = crate::objective_control::BotPlantSource {
            player: 1,
            allocation: 23,
            life: 1,
            team: 1,
            world: 1,
            round: engine.round_number,
        };
        let runtime = engine.ground_runtime.as_mut().unwrap();
        runtime
            .apply_blast_with_bot_origin(key, 135., u32::MAX, Some(source))
            .unwrap();
    } else {
        ground_hit(&mut engine, 3300).unwrap();
    }
    engine
        .tick_acquisition_objectives(Duration::from_millis(3300), 3)
        .unwrap();
    assert_eq!(
        engine
            .ground_runtime
            .as_ref()
            .unwrap()
            .get(4)
            .unwrap()
            .state,
        3
    );
    assert!(
        engine
            .ground_runtime
            .as_ref()
            .unwrap()
            .get(4)
            .unwrap()
            .health_quarter_points
            > 0
    );
    engine
}

#[test]
fn acquisition_ground_effects_critical_death_kills_occupant_and_recapture_restores_same_vehicle() {
    let mut engine = destruction_fixture(false);
    let key = engine.ground_runtime.as_ref().unwrap().get(4).unwrap().key;
    engine
        .tick_acquisition_objectives(Duration::from_millis(7300), 4)
        .unwrap();
    let ground = engine.ground_runtime.as_ref().unwrap().get(4).unwrap();
    assert_eq!(ground.key, key);
    assert_eq!(ground.state, 4);
    assert_eq!(ground.body, ground::Body::Inactive);
    let occupant = &engine.peers["synthetic-reservation-0000"];
    assert!(!occupant.life.is_alive());
    assert!(occupant.ground_seat.is_none());
    // The vehicle's death publishes the occupant leaving its seat.
    let terminal = crate::seated_death::tests::terminal_for(
        occupant.pending_shared.iter().map(Vec::as_slice),
        occupant.reservation.player,
        engine.wire_profile,
    );
    crate::seated_death::tests::assert_seated_exit(&terminal);
    assert_eq!(occupant.last_damage.last_attacker, 0x0380_0001);
    let world = engine.objective_world.as_ref().unwrap();
    assert_eq!(
        world.state(FIRST_CAPTURE),
        Some(ObjectiveState::Deactivated)
    );
    assert_eq!(
        world.state(FIRST_CAPTURE + 1),
        Some(ObjectiveState::Inactive)
    );
    let record = engine
        .writer
        .split(|byte| *byte == b'\n')
        .filter_map(|line| serde_json::from_slice::<Value>(line).ok())
        .find(|row| row["op"] == "acquisition_objective_tick" && row["timestamp_ms"] == 7300)
        .unwrap();
    let effects = record["effects"].as_array().unwrap();
    let ordered = [
        "KillOccupants",
        "DestructionExplosion",
        "RequestFspDeactivation",
        "RemovePhysics",
    ]
    .map(|phase| {
        effects
            .iter()
            .position(|row| {
                row["effect"]
                    .as_str()
                    .is_some_and(|effect| effect.starts_with(phase))
            })
            .unwrap()
    });
    assert!(ordered.windows(2).all(|pair| pair[0] < pair[1]));
    let victim_hp = engine
        .active
        .as_ref()
        .unwrap()
        .life
        .damage_health()
        .unwrap()
        .1;
    assert!(
        victim_hp > 0 && victim_hp < 400,
        "unoccluded on-foot player should take radial damage"
    );

    engine
        .tick_acquisition_objectives(Duration::from_millis(28_000), 5)
        .unwrap();
    assert_eq!(
        engine
            .ground_runtime
            .as_ref()
            .unwrap()
            .get(4)
            .unwrap()
            .state,
        4
    );
    let shooter = engine.active.take().unwrap();
    engine.peers.insert(shooter.reservation.id.clone(), shooter);
    engine.active = engine.peers.remove("synthetic-reservation-0000");
    let actor = engine.active.as_mut().unwrap();
    actor.life = CharacterLife::Alive {
        generation: 2,
        health_quarter_points: 400,
    };
    actor.tracking.server_position(
        replication::CharacterPosition {
            x: 100.,
            y: 3.,
            z: 100.,
        },
        player_tracking::PositionOrigin::ServerExit,
        28_100,
    );
    let request = protocol::ObjectiveInteraction {
        kind: protocol::ObjectiveInteractionKind::Activate,
        target_object_id: 0x0c80_0000 | u32::from(FIRST_CAPTURE),
        actor_index: 0,
        trigger_index: 0,
    };
    assert!(
        !dispatch(&mut engine, &request.encode(), 28_100)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        engine
            .objective_world
            .as_ref()
            .unwrap()
            .state(FIRST_CAPTURE),
        Some(ObjectiveState::Activating)
    );
    engine
        .tick_acquisition_objectives(Duration::from_millis(31_100), 6)
        .unwrap();
    engine.tick_acquisition_ground_registration(6).unwrap();
    let restored = engine.ground_runtime.as_ref().unwrap().get(4).unwrap();
    assert_eq!(restored.key, key);
    assert_eq!(restored.health_quarter_points, 600);
    assert!(engine.ground_vehicle_is_operational(4));
}

#[test]
fn acquisition_ground_effects_blocked_blast_and_failed_record_preserve_authority() {
    let mut engine = destruction_fixture(true);
    let ground = engine.ground_runtime.clone();
    engine.max_log = engine.written;
    assert_eq!(
        engine
            .tick_acquisition_objectives(Duration::from_millis(7300), 4)
            .err(),
        Some("log_limit_reached")
    );
    assert_eq!(engine.ground_runtime, ground);
    assert!(engine.peers["synthetic-reservation-0000"].life.is_alive());
    assert_eq!(
        engine
            .acquisition_gameplay
            .as_ref()
            .unwrap()
            .last_ground_update,
        Some(Duration::from_millis(3300))
    );

    let mut engine = destruction_fixture(true);
    engine
        .tick_acquisition_objectives(Duration::from_millis(7300), 4)
        .unwrap();
    assert!(!engine.peers["synthetic-reservation-0000"].life.is_alive());
    assert_eq!(
        engine
            .active
            .as_ref()
            .unwrap()
            .life
            .damage_health()
            .unwrap()
            .1,
        400
    );
}

#[test]
fn acquisition_ground_effects_delivery_inert_timer_advances_without_objective_changes() {
    let mut engine = delivery::delivery_fixture();
    delivery::position(&mut engine, 1000., 1040.);
    engine
        .tick_acquisition_objectives(Duration::from_millis(3400), 3)
        .unwrap();
    engine
        .tick_acquisition_objectives(Duration::from_millis(7400), 4)
        .unwrap();
    let v = engine.ground_runtime.as_ref().unwrap().get(4).unwrap();
    assert_eq!(v.state, 8);
    assert_eq!(v.inert_seconds, 8.);
    engine
        .tick_acquisition_objectives(Duration::from_millis(15400), 5)
        .unwrap();
    let v = engine.ground_runtime.as_ref().unwrap().get(4).unwrap();
    assert_eq!(v.state, 7);
    assert_eq!(v.body, ground::Body::Inactive);
}

#[test]
fn detached_bot_origin_reaches_ground_critical_occupant_blast_and_fsp() {
    let mut engine = destruction_fixture_with_bot_origin(false, true);
    let before = engine.ground_runtime.as_ref().unwrap().get(4).unwrap();
    // First 100 ms critical tick reduces 15 HP by 0.45 before projection.
    assert_eq!(before.health_quarter_points, 58);
    let origin = before.bot_plant_source.unwrap();
    assert_eq!(origin.team, 1);

    // Use the second existing synthetic Escort allocation and its own source
    // spawn identity. Place it within the first vehicle's inner blast radius.
    let runtime = engine.ground_runtime.as_mut().unwrap();
    let second = runtime.get(5).unwrap();
    let second_key = second.key;
    let respawn = ground::QualifiedRespawn {
        spawn_point_index: second.source_spawn_point_index,
        spawn_point_token: second.source_spawn_point_token,
        active: true,
        pose: ground::Pose {
            position: replication::CharacterPosition {
                x: 997.,
                y: 3.,
                z: 1000.,
            },
            rotation: second.pose.rotation,
        },
    };
    runtime
        .respawn_existing(second_key, respawn, false)
        .unwrap();
    runtime.update(second_key, 0., 0).unwrap();

    engine
        .tick_acquisition_objectives(Duration::from_millis(7300), 4)
        .unwrap();
    let ground = engine.ground_runtime.as_ref().unwrap().get(4).unwrap();
    assert_eq!(ground.state, 4);
    assert_eq!(ground.bot_plant_source, Some(origin));
    assert_eq!(ground.damage_wire().last_attacker, u32::MAX);
    let second = engine.ground_runtime.as_ref().unwrap().get(5).unwrap();
    assert_eq!(second.key, second_key);
    let second_hp = second.health_quarter_points;
    assert_eq!(second.bot_plant_source, Some(origin));
    assert_eq!(second.damage_wire().last_attacker, u32::MAX);
    let occupant = &engine.peers["synthetic-reservation-0000"];
    assert!(!occupant.life.is_alive());
    assert_eq!(occupant.last_damage.last_attacker, u32::MAX);
    let nearby = engine.active.as_ref().unwrap();
    assert!(nearby.life.damage_health().unwrap().1 < 400);
    assert_eq!(nearby.last_damage.last_attacker, u32::MAX);
    let record = engine
        .writer
        .split(|byte| *byte == b'\n')
        .filter_map(|line| serde_json::from_slice::<Value>(line).ok())
        .find(|row| row["op"] == "acquisition_objective_tick" && row["timestamp_ms"] == 7300)
        .unwrap();
    let effects = record["effects"].as_array().unwrap();
    let chained: Vec<_> = effects
        .iter()
        .filter(|row| {
            row["op"] == "acquisition_destruction_ground_blast"
                && row["target"] == second_key.object_id()
        })
        .collect();
    // This legacy destruction collector emits one dose for each overlapping
    // represented root. Each root observes the same model-origin falloff.
    assert_eq!(chained.len(), 2, "{chained:?}");
    assert_ne!(chained[0]["root"], chained[1]["root"]);
    assert!(chained.iter().all(|row| row["hp"] == 43.75));
    assert_eq!(second_hp, 250); // 150 HP minus two positive 43.75 HP root doses.
    assert!(effects.iter().any(|row| {
        row["op"] == "acquisition_ground_forced_fsp_source"
            && row["source_team"] == 1
            && row["attacker"] == u32::MAX
    }));
}

#[test]
fn detached_ground_damage_origin_survives_repair_and_changes_only_on_applied_damage() {
    let mut engine = ground_firearm_fixture(0., false);
    let source = crate::objective_control::BotPlantSource {
        player: 1,
        allocation: 23,
        life: 1,
        team: 1,
        world: 1,
        round: engine.round_number,
    };
    let ground = engine.ground_runtime.as_mut().unwrap();
    let key = ground.get(4).unwrap().key;
    ground
        .apply_blast_with_bot_origin(key, 50., u32::MAX, Some(source))
        .unwrap();
    assert_eq!(ground.get(4).unwrap().bot_plant_source, Some(source));
    ground.heal_percent(key, 2.).unwrap();
    assert_eq!(ground.get(4).unwrap().bot_plant_source, Some(source));
    assert!(
        ground
            .apply_explosion_damage(key, 0., 0x0380_0001)
            .unwrap()
            .is_empty()
    );
    assert_eq!(ground.get(4).unwrap().bot_plant_source, Some(source));
    ground.apply_explosion_damage(key, 1., 0x0380_0001).unwrap();
    assert_eq!(ground.get(4).unwrap().bot_plant_source, None);
    assert_eq!(
        ground.get(4).unwrap().damage_wire().last_attacker,
        0x0380_0001
    );
}
