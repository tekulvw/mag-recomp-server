//! Authenticated Company strategics and their existing gameplay consumers.
use crate::{strategic_abilities, *};

impl<W: Write> Engine<W> {
    pub(crate) fn strategic_spawn_interval(
        &self,
        team: u8,
        base_ms: u32,
    ) -> Result<u32, &'static str> {
        if team > 1 || base_ms == 0 {
            return Err("invalid_strategic_spawn_interval");
        }
        let modifier = self
            .active_abilities
            .as_ref()
            .and_then(|runtime| runtime.strategic.as_ref())
            .map_or(0., |state| state.spawn_interval_modifier(team));
        let interval = strategic_abilities::spawn_interval_ms(base_ms, modifier);
        if interval == 0 {
            return Err("invalid_strategic_spawn_interval");
        }
        Ok(interval)
    }
}

impl<W: Write> Engine<W> {
    pub(crate) fn plan_strategic_ability(
        &self,
        actor: &Active,
        request: active_abilities::StrategicRequest,
        now: Duration,
    ) -> Result<
        (
            Option<active_ability_engine::Runtime>,
            serde_json::Value,
            Option<&'static str>,
        ),
        &'static str,
    > {
        use strategic_abilities::{StrategicAbility, StrategicStart};
        let denied = |reason| {
            Ok((
                None,
                serde_json::json!({"type":"strategic_ability","accepted":false,"reason":reason}),
                Some(reason),
            ))
        };
        let Some(mut next) = self.active_abilities.clone() else {
            return denied("strategic_ability_execution_pending");
        };
        if !self.match_started || self.match_terminal.is_some() {
            return denied("match_not_playing");
        }
        if self.match_profile().game_type != 2 {
            return denied("strategic_mode_unavailable");
        }
        if !actor.entered_game || !actor.ready_for_updates || actor.initial_objects.is_none() {
            return denied("strategic_actor_not_created");
        }
        let Some(team) = self.strategic_leadership_team(actor) else {
            return denied("strategic_requires_native_company_leader");
        };
        if matches!(actor.life, CharacterLife::AwaitingRespawn { .. }) {
            return denied("strategic_character_dead");
        }
        let Some(state) = next.strategic.as_mut() else {
            return denied("strategic_ability_execution_pending");
        };
        let ability = match StrategicAbility::from_code(request.ability_code) {
            Ok(value) => value,
            Err(reason) => return denied(reason),
        };
        let work = match state.activate(2, team, ability, now.as_millis() as u32) {
            Ok(work) => work,
            Err(reason) => return denied(reason),
        };
        if let StrategicStart::RefreshCommands { team } = work {
            next.refresh_command_team(team)?;
        }
        next.version += 1;
        Ok((
            Some(next),
            serde_json::json!({"type":"strategic_ability","accepted":true,
            "ability":ability.name(),"team":team,"activate_time_ms":now.as_millis() as u32}),
            None,
        ))
    }

    pub(crate) fn plan_strategic_wave_clocks(
        &self,
        runtime: &active_ability_engine::Runtime,
        _now: Duration,
    ) -> Result<[Option<TeamWaveClock>; 2], &'static str> {
        let mut clocks = self.team_waves;
        if self.match_profile().game_type != 2 {
            return Ok(clocks);
        }
        let base = self.match_profile().spawn_wave_ms;
        for (team, clock) in clocks.iter_mut().enumerate() {
            let modifier = runtime
                .strategic
                .as_ref()
                .map_or(0., |s| s.spawn_interval_modifier(team as u8));
            let interval = strategic_abilities::spawn_interval_ms(base[team], modifier);
            if interval == 0 {
                return Err("invalid_strategic_spawn_interval");
            }
            if let Some(current) = *clock {
                let period = Duration::from_millis(u64::from(interval));
                if period != current.period {
                    *clock = Some(current.with_renewal_period(period)?);
                }
            }
        }
        Ok(clocks)
    }

    pub(crate) fn stage_company_strategics(
        &mut self,
        runtime: &mut active_ability_engine::Runtime,
        now: Duration,
    ) -> Result<(), &'static str> {
        if self.match_profile().game_type != 2 {
            return Ok(());
        }
        let Some(current) = runtime.strategic.as_ref() else {
            return Ok(());
        };
        let (next, scans) = current.prepare_tick(now.as_millis() as u32)?;
        runtime.strategic = Some(next);
        self.team_waves = self.plan_strategic_wave_clocks(runtime, now)?;
        for scan in scans.into_iter().flatten() {
            for index in self.squad_members(scan.squad, None) {
                if index != u16::MAX
                    && let Some(identity) = self.current_detection_identity(index)
                {
                    runtime.mark_character(index, identity);
                }
            }
        }
        Ok(())
    }

    pub(crate) fn current_detection_identity(&self, index: u16) -> Option<String> {
        self.active
            .iter()
            .chain(self.peers.values())
            .find(|actor| actor.reservation.player == index && actor.initial_objects.is_some())
            .map(active_ability_engine::detection_identity)
            .or_else(|| {
                self.bot_combat_generations(index)
                    .next()
                    .map(|generation| format!("bot:{index}:{generation}"))
            })
    }

    /// The player manager renews due waves before Company expiry is processed.
    pub(crate) fn advance_strategic_player_waves(
        &mut self,
        now: Duration,
        wall: u64,
    ) -> Result<(), &'static str> {
        if !self.match_started
            || self.match_terminal.is_some()
            || self.match_profile().game_type != 2
            || !self
                .active_abilities
                .as_ref()
                .is_some_and(|runtime| runtime.strategic.is_some())
        {
            return Ok(());
        }
        let mut next = self.team_waves;
        let mut changed = false;
        for clock in next.iter_mut().flatten() {
            let previous = clock.first_deadline;
            clock.advance_native_cycle(now)?;
            changed |= previous != clock.first_deadline;
        }
        if changed {
            self.record(serde_json::json!({"op":"strategic_player_waves_renewed","timestamp_ms":now.as_millis(),
                "deadlines_ms":next.map(|clock|clock.map(|c|c.first_deadline.as_millis()))}),wall)?;
            self.team_waves = next;
        }
        Ok(())
    }
}
