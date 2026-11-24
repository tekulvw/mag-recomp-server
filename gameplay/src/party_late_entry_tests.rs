//! Synthetic late group entry into a running, bot-filled retail match.
use crate::*;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

const MAP: &str = "synthetic_late_arena";

struct Files(PathBuf);
impl Files {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "mag-synthetic-late-party-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn write(&self, name: &str, data: Value) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, serde_json::to_vec(&data).unwrap()).unwrap();
        path
    }
    /// Sabotage 0 vs 1: two squads of four per side, eight per side.
    fn spawns(&self) -> PathBuf {
        let squad = |index: u16, team: u8, faction: Value| {
            json!({"index":index,"team":team,"platoon":0,"squad":index % 16,
                "faction":faction,"available":[0],"default_spawn":0})
        };
        self.write("spawns.json", json!({
            "format":"mag_spawn_catalog","schema_version":1,"source":"SYNTHETIC late party arena",
            "active":{"map_name":MAP,"squad_index":0,"default_spawn":0,"available":[0]},
            "match_profile":{"game_type":0,"auto_assign_teams":true,"team_capacity":8,
                "squad_capacity":4,"attacking_faction":0,"has_bunkers":false,"has_objectives":false},
            "maps":[{"name":MAP,"owner_faction":1,"spawn_count":1,
                "locations":[{"index":0,"token":7,"kind":"on_foot","position":{"x":2.,"y":0.,"z":2.}}],
                "squads":[squad(0, 0, Value::Null), squad(1, 0, Value::Null),
                    squad(16, 1, json!(1)), squad(17, 1, json!(1))]}]
        }))
    }
    fn manifest(&self) -> PathBuf {
        let floor = self.write("floor.json", json!({
            "format":"mag_collision_world","schema_version":1,"coordinate_space":"world",
            "metadata":{"map_name":MAP,"build":"synthetic","coverage":"provisional",
                "provenance":["Synthetic late party floor"],"unknowns":["Not a game map"],
                "physics_regions":[{"min":[-20.,-5.,-20.],"max":[20.,8.,20.],"ground_verified":true,
                    "obstacles_verified":true,"provenance":["Synthetic floor"]}]},
            "instances":[{"id":"floor","vertices_xyz":[[-20.,0.,-20.],[20.,0.,-20.],[20.,0.,20.],[-20.,0.,20.]],
                "triangles":[[0,2,1],[0,3,2]]}]
        }));
        let slots: Vec<_> = (0..16)
            .map(|i| json!({"team": i % 2, "position": {"x": f64::from(i) + 1., "y": 0., "z": 2.}}))
            .collect();
        let population = self.write("population.json", json!({
            "format":"mag_bot_population_v1","map_name":MAP,"firearms":{"enabled":false},"slots":slots
        }));
        let names: Vec<_> = (0..16)
            .map(|i| json!({"name": format!("Synthetic Bot {i}")}))
            .collect();
        let names = self.write("names.json", json!({"entries": names}));
        self.write(
            "manifest.json",
            json!({"format":"mag_match_profiles_v1","profiles":[{
                "profile_id":"late","spawn_catalog":self.spawns(),"collision_world":floor,
                "bot_population":population,"bot_fill_total":16,"bot_names":names
            }]}),
        )
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

fn worker(files: &Files) -> Engine<Vec<u8>> {
    let resources = EngineResources::for_application(
        Arc::new(crate::bot_registry::identity_tests::bootstrap_items()),
        spawn_catalog::SpawnCatalog::load(files.spawns()).unwrap(),
        None,
        crate::resources::tests::wire_profiles(),
        20231,
    )
    .unwrap();
    let mut engine = Engine::from_resources(Vec::new(), 1_000_000, resources)
        .with_match_profiles(match_profiles::Registry::load(files.manifest()).unwrap())
        .unwrap();
    engine.configure_party_reservations().unwrap();
    let selected = engine.handle(
        Request::SelectMatchProfile {
            v: BRIDGE_VERSION,
            request_id: "select".into(),
            formation_id: "late-formation".into(),
            profile_id: "late".into(),
        },
        Duration::ZERO,
        1,
    );
    assert!(selected.ok, "{:?}", selected.error);
    assert_eq!(selected.selected_match.unwrap().admission_epoch, Some(1));
    engine
}

fn character(faction: u8, name: &str) -> Value {
    json!({"name": name, "faction": faction, "head": 0, "voice": 0, "universe_id": 26})
}

/// One solo human that has claimed its transport: the match is running.
fn claimed_solo(engine: &mut Engine<Vec<u8>>, player: u16, faction: u8) {
    let request = parse_request(
        json!({"v":BRIDGE_VERSION,"op":"reserve","request_id":"solo",
            "reservation_id":format!("synthetic-solo-{player}"),"world_id":7,"player_id":player,
            "account_id":100 + u32::from(player),"session_key":"aaaaaaaaaaaaaaaa",
            "access_key":"bbbbbbbbbbbbbbbb","character":character(faction, "Synthetic Solo")})
        .to_string()
        .as_bytes(),
    )
    .unwrap();
    let reply = engine.handle(request, Duration::ZERO, 1);
    assert!(reply.ok, "{:?}", reply.error);
    let id = format!("synthetic-solo-{player}");
    let actor = engine
        .active
        .iter_mut()
        .chain(engine.peers.values_mut())
        .find(|actor| actor.reservation.id == id)
        .unwrap();
    actor.claimed = Some(Claimed {
        wire: vec![],
        np_identity: [0; 36],
        game_info: protocol::GameInfo {
            client_index: player,
            level_name: MAP.into(),
            start_time_ms: 0,
            attacking_faction: 0,
            defending_faction: 1,
            maximum_points: [1000, 1000],
            current_points: [1000, 1000],
            game_type: 0,
        },
    });
}

fn party(cohort: &str, faction: u8, players: &[u16]) -> Value {
    let members: Vec<_> = players
        .iter()
        .map(|&player| {
            json!({"reservation_id": format!("synthetic-{cohort}-{player}"), "player_id": player,
                "account_id": 1000 + u32::from(player), "session_key": "aaaaaaaaaaaaaaaa",
                "access_key": "bbbbbbbbbbbbbbbb", "character": character(faction, "Synthetic Member")})
        })
        .collect();
    json!({"v": BRIDGE_VERSION, "op": "reserve_party", "request_id": "late-party",
        "formation_id": "late-formation", "profile_id": "late", "cohort_id": cohort,
        "world_id": 7, "admission_epoch": 1, "members": members})
}

fn order(engine: &mut Engine<Vec<u8>>, value: &Value) -> Response {
    let request = parse_request(&serde_json::to_vec(value).unwrap()).unwrap();
    engine.handle(request, Duration::from_millis(10), 1)
}

fn humans(engine: &Engine<Vec<u8>>) -> usize {
    engine.active.iter().count() + engine.peers.len()
}

fn occupants(engine: &Engine<Vec<u8>>, squad: u16) -> (usize, usize) {
    (
        engine.squad_reservation_count(squad),
        engine.bot_registry.members(squad).count(),
    )
}

fn team_total(engine: &Engine<Vec<u8>>, team: u8) -> usize {
    engine
        .active
        .iter()
        .chain(engine.peers.values())
        .filter(|actor| actor.team() == team)
        .count()
        + engine
            .bot_registry
            .bots
            .values()
            .filter(|bot| bot.target.squad.team() == team)
            .count()
}

fn reconcile(engine: &mut Engine<Vec<u8>>) {
    engine
        .reconcile_bot_population(Duration::from_millis(20), 1, &mut Default::default())
        .unwrap();
}

/// A running match with one claimed human on side 0 and fifteen bots.
fn running(files: &Files) -> Engine<Vec<u8>> {
    let mut engine = worker(files);
    claimed_solo(&mut engine, 0, 0);
    reconcile(&mut engine);
    assert_eq!(engine.bot_registry.bots.len(), 15);
    assert_eq!(humans(&engine) + engine.bot_registry.bots.len(), 16);
    engine
}

#[test]
fn late_party_joins_one_squad_of_a_running_bot_filled_match() {
    let files = Files::new();
    let mut engine = running(&files);
    assert_eq!(occupants(&engine, 1), (0, 4), "an all-bot squad");
    let reply = order(&mut engine, &party("cohort-a", 0, &[1, 2, 3]));
    assert!(reply.ok, "{:?}", reply.error);
    let reply = reply.party_reservation.unwrap();
    assert_eq!((reply.team, reply.squad), (Some(0), Some(1)));
    for id in &reply.reservation_ids {
        assert_eq!(engine.peers[id].squad_catalog.squad_index(), 1);
        assert!(engine.peers[id].claimed.is_none());
    }
    // Bots gave up three seats at once: the squad, side and world fit.
    assert_eq!(occupants(&engine, 1), (3, 1));
    assert_eq!(humans(&engine), 4);
    assert!(team_total(&engine, 0) <= 8 && team_total(&engine, 1) <= 8);
    assert!(humans(&engine) + engine.bot_registry.bots.len() <= 16);
    for player in 1..=3 {
        assert!(!engine.bot_registry.bots.contains_key(&player));
    }
    // Ordinary reconciliation keeps the humans' seats and the capacity.
    reconcile(&mut engine);
    assert_eq!(occupants(&engine, 1), (3, 1));
    assert_eq!(humans(&engine) + engine.bot_registry.bots.len(), 16);
    let log = String::from_utf8_lossy(&engine.writer);
    assert!(log.contains("\"late_entry\":true"));
    assert!(!log.contains("aaaaaaaaaaaaaaaa"));
}

#[test]
fn late_party_refusal_is_atomic_and_names_its_cause() {
    let files = Files::new();
    let mut engine = running(&files);
    let bots: Vec<u16> = engine.bot_registry.bots.keys().copied().collect();
    let snapshot = engine.writer.clone();
    let sequence = engine.sequence;
    let unchanged = |engine: &Engine<Vec<u8>>| {
        assert_eq!(engine.writer, snapshot);
        assert_eq!(engine.sequence, sequence);
        assert_eq!(humans(engine), 1);
        assert_eq!(
            engine.bot_registry.bots.keys().copied().collect::<Vec<_>>(),
            bots
        );
    };
    // A later member conflicts with the running human: nobody is admitted.
    let mut conflict = party("cohort-a", 0, &[1, 2, 3]);
    conflict["members"][2]["account_id"] = json!(100);
    assert_eq!(
        order(&mut engine, &conflict).error.as_deref(),
        Some("party_member_conflict")
    );
    unchanged(&engine);
    // More members than the side's free and bot-held seats together.
    assert_eq!(
        order(
            &mut engine,
            &party("cohort-b", 0, &[1, 2, 3, 4, 5, 6, 7, 8])
        )
        .error
        .as_deref(),
        Some("party_no_room")
    );
    unchanged(&engine);
    assert_eq!(
        order(&mut engine, &party("cohort-c", 2, &[1, 2]))
            .error
            .as_deref(),
        Some("party_faction_mismatch")
    );
    let mut mixed = party("cohort-d", 0, &[1, 2]);
    mixed["members"][1]["character"]["faction"] = json!(1);
    assert_eq!(
        order(&mut engine, &mixed).error.as_deref(),
        Some("party_faction_mismatch")
    );
    unchanged(&engine);
    engine.match_terminal = Some(match_flow::MatchTerminal {
        world_id: 7,
        match_id: "synthetic-ending".into(),
        reason: "synthetic".into(),
        reservation_ids: vec!["synthetic-solo-0".into()],
    });
    assert_eq!(
        order(&mut engine, &party("cohort-e", 0, &[1, 2]))
            .error
            .as_deref(),
        Some("party_match_ending")
    );
    unchanged(&engine);
}

#[test]
fn late_party_without_one_free_squad_stays_together_on_its_side() {
    let files = Files::new();
    let mut engine = worker(&files);
    // Two humans in each side-0 squad leave no squad with three free seats.
    for player in 0..4 {
        claimed_solo(&mut engine, player, 0);
    }
    assert_eq!(occupants(&engine, 0).0, 2);
    assert_eq!(occupants(&engine, 1).0, 2);
    reconcile(&mut engine);
    let reply = order(&mut engine, &party("cohort-a", 0, &[4, 5, 6]));
    assert!(reply.ok, "{:?}", reply.error);
    let reply = reply.party_reservation.unwrap();
    assert_eq!(reply.team, Some(0));
    let squads: Vec<u16> = reply
        .reservation_ids
        .iter()
        .map(|id| engine.peers[id].squad_catalog.squad_index())
        .collect();
    assert_eq!(
        squads[0],
        reply.squad.unwrap(),
        "the reply names the leader's squad"
    );
    assert!(squads.iter().all(|squad| *squad < 16));
    assert_eq!(
        squads.iter().filter(|squad| **squad == squads[0]).count(),
        2
    );
    assert_eq!((occupants(&engine, 0).0, occupants(&engine, 1).0), (4, 3));
    assert!(team_total(&engine, 0) <= 8);
    assert!(occupants(&engine, 0).0 + occupants(&engine, 0).1 <= 4);
    assert!(occupants(&engine, 1).0 + occupants(&engine, 1).1 <= 4);
    // A forming match keeps the one-squad rule: it never splits a group.
    let mut forming = worker(&files);
    for player in 0..4_u16 {
        let request = parse_request(
            json!({"v":BRIDGE_VERSION,"op":"reserve","request_id":"solo",
                "reservation_id":format!("synthetic-forming-{player}"),"world_id":7,"player_id":player,
                "account_id":100 + u32::from(player),"session_key":"aaaaaaaaaaaaaaaa",
                "access_key":"bbbbbbbbbbbbbbbb","character":character(0, "Synthetic Solo")})
            .to_string()
            .as_bytes(),
        )
        .unwrap();
        assert!(forming.handle(request, Duration::ZERO, 1).ok);
    }
    assert_eq!(
        order(&mut forming, &party("cohort-a", 0, &[4, 5, 6]))
            .error
            .as_deref(),
        Some("party_no_room")
    );
}
