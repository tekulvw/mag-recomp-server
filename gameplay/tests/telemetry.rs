// SYNTHETIC: all inputs in this suite are authored in code; no game catalogs or captures.
use mag_gameplay::telemetry::{TickRecorder, TickSample};
use std::{
    io::{self, Write},
    sync::{Arc, Condvar, Mutex, mpsc},
    time::Duration,
};
struct SlowWriter(Arc<(Mutex<bool>, Condvar)>);
impl Write for SlowWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let (lock, ready) = &*self.0;
        let mut released = lock.lock().unwrap();
        while !*released {
            released = ready.wait(released).unwrap();
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
#[test]
fn a_stalled_telemetry_sink_cannot_block_gameplay_or_grow_its_queue() {
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let recorder = TickRecorder::start(SlowWriter(gate.clone()), 1_000_000);
    let producer = recorder.clone();
    let (done, completed) = mpsc::channel();
    std::thread::spawn(move || {
        for _ in 0..10_000 {
            producer.record(TickSample::new(true, 100, 5, 10, true, Some(20)));
        }
        done.send(()).unwrap();
    });
    let finished = completed.recv_timeout(Duration::from_secs(2)).is_ok();
    *gate.0.lock().unwrap() = true;
    gate.1.notify_all();
    assert!(finished, "telemetry enqueue blocked on stalled writer");
    assert!(
        recorder.dropped_samples() > 0,
        "bounded queue did not report loss"
    );
}
