//! World-owned self-healing. AI admission is policy; all healing amounts and
//! timing use the same extracted item catalog and effect as human healing.
use super::*;

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Policy {
    pub enabled: bool,
    pub health_threshold_per_mille: u16,
    /// Bots carrying an item that heals others (one with an authored
    /// recipient sphere, the medkit) use it on injured teammates in reach.
    pub teammates: bool,
    /// Squad role slots whose bots carry that item in place of their
    /// self-healing item. Empty keeps every bot on its default kit.
    pub medic_role_slots: Vec<u8>,
    /// A bot medic revives a downed teammate in reach of its medkit.
    pub revive: bool,
    /// Health a bot revival restores, as a share of the reviver's maximum.
    /// Bots own no skills, so this stands in for the resuscitation modifier.
    pub revive_health_per_mille: u16,
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            enabled: true,
            health_threshold_per_mille: 750,
            teammates: true,
            medic_role_slots: vec![1],
            revive: true,
            revive_health_per_mille: 250,
        }
    }
}
impl Policy {
    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        if self.health_threshold_per_mille > 1000
            || !(1..=1000).contains(&self.revive_health_per_mille)
            || self.medic_role_slots.iter().any(|&slot| slot >= 8)
        {
            return Err("invalid_bot_healing_policy");
        }
        Ok(())
    }
    pub(crate) fn medic(&self, role_slot: u8) -> bool {
        self.enabled && self.teammates && self.medic_role_slots.contains(&role_slot)
    }
}
/// The catalog item that heals others, and its authored reach.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TeamItem {
    pub id: u16,
    pub forward_offset_m: f64,
    pub radius_m: f64,
}
pub(crate) fn team_item(items: &item_catalog::ItemCatalog) -> Option<TeamItem> {
    let catalog = items.retail_combat().ok()?;
    catalog.healing().iter().find_map(|(&id, metadata)| {
        let sphere = metadata.recipient_sphere.as_ref()?;
        (sphere.radius.is_finite()
            && sphere.radius > 0.
            && sphere.forward_offset.is_finite()
            && sphere.forward_offset >= 0.)
            .then_some(TeamItem {
                id,
                forward_offset_m: sphere.forward_offset,
                radius_m: sphere.radius,
            })
    })
}
/// A medic's kit: the default kit with its self-healing item replaced by the
/// item that heals others. Kits without a healing slot stay unchanged.
pub(crate) fn medic_loadout(
    policy: &Policy,
    role_slot: u8,
    items: &item_catalog::ItemCatalog,
    mut kit: replication::CharacterLoadout,
) -> replication::CharacterLoadout {
    if !policy.medic(role_slot) {
        return kit;
    }
    let Some(team) = team_item(items) else {
        return kit;
    };
    let index = usize::from(kit.handle.inventory_index);
    let Some(inventory) = kit.inventories.get_mut(index) else {
        return kit;
    };
    if inventory.slots.contains(&Some(team.id)) {
        return kit;
    }
    if let Some(slot) = inventory
        .slots
        .iter_mut()
        .find(|slot| slot.is_some_and(|id| items.healing(id).is_ok()))
    {
        *slot = Some(team.id);
    }
    kit
}
fn medkit_slot(
    items: &item_catalog::ItemCatalog,
    team: TeamItem,
    kit: &replication::CharacterLoadout,
) -> Option<(u8, healing::HealItem)> {
    let inventory = kit
        .inventories
        .get(usize::from(kit.handle.inventory_index))?;
    let slot = inventory.slots.iter().position(|s| *s == Some(team.id))? as u8;
    healing::HealItem::from_loadout_with_catalog(kit, slot, items)
        .ok()
        .map(|item| (slot, item))
}
fn horizontal(a: [f64; 3], b: [f64; 3]) -> f64 {
    (a[0] - b[0]).hypot(a[2] - b[2])
}
#[derive(Clone, Copy, Debug)]
enum Patient {
    Bot(u16),
    Human(u16),
    Downed(u16),
}
fn key(patient: Patient) -> (u8, u16) {
    match patient {
        Patient::Bot(id) => (0, id),
        Patient::Human(id) | Patient::Downed(id) => (1, id),
    }
}
struct Candidate {
    patient: Patient,
    feet: [f64; 3],
    /// Health share in per mille; zero for a downed teammate.
    share: u32,
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
                        seated_exit: None,
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
    /// Bot medics use the item that heals others on injured or downed
    /// teammates (bots and humans) inside its authored recipient sphere.
    fn tick_bot_teammate_healing(
        &mut self,
        now: Duration,
        wall: u64,
    ) -> Result<Vec<OutboundMessage>, &'static str> {
        let Some(policy) = self
            .bot_registry
            .config
            .as_ref()
            .map(|c| c.healing.clone())
            .filter(|p| p.enabled && p.teammates)
        else {
            return Ok(Vec::new());
        };
        let world = self
            .active
            .iter()
            .chain(self.peers.values())
            .find(|a| a.ready_for_updates && a.initial_objects.is_some())
            .map(|a| a.reservation.world);
        let (Some(world), Some(team_item)) = (world, team_item(&self.item_catalog)) else {
            return Ok(Vec::new());
        };
        if !self.match_started
            || self.match_terminal.is_some()
            || !self.round_combat_allowed(now)
            || self.wire_profile.application_id() != 20231
        {
            return Ok(Vec::new());
        }
        let leadership = self.plan_leadership(now)?;
        let healer_modifiers = skill_catalog::OwnedModifiers::default();
        let bot_maximum = healer_modifiers.max_health_quarter_points()?;
        let threshold = u32::from(policy.health_threshold_per_mille);
        let revive = policy.revive && self.incapacitation_rules.is_some();
        let mut candidates: Vec<(u8, Candidate)> = Vec::new();
        for a in self.active.iter().chain(self.peers.values()).filter(|a| {
            a.reservation.world == world && a.entered_game && a.ready_for_updates && !a.is_mounted()
        }) {
            let Some(sample) = a.tracking.sample else {
                continue;
            };
            let feet = [
                f64::from(sample.position.x),
                f64::from(sample.position.y),
                f64::from(sample.position.z),
            ];
            let player = a.reservation.player;
            match a.life {
                CharacterLife::Alive {
                    health_quarter_points,
                    ..
                } => {
                    let maximum = u32::from(leadership.modifiers(a).max_health_quarter_points()?);
                    if a.healing.is_none()
                        && u32::from(health_quarter_points) * 1000 <= maximum * threshold
                    {
                        candidates.push((
                            a.team(),
                            Candidate {
                                patient: Patient::Human(player),
                                feet,
                                share: u32::from(health_quarter_points) * 1000 / maximum.max(1),
                            },
                        ));
                    }
                }
                CharacterLife::Incapacitated { downed, .. }
                    if revive && !downed.is_expired(now) =>
                {
                    candidates.push((
                        a.team(),
                        Candidate {
                            patient: Patient::Downed(player),
                            feet,
                            share: 0,
                        },
                    ));
                }
                _ => {}
            }
        }
        for (&id, bot) in &self.bot_registry.bots {
            let Some(r) = bot.target.runtime else {
                continue;
            };
            let healing_self =
                bot.healing.generation == Some(r.generation) && bot.healing.effect.is_some();
            if bot.health > 0
                && !matches!(r.phase, development_target::BotPhase::Dead)
                && !healing_self
                && bot.received_healing.is_none()
                && u32::from(bot.health) * 1000 <= u32::from(bot_maximum) * threshold
            {
                candidates.push((
                    bot.target.squad.team(),
                    Candidate {
                        patient: Patient::Bot(id),
                        feet: r.physical_position(),
                        share: u32::from(bot.health) * 1000 / u32::from(bot_maximum),
                    },
                ));
            }
        }
        let mut messages = Vec::new();
        if candidates.is_empty() {
            return Ok(messages);
        }
        let reach = team_item.forward_offset_m + team_item.radius_m;
        let mut taken = std::collections::BTreeSet::new();
        let ids: Vec<_> = self.bot_registry.bots.keys().copied().collect();
        for id in ids {
            let bot = &self.bot_registry.bots[&id];
            let Some(r) = bot.target.runtime else {
                continue;
            };
            let Some((slot, item)) =
                medkit_slot(&self.item_catalog, team_item, &bot.target.loadout)
            else {
                continue;
            };
            let ready =
                bot.healing.generation != Some(r.generation) || now >= bot.healing.next_allowed;
            if !ready
                || bot.health == 0
                || matches!(r.phase, development_target::BotPhase::Dead)
                || !r.physics_motion.is_some_and(|m| m.grounded)
                || r.navmesh_state.on_player
                || bot.drop_committed
                || bot.has_supply_action()
            {
                continue;
            }
            let me = r.physical_position();
            let team = bot.target.squad.team();
            // In a firefight only a badly hurt or downed teammate is worth it.
            let fighting = bot.firearm.target_index().is_some();
            let near: Vec<&Candidate> = candidates
                .iter()
                .filter(|(t, c)| {
                    *t == team
                        && !matches!(c.patient, Patient::Bot(b) if b == id)
                        && !taken.contains(&key(c.patient))
                        && horizontal(me, c.feet) <= reach
                        && (me[1] - c.feet[1]).abs() <= 2.
                        && (!fighting || c.share <= 400)
                })
                .map(|(_, c)| c)
                .collect();
            let Some(primary) = near.iter().min_by(|a, b| {
                a.share
                    .cmp(&b.share)
                    .then(horizontal(me, a.feet).total_cmp(&horizontal(me, b.feet)))
            }) else {
                continue;
            };
            // The medkit acts on a sphere just ahead of the user: face the
            // neediest teammate and treat everyone inside it.
            let d = horizontal(me, primary.feet);
            let center = if d > 1e-3 {
                let k = team_item.forward_offset_m / d;
                [
                    me[0] + (primary.feet[0] - me[0]) * k,
                    me[1],
                    me[2] + (primary.feet[2] - me[2]) * k,
                ]
            } else {
                me
            };
            let patients: Vec<_> = near
                .iter()
                .filter(|c| horizontal(center, c.feet) <= team_item.radius_m)
                .map(|c| c.patient)
                .collect();
            if patients.is_empty() {
                continue;
            }
            let generation = r.generation;
            let squad = bot.target.squad.squad_index();
            let cooldown = item.cooldown_with_catalog(&self.item_catalog)?;
            let mut treated = Vec::new();
            for patient in patients {
                taken.insert(key(patient));
                match patient {
                    Patient::Bot(index) => {
                        let target = &self.bot_registry.bots[&index];
                        let runtime = target.target.runtime.ok_or("bot_runtime_missing")?;
                        let effect = healing::HealingEffect::new_for_recipient(
                            item,
                            runtime.generation,
                            now,
                            &self.item_catalog,
                            &healer_modifiers,
                            &skill_catalog::OwnedModifiers::default(),
                        )?;
                        let received = Received {
                            index,
                            allocation: target.allocation_generation,
                            world,
                            round: self.round_number,
                            team,
                            healer_allocation: 0,
                            session: progression::HealingSession {
                                effect,
                                healer: None,
                                effect_id: format!(
                                    "bot-heal-{id:04x}-{generation:08x}-{:016x}-bot-{index:04x}",
                                    now.as_millis()
                                ),
                            },
                        };
                        treated.push(json!({"kind":"bot","object_index":index,"health_before":target.health}));
                        self.install_received_bot_healing(received);
                    }
                    Patient::Human(player) => {
                        let Some(target) = self
                            .active
                            .iter_mut()
                            .chain(self.peers.values_mut())
                            .find(|a| {
                                a.reservation.player == player && a.reservation.world == world
                            })
                        else {
                            continue;
                        };
                        let CharacterLife::Alive {
                            generation: life,
                            health_quarter_points,
                        } = target.life
                        else {
                            continue;
                        };
                        let effect = healing::HealingEffect::new_for_recipient(
                            item,
                            life,
                            now,
                            &self.item_catalog,
                            &healer_modifiers,
                            &leadership.modifiers(target),
                        )?;
                        target.healing = Some(progression::HealingSession {
                            effect,
                            healer: None,
                            effect_id: format!(
                                "bot-heal-{id:04x}-{generation:08x}-{:016x}-{player:04x}",
                                now.as_millis()
                            ),
                        });
                        treated.push(json!({"kind":"human","player":player,"health_before":health_quarter_points}));
                    }
                    Patient::Downed(player) => {
                        if let Some(record) =
                            self.bot_revive(id, squad, player, world, &policy, now, &mut messages)?
                        {
                            treated.push(record);
                        }
                    }
                }
            }
            // The user stands inside its own sphere and is treated as well.
            let bot = self.bot_registry.bots.get_mut(&id).unwrap();
            if bot.healing.generation != Some(generation) {
                bot.healing = State {
                    generation: Some(generation),
                    ..Default::default()
                };
            }
            let kit = &bot.target.loadout;
            bot.healing.equipment = Some((
                kit.handle.update_id,
                kit.handle.inventory_index,
                slot,
                item.item_id(),
            ));
            let own = bot.health < bot_maximum
                && bot.healing.effect.is_none()
                && bot.received_healing.is_none()
                && horizontal(center, me) <= team_item.radius_m;
            if own {
                bot.healing.effect = Some(healing::HealingEffect::new_with_modifiers(
                    item,
                    generation,
                    now,
                    &self.item_catalog,
                    &healer_modifiers,
                )?);
            }
            bot.healing.next_allowed = now.checked_add(cooldown).ok_or("healing_clock_overflow")?;
            self.record(
                json!({"op":"bot_teammate_healing_started","timestamp_ms":now.as_millis() as u64,
                    "object_index":id,"generation":generation,"team":team,"gear_slot":slot,
                    "item_id":item.item_id(),"center":center,"radius_m":team_item.radius_m,
                    "in_combat":fighting,"self_treated":own,"patients":treated}),
                wall,
            )?;
        }
        Ok(messages)
    }

    /// A bot medic revives a downed human teammate, as the human Resuscitate
    /// request does: the patient's damage record is restored to the policy's
    /// share of health and both squads hear the resuscitation notice.
    #[allow(clippy::too_many_arguments)]
    fn bot_revive(
        &mut self,
        healer: u16,
        healer_squad: u16,
        player: u16,
        world: u32,
        policy: &Policy,
        now: Duration,
        messages: &mut Vec<OutboundMessage>,
    ) -> Result<Option<Value>, &'static str> {
        let leadership = self.plan_leadership(now)?;
        let Some(patient) = self
            .active
            .iter()
            .chain(self.peers.values())
            .find(|a| a.reservation.player == player && a.reservation.world == world)
        else {
            return Ok(None);
        };
        let CharacterLife::Incapacitated { generation, downed } = patient.life else {
            return Ok(None);
        };
        if downed.is_expired(now) {
            return Ok(None);
        }
        let health = incapacitation::resuscitated_health_quarter_points(
            skill_catalog::OwnedModifiers::default().max_health_hp(),
            leadership.modifiers(patient).max_health_hp(),
            f32::from(policy.revive_health_per_mille) / 1000.,
        )?;
        if health == 0 {
            return Ok(None);
        }
        let damage = replication::CharacterDamage {
            last_location_and_type: (7 << 4) | (patient.last_damage.last_location_and_type & 15),
            health_quarter_points: health,
            conditions: patient.last_damage.conditions & !2,
            ..patient.last_damage
        };
        let packet = replication::CharacterDamageUpdate {
            seated_exit: None,
            sequence: 2,
            timestamp_ms: now.as_millis() as u32,
            object_index: player,
            damage,
            forced_state: None,
        }
        .encode_with_profile(self.wire_profile)?;
        let notice = protocol::RetailResuscitationNotification {
            healer,
            patient: player,
        }
        .encode()?;
        let patient_squad = patient.squad_catalog.squad_index();
        let id = patient.reservation.id.clone();
        for member in self.active.iter().chain(self.peers.values()).filter(|a| {
            a.reservation.world == world
                && a.ready_for_updates
                && a.initial_objects.is_some()
                && [healer_squad, patient_squad].contains(&a.squad_catalog.squad_index())
        }) {
            messages.push(OutboundMessage {
                reservation_id: member.reservation.id.clone(),
                world_id: member.reservation.world,
                player_id: member.reservation.player,
                plugin_id: 1,
                payload_hex: protocol::encode_hex(&notice),
            });
        }
        let patient = self
            .active
            .iter_mut()
            .chain(self.peers.values_mut())
            .find(|a| a.reservation.id == id)
            .ok_or("stale_revive_patient")?;
        patient.commit_revive(generation, damage)?;
        self.firearm_assists.forget_victim(player);
        // Queued shared packets are resequenced per recipient on delivery.
        self.broadcast_bot_packet(packet);
        Ok(Some(
            json!({"kind":"revived","player":player,"generation":generation,
            "health_quarter_points":health}),
        ))
    }

    pub(crate) fn tick_bot_healing(
        &mut self,
        now: Duration,
        wall: u64,
    ) -> Result<Vec<OutboundMessage>, &'static str> {
        self.tick_received_bot_healing(now, wall)?;
        let messages = self.tick_bot_teammate_healing(now, wall)?;
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
                        seated_exit: None,
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
                self.broadcast_bot_state_packet(id, packet);
            }
            self.bot_registry.bots.get_mut(&id).unwrap().healing = state;
        }
        Ok(messages)
    }
}
