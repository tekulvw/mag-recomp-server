//! Synthetic bot fill: bots pack squads in order instead of spreading over them.
use crate::*;
use std::collections::BTreeMap;
use std::sync::Arc;

const MAP: &str = "synthetic_packing_arena";

/// One mode's admission shape: game type, squads per side and squad capacity.
#[derive(Clone, Copy, Debug)]
struct Mode {
    game_type: u8,
    squads_per_team: u16,
    squad_capacity: usize,
}
const SABOTAGE: Mode = Mode {
    game_type: 0,
    squads_per_team: 4,
    squad_capacity: 8,
};
const ACQUISITION: Mode = Mode {
    game_type: 1,
    squads_per_team: 8,
    squad_capacity: 4,
};
const DOMINATION: Mode = Mode {
    game_type: 2,
    squads_per_team: 16,
    squad_capacity: 4,
};

fn squad_indices(mode: Mode, team: u16) -> impl Iterator<Item = u16> {
    (0..mode.squads_per_team).map(move |i| team * 16 + i)
}

/// A team32 match whose bot population binds `squad_capacity` slots to every squad.
fn engine(mode: Mode, total: usize) -> Engine<Vec<u8>> {
    build(mode, total, &[])
}

/// Height of the walled-off spawn platform in `build`.
const ISLAND_Y: f64 = 3.;

/// As `engine`, optionally with the spawn of squad list entry `island`
/// (and its slots) on a 3 m high platform nothing walks or drops off, a
/// navmesh baked from that ground, a capture objective on the open floor
/// and fall rules, so squads can be checked for a route to an objective.
fn build(mode: Mode, total: usize, islands: &[usize]) -> Engine<Vec<u8>> {
    // Squad k (in list order) owns spawn location k; its slots sit beside it.
    let all: Vec<u16> = (0..2).flat_map(|team| squad_indices(mode, team)).collect();
    let anchor = |k: usize| (10. + (k % 8) as f64 * 12., 10. + (k / 8) as f64 * 12.);
    let squads: Vec<_> = all
        .iter()
        .enumerate()
        .map(|(k, &index)| {
            json!({"index":index,"team":index / 16,"platoon":(index % 16) / 4,"squad":index % 4,
                "faction":if index < 16 { Value::Null } else { json!(1) },
                "available":[k],"default_spawn":k})
        })
        .collect();
    let locations: Vec<_> = (0..all.len())
        .map(|k| {
            let (x, z) = anchor(k);
            let y = if islands.contains(&k) { ISLAND_Y } else { 0. };
            json!({"index":k,"token":100 + k,"kind":"on_foot","position":{"x":x,"y":y,"z":z}})
        })
        .collect();
    let spawns: spawn_catalog::SpawnCatalog = serde_json::from_value(json!({
        "format":"mag_spawn_catalog","schema_version":1,"source":"SYNTHETIC squad packing arena",
        "active":{"map_name":MAP,"squad_index":0,"default_spawn":0,"available":[0]},
        "match_profile":{"game_type":mode.game_type,"auto_assign_teams":true,"team_capacity":32,
            "squad_capacity":mode.squad_capacity,"attacking_faction":0,"has_bunkers":false,
            "has_objectives":false},
        "maps":[{"name":MAP,"owner_faction":1,"spawn_count":all.len(),
            "locations":locations,"squads":squads}]
    }))
    .unwrap();
    let names = bot_names::Catalog::from_json(
        &serde_json::to_vec(&json!({"entries":(0..64)
            .map(|i| json!({"name": format!("Synthetic Packer {i}")}))
            .collect::<Vec<_>>()}))
        .unwrap(),
    )
    .unwrap();
    let resources = EngineResources::for_application(
        Arc::new(crate::bot_registry::identity_tests::bootstrap_items()),
        spawns,
        None,
        crate::resources::tests::wire_profiles(),
        20231,
    )
    .unwrap()
    .with_bot_names(names);
    let mut engine = Engine::from_resources(Vec::new(), 100_000_000, resources);
    engine.maximum_players = 64;
    let mut vertices = vec![
        [0., 0., 0.],
        [120., 0., 0.],
        [120., 0., 120.],
        [0., 0., 120.],
    ];
    let mut triangles = vec![[0u32, 2, 1], [0, 3, 2]];
    for &k in islands {
        let (x, z) = anchor(k);
        let (x0, x1, z0, z1) = (x - 3., x + 8., z - 2., z + 6.);
        let n = vertices.len() as u32;
        vertices.extend([
            [x0, ISLAND_Y, z0],
            [x1, ISLAND_Y, z0],
            [x1, ISLAND_Y, z1],
            [x0, ISLAND_Y, z1],
        ]);
        triangles.extend([[n, n + 2, n + 1], [n, n + 3, n + 2]]);
    }
    let floor = json!({"format":"mag_collision_world","schema_version":1,"coordinate_space":"world",
        "metadata":{"map_name":MAP,"build":"synthetic","coverage":"provisional",
            "provenance":["Synthetic packing floor"],"unknowns":["Not a game map"],
            "physics_regions":[{"min":[0.,-5.,0.],"max":[120.,8.,120.],"ground_verified":true,
                "obstacles_verified":true,"provenance":["Synthetic floor"]}]},
        "instances":[{"id":"floor","vertices_xyz":vertices,"triangles":triangles}]});
    let world = collision::CollisionWorld::from_bytes(
        &serde_json::to_vec(&floor).unwrap(),
        collision::CollisionLimits::default(),
    )
    .unwrap();
    if !islands.is_empty() {
        engine = routed(engine, &world, &vertices, &triangles);
    }
    engine = engine.with_collision_world(world);
    // Bound slots, listed squad by squad, each at its own spot beside the
    // squad's spawn location.
    let slots: Vec<_> = all
        .iter()
        .enumerate()
        .flat_map(|(k, &squad)| (0..mode.squad_capacity).map(move |seat| (k, squad, seat)))
        .map(|(k, squad, seat)| {
            let (x, z) = anchor(k);
            let y = if islands.contains(&k) { ISLAND_Y } else { 0. };
            json!({"team":squad / 16,"squad":squad,"source_spawn":k,"position":{
                "x":x + (seat % 4) as f64 * 1.5,"y":y,"z":z + 1.5 + (seat / 4) as f64 * 1.5}})
        })
        .collect();
    let population: bot_registry::BotPopulation = serde_json::from_value(json!({
        "format":"mag_bot_population_v1","map_name":MAP,"firearms":{"enabled":false},"slots":slots
    }))
    .unwrap();
    engine.with_bot_population(population, total).unwrap()
}

