//! Wire verification using owned gameplay subprocesses and invented catalogs.
#[path = "support/synthetic_runtime_catalogs.rs"]
mod synthetic_runtime_catalogs;

use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    net::{SocketAddr, TcpStream},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};
use synthetic_runtime_catalogs::SyntheticCatalogs;

const DEADLINE: Duration = Duration::from_secs(5);
const WORLD: u32 = 37;
const SLOT: u16 = 0;
const RESERVATION: &str = "synthetic-worker-slot";

struct Worker {
    child: Child,
    address: Option<SocketAddr>,
    log: PathBuf,
    _catalogs: SyntheticCatalogs,
}

impl Worker {
    fn spawn(worker_id: Option<&str>) -> Self {
        let catalogs = SyntheticCatalogs::new();
        let log = catalogs.0.join("worker.jsonl");
        let mut command = Command::new(env!("CARGO_BIN_EXE_mag-gameplay"));
        command
            .args(catalogs.runtime_args())
            .args(["--bind", "127.0.0.1:0", "--log"])
            .arg(&log)
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        if let Some(id) = worker_id {
            command.args(["--worker-id", id]);
        }
        let mut worker = Self {
            child: command.spawn().unwrap(),
            address: None,
            log,
            _catalogs: catalogs,
        };
        let stdout = worker.child.stdout.take().unwrap();
        let (send, receive) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Ok(value) = serde_json::from_str::<Value>(&line)
                    && value["event"] == "listening"
                {
                    send.send(value["address"].as_str().unwrap().parse())
                        .unwrap();
                    return;
                }
            }
        });
        worker.address = Some(
            receive
                .recv_timeout(DEADLINE)
                .expect("synthetic worker never reached the listening state")
                .expect("valid ephemeral worker address"),
        );
        worker
    }

    fn open(&self, line: &str) -> BufReader<TcpStream> {
        let mut stream = TcpStream::connect_timeout(&self.address.unwrap(), DEADLINE).unwrap();
        stream.set_read_timeout(Some(DEADLINE)).unwrap();
        stream.set_write_timeout(Some(DEADLINE)).unwrap();
        writeln!(stream, "{line}").unwrap();
        BufReader::new(stream)
    }

    fn request(&self, request: &Value) -> Value {
        frame(&mut self.open(&request.to_string()))
    }

    fn hello(&self) -> Value {
        self.request(&json!({"op":"hello","v":2,"request_id":"hello"}))
    }

    fn identity(&self, worker_id: &str) -> Value {
        let response = self.hello();
        assert_eq!(response["ok"], true, "{response}");
        assert_eq!(response["request_id"], "hello");
        let identity = response["worker_identity"].clone();
        assert_eq!(identity["worker_id"], worker_id);
        let incarnation = identity["incarnation_id"].as_str().unwrap();
        assert!((1..=64).contains(&incarnation.len()));
        assert!(
            incarnation
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        );
        assert!(
            response["capabilities"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == "worker_registry_v1")
        );
        identity
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn frame(stream: &mut BufReader<TcpStream>) -> Value {
    let mut line = String::new();
    assert!(
        stream.read_line(&mut line).unwrap() > 0,
        "expected JSON frame"
    );
    serde_json::from_str(&line).unwrap()
}

fn with_fence(mut request: Value, fence: &Value) -> Value {
    request["worker_identity"] = fence.clone();
    request
}

fn reserve() -> Value {
    json!({"op":"reserve","v":2,"request_id":"reserve","reservation_id":RESERVATION,
        "world_id":WORLD,"player_id":SLOT,"account_id":1,
        "session_key":"aaaaaaaaaaaaaaaa","access_key":"bbbbbbbbbbbbbbbb",
        "character":{"name":"Synthetic Player","faction":0,"head":0,"voice":0,"universe_id":26}})
}

fn release() -> Value {
    json!({"op":"release","v":2,"request_id":"release","reservation_id":RESERVATION})
}

fn configure() -> Value {
    json!({"op":"configure_match","v":2,"request_id":"configure","application_id":20231,
        "minimum_players":1,"maximum_players":1})
}

fn event() -> Value {
    json!({"op":"event","v":2,"request_id":"request","reservation_id":RESERVATION,
        "world_id":WORLD,"player_id":SLOT,"transport":"tcp","direction":"client_to_server",
        "rt_type":61,"plugin_id":1,"payload_hex":"000b0000000b"})
}

fn progression() -> Value {
    json!({"op":"progression_committed","v":2,"request_id":"progression","reservation_id":RESERVATION,
        "world_id":WORLD,"player_id":SLOT,"reward_id":"synthetic-worker-fence-reward",
        "progression":{"experience":0,"level":1,"skill_points":0},"match_experience":0})
}

fn refresh_progression() -> Value {
    json!({"op":"refresh_progression","v":2,"request_id":"refresh-progression","reservation_id":RESERVATION,
        "world_id":WORLD,"player_id":SLOT,"account_id":"synthetic-progress-account","character_id":1,
        "progression":{"experience":20,"level":2,"skill_points":1}})
}

fn reply(response: Value, request_id: &Value, fence: &Value, error: Option<&str>) {
    assert_eq!(&response["request_id"], request_id, "{response}");
    assert_eq!(&response["worker_identity"], fence, "{response}");
    assert_eq!(response["error"], json!(error), "{response}");
    assert_eq!(response["ok"], error.is_none(), "{response}");
}

#[test]
fn configured_hello_is_read_only_and_legacy_wire_is_unchanged() {
    let worker = Worker::spawn(Some("arena_A"));
    let prior = std::fs::read(&worker.log).unwrap();
    let identity = worker.identity("arena_A");
    let hello = json!({"op":"hello","v":2,"request_id":"bound-hello"});
    reply(
        worker.request(&with_fence(hello.clone(), &identity)),
        &hello["request_id"],
        &identity,
        None,
    );
    let wrong = json!({"worker_id":"arena_A","incarnation_id":"stale"});
    reply(
        worker.request(&with_fence(hello.clone(), &wrong)),
        &hello["request_id"],
        &identity,
        Some("worker_identity_mismatch"),
    );
    assert_eq!(std::fs::read(&worker.log).unwrap(), prior);
    let malformed = frame(&mut worker.open("{"));
    assert_eq!(malformed["error"], "invalid_json_or_schema");
    assert_eq!(malformed["worker_identity"], identity);

    let legacy = Worker::spawn(None);
    let hello = legacy.hello();
    assert_eq!(hello["ok"], true, "{hello}");
    assert!(hello.get("worker_identity").is_none());
    assert!(
        hello["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .all(|v| v != "worker_registry_v1")
    );
    assert_eq!(legacy.request(&configure())["ok"], true);
    let fenced = legacy.request(&with_fence(configure(), &identity));
    assert_eq!(fenced["error"], "invalid_json_or_schema");
    assert!(fenced.get("worker_identity").is_none());
}

#[test]
fn stale_or_missing_fence_rejects_each_operation_before_engine_mutation() {
    let worker = Worker::spawn(Some("arena-A"));
    let identity = worker.identity("arena-A");
    let stale = json!({"worker_id":"arena-A","incarnation_id":"superseded"});
    let prior = std::fs::read(&worker.log).unwrap();
    let select = json!({"op":"select_match_profile","v":2,"request_id":"select",
        "formation_id":"synthetic-formation","profile_id":"synthetic-profile"});
    for request in [
        select.clone(),
        configure(),
        reserve(),
        event(),
        progression(),
        refresh_progression(),
        release(),
    ] {
        reply(
            worker.request(&request),
            &request["request_id"],
            &identity,
            Some("worker_identity_required"),
        );
        reply(
            worker.request(&with_fence(request.clone(), &stale)),
            &request["request_id"],
            &identity,
            Some("worker_identity_mismatch"),
        );
    }
    assert_eq!(std::fs::read(&worker.log).unwrap(), prior);

    // These must reach the typed Engine handlers.
    let mut persistent_reservation = reserve();
    persistent_reservation["character"]["account_id"] = json!("synthetic-progress-account");
    persistent_reservation["character"]["character_id"] = json!(1);
    persistent_reservation["character"]["progression"] =
        json!({"experience":0,"level":1,"skill_points":0});
    for (request, error) in [
        (select, Some("match_profiles_disabled")),
        (configure(), None),
        (persistent_reservation, None),
        (event(), None),
        (progression(), Some("unknown_reward_id")),
        (refresh_progression(), None),
        (release(), None),
    ] {
        reply(
            worker.request(&with_fence(request.clone(), &identity)),
            &request["request_id"],
            &identity,
            error,
        );
    }
}

#[test]
fn fenced_requests_keep_the_existing_strict_schema() {
    let worker = Worker::spawn(Some("arena-A"));
    let identity = worker.identity("arena-A");
    let encoded = with_fence(reserve(), &identity).to_string();
    for malformed in [
        encoded.replacen("\"faction\":0", "\"faction\":1,\"faction\":0", 1),
        encoded.replacen(
            "\"request_id\":",
            "\"request_id\":\"another\",\"request_id\":",
            1,
        ),
        encoded.replacen(
            "\"worker_identity\":",
            "\"worker_identity\":null,\"worker_identity\":",
            1,
        ),
        encoded.replacen("\"character\":", "\"unexpected\":true,\"character\":", 1),
    ] {
        assert_ne!(malformed, encoded);
        reply(
            frame(&mut worker.open(&malformed)),
            &json!("reserve"),
            &identity,
            Some("invalid_json_or_schema"),
        );
    }
    let wrong =
        json!({"worker_id":"arena-A","incarnation_id":identity["incarnation_id"],"extra":true});
    reply(
        worker.request(&with_fence(reserve(), &wrong)),
        &json!("reserve"),
        &identity,
        Some("worker_identity_mismatch"),
    );
    reply(
        worker.request(&with_fence(configure(), &identity)),
        &json!("configure"),
        &identity,
        None,
    );
    assert_eq!(
        worker.request(&with_fence(reserve(), &identity))["ok"],
        true
    );
}

#[test]
fn stale_subscriber_cannot_claim_slot_and_tick_frames_carry_the_current_fence() {
    let worker = Worker::spawn(Some("arena-A"));
    let identity = worker.identity("arena-A");
    let stale = json!({"worker_id":"arena-A","incarnation_id":"superseded"});
    let subscribe = json!({"op":"subscribe_world","v":2,"request_id":"subscribe","world_id":WORLD});
    reply(
        worker.request(&subscribe),
        &subscribe["request_id"],
        &identity,
        Some("worker_identity_required"),
    );
    reply(
        worker.request(&with_fence(subscribe.clone(), &stale)),
        &subscribe["request_id"],
        &identity,
        Some("worker_identity_mismatch"),
    );
    let mut valid = worker.open(&with_fence(subscribe.clone(), &identity).to_string());
    reply(frame(&mut valid), &subscribe["request_id"], &identity, None);
    reply(frame(&mut valid), &subscribe["request_id"], &identity, None);
    reply(
        worker.request(&with_fence(subscribe.clone(), &identity)),
        &subscribe["request_id"],
        &identity,
        Some("world_subscription_exists"),
    );
}

#[test]
fn workers_with_colliding_indices_and_restarted_worker_have_separate_state() {
    let first = Worker::spawn(Some("arena-A"));
    let identity_a = first.identity("arena-A");
    let second = Worker::spawn(Some("arena-B"));
    let identity_b = second.identity("arena-B");
    for (worker, identity) in [(&first, &identity_a), (&second, &identity_b)] {
        reply(
            worker.request(&with_fence(configure(), identity)),
            &json!("configure"),
            identity,
            None,
        );
        reply(
            worker.request(&with_fence(reserve(), identity)),
            &json!("reserve"),
            identity,
            None,
        );
    }

    reply(
        second.request(&with_fence(release(), &identity_a)),
        &json!("release"),
        &identity_b,
        Some("worker_identity_mismatch"),
    );
    reply(
        first.request(&with_fence(release(), &identity_a)),
        &json!("release"),
        &identity_a,
        None,
    );
    // A reconnecting Horizon re-sends the same limits: confirmed, not locked.
    reply(
        second.request(&with_fence(configure(), &identity_b)),
        &json!("configure"),
        &identity_b,
        None,
    );
    let mut changed = configure();
    changed["maximum_players"] = json!(2);
    reply(
        second.request(&with_fence(changed, &identity_b)),
        &json!("configure"),
        &identity_b,
        Some("match_configuration_locked"),
    );
    reply(
        first.request(&with_fence(event(), &identity_a)),
        &json!("request"),
        &identity_a,
        Some("no_reservation"),
    );
    let response_b = second.request(&with_fence(event(), &identity_b));
    reply(response_b.clone(), &json!("request"), &identity_b, None);
    let messages = response_b["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["world_id"], WORLD);
    assert_eq!(messages[0]["player_id"], SLOT);
    assert_eq!(messages[0]["reservation_id"], RESERVATION);
    let payload = messages[0]["payload_hex"].as_str().unwrap();
    assert!(payload.starts_with("000c"));
    assert!(payload.ends_with("0000000b"));
    drop(first);

    let replacement = Worker::spawn(Some("arena-A"));
    let replacement_id = replacement.identity("arena-A");
    assert_ne!(
        replacement_id["incarnation_id"],
        identity_a["incarnation_id"]
    );
    reply(
        replacement.request(&with_fence(reserve(), &identity_a)),
        &json!("reserve"),
        &replacement_id,
        Some("worker_identity_mismatch"),
    );
    reply(
        replacement.request(&with_fence(configure(), &replacement_id)),
        &json!("configure"),
        &replacement_id,
        None,
    );
    reply(
        replacement.request(&with_fence(reserve(), &replacement_id)),
        &json!("reserve"),
        &replacement_id,
        None,
    );
}

#[test]
fn served_worker_refuses_admission_until_match_is_configured() {
    // A restarted worker must not admit anyone under the singleton default.
    let legacy = Worker::spawn(None);
    assert_eq!(legacy.hello()["ok"], true);
    let refused = legacy.request(&reserve());
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(refused["error"], "match_not_configured", "{refused}");
    assert_eq!(legacy.request(&configure())["ok"], true);
    let admitted = legacy.request(&reserve());
    assert_eq!(admitted["ok"], true, "{admitted}");

    let worker = Worker::spawn(Some("arena-A"));
    let identity = worker.identity("arena-A");
    reply(
        worker.request(&with_fence(reserve(), &identity)),
        &json!("reserve"),
        &identity,
        Some("match_not_configured"),
    );
    reply(
        worker.request(&with_fence(configure(), &identity)),
        &json!("configure"),
        &identity,
        None,
    );
    reply(
        worker.request(&with_fence(reserve(), &identity)),
        &json!("reserve"),
        &identity,
        None,
    );
}

#[test]
fn cli_validates_worker_id_bounds_before_startup() {
    let oversized = "x".repeat(65);
    for invalid in [None, Some(""), Some("a/b"), Some("é"), Some(&oversized)] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_mag-gameplay"))
            .arg("--worker-id")
            .args(invalid)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + DEADLINE;
        while child.try_wait().unwrap().is_none() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        if child.try_wait().unwrap().is_none() {
            let _ = child.kill();
        }
        let output = child.wait_with_output().unwrap();
        assert!(!output.status.success());
        let error = String::from_utf8(output.stderr).unwrap();
        assert!(error.contains("--worker-id requires 1..=64"), "{error}");
    }

    let id = "x".repeat(64);
    let worker = Worker::spawn(Some(&id));
    worker.identity(&id);
}

fn event_records(worker: &Worker) -> usize {
    std::fs::read_to_string(&worker.log)
        .unwrap_or_default()
        .lines()
        .filter(|line| line.contains(r#""op":"event""#))
        .count()
}

/// Wait for the asynchronous event log to settle on a record count.
fn settled_event_records(worker: &Worker) -> usize {
    let mut seen = event_records(worker);
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        std::thread::sleep(Duration::from_millis(200));
        let now = event_records(worker);
        if now == seen || Instant::now() > deadline {
            return now;
        }
        seen = now;
    }
}

#[test]
fn served_world_stream_resumes_on_the_same_process_and_events_apply_once() {
    let worker = Worker::spawn(None);
    let hello = worker.hello();
    for capability in ["world_stream_resume_v1", "event_replay_v1"] {
        assert!(
            hello["capabilities"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == capability),
            "{hello}"
        );
    }
    assert_eq!(worker.request(&configure())["ok"], true);
    assert_eq!(worker.request(&reserve())["ok"], true);

    // A retried event ID is applied once and answered with the same reply.
    let before = settled_event_records(&worker);
    let first = worker.request(&event());
    assert_eq!(first["ok"], true, "{first}");
    let applied = settled_event_records(&worker);
    assert_eq!(applied, before + 1, "the event was not recorded");
    assert_eq!(worker.request(&event()), first, "a retry got another reply");
    assert_eq!(
        settled_event_records(&worker),
        applied,
        "a retry was applied again"
    );

    let subscribe = json!({"op":"subscribe_world","v":2,"request_id":"subscribe",
        "world_id":WORLD,"positions":true});
    let mut stream = worker.open(&subscribe.to_string());
    let ack = frame(&mut stream);
    assert_eq!(ack["ok"], true, "{ack}");
    let epoch = ack["world_stream"]["epoch"].as_str().unwrap().to_owned();
    assert_eq!(epoch.len(), 32);
    let base = ack["world_stream"]["sequence"].as_u64().unwrap();
    let handled = frame(&mut stream);
    assert_eq!(handled["world_stream"]["sequence"], base + 1, "{handled}");
    let lost = frame(&mut stream);
    assert_eq!(lost["world_stream"]["sequence"], base + 2, "{lost}");
    drop(stream);

    // Horizon handled only base + 1; resume replays the rest in order.
    let resume = |epoch: &str, after: u64| {
        json!({"op":"subscribe_world","v":2,"request_id":"resume","world_id":WORLD,
            "positions":true,"resume":{"epoch":epoch,"after":after}})
        .to_string()
    };
    let deadline = Instant::now() + DEADLINE;
    let (mut resumed, ack) = loop {
        let mut stream = worker.open(&resume(&epoch, base + 1));
        let ack = frame(&mut stream);
        // The broken subscription is retired once the worker sees its close.
        if ack["error"] != "world_subscription_exists" || Instant::now() > deadline {
            break (stream, ack);
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    assert_eq!(ack["ok"], true, "{ack}");
    assert_eq!(ack["request_id"], "resume");
    assert_eq!(ack["world_stream"]["sequence"], base + 1);
    let replayed = frame(&mut resumed);
    assert_eq!(replayed["world_stream"]["sequence"], base + 2, "{replayed}");
    assert_eq!(replayed["request_id"], "resume");
    let live = frame(&mut resumed);
    assert_eq!(live["world_stream"]["sequence"], base + 3, "{live}");
    drop(resumed);

    // Another process's position is refused.
    let deadline = Instant::now() + DEADLINE;
    let refused = loop {
        let refused = frame(&mut worker.open(&resume("00000000000000000000000000000000", 1)));
        if refused["error"] != "world_subscription_exists" || Instant::now() > deadline {
            break refused;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(refused["error"], "world_resume_unavailable");
}
