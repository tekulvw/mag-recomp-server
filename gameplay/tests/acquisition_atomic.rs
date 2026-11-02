//! Invented owned effect world. This tests the adapter, not native physics parity.
use mag_gameplay::{
    acquisition_atomic::*,
    acquisition_control::{Catalog, Command, ObservedState, UnboundCall},
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
#[path = "support/acquisition_fixture.rs"]
mod support;

#[derive(Clone, Debug, PartialEq)]
struct World {
    states: BTreeMap<u32, ObservedState>,
    targets: BTreeMap<u32, u8>,
    hp: i32,
    vehicles: u8,
    trace: Vec<String>,
}
struct Backend {
    world: World,
    fail: Option<&'static str>,
    external_revision: std::sync::Arc<std::sync::atomic::AtomicU64>,
}
struct Stage {
    world: World,
    fail: Option<&'static str>,
    external_revision: u64,
}
impl Backend {
    fn new(fail: Option<&'static str>) -> Self {
        let mut states: BTreeMap<_, _> = (100..108).map(|x| (x, ObservedState::Other)).collect();
        states.insert(102, ObservedState::Completed);
        Self {
            world: World {
                states,
                targets: BTreeMap::new(),
                hp: 100,
                vehicles: 0,
                trace: Vec::new(),
            },
            fail,
            external_revision: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
        }
    }
}
impl Stage {
    fn phase(&mut self, phase: &'static str) -> Result<(), &'static str> {
        self.world.trace.push(phase.into());
        if self.fail == Some(phase) {
            Err("unsupported_or_failed_effect")
        } else {
            Ok(())
        }
    }
}
impl EffectBackend for Backend {
    type Output = Vec<String>;
    type Stage = Stage;
    fn begin(&self) -> Result<Stage, &'static str> {
        if self.fail == Some("begin") {
            return Err("missing_effect_backend");
        }
        Ok(Stage {
            world: self.world.clone(),
            fail: self.fail,
            external_revision: self
                .external_revision
                .load(std::sync::atomic::Ordering::Relaxed),
        })
    }
    fn check(&self, stage: &Stage) -> Result<(), &'static str> {
        if self
            .external_revision
            .load(std::sync::atomic::Ordering::Relaxed)
            != stage.external_revision
        {
            Err("stale_effect_inputs")
        } else {
            Ok(())
        }
    }
    fn install(&mut self, stage: Stage) {
        self.world = stage.world;
    }
}
impl CallbackStage for Stage {
    type Output = Vec<String>;
    fn enter(&mut self, entry: &Entry) -> Result<(), &'static str> {
        self.world.states.insert(
            entry.objective_token,
            if self.fail == Some("entry_state") {
                ObservedState::Other
            } else {
                entry.state.observed()
            },
        );
        self.world.vehicles += 1;
        self.phase("entry")
    }
    fn observations(&self) -> Result<BTreeMap<u32, ObservedState>, &'static str> {
        if self.fail == Some("observations") {
            return Err("missing_observations");
        }
        Ok(self.world.states.clone())
    }
    fn command(&mut self, command: &Command) -> Result<(), &'static str> {
        match command {
            Command::Objective { target, .. } => {
                self.world.targets.insert(*target, 1);
                if self.fail == Some("current_changed") {
                    self.world.states.insert(101, ObservedState::Completed);
                }
                self.phase("objective")
            }
            Command::DamageVolume { .. } => {
                self.world.hp = 0;
                self.phase("damage")
            }
        }
    }
    fn trailing(&mut self, _call: &UnboundCall) -> Result<(), &'static str> {
        if self.fail == Some("trailing_state") {
            self.world.states.insert(101, ObservedState::Completed);
        }
        self.phase("trailing")
    }
    fn assignments(&mut self, _entry: &Entry) -> Result<(), &'static str> {
        self.phase("assignments")
    }
    fn validate(&self) -> Result<(), &'static str> {
        if self.fail == Some("publication") {
            Err("invalid_publication")
        } else {
            Ok(())
        }
    }
    fn audit(&self) -> Value {
        json!({"trace":self.world.trace,"hp":self.world.hp,"vehicles":self.world.vehicles})
    }
    fn output(&self) -> Vec<String> {
        self.world.trace.clone()
    }
}
fn adapter(fail: Option<&'static str>, capacity: usize) -> AtomicCallbacks<Backend> {
    let (spawns, mut graph) = support::fixture(0);
    graph["groups"][1]["branches"][0]["commands"][0]["arguments"] = json!([-1, -1, -1, -1]);
    AtomicCallbacks::new(
        Catalog::from_json(&graph.to_string(), &spawns).unwrap(),
        Backend::new(fail),
        WorldKey {
            world_id: 1,
            incarnation: 4,
        },
        capacity,
    )
    .unwrap()
}
fn request(id: u64) -> Request {
    Request {
        world: WorldKey {
            world_id: 1,
            incarnation: 4,
        },
        event_id: id,
        entry: Entry {
            objective_token: 100,
            state: EnteredState::Completed,
            actor_index: 7,
            timestamp_ms: 80,
        },
    }
}

