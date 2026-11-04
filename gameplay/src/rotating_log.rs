//! Segmented append-only log files with time-based retention.
//!
//! Callers write one complete record and then flush, as the evidence recorder
//! and tick telemetry already do. Bytes are buffered until that flush, so a
//! rotation always happens between records and no record is split across
//! segments. Rotation opens the next segment only when the current one already
//! holds data and the pending record would push it past the segment size, so a
//! record larger than the segment size occupies a segment of its own.
//!
//! The first segment is the configured path itself; later segments append a
//! six-digit index (`events.jsonl.000001`, ...). Every segment is created
//! exclusively, so existing files are never overwritten. A closed segment is
//! removed once the retention duration has elapsed since it was closed; the
//! active segment is never removed. Record sequence numbers continue across
//! segments, so a gap left by retention is visible.
use std::{
    collections::VecDeque,
    fs::{File, OpenOptions},
    io::{self, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

/// Smallest accepted segment size; smaller values would rotate on almost every record.
pub const MINIMUM_SEGMENT_BYTES: u64 = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RotationPolicy {
    pub segment_bytes: u64,
    pub retention: Option<Duration>,
}

impl RotationPolicy {
    pub fn validate(self) -> Result<Self, &'static str> {
        if self.segment_bytes < MINIMUM_SEGMENT_BYTES {
            return Err("log_segment_bytes_too_small");
        }
        if self.retention.is_some_and(|r| r.is_zero()) {
            return Err("log_retention_must_be_positive");
        }
        Ok(self)
    }
}

pub struct RotatingLog {
    base: PathBuf,
    policy: RotationPolicy,
    clock: fn() -> SystemTime,
    file: File,
    index: u64,
    size: u64,
    pending: Vec<u8>,
    closed: VecDeque<(PathBuf, SystemTime)>,
}

fn create(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

impl RotatingLog {
    pub fn create(base: PathBuf, policy: RotationPolicy) -> io::Result<Self> {
        Self::with_clock(base, policy, SystemTime::now)
    }
    fn with_clock(
        base: PathBuf,
        policy: RotationPolicy,
        clock: fn() -> SystemTime,
    ) -> io::Result<Self> {
        let policy = policy.validate().map_err(io::Error::other)?;
        if base.file_name().is_none() {
            return Err(io::Error::other("log path needs a file name"));
        }
        let file = create(&base)?;
        Ok(Self {
            base,
            policy,
            clock,
            file,
            index: 0,
            size: 0,
            pending: Vec::new(),
            closed: VecDeque::new(),
        })
    }
    pub fn segment_path(&self, index: u64) -> PathBuf {
        if index == 0 {
            return self.base.clone();
        }
        let mut name = self.base.file_name().unwrap().to_os_string();
        name.push(format!(".{index:06}"));
        self.base.with_file_name(name)
    }
    fn rotate(&mut self) -> io::Result<()> {
        let next = create(&self.segment_path(self.index + 1))?;
        self.file.flush()?;
        let finished = std::mem::replace(&mut self.file, next);
        drop(finished);
        self.closed
            .push_back((self.segment_path(self.index), (self.clock)()));
        self.index += 1;
        self.size = 0;
        Ok(())
    }
    fn prune(&mut self) -> io::Result<()> {
        let Some(retention) = self.policy.retention else {
            return Ok(());
        };
        let now = (self.clock)();
        while let Some((path, closed_at)) = self.closed.front() {
            let expired = now
                .duration_since(*closed_at)
                .is_ok_and(|age| age >= retention);
            if !expired {
                break;
            }
            match std::fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            self.closed.pop_front();
        }
        Ok(())
    }
}

impl Write for RotatingLog {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.pending.extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        if !self.pending.is_empty() {
            let len = self.pending.len() as u64;
            if self.size > 0 && self.size.saturating_add(len) > self.policy.segment_bytes {
                self.rotate()?;
            }
            self.file.write_all(&self.pending)?;
            self.file.flush()?;
            self.size += len;
            self.pending.clear();
        }
        self.prune()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NOW_S: AtomicU64 = AtomicU64::new(1_000);
    fn clock() -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(NOW_S.load(Ordering::SeqCst))
    }
    fn dir(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("mag-rotating-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }
    fn record(log: &mut RotatingLog, byte: u8, len: usize) {
        let mut line = vec![byte; len - 1];
        line.push(b'\n');
        // Two partial writes, as write_all may issue, then one flush.
        log.write_all(&line[..len / 2]).unwrap();
        log.write_all(&line[len / 2..]).unwrap();
        log.flush().unwrap();
    }
    fn policy(retention: Option<u64>) -> RotationPolicy {
        RotationPolicy {
            segment_bytes: MINIMUM_SEGMENT_BYTES,
            retention: retention.map(Duration::from_secs),
        }
    }

    #[test]
    fn rotates_between_whole_records_and_never_overwrites() {
        let root = dir("rotate");
        let base = root.join("events.jsonl");
        let mut log = RotatingLog::with_clock(base.clone(), policy(None), clock).unwrap();
        let record_len = 40_000;
        for byte in *b"abc" {
            record(&mut log, byte, record_len);
        }
        let first = std::fs::read(&base).unwrap();
        let second = std::fs::read(log.segment_path(1)).unwrap();
        let third = std::fs::read(log.segment_path(2)).unwrap();
        assert_eq!(first.len(), record_len);
        assert!(first.iter().take(record_len - 1).all(|b| *b == b'a'));
        assert!(second.starts_with(b"bbbb") && second.ends_with(b"b\n"));
        assert!(third.starts_with(b"cccc"));
        assert_eq!(
            log.segment_path(1).file_name().unwrap(),
            "events.jsonl.000001"
        );
        // An oversized record still lands whole, in a segment of its own.
        record(&mut log, b'd', 70_000);
        assert_eq!(std::fs::read(log.segment_path(3)).unwrap().len(), 70_000);
        // A pre-existing next segment is never overwritten.
        std::fs::write(log.segment_path(4), b"keep").unwrap();
        log.write_all(b"e\n").unwrap();
        assert!(log.flush().is_err());
        assert_eq!(std::fs::read(log.segment_path(4)).unwrap(), b"keep");
        assert!(RotatingLog::with_clock(base, policy(None), clock).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn retention_removes_only_expired_closed_segments() {
        let root = dir("retain");
        let base = root.join("events.jsonl");
        NOW_S.store(1_000, Ordering::SeqCst);
        let mut log = RotatingLog::with_clock(base.clone(), policy(Some(600)), clock).unwrap();
        record(&mut log, b'a', 40_000);
        record(&mut log, b'b', 40_000); // closes segment 0 at t=1000
        NOW_S.store(1_300, Ordering::SeqCst);
        record(&mut log, b'c', 40_000); // closes segment 1 at t=1300
        NOW_S.store(1_599, Ordering::SeqCst);
        log.flush().unwrap();
        assert!(base.exists());
        NOW_S.store(1_600, Ordering::SeqCst);
        log.flush().unwrap();
        assert!(!base.exists(), "segment 0 expired");
        assert!(log.segment_path(1).exists());
        assert!(log.segment_path(2).exists(), "active segment is kept");
        NOW_S.store(100_000, Ordering::SeqCst);
        log.flush().unwrap();
        assert!(!log.segment_path(1).exists());
        assert!(
            log.segment_path(2).exists(),
            "active segment is never removed"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_unusable_policies() {
        assert!(
            RotationPolicy {
                segment_bytes: 1024,
                retention: None
            }
            .validate()
            .is_err()
        );
        assert!(policy(Some(0)).validate().is_err());
        assert!(policy(Some(1)).validate().is_ok());
    }
}