/// Navmesh, fall rules and one capture objective on the open floor.
fn routed(
    engine: Engine<Vec<u8>>,
    world: &collision::CollisionWorld,
    vertices: &[[f64; 3]],
    triangles: &[[u32; 3]],
) -> Engine<Vec<u8>> {
    use sha2::{Digest, Sha256};
    let vertices: Vec<[f32; 3]> = vertices.iter().map(|p| p.map(|v| v as f32)).collect();
    let mut hash = Sha256::new();
    hash.update(b"MAG-NAV-GEOMETRY-v1\0");
    hash.update((vertices.len() as u32).to_le_bytes());
    hash.update((triangles.len() as u32).to_le_bytes());
    for v in vertices.iter().flatten() {
        hash.update(v.to_le_bytes());
    }
    for i in triangles.iter().flatten() {
        hash.update(i.to_le_bytes());
    }
    let artifact = navmesh::bake(navmesh::BakeInput {
        format: "mag_navmesh_bake_input_v1".into(),
        map_name: MAP.into(),
        world_sha256: world.source_sha256().into(),
        geometry_sha256: format!("{:x}", hash.finalize()),
        bounds: navmesh::Bounds {
            min: [0., -1., 0.],
            max: [120., 6., 120.],
        },
        physics_regions: world.metadata().physics_regions.clone(),
        vertices,
        triangles: triangles.to_vec(),
    })
    .unwrap();
    let mesh = navmesh::NavMesh::load(artifact, world.source_sha256(), MAP).unwrap();
    let rules = fall_damage::Rules {
        schema_version: 1,
        source_build: "synthetic".into(),
        application_id: 20231,
        min_height: 10.,
        damage_factor: 1.,
        minimum_damage_exclusive: 0.,
        jump_fall_state: 0,
        damage_type: 5,
        damage_source_type: 1,
    };
    let mut engine = engine
        .with_navmesh_world(mesh)
        .with_fall_damage_rules(rules);
    let objectives = objective_catalog::ObjectiveCatalog::from_json(
        &json!({"schema_version":1,"format":"mag_retail_objective_catalog","application_id":20231,
        "protocol":protocol::ProtocolInfo::RETAIL_PROTOCOL,"game_type":0,"map_name":MAP,
        "entity_family":12,"network_type":12,
        "source":{"authored_catalog_sha256":"0".repeat(64),"binding_capture_sha256":"1".repeat(64),
            "startup_proof_sha256":"2".repeat(64),"source_image_sha256":"3".repeat(64),
            "capture_qualification":"bounded_concurrent_token_slot_capture"},
        "objects":([5u16, 6].map(|index| json!({"token":format!("0x{index:08x}"),"object_index":index,
            "initial_state":1,"interaction":{"owner_team":1,"activate_ms":100,"complete_ms":20_000,
                "deactivate_ms":100,"can_deactivate":true,"position":[60.,0.,f64::from(index) * 10.],
                "trigger_count":1,"maximum_distance_m":3.,
                "distance_policy":"development_point_distance"}}))),
        // Each side's first two squads are authored to objective 5, the last
        // two to objective 6, as Acquisition pairs platoons with prototypes.
        "squads":(0..2u16).flat_map(|team| (0..4u16).map(move |slot| {
            let objective = if slot < 2 { 5 } else { 6 };
            json!({"team":team,"squad_index":team*16+slot,"initial_objective_index":objective,
                "authored_objectives":[{"object_index":objective,"order":1}]})
        })).collect::<Vec<_>>()})
        .to_string(),
    )
    .unwrap();
    engine.objective_catalog = Some(objectives);
    engine
}

