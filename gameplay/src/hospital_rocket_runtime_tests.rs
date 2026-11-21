use super::*;

const WEAPON: hospital_rockets::Weapon = hospital_rockets::Weapon {
    refire_seconds: 1.25,
    reload_seconds: 3.5,
    infinite_ammo: true,
    magazine_size: 2,
    initial_magazine_count: 7,
};

#[test]
fn hospital_rocket_magazine_is_independent_of_authored_reserve_count_and_reloads() {
    let at = Duration::from_secs(1);
    let first = Magazine::after_shot(None, WEAPON, at).unwrap();
    assert_eq!(first.rounds, 1);
    assert_eq!(first.refill, None);
    assert_eq!(
        Magazine::after_shot(Some(first), WEAPON, at + Duration::from_secs(1)),
        Err("hospital_rocket_refire")
    );
    let second = Magazine::after_shot(Some(first), WEAPON, first.next_fire).unwrap();
    assert_eq!(second.rounds, 0);
    assert_eq!(
        Magazine::after_shot(Some(second), WEAPON, second.next_fire),
        Err("hospital_rocket_reloading")
    );
    let refill = second.refill.unwrap();
    let third = Magazine::after_shot(Some(second), WEAPON, refill).unwrap();
    assert_eq!(third.rounds, 1);
}

fn equipped_fixture() -> Engine<Vec<u8>> {
    let mut engine = structures::domination_bunker_seat_tests::fixture();
    let catalog = hospital_rockets::tests::synthetic_catalog();
    let weapon = catalog.variants()[0].secondary_item_id;
    engine.hospital_rockets = Some(std::sync::Arc::new(catalog));
    let actor = engine.active.as_mut().unwrap();
    actor.mounted_bunker = Some(5);
    actor.mounted_seat = Some(0);
    let turret = hospital_turrets::Runtime::new(weapon - 1, Some(weapon));
    let (turret, definition) = turret.mount(actor).unwrap();
    let item = replication::InventoryItem {
        inventory_update_id: definition.version,
        gear_slot: 0,
        ammo_pool: 0,
        ammo_count: 0,
        reloading: false,
    };
    let (turret, _) = turret
        .report(hospital_turret_wire::Fields {
            primary: Some(item),
            secondary: Some(replication::InventoryItem {
                gear_slot: 1,
                ..item
            }),
            ..Default::default()
        })
        .unwrap();
    engine
        .bunker_world
        .as_mut()
        .unwrap()
        .hospital_turrets
        .insert(5, turret);
    engine
}

pub(crate) fn launch_wire(engine: &Engine<Vec<u8>>, slot: u16, ammo: u8) -> Vec<u8> {
    let player = engine.active.as_ref().unwrap().reservation.player;
    let mut bits = replication::BitWriter::with_profile(engine.wire_profile);
    bits.write(0x0210, 16).unwrap();
    for vector in [1.25_f32, 2.5, 0.125, 0., 0., 0., 1., 0., 0., 0.] {
        bits.write(vector.to_bits(), 32).unwrap();
    }
    for (value, n) in [
        (u32::from(ammo), 8),
        (511, 10),
        (u32::MAX, 32),
        (0x03800000 | u32::from(player), 32),
        (u32::from(player), 16),
        (u32::from(slot), 16),
        (0, 6),
    ] {
        bits.write(value, n).unwrap();
    }
    bits.finish()
}

#[test]
fn hospital_rocket_retail_slot_three_creation_is_recorded_once() {
    let mut engine = equipped_fixture();
    let request = launch_wire(&engine, 3, 1);
    let packets = spawn_progression::dispatch(&mut engine, &request, 1000).unwrap();
    let creation_mask = packets
        .iter()
        .find_map(|packet| {
            let bytes = protocol::decode_hex(&packet.payload_hex);
            replication::ObjectUpdate::decode_with_profile(&bytes, engine.wire_profile)
                .ok()
                .map(|update| update.header.object_type_mask)
        })
        .expect("created after authenticated indexed hospital launch");
    assert_eq!(creation_mask, 1 << 19);
    let replay = spawn_progression::dispatch(&mut engine, &request, 1001).unwrap();
    assert!(replay.is_empty());
}

#[test]
fn hospital_rocket_retains_qualified_request_direction_inside_source_aim_cone() {
    let engine = equipped_fixture();
    let actor = engine.active.as_ref().unwrap();
    let mut request = protocol::FireGrenade {
        position: [1.25, 2.5, 0.125],
        rotation_xyzw: [0.0, 0.08715574, 0.0, 0.9961947],
        initial_velocity: [0.0; 3],
        ammo_index: 1,
        throw_power: 1.0,
        target_id: u32::MAX,
        shooter_id: 0x0380_0000 | u32::from(actor.reservation.player),
        shooter_client_index: actor.reservation.player,
        projectile_index: 3,
    };
    let now = Duration::from_secs(1);
    let planned = |request| {
        let LaunchDisposition::Created(launch) = engine
            .plan_hospital_rocket_launch(actor, request, &[0; REQUEST_BYTES], now, 5)
            .unwrap()
        else {
            panic!("new rocket");
        };
        let Advance::Flying(next) = launch
            .flight
            .motion
            .advance::<()>(1100, |_, _, _| Ok(None))
            .unwrap()
        else {
            panic!("rocket should be in flight");
        };
        next.position()
    };
    let point = planned(request);
    assert!((point[0] - (1.25 + 6. * 10_f64.to_radians().sin())).abs() < 1e-6);
    assert!((point[2] - (0.125 + 6. * 10_f64.to_radians().cos())).abs() < 1e-6);

    request.rotation_xyzw = request.rotation_xyzw.map(|component| -component);
    assert_eq!(planned(request), point);
}

