//! SYNTHETIC armed-objective checks: planner unit tests plus engine checks on a
//! generated floor, navmesh and objective catalog. No game data.
use super::*;
use crate::bot_registry::BotPopulation;
use crate::objective_catalog::ObjectiveCatalog;
use crate::replication::ObjectiveState;

fn candidate(id: u16, team: u8, squad: u16, x: f64, z: f64) -> Candidate {
    Candidate {
        id,
        generation: 1,
        team,
        squad,
        position: [x, 0., z],
        available: true,
    }
}

fn caps(defenders: usize, attackers: usize) -> ArmedPolicy {
    ArmedPolicy {
        defender_cap: defenders,
        attacker_cap: attackers,
        ..Default::default()
    }
}

fn armed(index: u16, x: f64, z: f64) -> Armed {
    Armed {
        index,
        center: [x, 0., z],
        defender_team: 1,
    }
}

fn plan(
    book: &mut Book,
    a: &[Armed],
    bots: &[Candidate],
    p: &ArmedPolicy,
) -> BTreeMap<u16, Assignment> {
    book.plan(a, bots, p, &mut Vec::new())
}

#[test]
fn defenders_converge_and_attackers_guard_nearest_squads_first() {
    // Squad 17 has the closest defender, so both its members join before squad 16.
    let bots = [
        candidate(1, 1, 16, 10., 0.),
        candidate(2, 1, 16, 12., 0.),
        candidate(3, 1, 17, 5., 0.),
        candidate(4, 1, 17, 30., 0.),
        candidate(5, 0, 0, 8., 0.),
        candidate(6, 0, 1, 20., 0.),
    ];
    let duties = plan(
        &mut Book::default(),
        &[armed(7, 0., 0.)],
        &bots,
        &caps(2, 1),
    );
    let converge: Vec<_> = duties
        .iter()
        .filter(|(_, d)| d.duty == Duty::Converge)
        .map(|(id, _)| *id)
        .collect();
    assert_eq!(converge, [3, 4]);
    assert_eq!(duties[&5].duty, Duty::Guard);
    assert!(!duties.contains_key(&6));
    assert!(duties.values().all(|d| d.objective == 7));
    // Each team's places are distinct and start at zero.
    assert_eq!(duties[&3].slot, 0);
    assert_eq!(duties[&4].slot, 1);
    assert_eq!(duties[&5].slot, 0);
}

#[test]
fn reach_and_availability_bound_who_may_join() {
    let mut busy = candidate(2, 1, 16, 1., 0.);
    busy.available = false;
    let bots = [
        candidate(1, 1, 16, 200., 0.),
        busy,
        candidate(3, 2, 16, 1., 0.),
    ];
    let duties = plan(
        &mut Book::default(),
        &[armed(7, 0., 0.)],
        &bots,
        &caps(8, 8),
    );
    assert!(duties.is_empty(), "{duties:?}");
}

#[test]
fn membership_is_sticky_and_refills_after_a_death() {
    let p = caps(1, 0);
    let mut book = Book::default();
    let mut bots = vec![candidate(1, 1, 16, 10., 0.), candidate(2, 1, 16, 20., 0.)];
    assert_eq!(
        plan(&mut book, &[armed(7, 0., 0.)], &bots, &p)
            .keys()
            .collect::<Vec<_>>(),
        [&1]
    );
    // Walking past a closer teammate never steals the duty mid-walk.
    bots[1].position = [1., 0., 0.];
    assert_eq!(
        plan(&mut book, &[armed(7, 0., 0.)], &bots, &p)
            .keys()
            .collect::<Vec<_>>(),
        [&1]
    );
    // A new life is a new bot for the book: the place is refilled.
    bots[0].generation = 2;
    let mut changes = Vec::new();
    let duties = book.plan(&[armed(7, 0., 0.)], &bots, &p, &mut changes);
    assert_eq!(duties.keys().collect::<Vec<_>>(), [&2]);
    assert_eq!(duties[&2].slot, 0);
    assert!(changes.iter().any(|c| c.bot == 1 && !c.joined));
    assert!(changes.iter().any(|c| c.bot == 2 && c.joined));
}

