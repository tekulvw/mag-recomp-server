//! Vehicle-class spawn timestamps and ordered existing-vehicle queues.
//! Intake runs after the manager's due-wave processing. A vehicle that joins
//! when its death counter observes an advanced stamp waits for the next wave.
use crate::acquisition_ground_runtime::Key;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VehicleWaves {
    intervals_ms: [u32; 2],
    timestamps_ms: [u32; 2],
    queued: [Vec<Key>; 2],
}

impl VehicleWaves {
    /// The initial vehicle stamp is match network time plus the vehicle
    /// interval. The separate initial player-spawn delays are not used here.
    /// The admitted Acquisition profile has unmodified integer team intervals.
    pub fn start(match_epoch_ms: u32, intervals_ms: [u32; 2]) -> Result<Self, &'static str> {
        if intervals_ms.contains(&0) {
            return Err("invalid_acquisition_vehicle_spawn_interval");
        }
        Ok(Self {
            intervals_ms,
            timestamps_ms: intervals_ms.map(|interval| match_epoch_ms.wrapping_add(interval)),
            queued: Default::default(),
        })
    }

    pub fn next_timestamp(&self, team: u8) -> Result<u32, &'static str> {
        self.timestamps_ms
            .get(usize::from(team))
            .copied()
            .ok_or("invalid_acquisition_vehicle_spawn_team")
    }

    pub fn queue(&mut self, team: u8, key: Key) -> Result<(), &'static str> {
        let team = usize::from(team);
        if team >= self.queued.len() || key.index >= 32 || key.generation == 0 {
            return Err("invalid_acquisition_vehicle_spawn_queue_entry");
        }
        for (owner, entries) in self.queued.iter().enumerate() {
            if let Some(existing) = entries.iter().find(|entry| entry.index == key.index) {
                return if owner == team && *existing == key {
                    Ok(())
                } else {
                    Err("conflicting_acquisition_vehicle_spawn_queue_entry")
                };
            }
        }
        self.queued[team].push(key);
        Ok(())
    }

    /// One manager update. Equality is due and a late tick publishes one
    /// advancement from current network time, without replaying missed waves.
    /// The caller retries source-controlled spawn denials by requeueing.
    pub fn tick(&mut self, now_ms: u32) -> [Option<Vec<Key>>; 2] {
        std::array::from_fn(|team| {
            (self.timestamps_ms[team] <= now_ms).then(|| {
                self.timestamps_ms[team] = now_ms.wrapping_add(self.intervals_ms[team]);
                std::mem::take(&mut self.queued[team])
            })
        })
    }
}
