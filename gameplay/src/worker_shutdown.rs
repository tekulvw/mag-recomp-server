//! SIGTERM and SIGINT for a served worker. The container runs this binary as
//! PID 1, where the kernel ignores a signal that has no handler: a rollout's
//! SIGTERM used to do nothing until the kubelet's SIGKILL, leaving the world
//! half alive for the whole grace period. The first signal starts a drain; a
//! second one exits at once.
use mag_gameplay::Engine;
use std::io::{self, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static SIGNALS: AtomicUsize = AtomicUsize::new(0);
const SIGINT: i32 = 2;
const SIGTERM: i32 = 15;
/// 128 + SIGTERM, as the default action would report.
const FORCED_EXIT: i32 = 143;

unsafe extern "C" {
    fn signal(signum: i32, handler: extern "C" fn(i32)) -> usize;
    fn _exit(status: i32) -> !;
}

extern "C" fn on_signal(_: i32) {
    // Async-signal-safe: one atomic and, on the second signal, _exit.
    if SIGNALS.fetch_add(1, Ordering::SeqCst) >= 1 {
        unsafe { _exit(FORCED_EXIT) }
    }
}

/// Install the handlers. Only a served worker does this; one-shot commands
/// keep the default action so an interrupt still stops them.
pub fn install() {
    unsafe {
        signal(SIGTERM, on_signal);
        signal(SIGINT, on_signal);
    }
}

pub fn requested() -> bool {
    SIGNALS.load(Ordering::SeqCst) > 0
}

/// How long a draining worker waits.
#[derive(Clone, Copy, Debug)]
pub struct DrainPolicy {
    /// Longest wait for every human to leave (the match ends and results are
    /// delivered, or Horizon releases them). Zero ends at once.
    pub drain: Duration,
    /// Then the longest wait for Horizon to acknowledge rewards and loadout
    /// selections already reported.
    pub receipt_flush: Duration,
}

impl Default for DrainPolicy {
    fn default() -> Self {
        Self {
            drain: Duration::ZERO,
            receipt_flush: Duration::from_secs(5),
        }
    }
}

pub const MAXIMUM_DRAIN_SECONDS: u64 = 86_400;
pub const MAXIMUM_RECEIPT_FLUSH_SECONDS: u64 = 600;

/// The journal writer, which the exit path closes so queued records reach disk
/// (process::exit runs no destructors).
#[derive(Clone)]
pub struct ClosableLog(Arc<Mutex<Option<Box<dyn Write + Send>>>>);

impl ClosableLog {
    pub fn new(writer: Box<dyn Write + Send>) -> Self {
        Self(Arc::new(Mutex::new(Some(writer))))
    }
    pub fn close(&self) {
        let writer = self.0.lock().ok().and_then(|mut writer| writer.take());
        drop(writer);
    }
    fn with<T>(&self, f: impl FnOnce(&mut (dyn Write + Send)) -> io::Result<T>) -> io::Result<T> {
        let mut guard = self
            .0
            .lock()
            .map_err(|_| io::Error::other("event log unavailable"))?;
        let writer = guard
            .as_mut()
            .ok_or_else(|| io::Error::other("event log closed"))?;
        f(writer.as_mut())
    }
}

impl Write for ClosableLog {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.with(|writer| writer.write(bytes))
    }
    fn flush(&mut self) -> io::Result<()> {
        self.with(|writer| writer.flush())
    }
}

/// One drain step's view of the engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DrainState {
    pub humans: usize,
    pub unsettled_receipts: usize,
}

/// Why a draining worker exits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExitReason {
    Drained,
    DrainDeadline,
}

impl ExitReason {
    fn name(self) -> &'static str {
        match self {
            Self::Drained => "drained",
            Self::DrainDeadline => "drain_deadline",
        }
    }
}

/// Run a drain against `observe` until it says the worker may exit. Separate
/// from the engine so the timing rules are testable.
pub fn drain(
    policy: DrainPolicy,
    mut observe: impl FnMut() -> DrainState,
    mut sleep: impl FnMut(Duration),
    now: impl Fn() -> Instant,
) -> (ExitReason, DrainState) {
    let step = Duration::from_millis(100);
    let began = now();
    let reason = loop {
        let state = observe();
        if state.humans == 0 && state.unsettled_receipts == 0 {
            return (ExitReason::Drained, state);
        }
        if state.humans == 0 || now().duration_since(began) >= policy.drain {
            break if state.humans == 0 {
                ExitReason::Drained
            } else {
                ExitReason::DrainDeadline
            };
        }
        sleep(step);
    };
    // Rewards already reported still deserve Horizon's acknowledgement.
    let flushing = now();
    loop {
        let state = observe();
        if state.unsettled_receipts == 0 || now().duration_since(flushing) >= policy.receipt_flush {
            return (reason, state);
        }
        sleep(step);
    }
}

