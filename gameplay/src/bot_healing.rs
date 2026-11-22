//! World-owned self-healing. AI admission is policy; all healing amounts and
//! timing use the same extracted item catalog and effect as human healing.
use super::*;

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Policy {
    pub enabled: bool,
    pub health_threshold_per_mille: u16,
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            enabled: true,
            health_threshold_per_mille: 750,
        }
    }
}
impl Policy {
    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        if self.health_threshold_per_mille > 1000 {
            return Err("invalid_bot_healing_policy");
        }
        Ok(())
    }
}
#[derive(Clone, Default)]
pub(super) struct State {
    generation: Option<u32>,
    equipment: Option<(u8, u8, u8, u16)>,
    effect: Option<healing::HealingEffect>,
    next_allowed: Duration,
}
/// A normal human Medkit treatment bound to the bot's current allocation and life.
#[derive(Clone, Debug)]
pub(super) struct Received {
    index: u16,
    allocation: u32,
    world: u32,
    round: u64,
    team: u8,
    healer_allocation: u64,
    session: progression::HealingSession,
}

impl Received {
    fn matches(&self, bot: &bot_registry::WorldBot) -> bool {
        bot.allocation_generation == self.allocation
            && bot.health > 0
            && bot.target.squad.team() == self.team
            && bot.target.runtime.is_some_and(|runtime| {
                runtime.generation == self.session.generation
                    && !matches!(runtime.phase, development_target::BotPhase::Dead)
            })
    }
}

