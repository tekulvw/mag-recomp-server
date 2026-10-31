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
impl<W: Write> Engine<W> {
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