#[test]
fn ending_the_armed_state_releases_every_duty() {
    let bots = [candidate(1, 1, 16, 3., 0.), candidate(5, 0, 0, 8., 0.)];
    let mut book = Book::default();
    let p = caps(2, 2);
    assert_eq!(plan(&mut book, &[armed(7, 0., 0.)], &bots, &p).len(), 2);
    let mut changes = Vec::new();
    assert!(book.plan(&[], &bots, &p, &mut changes).is_empty());
    assert!(book.is_empty());
    assert_eq!(changes.iter().filter(|c| !c.joined).count(), 2);
    // Turning the feature off also releases a running episode.
    plan(&mut book, &[armed(7, 0., 0.)], &bots, &p);
    let off = ArmedPolicy {
        enabled: false,
        ..p
    };
    assert!(plan(&mut book, &[armed(7, 0., 0.)], &bots, &off).is_empty());
    assert!(book.is_empty());
}

fn sixty_four() -> Vec<Candidate> {
    (0..64u16)
        .map(|id| {
            let team = (id % 2) as u8;
            let squad = u16::from(team) * 16 + (id / 2) % 8;
            candidate(
                id + 1,
                team,
                squad,
                f64::from(id * 7 % 61),
                f64::from(id * 13 % 59),
            )
        })
        .collect()
}

#[test]
fn identical_inputs_give_identical_duties_in_any_order() {
    let bots = sixty_four();
    let mut reversed = bots.clone();
    reversed.reverse();
    let objectives = [armed(9, 50., 50.), armed(3, 5., 5.)];
    let p = ArmedPolicy::default();
    let a = plan(&mut Book::default(), &objectives, &bots, &p);
    let b = plan(
        &mut Book::default(),
        &[objectives[1], objectives[0]],
        &reversed,
        &p,
    );
    assert_eq!(a, b);
    assert!(!a.is_empty());
}

#[test]
fn sixty_four_bots_and_three_armed_objectives_respect_every_cap() {
    let bots = sixty_four();
    let objectives = [armed(1, 0., 0.), armed(2, 60., 0.), armed(3, 30., 58.)];
    let p = ArmedPolicy::default();
    let mut book = Book::default();
    let started = std::time::Instant::now();
    let mut duties = BTreeMap::new();
    for _ in 0..100 {
        duties = plan(&mut book, &objectives, &bots, &p);
    }
    for o in &objectives {
        let of = |duty: Duty| {
            duties
                .values()
                .filter(|d| d.objective == o.index && d.duty == duty)
                .count()
        };
        assert_eq!(of(Duty::Converge), p.defender_cap);
        assert_eq!(of(Duty::Guard), p.attacker_cap);
    }
    // One bot holds at most one duty, and most bots keep their normal lane.
    assert_eq!(duties.len(), 3 * (p.defender_cap + p.attacker_cap));
    // Bounded CPU: a hundred full re-plans of 64 bots stay far below one
    // 50 ms world tick even in an unoptimised test build.
    assert!(started.elapsed() < std::time::Duration::from_millis(250));
}

#[test]
fn duty_points_spread_on_their_rings() {
    let p = ArmedPolicy::default();
    let center = [10., 2., 10.];
    let ring = |duty: Duty, slot: u8, attempt: usize| {
        let q = duty_point(
            center,
            4,
            Assignment {
                objective: 4,
                duty,
                slot,
            },
            &p,
            attempt,
        );
        assert_eq!(q[1], 2.5);
        (q[0] - center[0]).hypot(q[2] - center[2])
    };
    for slot in 0..8 {
        for attempt in 0..6 {
            let r = ring(Duty::Converge, slot, attempt);
            assert!(r >= p.converge_radius_m[0] - 1e-9 && r <= p.converge_radius_m[1] + 1e-9);
            let r = ring(Duty::Guard, slot, attempt);
            assert!(r >= p.guard_radius_m[0] - 1e-9 && r <= p.guard_radius_m[1] + 1e-9);
        }
    }
    let at = |slot| {
        duty_point(
            center,
            4,
            Assignment {
                objective: 4,
                duty: Duty::Guard,
                slot,
            },
            &p,
            0,
        )
    };
    for a in 0..6u8 {
        for b in a + 1..6 {
            let (p, q) = (at(a), at(b));
            assert!(
                (p[0] - q[0]).hypot(p[2] - q[2]) > 2.,
                "slots {a} and {b} overlap"
            );
        }
    }
}

