//! Synthetic Domination lifecycle: the world goes idle once its last human leaves.
use crate::*;
use std::time::Duration;

fn records(engine: &Engine<Vec<u8>>, op: &str) -> usize {
    String::from_utf8_lossy(&engine.writer)
        .matches(&format!("\"op\":\"{op}\""))
        .count()
}

fn ms(ms: u64) -> Duration {
    Duration::from_millis(ms)
}

#[test]
fn domination_stops_ticking_after_the_last_release_and_the_next_match_still_forms() {
    use crate::spawn_progression::tests::{ready, reserve};
    let mut e = crate::domination_gameplay_engine::tests::structural_repair::repair_fixture();
    e.ground_runtime = Some(Default::default());
    let state = &mut e.domination_gameplay.as_mut().unwrap().state;
    state.ground_respawn_physics = Some(Default::default());
    // The synthetic mission start links its FSP vehicle to an empty spawn.
    std::sync::Arc::make_mut(&mut state.known_empty_vehicle_spawns).insert(2);
    // Its ability command names the synthetic platoon token 31.
    std::sync::Arc::make_mut(&mut state.platoons).insert(31, 0);
    e.enable_world_ticks();
    // A running match ticks and records the Domination world.
    for step in 1..=3 {
        let reply = e.tick_world(ms(step * 50), 1, "tick".into());
        assert!(reply.ok, "{:?}", reply.error);
    }
    assert!(records(&e, "domination_gameplay") >= 3);

    let reply = e.handle(
        Request::Release {
            v: BRIDGE_VERSION,
            request_id: "release".into(),
            reservation_id: "synthetic-reservation-0000".into(),
        },
        ms(200),
        1,
    );
    assert!(reply.ok, "{:?}", reply.error);
    assert!(e.active.is_none() && e.peers.is_empty());
    // The quorum-free world still counts as started, as it did live.
    assert!(e.match_started);
    let released = e.writer.len();
    for step in 5..=40 {
        let reply = e.tick_world(ms(step * 50), 1, "tick".into());
        assert!(reply.ok, "{:?}", reply.error);
        e.tick_objectives(ms(step * 50 + 1), 1).unwrap();
    }
    let after_release = String::from_utf8_lossy(&e.writer[released..]).into_owned();
    assert_eq!(
        after_release
            .matches("\"op\":\"domination_gameplay\"")
            .count(),
        0,
        "an empty Domination world does not tick"
    );
    assert_eq!(after_release.lines().count(), 0, "nor record anything else");

    // The next cohort forms normally: a reserved human that has not entered
    // keeps the world idle, and entering starts the new mission.
    e.item_catalog = crate::spawn_progression::tests::engine().item_catalog;
    reserve(&mut e, 1);
    let reserved = e.writer.len();
    let reply = e.tick_world(ms(2100), 1, "tick".into());
    assert!(reply.ok, "{:?}", reply.error);
    assert_eq!(e.writer.len(), reserved, "not entered yet: still idle");
    ready(e.active.as_mut().unwrap());
    let reply = e.tick_world(ms(2150), 1, "tick".into());
    assert!(reply.ok, "{:?}", reply.error);
    let after = String::from_utf8_lossy(&e.writer[reserved..]).into_owned();
    assert!(after.contains("\"op\":\"domination_gameplay\""));
    assert!(
        after.contains("domination_mission_start"),
        "the next match starts its mission"
    );
}
