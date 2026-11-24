//! Event-log writes that never block the engine on storage.
//!
//! The engine records every evidence row while it holds the world lock that
//! the 50 ms scheduler and every bridge request share. A synchronous `write`
//! that stalls on a busy disk therefore froze the world for seconds, long
//! enough for Horizon to time out events and end the world stream. This
//! writer moves storage to its own thread behind a byte-bounded queue.
//!
//! Callers keep the existing contract: write one complete record, then flush.
//! A flush hands the record to the queue and returns at once. When the queue
//! is full the record is dropped and counted instead of blocking; the
//! record's sequence number leaves a visible gap, and the writer thread
//! reports the dropped count on stderr once it catches up. A storage failure
//! is sticky: the next flush reports it, exactly as a direct writer would.
use std::{
    io::{self, Write},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc::{self, Sender},
    },
    thread::JoinHandle,
};

/// Queued bytes allowed before records are dropped (about four minutes of a
/// busy 64-bot match's evidence).
pub const DEFAULT_QUEUE_BYTES: usize = 32 * 1024 * 1024;

struct Shared {
    queued_bytes: AtomicUsize,
    dropped_records: AtomicU64,
    failed: AtomicBool,
}

pub struct AsyncLogWriter {
    pending: Vec<u8>,
    sender: Option<Sender<Vec<u8>>>,
    shared: Arc<Shared>,
    limit: usize,
    worker: Option<JoinHandle<()>>,
}

/// Reports why records were dropped; never includes record content.
pub type DropReporter = Arc<Mutex<dyn FnMut(u64, u64) + Send>>;

impl AsyncLogWriter {
    pub fn start<W: Write + Send + 'static>(writer: W) -> Self {
        Self::with_limit(writer, DEFAULT_QUEUE_BYTES, default_reporter())
    }

    pub fn with_limit<W: Write + Send + 'static>(
        mut writer: W,
        limit: usize,
        report: DropReporter,
    ) -> Self {
        let (sender, receiver) = mpsc::channel::<Vec<u8>>();
        let shared = Arc::new(Shared {
            queued_bytes: AtomicUsize::new(0),
            dropped_records: AtomicU64::new(0),
            failed: AtomicBool::new(false),
        });
        let worker_shared = Arc::clone(&shared);
        let worker = std::thread::Builder::new()
            .name("event-log".into())
            .spawn(move || {
                let mut reported = 0_u64;
                for record in receiver {
                    let len = record.len();
                    if !worker_shared.failed.load(Ordering::Acquire)
                        && writer
                            .write_all(&record)
                            .and_then(|_| writer.flush())
                            .is_err()
                    {
                        worker_shared.failed.store(true, Ordering::Release);
                    }
                    worker_shared.queued_bytes.fetch_sub(len, Ordering::AcqRel);
                    let dropped = worker_shared.dropped_records.load(Ordering::Acquire);
                    if dropped != reported {
                        if let Ok(mut report) = report.lock() {
                            report(dropped - reported, dropped);
                        }
                        reported = dropped;
                    }
                }
            })
            .expect("event-log writer thread");
        Self {
            pending: Vec::new(),
            sender: Some(sender),
            shared,
            limit,
            worker: Some(worker),
        }
    }

    pub fn dropped_records(&self) -> u64 {
        self.shared.dropped_records.load(Ordering::Acquire)
    }
}

fn default_reporter() -> DropReporter {
    Arc::new(Mutex::new(|dropped: u64, total: u64| {
        let _ = writeln!(
            io::stderr().lock(),
            "{}",
            serde_json::json!({"event":"event_log_records_dropped","dropped":dropped,"dropped_total":total})
        );
    }))
}

impl Write for AsyncLogWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.pending.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.shared.failed.load(Ordering::Acquire) {
            self.pending.clear();
            return Err(io::Error::other("event log writer failed"));
        }
        if self.pending.is_empty() {
            return Ok(());
        }
        let record = std::mem::take(&mut self.pending);
        let len = record.len();
        let admitted = self
            .shared
            .queued_bytes
            .try_update(Ordering::AcqRel, Ordering::Acquire, |queued| {
                (queued.saturating_add(len) <= self.limit).then_some(queued + len)
            })
            .is_ok();
        let sent = admitted
            && self
                .sender
                .as_ref()
                .is_some_and(|sender| sender.send(record).is_ok());
        if !sent {
            if admitted {
                self.shared.queued_bytes.fetch_sub(len, Ordering::AcqRel);
            }
            self.shared.dropped_records.fetch_add(1, Ordering::AcqRel);
        }
        Ok(())
    }
}