#[test]
fn bundled_policy_carries_the_defaults_and_rejects_bad_limits() {
    let bundled = crate::bot_objectives::ObjectivePolicy::bundled();
    assert_eq!(bundled.armed, ArmedPolicy::default());
    assert!(ArmedPolicy::default().validate());
    for bad in [
        ArmedPolicy {
            defender_cap: 33,
            ..Default::default()
        },
        ArmedPolicy {
            reach_m: 5.,
            ..Default::default()
        },
        ArmedPolicy {
            guard_radius_m: [9., 3.],
            ..Default::default()
        },
        ArmedPolicy {
            converge_radius_m: [f64::NAN, 3.],
            ..Default::default()
        },
    ] {
        assert!(!bad.validate());
    }
}

// ---- Engine checks -------------------------------------------------------

const PLANTED: u16 = 10;
const LANE: u16 = 11;
const CAPTURE: u16 = 12;
const PLANTED_AT: [f32; 3] = [60., 0., 60.];
const LANE_AT: [f32; 3] = [60., 0., 105.];

fn geometry_hash(vertices: &[[f64; 3]], triangles: &[[u32; 3]]) -> String {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    hash.update(b"MAG-NAV-GEOMETRY-v1\0");
    hash.update((vertices.len() as u32).to_le_bytes());
    hash.update((triangles.len() as u32).to_le_bytes());
    for x in vertices.iter().flatten() {
        hash.update((*x as f32).to_le_bytes());
    }
    for index in triangles.iter().flatten() {
        hash.update(index.to_le_bytes());
    }
    format!("{:x}", hash.finalize())
}

fn demolition(index: u16, at: [f32; 3], detonate_ms: u32) -> Value {
    json!({"token":format!("0x{index:08x}"),"object_index":index,"initial_state":1,
        "demolition":{"owner_team":1,"arm_ms":100,"detonate_ms":detonate_ms,"defuse_ms":200,
            "can_defuse":true,"activation_score":0,"deactivation_score":0,"position":at,
            "trigger_count":1,"maximum_distance_m":3.,"distance_policy":"development_point_distance"}})
}

fn objectives(can_deactivate: bool) -> ObjectiveCatalog {
    let objects = vec![
        demolition(PLANTED, PLANTED_AT, 30_000),
        demolition(LANE, LANE_AT, 30_000),
        json!({"token":format!("0x{CAPTURE:08x}"),"object_index":CAPTURE,"initial_state":1,
            "interaction":{"owner_team":1,"activate_ms":100,"complete_ms":20_000,"deactivate_ms":100,
                "can_deactivate":can_deactivate,"position":[20.,0.,20.],"trigger_count":1,
                "maximum_distance_m":3.,"distance_policy":"development_point_distance"}}),
    ];
    // Every squad's lane is LANE; nothing sends a squad to PLANTED or CAPTURE.
    ObjectiveCatalog::from_json(
        &json!({"schema_version":1,"format":"mag_retail_objective_catalog","application_id":20231,
        "protocol":protocol::ProtocolInfo::RETAIL_PROTOCOL,"game_type":0,"map_name":"synthetic_arena",
        "entity_family":12,"network_type":12,
        "source":{"authored_catalog_sha256":"0".repeat(64),"binding_capture_sha256":"1".repeat(64),
            "startup_proof_sha256":"2".repeat(64),"source_image_sha256":"3".repeat(64),
            "capture_qualification":"bounded_concurrent_token_slot_capture"},
        "objects":objects,
        "squads":(0..2u16).flat_map(|team| (0..4u16).map(move |slot| json!({
            "team":team,"squad_index":team*16+slot,"initial_objective_index":LANE,
            "authored_objectives":[{"object_index":LANE,"order":1},{"object_index":PLANTED,"order":2},
                {"object_index":CAPTURE,"order":3}]
        }))).collect::<Vec<_>>()})
        .to_string(),
    )
    .unwrap()
}