#[test]
fn all_effects_are_staged_in_native_order_and_recorded_before_install() {
    let mut a = adapter(None, 4);
    let before = a.backend().world.clone();
    let prepared = a.prepare(request(1)).unwrap();
    assert_eq!(a.backend().world, before);
    let mut recorded = None;
    let outcome = a
        .commit(prepared, |audit, output| {
            recorded = Some((audit.clone(), output.clone()));
            Ok(())
        })
        .unwrap();
    assert!(!outcome.replayed);
    assert_eq!(
        outcome.output,
        ["entry", "objective", "damage", "trailing", "assignments"]
    );
    assert_eq!(a.backend().world.hp, 0);
    assert_eq!(a.backend().world.vehicles, 1);
    assert_eq!(a.backend().world.states[&101], ObservedState::Other);
    assert_eq!(recorded.unwrap().0.backend["hp"], 0);
}

#[test]
fn every_failed_phase_and_recording_failure_leave_live_state_unchanged() {
    for phase in [
        "begin",
        "entry",
        "entry_state",
        "observations",
        "objective",
        "current_changed",
        "damage",
        "trailing",
        "trailing_state",
        "assignments",
        "publication",
    ] {
        let a = adapter(Some(phase), 4);
        let before = a.backend().world.clone();
        assert!(a.prepare(request(1)).is_err(), "{phase}");
        assert_eq!(a.backend().world, before);
    }
    let mut a = adapter(None, 4);
    let before = a.backend().world.clone();
    let p = a.prepare(request(1)).unwrap();
    assert!(a.commit(p, |_, _| Err("disk_failure")).is_err());
    assert_eq!(a.backend().world, before);
    let p = a.prepare(request(1)).unwrap();
    assert!(!a.commit(p, |_, _| Ok(())).unwrap().replayed);
}

#[test]
fn cached_replay_does_not_record_or_apply_effects_twice_and_ids_cannot_be_reused() {
    let mut a = adapter(None, 2);
    let p = a.prepare(request(1)).unwrap();
    let first = a.commit(p, |_, _| Ok(())).unwrap();
    let p = a.prepare(request(1)).unwrap();
    let replay = a
        .commit(p, |_, _| panic!("must not record replay"))
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(first.output, replay.output);
    assert_eq!(a.backend().world.vehicles, 1);
    let mut changed = request(1);
    changed.entry.actor_index = 8;
    assert!(a.prepare(changed).is_err());
    for id in [2, 3] {
        let p = a.prepare(request(id)).unwrap();
        a.commit(p, |_, _| Ok(())).unwrap();
    }
    assert!(a.prepare(request(1)).is_err());
}

#[test]
fn stale_or_foreign_preparations_and_wrong_world_are_rejected_before_record() {
    let mut a = adapter(None, 4);
    let old = a.prepare(request(2)).unwrap();
    let p = a.prepare(request(1)).unwrap();
    a.commit(p, |_, _| Ok(())).unwrap();
    assert!(
        a.commit(old, |_, _| panic!("stale must not record"))
            .is_err()
    );
    let other = adapter(None, 4);
    let p = other.prepare(request(2)).unwrap();
    assert!(
        a.commit(p, |_, _| panic!("foreign must not record"))
            .is_err()
    );
    let mut wrong = request(2);
    wrong.world.incarnation = 5;
    assert!(a.prepare(wrong).is_err());
    let mut unknown = request(2);
    unknown.entry.objective_token = 999;
    assert!(a.prepare(unknown).is_err());
    let p = a.prepare(request(2)).unwrap();
    a.backend()
        .external_revision
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    assert!(
        a.commit(p, |_, _| panic!("stale external inputs must not record"))
            .is_err()
    );
    assert!(
        AtomicCallbacks::new(
            {
                let (s, g) = support::fixture(0);
                Catalog::from_json(&g.to_string(), &s).unwrap()
            },
            Backend::new(None),
            WorldKey {
                world_id: 1,
                incarnation: 4
            },
            0
        )
        .is_err()
    );
}

#[test]
fn duplicate_preparation_replays_after_commit_but_evicted_replay_stays_rejected() {
    let mut a = adapter(None, 1);
    let first = a.prepare(request(1)).unwrap();
    let duplicate = a.prepare(request(1)).unwrap();
    a.commit(first, |_, _| Ok(())).unwrap();
    assert!(
        a.commit(duplicate, |_, _| panic!("duplicate must not record"))
            .unwrap()
            .replayed
    );
    let old_replay = a.prepare(request(1)).unwrap();
    let next = a.prepare(request(2)).unwrap();
    a.commit(next, |_, _| Ok(())).unwrap();
    assert!(
        a.commit(old_replay, |_, _| panic!("evicted must not record"))
            .is_err()
    );
}
