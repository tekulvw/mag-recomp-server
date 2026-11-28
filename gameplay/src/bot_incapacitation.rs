//! Downed registry bots (docs/incapacitation.md, "Downed bots").
//!
//! A bot goes down under the same rules as a human: an eligible lethal hit
//! below the incapacitated damage threshold, never a KineticResponse hit. The
//! body keeps health 0 and the dead bot phase, so it cannot move, fire, heal
//! or be targeted, but it is held out of its team's respawn wave. A human or
//! bot medic on its side can revive it; finishing damage, bleed-out or the
//! early give-up below make the death final, and the bot then joins the first
//! team wave after that moment.
//!
//! Bots never press Bleed out, so a downed bot gives up on its own when no
//! medic is close enough to help: once `medic_check_delay_ms` has passed, and
//! every `medic_recheck_ms` after that, it looks for a living friendly
//! character able to revive it within `medic_radius_m`. When there is none it
//! bleeds out at once. It never waits past the authored downed time.
use super::*;

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Policy {
    /// Registry bots go down under `--incapacitation-rules`. Without those
    /// rules every bot death stays terminal.
    pub enabled: bool,
    /// How long after going down a bot first looks for a medic.
    pub medic_check_delay_ms: u32,
    /// How often a bot that found a medic looks again.
    pub medic_recheck_ms: u32,
    /// How close (straight-line, metres) a medic must be for the bot to wait.
    pub medic_radius_m: f64,
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            enabled: true,
            medic_check_delay_ms: 10_000,
            medic_recheck_ms: 1_000,
            medic_radius_m: 30.,
        }
    }
}
impl Policy {
    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        if self.medic_recheck_ms < 100
            || !self.medic_radius_m.is_finite()
            || self.medic_radius_m < 0.
        {
            return Err("invalid_bot_incapacitation_policy");
        }
        Ok(())
    }
}

/// A downed bot life.
#[derive(Clone, Copy, Debug)]
pub(crate) struct BotDowned {
    pub(crate) state: incapacitation::Downed,
    pub(crate) at: Duration,
    pub(crate) next_medic_check: Duration,
    /// The body's physics state when it went down, restored on revival.
    pub(crate) motion: Option<bot_physics::Motion>,
    /// A check has found a medic in range during this downed life.
    pub(crate) medic_seen: bool,
}
impl BotDowned {
    pub(crate) fn new(
        state: incapacitation::Downed,
        at: Duration,
        motion: Option<bot_physics::Motion>,
        config: &Option<bot_registry::BotPopulation>,
    ) -> Self {
        let delay = config.as_ref().map_or_else(
            || Policy::default().medic_check_delay_ms,
            |c| c.incapacitation.medic_check_delay_ms,
        );
        Self {
            state,
            at,
            next_medic_check: at.saturating_add(Duration::from_millis(delay.into())),
            motion,
            medic_seen: false,
        }
    }
}

fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

impl<W: Write> Engine<W> {
    pub(crate) fn bot_incapacitation_policy(&self) -> Option<&Policy> {
        self.bot_registry
            .config
            .as_ref()
            .map(|c| &c.incapacitation)
            .filter(|p| p.enabled)
    }