pub(crate) fn tick_fixture() -> Engine<Vec<u8>> {
    use std::sync::Arc;

    let mut engine = equipped_fixture();
    let mut world = domination_gameplay_engine::tests::structural_repair::repair_fixture();
    engine.domination_gameplay = world.domination_gameplay.take();
    engine.domination_round = world.domination_round.take();
    engine.objective_catalog = world.objective_catalog.take();
    engine.objective_world = world.objective_world.take();
    engine.ground_runtime = Some(Default::default());
    engine.spawn_catalog = world.spawn_catalog.take();
    let mut human = human_kill_reward_tests::fixture();
    engine.item_catalog = human.item_catalog;
    let mut target = human.peers.remove("synthetic-reservation-0000").unwrap();
    target.reservation.id = "synthetic-rocket-target".into();
    target.reservation.player = 1;
    target.wire_profile = engine.wire_profile;
    target.life = CharacterLife::Alive {
        generation: 4,
        health_quarter_points: 400,
    };
    let spawns = engine.spawn_catalog.as_ref().unwrap();
    target.squad_catalog = Arc::new(spawns.for_squad(0).unwrap());
    target.owner_fields.position = None;
    target.tracking.server_position(
        CharacterPosition {
            x: 2.,
            y: 0.5,
            z: 2.5,
        },
        player_tracking::PositionOrigin::ServerSpawn,
        1,
    );
    let rules = character_damage_rules::Rules::from_json(
        &json!({
            "schema_version":1,"application_id":20231,"map_name":spawns.map_name(),
            "game_type":2,"friendly_fire_multiplier":0.25,"provenance":{
                "spawn_catalog_sha256":spawns.source_sha256(),"source":"synthetic"},
        })
        .to_string(),
    )
    .unwrap();
    engine = engine.with_character_damage_rules(rules).unwrap();
    let mut actor = engine.active.take().unwrap();
    actor.squad_catalog = Arc::new(
        engine
            .spawn_catalog
            .as_ref()
            .unwrap()
            .for_squad(16)
            .unwrap(),
    );

    let digest = "a".repeat(64);
    let body = dynamic_collision::Catalog::from_json(&json!({
        "format":"mag_dynamic_bodies","schema_version":1,
        "source":{"native_binding_sha256":digest,"shape_data_sha256":digest},
        "templates":[{"id":"synthetic-character","kind":"character","roots":[
            {"id":"capsule","layer":21,"dead_layer":6,"translation":[0.,0.,0.],"rotation":"identity",
             "shape":{"kind":"capsule","a":[0.,0.,0.],"b":[0.,2.,0.],"radius":0.25}}
        ]}],
    }).to_string()).unwrap();
    let body_hash = body.fingerprint().to_owned();
    engine = engine
        .with_dynamic_collision_bodies(
            body,
            dynamic_collision_runtime::RuntimeBindings {
                character_template: "synthetic-character".into(),
                vehicles: Vec::new(),
            },
        )
        .unwrap();
    engine.active = Some(actor);
    engine.peers.insert(target.reservation.id.clone(), target);

    let static_world = collision::CollisionWorld::from_bytes(
        json!({
            "format":"mag_collision_world","schema_version":1,"coordinate_space":"world",
            "metadata":{"map_name":engine.map_name(),"build":"synthetic",
                "coverage":"provisional","provenance":["synthetic"],"unknowns":[]},
            "instances":[{"id":"test-wall","vertices_xyz":[[0.,0.,3.],[2.5,0.,3.],[1.25,6.,3.]],
                "triangles":[[0,1,2]],"material_index":1,
                "collision":{"filter_info":9,"ray_child_filter_info":0,"convex_radius":0.,
                    "provenance":"synthetic","child_filter_policy":"uniform"}}]
        })
        .to_string()
        .as_bytes(),
        collision::CollisionLimits::default(),
    )
    .unwrap();
    let bindings = test_contact_bindings(&engine, static_world.source_sha256(), &body_hash);
    engine.collision_world = Some(static_world);
    let state = &mut engine.domination_gameplay.as_mut().unwrap().state;
    state.ground_respawn_physics = Some(Default::default());
    Arc::make_mut(&mut state.known_empty_vehicle_spawns).insert(2);
    state.filter_rows[6] = 1 << 21;
    state.filter_rows[23] = (1 << 9) | (1 << 21) | 1;
    state.filter_rows[24] = (1 << 9) | (1 << 21) | 1;
    state.projectile_contacts = Some(Arc::clone(&bindings));
    engine.projectile_contacts = Some(bindings);
    engine.enable_world_ticks();
    engine
}