/// A 120 m generated floor and navmesh with `slots` as (team, squad, x, z).
fn engine(slots: &[(u8, u16, f32, f32)], can_deactivate: bool) -> Engine<Vec<u8>> {
    let items = crate::bot_registry::identity_tests::bootstrap_items();
    let names = crate::bot_names::Catalog::from_json(
        &serde_json::to_vec(&json!({"entries":(0..64).map(|i| json!({"name":format!("Synthetic {i}")})).collect::<Vec<_>>()}))
            .unwrap(),
    )
    .unwrap();
    // One authored on-foot point under every slot (slots bind to a nearby source spawn).
    let mut locations = vec![
        json!({"index":0,"token":100,"kind":"on_foot","position":{"x":10.,"y":0.,"z":10.}}),
        json!({"index":1,"token":101,"kind":"on_foot","position":{"x":110.,"y":0.,"z":110.}}),
    ];
    locations.extend(slots.iter().enumerate().map(|(i, (_, _, x, z))| {
        json!({"index":i+2,"token":102+i,"kind":"on_foot","position":{"x":x,"y":0.,"z":z}})
    }));
    let squads: Vec<_> = (0..2u16)
        .flat_map(|team| (0..4u16).map(move |slot| (team, team * 16 + slot)))
        .map(|(team, index)| {
            let mut available = vec![team as usize];
            available.extend(
                slots
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| s.1 == index)
                    .map(|(i, _)| i + 2),
            );
            json!({"index":index,"team":team,"platoon":0,"squad":index%16,
                "faction":if team==1 {Some(0)} else {None},"available":available,"default_spawn":team})
        })
        .collect();
    let spawns: spawn_catalog::SpawnCatalog = serde_json::from_value(json!({
        "format":"mag_spawn_catalog","schema_version":1,"source":"SYNTHETIC armed objectives",
        "active":{"map_name":"synthetic_arena","squad_index":0,"default_spawn":0,"available":[0]},
        "match_profile":{"game_type":0,"auto_assign_teams":true,"team_capacity":32,"squad_capacity":8,
            "has_bunkers":false,"has_objectives":true,"attacking_faction":1,"spawn_wave_ms":[100,100],
            "initial_spawn_delay_ms":[100,100]},
        "maps":[{"name":"synthetic_arena","owner_faction":0,"spawn_count":locations.len(),
            "runtime_indices_verified":true,"runtime_indices_basis":"SYNTHETIC pool identity",
            "locations":locations,"squads":squads}]}))
    .unwrap();
    let resources = EngineResources::for_application(
        std::sync::Arc::new(items),
        spawns,
        None,
        crate::resources::tests::wire_profiles(),
        20231,
    )
    .unwrap()
    .with_bot_names(names);
    let mut engine = Engine::from_resources(Vec::new(), 1_000_000, resources)
        .with_objective_catalog(objectives(can_deactivate))
        .unwrap();
    engine.maximum_players = 64;
    let floor = [
        [0., 0., 0.],
        [120., 0., 0.],
        [120., 0., 120.],
        [0., 0., 120.],
    ];
    let world = collision::CollisionWorld::from_bytes(
        &serde_json::to_vec(&json!({"format":"mag_collision_world","schema_version":1,
            "coordinate_space":"world",
            "metadata":{"map_name":"synthetic_arena","build":"synthetic","coverage":"provisional",
                "provenance":["Synthetic armed objective floor"],"unknowns":["Not a game map"],
                "physics_regions":[{"min":[-1.,-5.,-1.],"max":[121.,20.,121.],"ground_verified":true,
                    "obstacles_verified":true,"provenance":["Synthetic floor"]}]},
            "instances":[{"id":"floor","vertices_xyz":floor,"triangles":[[0,2,1],[0,3,2]]}]}))
        .unwrap(),
        collision::CollisionLimits::default(),
    )
    .unwrap();
    let walk = vec![
        [1., 0., 1.],
        [119., 0., 1.],
        [119., 0., 119.],
        [1., 0., 119.],
    ];
    let triangles = vec![[0, 2, 1], [0, 3, 2]];
    let artifact = navmesh::bake(navmesh::BakeInput {
        format: "mag_navmesh_bake_input_v1".into(),
        map_name: "synthetic_arena".into(),
        world_sha256: world.source_sha256().into(),
        geometry_sha256: geometry_hash(&walk, &triangles),
        bounds: navmesh::Bounds {
            min: [-1., -1., -1.],
            max: [121., 4., 121.],
        },
        physics_regions: world.metadata().physics_regions.clone(),
        vertices: walk.iter().map(|p| p.map(|x| x as f32)).collect(),
        triangles,
    })
    .unwrap();
    let mesh = navmesh::NavMesh::load(artifact, world.source_sha256(), "synthetic_arena").unwrap();
    engine = engine.with_collision_world(world).with_navmesh_world(mesh);
    let config: BotPopulation = serde_json::from_value(json!({"format":"mag_bot_population_v1",
        "map_name":"synthetic_arena","autonomous":true,"firearms":{"enabled":false},
        "slots":slots.iter().enumerate().map(|(i, (team, squad, x, z))| json!({"team":team,"squad":squad,
            "source_spawn":i+2,"position":{"x":x,"y":0.,"z":z}})).collect::<Vec<_>>()}))
    .unwrap();
    let mut engine = engine
        .with_bot_population(config, slots.len())
        .unwrap_or_else(|reason| panic!("{reason}"));
    engine
        .reconcile_bot_population(Duration::ZERO, 1, &mut Default::default())
        .unwrap();
    assert_eq!(engine.bot_registry.bots.len(), slots.len());
    // These duty fixtures start every bot at its own slot. (Each slot has its
    // own authored point here, so defending lives would otherwise all choose
    // the points nearest their objective; bot_spawn_selection_tests covers
    // that choice.) Settle every body on the floor, as the first movement
    // tick would.
    let body = bot_physics::Body::default();
    let terrain =
        bot_physics::WorldTerrain::new(engine.collision_world.as_ref(), engine.map_name()).unwrap();
    let homes = engine.bot_registry.config.as_ref().unwrap().slots.clone();
    let settled: Vec<_> = engine
        .bot_registry
        .bots
        .iter()
        .map(|(&id, b)| {
            let p = homes[b.slot].position;
            let start = terrain
                .start_motion([p.x.into(), 0.5, p.z.into()], body)
                .unwrap();
            (id, p, terrain.advance(start, [0.; 3], 1., body).unwrap())
        })
        .collect();
    for (id, home, motion) in settled {
        assert!(motion.grounded && !motion.blocked);
        let target = &mut engine.bot_registry.bots.get_mut(&id).unwrap().target;
        target.position = home;
        let r = target.runtime.as_mut().unwrap();
        r.physics_motion = Some(motion);
        r.position = home;
        r.position.y = motion.position[1] as f32;
    }
    engine
        .bot_registry
        .config
        .as_mut()
        .unwrap()
        .objective_policy
        .as_mut()
        .unwrap()
        .interactions_enabled = true;
    engine.match_started = true;
    engine
}

