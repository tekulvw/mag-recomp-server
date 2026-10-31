//! Shared elapsed-time score clock; authored rate and team live in the disk catalog.
use std::time::Duration;

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Clock {
    epoch: Option<Duration>,
    last: Duration,
    running_elapsed: Duration,
    paused: bool,
}
impl Clock {
    pub(crate) fn advance(
        self,
        epoch: Duration,
        now: Duration,
        paused: bool,
        interval_ms: u32,
    ) -> Self {
        let mut next = if self.epoch == Some(epoch) {
            self
        } else {
            Self {
                epoch: Some(epoch),
                last: epoch,
                ..Self::default()
            }
        };
        if !next.paused {
            next.running_elapsed = next
                .running_elapsed
                .saturating_add(now.saturating_sub(next.last));
        }
        if paused && !next.paused {
            // Native disabling clears the pending deadline; resume starts a full interval.
            let interval = u128::from(interval_ms);
            let elapsed_ms = next.running_elapsed.as_millis() / interval * interval;
            next.running_elapsed = Duration::from_millis(elapsed_ms as u64);
        }
        next.last = now;
        next.paused = paused;
        next
    }
    pub(crate) fn points(self, initial: i16, interval_ms: u32, amount: i16) -> i16 {
        let ticks = self.running_elapsed.as_millis() / u128::from(interval_ms);
        let lost = ticks.saturating_mul(amount as u128);
        (initial as u128).saturating_sub(lost) as i16
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_clock_coalesces_elapsed_time_and_excludes_paused_intervals() {
        let epoch = Duration::from_secs(10);
        let clock = Clock::default().advance(epoch, Duration::from_secs(15), false, 1000);
        assert_eq!(clock.points(1200, 1000, 1), 1195);
        let clock = clock.advance(epoch, Duration::from_millis(17500), true, 1000);
        assert_eq!(clock.points(1200, 1000, 1), 1193);
        let clock = clock.advance(epoch, Duration::from_secs(27), false, 1000);
        assert_eq!(clock.points(1200, 1000, 1), 1193);
        let clock = clock.advance(epoch, Duration::from_millis(27500), false, 1000);
        assert_eq!(clock.points(1200, 1000, 1), 1193);
        let clock = clock.advance(epoch, Duration::from_secs(30), false, 1000);
        assert_eq!(clock.points(1200, 1000, 1), 1190);
        assert_eq!(
            clock
                .advance(epoch, Duration::from_secs(5000), false, 1000)
                .points(1200, 1000, 1),
            0
        );
    }
}