impl Drop for AsyncLogWriter {
    /// Finish queued records so a completed run's evidence is complete.
    fn drop(&mut self) {
        self.sender.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::Receiver;
    use std::time::Duration;

    /// Storage that blocks until the test releases it, like a stalled disk.
    struct GatedStorage {
        gate: Receiver<()>,
        written: Arc<Mutex<Vec<u8>>>,
        fail: bool,
    }
    impl Write for GatedStorage {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            let _ = self.gate.recv();
            if self.fail {
                return Err(io::Error::other("synthetic storage failure"));
            }
            self.written.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    type Reports = Arc<Mutex<Vec<(u64, u64)>>>;

    fn reporter() -> (DropReporter, Reports) {
        let reports = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&reports);
        let report: DropReporter = Arc::new(Mutex::new(move |dropped, total| {
            seen.lock().unwrap().push((dropped, total));
        }));
        (report, reports)
    }

    fn record(log: &mut AsyncLogWriter, text: &str) -> io::Result<()> {
        log.write_all(text.as_bytes())?;
        log.flush()
    }

    #[test]
    fn stalled_storage_never_blocks_the_caller_and_drops_are_counted() {
        let (release, gate) = mpsc::channel();
        let written = Arc::new(Mutex::new(Vec::new()));
        let storage = GatedStorage {
            gate,
            written: Arc::clone(&written),
            fail: false,
        };
        let (report, reports) = reporter();
        let mut log = AsyncLogWriter::with_limit(storage, 9, report);
        let began = std::time::Instant::now();
        // Both records fit the nine-byte queue while storage is stalled; the
        // rest are dropped and counted without waiting for storage.
        record(&mut log, "aaaa\n").unwrap();
        record(&mut log, "bbb\n").unwrap();
        for _ in 0..5 {
            record(&mut log, "cccc\n").unwrap();
        }
        assert!(began.elapsed() < Duration::from_secs(1));
        assert_eq!(log.dropped_records(), 5);
        for _ in 0..2 {
            release.send(()).unwrap();
        }
        drop(release);
        drop(log);
        assert_eq!(written.lock().unwrap().as_slice(), b"aaaa\nbbb\n");
        let reports = reports.lock().unwrap();
        assert_eq!(reports.iter().map(|(dropped, _)| dropped).sum::<u64>(), 5);
        assert_eq!(reports.last().unwrap().1, 5);
    }

    #[test]
    fn records_reach_storage_in_order_and_drop_waits_for_them() {
        let (release, gate) = mpsc::channel();
        let written = Arc::new(Mutex::new(Vec::new()));
        let storage = GatedStorage {
            gate,
            written: Arc::clone(&written),
            fail: false,
        };
        let (report, reports) = reporter();
        let mut log = AsyncLogWriter::with_limit(storage, 1024, report);
        for index in 0..10 {
            record(&mut log, &format!("{index}\n")).unwrap();
            release.send(()).unwrap();
        }
        drop(log);
        assert_eq!(
            String::from_utf8(written.lock().unwrap().clone()).unwrap(),
            (0..10).map(|i| format!("{i}\n")).collect::<String>()
        );
        assert!(reports.lock().unwrap().is_empty());
    }

    #[test]
    fn a_storage_failure_is_reported_by_a_later_flush() {
        let (release, gate) = mpsc::channel();
        let storage = GatedStorage {
            gate,
            written: Arc::new(Mutex::new(Vec::new())),
            fail: true,
        };
        let (report, _) = reporter();
        let mut log = AsyncLogWriter::with_limit(storage, 1024, report);
        record(&mut log, "first\n").unwrap();
        release.send(()).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut failed = false;
        while std::time::Instant::now() < deadline {
            if record(&mut log, "next\n").is_err() {
                failed = true;
                break;
            }
            let _ = release.send(());
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(failed, "a failed writer must surface on a later flush");
        drop(release);
    }
}
