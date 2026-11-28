//! Opt-in wall time of each world-tick subsystem, for local diagnostics.
//!
//! Disabled engines never read the clock: every probe is one `Option` check.
use std::time::{Duration, Instant};

/// One timed part of a world tick. `Bots*` sub-phases are included in `Bots`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Idle and backlog expiry of recipients.
    Peers,
    /// Incapacitation timers and Acquisition ground registration.
    Incapacitation,
    Perception,
    /// Bot cover selection and peek cycles.
    Cover,
    /// The whole bot step, including the three sub-phases below.
    Bots,
    /// Population reconcile and per-recipient bot creation snapshots.
    BotsPopulation,
    /// Resupply planning and drop admission.
    BotsDrops,
    /// Objective goals and route planning.
    BotsObjectives,
    /// Medic approaches to downed teammates, inside `BotsObjectives`.
    BotsMedics,
    /// Strategic player waves and team spawn-wave deadlines.
    Waves,
    Firearms,
    Healing,
    Objectives,
    /// Leadership auto repair and heal, squad orders and ability state.
    Support,
    /// Rockets, guided warnings, gas and projectile publications.
    Projectiles,
    MatchClock,
    /// Human static resupply.
    Resupply,
    /// Shared reliable FIFO and notification drain to recipients.
    Drain,
    /// Results cohorts after the match ends.
    Results,
    /// Headless simulation only: waiting for the cover and spawn-route
    /// workers, which run off the tick live. Not part of any other phase.
    SimWorkerWait,
}

impl Phase {
    pub const ALL: [Phase; 20] = [
        Phase::Peers,
        Phase::Incapacitation,
        Phase::Perception,
        Phase::Cover,
        Phase::Bots,
        Phase::BotsPopulation,
        Phase::BotsDrops,
        Phase::BotsObjectives,
        Phase::BotsMedics,
        Phase::Waves,
        Phase::Firearms,
        Phase::Healing,
        Phase::Objectives,
        Phase::Support,
        Phase::Projectiles,
        Phase::MatchClock,
        Phase::Resupply,
        Phase::Drain,
        Phase::Results,
        Phase::SimWorkerWait,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Phase::Peers => "peers",
            Phase::Incapacitation => "incapacitation",
            Phase::Perception => "perception",
            Phase::Cover => "cover",
            Phase::Bots => "bots",
            Phase::BotsPopulation => "bots.population",
            Phase::BotsDrops => "bots.drops",
            Phase::BotsObjectives => "bots.objectives",
            Phase::BotsMedics => "bots.objectives.medics",
            Phase::Waves => "waves",
            Phase::Firearms => "firearms",
            Phase::Healing => "healing",
            Phase::Objectives => "objectives",
            Phase::Support => "support",
            Phase::Projectiles => "projectiles",
            Phase::MatchClock => "match_clock",
            Phase::Resupply => "resupply",
            Phase::Drain => "drain",
            Phase::Results => "results",
            Phase::SimWorkerWait => "sim.worker_wait",
        }
    }

    fn slot(self) -> usize {
        self as usize
    }
}

/// Accumulated time per phase since the last take.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TickTiming {
    nanos: [u64; Phase::ALL.len()],
}

impl TickTiming {
    pub fn add(&mut self, phase: Phase, elapsed: Duration) {
        let slot = &mut self.nanos[phase.slot()];
        *slot = slot.saturating_add(u64::try_from(elapsed.as_nanos()).unwrap_or(u64::MAX));
    }

    pub fn get(&self, phase: Phase) -> Duration {
        Duration::from_nanos(self.nanos[phase.slot()])
    }

    /// Every phase in `Phase::ALL` order, with its accumulated time.
    pub fn phases(&self) -> impl Iterator<Item = (Phase, Duration)> + '_ {
        Phase::ALL.into_iter().map(|phase| (phase, self.get(phase)))
    }
}

impl<W: std::io::Write> crate::Engine<W> {
    /// Record per-phase world-tick time from now on. Off by default.
    pub fn enable_tick_timing(&mut self) {
        self.tick_timing = Some(TickTiming::default());
    }

    /// The phases accumulated since the previous take; `None` while disabled.
    pub fn take_tick_timing(&mut self) -> Option<TickTiming> {
        self.tick_timing.as_mut().map(std::mem::take)
    }

    pub(crate) fn tick_timing_start(&self) -> Option<Instant> {
        self.tick_timing.is_some().then(Instant::now)
    }

    pub(crate) fn tick_timing_end(&mut self, phase: Phase, started: Option<Instant>) {
        if let (Some(timing), Some(started)) = (self.tick_timing.as_mut(), started) {
            timing.add(phase, started.elapsed());
        }
    }
}

/// Time `$body` as `$phase` when timing is enabled; yields `$body`'s value.
macro_rules! timed {
    ($engine:expr, $phase:expr, $body:expr) => {{
        let started = $engine.tick_timing_start();
        let value = $body;
        $engine.tick_timing_end($phase, started);
        value
    }};
}
pub(crate) use timed;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phases_have_unique_names_in_slot_order() {
        let names: std::collections::BTreeSet<_> = Phase::ALL.iter().map(|p| p.name()).collect();
        assert_eq!(names.len(), Phase::ALL.len());
        for (slot, phase) in Phase::ALL.iter().enumerate() {
            assert_eq!(phase.slot(), slot);
        }
    }

    #[test]
    fn time_accumulates_per_phase_and_saturates() {
        let mut timing = TickTiming::default();
        timing.add(Phase::Drain, Duration::from_micros(3));
        timing.add(Phase::Drain, Duration::from_micros(4));
        timing.add(Phase::Bots, Duration::MAX);
        timing.add(Phase::Bots, Duration::from_secs(1));
        assert_eq!(timing.get(Phase::Drain), Duration::from_micros(7));
        assert_eq!(timing.get(Phase::Bots), Duration::from_nanos(u64::MAX));
        assert_eq!(timing.get(Phase::Perception), Duration::ZERO);
        assert_eq!(timing.phases().count(), Phase::ALL.len());
    }
}
