//! A served worker can outlive the Horizon process that admitted its players.
//! Horizon names its own process on `subscribe_world` (`control_plane_rebind_v1`).
//! A new name means the previous Horizon, its DME transports and every
//! admission it made are gone. This worker then drops what belonged to them
//! and binds its world to the new Horizon's world ID, so the new subscription
//! is neither refused as a world mismatch nor fed frames that name the old world.
use crate::*;
use std::collections::BTreeSet;

pub const CAPABILITY: &str = "control_plane_rebind_v1";

/// Horizon sends 16 to 64 characters of `[A-Za-z0-9_-]`.
pub fn valid_control_plane(id: &str) -> bool {
    (16..=64).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

impl<W: Write> Engine<W> {
    /// Bind this worker to the subscribing Horizon process. The same process
    /// (a reconnect, a resume) changes nothing. Another process first releases
    /// every reservation the previous one made, drops the rewards and loadout
    /// selections only it could have acknowledged, and moves a finished
    /// match's terminal to `world_id`. Returns the summary to log, or None when
    /// nothing was held.
    pub fn bind_control_plane(
        &mut self,
        control_plane: &str,
        world_id: u32,
        now: Duration,
        wall: u64,
    ) -> Result<Option<Value>, &'static str> {
        if !valid_control_plane(control_plane) {
            return Err("invalid_control_plane");
        }
        if self.control_plane.as_deref() == Some(control_plane) {
            return Ok(None);
        }
        let live: BTreeSet<String> = self
            .peers
            .keys()
            .cloned()
            .chain(self.active.iter().map(|a| a.reservation.id.clone()))
            .collect();
        let results: Vec<String> = self
            .results_cohorts
            .values()
            .flat_map(|c| c.actors.keys().cloned())
            .collect();
        let rewards: Vec<&progression::RewardEvent> = self
            .reward_order
            .iter()
            .filter_map(|id| self.pending_rewards.get(id))
            .collect();
        let selections: Vec<&loadout_selection::CommittedLoadoutSelection> = self
            .active
            .iter()
            .chain(self.peers.values())
            .filter_map(|a| a.pending_loadout_selection.as_ref())
            .chain(self.retired_loadout_selections.values())
            .collect();
        let previous_worlds: BTreeSet<u32> = self
            .peers
            .values()
            .chain(self.active.iter())
            .map(|a| a.reservation.world)
            .chain(self.results_cohorts.values().map(|c| c.terminal.world_id))
            .chain(self.match_terminal.iter().map(|t| t.world_id))
            .collect();
        let held = !live.is_empty()
            || !self.results_cohorts.is_empty()
            || !rewards.is_empty()
            || !selections.is_empty()
            || self
                .match_terminal
                .as_ref()
                .is_some_and(|t| t.world_id != world_id || !t.reservation_ids.is_empty());
        if !held {
            self.record(
                json!({"op":"control_plane_bound","world_id":world_id,
                    "replaced":self.control_plane.is_some()}),
                wall,
            )?;
            self.control_plane = Some(control_plane.to_owned());
            return Ok(None);
        }
        let summary = json!({"world_id":world_id,"previous_world_ids":previous_worlds,
            "replaced":self.control_plane.is_some(),"released_reservations":live.len()+results.len(),
            "dropped_rewards":rewards.len(),"dropped_loadout_selections":selections.len(),
            "match_terminal":self.match_terminal.is_some()});
        // The journal keeps every dropped receipt for audit; nothing replays it.
        let mut record = summary.clone();
        record["op"] = json!("control_plane_rebound");
        record["reservation_ids"] = json!(live.iter().chain(&results).collect::<Vec<_>>());
        record["rewards"] = json!(rewards);
        record["loadout_selections"] = json!(selections);
        self.record(record, wall)?;

        self.expire_world_peers_with(now, wall, &live)?;
        for id in &results {
            self.retire_reservation_id(id);
        }
        self.results_cohorts.clear();
        self.pending_rewards.clear();
        self.reward_order.clear();
        self.retired_loadout_selections.clear();
        if let Some(terminal) = &mut self.match_terminal {
            terminal.world_id = world_id;
            terminal.reservation_ids.clear();
        }
        self.control_plane = Some(control_plane.to_owned());
        let mut event = summary;
        event["event"] = json!("control_plane_rebound");
        Ok(Some(event))
    }
}