fn set_caps(engine: &mut Engine<Vec<u8>>, defenders: usize, attackers: usize) {
    let p = &mut engine
        .bot_registry
        .config
        .as_mut()
        .unwrap()
        .objective_policy
        .as_mut()
        .unwrap()
        .armed;
    p.defender_cap = defenders;
    p.attacker_cap = attackers;
}

fn plan_tick(engine: &mut Engine<Vec<u8>>, ms: u64) {
    let actors = engine.actor_grid(Duration::from_millis(ms)).unwrap();
    engine
        .tick_bot_objectives(Duration::from_millis(ms), 1, &actors)
        .unwrap();
}

fn bot_at(engine: &Engine<Vec<u8>>, x: f32, z: f32) -> u16 {
    *engine
        .bot_registry
        .bots
        .iter()
        .find(|(_, b)| {
            let p = b.target.runtime.unwrap().position;
            (p.x - x).abs() < 0.5 && (p.z - z).abs() < 0.5
        })
        .unwrap_or_else(|| panic!("no bot at {x},{z}"))
        .0
}

fn operators(engine: &Engine<Vec<u8>>) -> Vec<u16> {
    engine
        .bot_registry
        .bots
        .iter()
        .filter(|(_, b)| {
            b.task
                .is_operator_for(PLANTED, b.target.runtime.unwrap().generation)
        })
        .map(|(id, _)| *id)
        .collect()
}

fn records(engine: &Engine<Vec<u8>>, op: &str) -> Vec<Value> {
    engine
        .writer
        .split(|&b| b == b'\n')
        .filter(|r| !r.is_empty())
        .filter_map(|r| serde_json::from_slice::<Value>(r).ok())
        .filter(|r| r["op"] == op)
        .collect()
}

/// Two defender squads and two attacker squads; the near ones sit by PLANTED.
const SLOTS: [(u8, u16, f32, f32); 8] = [
    (1, 16, 64., 60.),
    (1, 16, 66., 60.),
    (1, 17, 90., 60.),
    (1, 17, 92., 60.),
    (0, 0, 54., 60.),
    (0, 0, 52., 60.),
    (0, 1, 20., 60.),
    (0, 1, 22., 60.),
];

