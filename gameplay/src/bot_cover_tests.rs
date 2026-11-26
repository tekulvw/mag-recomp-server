//! SYNTHETIC cover checks on a generated floor with generated walls: one
//! human and one bot of the other team, a synthetic rifle and a navmesh baked
//! from the same generated geometry. No game data.
use super::*;
use crate::bot_cover::State;
use crate::development_target::BotPhase;
use crate::spawn_progression::ready;

/// A high wall south of the bot's start, along X, and a low wall farther
/// east. Heights are above the floor at y = 0.
const HIGH_WALL: ([f64; 3], [f64; 3]) = ([24., 0., 30.], [32., 2.6, 30.6]);
const LOW_WALL: ([f64; 3], [f64; 3]) = ([44., 0., 30.], [52., 0.9, 30.6]);

fn cuboid(min: [f64; 3], max: [f64; 3], base: u32) -> (Vec<[f64; 3]>, Vec<[u32; 3]>) {
    let v = (0..8)
        .map(|i| {
            [
                if i & 1 == 0 { min[0] } else { max[0] },
                if i & 2 == 0 { min[1] } else { max[1] },
                if i & 4 == 0 { min[2] } else { max[2] },
            ]
        })
        .collect();
    let faces = [
        [0, 1, 3, 2],
        [4, 6, 7, 5],
        [0, 4, 5, 1],
        [2, 3, 7, 6],
        [0, 2, 6, 4],
        [1, 5, 7, 3],
    ];
    let t = faces
        .iter()
        .flat_map(|f| [[f[0], f[1], f[2]], [f[0], f[2], f[3]]])
        .map(|t| t.map(|i| i + base))
        .collect();
    (v, t)
}

fn geometry() -> (Vec<[f64; 3]>, Vec<[u32; 3]>) {
    let mut vertices = vec![[0., 0., 0.], [60., 0., 0.], [60., 0., 60.], [0., 0., 60.]];
    let mut triangles = vec![[0, 2, 1], [0, 3, 2]];
    for (min, max) in [HIGH_WALL, LOW_WALL] {
        let (v, t) = cuboid(min, max, vertices.len() as u32);
        vertices.extend(v);
        triangles.extend(t);
    }
    (vertices, triangles)
}

fn reserve(engine: &mut Engine<Vec<u8>>) {
    let response = engine.handle(
        Request::Reserve {
            v: BRIDGE_VERSION,
            request_id: "synthetic-cover-reserve".into(),
            reservation_id: "synthetic-cover-0000".into(),
            world_id: 1,
            player_id: 0,
            account_id: 1,
            session_key: "aaaaaaaaaaaaaaaa".into(),
            access_key: "bbbbbbbbbbbbbbbb".into(),
            character: serde_json::from_value(json!({"name":"Synthetic Rifleman","faction":2,
                "head":0,"voice":0,"universe_id":26,"character_id":7}))
            .unwrap(),
        },
        Duration::ZERO,
        1,
    );
    assert!(response.ok, "{:?}", response.error);
}

fn place_human(engine: &mut Engine<Vec<u8>>, x: f32, z: f32) {
    engine.active.as_mut().unwrap().tracking.server_position(
        replication::CharacterPosition { x, y: 0., z },
        player_tracking::PositionOrigin::ServerSpawn,
        0,
    );
}