/// A human reservation on side 0 placed in `squad`.
fn human(engine: &mut Engine<Vec<u8>>, player: u16, squad: u16) {
    let response = engine.handle(
        Request::Reserve {
            v: BRIDGE_VERSION,
            request_id: format!("synthetic-reserve-{player}"),
            reservation_id: format!("synthetic-packing-{player}"),
            world_id: 1,
            player_id: player,
            account_id: u32::from(player) + 1,
            session_key: "aaaaaaaaaaaaaaaa".into(),
            access_key: "bbbbbbbbbbbbbbbb".into(),
            character: serde_json::from_value(json!({
                "name":format!("Synthetic Human {player}"),"faction":0,"head":0,"voice":0,
                "universe_id":26,"character_id":u32::from(player) + 1
            }))
            .unwrap(),
        },
        Duration::ZERO,
        1,
    );
    assert!(response.ok, "{:?}", response.error);
    let catalog = engine
        .spawn_catalog
        .as_ref()
        .unwrap()
        .for_squad(squad)
        .unwrap();
    let actor = engine
        .active
        .iter_mut()
        .chain(engine.peers.values_mut())
        .find(|actor| actor.reservation.player == player)
        .unwrap();
    actor.squad_catalog = Arc::new(catalog);
}

fn reconcile(engine: &mut Engine<Vec<u8>>, ms: u64) {
    engine
        .reconcile_bot_population(Duration::from_millis(ms), 1, &mut Default::default())
        .unwrap();
}

/// (humans, bots) seated in each squad of one side, in squad index order.
fn seats(engine: &Engine<Vec<u8>>, mode: Mode, team: u16) -> Vec<(usize, usize)> {
    squad_indices(mode, team)
        .map(|squad| {
            (
                engine.squad_reservation_count(squad),
                engine.bot_registry.members(squad).count(),
            )
        })
        .collect()
}