#[test]
fn arming_retasks_the_nearest_defenders_with_one_defuser_and_attackers_guard() {
    let mut e = engine(&SLOTS, true);
    set_caps(&mut e, 2, 2);
    plan_tick(&mut e, 100);
    assert!(
        e.bot_registry
            .bots
            .values()
            .all(|b| b.task.objective == Some(LANE) && b.task.armed.is_none())
    );
    e.objective_world
        .as_mut()
        .unwrap()
        .development_arm(PLANTED, Duration::from_millis(150))
        .unwrap();
    plan_tick(&mut e, 200);
    let near_defenders = [bot_at(&e, 64., 60.), bot_at(&e, 66., 60.)];
    let near_attackers = [bot_at(&e, 54., 60.), bot_at(&e, 52., 60.)];
    for id in near_defenders.iter().chain(&near_attackers) {
        assert_eq!(
            e.bot_registry.bots[id].task.objective,
            Some(PLANTED),
            "bot {id}"
        );
    }
    for id in near_defenders {
        assert_eq!(
            e.bot_registry.bots[&id].task.armed.unwrap().duty,
            Duty::Converge
        );
    }
    for id in near_attackers {
        assert_eq!(
            e.bot_registry.bots[&id].task.armed.unwrap().duty,
            Duty::Guard
        );
        assert!(!e.bot_registry.bots[&id].task.operator);
    }
    // Guards stand within the contest radius, so nobody starts a defuse yet.
    assert!(operators(&e).is_empty());
    // The far squads keep their lane: the caps hold.
    for (x, z) in [(90., 60.), (92., 60.), (20., 60.), (22., 60.)] {
        let b = &e.bot_registry.bots[&bot_at(&e, x, z)];
        assert_eq!(b.task.objective, Some(LANE));
        assert!(b.task.armed.is_none());
    }
    assert_eq!(
        records(&e, "bot_armed_objective_duty")
            .iter()
            .filter(|r| r["joined"] == true)
            .count(),
        4
    );
    // Guards already covering the charge hold their spot; the converging
    // defender plans a place on its ring.
    for _ in 0..20 {
        plan_tick(&mut e, 250);
    }
    let center = PLANTED_AT.map(f64::from);
    let p = ArmedPolicy::default();
    let radius = |id: &u16| {
        let g = e.bot_registry.bots[id]
            .task
            .role_goal
            .expect("planned duty goal");
        (f64::from(g.x) - center[0]).hypot(f64::from(g.z) - center[2])
    };
    assert!(radius(&near_defenders[1]) <= p.converge_radius_m[1] + 0.5);
    for (id, start) in near_attackers.iter().zip([54., 52.]) {
        let g = e.bot_registry.bots[id]
            .task
            .role_goal
            .expect("planned guard goal");
        assert!(
            (g.x - start).abs() < 0.6 && (g.z - 60.).abs() < 0.6,
            "{g:?}"
        );
        assert!(radius(id) <= p.guard_radius_m[1]);
    }
    let tasks: Vec<_> = records(&e, "bot_objective_task")
        .into_iter()
        .filter(|r| r["objective"] == PLANTED)
        .collect();
    assert!(tasks.iter().any(|r| r["destination_role"] == "armed_guard"));
}

#[test]
fn defenders_start_a_defuse_once_clear_or_in_the_last_seconds() {
    // Contested: attackers stand by the charge. Only the late window opens it.
    let mut e = engine(&SLOTS, true);
    set_caps(&mut e, 2, 2);
    plan_tick(&mut e, 100);
    e.objective_world
        .as_mut()
        .unwrap()
        .development_arm(PLANTED, Duration::from_millis(150))
        .unwrap();
    plan_tick(&mut e, 200);
    assert!(!e.disarm_open(PLANTED, Duration::from_millis(200), &ArmedPolicy::default()));
    assert!(operators(&e).is_empty());
    // The fuse ends at 30.15 s; 8 s before that a defender goes in anyway.
    plan_tick(&mut e, 22_000);
    assert!(operators(&e).is_empty());
    plan_tick(&mut e, 22_200);
    assert_eq!(operators(&e), [bot_at(&e, 64., 60.)]);

    // Clear: no attacker near the charge. The nearest defender goes at once,
    // and exactly one defender is the operator.
    let clear: Vec<_> = SLOTS
        .iter()
        .map(|&(team, squad, x, z)| {
            (
                team,
                squad,
                if team == 0 && squad == 0 { x - 40. } else { x },
                z,
            )
        })
        .collect();
    let mut e = engine(&clear, true);
    set_caps(&mut e, 2, 2);
    plan_tick(&mut e, 100);
    e.objective_world
        .as_mut()
        .unwrap()
        .development_arm(PLANTED, Duration::from_millis(150))
        .unwrap();
    plan_tick(&mut e, 200);
    assert!(e.disarm_open(PLANTED, Duration::from_millis(200), &ArmedPolicy::default()));
    assert_eq!(operators(&e), [bot_at(&e, 64., 60.)]);
}