pub(crate) fn test_contact_bindings(
    engine: &Engine<Vec<u8>>,
    static_hash: &str,
    body_hash: &str,
) -> std::sync::Arc<domination_projectile_collision::ContactBindings> {
    use std::sync::Arc;
    let digest = "a".repeat(64);
    let contact =
        |layer| json!({"root_filter_info":layer,"child_filter_info":0,"convex_radius":0.});
    let hospital: Vec<_> = (0..3)
        .map(|faction| {
            json!({
                "binding":{"asset_sha256":digest,"faction":faction,"packfile_sha256":digest,
                    "part_ids":["hospital-face"]},"contact":contact(22)
            })
        })
        .collect();
    Arc::new(domination_projectile_collision::ContactBindings::from_json(json!({
        "schema":"mag_domination_projectile_contacts_v1","map_name":engine.map_name(),
        "retail_elf_sha256":digest,"proof_sha256":digest,"static_world_sha256":static_hash,
        "dynamic_catalog_sha256":body_hash,"static_coverage_policy":"represented_reference_pose",
        "ground_phantom_roots":["wheel"],"sequence":[{
            "binding":{"source_sha256":digest,"neutral_sha256":digest,"collision_tag":9,
                "part_ids":["test-wall"]},"contact":contact(9)}],
        "hospital":hospital,"helo":[]
    }).to_string().as_bytes()).unwrap())
}

pub(crate) fn launch_for_tick(engine: &mut Engine<Vec<u8>>) {
    let request = launch_wire(engine, 3, 1);
    assert!(
        !spawn_progression::dispatch(engine, &request, 1000)
            .unwrap()
            .is_empty()
    );
    assert!(
        engine
            .active
            .as_ref()
            .unwrap()
            .hospital_rocket_flights
            .occupies(3)
    );
}

fn flight(engine: &Engine<Vec<u8>>) -> &Slot {
    engine
        .active
        .as_ref()
        .unwrap()
        .hospital_rocket_flights
        .slots[3]
        .as_ref()
        .unwrap()
}

fn audit(engine: &Engine<Vec<u8>>, op: &str) -> Vec<Value> {
    String::from_utf8_lossy(&engine.writer)
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|line| line["op"] == op)
        .collect()
}

#[test]
fn hospital_rocket_world_tick_applies_one_damage_and_orders_inactive_after_terminal() {
    let mut engine = tick_fixture();
    launch_for_tick(&mut engine);

    // An admitted projectile survives the shooter leaving the seat.
    let actor = engine.active.as_mut().unwrap();
    actor.mounted_seat = None;
    actor.mounted_bunker = None;
    actor.tracking.server_position(
        CharacterPosition {
            x: 40.,
            y: 0.5,
            z: 40.,
        },
        player_tracking::PositionOrigin::ServerCorrection,
        1010,
    );
    let response = engine.tick_world(Duration::from_millis(1050), 1, "rocket-contact".into());
    assert!(response.ok, "{:?}", response.error);

    let target = &engine.peers["synthetic-rocket-target"];
    let damage = target.last_damage;
    assert!(
        damage.health_quarter_points < 400,
        "damage={damage:?} terminal={:?} retired={:?}",
        audit(&engine, "hospital_rocket_terminal"),
        audit(&engine, "hospital_rocket_retired")
    );
    let rules = engine.item_catalog.character_blast_rules();
    assert_eq!(
        damage.last_location_and_type,
        (rules.damage_type << 4) | rules.hit_location
    );
    assert_eq!(damage.last_attacker, 0x0380_0000);
    assert_eq!(damage.last_source_object, 0x1380_0000);
    assert!(matches!(flight(&engine), Slot::Cleanup { .. }));
    assert!(
        engine
            .active
            .as_ref()
            .unwrap()
            .hospital_rocket_flights
            .snapshot_packets(1050, engine.wire_profile)
            .unwrap()
            .is_empty()
    );

    let terminals = audit(&engine, "hospital_rocket_terminal");
    assert_eq!(terminals.len(), 1);
    assert_eq!(
        terminals[0]["terminal"]["obstacle"]["instance"],
        "test-wall"
    );
    let terminal = protocol::decode_hex(terminals[0]["packet"].as_str().unwrap());
    let update =
        replication::ObjectUpdate::decode_with_profile(&terminal, engine.wire_profile).unwrap();
    let fields =
        projectile::decode_projectile_updates_with_profile(&update, engine.wire_profile).unwrap();
    assert_eq!(fields[0].fields.state, Some(2));
    assert_eq!(fields[0].fields.position.unwrap().z, 2.875);

    let early = engine.tick_world(
        Duration::from_millis(1500),
        1,
        "rocket-cleanup-early".into(),
    );
    assert!(early.ok, "{:?}", early.error);
    assert!(matches!(flight(&engine), Slot::Cleanup { .. }));
    let completed = engine.tick_world(Duration::from_millis(1550), 1, "rocket-cleanup".into());
    assert!(completed.ok, "{:?}", completed.error);
    assert!(
        !engine
            .active
            .as_ref()
            .unwrap()
            .hospital_rocket_flights
            .occupies(3)
    );
    assert_eq!(audit(&engine, "hospital_rocket_retired").len(), 1);
    assert_eq!(audit(&engine, "hospital_rocket_terminal").len(), 1);
    assert_eq!(
        engine.peers["synthetic-rocket-target"]
            .last_damage
            .health_quarter_points,
        damage.health_quarter_points
    );
}

