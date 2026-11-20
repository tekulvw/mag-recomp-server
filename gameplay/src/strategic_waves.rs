//! Strategic changes affect future reinforcement cycles, not the current deadline.
use crate::*;

impl TeamWaveClock {
    pub(crate) fn with_renewal_period(self, period: Duration) -> Result<Self, &'static str> {
        if period.is_zero() {
            return Err("invalid_strategic_spawn_interval");
        }
        // Even an overdue timer belongs to the pending native manager cycle.
        Ok(Self { period, ..self })
    }

    /// Native manager renews once from the observed clock, without replaying missed waves.
    pub(crate) fn advance_native_cycle(&mut self, now: Duration) -> Result<(), &'static str> {
        if now >= self.first_deadline {
            self.first_deadline = now.checked_add(self.period).ok_or("wave_clock_overflow")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ms(value: u64) -> Duration {
        Duration::from_millis(value)
    }

    #[test]
    fn interval_change_preserves_current_wave_then_uses_new_period() {
        let clock = TeamWaveClock {
            first_deadline: ms(10_000),
            period: ms(20_000),
        };
        let mut slowed = clock.with_renewal_period(ms(30_000)).unwrap();
        assert_eq!(slowed.next_after(ms(5_000)).unwrap(), ms(10_000));
        assert_eq!(slowed.next_after(ms(10_000)).unwrap(), ms(40_000));
        slowed.advance_native_cycle(ms(10_000)).unwrap();
        let restored = slowed.with_renewal_period(ms(20_000)).unwrap();
        assert_eq!(restored.next_after(ms(25_000)).unwrap(), ms(40_000));
        assert_eq!(restored.next_after(ms(40_000)).unwrap(), ms(60_000));
    }

    #[test]
    fn strategic_order_between_deadline_and_world_tick_preserves_the_due_wave() {
        let clock = TeamWaveClock {
            first_deadline: ms(10_000),
            period: ms(20_000),
        };
        let mut changed = clock.with_renewal_period(ms(30_000)).unwrap();
        assert_eq!(changed.first_deadline, ms(10_000));
        changed.advance_native_cycle(ms(10_020)).unwrap();
        assert_eq!(changed.first_deadline, ms(40_020));
    }

    #[test]
    fn expiry_after_due_wave_does_not_retime_the_renewed_wave() {
        let mut clock = TeamWaveClock {
            first_deadline: ms(10_000),
            period: ms(30_000),
        };
        clock.advance_native_cycle(ms(10_000)).unwrap();
        assert_eq!(clock.first_deadline, ms(40_000));
        clock.advance_native_cycle(ms(40_020)).unwrap();
        let clock = clock.with_renewal_period(ms(20_000)).unwrap();
        assert_eq!(clock.next_after(ms(40_020)).unwrap(), ms(70_020));
        assert_eq!(clock.next_after(ms(70_020)).unwrap(), ms(90_020));
    }
}
