//! Synthetic worlds above the legacy size: the human cap is separate from the
//! world's character capacity, bots leave the human index range for last, and
//! a full Character pool still admits a human.
use crate::*;
use std::collections::BTreeSet;
use std::sync::Arc;

const MAP: &str = "synthetic_capacity_arena";

/// A world shape: game type, squads per side, eight-member squads.
#[derive(Clone, Copy, Debug)]
struct Shape {
    game_type: u8,
    squads_per_team: u16,
    team_capacity: usize,
}
/// Acquisition 128: two platoons of four eight-member squads per side.
const ACQUISITION_128: Shape = Shape {
    game_type: 1,
    squads_per_team: 8,
    team_capacity: 64,
};
/// Domination 256: every squad address, eight members each.
const DOMINATION_256: Shape = Shape {
    game_type: 2,
    squads_per_team: 16,
    team_capacity: 128,
};
/// A legacy-size Domination world with retail eight-member squads.
const DOMINATION_64: Shape = Shape {
    game_type: 2,
    squads_per_team: 16,
    team_capacity: 32,
};
const SQUAD: usize = 8;

fn squad_indices(shape: Shape, team: u16) -> impl Iterator<Item = u16> {
    (0..shape.squads_per_team).map(move |i| team * 16 + i)
}

/// An invented flat arena whose bot population binds eight slots to every
/// squad, filled to `total` characters.
fn engine(shape: Shape, total: usize) -> Engine<Vec<u8>> {
    let all: Vec<u16> = (0..2).flat_map(|team| squad_indices(shape, team)).collect();
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
            json!({"index":k,"token":100 + k,"kind":"on_foot","position":{"x":x,"y":0.,"z":z}})
        })
        .collect();
    let spawns: spawn_catalog::SpawnCatalog = serde_json::from_value(json!({
        "format":"mag_spawn_catalog","schema_version":1,"source":"SYNTHETIC capacity arena",
        "active":{"map_name":MAP,"squad_index":0,"default_spawn":0,"available":[0]},
        "match_profile":{"game_type":shape.game_type,"auto_assign_teams":true,
            "team_capacity":shape.team_capacity,"squad_capacity":SQUAD,"attacking_faction":0,
            "has_bunkers":false,"has_objectives":false},
        "maps":[{"name":MAP,"owner_faction":1,"spawn_count":all.len(),
            "locations":locations,"squads":squads}]
    }))
    .unwrap();
    let names = bot_names::Catalog::from_json(
        &serde_json::to_vec(&json!({"entries":(0..world_capacity::MAX_WORLD_CHARACTERS)
            .map(|i| json!({"name": format!("Synthetic Capacity {i}")}))
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
    engine.maximum_players = engine.human_admission_capacity_for(20231);
    let floor = json!({"format":"mag_collision_world","schema_version":1,"coordinate_space":"world",
        "metadata":{"map_name":MAP,"build":"synthetic","coverage":"provisional",
            "provenance":["Synthetic capacity floor"],"unknowns":["Not a game map"],
            "physics_regions":[{"min":[0.,-5.,0.],"max":[120.,8.,120.],"ground_verified":true,
                "obstacles_verified":true,"provenance":["Synthetic floor"]}]},
        "instances":[{"id":"floor","vertices_xyz":[[0.,0.,0.],[120.,0.,0.],[120.,0.,120.],[0.,0.,120.]],
            "triangles":[[0,2,1],[0,3,2]]}]});
    engine = engine.with_collision_world(
        collision::CollisionWorld::from_bytes(
            &serde_json::to_vec(&floor).unwrap(),
            collision::CollisionLimits::default(),
        )
        .unwrap(),
    );
    let slots: Vec<_> = all
        .iter()
        .enumerate()
        .flat_map(|(k, &squad)| (0..SQUAD).map(move |seat| (k, squad, seat)))
        .map(|(k, squad, seat)| {
            let (x, z) = anchor(k);
            json!({"team":squad / 16,"squad":squad,"source_spawn":k,"position":{
                "x":x + (seat % 4) as f64 * 1.5,"y":0.,"z":z + 1.5 + (seat / 4) as f64 * 1.5}})
        })
        .collect();
    let population: bot_registry::BotPopulation = serde_json::from_value(json!({
        "format":"mag_bot_population_v1","map_name":MAP,"firearms":{"enabled":false},"slots":slots
    }))
    .unwrap();
    engine.with_bot_population(population, total).unwrap()
}

fn reserve(engine: &mut Engine<Vec<u8>>, player: u16, faction: u8) -> Response {
    engine.handle(
        Request::Reserve {
            v: BRIDGE_VERSION,
            request_id: format!("synthetic-reserve-{player}"),
            reservation_id: format!("synthetic-capacity-{player:04}"),
            world_id: 1,
            player_id: player,
            account_id: u32::from(player) + 1,
            session_key: "aaaaaaaaaaaaaaaa".into(),
            access_key: "bbbbbbbbbbbbbbbb".into(),
            character: serde_json::from_value(json!({
                "name":format!("Synthetic Human {player}"),"faction":faction,"head":0,"voice":0,
                "universe_id":26,"character_id":u32::from(player) + 1
            }))
            .unwrap(),
        },
        Duration::ZERO,
        1,
    )
}

fn reconcile(engine: &mut Engine<Vec<u8>>, ms: u64) {
    engine
        .reconcile_bot_population(Duration::from_millis(ms), 1, &mut Default::default())
        .unwrap();
}

fn humans(engine: &Engine<Vec<u8>>) -> BTreeSet<u16> {
    engine
        .peers
        .values()
        .chain(engine.active.iter())
        .map(|actor| actor.reservation.player)
        .collect()
}

fn bots(engine: &Engine<Vec<u8>>) -> BTreeSet<u16> {
    engine.bot_registry.bots.keys().copied().collect()
}

fn records(engine: &Engine<Vec<u8>>, op: &str) -> Vec<Value> {
    String::from_utf8_lossy(&engine.writer)
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|record| record["op"] == op)
        .collect()
}

#[test]
fn large_worlds_admit_humans_against_their_own_cap() {
    for shape in [ACQUISITION_128, DOMINATION_256] {
        let world = shape.team_capacity * 2;
        let engine = engine(shape, world);
        assert_eq!(engine.admission_capacity(), world, "{shape:?}");
        assert_eq!(engine.human_admission_capacity_for(20231), 32, "{shape:?}");
        assert_eq!(engine.human_team_capacity(), 16, "{shape:?}");
        // The world's size is not a human limit: Horizon may configure at most
        // the human cap, and per-side minima within the side's human cap.
        for maximum in [33, world] {
            assert_eq!(
                engine_limits(shape, 1, maximum).err(),
                Some("invalid_match_limits"),
                "{shape:?} {maximum}"
            );
        }
        assert!(engine_limits(shape, 1, 32).is_ok());
        let quorum = self::engine(shape, world);
        assert_eq!(
            quorum.validate_team_quorum([17, 1], 32),
            Err("invalid_team_quorum")
        );
        assert_eq!(quorum.validate_team_quorum([16, 16], 32), Ok(()));
    }
}

fn engine_limits(shape: Shape, minimum: usize, maximum: usize) -> Result<(), &'static str> {
    engine(shape, 2)
        .with_match_limits(minimum, maximum)
        .map(|_| ())
}

#[test]
fn the_human_cap_holds_per_side_and_per_world_whatever_the_world_size() {
    let mut engine = engine(ACQUISITION_128, 128);
    // Sixteen humans fill side 0; the seventeenth is refused although the
    // side still has 48 bot seats.
    for player in 0..12 {
        let response = reserve(&mut engine, player, 0);
        assert!(response.ok, "{player}: {:?}", response.error);
    }
    // Groups count against the same side cap: four more fit, five do not,
    // whether the group forms with the match or joins it late.
    assert!(engine.allocate_squad_for(0, 4).is_ok());
    assert_eq!(engine.allocate_squad_for(0, 5).err(), Some("match_full"));
    assert!(engine.allocate_split_party(0, 4).is_ok());
    assert_eq!(
        engine.allocate_split_party(0, 5).err(),
        Some("party_no_room")
    );
    for player in 12..16 {
        let response = reserve(&mut engine, player, 0);
        assert!(response.ok, "{player}: {:?}", response.error);
    }
    assert_eq!(
        reserve(&mut engine, 16, 0).error.as_deref(),
        Some("match_full")
    );
    // The other side still admits its own sixteen.
    for player in 16..32 {
        let response = reserve(&mut engine, player, 1);
        assert!(response.ok, "{player}: {:?}", response.error);
    }
    assert_eq!(humans(&engine).len(), 32);
    // Thirty-two humans fill the world; nothing above the human range is a
    // human slot, even though the world holds 128 characters.
    assert_eq!(
        reserve(&mut engine, 32, 1).error.as_deref(),
        Some("match_full")
    );
    reconcile(&mut engine, 0);
    assert_eq!(engine.bot_registry.bots.len(), 96);
    assert!(bots(&engine).iter().all(|&index| index >= 32));
}

#[test]
fn bots_leave_the_human_index_range_free_while_the_pool_has_room() {
    let mut engine = engine(ACQUISITION_128, 128);
    assert!(reserve(&mut engine, 0, 0).ok);
    reconcile(&mut engine, 0);
    let bots = bots(&engine);
    assert_eq!(bots.len(), 127);
    // Acquisition 128 leaves the whole human range free: every bot sits
    // above it, highest index first.
    assert_eq!(
        bots,
        (129..world_capacity::MAX_WORLD_CHARACTERS as u16).collect()
    );
    // Any human Horizon places on an index below the cap finds it free, so
    // no bot has to yield its index.
    for player in 1..8 {
        assert!(reserve(&mut engine, player, player as u8 % 2).ok);
    }
    reconcile(&mut engine, 100);
    assert!(records(&engine, "bot_population_waiting").is_empty());
    assert_eq!(engine.bot_registry.bots.len(), 120);
    assert!(self::bots(&engine).iter().all(|&index| index >= 32));
}

#[test]
fn a_full_256_pool_admits_a_human_by_retiring_a_bot() {
    let mut engine = engine(DOMINATION_256, 256);
    assert!(reserve(&mut engine, 0, 0).ok);
    reconcile(&mut engine, 0);
    // One human and 255 bots use every Character index. Bots took the
    // indices above the human range first and borrowed the top of it last.
    let all: BTreeSet<u16> = (0..world_capacity::MAX_WORLD_CHARACTERS as u16).collect();
    assert_eq!(engine.bot_registry.bots.len(), 255);
    let full: BTreeSet<u16> = bots(&engine).union(&humans(&engine)).copied().collect();
    assert_eq!(full, all);

    // A second human on a borrowed index: that bot yields the index, so the
    // world stays at 256 characters with no index shared.
    let before = engine.writer.len();
    assert!(reserve(&mut engine, 1, 1).ok);
    reconcile(&mut engine, 100);
    assert!(!bots(&engine).contains(&1));
    assert_eq!(engine.bot_registry.bots.len(), 254);
    let full: BTreeSet<u16> = bots(&engine).union(&humans(&engine)).copied().collect();
    assert_eq!(full, all);
    assert!(bots(&engine).is_disjoint(&humans(&engine)));
    assert!(!String::from_utf8_lossy(&engine.writer[before..]).contains("no_free_character_index"));

    // Humans keep arriving up to the cap; each takes over a borrowed index.
    for player in 2..32 {
        assert!(
            reserve(&mut engine, player, player as u8 % 2).ok,
            "{player}"
        );
    }
    reconcile(&mut engine, 200);
    assert_eq!(engine.bot_registry.bots.len(), 224);
    assert_eq!(
        bots(&engine),
        (32..world_capacity::MAX_WORLD_CHARACTERS as u16).collect()
    );
    assert_eq!(humans(&engine), (0..32).collect());
}

#[test]
fn large_worlds_raise_each_recipients_queue_bound() {
    let mut engine = engine(DOMINATION_256, 256);
    assert!(reserve(&mut engine, 0, 0).ok);
    let actor = engine.active.as_ref().unwrap();
    assert_eq!(actor.shared_queue_limit, 768);
    let mut legacy = self::engine(DOMINATION_64, 64);
    assert_eq!(legacy.human_admission_capacity_for(20231), 64);
    assert_eq!(legacy.human_team_capacity(), 32);
    assert!(reserve(&mut legacy, 0, 0).ok);
    assert_eq!(
        legacy.active.as_ref().unwrap().shared_queue_limit,
        multiplayer::MAX_SHARED_QUEUE
    );
}

/// More queued frames than one bridge reply carries: the reply stops at its
/// budget, every recipient keeps the rest in order, and the deferral is
/// counted and reported without failing the reply or evicting anyone.
#[test]
fn a_reply_over_the_bridge_budget_defers_frames_in_order_and_reports_it() {
    let mut engine = engine(ACQUISITION_128, 2);
    for player in 0..32 {
        assert!(reserve(&mut engine, player, player as u8 % 2).ok);
    }
    let frame = replication::multiplayer::CharacterRemoval {
        sequence: 1,
        timestamp_ms: 0,
        player: 200,
    }
    .encode_with_profile(engine.wire_profile)
    .unwrap();
    for actor in engine.peers.values_mut().chain(engine.active.iter_mut()) {
        crate::spawn_progression::tests::ready(actor);
        for _ in 0..10 {
            actor.queue_shared(frame.clone());
        }
    }
    let profile = engine.wire_profile;
    let mut delivered = std::collections::BTreeMap::<u16, Vec<u32>>::new();
    let mut drain = |engine: &mut Engine<Vec<u8>>, wall: u64| {
        let messages = engine.drain_shared(Vec::new(), wall).unwrap();
        assert!(messages.len() <= 128);
        for message in &messages {
            let wire = protocol::decode_hex(&message.payload_hex);
            let sequence = replication::ObjectUpdate::decode_with_profile(&wire, profile)
                .unwrap()
                .header
                .sequence;
            delivered
                .entry(message.player_id)
                .or_default()
                .push(sequence);
        }
        messages.len()
    };
    // 320 queued frames: the first reply carries its 128-message budget.
    assert_eq!(drain(&mut engine, 10), 128);
    let reports = records(&engine, "shared_replication_deferred");
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0]["queued_frames"], 192);
    assert_eq!(reports[0]["recipients"], 32);
    assert_eq!(reports[0]["message_limit"], 128);
    assert_eq!(reports[0]["deferred_replies_total"], 1);
    // Counted every time, reported at most once per second.
    assert_eq!(drain(&mut engine, 10), 128);
    assert_eq!(engine.shared_deferral.replies, 2);
    assert_eq!(records(&engine, "shared_replication_deferred").len(), 1);
    assert_eq!(drain(&mut engine, 11), 64);
    assert_eq!(engine.shared_deferral.replies, 2);
    assert_eq!(drain(&mut engine, 12), 0);
    // Nobody was evicted, and every recipient got its ten frames once, in
    // order, under consecutive sequence numbers.
    assert!(
        engine
            .peers
            .values()
            .chain(engine.active.iter())
            .all(|actor| !actor.replication_backlogged && actor.pending_shared.is_empty())
    );
    assert_eq!(delivered.len(), 32);
    for (player, sequences) in delivered {
        assert_eq!(sequences, (2..12).collect::<Vec<_>>(), "{player}");
    }
    assert!(records(&engine, "replication_backlog_released").is_empty());
}
