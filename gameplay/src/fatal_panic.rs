//! A panic in a served worker ends the process. The world scheduler and the
//! bridge connections share one engine behind a mutex: a panic on any of
//! them poisons it, the world stops ticking and every later request fails,
//! yet the process and its listener stay up, so the pod still looks Running
//! and nothing restarts it. Exiting instead lets Kubernetes start a fresh
//! incarnation, which Medius writes off and adopts, releasing its players.
//!
//! Background jobs whose panics their owners already handle by joining them
//! (navigation cover and structure analysis) are the only exceptions.
use mag_gameplay::JOINED_JOB_THREADS;
use std::io::{self, Write};
use std::sync::{OnceLock, mpsc};
use std::time::Duration;

/// Rust's own exit status for a panicking `main`.
pub const PANIC_EXIT: i32 = 101;
/// Longest wait for the event log to finish its queued records.
const LOG_CLOSE_LIMIT: Duration = Duration::from_secs(2);

static LOG: OnceLock<crate::worker_shutdown::ClosableLog> = OnceLock::new();

/// Whether a panic on the thread named `thread` must end the process.
pub fn is_fatal(thread: Option<&str>) -> bool {
    !thread.is_some_and(|name| JOINED_JOB_THREADS.contains(&name))
}

/// Install the hook for a served worker. `log` is closed before exiting so
/// the records leading up to the panic reach storage.
pub fn install(log: crate::worker_shutdown::ClosableLog) {
    let _ = LOG.set(log);
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // The default hook prints the message and location to stderr.
        previous(info);
        let current = std::thread::current();
        let thread = current.name();
        if !is_fatal(thread) {
            return;
        }
        if std::env::var_os("RUST_BACKTRACE").is_none() {
            let _ = writeln!(
                io::stderr().lock(),
                "{}",
                std::backtrace::Backtrace::force_capture()
            );
        }
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()));
        let _ = writeln!(
            io::stdout().lock(),
            "{}",
            serde_json::json!({"event":"worker_panicked","thread":thread,
                "location":location,"exit_status":PANIC_EXIT})
        );
        let _ = io::stdout().flush();
        // The panicking thread may hold the log, or be its writer: close it
        // from a helper and give up after a bound rather than deadlock.
        if let Some(log) = LOG.get() {
            let (done, closed) = mpsc::channel();
            let log = log.clone();
            if std::thread::Builder::new()
                .name("panic-log-close".into())
                .spawn(move || {
                    log.close();
                    let _ = done.send(());
                })
                .is_ok()
            {
                let _ = closed.recv_timeout(LOG_CLOSE_LIMIT);
            }
        }
        std::process::exit(PANIC_EXIT);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn world_and_connection_panics_are_fatal_but_joined_jobs_are_not() {
        // The world scheduler and bridge connections are unnamed or named
        // anything but a joined job: all of them end the process.
        assert!(is_fatal(None));
        assert!(is_fatal(Some("world-scheduler")));
        assert!(is_fatal(Some("event-log")));
        for job in JOINED_JOB_THREADS {
            assert!(!is_fatal(Some(job)), "{job}");
        }
    }
}
