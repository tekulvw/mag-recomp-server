//! World-clock half of the respawn queue: a character that already lived
//! rejoins the next team wave as soon as it is dead, without waiting for its
//! owner's next update.
use super::*;

/// The respawn queue's wave cutoff: a dead character that rejoins the queue
/// by itself less than this long before its team's wave goes to the
/// following wave, unless its client confirms in time for the earlier one.
///
/// Retail has no such constant. Its server queues a dead character only when
/// the client confirms, and the original client confirms by itself only once
/// its respawn screen opens: `fRespawnTime` (2.5 s) after the death, a few
/// frames later and once loading is complete. A death within about 3 s of a
/// wave therefore missed it, and a confirmation made any time before a wave
/// took that wave. The automatic requeue here does not wait for that
/// confirmation, so it keeps the same distance itself: `fRespawnTime` plus 1 s
/// for the confirmation's frames, delivery and the world tick, which also
/// leaves the respawn screen's whole-second timer above zero when it opens.
/// A confirmation still takes the next wave (see `confirm_respawn`).
///
/// A life sent before the respawn screen opens reaches a client still playing
/// the death, and that client never leaves the state the new life starts in:
/// a parachute drop falls without a parachute, the kit is not rebuilt (no
/// new magazines), the client stops reporting its state and confirming its
/// respawns, and every later life of that connection starts the same way.
pub(crate) const RESPAWN_QUEUE_CUTOFF: Duration = Duration::from_millis(3500);

impl TeamWaveClock {
    /// The wave a dead character that enters the queue at `entry` by itself
    /// (after any death: a kill, a self kill, a fall, an admin kill or a
    /// bleed-out) spawns on: the first one more than the cutoff later.
    pub(crate) fn wave_after_death(self, entry: Duration) -> Result<Duration, &'static str> {
        self.next_after(
            entry
                .checked_add(RESPAWN_QUEUE_CUTOFF)
                .ok_or("wave_clock_overflow")?,
        )
    }
}

impl<W: Write> Engine<W> {
    /// Whether a dead character, after its first life, joins the next wave by
    /// itself. A character that cancelled after this death stays out until it
    /// confirms again, and nothing joins once the match has ended.
    pub(super) fn rejoins_respawn_queue(&self, actor: &Active) -> bool {
        matches!(actor.life, CharacterLife::AwaitingRespawn { .. })
            && !actor.respawn_held
            && self.match_started
            && self.match_terminal.is_none()
            && !(self.maximum_players == 1
                && actor.replication_history.len() + 2 > MAX_REPLICATION_HISTORY)
            && update_actions::respawn_point(self, actor).is_some()
    }

    /// Queue every dead human that rejoins by itself for its team's first wave
    /// past the cutoff, at the world tick that first sees it dead. A dead client sends owner
    /// updates only about once a second, so waiting for one let a wave that
    /// fell between the death (or bleed-out) and that update go by unused.
    /// The owner's next update spawns it once the wave is due.
    pub(crate) fn queue_dead_humans_for_next_wave(
        &mut self,
        now: Duration,
        wall: u64,
    ) -> Result<(), &'static str> {
        if !self.match_started || self.match_terminal.is_some() {
            return Ok(());
        }
        let mut clocks = self.team_waves;
        let mut started = [false; 2];
        let mut queued = Vec::new();
        for actor in self.peers.values().chain(self.active.iter()) {
            if !actor.ready_for_updates
                || !actor.entered_game
                || actor.initial_objects.is_none()
                || actor.pending_spawn_deadline.is_some()
                || !self.actor_matches_team(actor)
                || !self.rejoins_respawn_queue(actor)
            {
                continue;
            }
            let team = actor.team();
            let slot = usize::from(team);
            let clock = match clocks[slot] {
                Some(clock) => clock,
                None => {
                    let clock = control::team_wave_clock(self, team, now)?;
                    clocks[slot] = Some(clock);
                    started[slot] = true;
                    clock
                }
            };
            queued.push((
                actor.reservation.id.clone(),
                team,
                clock.wave_after_death(now)?,
            ));
        }
        if queued.is_empty() {
            return Ok(());
        }
        self.record(
            json!({"op":"automatic_respawn_queued","timestamp_ms":now.as_millis() as u64,
            "characters":queued.iter().map(|(id,team,deadline)| json!({
                "reservation_id":id,"team":team,"deadline_ms":deadline.as_millis() as u64
            })).collect::<Vec<_>>()}),
            wall,
        )?;
        for (slot, clock) in clocks.into_iter().enumerate() {
            if started[slot] {
                self.team_waves[slot] = clock;
            }
        }
        for (id, _, deadline) in queued {
            if let Some(actor) = self
                .peers
                .values_mut()
                .chain(self.active.iter_mut())
                .find(|actor| actor.reservation.id == id)
            {
                actor.pending_spawn_deadline = Some(deadline);
            }
        }
        Ok(())
    }
}
