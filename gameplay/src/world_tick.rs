//! One monotonic world clock, driven by the process scheduler rather than clients.
use super::*;

pub const WORLD_TICK_INTERVAL: Duration = Duration::from_millis(50);
impl<W: Write> Engine<W> {
    pub fn enable_world_ticks(&mut self) {
        self.world_tick_enabled = true;
    }
    /// Advances at most once per cadence even when several transports are active.
    pub fn tick_world(&mut self, now: Duration, wall: u64, request_id: String) -> Response {
        if !self.world_tick_enabled
            || self.last_world_tick.is_some_and(|last| {
                now <= last
                    || now.as_millis() / WORLD_TICK_INTERVAL.as_millis()
                        == last.as_millis() / WORLD_TICK_INTERVAL.as_millis()
            })
        {
            return self.world_response(request_id, Vec::new());
        }
        self.last_world_tick = Some(now);
        let result = (|| {
            let mut messages = self.expire_world_peers(now, wall)?;
            messages.extend(self.tick_bots(now, wall)?);
            self.queue_team_wave_deadlines(now, wall)?;
            messages.extend(self.tick_bot_firearms(now, wall)?);
            self.tick_bot_healing(now, wall)?;
            messages.extend(self.tick_objectives(now, wall)?);
            messages.extend(self.tick_match_clock(now, wall)?);
            self.queue_projectile_publications(now)?;
            self.drain_shared(messages, wall)
        })();
        match result {
            Ok(messages) => self.world_response(request_id, messages),
            Err(code) => Response::error(request_id, code),
        }
    }
    /// Squad dead-row timers read the observer's team deadline even while
    /// that observer is alive. Publish each rollover once per recipient.
    pub(super) fn queue_team_wave_deadlines(
        &mut self,
        now: Duration,
        wall: u64,
    ) -> Result<(), &'static str> {
        if !self.match_started || self.match_terminal.is_some() {
            return Ok(());
        }
        let mut planned = Vec::new();
        for actor in self.peers.values().chain(self.active.iter()) {
            if !actor.ready_for_updates || !actor.entered_game || actor.initial_objects.is_none()
                // A queued spawn must consume its due wave before the HUD rolls.
                || actor.pending_spawn_deadline.is_some_and(|deadline| deadline <= now)
            {
                continue;
            }
            let team = actor.team();
            let Some(clock) = self.team_waves[usize::from(team)] else {
                continue;
            };
            let deadline = clock.next_after(now)?;
            if actor.published_wave_deadline == Some(deadline) {
                continue;
            }
            let packet = replication::TeamSpawnDeadline {
                sequence: 1,
                timestamp_ms: now.as_millis() as u32,
                object_index: 0,
                team,
                deadline_ms: deadline.as_millis() as u32,
            }
            .encode_with_profile(self.wire_profile)?;
            planned.push((actor.reservation.id.clone(), team, deadline, packet));
        }
        if planned.is_empty() {
            return Ok(());
        }
        self.record(
            json!({"op":"team_wave_deadlines_published",
            "timestamp_ms":now.as_millis() as u64,
            "recipients":planned.iter().map(|(id,team,deadline,_)| json!({
                "reservation_id":id,"team":team,"deadline_ms":deadline.as_millis() as u64
            })).collect::<Vec<_>>() }),
            wall,
        )?;
        for (id, _, deadline, packet) in planned {
            if let Some(actor) = self
                .peers
                .values_mut()
                .chain(self.active.iter_mut())
                .find(|actor| actor.reservation.id == id)
            {
                actor.queue_shared(packet);
                actor.published_wave_deadline = Some(deadline);
            }
        }
        Ok(())
    }
    fn world_response(&self, request_id: String, messages: Vec<OutboundMessage>) -> Response {
        Response {
            v: BRIDGE_VERSION,
            request_id,
            ok: true,
            error: None,
            messages,
            rewards: self.pending_reward_batch(),
            match_terminal: self.match_terminal.clone(),
        }
    }
    pub fn accepts_subscription_world(&self, world_id: u32) -> bool {
        self.peers
            .values()
            .chain(self.active.iter())
            .all(|a| a.reservation.world == world_id)
    }
}