fn mesh_for(world: &collision::CollisionWorld, map: &str) -> navmesh::NavMesh {
    let (vertices, triangles) = geometry();
    let geometry_sha256 = {
        use sha2::{Digest, Sha256};
        let mut hash = Sha256::new();
        hash.update(b"MAG-NAV-GEOMETRY-v1\0");
        hash.update((vertices.len() as u32).to_le_bytes());
        hash.update((triangles.len() as u32).to_le_bytes());
        for v in vertices.iter().flatten() {
            hash.update(f32::to_le_bytes(*v as f32));
        }
        for i in triangles.iter().flatten() {
            hash.update(u32::to_le_bytes(*i));
        }
        format!("{:x}", hash.finalize())
    };
    let artifact = navmesh::bake(navmesh::BakeInput {
        format: "mag_navmesh_bake_input_v1".into(),
        map_name: map.into(),
        world_sha256: world.source_sha256().into(),
        geometry_sha256,
        bounds: navmesh::Bounds {
            min: [1., -1., 1.],
            max: [59., 4., 59.],
        },
        physics_regions: world.metadata().physics_regions.clone(),
        vertices: vertices.iter().map(|v| v.map(|c| c as f32)).collect(),
        triangles,
    })
    .unwrap();
    navmesh::NavMesh::load(artifact, world.source_sha256(), map).unwrap()
}

fn world() -> collision::CollisionWorld {
    let (vertices, triangles) = geometry();
    let floor = json!({"format":"mag_collision_world","schema_version":1,"coordinate_space":"world",
        "metadata":{"map_name":"synthetic_arena","build":"synthetic","coverage":"provisional",
            "provenance":["Synthetic cover arena"],"unknowns":["Not a game map"],
            "physics_regions":[{"min":[-1.,-5.,-1.],"max":[61.,8.,61.],"ground_verified":true,
                "obstacles_verified":true,"provenance":["Synthetic floor and walls"]}]},
        "instances":[{"id":"arena","vertices_xyz":vertices,"triangles":triangles}]});
    collision::CollisionWorld::from_bytes(
        &serde_json::to_vec(&floor).unwrap(),
        collision::CollisionLimits::default(),
    )
    .unwrap()
}

/// A bot standing at `at` on the arena, armed with the synthetic rifle, with
/// cover enabled (`crouch` for low cover).
fn fixture(prefix: &str, at: [f64; 2], crouch: bool) -> (Engine<Vec<u8>>, u16) {
    let mut engine = crate::bot_registry::identity_tests::engine(prefix);
    reserve(&mut engine);
    let source = engine.active.as_mut().unwrap();
    ready(source);
    source.life = CharacterLife::Alive {
        generation: 4,
        health_quarter_points: 400,
    };
    engine
        .reconcile_bot_population(Duration::ZERO, 1, &mut Default::default())
        .unwrap();
    let bot = engine.bot_combat_snapshots()[0].object_index;
    let mut armed = crate::human_kill_reward_tests::fixture();
    let equipped = armed.active.take().unwrap();
    engine.item_catalog = armed.item_catalog;
    let source = engine.active.as_mut().unwrap();
    source.current_inventory_handle = equipped.current_inventory_handle;
    source.spawned_loadout = equipped.spawned_loadout;
    source.observed_primary = equipped.observed_primary;
    source.primary_equipped = true;
    source.owner_fields.state = Some(replication::CharacterState {
        state: 1,
        stance: 0,
        user_data: [0; 3],
    });
    let kit = source.equipped_loadout().unwrap();
    let faction = source.reservation.character.faction;
    let b = engine.bot_registry.bots.get_mut(&bot).unwrap();
    b.target.faction = faction;
    b.target.loadout = kit;
    engine.item_catalog = crate::human_kill_reward_tests::bot_firearm_items();
    engine.match_started = true;
    let config = engine.bot_registry.config.as_mut().unwrap();
    config.firearms = crate::bot_firearms::Policy {
        reaction_ms: 1,
        turn_degrees_per_second: 720.,
        aim_tolerance_degrees: 15.,
        accuracy_per_mille: 0,
        far_accuracy_per_mille: 0,
        ..Default::default()
    };
    config.cover = crate::bot_cover::Policy {
        enabled: true,
        crouch,
        ..Default::default()
    };
    engine = engine.with_collision_world(world());
    let body = bot_physics::Body::default();
    let terrain =
        bot_physics::WorldTerrain::new(engine.collision_world.as_ref(), engine.map_name()).unwrap();
    let supported = terrain
        .advance(
            terrain.start_motion([at[0], 0.5, at[1]], body).unwrap(),
            [0.; 3],
            1.,
            body,
        )
        .unwrap();
    assert!(supported.grounded && !supported.blocked);
    let runtime = engine
        .bot_registry
        .bots
        .get_mut(&bot)
        .unwrap()
        .target
        .runtime
        .as_mut()
        .unwrap();
    runtime.physics_motion = Some(supported);
    runtime.position = replication::CharacterPosition {
        x: at[0] as f32,
        y: supported.position[1] as f32,
        z: at[1] as f32,
    };
    runtime.phase = BotPhase::Complete;
    let mesh = mesh_for(engine.collision_world.as_ref().unwrap(), engine.map_name());
    let b = engine.bot_registry.bots.get_mut(&bot).unwrap();
    b.move_attempted = true;
    (engine.with_navmesh_world(mesh), bot)
}