fn team_total(engine: &Engine<Vec<u8>>, mode: Mode, team: u16) -> usize {
    seats(engine, mode, team).iter().map(|(h, b)| h + b).sum()
}

/// Squads with a human first, then index order: every squad is full until
/// one partial squad, and every squad after it is empty.
fn assert_packed(engine: &Engine<Vec<u8>>, mode: Mode, team: u16) {
    let seats = seats(engine, mode, team);
    let mut order: Vec<_> = (0..seats.len()).collect();
    order.sort_by_key(|&i| (seats[i].0 == 0, i));
    let filled: Vec<_> = order.iter().map(|&i| seats[i].0 + seats[i].1).collect();
    let full = filled
        .iter()
        .take_while(|&&n| n == mode.squad_capacity)
        .count();
    assert!(
        filled[full..].iter().skip(1).all(|&n| n == 0),
        "{mode:?} side {team} spreads its bots: {seats:?}"
    );
}

fn placements(engine: &Engine<Vec<u8>>) -> BTreeMap<u16, (u16, usize)> {
    engine
        .bot_registry
        .bots
        .iter()
        .map(|(&id, bot)| (id, (bot.target.squad.squad_index(), bot.slot)))
        .collect()
}

#[test]
fn bots_fill_squads_in_order_for_every_mode() {
    for mode in [SABOTAGE, ACQUISITION, DOMINATION] {
        for total in [2, 9, 20, 64] {
            let mut engine = engine(mode, total);
            reconcile(&mut engine, 0);
            assert_eq!(engine.bot_registry.bots.len(), total, "{mode:?} {total}");
            // The fill total and the side balance are unchanged by packing.
            assert_eq!(team_total(&engine, mode, 0), total.div_ceil(2));
            assert_eq!(team_total(&engine, mode, 1), total / 2);
            for team in 0..2 {
                assert_packed(&engine, mode, team);
            }
            // Deterministic: the same inputs seat the same bots identically.
            let mut again = self::engine(mode, total);
            reconcile(&mut again, 0);
            assert_eq!(placements(&engine), placements(&again), "{mode:?} {total}");
        }
    }
}

#[test]
fn a_full_domination_fill_packs_whole_squads_around_the_human() {
    // Live shape: 63 bots and one human used to sit two to a squad across
    // all 32 squads.
    let mut engine = engine(DOMINATION, 64);
    human(&mut engine, 0, 2);
    reconcile(&mut engine, 0);
    assert_eq!(engine.bot_registry.bots.len(), 63);
    let full = (0, 4);
    let mut side0 = vec![full; 8];
    side0[2] = (1, 3);
    side0.extend([(0, 0); 8]);
    assert_eq!(
        seats(&engine, DOMINATION, 0),
        side0,
        "human squad, then 0.."
    );
    let mut side1 = vec![full; 8];
    side1.extend([(0, 0); 8]);
    assert_eq!(seats(&engine, DOMINATION, 1), side1);
    for team in 0..2 {
        assert_packed(&engine, DOMINATION, team);
    }
}

#[test]
fn the_humans_squad_fills_first_in_every_mode() {
    for mode in [SABOTAGE, ACQUISITION, DOMINATION] {
        let mut engine = engine(mode, 20);
        // The highest squad on side 0, not the lowest.
        let squad = mode.squads_per_team - 1;
        human(&mut engine, 0, squad);
        reconcile(&mut engine, 0);
        assert_eq!(engine.bot_registry.bots.len(), 19);
        assert_eq!(team_total(&engine, mode, 0), 10);
        assert_eq!(team_total(&engine, mode, 1), 10);
        let side0 = seats(&engine, mode, 0);
        assert_eq!(
            side0[usize::from(squad)],
            (1, mode.squad_capacity.min(10) - 1),
            "{mode:?}"
        );
        assert_packed(&engine, mode, 0);
        assert_packed(&engine, mode, 1);
    }
}

