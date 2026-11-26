//! World-clock half of the respawn queue: a character that already lived
//! rejoins the next team wave as soon as it is dead, without waiting for its
//! owner's next update.
use super::*;

/// How long the original client keeps a character that bled out on its own
/// before it opens the respawn screen and confirms by itself: the retail
/// character gameplay setting `fRespawnTime` (see docs/incapacitation.md).
pub(crate) const BLEED_OUT_RESPAWN_SCREEN_DELAY: Duration = Duration::from_millis(2500);

impl Active {
    /// The time from which this dead character's next wave is chosen: now, or
    /// later while its client is still counting down to its respawn screen.
    /// A life sent before that screen opens reaches a client that is still
    /// playing its bleed-out, which then never leaves the state the new life
    /// starts in (a parachute drop never lands, and the character can no
    /// longer enter a vehicle).
    pub(crate) fn respawn_queue_floor(&self, now: Duration) -> Duration {
        self.respawn_screen_ready_at
            .map_or(now, |ready| ready.max(now))
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

    /// Queue every dead human that rejoins by itself for its team's next wave,
    /// at the world tick that first sees it dead. A dead client sends owner
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
                clock.next_after(actor.respawn_queue_floor(now))?,
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