    /// The bot counterpart of `plan_human_damage`: the damage model to publish
    /// and the bot's downed state after the hit. A living bot goes down under
    /// the human admission rule; a downed bot adds the hit to its finishing
    /// budget and keeps the attribution of the hit that downed it. Bots own no
    /// skills, so the downed time is the authored one.
    pub(crate) fn plan_bot_damage(
        &self,
        id: u16,
        amount: f32,
        mut damage: replication::CharacterDamage,
        now: Duration,
    ) -> Result<(replication::CharacterDamage, Option<incapacitation::Downed>), &'static str> {
        let Some(bot) = self.bot_registry.bots.get(&id) else {
            return Ok((damage, None));
        };
        let Some(current) = bot.downed else {
            if damage.health_quarter_points == 0
                && bot.health > 0
                && !crate::incapacitation_runtime::kinetic_response(damage)
                && self.wire_profile.application_id() == 20231
                && self.bot_incapacitation_policy().is_some()
                && let Some(rules) = &self.incapacitation_rules
                // Registry bots are always on foot in the ordinary state.
                && let Some(downed) = rules.admit(amount, 1, 0., now)?
            {
                damage.conditions |= 2;
                return Ok((damage, Some(downed)));
            }
            return Ok((damage, None));
        };
        let rules = self
            .incapacitation_rules
            .as_ref()
            .ok_or("incapacitation_rules_missing")?;
        let mut downed = current.state;
        let incoming_type = damage.last_location_and_type & 0xf0;
        let mut model = bot.damage;
        let expired = downed.is_expired(now);
        model.last_location_and_type =
            (if expired { 0 } else { incoming_type }) | (model.last_location_and_type & 15);
        let terminal = expired || downed.apply_damage(rules, amount)?;
        model.conditions = if terminal {
            model.conditions & !2
        } else {
            model.conditions | 2
        };
        Ok((model, (!terminal).then_some(downed)))
    }

    /// Revivals in bot `id`'s current life.
    pub(crate) fn bot_revivals(&self, id: u16) -> u32 {
        self.bot_registry.bots.get(&id).map_or(0, |b| b.revivals)
    }

    /// Whether `id` is a downed bot that a medic can still revive.
    pub(crate) fn bot_revivable(&self, id: u16, now: Duration) -> Option<u32> {
        let bot = self.bot_registry.bots.get(&id)?;
        let downed = bot.downed?;
        let runtime = bot.target.runtime?;
        (!downed.state.is_expired(now)).then_some(runtime.generation)
    }

    /// The damage model a revival publishes: `source_max_hp * modifier`,
    /// capped to the bot's (unmodified) maximum, with the downed condition
    /// cleared and the downing attribution kept, as a human revival does.
    pub(crate) fn plan_bot_revive_damage(
        &self,
        id: u16,
        source_max_hp: f32,
        modifier: f32,
    ) -> Result<Option<replication::CharacterDamage>, &'static str> {
        let Some(bot) = self.bot_registry.bots.get(&id) else {
            return Ok(None);
        };
        let health = incapacitation::resuscitated_health_quarter_points(
            source_max_hp,
            skill_catalog::OwnedModifiers::default().max_health_hp(),
            modifier,
        )?;
        if health == 0 {
            return Ok(None);
        }
        Ok(Some(replication::CharacterDamage {
            last_location_and_type: (7 << 4) | (bot.damage.last_location_and_type & 15),
            health_quarter_points: health,
            conditions: bot.damage.conditions & !2,
            ..bot.damage
        }))
    }

    /// Bring a downed bot back in the same life where it lies. The server
    /// owns the bot, so it also publishes the bot's recovery state (standing)
    /// with the restored damage model.
    pub(crate) fn commit_bot_revive(
        &mut self,
        id: u16,
        generation: u32,
        damage: replication::CharacterDamage,
        now: Duration,
    ) -> Result<(), &'static str> {
        let bot = self
            .bot_registry
            .bots
            .get_mut(&id)
            .ok_or("stale_revive_patient")?;
        let mut runtime = bot.target.runtime.ok_or("bot_runtime_missing")?;
        let Some(downed) = bot.downed else {
            return Err("stale_revive_patient");
        };
        if runtime.generation != generation
            || downed.state.is_expired(now)
            || damage.health_quarter_points == 0
            || damage.conditions & 2 != 0
        {
            return Err("stale_revive_patient");
        }
        bot.downed = None;
        bot.revivals = bot.revivals.saturating_add(1);
        bot.health = damage.health_quarter_points;
        bot.damage = damage;
        bot.stance = 0;
        bot.healing = Default::default();
        bot.received_healing = None;
        // Resume the objective task the bot had when it went down: its
        // destination stands, only the route is planned again.
        bot.task = bot.task_before_down.take().unwrap_or_default();
        bot.task.resume_after_supply(now);
        bot.move_attempted = false;
        runtime.phase = development_target::BotPhase::Complete;
        runtime.since = now;
        runtime.last_update = now;
        runtime.physics_motion = downed.motion.map(|motion| bot_physics::Motion {
            vertical_velocity: 0.,
            slide_velocity: [0.; 2],
            ..motion
        });
        bot.target.runtime = Some(runtime);
        bot.target.pending_move = None;
        let packet = replication::CharacterDamageUpdate {
            seated_exit: None,
            sequence: 2,
            timestamp_ms: now.as_millis() as u32,
            object_index: id,
            damage,
            forced_state: Some(replication::CharacterState {
                state: 1,
                stance: 0,
                user_data: [0; 3],
            }),
        }
        .encode_with_profile(self.wire_profile)?;
        self.firearm_assists.forget_victim(id);
        self.broadcast_bot_state_packet(id, packet);
        Ok(())
    }

    /// A living friendly character within the policy radius that can revive
    /// downed bot `id`: a human on foot carrying the medkit with a positive
    /// `kCharModResuscitatePercent`, or a bot medic carrying the medkit when
    /// bot revives are enabled. Returns a short description for the audit.
    pub(crate) fn bot_revive_medic_near(
        &self,
        id: u16,
        now: Duration,
    ) -> Result<Option<Value>, &'static str> {
        let (Some(policy), Some(bot)) = (
            self.bot_incapacitation_policy(),
            self.bot_registry.bots.get(&id),
        ) else {
            return Ok(None);
        };
        let Some(runtime) = bot.target.runtime else {
            return Ok(None);
        };
        let Some(team_item) = crate::bot_healing::team_item(&self.item_catalog) else {
            return Ok(None);
        };
        let me = runtime.physical_position();
        let team = bot.target.squad.team();
        let radius = policy.medic_radius_m;
        let leadership = self.plan_leadership(now)?;
        for a in self.active.iter().chain(self.peers.values()) {
            if !a.entered_game
                || !a.ready_for_updates
                || a.is_mounted()
                || a.team() != team
                || !matches!(a.life, CharacterLife::Alive { health_quarter_points, .. } if health_quarter_points > 0)
                || !self.accepts_subscription_world(a.reservation.world)
            {
                continue;
            }
            let Some(sample) = a.tracking.sample else {
                continue;
            };
            let feet = [
                f64::from(sample.position.x),
                f64::from(sample.position.y),
                f64::from(sample.position.z),
            ];
            let carries = a.equipped_loadout().is_some_and(|kit| {
                kit.inventories
                    .get(usize::from(kit.handle.inventory_index))
                    .is_some_and(|inventory| inventory.slots.contains(&Some(team_item.id)))
            });
            let d = distance(me, feet);
            if carries
                && d <= radius
                && leadership.modifiers(a).value("kCharModResuscitatePercent") > 0.
            {
                return Ok(Some(
                    json!({"kind":"human","player":a.reservation.player,"distance_m":d}),
                ));
            }
        }
        let bots_revive = self
            .bot_registry
            .config
            .as_ref()
            .is_some_and(|c| c.healing.enabled && c.healing.teammates && c.healing.revive);
        if !bots_revive {
            return Ok(None);
        }
        for (&other, medic) in &self.bot_registry.bots {
            let Some(r) = medic.target.runtime else {
                continue;
            };
            if other == id
                || medic.health == 0
                || medic.downed.is_some()
                || matches!(r.phase, development_target::BotPhase::Dead)
                || medic.target.squad.team() != team
                || crate::bot_healing::medkit_slot(
                    &self.item_catalog,
                    team_item,
                    &medic.target.loadout,
                )
                .is_none()
            {
                continue;
            }
            let d = distance(me, r.physical_position());
            if d <= radius {
                return Ok(Some(
                    json!({"kind":"bot","object_index":other,"distance_m":d}),
                ));
            }
        }
        Ok(None)
    }

    /// The world clock owns bot bleed-out: the authored downed time, or the
    /// early give-up when no medic is near enough to revive.
    pub(crate) fn tick_bot_incapacitation(
        &mut self,
        now: Duration,
        wall: u64,
    ) -> Result<(), &'static str> {
        let downed: Vec<_> = self
            .bot_registry
            .bots
            .iter()
            .filter_map(|(&id, bot)| bot.downed.map(|d| (id, d)))
            .collect();
        if downed.is_empty() {
            return Ok(());
        }
        let recheck = Duration::from_millis(
            self.bot_incapacitation_policy()
                .map_or(Policy::default().medic_recheck_ms, |p| p.medic_recheck_ms)
                .into(),
        );
        for (id, downed) in downed {
            let reason = if downed.state.is_expired(now) {
                Some("expired")
            } else if now >= downed.next_medic_check {
                match self.bot_revive_medic_near(id, now)? {
                    Some(medic) => {
                        // Audit the first check that finds a medic; later
                        // rechecks stay quiet unless the bot bleeds out.
                        if !downed.medic_seen {
                            self.record(
                                json!({"op":"bot_downed_waiting_for_medic","timestamp_ms":now.as_millis() as u64,
                                    "object_index":id,"medic":medic}),
                                wall,
                            )?;
                        }
                        if let Some(d) = self
                            .bot_registry
                            .bots
                            .get_mut(&id)
                            .and_then(|b| b.downed.as_mut())
                        {
                            d.next_medic_check = now.saturating_add(recheck);
                            d.medic_seen = true;
                        }
                        None
                    }
                    None => Some("no_medic_in_range"),
                }
            } else {
                None
            };
            if let Some(reason) = reason {
                self.bleed_out_bot(id, reason, now, wall)?;
            }
        }
        Ok(())
    }

    /// End a downed bot life: publish the downing attribution with the
    /// downed condition bits cleared (as a human expiry or give-up does) and
    /// start the death from now, so the bot joins its team's next wave.
    pub(crate) fn bleed_out_bot(
        &mut self,
        id: u16,
        reason: &'static str,
        now: Duration,
        wall: u64,
    ) -> Result<(), &'static str> {
        let bot = self.bot_registry.bots.get(&id).ok_or("bot_not_found")?;
        let downed = bot.downed.ok_or("stale_bot_bleed_out")?;
        let mut runtime = bot.target.runtime.ok_or("bot_runtime_missing")?;
        let damage = replication::CharacterDamage {
            conditions: bot.damage.conditions & !3,
            last_location_and_type: bot.damage.last_location_and_type & 15,
            ..bot.damage
        };
        let packet = replication::CharacterDamageUpdate {
            seated_exit: None,
            sequence: 2,
            timestamp_ms: now.as_millis() as u32,
            object_index: id,
            damage,
            forced_state: None,
        }
        .encode_with_profile(self.wire_profile)?;
        self.record(
            json!({"op":"bot_bled_out","timestamp_ms":now.as_millis() as u64,
                "object_index":id,"generation":runtime.generation,"reason":reason,
                "downed_ms":now.saturating_sub(downed.at).as_millis() as u64}),
            wall,
        )?;
        let bot = self.bot_registry.bots.get_mut(&id).unwrap();
        bot.downed = None;
        bot.task_before_down = None;
        bot.damage = damage;
        runtime.since = now;
        runtime.last_update = now;
        bot.target.runtime = Some(runtime);
        self.broadcast_bot_state_packet(id, packet);
        Ok(())
    }
}

#[cfg(test)]
#[path = "bot_incapacitation_tests.rs"]
mod tests;