fn tick(engine: &mut Engine<Vec<u8>>, ms: u64) {
    let now = Duration::from_millis(ms);
    engine.tick_bot_perception(now, 1).unwrap();
    engine.tick_bot_cover(now, 1).unwrap();
    engine.tick_bots(now, 1).unwrap();
    engine.tick_bot_firearms(now, 1).unwrap();
}

fn position(engine: &Engine<Vec<u8>>, bot: u16) -> [f64; 3] {
    engine.bot_registry.bots[&bot]
        .target
        .runtime
        .unwrap()
        .physical_position()
}

fn records(engine: &Engine<Vec<u8>>, op: &str) -> Vec<Value> {
    String::from_utf8_lossy(&engine.writer)
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|row| row["op"] == op)
        .collect()
}

/// The human (object 0) hits the bot once.
fn shoot_bot(engine: &mut Engine<Vec<u8>>, bot: u16) {
    let b = engine.bot_registry.bots.get_mut(&bot).unwrap();
    b.damage.last_attacker = 0x0380_0000;
    b.damage_revision += 1;
}

fn in_state(engine: &Engine<Vec<u8>>, bot: u16, state: State) -> bool {
    engine
        .bot_cover
        .episode(bot)
        .is_some_and(|e| e.state() == state)
}

/// Whether the human at `human` (feet) sees the bot's torso, as a shot would.
fn human_sees_bot(engine: &Engine<Vec<u8>>, human: [f64; 3], bot: u16) -> bool {
    let feet = position(engine, bot);
    let (mut rays, mut work) = (4, usize::MAX);
    crate::bot_perception::clear_view(
        engine.collision_world.as_ref(),
        None,
        engine.map_name(),
        crate::bot_perception::torso(human),
        crate::bot_perception::torso_at(feet, engine.bot_cover.body_height(bot)),
        &mut rays,
        &mut work,
    )
    .unwrap()
}

#[test]
fn walls_yield_high_spots_with_peeks_and_low_spots() {
    let world = world();
    let mesh = mesh_for(&world, "synthetic_arena");
    let map = crate::bot_cover::build_static(&world, &mesh).unwrap();
    // North face of the high wall, protecting toward -Z (the wall).
    let north: Vec<_> = map
        .near([28., 0., 31.], 5.)
        .filter(|s| s.high && s.dir[1] < -0.9 && s.feet[2] > 30.6)
        .collect();
    assert!(!north.is_empty(), "high spots behind the wall");
    for s in &north {
        assert!(s.feet[0] > 23.5 && s.feet[0] < 32.5, "{s:?}");
        // Only spots near the wall's ends see past it.
        for p in s.peeks.iter().flatten() {
            assert!(p[0] < 25. || p[0] > 31., "peek past an end: {s:?}");
        }
    }
    assert!(north.iter().any(|s| s.peeks.iter().any(Option::is_some)));
    // The low wall gives low spots on both long faces.
    let low: Vec<_> = map.near([48., 0., 30.3], 4.).filter(|s| !s.high).collect();
    assert!(low.iter().any(|s| s.dir[1] < -0.9), "{low:?}");
    assert!(low.iter().any(|s| s.dir[1] > 0.9), "{low:?}");
    // Open floor and the arena's open edges are no cover.
    assert_eq!(map.near([10., 0., 10.], 5.).count(), 0);
    assert_eq!(map.near([2., 0., 58.], 2.).count(), 0);
}

