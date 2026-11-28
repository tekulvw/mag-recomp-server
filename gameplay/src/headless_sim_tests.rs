//! Synthetic headless sim: profile selection, draining observers, per-tick
//! timing and per-observer traffic accounting.
use crate::headless_sim::{HeadlessSimOptions, MAX_SIM_EVENT_HZ};
use crate::tick_timing::Phase;
use crate::*;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

const MAP: &str = "synthetic_sim_arena";

struct Files(PathBuf);
impl Files {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "mag-synthetic-headless-sim-{}-{}",
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
            "format":"mag_spawn_catalog","schema_version":1,"source":"SYNTHETIC headless sim arena",
            "active":{"map_name":MAP,"squad_index":0,"default_spawn":0,"available":[0]},
            "match_profile":{"game_type":0,"auto_assign_teams":true,"team_capacity":8,
                "squad_capacity":4,"attacking_faction":0,"has_bunkers":false,"has_objectives":false},
            "maps":[{"name":MAP,"owner_faction":1,"spawn_count":1,
                "locations":[{"index":0,"token":7,"kind":"on_foot","position":{"x":2.,"y":0.,"z":2.}}],
                "squads":[squad(0, 0, Value::Null), squad(1, 0, Value::Null),
                    squad(16, 1, json!(1)), squad(17, 1, json!(1))]}]
        }))
    }
    /// Two profiles on one map that differ only in their fill total.
    fn manifest(&self) -> PathBuf {
        let floor = self.write("floor.json", json!({
            "format":"mag_collision_world","schema_version":1,"coordinate_space":"world",
            "metadata":{"map_name":MAP,"build":"synthetic","coverage":"provisional",
                "provenance":["Synthetic headless sim floor"],"unknowns":["Not a game map"],
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
        let spawns = self.spawns();
        let profile = |id: &str, fill: usize| {
            json!({"profile_id":id,"spawn_catalog":spawns,"collision_world":floor,
                "bot_population":population,"bot_fill_total":fill,"bot_names":names})
        };
        self.write(
            "manifest.json",
            json!({"format":"mag_match_profiles_v1",
                "profiles":[profile("small", 8), profile("large", 16)]}),
        )
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

/// A worker as `main` builds it for `--match-profiles`, with nothing selected.
fn worker(files: &Files) -> Engine<Vec<u8>> {
    let resources = EngineResources::for_application(
        Arc::new(crate::bot_registry::identity_tests::bootstrap_items()),
        spawn_catalog::SpawnCatalog::load(files.spawns()).unwrap(),
        None,
        crate::resources::tests::wire_profiles(),
        20231,
    )
    .unwrap();
    Engine::from_resources(Vec::new(), 100_000_000, resources)
        .with_match_profiles(match_profiles::Registry::load(files.manifest()).unwrap())
        .unwrap()
}

fn options(seconds: u64, profile: &str, observers: u16) -> HeadlessSimOptions {
    HeadlessSimOptions {
        profile: Some(profile.into()),
        observers,
        ..HeadlessSimOptions::new(seconds)
    }
}

fn lookup(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
    let vars: BTreeMap<String, String> = vars
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    move |name| vars.get(name).cloned()
}

fn tick_lines(ticks: Vec<u8>) -> Vec<Value> {
    String::from_utf8(ticks)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
fn options_default_to_one_observer_and_read_every_setting() {
    let defaults = HeadlessSimOptions::from_lookup(60, lookup(&[])).unwrap();
    assert_eq!(defaults, HeadlessSimOptions::new(60));
    assert_eq!(
        (defaults.observers, defaults.event_hz, defaults.faction),
        (1, 0, None)
    );
    let set = HeadlessSimOptions::from_lookup(
        900,
        lookup(&[
            ("MAG_SIM_PROFILE", "large"),
            ("MAG_SIM_OBSERVERS", "32"),
            ("MAG_SIM_EVENT_HZ", "10"),
            ("MAG_SIM_FACTION", "1"),
            ("MAG_SIM_SNAPSHOTS", "1"),
            ("MAG_SIM_WARMUP_S", "120"),
            ("MAG_SIM_COMPLETE_STRUCTURALS", "24@60, 27@90"),
            ("MAG_SIM_CLOCK", "wall"),
            ("MAG_SIM_STALL", "900@30"),
        ]),
    )
    .unwrap();
    assert_eq!(
        set,
        HeadlessSimOptions {
            seconds: 900,
            profile: Some("large".into()),
            observers: 32,
            event_hz: 10,
            faction: Some(1),
            snapshots: true,
            warmup_seconds: 120,
            structural_completions: vec![(24, 60), (27, 90)],
            wall_clock: true,
            stall: Some((900, 30)),
        }
    );
}

#[test]
fn invalid_options_fail_instead_of_falling_back() {
    let too_fast = (MAX_SIM_EVENT_HZ + 1).to_string();
    for (name, value) in [
        ("MAG_SIM_PROFILE", " "),
        ("MAG_SIM_OBSERVERS", "0"),
        ("MAG_SIM_OBSERVERS", "many"),
        ("MAG_SIM_EVENT_HZ", too_fast.as_str()),
        ("MAG_SIM_FACTION", "256"),
        ("MAG_SIM_SNAPSHOTS", "yes"),
        ("MAG_SIM_WARMUP_S", "60"),
        ("MAG_SIM_COMPLETE_STRUCTURALS", "24"),
        ("MAG_SIM_COMPLETE_STRUCTURALS", "24@0"),
        ("MAG_SIM_COMPLETE_STRUCTURALS", "24@61"),
        ("MAG_SIM_COMPLETE_STRUCTURALS", "x@10"),
        ("MAG_SIM_CLOCK", "real"),
        ("MAG_SIM_STALL", "900@30"),
    ] {
        let error = HeadlessSimOptions::from_lookup(60, lookup(&[(name, value)])).unwrap_err();
        assert!(error.contains(name), "{name}={value:?}: {error}");
    }
}

#[test]
fn tick_timing_is_off_until_enabled() {
    let files = Files::new();
    let mut engine = worker(&files);
    assert_eq!(engine.take_tick_timing(), None);
    engine.enable_tick_timing();
    assert_eq!(
        engine.take_tick_timing(),
        Some(tick_timing::TickTiming::default())
    );
}

#[test]
fn the_selected_profile_sets_the_fill_and_two_observers_drain_their_queues() {
    for (profile, fill) in [("small", 8), ("large", 16)] {
        let files = Files::new();
        let mut engine = worker(&files);
        let mut ticks = Vec::new();
        let report = engine
            .run_headless_sim(&options(10, profile, 2), 1, Some(&mut ticks))
            .unwrap();
        assert_eq!(report.profile.as_deref(), Some(profile));
        assert_eq!(report.tick_errors, 0, "{:?}", report.first_tick_errors);
        // The fill total counts both observers.
        assert_eq!(report.bots_max, fill - 2, "{profile}");
        assert_eq!(report.measured_ticks, 200);
        assert_eq!(report.evictions, 0);
        // Observers alternate attacking and defending factions.
        let factions: Vec<_> = report
            .recipients
            .iter()
            .map(|r| (r.player_id, r.faction))
            .collect();
        assert_eq!(factions, [(0, 0), (1, 1)]);
        for recipient in &report.recipients {
            // Every bot's creation snapshot reached the observer.
            assert!(
                recipient.sent_packets_per_s * 10. >= (fill - 2) as f64,
                "{profile}: {recipient:?}"
            );
            assert!(recipient.sent_bytes_per_s > recipient.sent_packets_per_s);
            assert!(recipient.queue_depth.max < 64., "{recipient:?}");
            assert_eq!(recipient.measured_s, 10.);
            assert_eq!(recipient.evicted_at_ms, None);
        }
        let observers: Vec<_> = engine.peers.values().chain(engine.active.iter()).collect();
        assert_eq!(observers.len(), 2);
        assert!(observers.iter().all(|a| a.shared_frames_drained > 0));

        let lines = tick_lines(ticks);
        assert_eq!(lines.len(), 200);
        let mut sent = [0u64; 2];
        for line in &lines {
            let phases = line["phases_ns"].as_object().unwrap();
            assert_eq!(phases.len(), Phase::ALL.len());
            assert!(Phase::ALL.iter().all(|p| phases.contains_key(p.name())));
            let per_observer = line["observers"].as_array().unwrap();
            assert_eq!(per_observer.len(), 2);
            for (slot, row) in per_observer.iter().enumerate() {
                assert_eq!(row[0], slot);
                sent[slot] += row[1].as_u64().unwrap();
            }
            assert_eq!(
                line["sent_packets"].as_u64().unwrap(),
                per_observer
                    .iter()
                    .map(|row| row[1].as_u64().unwrap())
                    .sum::<u64>()
            );
        }
        for (slot, recipient) in report.recipients.iter().enumerate() {
            assert_eq!(sent[slot] as f64, recipient.sent_packets_per_s * 10.);
            // What was offered is what was sent plus what is still queued.
            let queued = lines.last().unwrap()["observers"][slot][3]
                .as_u64()
                .unwrap();
            assert!(
                (recipient.offered_packets_per_s * 10. - (sent[slot] + queued) as f64).abs() < 1e-6
            );
        }
    }
}

#[test]
fn warmup_is_left_out_of_the_summary_but_not_the_tick_log() {
    let files = Files::new();
    let mut engine = worker(&files);
    let mut ticks = Vec::new();
    let options = HeadlessSimOptions {
        warmup_seconds: 4,
        ..options(10, "small", 1)
    };
    let report = engine
        .run_headless_sim(&options, 1, Some(&mut ticks))
        .unwrap();
    // Ticks at 4.00 s and later are measured.
    assert_eq!(report.measured_ticks, 121);
    assert!((report.recipients[0].measured_s - 121. * 0.05).abs() < 1e-9);
    assert_eq!(tick_lines(ticks).len(), 200);
}

#[test]
fn simulated_client_events_add_shared_drains() {
    let files = Files::new();
    let mut engine = worker(&files);
    let options = HeadlessSimOptions {
        event_hz: 10,
        ..options(10, "large", 2)
    };
    let report = engine.run_headless_sim(&options, 1, None).unwrap();
    assert_eq!(report.tick_errors, 0, "{:?}", report.first_tick_errors);
    assert_eq!(report.event_hz, 10);
    // Two observers at 10 Hz each: one reply drain per 50 ms tick.
    assert!((report.event_drains_per_s - 20.).abs() < 1e-9);
    assert_eq!(report.evictions, 0);
}

#[test]
fn a_backlogged_observer_is_evicted_and_reported() {
    let files = Files::new();
    let mut engine = worker(&files);
    let options = options(2, "small", 2);
    engine.enable_world_ticks();
    engine.enable_tick_timing();
    let joined = engine
        .headless_sim_join(&options, Duration::ZERO, 1)
        .unwrap();
    // A full reliable FIFO marks the recipient; the next tick evicts it, as live.
    engine
        .peers
        .values_mut()
        .chain(engine.active.iter_mut())
        .find(|a| a.reservation.player == 1)
        .unwrap()
        .replication_backlogged = true;
    let mut ticks = Vec::new();
    let report = engine
        .headless_sim_measure(&options, joined, 1, Some(&mut ticks))
        .unwrap();
    assert_eq!(report.evictions, 1);
    let evicted = &report.recipients[1];
    assert_eq!((evicted.player_id, evicted.evicted_at_ms), (1, Some(50)));
    assert_eq!(evicted.measured_s, 0.);
    assert_eq!(report.recipients[0].evicted_at_ms, None);
    assert!(String::from_utf8_lossy(&engine.writer).contains("replication_backlog_released"));
    // The tick log stops listing the evicted observer.
    let lines = tick_lines(ticks);
    assert!(
        lines
            .iter()
            .all(|l| l["observers"].as_array().unwrap().len() == 1)
    );
}

#[test]
fn without_a_profile_the_catalog_world_admits_every_human_seat() {
    let files = Files::new();
    let resources = EngineResources::for_application(
        Arc::new(crate::bot_registry::identity_tests::bootstrap_items()),
        spawn_catalog::SpawnCatalog::load(files.spawns()).unwrap(),
        None,
        crate::resources::tests::wire_profiles(),
        20231,
    )
    .unwrap();
    let mut engine = Engine::from_resources(Vec::new(), 100_000_000, resources);
    let options = HeadlessSimOptions {
        observers: 2,
        ..HeadlessSimOptions::new(1)
    };
    let report = engine.run_headless_sim(&options, 1, None).unwrap();
    assert_eq!(report.tick_errors, 0, "{:?}", report.first_tick_errors);
    // The synthetic catalog seats eight per side.
    assert_eq!(engine.maximum_players, 16);
    assert_eq!(report.recipients.len(), 2);
}

#[test]
fn unknown_profiles_and_too_many_observers_fail_clearly() {
    let files = Files::new();
    let error = worker(&files)
        .run_headless_sim(&options(1, "missing", 1), 1, None)
        .unwrap_err();
    assert!(error.contains("bridge setup failed"), "{error}");
    let error = worker(&files)
        .run_headless_sim(&options(1, "small", 17), 1, None)
        .unwrap_err();
    assert!(error.contains("17 observers exceed"), "{error}");
}