#[test]
fn hospital_rocket_terminal_record_failure_is_atomic() {
    let mut engine = tick_fixture();
    launch_for_tick(&mut engine);
    let old_flight = flight(&engine).flight().motion;
    let prior_actor_queue = engine.active.as_ref().unwrap().pending_shared.clone();
    let prior_target_queue = engine.peers["synthetic-rocket-target"]
        .pending_shared
        .clone();
    let prior_target_life = engine.peers["synthetic-rocket-target"].life.damage_health();
    engine.max_log = engine.written;
    let result = engine.tick_hospital_rockets(Duration::from_millis(1050), 1);
    assert_eq!(result.err(), Some("log_limit_reached"));
    assert_eq!(flight(&engine).flight().motion, old_flight);
    assert_eq!(
        engine.active.as_ref().unwrap().pending_shared,
        prior_actor_queue
    );
    assert_eq!(
        engine.peers["synthetic-rocket-target"].pending_shared,
        prior_target_queue
    );
    assert_eq!(
        engine.peers["synthetic-rocket-target"].life.damage_health(),
        prior_target_life
    );
    assert!(audit(&engine, "hospital_rocket_terminal").is_empty());
}

#[test]
fn hospital_rocket_respawn_keeps_shooter_current_physical_body_excluded() {
    let mut engine = tick_fixture();
    launch_for_tick(&mut engine);
    let now = Duration::from_millis(1010);
    let actor = engine.active.as_mut().unwrap();
    actor
        .apply_peer_damage(
            1,
            replication::CharacterDamage {
                health_quarter_points: 0,
                ..actor.last_damage
            },
            now,
        )
        .unwrap();
    actor.life = CharacterLife::Alive {
        generation: 2,
        health_quarter_points: 400,
    };
    actor.tracking.server_position(
        CharacterPosition {
            x: 1.25,
            y: 0.5,
            z: 0.2,
        },
        player_tracking::PositionOrigin::ServerSpawn,
        1010,
    );
    assert!(matches!(flight(&engine), Slot::Flying(_)));
    engine
        .tick_hospital_rockets(Duration::from_millis(1050), 1)
        .unwrap();
    let terminals = audit(&engine, "hospital_rocket_terminal");
    assert_eq!(
        terminals.len(),
        1,
        "retired={:?} motion={:?}",
        audit(&engine, "hospital_rocket_retired"),
        audit(&engine, "hospital_rocket_motion")
    );
    assert_eq!(
        terminals[0]["terminal"]["obstacle"]["instance"],
        "test-wall"
    );
    assert_eq!(terminals[0]["source"]["life_generation"], 1);
}

#[test]
fn hospital_rocket_terminal_defers_on_capacity_while_fifo_can_drain() {
    let mut engine = tick_fixture();
    launch_for_tick(&mut engine);
    let actor = engine.active.as_mut().unwrap();
    let ordinary = replication::TeamSpawnDeadline {
        sequence: 1,
        timestamp_ms: 1020,
        object_index: 0,
        team: 0,
        deadline_ms: 3000,
    }
    .encode_with_profile(engine.wire_profile)
    .unwrap();
    for _ in 0..multiplayer::MAX_SHARED_QUEUE {
        actor.queue_shared(ordinary.clone());
    }

    let now = Duration::from_millis(1050);
    assert!(engine.tick_hospital_rockets(now, 1).unwrap().is_empty());
    assert!(matches!(flight(&engine), Slot::Flying(_)));
    assert!(audit(&engine, "hospital_rocket_terminal").is_empty());
    assert_eq!(
        engine.peers["synthetic-rocket-target"]
            .life
            .damage_health()
            .unwrap()
            .1,
        400
    );
    assert!(!engine.drain_shared(Vec::new(), 1).unwrap().is_empty());

    let response = engine.tick_world(Duration::from_millis(1100), 1, "rocket-resumed".into());
    assert!(response.ok, "{:?}", response.error);
    assert!(matches!(flight(&engine), Slot::Cleanup { .. }));
    assert_eq!(audit(&engine, "hospital_rocket_terminal").len(), 1);
}