#[test]
fn guards_prefer_the_disarming_defender_without_leaving_their_post() {
    let mut e = engine(&SLOTS, true);
    set_caps(&mut e, 2, 2);
    plan_tick(&mut e, 100);
    e.objective_world
        .as_mut()
        .unwrap()
        .development_arm(PLANTED, Duration::from_millis(150))
        .unwrap();
    plan_tick(&mut e, 200);
    let defuser = bot_at(&e, 64., 60.);
    let generation = e.bot_registry.bots[&defuser]
        .target
        .runtime
        .unwrap()
        .generation;
    let defuse = e.objective_world.as_mut().unwrap().interact(
        protocol::ObjectiveInteraction {
            target_object_id: 0x0c80_0000 | u32::from(PLANTED),
            kind: protocol::ObjectiveInteractionKind::Deactivate,
            actor_index: defuser,
            trigger_index: 0,
        },
        crate::objective_control::ObjectiveActor {
            player: defuser,
            team: 1,
            generation,
            position: replication::CharacterPosition {
                x: PLANTED_AT[0] + 1.,
                y: 0.,
                z: PLANTED_AT[2],
            },
        },
        Duration::from_millis(250),
    );
    assert!(defuse.accepted, "{}", defuse.reason);
    assert_eq!(
        e.objective_world.as_ref().unwrap().state(PLANTED),
        Some(ObjectiveState::Deactivating)
    );
    let guards = [bot_at(&e, 54., 60.), bot_at(&e, 52., 60.)];
    let before: Vec<_> = guards
        .iter()
        .map(|id| e.bot_registry.bots[id].task.armed)
        .collect();
    plan_tick(&mut e, 300);
    for (id, duty) in guards.iter().zip(before) {
        let b = &e.bot_registry.bots[id];
        // The duty (and so the planned post) is unchanged by the disarm.
        assert_eq!(b.task.armed, duty);
        assert_eq!(b.task.armed.unwrap().duty, Duty::Guard);
        assert_eq!(e.armed_priority_target(b), Some(defuser));
    }
    // Defenders never get a priority target from the guard rule.
    assert_eq!(
        e.armed_priority_target(&e.bot_registry.bots[&defuser]),
        None
    );
}

#[test]
fn defuse_or_detonation_returns_both_teams_to_their_lane() {
    for detonate in [false, true] {
        let mut e = engine(&SLOTS, true);
        set_caps(&mut e, 2, 2);
        plan_tick(&mut e, 100);
        e.objective_world
            .as_mut()
            .unwrap()
            .development_arm(PLANTED, Duration::from_millis(150))
            .unwrap();
        plan_tick(&mut e, 200);
        assert_eq!(
            e.bot_registry
                .bots
                .values()
                .filter(|b| b.task.armed.is_some())
                .count(),
            4
        );
        let world = e.objective_world.as_mut().unwrap();
        if detonate {
            // The fuse runs out: the charge completes.
            world.tick(Duration::from_millis(30_200), &Default::default());
            assert_eq!(world.state(PLANTED), Some(ObjectiveState::Completed));
        } else {
            let defuser = 1000;
            let actor = crate::objective_control::ObjectiveActor {
                player: defuser,
                team: 1,
                generation: 1,
                position: replication::CharacterPosition {
                    x: PLANTED_AT[0],
                    y: 0.,
                    z: PLANTED_AT[2],
                },
            };
            let request = protocol::ObjectiveInteraction {
                target_object_id: 0x0c80_0000 | u32::from(PLANTED),
                kind: protocol::ObjectiveInteractionKind::Deactivate,
                actor_index: defuser,
                trigger_index: 0,
            };
            assert!(
                world
                    .interact(request, actor, Duration::from_millis(250))
                    .accepted
            );
            world.tick(
                Duration::from_millis(600),
                &BTreeMap::from([(defuser, actor)]),
            );
            assert_eq!(world.state(PLANTED), Some(ObjectiveState::Deactivated));
        }
        plan_tick(&mut e, 30_300);
        for b in e.bot_registry.bots.values() {
            assert!(b.task.armed.is_none());
            assert_eq!(b.task.objective, Some(LANE));
            assert!(
                !b.task.operator || b.target.squad.team() == 0,
                "only the lane's planter may operate"
            );
        }
        assert_eq!(
            records(&e, "bot_armed_objective_duty")
                .iter()
                .filter(|r| r["joined"] == false)
                .count(),
            4
        );
    }
}

