//! SYNTHETIC perception checks on a generated floor: one human and one bot of
//! the other team, a synthetic rifle and a baked flat navmesh. No game data.
use super::*;
use crate::development_target::BotPhase;
use crate::spawn_progression::ready;

fn reserve(engine: &mut Engine<Vec<u8>>) {
    let response = engine.handle(
        Request::Reserve {
            v: BRIDGE_VERSION,
            request_id: "synthetic-perception-reserve".into(),
            reservation_id: "synthetic-perception-0000".into(),
            world_id: 1,
            player_id: 0,
            account_id: 1,
            session_key: "aaaaaaaaaaaaaaaa".into(),
            access_key: "bbbbbbbbbbbbbbbb".into(),
            character: serde_json::from_value(json!({"name":"Synthetic Listener","faction":2,
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

/// A bot standing on a flat synthetic floor, armed with the synthetic rifle,
/// facing a walk toward -Z. The human is placed by each test.
fn fixture(prefix: &str) -> (Engine<Vec<u8>>, u16) {
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
    assert_ne!(
        source.team(),
        engine.bot_registry.bots[&bot].target.squad.team()
    );
    let b = engine.bot_registry.bots.get_mut(&bot).unwrap();
    b.target.faction = faction;
    b.target.loadout = kit;
    engine.item_catalog = crate::human_kill_reward_tests::bot_firearm_items();
    engine.match_started = true;
    engine.bot_registry.config.as_mut().unwrap().firearms = crate::bot_firearms::Policy {
        reaction_ms: 1,
        turn_degrees_per_second: 720.,
        aim_tolerance_degrees: 15.,
        accuracy_per_mille: 1000,
        far_accuracy_per_mille: 1000,
        ..Default::default()
    };
    // A larger generated floor (positions are unsigned on the wire).
    let floor = json!({"format":"mag_collision_world","schema_version":1,"coordinate_space":"world",
        "metadata":{"map_name":"synthetic_arena","build":"synthetic","coverage":"provisional",
            "provenance":["Synthetic perception floor"],"unknowns":["Not a game map"],
            "physics_regions":[{"min":[-1.,-5.,-1.],"max":[61.,8.,61.],"ground_verified":true,
                "obstacles_verified":true,"provenance":["Synthetic floor"]}]},
        "instances":[{"id":"floor","vertices_xyz":[[0.,0.,0.],[60.,0.,0.],[60.,0.,60.],[0.,0.,60.]],
            "triangles":[[0,2,1],[0,3,2]]}]});
    engine = engine.with_collision_world(
        collision::CollisionWorld::from_bytes(
            &serde_json::to_vec(&floor).unwrap(),
            collision::CollisionLimits::default(),
        )
        .unwrap(),
    );
    // Stand the bot on the floor at (30, 0, 40).
    let body = bot_physics::Body::default();
    let terrain =
        bot_physics::WorldTerrain::new(engine.collision_world.as_ref(), engine.map_name()).unwrap();
    let supported = terrain
        .advance(
            terrain.start_motion([30., 0.5, 40.], body).unwrap(),
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
        x: 30.,
        y: supported.position[1] as f32,
        z: 40.,
    };
    let world = engine.collision_world.as_ref().unwrap();
    let vertices = vec![[1., 0., 1.], [59., 0., 1.], [59., 0., 59.], [1., 0., 59.]];
    let triangles = vec![[0, 2, 1], [0, 3, 2]];
    let geometry_sha256 = {
        use sha2::{Digest, Sha256};
        let mut hash = Sha256::new();
        hash.update(b"MAG-NAV-GEOMETRY-v1\0");
        hash.update(4u32.to_le_bytes());
        hash.update(2u32.to_le_bytes());
        for v in vertices.iter().flatten() {
            hash.update(f32::to_le_bytes(*v));
        }
        for i in triangles.iter().flatten() {
            hash.update(u32::to_le_bytes(*i));
        }
        format!("{:x}", hash.finalize())
    };
    let artifact = navmesh::bake(navmesh::BakeInput {
        format: "mag_navmesh_bake_input_v1".into(),
        map_name: engine.map_name().into(),
        world_sha256: world.source_sha256().into(),
        geometry_sha256,
        bounds: navmesh::Bounds {
            min: [1., -1., 1.],
            max: [59., 4., 59.],
        },
        physics_regions: world.metadata().physics_regions.clone(),
        vertices,
        triangles,
    })
    .unwrap();
    let mesh = navmesh::NavMesh::load(artifact, world.source_sha256(), engine.map_name()).unwrap();
    (engine.with_navmesh_world(mesh), bot)
}

/// Walk the bot from its slot toward -Z along its own x.
fn walk(engine: &mut Engine<Vec<u8>>, bot: u16, z: f32) -> replication::CharacterPosition {
    let mesh = engine.navmesh_world.clone().unwrap();
    let map = engine.map_name().to_owned();
    let b = engine.bot_registry.bots.get_mut(&bot).unwrap();
    let from = b.target.runtime.unwrap().position;
    let destination = replication::CharacterPosition {
        x: from.x,
        y: from.y,
        z,
    };
    b.target
        .begin_registry_move(
            mesh,
            &map,
            BotMove {
                mode: BotMoveMode::Navmesh,
                destination,
                duration_ms: 20_000,
                update_ms: 250,
                yaw: None,
                sprint: false,
            },
            Duration::ZERO,
            b.health,
            None,
        )
        .unwrap();
    b.move_attempted = true;
    // Already facing the walk (-Z in the retail yaw basis).
    b.yaw = 512;
    destination
}

fn tick(engine: &mut Engine<Vec<u8>>, ms: u64) {
    let now = Duration::from_millis(ms);
    engine.tick_bot_perception(now, 1).unwrap();
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

fn relocating(engine: &Engine<Vec<u8>>, bot: u16) -> bool {
    matches!(
        engine.bot_registry.bots[&bot].target.runtime.unwrap().phase,
        BotPhase::Relocating
    )
}

fn records(engine: &Engine<Vec<u8>>, op: &str) -> Vec<Value> {
    String::from_utf8_lossy(&engine.writer)
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|row| row["op"] == op)
        .collect()
}

fn human_dies(engine: &mut Engine<Vec<u8>>) {
    engine.active.as_mut().unwrap().life = CharacterLife::AwaitingRespawn { generation: 4 };
}

#[test]
fn enemy_crossing_the_view_interrupts_the_route_the_bot_engages_and_then_resumes() {
    let (mut engine, bot) = fixture("SP Sight");
    // Long enough for the threat to clear before the hold cap.
    engine
        .bot_registry
        .config
        .as_mut()
        .unwrap()
        .perception
        .max_hold_ms = 20_000;
    let start = position(&engine, bot);
    // Behind the walk and outside the all-round radius: not noticed.
    place_human(&mut engine, start[0] as f32, start[2] as f32 + 6.);
    let destination = walk(&mut engine, bot, 15.);
    let mut t = 0;
    while t < 1000 {
        tick(&mut engine, t);
        t += 50;
    }
    assert!(relocating(&engine, bot));
    assert!(position(&engine, bot)[2] < start[2] - 1., "still walking");
    assert!(
        records(&engine, "bot_perception_interrupt").is_empty(),
        "{:?}",
        records(&engine, "bot_perception_interrupt")
    );
    // The enemy steps into the forward cone, off the walking line.
    place_human(&mut engine, start[0] as f32 - 8., 20.);
    let crossed = t;
    let mut interrupted = None;
    while t < crossed + 3000 {
        tick(&mut engine, t);
        if interrupted.is_none() && !records(&engine, "bot_perception_interrupt").is_empty() {
            interrupted = Some((t, position(&engine, bot)));
        }
        t += 50;
    }
    let (at, held_at) = interrupted.expect("crossing enemy interrupts the route");
    let reaction = at - crossed;
    assert!((200..=900).contains(&reaction), "reaction {reaction} ms");
    let interrupt = &records(&engine, "bot_perception_interrupt")[0];
    assert_eq!(interrupt["cause"], "sight");
    assert_eq!(interrupt["enemy"], 0);
    let shots = records(&engine, "bot_firearm_shot");
    assert!(!shots.is_empty(), "the interrupted bot engages");
    assert!(shots.iter().all(|s| s["target"] == 0));
    assert!(relocating(&engine, bot), "the route is kept while engaging");
    let now = position(&engine, bot);
    assert!(
        (now[2] - held_at[2]).abs() < 0.3,
        "paused in place while engaging"
    );
    // The threat clears; after the memory period the walk resumes to the goal.
    human_dies(&mut engine);
    let cleared = t;
    while t < cleared + 20_000 && relocating(&engine, bot) {
        tick(&mut engine, t);
        t += 50;
    }
    let resumes = records(&engine, "bot_perception_resume");
    assert_eq!(resumes.len(), 1, "{resumes:?}");
    assert_eq!(resumes[0]["reason"], "threat_cleared");
    assert_eq!(records(&engine, "bot_perception_interrupt").len(), 1);
    let end = position(&engine, bot);
    assert!(
        (end[2] - f64::from(destination.z)).abs() < 1.,
        "resumed route reaches its goal: {end:?}"
    );
    assert!(
        records(&engine, "bot_move_stopped").is_empty()
            && !String::from_utf8_lossy(&engine.writer).contains("bot_navigation_deadline"),
        "the pause did not consume the move deadline"
    );
}

#[test]
fn an_enemy_behind_is_ignored_until_its_gunfire_turns_the_bot() {
    let (mut engine, bot) = fixture("SP Hearing");
    let start = position(&engine, bot);
    place_human(&mut engine, start[0] as f32 + 1., start[2] as f32 + 7.);
    walk(&mut engine, bot, 15.);
    let mut t = 0;
    while t < 1000 {
        tick(&mut engine, t);
        t += 50;
    }
    assert!(records(&engine, "bot_perception_interrupt").is_empty());
    assert!(records(&engine, "bot_firearm_shot").is_empty());
    // The human fires one round (its reported magazine falls).
    let primary = engine
        .active
        .as_mut()
        .unwrap()
        .observed_primary
        .as_mut()
        .unwrap();
    primary.ammo_count -= 1;
    let fired = t;
    while t < fired + 3000 {
        tick(&mut engine, t);
        t += 50;
    }
    let interrupts = records(&engine, "bot_perception_interrupt");
    assert_eq!(interrupts.len(), 1, "{interrupts:?}");
    assert_eq!(interrupts[0]["cause"], json!({"heard":"firearm"}));
    // Turned around, saw the shooter and engaged it.
    let shots = records(&engine, "bot_firearm_shot");
    assert!(
        !shots.is_empty(),
        "the bot turns toward the gunfire and engages"
    );
}

#[test]
fn perception_off_keeps_the_previous_walking_behaviour() {
    for config_switch in [false, true] {
        let (mut engine, bot) = fixture("SP Off");
        if config_switch {
            engine
                .bot_registry
                .config
                .as_mut()
                .unwrap()
                .perception
                .enabled = false;
        } else {
            engine.set_bot_perception_override(Some(false));
        }
        let start = position(&engine, bot);
        place_human(&mut engine, start[0] as f32 - 8., 20.);
        walk(&mut engine, bot, 15.);
        let mut t = 0;
        let mut last = start;
        while relocating(&engine, bot) && t < 20_000 {
            tick(&mut engine, t);
            let p = position(&engine, bot);
            assert!(p[2] <= last[2] + 1e-6, "never pauses or backs up");
            last = p;
            t += 50;
        }
        assert!(records(&engine, "bot_perception_interrupt").is_empty());
        assert!(
            records(&engine, "bot_firearm_shot").is_empty(),
            "no shots while walking"
        );
        assert!(engine.bot_perception.held_ids().is_empty());
    }
}

#[test]
fn a_suppressor_attachment_makes_gunfire_quieter() {
    let (engine, bot) = fixture("SP Muzzle");
    let items = engine.item_catalog.clone();
    let mut b = engine.bot_registry.bots[&bot].clone();
    assert_ne!(
        crate::bot_perception::bot_firearm_kind(&items, &b),
        crate::bot_perception::SoundKind::Suppressed
    );
    let index = usize::from(b.target.loadout.handle.inventory_index);
    b.target.loadout.inventories[index].slots[16] = Some(1);
    assert_eq!(
        crate::bot_perception::bot_firearm_kind(&items, &b),
        crate::bot_perception::SoundKind::Suppressed
    );
}

#[test]
fn sixty_four_bots_respect_the_per_tick_sight_budget() {
    let (mut engine, bot) = fixture("SP Budget");
    let template = engine.bot_registry.bots[&bot].clone();
    let policy = crate::bot_perception::Policy {
        proximity_m: 30.,
        max_sight_rays_per_tick: 16,
        ..Default::default()
    };
    engine.bot_registry.config.as_mut().unwrap().perception = policy.clone();
    engine.bot_registry.bots.clear();
    for i in 0..64u16 {
        let mut b = template.clone();
        b.target.object_index = 10 + i;
        b.target.squad = replication::SquadAddress::from_index(16 * (i % 2)).unwrap();
        let runtime = b.target.runtime.as_mut().unwrap();
        let mut motion = runtime.physics_motion.unwrap();
        motion.position[0] = 20. + f64::from(i % 8) * 2.;
        motion.position[2] = 20. + f64::from(i / 8) * 2.;
        runtime.physics_motion = Some(motion);
        engine.bot_registry.bots.insert(10 + i, b);
    }
    // Every bot sees many close enemies; only the budget limits the checks.
    // 64 bots at 16 rays per tick: the round robin must reach all of them
    // within this many ticks.
    const ROUND_ROBIN_TICKS: u64 = 20;
    let mut checked = std::collections::BTreeSet::new();
    let mut previous = 0;
    let mut slowest = Duration::ZERO;
    for step in 0..ROUND_ROBIN_TICKS {
        let started = Instant::now();
        engine
            .tick_bot_perception(Duration::from_millis(step * 50), 1)
            .unwrap();
        slowest = slowest.max(started.elapsed());
        let rays = engine.bot_perception.stats_rays();
        assert!(
            rays - previous <= policy.max_sight_rays_per_tick as u64,
            "tick {step} used {} rays",
            rays - previous
        );
        previous = rays;
        checked.extend(engine.bot_perception.noticed_ids());
    }
    assert!(previous > 0, "the budget was exercised");
    assert_eq!(
        checked.len(),
        64,
        "round robin reaches every bot within {ROUND_ROBIN_TICKS} ticks"
    );
    // Wall time depends on machine load, so it is reported, never asserted.
    eprintln!("slowest perception tick for 64 bots: {slowest:?}");
}

/// A bot standing still faces away from a human who hits it from 10 m behind,
/// without any gunfire the bot can hear: only the damage reveals the attacker.
fn hit_from_behind(perception: Option<bool>) -> (Vec<Value>, Vec<Value>, u64) {
    let (mut engine, bot) = fixture("SP Damage");
    engine.set_bot_perception_override(perception);
    // Ordinary engagement limits, so the primed reaction is what is measured.
    let firearms = &mut engine.bot_registry.config.as_mut().unwrap().firearms;
    firearms.reaction_ms = 800;
    firearms.turn_degrees_per_second = 90.;
    let start = position(&engine, bot);
    place_human(&mut engine, start[0] as f32, start[2] as f32 + 10.);
    // Facing -Z; the human is at +Z.
    engine.bot_registry.bots.get_mut(&bot).unwrap().yaw = 512;
    let mut t = 0;
    while t < 1000 {
        tick(&mut engine, t);
        t += 50;
    }
    assert!(records(&engine, "bot_firearm_shot").is_empty(), "unseen");
    let generation = engine.bot_registry.bots[&bot]
        .target
        .runtime
        .unwrap()
        .generation;
    let mut damage = engine.bot_registry.bots[&bot].damage;
    damage.health_quarter_points = 300;
    damage.last_attacker = 0x0380_0000;
    engine
        .commit_bot_damage(bot, generation, damage, Duration::from_millis(t))
        .unwrap();
    let hit = t;
    while t < hit + 4000 {
        tick(&mut engine, t);
        t += 50;
    }
    let first = records(&engine, "bot_firearm_shot")
        .first()
        .map_or(u64::MAX, |s| s["timestamp_ms"].as_u64().unwrap() - hit);
    (
        records(&engine, "bot_perception_interrupt"),
        records(&engine, "bot_firearm_shot"),
        first,
    )
}

#[test]
fn a_bot_hit_from_behind_turns_and_returns_fire() {
    let (interrupts, shots, first) = hit_from_behind(None);
    // Standing still: nothing to interrupt, but the bot faces its attacker.
    assert!(interrupts.is_empty(), "{interrupts:?}");
    assert!(!shots.is_empty(), "the bot returns fire");
    assert!(shots.iter().all(|s| s["target"] == 0));
    // Reaction 200 ms, a 180 degree turn at the attention rate (0.75 s), the
    // primed weapon delay and the final aim stay well under two seconds.
    assert!(first <= 2000, "first return shot after {first} ms");
    let (_, shots, _) = hit_from_behind(Some(false));
    assert!(shots.is_empty(), "without perception damage is not noticed");
}

#[test]
fn a_walking_bot_hit_from_behind_stops_to_fight() {
    let (mut engine, bot) = fixture("SP Damage Walk");
    let start = position(&engine, bot);
    place_human(&mut engine, start[0] as f32, start[2] as f32 + 10.);
    walk(&mut engine, bot, 15.);
    let mut t = 0;
    while t < 500 {
        tick(&mut engine, t);
        t += 50;
    }
    assert!(relocating(&engine, bot));
    let generation = engine.bot_registry.bots[&bot]
        .target
        .runtime
        .unwrap()
        .generation;
    let mut damage = engine.bot_registry.bots[&bot].damage;
    damage.health_quarter_points = 380;
    damage.last_attacker = 0x0380_0000;
    engine
        .commit_bot_damage(bot, generation, damage, Duration::from_millis(t))
        .unwrap();
    let hit = t;
    while t < hit + 2000 {
        tick(&mut engine, t);
        t += 50;
    }
    let interrupts = records(&engine, "bot_perception_interrupt");
    assert_eq!(interrupts.len(), 1, "{interrupts:?}");
    assert_eq!(interrupts[0]["cause"], "damaged");
    assert_eq!(interrupts[0]["enemy"], 0);
    let at = interrupts[0]["timestamp_ms"].as_u64().unwrap() - hit;
    assert!(at <= 300, "a hit uses the shortest reaction: {at} ms");
    assert!(!records(&engine, "bot_firearm_shot").is_empty());
}

#[test]
fn a_bot_standing_in_a_queue_may_fight_but_not_while_walking_to_it() {
    let (mut engine, bot) = fixture("SP Queue");
    let b = engine.bot_registry.bots.get_mut(&bot).unwrap();
    b.drop_waiting = Some(7);
    b.drop_holding_goal = b.target.runtime.map(|r| r.position);
    b.target.runtime.as_mut().unwrap().phase = BotPhase::Complete;
    assert!(crate::bot_firearms::safe(&engine.bot_registry.bots[&bot]));
    let b = engine.bot_registry.bots.get_mut(&bot).unwrap();
    b.target.runtime.as_mut().unwrap().phase = BotPhase::Relocating;
    assert!(!crate::bot_firearms::safe(&engine.bot_registry.bots[&bot]));
    // A passage in progress never fights.
    let b = engine.bot_registry.bots.get_mut(&bot).unwrap();
    b.target.runtime.as_mut().unwrap().phase = BotPhase::Complete;
    b.drop_committed = true;
    assert!(!crate::bot_firearms::safe(&engine.bot_registry.bots[&bot]));
}

#[test]
fn a_walking_bot_notices_an_enemy_at_rifle_range_a_little_later_than_a_near_one() {
    let mut reactions = Vec::new();
    for ahead in [8f32, 35.] {
        let (mut engine, bot) = fixture("SP Range");
        engine
            .bot_registry
            .config
            .as_mut()
            .unwrap()
            .perception
            .max_hold_ms = 20_000;
        let start = position(&engine, bot);
        place_human(&mut engine, start[0] as f32 + 1., start[2] as f32 - ahead);
        walk(&mut engine, bot, 2.);
        let mut t = 0;
        while t < 3000 && records(&engine, "bot_perception_interrupt").is_empty() {
            tick(&mut engine, t);
            t += 50;
        }
        let interrupt = records(&engine, "bot_perception_interrupt");
        assert_eq!(interrupt.len(), 1, "enemy {ahead} m ahead is seen");
        assert_eq!(interrupt[0]["cause"], "sight");
        assert!(interrupt[0]["distance_m"].as_f64().unwrap() <= f64::from(ahead) + 1.);
        reactions.push(interrupt[0]["timestamp_ms"].as_u64().unwrap());
    }
    assert!(
        reactions[1] > reactions[0],
        "farther enemies take longer to notice: {reactions:?}"
    );
}