#[test]
fn a_shot_bot_moves_behind_cover_that_blocks_the_shooter_then_peeks_and_fires() {
    // The bot stands in the open east of the high wall's end; the human is
    // south of the wall.
    let (mut engine, bot) = fixture("CV Shot", [34.5, 33.], false);
    let human = [28., 0., 14.];
    place_human(&mut engine, human[0] as f32, human[2] as f32);
    assert!(human_sees_bot(&engine, human, bot));
    let mut t = 0;
    tick(&mut engine, t);
    shoot_bot(&mut engine, bot);
    while t < 8000 && !in_state(&engine, bot, State::Hidden) {
        t += 50;
        tick(&mut engine, t);
    }
    let starts = records(&engine, "bot_cover_start");
    assert_eq!(starts.len(), 1, "{starts:?}");
    assert!(in_state(&engine, bot, State::Hidden), "reached cover");
    let e = engine.bot_cover.episode(bot).unwrap();
    assert!(e.spot().high && e.spot().feet[2] > 30.6, "{:?}", e.spot());
    assert!(!human_sees_bot(&engine, human, bot), "cover hides the bot");
    // From the peek spot the bot sees the human and fires.
    let fire = e.fire();
    let shots_before = records(&engine, "bot_firearm_shot").len();
    let mut fired_from = None;
    while t < 16_000 && fired_from.is_none() {
        t += 50;
        tick(&mut engine, t);
        let shots = records(&engine, "bot_firearm_shot");
        if shots.len() > shots_before {
            fired_from = Some(shots.last().unwrap()["origin"].clone());
        }
    }
    let origin = fired_from.expect("fires from cover");
    let origin: [f64; 3] = serde_json::from_value(origin).unwrap();
    assert!(
        (origin[0] - fire[0]).hypot(origin[2] - fire[2]) < 0.6,
        "fired from the peek spot {fire:?}: {origin:?}"
    );
    // ...and goes back behind the wall.
    let mut back = false;
    while t < 24_000 && !back {
        t += 50;
        tick(&mut engine, t);
        back = in_state(&engine, bot, State::Hidden) && !human_sees_bot(&engine, human, bot);
    }
    assert!(back, "returns behind cover");
}

#[test]
fn a_flanked_bot_relocates_to_cover_against_the_new_direction() {
    let (mut engine, bot) = fixture("CV Flank", [34.5, 33.], false);
    let mut human = [28., 0., 14.];
    place_human(&mut engine, human[0] as f32, human[2] as f32);
    let mut t = 0;
    tick(&mut engine, t);
    shoot_bot(&mut engine, bot);
    while t < 8000 && !in_state(&engine, bot, State::Hidden) {
        t += 50;
        tick(&mut engine, t);
    }
    assert!(engine.bot_cover.episode(bot).is_some());
    // The enemy comes round to the north side and hits the bot again.
    human = [28., 0., 50.];
    place_human(&mut engine, human[0] as f32, human[2] as f32);
    let flank_at = t;
    let mut relocated = false;
    while t < flank_at + 10_000 && !relocated {
        t += 50;
        tick(&mut engine, t);
        if t == flank_at + 100 {
            shoot_bot(&mut engine, bot);
        }
        relocated = engine
            .bot_cover
            .episode(bot)
            .is_some_and(|e| e.state() == State::Hidden && e.spot().dir[1] > 0.9);
    }
    let ends = records(&engine, "bot_cover_end");
    assert!(ends.iter().any(|e| e["reason"] == "flanked"), "{ends:?}");
    assert!(relocated, "{:?}", records(&engine, "bot_cover_start"));
    assert!(!human_sees_bot(&engine, human, bot));
}