#[test]
fn sabotage_capture_defenders_deactivate_only_when_the_catalog_allows_it() {
    for can_deactivate in [true, false] {
        let mut e = engine(&SLOTS, can_deactivate);
        let world = e.objective_world.as_mut().unwrap();
        let attacker = crate::objective_control::ObjectiveActor {
            player: 1000,
            team: 0,
            generation: 1,
            position: replication::CharacterPosition {
                x: 20.,
                y: 0.,
                z: 20.,
            },
        };
        let request = protocol::ObjectiveInteraction {
            target_object_id: 0x0c80_0000 | u32::from(CAPTURE),
            kind: protocol::ObjectiveInteractionKind::Activate,
            actor_index: 1000,
            trigger_index: 0,
        };
        assert!(
            world
                .interact(request, attacker, Duration::from_millis(100))
                .accepted
        );
        world.tick(
            Duration::from_millis(300),
            &BTreeMap::from([(1000, attacker)]),
        );
        assert_eq!(world.state(CAPTURE), Some(ObjectiveState::Activated));
        let world = e.objective_world.as_ref().unwrap();
        let object = e
            .objective_catalog
            .as_ref()
            .unwrap()
            .objects
            .iter()
            .find(|o| o.object_index == CAPTURE)
            .unwrap();
        let defender =
            crate::bot_objectives::operator_action(object, ObjectiveState::Activated, 1, world)
                .map(|(kind, _)| kind);
        assert_eq!(
            defender,
            can_deactivate.then_some(protocol::ObjectiveInteractionKind::Deactivate)
        );
        assert!(
            crate::bot_objectives::operator_action(object, ObjectiveState::Activated, 0, world)
                .is_none()
        );
        // Only a reversible capture counts as armed.
        let armed: Vec<_> = e.armed_objectives().iter().map(|a| a.index).collect();
        assert_eq!(
            armed,
            if can_deactivate {
                vec![CAPTURE]
            } else {
                vec![]
            }
        );
    }
}

#[test]
fn sixty_four_bots_retask_inside_the_shared_planner_budget() {
    let slots: Vec<_> = (0..64u16)
        .map(|i| {
            let team = (i % 2) as u8;
            let squad = u16::from(team) * 16 + (i / 2) % 4;
            (
                team,
                squad,
                10. + f32::from(i % 8) * 12.,
                10. + f32::from(i / 8) * 12.,
            )
        })
        .collect();
    let mut e = engine(&slots, true);
    for tick in 0..10 {
        plan_tick(&mut e, 100 + tick * 50);
    }
    e.objective_world
        .as_mut()
        .unwrap()
        .development_arm(PLANTED, Duration::from_millis(700))
        .unwrap();
    let before = records(&e, "bot_planning_slice").len();
    for tick in 0..40 {
        plan_tick(&mut e, 750 + tick * 50);
    }
    let p = ArmedPolicy::default();
    let duty = |d: Duty| {
        e.bot_registry
            .bots
            .values()
            .filter(|b| b.task.armed.is_some_and(|a| a.duty == d))
            .count()
    };
    assert_eq!(duty(Duty::Converge), p.defender_cap);
    assert_eq!(duty(Duty::Guard), p.attacker_cap);
    let policy = crate::bot_objectives::ObjectivePolicy::bundled();
    for slice in &records(&e, "bot_planning_slice")[before..] {
        let wave = slice["wave"] == true;
        let (checks, queries) = if wave {
            (
                policy.wave_candidate_checks_per_tick,
                policy.wave_route_queries_per_tick,
            )
        } else {
            (
                policy.candidate_checks_per_tick,
                policy.route_queries_per_tick,
            )
        };
        assert!(
            slice["candidate_checks"].as_u64().unwrap() as usize <= checks,
            "{slice}"
        );
        assert!(
            slice["route_queries"].as_u64().unwrap() as usize <= queries,
            "{slice}"
        );
    }
    // Every re-tasked bot got a duty goal through the shared planner, inside
    // its duty's outer radius (a held spot or a ring place).
    let center = PLANTED_AT.map(f64::from);
    for b in e.bot_registry.bots.values() {
        let Some(duty) = b.task.armed.filter(|_| !b.task.operator) else {
            continue;
        };
        let g = b.task.role_goal.expect("planned duty goal");
        let r = (f64::from(g.x) - center[0]).hypot(f64::from(g.z) - center[2]);
        assert!(r <= duty.duty.ring(&p)[1] + 0.5, "{duty:?} {r}");
    }
}