#[test]
fn hospital_rocket_and_frag_reject_shared_wire_slot_conflicts_and_ignore_rocket_claims() {
    let mut engine = equipped_fixture();
    engine.item_catalog = human_kill_reward_tests::grenade_items_for_ground_test(10.);
    let actor = engine.active.as_mut().unwrap();
    let CharacterLife::Alive { generation, .. } = actor.life else {
        panic!("living fixture")
    };
    actor
        .projectiles
        .admit_item_with_catalog(0, generation, 900, 0, &[0; 57], 4, &engine.item_catalog)
        .unwrap();
    let conflict = launch_wire(&engine, 0, 1);
    assert!(
        spawn_progression::dispatch(&mut engine, &conflict, 950)
            .unwrap()
            .is_empty()
    );
    assert!(
        !engine
            .active
            .as_ref()
            .unwrap()
            .hospital_rocket_flights
            .occupies(0)
    );

    launch_for_tick(&mut engine);
    let header = projectile_header(3, 20, 1010);
    let mut mixed = replication::BitWriter::with_profile(engine.wire_profile);
    UpdateHeader {
        object_type_mask: header.object_type_mask | 1 << 16,
        ..header
    }
    .encode(&mut mixed)
    .unwrap();
    for (state, position) in [
        (
            None,
            CharacterPosition {
                x: 1.,
                y: 1.,
                z: 1.,
            },
        ),
        (
            Some(2),
            CharacterPosition {
                x: 100.,
                y: 1.,
                z: 100.,
            },
        ),
    ] {
        mixed.write(1, 1).unwrap();
        mixed.write(0, 16).unwrap();
        ProjectileOwnerFields {
            state,
            position: Some(position),
            rotation_xyzw: None,
        }
        .encode_fields(&mut mixed)
        .unwrap();
        mixed.write(0, 1).unwrap();
    }
    spawn_progression::dispatch(&mut engine, &mixed.finish(), 1010).unwrap();
    let actor = engine.active.as_ref().unwrap();
    assert_eq!(
        actor.projectiles.get(0).unwrap().last_position,
        Some(CharacterPosition {
            x: 1.,
            y: 1.,
            z: 1.
        })
    );
    assert_eq!(
        flight(&engine).flight().motion.position(),
        [1.25, 2.5, 0.125]
    );
    assert!(matches!(flight(&engine), Slot::Flying(_)));

    // Use the fourth retail slot through the ordinary on-foot dispatcher.
    let mut engine = equipped_fixture();
    engine.item_catalog = human_kill_reward_tests::grenade_items_for_ground_test(10.);
    let rocket = launch_wire(&engine, 3, 1);
    assert!(
        !spawn_progression::dispatch(&mut engine, &rocket, 1000)
            .unwrap()
            .is_empty()
    );
    let mut human = human_kill_reward_tests::fixture();
    let mut kit = human
        .active
        .as_mut()
        .unwrap()
        .spawned_loadout
        .take()
        .unwrap();
    kit.inventories[0].slots[4] = Some(4);
    let actor = engine.active.as_mut().unwrap();
    actor.mounted_bunker = None;
    actor.mounted_seat = None;
    actor.grenade_remaining = 1;
    actor.spawned_loadout = Some(kit);
    let frag = launch_wire(&engine, 3, 4);
    assert!(
        spawn_progression::dispatch(&mut engine, &frag, 1010)
            .unwrap()
            .is_empty()
    );
    let actor = engine.active.as_ref().unwrap();
    assert_eq!(actor.grenade_remaining, 1);
    assert!(actor.projectiles.get(3).is_none());
    let last: Value = serde_json::from_str(
        String::from_utf8_lossy(&engine.writer)
            .lines()
            .last()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(last["gameplay_denial"], "projectile_slot_in_use");
}

#[test]
fn retail_fourth_frag_occupancy_and_cleanup_block_hospital_rocket() {
    let mut engine = equipped_fixture();
    engine.item_catalog = human_kill_reward_tests::grenade_items_for_ground_test(10.);
    let mut human = human_kill_reward_tests::fixture();
    let mut kit = human
        .active
        .as_mut()
        .unwrap()
        .spawned_loadout
        .take()
        .unwrap();
    kit.inventories[0].slots[4] = Some(4);
    let actor = engine.active.as_mut().unwrap();
    actor.mounted_bunker = None;
    actor.mounted_seat = None;
    actor.grenade_remaining = 1;
    actor.spawned_loadout = Some(kit);
    let frag = launch_wire(&engine, 3, 4);
    let created = spawn_progression::dispatch(&mut engine, &frag, 1000).unwrap();
    assert_eq!(created.len(), 1);
    assert!(engine.active.as_ref().unwrap().projectiles.get(3).is_some());

    // Present the same qualified gunner grant while the on-foot frag occupies
    // the player's shared family-19 object identity.
    let actor = engine.active.as_mut().unwrap();
    actor.mounted_bunker = Some(5);
    actor.mounted_seat = Some(0);
    let rocket = launch_wire(&engine, 3, 1);
    assert!(
        spawn_progression::dispatch(&mut engine, &rocket, 1010)
            .unwrap()
            .is_empty()
    );
    let source_denial =
        |engine: &Engine<Vec<u8>>| audit(engine, "event").pop().unwrap()["gameplay_denial"].clone();
    assert_eq!(source_denial(&engine), "projectile_slot_in_use");

    let owner = ProjectileOwnerDelta {
        header: projectile_header(3, 20, 1100),
        update: &ProjectileOwnerUpdate {
            slot: 3,
            object_index: engine.active.as_ref().unwrap().reservation.player,
            fields: ProjectileOwnerFields {
                state: Some(2),
                position: Some(CharacterPosition {
                    x: 1.,
                    y: 2.,
                    z: 3.,
                }),
                rotation_xyzw: None,
            },
        },
    }
    .encode_with_profile(engine.wire_profile)
    .unwrap();
    spawn_progression::dispatch(&mut engine, &owner, 1100).unwrap();
    let actor = engine.active.as_ref().unwrap();
    assert!(actor.projectiles.get(3).unwrap().completed);
    assert!(engine.projectile_cleanup_pending(actor, 3));
    assert!(
        spawn_progression::dispatch(&mut engine, &rocket, 1101)
            .unwrap()
            .is_empty()
    );
    assert_eq!(source_denial(&engine), "projectile_cleanup_pending");
    assert!(
        !engine
            .active
            .as_ref()
            .unwrap()
            .hospital_rocket_flights
            .occupies(3)
    );
}

#[test]
fn hospital_rocket_release_queues_inactive_before_character_removal() {
    let mut engine = tick_fixture();
    launch_for_tick(&mut engine);
    let now = Duration::from_millis(1010);
    let source = engine.active.as_ref().unwrap().reservation.id.clone();
    // Use multiplayer request path and inspect the recipient's reliable order.
    engine.maximum_players = 4;
    let response = engine.handle(
        Request::Release {
            v: BRIDGE_VERSION,
            request_id: "release-rocket-shooter".into(),
            reservation_id: source,
        },
        now,
        1,
    );
    assert!(response.ok, "{:?}", response.error);
    let masks = response
        .messages
        .into_iter()
        .filter_map(|msg| {
            let wire = protocol::decode_hex(&msg.payload_hex);
            replication::ObjectUpdate::decode_with_profile(&wire, engine.wire_profile)
                .ok()
                .map(|update| update.header.object_type_mask)
        })
        .collect::<Vec<_>>();
    let inactive = masks.iter().position(|&mask| mask == 1 << 19).unwrap();
    let character = masks.iter().position(|&mask| mask == 1 << 3).unwrap();
    assert!(inactive < character);
    assert!(engine.active.is_none());
    assert!(audit(&engine, "hospital_rocket_terminal").is_empty());
}

#[test]
fn hospital_rocket_large_face_crossing_preserves_certified_contact() {
    use crate::{
        dynamic_collision::Pose,
        projectile_contact::{Coverage, TriangleId, TriangleObstacle, sweep_sphere},
    };

    let cast = |end| {
        let limits = QueryLimits::default();
        let mut budget = limits.max_support_points;
        sweep_sphere(
            SphereSweep {
                start: [1.25, 2.5, 0.125],
                end: [1.25, 2.5, end],
                radius: 0.125,
                associated_character: None,
            },
            Coverage::CompleteCurrent,
            &[TriangleObstacle {
                id: TriangleId::Static {
                    instance: "test-wall",
                    triangle: 0,
                },
                vertices: [[0., 0., 3.], [2.5, 0., 3.], [1.25, 6., 3.]],
                pose: Pose::at([0.; 3]),
                margin: 0.,
            }],
            &[],
            limits,
            &mut budget,
        )
        .unwrap()
    };
    let through = cast(3.125).unwrap();
    assert!((through.center[2] - 2.875).abs() < 1e-6);
    let endpoint = cast(2.875).unwrap();
    assert!((endpoint.fraction - 1.).abs() < 1e-7);
    assert!(cast(2.875 - 2e-8).is_none());
}

mod combined_helo_terminal {
    use super::*;
    const TARGET: &str = "synthetic-rocket-target";
    const NOW: Duration = Duration::from_millis(1050);

    fn fixture(enabled: bool) -> Engine<Vec<u8>> {
        let mut engine = crate::domination_structural_world::combined_ground_rocket_fixture(true);
        crate::domination_structural_world::hospital_rocket_helo_damage::tests::install_helo_fixture(
            &mut engine, CharacterPosition { x: 1.5, y: 2.5, z: 2.0 },
        );
        if enabled {
            engine
                .domination_gameplay
                .as_mut()
                .unwrap()
                .state
                .hospital_rocket_helo_areas = Some(std::sync::Arc::new(
                crate::hospital_rocket_helo::Catalog::synthetic(),
            ));
        }
        for actor in engine.active.iter_mut().chain(engine.peers.values_mut()) {
            actor.helo_creation_sent = true;
            actor.pending_shared.clear();
        }
        engine
    }
    fn health(engine: &Engine<Vec<u8>>) -> [u16; 3] {
        [
            engine.peers[TARGET].last_damage.health_quarter_points,
            engine
                .ground_runtime
                .as_ref()
                .unwrap()
                .get(0)
                .unwrap()
                .health_quarter_points,
            engine
                .domination_gameplay
                .as_ref()
                .unwrap()
                .state
                .helo
                .as_ref()
                .unwrap()
                .carriers[0]
                .damage
                .wire
                .health_quarter_points,
        ]
    }
    fn flight(engine: &Engine<Vec<u8>>) -> Flight {
        match engine
            .active
            .as_ref()
            .unwrap()
            .hospital_rocket_flights
            .slots[3]
            .as_ref()
            .unwrap()
        {
            Slot::Flying(flight) => flight.clone(),
            _ => panic!("expected the admitted flight"),
        }
    }
    fn finish(engine: &mut Engine<Vec<u8>>) -> Result<Vec<OutboundMessage>, &'static str> {
        // Isolate the shared terminal transaction. Motion/contact and source
        // bindings are covered by the separate full source replay.
        let flight = flight(engine);
        engine.finish_rocket(
            flight,
            End::Detonation(Terminal {
                position: [1.5, 2.5, 3.0],
                elapsed_ms: 50.0,
                reason: TerminalReason::AgeExpired,
            }),
            NOW,
            1,
        )
    }
    fn terminals(engine: &Engine<Vec<u8>>) -> Vec<Value> {
        engine
            .writer
            .split(|&byte| byte == b'\n')
            .filter_map(|line| serde_json::from_slice::<Value>(line).ok())
            .filter(|row| row["op"] == "hospital_rocket_terminal")
            .collect()
    }

    #[test]
    fn combined_helo_terminal_records_all_three_domains_and_respects_creation_order() {
        let mut engine = fixture(true);
        engine.active.as_mut().unwrap().helo_creation_sent = false;
        let before = health(&engine);
        finish(&mut engine).unwrap();
        let after = health(&engine);
        assert!(
            after
                .into_iter()
                .zip(before)
                .all(|(after, before)| after < before)
        );
        let audit = terminals(&engine);
        assert_eq!(audit.len(), 1);
        assert!(audit[0]["damage_denial"].is_null(), "{audit:?}");
        assert_eq!(audit[0]["helo_effects"].as_array().unwrap().len(), 1);
        let masks = |actor: &Active| {
            actor
                .pending_shared
                .iter()
                .map(|packet| {
                    replication::ObjectUpdate::decode_with_profile(packet, engine.wire_profile)
                        .unwrap()
                        .header
                        .object_type_mask
                })
                .collect::<Vec<_>>()
        };
        assert!(!masks(engine.active.as_ref().unwrap()).contains(&(1 << 2)));
        let observer = masks(&engine.peers[TARGET]);
        assert!(
            observer
                .windows(3)
                .any(|masks| masks == [1 << 1, 1 << 2, 1 << 19]),
            "{observer:?}"
        );
        let carrier = &engine
            .domination_gameplay
            .as_ref()
            .unwrap()
            .state
            .helo
            .as_ref()
            .unwrap()
            .carriers[0];
        assert_eq!(
            format!("{:?}", carrier.creation(1, 1050).base.damage),
            format!("{:?}", carrier.damage.wire)
        );
        assert_eq!(carrier.damage.wire.last_source_object, u32::MAX);
        assert!(matches!(
            engine
                .active
                .as_ref()
                .unwrap()
                .hospital_rocket_flights
                .slots[3],
            Some(Slot::Cleanup { .. })
        ));
    }

    #[test]
    fn combined_helo_terminal_recorder_refusal_preserves_health_queues_and_flight() {
        let mut engine = fixture(true);
        let before = health(&engine);
        let flying = format!("{:?}", flight(&engine));
        let queues = [
            engine.active.as_ref().unwrap().pending_shared.clone(),
            engine.peers[TARGET].pending_shared.clone(),
        ];
        engine.max_log = engine.written;
        assert_eq!(finish(&mut engine).err(), Some("log_limit_reached"));
        assert_eq!(health(&engine), before);
        assert_eq!(format!("{:?}", flight(&engine)), flying);
        assert_eq!(engine.active.as_ref().unwrap().pending_shared, queues[0]);
        assert_eq!(engine.peers[TARGET].pending_shared, queues[1]);
        assert!(terminals(&engine).is_empty());
    }

    #[test]
    fn combined_helo_terminal_reserves_the_extra_packet_then_retries_after_drain() {
        let fill = |engine: &mut Engine<Vec<u8>>| {
            let packet = replication::TeamSpawnDeadline {
                sequence: 1,
                timestamp_ms: 1020,
                object_index: 0,
                team: 0,
                deadline_ms: 3000,
            }
            .encode_with_profile(engine.wire_profile)
            .unwrap();
            let actor = engine.active.as_mut().unwrap();
            while actor.ordinary_shared_len() < multiplayer::MAX_SHARED_QUEUE - 3 {
                actor.queue_shared(packet.clone());
            }
        };
        let mut without = fixture(false);
        fill(&mut without);
        finish(&mut without).unwrap();
        assert_eq!(terminals(&without).len(), 1);
        let mut combined = fixture(true);
        fill(&mut combined);
        let before = health(&combined);
        let flying = format!("{:?}", flight(&combined));
        assert!(finish(&mut combined).unwrap().is_empty());
        assert_eq!(health(&combined), before);
        assert_eq!(format!("{:?}", flight(&combined)), flying);
        assert!(terminals(&combined).is_empty());
        assert!(!combined.drain_shared(Vec::new(), 1).unwrap().is_empty());
        finish(&mut combined).unwrap();
        assert_eq!(terminals(&combined).len(), 1);
        assert!(
            health(&combined)
                .into_iter()
                .zip(before)
                .all(|(after, before)| after < before)
        );
    }
}