impl<W: Write> Engine<W> {
    pub(super) fn prepare_received_bot_healing(
        &self,
        healer: &Active,
        target: u32,
        item: healing::HealItem,
        sequence: u64,
        now: Duration,
        healer_modifiers: &skill_catalog::OwnedModifiers,
    ) -> Result<Option<Received>, &'static str> {
        let index = target as u16;
        if !self.match_started
            || self.match_terminal.is_some()
            || !self.round_combat_allowed(now)
            || self.wire_profile.application_id() != 20231
            || target != (0x0380_0000 | u32::from(index))
        {
            return Ok(None);
        }
        let Some(bot) = self.bot_registry.bots.get(&index) else {
            return Ok(None);
        };
        let Some(runtime) = bot.target.runtime else {
            return Ok(None);
        };
        let recipient_modifiers = skill_catalog::OwnedModifiers::default();
        if !self.item_catalog.retail_combat().is_ok_and(|combat| {
            combat
                .healing_for_schedule(item.item_id())
                .is_ok_and(|profile| {
                    profile.definition["group"] == "active_items"
                        && profile.definition["name"] == "medkit"
                })
        }) || bot.target.squad.team() != healer.team()
            || matches!(runtime.phase, development_target::BotPhase::Dead)
            || bot.health == 0
            || bot.health >= recipient_modifiers.max_health_quarter_points()?
            || (bot.healing.generation == Some(runtime.generation) && bot.healing.effect.is_some())
            || bot
                .received_healing
                .as_ref()
                .is_some_and(|session| session.matches(bot))
        {
            return Ok(None);
        }
        let effect = healing::HealingEffect::new_for_recipient(
            item,
            runtime.generation,
            now,
            &self.item_catalog,
            healer_modifiers,
            &recipient_modifiers,
        )?;
        Ok(Some(Received {
            index,
            allocation: bot.allocation_generation,
            world: healer.reservation.world,
            round: self.round_number,
            team: healer.team(),
            healer_allocation: healer.allocation_sequence,
            session: progression::HealingSession {
                effect,
                healer: progression::RewardRecipient::from_active(healer),
                effect_id: format!(
                    "heal-{}-{sequence:016x}-bot-{index:04x}-{:08x}-{:08x}",
                    healer.reservation.id, bot.allocation_generation, runtime.generation,
                ),
            },
        }))
    }

    pub(super) fn install_received_bot_healing(&mut self, received: Received) {
        if let Some(bot) = self.bot_registry.bots.get_mut(&received.index)
            && received.matches(bot)
        {
            bot.received_healing = Some(received);
        }
    }

    fn tick_received_bot_healing(&mut self, now: Duration, wall: u64) -> Result<(), &'static str> {
        let current_world = self
            .active
            .iter()
            .chain(self.peers.values())
            .next()
            .map(|actor| actor.reservation.world);
        let ids: Vec<_> = self
            .bot_registry
            .bots
            .iter()
            .filter_map(|(&index, bot)| bot.received_healing.as_ref().map(|_| index))
            .collect();
        for index in ids {
            let bot = &self.bot_registry.bots[&index];
            let received = bot.received_healing.as_ref().unwrap();
            if current_world != Some(received.world)
                || self.round_number != received.round
                || self.match_terminal.is_some()
                || !self.round_combat_allowed(now)
                || !received.matches(bot)
            {
                self.bot_registry
                    .bots
                    .get_mut(&index)
                    .unwrap()
                    .received_healing = None;
                continue;
            }
            let before = bot.health;
            let maximum = skill_catalog::OwnedModifiers::default().max_health_quarter_points()?;
            let Some((damage, remaining, ticks)) =
                received.session.advance_with_receipts(now, bot.damage)
            else {
                continue;
            };
            let mut rewards = Vec::new();
            if let Some(healer) = &received.session.healer
                && !self.attacker_is_friendly(
                    bot.damage.last_attacker,
                    received.world,
                    received.team,
                )
            {
                let current_healer = self.active.iter().chain(self.peers.values()).find(|actor| {
                    actor.allocation_sequence == received.healer_allocation
                        && healer.matches_active(actor)
                });
                for (tick, units) in &ticks {
                    if *units == 0 {
                        continue;
                    }
                    let reward = healer.event(
                        format!("{}-{tick:02x}", received.session.effect_id),
                        progression::RewardKind::TeammateHealed,
                        *units,
                    );
                    let reward = if let Some(actor) = current_healer {
                        self.frago_action_reward(
                            reward,
                            actor,
                            bot.target
                                .runtime
                                .map(|runtime| frago::point(runtime.position)),
                            frago::Frame::default(),
                        )
                    } else {
                        reward
                    };
                    rewards.push(reward);
                }
            }
            let mut next = received.clone();
            let generation = next.session.generation;
            let packet = (!ticks.is_empty())
                .then(|| {
                    replication::CharacterDamageUpdate {
                        sequence: 2,
                        timestamp_ms: now.as_millis() as u32,
                        object_index: index,
                        damage,
                        forced_state: None,
                    }
                    .encode_with_profile(self.wire_profile)
                })
                .transpose()?;
            self.check_reward_capacity(&rewards)?;
            self.record(json!({
                "op":"received_bot_healing_tick","object_index":index,"generation":generation,
                "allocation_generation":next.allocation,"world_id":next.world,"round":next.round,
                "timestamp_ms":now.as_millis() as u64,"health_before":before,
                "health_after":damage.health_quarter_points,"restored_hp_per_tick":ticks,
                "completed":remaining.is_none(),"rewards":rewards,
            }), wall)?;
            if let Some(packet) = packet {
                self.commit_bot_healing(index, generation, before, damage, maximum)?;
                self.broadcast_known_bot_heal(index, packet);
            }
            self.commit_rewards(rewards);
            self.bot_registry
                .bots
                .get_mut(&index)
                .unwrap()
                .received_healing = remaining.map(|effect| {
                next.session.effect = effect;
                next
            });
        }
        Ok(())
    }

    /// Healing alone admits HP increases. Keep the damage-only guard strict.
    pub(crate) fn commit_bot_healing(
        &mut self,
        id: u16,
        generation: u32,
        expected_health: u16,
        resolved: replication::CharacterDamage,
        maximum_health: u16,
    ) -> Result<(), &'static str> {
        let bot = self
            .bot_registry
            .bots
            .get_mut(&id)
            .ok_or("stale_bot_healing")?;
        if bot.target.runtime.is_none_or(|r| {
            r.generation != generation || matches!(r.phase, development_target::BotPhase::Dead)
        }) || bot.health == 0
            || bot.health != expected_health
            || resolved.health_quarter_points < bot.health
            || resolved.health_quarter_points > maximum_health
        {
            return Err("stale_bot_healing");
        }
        bot.health = resolved.health_quarter_points;
        bot.damage = resolved;
        Ok(())
    }
    pub(crate) fn tick_bot_healing(
        &mut self,
        now: Duration,
        wall: u64,
    ) -> Result<(), &'static str> {
        self.tick_received_bot_healing(now, wall)?;
        let policy = self.bot_registry.config.as_ref().map(|c| c.healing.clone());
        // Registry bots own no skills/account. Use the same unmodified 100 HP
        // character rules they use for spawn and damage, without human identity.
        let modifiers = skill_catalog::OwnedModifiers::default();
        let maximum = modifiers.max_health_quarter_points()?;
        let enabled = policy.as_ref().is_some_and(|p| p.enabled)
            && self.match_started
            && self.match_terminal.is_none()
            && self.wire_profile.application_id() == 20231
            && self
                .active
                .iter()
                .chain(self.peers.values())
                .any(|a| a.ready_for_updates && a.initial_objects.is_some());
        let ids: Vec<_> = self.bot_registry.bots.keys().copied().collect();
        for id in ids {
            let bot = &self.bot_registry.bots[&id];
            let runtime = bot.target.runtime.ok_or("bot_runtime_missing")?;
            let mut state = bot.healing.clone();
            if bot.health == 0 || matches!(runtime.phase, development_target::BotPhase::Dead) {
                self.bot_registry.bots.get_mut(&id).unwrap().healing = State::default();
                continue;
            }
            if state.generation != Some(runtime.generation) {
                state = State {
                    generation: Some(runtime.generation),
                    ..Default::default()
                };
            }
            let kit = &bot.target.loadout;
            let item = kit
                .inventories
                .get(usize::from(kit.handle.inventory_index))
                .and_then(|inventory| {
                    inventory.slots.iter().enumerate().find_map(|(slot, _)| {
                        healing::HealItem::from_loadout_with_catalog(
                            kit,
                            slot as u8,
                            &self.item_catalog,
                        )
                        .ok()
                        .map(|item| (slot as u8, item))
                    })
                });
            let equipment = item.map(|(slot, item)| {
                (
                    kit.handle.update_id,
                    kit.handle.inventory_index,
                    slot,
                    item.item_id(),
                )
            });
            if state.equipment != equipment {
                // Same-life equipment changes invalidate the effect, never the
                // already established cooldown.
                state.effect = None;
                state.equipment = equipment;
            }
            if !enabled {
                state.effect = None;
                self.bot_registry.bots.get_mut(&id).unwrap().healing = state;
                continue;
            }
            let before = bot.health;
            let damage = bot.damage;
            let generation = runtime.generation;
            let mut receipts = Vec::new();
            if state.effect.is_none()
                && bot.received_healing.is_none()
                && now >= state.next_allowed
                && before < maximum
                && u32::from(before) * 1000
                    <= u32::from(maximum)
                        * u32::from(policy.as_ref().unwrap().health_threshold_per_mille)
                && bot_firearms::safe(bot)
                && !bot.target.registry_ground_recovery()
                && bot.holding_access.is_none()
                && let Some((slot, item)) = item
            {
                let effect = healing::HealingEffect::new_with_modifiers(
                    item,
                    generation,
                    now,
                    &self.item_catalog,
                    &modifiers,
                )?;
                state.next_allowed = now
                    .checked_add(item.cooldown_with_catalog(&self.item_catalog)?)
                    .ok_or("healing_clock_overflow")?;
                state.effect = Some(effect);
                receipts.push(json!({"op":"bot_healing_started","timestamp_ms":now.as_millis() as u64,"object_index":id,"generation":generation,"team":bot.target.squad.team(),"gear_slot":slot,"item_id":item.item_id(),"health_before":before,"maximum_health":maximum,"next_allowed_ms":state.next_allowed.as_millis() as u64,"tick_amount_hp":effect.tick_amount_hp(),"policy":"self_only_catalog_effect"}));
            }
            let mut publication = None;
            if let Some(effect) = state.effect
                && let Some((resolved, remaining, ticks)) =
                    effect.advance_with_receipts(now, damage)
            {
                state.effect = remaining;
                if !ticks.is_empty() {
                    let packet = replication::CharacterDamageUpdate {
                        sequence: 2,
                        timestamp_ms: now.as_millis() as u32,
                        object_index: id,
                        damage: resolved,
                        forced_state: None,
                    }
                    .encode_with_profile(self.wire_profile)?;
                    receipts.push(json!({"op":"bot_healing_tick","timestamp_ms":now.as_millis() as u64,"object_index":id,"generation":generation,"health_before":before,"health_after":resolved.health_quarter_points,"maximum_health":maximum,"restored_hp_per_tick":ticks,"completed":remaining.is_none()}));
                    publication = Some((resolved, packet));
                }
            }
            for receipt in receipts {
                self.record(receipt, wall)?;
            }
            if let Some((resolved, packet)) = publication {
                self.commit_bot_healing(id, generation, before, resolved, maximum)?;
                self.broadcast_bot_packet(packet);
            }
            self.bot_registry.bots.get_mut(&id).unwrap().healing = state;
        }
        Ok(())
    }
}