#[test]
fn humans_joining_keep_the_total_displace_bots_and_stay_packed() {
    let mut engine = engine(DOMINATION, 64);
    human(&mut engine, 0, 2);
    reconcile(&mut engine, 0);
    // A human taking a seat in a full all-bot squad: one of its bots yields
    // the seat, and the side's last-filled squad gives up the surplus bot,
    // so the refill tops that squad up again.
    human(&mut engine, 1, 1);
    reconcile(&mut engine, 100);
    assert_eq!(engine.bot_registry.bots.len(), 62);
    let mut side0 = vec![(0, 4); 8];
    side0[1] = (1, 3);
    side0[2] = (1, 3);
    side0.extend([(0, 0); 8]);
    assert_eq!(seats(&engine, DOMINATION, 0), side0);
    assert_eq!(team_total(&engine, DOMINATION, 1), 32);
    // A human in an empty squad: the bot that leaves comes from the
    // last-filled squad, so the squads in front stay full.
    human(&mut engine, 2, 12);
    reconcile(&mut engine, 200);
    assert_eq!(engine.bot_registry.bots.len(), 61);
    assert_eq!(team_total(&engine, DOMINATION, 0), 32);
    assert_eq!(team_total(&engine, DOMINATION, 1), 32);
    let side0 = seats(&engine, DOMINATION, 0);
    assert_eq!(side0[12], (1, 0));
    assert_eq!(side0[7], (0, 3), "{side0:?}");
    assert!(side0[..7].iter().all(|(h, b)| h + b == 4), "{side0:?}");
    assert!(
        side0[8..]
            .iter()
            .enumerate()
            .all(|(i, s)| i + 8 == 12 || *s == (0, 0)),
        "{side0:?}"
    );
}

#[test]
fn bots_fill_squads_that_can_reach_an_objective_first() {
    // Sabotage shape: four squads of eight a side. Side 1's first squad (16,
    // list entry 4) spawns on a platform with no way down.
    let total = 16;
    let mut engine = build(SABOTAGE, total, &[4]);
    reconcile(&mut engine, 0);
    assert_eq!(engine.bot_registry.bots.len(), total);
    assert_eq!(
        seats(&engine, SABOTAGE, 0),
        [(0, 8), (0, 0), (0, 0), (0, 0)]
    );
    assert_eq!(
        seats(&engine, SABOTAGE, 1),
        [(0, 0), (0, 8), (0, 0), (0, 0)],
        "side 1 skips the stranded squad 16 for squad 17"
    );
    let log = String::from_utf8_lossy(&engine.writer);
    assert!(
        log.lines()
            .any(|line| line.contains("bot_population_unroutable_squads")
                && line.contains("\"squads\":[16]")),
        "{log}"
    );
    // With every routable squad full, the stranded squad still fills last.
    let mut engine = build(SABOTAGE, 64, &[4]);
    reconcile(&mut engine, 0);
    assert_eq!(seats(&engine, SABOTAGE, 1), [(0, 8); 4]);
    // Without navigation data no squad is demoted: plain index order.
    let mut engine = self::engine(SABOTAGE, 16);
    reconcile(&mut engine, 0);
    assert_eq!(
        seats(&engine, SABOTAGE, 1),
        [(0, 8), (0, 0), (0, 0), (0, 0)]
    );
}

#[test]
fn bots_fill_squads_that_meet_the_other_side_at_an_objective() {
    // Side 1's squads 16 and 17 (authored to objective 5, like side 0's
    // squads 0 and 1) are stranded, so side 1 fills squad 18 (objective 6).
    // Side 0 then fills squad 2, also authored to objective 6, instead of
    // squad 1, so both sides go to the same objective.
    let total = 24;
    let mut engine = build(SABOTAGE, total, &[4, 5]);
    reconcile(&mut engine, 0);
    assert_eq!(engine.bot_registry.bots.len(), total);
    assert_eq!(
        seats(&engine, SABOTAGE, 1),
        [(0, 0), (0, 0), (0, 8), (0, 4)]
    );
    assert_eq!(
        seats(&engine, SABOTAGE, 0),
        [(0, 8), (0, 0), (0, 4), (0, 0)]
    );
}
