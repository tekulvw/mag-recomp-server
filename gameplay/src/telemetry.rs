//! Optional timing evidence; telemetry storage must never block gameplay.
use serde::Serialize;
use std::{
    io::Write,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, SyncSender},
    },
};
#[derive(Serialize)]
pub struct TickSample {
    scheduler: &'static str,
    gameplay_update: bool,
    started_us: u64,
    lock_wait_us: u64,
    engine_us: u64,
    bridge_ok: bool,
    budget_us: Option<u64>,
    over_budget: Option<bool>,
}
impl TickSample {
    pub fn independent(mut self) -> Self {
        self.scheduler = "independent_50ms";
        self
    }
    pub fn new(
        gameplay_update: bool,
        started_us: u64,
        lock_wait_us: u64,
        engine_us: u64,
        bridge_ok: bool,
        budget_us: Option<u64>,
    ) -> Self {
        Self {
            scheduler: "client_driven",
            gameplay_update,
            started_us,
            lock_wait_us,
            engine_us,
            bridge_ok,
            budget_us,
            over_budget: budget_us.map(|budget| lock_wait_us.saturating_add(engine_us) > budget),
        }
    }
}
#[derive(Clone)]
pub struct TickRecorder {
    sender: SyncSender<TickSample>,
    dropped: Arc<AtomicU64>,
}
impl TickRecorder {
    pub fn start<W: Write + Send + 'static>(mut writer: W, max_bytes: u64) -> Self {
        let (sender, receiver) = mpsc::sync_channel::<TickSample>(256);
        let dropped = Arc::new(AtomicU64::new(0));
        let observed_drops = dropped.clone();
        std::thread::spawn(move || {
            let mut written = 0_u64;
            for sample in receiver {
                let row = serde_json::json!({"event":"server_step_timing", "schema_version":1,
                    "scheduler":sample.scheduler, "sample":sample,
                    "dropped_samples_total":observed_drops.load(Ordering::Relaxed)});
                let mut bytes = serde_json::to_vec(&row).expect("finite integer telemetry");
                bytes.push(b'\n');
                if written.saturating_add(bytes.len() as u64) > max_bytes {
                    eprintln!("tick telemetry stopped: byte limit reached");
                    break;
                }
                if writer
                    .write_all(&bytes)
                    .and_then(|_| writer.flush())
                    .is_err()
                {
                    eprintln!("tick telemetry stopped: writer failed");
                    break;
                }
                written += bytes.len() as u64;
            }
        });
        Self { sender, dropped }
    }
    pub fn record(&self, sample: TickSample) {
        // Never wait on disk, a full queue, or a disconnected telemetry writer.
        if self.sender.try_send(sample).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
    pub fn dropped_samples(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}