fn wall() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Wait for a signal, drain, close the journal and exit the process.
pub fn spawn_monitor<W: Write + Send + 'static>(
    engine: Arc<Mutex<Engine<W>>>,
    policy: DrainPolicy,
    log: ClosableLog,
) {
    std::thread::spawn(move || {
        while !requested() {
            std::thread::sleep(Duration::from_millis(100));
        }
        let observe = || match engine.lock() {
            Ok(engine) => DrainState {
                humans: engine.drain_humans(),
                unsettled_receipts: engine.unsettled_receipts(),
            },
            // A poisoned engine has nothing left to deliver.
            Err(_) => DrainState {
                humans: 0,
                unsettled_receipts: 0,
            },
        };
        if let Ok(mut engine) = engine.lock() {
            engine.begin_drain(wall());
        }
        let began = Instant::now();
        let start = observe();
        let _ = writeln!(
            io::stdout().lock(),
            "{}",
            serde_json::json!({"event":"worker_draining","drain_seconds":policy.drain.as_secs(),
                "receipt_flush_seconds":policy.receipt_flush.as_secs(),"humans":start.humans,
                "unsettled_receipts":start.unsettled_receipts})
        );
        let (reason, end) = drain(policy, observe, std::thread::sleep, Instant::now);
        let _ = writeln!(
            io::stdout().lock(),
            "{}",
            serde_json::json!({"event":"worker_exiting","reason":reason.name(),"humans":end.humans,
                "unsettled_receipts":end.unsettled_receipts,"elapsed_ms":began.elapsed().as_millis() as u64})
        );
        let _ = io::stdout().flush();
        log.close();
        std::process::exit(0);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    /// Replays a fixed sequence of states on a virtual clock.
    fn run(policy: DrainPolicy, states: &[DrainState]) -> (ExitReason, DrainState, Duration) {
        let base = Instant::now();
        let clock = Cell::new(Duration::ZERO);
        let index = Cell::new(0usize);
        let (reason, state) = drain(
            policy,
            || {
                let i = index.get().min(states.len() - 1);
                index.set(index.get() + 1);
                states[i]
            },
            |step| clock.set(clock.get() + step),
            || base + clock.get(),
        );
        (reason, state, clock.get())
    }

    fn state(humans: usize, unsettled_receipts: usize) -> DrainState {
        DrainState {
            humans,
            unsettled_receipts,
        }
    }

    #[test]
    fn an_empty_worker_exits_at_once() {
        let (reason, _, waited) = run(DrainPolicy::default(), &[state(0, 0)]);
        assert_eq!(reason, ExitReason::Drained);
        assert_eq!(waited, Duration::ZERO);
    }

    #[test]
    fn a_match_that_ends_inside_the_drain_window_exits_drained() {
        let policy = DrainPolicy {
            drain: Duration::from_secs(60),
            receipt_flush: Duration::from_secs(5),
        };
        let (reason, end, waited) = run(
            policy,
            &[state(2, 1), state(2, 1), state(0, 1), state(0, 0)],
        );
        assert_eq!(reason, ExitReason::Drained);
        assert_eq!(end, state(0, 0));
        assert!(waited < Duration::from_secs(1));
    }

    #[test]
    fn the_drain_deadline_still_flushes_receipts_within_its_bound() {
        let policy = DrainPolicy {
            drain: Duration::from_secs(1),
            receipt_flush: Duration::from_secs(2),
        };
        let (reason, end, waited) = run(policy, &[state(3, 4)]);
        assert_eq!(reason, ExitReason::DrainDeadline);
        assert_eq!(end, state(3, 4));
        // About one second of drain and two of flushing, never unbounded.
        assert!(waited >= Duration::from_secs(3) && waited <= Duration::from_millis(3200));

        let settled = DrainPolicy {
            drain: Duration::ZERO,
            receipt_flush: Duration::from_secs(5),
        };
        let (reason, end, waited) = run(settled, &[state(1, 2), state(1, 1), state(1, 0)]);
        assert_eq!(reason, ExitReason::DrainDeadline);
        assert_eq!(end.unsettled_receipts, 0);
        assert!(waited < Duration::from_secs(1));
    }
}
