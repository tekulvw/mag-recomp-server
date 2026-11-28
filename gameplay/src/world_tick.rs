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
            use tick_timing::{Phase, timed};
            let mut messages = timed!(self, Phase::Peers, self.expire_world_peers(now, wall))?;
            timed!(self, Phase::Incapacitation, {
                self.tick_incapacitation(now, wall)?;
                self.tick_acquisition_ground_registration(wall)
            })?;
            // Bots move and plan against the structures standing now.
            timed!(
                self,
                Phase::Bots,
                self.refresh_bot_structure_obstacles(wall)
            )?;
            timed!(self, Phase::Perception, self.tick_bot_perception(now, wall))?;
            self.wait_bot_cover_builds();
            self.wait_bot_spawn_routes(wall)?;
            timed!(self, Phase::Cover, self.tick_bot_cover(now, wall))?;
            messages.extend(timed!(self, Phase::Bots, self.tick_bots(now, wall))?);
            timed!(self, Phase::Waves, {
                self.advance_strategic_player_waves(now, wall)?;
                self.queue_dead_humans_for_next_wave(now, wall)?;
                self.queue_team_wave_deadlines(now, wall)
            })?;
            messages.extend(timed!(
                self,
                Phase::Firearms,
                self.tick_bot_firearms(now, wall)
            )?);
            messages.extend(timed!(
                self,
                Phase::Healing,
                self.tick_bot_healing(now, wall)
            )?);
            messages.extend(timed!(
                self,
                Phase::Objectives,
                self.tick_objectives(now, wall)
            )?);
            timed!(self, Phase::Support, {
                self.tick_leadership_auto_repair(now, wall)?;
                self.tick_leadership_auto_heal(now, wall)?;
                self.queue_squad_orders(now, wall)?;
                self.queue_active_ability_state(now, wall)
            })?;
            messages.extend(timed!(self, Phase::Projectiles, {
                let mut rockets = self.tick_hospital_rockets(now, wall)?;
                rockets.extend(self.tick_infantry_rockets(now, wall)?);
                Ok::<_, &'static str>(rockets)
            })?);
            timed!(
                self,
                Phase::Projectiles,
                self.tick_guided_warnings(now, wall)
            )?;
            messages.extend(timed!(
                self,
                Phase::MatchClock,
                self.tick_match_clock(now, wall)
            )?);
            messages.extend(timed!(self, Phase::Projectiles, {
                let gas = self.tick_gas_grenades(now, wall)?;
                self.queue_projectile_publications(now)?;
                Ok::<_, &'static str>(gas)
            })?);
            timed!(
                self,
                Phase::Resupply,
                self.tick_human_static_resupply(now, wall)
            )?;
            let messages = timed!(self, Phase::Drain, self.drain_shared(messages, wall))?;
            timed!(
                self,
                Phase::Results,
                self.tick_results_cohorts(now, wall, messages)
            )
        })();
        match result {
            Ok(messages) => {
                let mut response = self.world_response(request_id, messages);
                // Only when due: a subscription began or a membership changed.
                response.voice_groups = self.next_voice_groups();
                response
            }
            Err(code) => Response::error(request_id, code),
        }
    }
    /// Record a failed world update before its subscriber is dropped, so the
    /// stream gap names its cause. Returns the same record for process logs.
    pub fn record_world_tick_failure(
        &mut self,
        world_id: u32,
        error: &str,
        now: Duration,
        wall: u64,
    ) -> Value {
        let mut failure = json!({"event":"world_tick_failed","error":error,"world_id":world_id,
            "timestamp_ms":now.as_millis() as u64,"round":self.round_number,
            "humans":self.active.iter().chain(self.peers.values()).count(),
            "bots":self.bot_registry.bots.len()});
        let mut record = failure.clone();
        record["op"] = json!("world_tick_failed");
        if let Err(code) = self.record(record, wall) {
            failure["record_error"] = json!(code);
        }
        failure
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
            // A queued character counts down to the wave it spawns on, which
            // the respawn queue's cutoff can put one past the team's next wave.
            let deadline = match actor.pending_spawn_deadline {
                Some(deadline) => deadline,
                None => clock.next_after(now)?,
            };
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
            messages,
            rewards: self.pending_reward_batch(),
            loadout_selections: self.pending_loadout_selections(),
            match_terminal: self.match_terminal.clone(),
            worker_draining: self.draining.then_some(true),
            ..Response::success(request_id)
        }
    }
    pub fn accepts_subscription_world(&self, world_id: u32) -> bool {
        self.peers
            .values()
            .chain(self.active.iter())
            .all(|a| a.reservation.world == world_id)
            && self
                .results_cohorts
                .values()
                .all(|c| c.terminal.world_id == world_id)
    }
}
