//! SYNTHETIC passage-queue regressions for a bot retired while it holds a
//! drop passage. A human joining a squad retires the bot on its seat; the
//! bot's permit, convoy slot and waiting tickets must go with it. The live
//! failure: a bot granted a passage was retired for a joining human, and the
//! next admission pass indexed the registry by the stale owner and panicked
//! the world thread. No game catalogs, meshes or captures.
use super::*;

const PASSAGE: u32 = 363;
const OTHER_PASSAGE: u32 = 221;

fn passage(id: u32) -> DropPassage {
    let body = bot_physics::Body::default();
    let (launch, landing) = ([4., 0., 4.], [4., 0., 8.]);
    DropPassage {
        id,
        launch,
        landing,
        protected_bounds: crate::actor_collision::Bounds::body(launch, body)
            .union(crate::actor_collision::Bounds::body(landing, body)),
        phase: DropPhase::Approaching,
        distance_to_launch: 2.,
        route_distance_to_launch: 2.,
    }
}

fn life(engine: &Engine<Vec<u8>>, id: u16) -> PassageLife {
    PassageLife {
        id,
        generation: engine.bot_registry.bots[&id]
            .target
            .runtime
            .unwrap()
            .generation,
    }
}

fn owner(engine: &Engine<Vec<u8>>, id: u16, drop_id: u32) -> PassageOwner {
    PassageOwner {
        life: life(engine, id),
        supply_owner: None,
        passage: passage(drop_id),
        last_position: engine.bot_registry.bots[&id]
            .target
            .runtime
            .unwrap()
            .position,
        progress_at: Duration::from_millis(900),
        entered: false,
        cert_started: false,
    }
}

/// Two bots in an autonomous population. Bot `a` owns [`PASSAGE`] (just
/// granted, as on the live server) and also waits for [`OTHER_PASSAGE`]
/// behind bot `b`'s convoy slot.
fn granted_owner_fixture() -> (Engine<Vec<u8>>, u16, u16) {
    let mut engine = identity_tests::populated_engine("Synthetic Retired Owner");
    let config = engine.bot_registry.config.as_mut().unwrap();
    config.autonomous = true;
    config.objective_policy = Some(crate::bot_objectives::ObjectivePolicy::bundled());
    let ids: Vec<u16> = engine.bot_registry.bots.keys().copied().collect();
    let (a, b) = (ids[0], ids[1]);
    let held = owner(&engine, a, PASSAGE);
    let convoy = owner(&engine, b, OTHER_PASSAGE);
    let waiting = life(&engine, a);
    let queues = &mut engine.bot_registry.drop_queues;
    queues.insert(
        PASSAGE,
        PassageQueue {
            owner: Some(held),
            ..Default::default()
        },
    );
    queues.insert(
        OTHER_PASSAGE,
        PassageQueue {
            departing: vec![convoy],
            waiting: [waiting].into(),
            ..Default::default()
        },
    );
    engine
        .bot_registry
        .bots
        .get_mut(&a)
        .unwrap()
        .drop_queue_passage = Some(passage(PASSAGE));
    (engine, a, b)
}

fn names_bot(engine: &Engine<Vec<u8>>, id: u16) -> bool {
    engine
        .bot_registry
        .drop_queues
        .values()
        .any(|q| q.holders().any(|o| o.life.id == id) || q.waiting.iter().any(|life| life.id == id))
}

fn admissions(engine: &Engine<Vec<u8>>) -> Vec<Value> {
    String::from_utf8_lossy(&engine.writer)
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|row| row["op"] == "bot_drop_admission")
        .collect()
}

#[test]
fn retiring_a_bot_for_a_human_forgets_its_passage_permits_and_tickets() {
    let (mut engine, a, b) = granted_owner_fixture();
    let now = Duration::from_millis(1000);
    engine.retire_bot_for_human(a, now, 1).unwrap();
    assert!(!engine.bot_registry.bots.contains_key(&a));
    assert!(!names_bot(&engine, a), "a retired bot keeps no queue entry");
    // The owned passage has nothing left and is dropped; the other keeps the
    // remaining bot's convoy slot.
    assert!(!engine.bot_registry.drop_queues.contains_key(&PASSAGE));
    let other = &engine.bot_registry.drop_queues[&OTHER_PASSAGE];
    assert_eq!(other.departing.len(), 1);
    assert_eq!(other.departing[0].life.id, b);
    assert!(other.waiting.is_empty());
    let released: Vec<_> = admissions(&engine)
        .into_iter()
        .filter(|row| row["reason"] == "retired_for_human")
        .map(|row| {
            assert_eq!(row["action"], "released");
            assert_eq!(row["object_index"], a);
            (row["drop_id"].as_u64().unwrap(), row["queue_role"].clone())
        })
        .collect();
    assert_eq!(
        released,
        [
            (u64::from(OTHER_PASSAGE), json!("waiting")),
            (u64::from(PASSAGE), json!("owner")),
        ]
    );
    // The live crash: the next admission pass after the retirement.
    engine
        .tick_drop_admissions(now + Duration::from_millis(50), 1)
        .unwrap();
    assert!(!names_bot(&engine, a));
}

#[test]
fn an_admission_pass_releases_an_owner_whose_bot_is_gone_instead_of_panicking() {
    // Any other path that drops a bot without its queue entries must not
    // take the world thread down: the pass releases the stale permit.
    let (mut engine, a, _) = granted_owner_fixture();
    engine.bot_registry.bots.remove(&a);
    engine
        .tick_drop_admissions(Duration::from_millis(1000), 1)
        .unwrap();
    assert!(!names_bot(&engine, a));
    assert!(admissions(&engine).iter().any(|row| {
        row["action"] == "released"
            && row["reason"] == "life_removed_noncollidable"
            && row["object_index"] == a
            && row["drop_id"] == PASSAGE
    }));
}

#[test]
fn traffic_lanes_skip_a_waiting_ticket_whose_bot_is_gone() {
    let (mut engine, a, b) = granted_owner_fixture();
    // A passage with no owner whose first waiter has left the registry.
    engine
        .bot_registry
        .drop_queues
        .get_mut(&PASSAGE)
        .unwrap()
        .owner = None;
    let waiting = life(&engine, a);
    engine
        .bot_registry
        .drop_queues
        .get_mut(&PASSAGE)
        .unwrap()
        .waiting
        .push_back(waiting);
    engine.bot_registry.bots.remove(&a);
    assert!(engine.bot_registry.traffic_lanes(b).is_empty());
}