#[test]
fn cover_ends_at_the_dwell_cap_and_the_bot_resumes_its_goal() {
    let (mut engine, bot) = fixture("CV Dwell", [34.5, 33.], false);
    let slot = engine.bot_registry.bots[&bot].slot;
    {
        let config = engine.bot_registry.config.as_mut().unwrap();
        config.cover.defender_dwell_ms = 3000;
        config.slots[slot].goal = Some(replication::CharacterPosition {
            x: 40.,
            y: 0.,
            z: 45.,
        });
    }
    let human = [28., 0., 14.];
    place_human(&mut engine, human[0] as f32, human[2] as f32);
    let mut t = 0;
    tick(&mut engine, t);
    shoot_bot(&mut engine, bot);
    while t < 20_000 && records(&engine, "bot_cover_end").is_empty() {
        t += 50;
        tick(&mut engine, t);
        // Keep the threat alive: the human keeps hitting the bot.
        if t % 1000 == 0 {
            shoot_bot(&mut engine, bot);
        }
    }
    let ends = records(&engine, "bot_cover_end");
    assert_eq!(ends.len(), 1, "{ends:?}");
    assert_eq!(ends[0]["reason"], "dwell", "{ends:?}");
    let ended = t;
    // The bot walks to its goal instead of staying in cover.
    let goal = [40., 45.];
    let away = |e: &Engine<Vec<u8>>| {
        let p = position(e, bot);
        (p[0] - goal[0]).hypot(p[2] - goal[1])
    };
    while t < ended + 15_000 && away(&engine) > 1.5 {
        t += 50;
        tick(&mut engine, t);
    }
    assert!(away(&engine) <= 1.5, "{:?}", position(&engine, bot));
}

#[test]
fn a_bot_crouches_behind_low_cover_and_stands_to_fire_over_it() {
    // East of the low wall's end, the human to the south.
    let (mut engine, bot) = fixture("CV Low", [54.5, 33.], true);
    let human = [48., 0., 14.];
    place_human(&mut engine, human[0] as f32, human[2] as f32);
    assert!(human_sees_bot(&engine, human, bot));
    let mut t = 0;
    tick(&mut engine, t);
    shoot_bot(&mut engine, bot);
    while t < 8000 && !in_state(&engine, bot, State::Hidden) {
        t += 50;
        tick(&mut engine, t);
    }
    let e = engine.bot_cover.episode(bot).expect("took low cover");
    assert!(!e.spot().high && e.spot().feet[2] > 30.6, "{:?}", e.spot());
    assert_eq!(engine.bot_registry.bots[&bot].stance, 1, "crouched");
    assert!(
        !human_sees_bot(&engine, human, bot),
        "the crouched bot is hidden"
    );
    // It stands up, visible to the human while it fires.
    while t < 16_000 && !in_state(&engine, bot, State::Exposed) {
        t += 50;
        tick(&mut engine, t);
    }
    assert!(in_state(&engine, bot, State::Exposed));
    assert_eq!(engine.bot_registry.bots[&bot].stance, 0);
    assert!(human_sees_bot(&engine, human, bot));
}

#[test]
fn cover_off_leaves_a_shot_bot_where_it_stands() {
    let (mut engine, bot) = fixture("CV Off", [34.5, 33.], false);
    engine.set_bot_cover_override(Some(false));
    let human = [28., 0., 14.];
    place_human(&mut engine, human[0] as f32, human[2] as f32);
    let start = position(&engine, bot);
    let mut t = 0;
    tick(&mut engine, t);
    shoot_bot(&mut engine, bot);
    while t < 4000 {
        t += 50;
        tick(&mut engine, t);
    }
    assert!(
        String::from_utf8_lossy(&engine.writer)
            .lines()
            .all(|l| !l.contains("bot_cover"))
    );
    let at = position(&engine, bot);
    assert!((at[0] - start[0]).hypot(at[2] - start[2]) < 0.2);
}
