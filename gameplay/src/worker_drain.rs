//! Shutdown of a served worker (SIGTERM, for example a Kubernetes rollout).
//! A draining worker admits no one and tells subscribers that asked
//! (`worker_drain_v1`), so Horizon stops placing players on it. The running
//! match and its result delivery continue; the process decides when to exit.
use crate::*;

pub const CAPABILITY: &str = "worker_drain_v1";
/// Refusal for every new admission once draining.
pub const REFUSAL: &str = "worker_draining";

impl<W: Write> Engine<W> {
    /// Stop admitting. Idempotent; returns whether this call started the drain.
    /// The journal record is best effort: a failed recorder must not keep the
    /// worker admitting.
    pub fn begin_drain(&mut self, wall: u64) -> bool {
        if self.draining {
            return false;
        }
        self.draining = true;
        let humans = self.drain_humans();
        let _ = self.record(
            json!({"op":"worker_draining","humans":humans,"pending_rewards":self.pending_rewards.len()}),
            wall,
        );
        true
    }

    pub fn is_draining(&self) -> bool {
        self.draining
    }

    /// Humans holding a seat in the live match or a results cohort.
    pub fn drain_humans(&self) -> usize {
        self.peers.len()
            + usize::from(self.active.is_some())
            + self
                .results_cohorts
                .values()
                .map(|c| c.actors.len())
                .sum::<usize>()
    }

    /// Rewards and loadout selections still waiting for Horizon's acknowledgement.
    pub fn unsettled_receipts(&self) -> usize {
        self.pending_rewards.len() + self.pending_loadout_selections().len()
    }

    /// Nothing left that a clean exit would cut short.
    pub fn drained(&self) -> bool {
        self.drain_humans() == 0 && self.unsettled_receipts() == 0
    }

    pub(crate) fn refuses_while_draining(&self, request: &Request) -> bool {
        self.draining
            && matches!(
                request,
                Request::Reserve { .. }
                    | Request::ReserveParty { .. }
                    | Request::SelectMatchProfile { .. }
            )
    }
}