mod hospital_blast_terminal {
    use super::*;
    use crate::domination_structural_world::domination_hospital_blast::tests::{
        TARGET as HOSPITAL, damage, install_hospital_fixture,
    };

    const PEER: &str = "synthetic-rocket-target";

    fn fixture(observer_created: bool) -> (Engine<Vec<u8>>, Flight) {
        let mut engine = tick_fixture();
        launch_for_tick(&mut engine);
        install_hospital_fixture(&mut engine, true);
        // The general flight fixture opts capsule layer 21 into its synthetic
        // material mask. The retail layer-24 blast ray excludes Character bodies.
        engine
            .domination_gameplay
            .as_mut()
            .unwrap()
            .state
            .filter_rows[24] &= !(1 << 21);
        let actor = engine.active.as_mut().unwrap();
        let Some(Slot::Flying(flight)) = &actor.hospital_rocket_flights.slots[3] else {
            panic!("expected the admitted flight");
        };
        let flight = flight.clone();
        actor.emplacement_creation.get_or_insert_with(Vec::new);
        let profile = engine.wire_profile;
        let peer = engine.peers.get_mut(PEER).unwrap();
        peer.emplacement_creation = observer_created.then(Vec::new);
        peer.tracking.server_position(
            CharacterPosition {
                x: 40.,
                y: 0.5,
                z: 40.,
            },
            player_tracking::PositionOrigin::ServerSpawn,
            1,
        );
        let packet = replication::TeamSpawnDeadline {
            sequence: 1,
            timestamp_ms: 1020,
            object_index: 0,
            team: 0,
            deadline_ms: 3000,
        }
        .encode_with_profile(profile)
        .unwrap();
        while peer.ordinary_shared_len() < multiplayer::MAX_SHARED_QUEUE - 1 {
            peer.queue_shared(packet.clone());
        }
        (engine, flight)
    }

    fn finish(
        engine: &mut Engine<Vec<u8>>,
        flight: Flight,
    ) -> Result<Vec<OutboundMessage>, &'static str> {
        engine.finish_rocket(
            flight,
            End::Detonation(Terminal {
                position: [3., 2.5, 1.],
                elapsed_ms: 50.,
                reason: TerminalReason::AgeExpired,
            }),
            Duration::from_millis(1050),
            1,
        )
    }

    #[test]
    fn rocket_hospital_blast_counts_only_creation_ready_recipients_and_keeps_firing_turret() {
        let (mut engine, flight) = fixture(false);
        assert_eq!(flight.hospital, HOSPITAL);
        let turret = engine.bunker_world.as_ref().unwrap().hospital_turrets[&HOSPITAL].clone();
        assert!(turret.rocket_magazine.is_some());
        finish(&mut engine, flight).unwrap();
        let terminal = engine
            .writer
            .split(|&byte| byte == b'\n')
            .rev()
            .filter_map(|line| serde_json::from_slice::<Value>(line).ok())
            .find(|row| row["op"] == "hospital_rocket_terminal")
            .expect("recorded rocket terminal");
        assert!(terminal["damage_denial"].is_null(), "{terminal}");
        assert!(damage(&engine).health_quarter_points < 800, "{terminal}");
        assert_eq!(damage(&engine).last_location_and_type, 0xa2);
        assert_eq!(damage(&engine).last_source_object, u32::MAX);
        assert_eq!(
            engine.bunker_world.as_ref().unwrap().hospital_turrets[&HOSPITAL],
            turret
        );
        let peer = &engine.peers[PEER];
        assert_eq!(peer.ordinary_shared_len(), multiplayer::MAX_SHARED_QUEUE);
        assert!(!peer.replication_backlogged);
        let last_mask = |actor: &Active| {
            replication::ObjectUpdate::decode_with_profile(
                actor.pending_shared.back().unwrap(),
                engine.wire_profile,
            )
            .unwrap()
            .header
            .object_type_mask
        };
        assert_eq!(last_mask(peer), 1 << 19);
        assert!(
            engine
                .active
                .as_ref()
                .unwrap()
                .pending_shared
                .iter()
                .any(|packet| {
                    replication::ObjectUpdate::decode_with_profile(packet, engine.wire_profile)
                        .is_ok_and(|row| row.header.object_type_mask == 1 << 13)
                })
        );

        let (mut engine, flight) = fixture(true);
        assert!(finish(&mut engine, flight.clone()).unwrap().is_empty());
        assert_eq!(damage(&engine).health_quarter_points, 800);
        assert!(matches!(
            engine
                .active
                .as_ref()
                .unwrap()
                .hospital_rocket_flights
                .slots[3],
            Some(Slot::Flying(_))
        ));
        engine.peers.get_mut(PEER).unwrap().pop_shared();
        finish(&mut engine, flight).unwrap();
        assert!(damage(&engine).health_quarter_points < 800);
        assert_eq!(
            engine.peers[PEER].ordinary_shared_len(),
            multiplayer::MAX_SHARED_QUEUE
        );
    }

    #[test]
    fn rocket_hospital_blast_recorder_refusal_preserves_flight_health_and_queues() {
        let (mut engine, flight) = fixture(false);
        let before = engine.active.as_ref().unwrap().pending_shared.clone();
        let peer_before = engine.peers[PEER].pending_shared.clone();
        engine.max_log = engine.written;
        assert_eq!(
            finish(&mut engine, flight).unwrap_err(),
            "log_limit_reached"
        );
        assert_eq!(damage(&engine).health_quarter_points, 800);
        let actor = engine.active.as_ref().unwrap();
        assert!(matches!(
            actor.hospital_rocket_flights.slots[3],
            Some(Slot::Flying(_))
        ));
        assert_eq!(actor.pending_shared, before);
        assert_eq!(engine.peers[PEER].pending_shared, peer_before);
    }
}
