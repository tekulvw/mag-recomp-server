//! Typed objective interaction replies and shared authoritative countdowns.
use super::*;
use crate::objective_control::{ObjectiveActor, ObjectiveWorld};

fn actor(active: &Active) -> Option<ObjectiveActor> {
    let CharacterLife::Alive { generation, .. } = active.life else {
        return None;
    };
    if !active.entered_game || !active.ready_for_updates || active.mounted_seat.is_some() {
        return None;
    }
    Some(ObjectiveActor {
        player: active.reservation.player,
        team: active.team(),
        generation,
        position: active.tracking.sample.as_ref()?.position,
    })
}

pub(super) fn interaction<W: Write>(
    ctx: &PlanningContext<'_, W>,
    plan: &mut EventPlan,
    request: protocol::ObjectiveInteraction,
) -> Result<Value, &'static str> {
    use protocol::ObjectiveInteractionKind as Kind;
    if matches!(request.kind, Kind::Ack | Kind::Nak) || !ctx.accepted_objective_route {
        return Ok(
            json!({"type":"objective_interaction","action":request.kind as u8,
            "target_object_id":request.target_object_id,"accepted":false,
            "reason":"objective_server_only_message"}),
        );
    }
    let mut accepted = false;
    let mut reason = "objective_unavailable";
    let mut reply_trigger_index = u8::MAX;
    let mut changed = None;
    if !ctx.history_available {
        reason = "replication_history_limit";
    } else if ctx.engine.match_terminal.is_some() {
        reason = "objective_match_finished";
    } else if !ctx.engine.match_started {
        reason = "objective_match_not_started";
    } else if let Some(actor) = actor(ctx.active) {
        if let Some(mut world) = ctx.engine.objective_world.clone() {
            let previous_owner = world.interaction_owner(request.target_object_id as u16);
            let modifier = plan
                .leadership
                .modifiers(ctx.active)
                .value("kCharModDemolitionTime");
            let outcome = if modifier == 0.0 {
                world.interact(request, actor, ctx.now)
            } else {
                world.interact_with_demolition_modifier(request, actor, ctx.now, modifier)
            };
            if let Some(delta) = outcome.changed
                && let Some((player, generation, trigger)) = previous_owner
                && generation == actor.generation
                && delta.state_actor.state != replication::ObjectiveState::Activating
            {
                plan.messages.push(exit_message(
                    ctx.active,
                    delta.object_index,
                    player,
                    trigger,
                ));
            }
            accepted = outcome.accepted;
            reason = outcome.reason;
            reply_trigger_index = outcome.reply_trigger_index;
            changed = outcome.changed;
            if accepted {
                world.attribute_operation(
                    request.target_object_id as u16,
                    progression::RewardRecipient::from_active(ctx.active),
                );
                plan.objective_world = Some(world);
            }
        }
    } else {
        reason = "objective_actor_not_on_foot";
    }
    if !accepted {
        plan.gameplay_denial = Some(reason);
    }
    let reply = protocol::ObjectiveInteraction {
        target_object_id: request.target_object_id,
        kind: if accepted { Kind::Ack } else { Kind::Nak },
        actor_index: ctx.active.reservation.player,
        trigger_index: reply_trigger_index,
    };
    plan.messages.push(OutboundMessage {
        reservation_id: ctx.active.reservation.id.clone(),
        world_id: ctx.active.reservation.world,
        player_id: ctx.active.reservation.player,
        plugin_id: 1,
        payload_hex: protocol::encode_hex(&reply.encode()),
    });
    if let Some(delta) = changed {
        let packet = replication::ObjectiveControlSnapshot {
            sequence: ctx.next_sequence,
            timestamp_ms: ctx.now.as_millis() as u32,
            objectives: vec![delta],
        }
        .encode_with_profile(ctx.engine.wire_profile)?;
        plan.replication.objective_delta = Some(packet.clone());
        plan.outgoing = Some(packet);
    }
    Ok(
        json!({"type":"objective_interaction","action":request.kind as u8,
        "target_object_id":request.target_object_id,"actor_index":request.actor_index,
        "trigger_index":request.trigger_index,"accepted":accepted,"reason":reason,
        "state":changed.map(|d|d.state_actor.state as u8),
        "expiration_time_ms":changed.map(|d|d.expiration_time_ms)}),
    )
}

impl<W: Write> Engine<W> {
    pub(crate) fn reset_objective_world(&mut self) {
        self.reset_spawn_progression();
        self.objective_world = self.objective_catalog.as_ref().map(ObjectiveWorld::new);
        self.bot_registry.director = Default::default();
        for bot in self.bot_registry.bots.values_mut() {
            bot.task = Default::default();
        }
        self.score_clock = score_clock::Clock::default();
        self.round = Default::default();
        self.match_terminal = None;
        self.reward_match_id = None;
        self.round_participants.clear();
    }

    /// Cancel the old actor while its bot identity is still known. Human life
    /// generations are independent and may reuse the same numeric slot and life.
    pub(crate) fn release_retiring_bot_objectives(
        &mut self,
        actor: ObjectiveActor,
        now: Duration,
        wall: u64,
    ) -> Result<(), &'static str> {
        let Some(mut world) = self.objective_world.clone() else {
            return Ok(());
        };
        let mut changed = Vec::new();
        for delta in world.snapshot(1, now.as_millis() as u32).objectives {
            let Some((id, generation, trigger)) = world.interaction_owner(delta.object_index)
            else {
                continue;
            };
            if id != actor.player {
                continue;
            }
            let outcome = world.interact(
                protocol::ObjectiveInteraction {
                    target_object_id: 0x0c80_0000 | u32::from(delta.object_index),
                    kind: protocol::ObjectiveInteractionKind::Release,
                    actor_index: actor.player,
                    trigger_index: trigger,
                },
                ObjectiveActor {
                    generation,
                    ..actor
                },
                now,
            );
            if let Some(delta) = outcome.changed {
                changed.push(delta);
            }
        }
        if changed.is_empty() {
            return Ok(());
        }
        let packet = replication::ObjectiveControlSnapshot {
            sequence: 1,
            timestamp_ms: now.as_millis() as u32,
            objectives: changed.clone(),
        }
        .encode_with_profile(self.wire_profile)?;
        self.record(json!({"op":"bot_objective_action","object_index":actor.player,"generation":actor.generation,"action":"release","reason":"bot_actor_retired","accepted":true,"objectives":changed.iter().map(|d|d.object_index).collect::<Vec<_>>(),"timestamp_ms":now.as_millis() as u64}), wall)?;
        self.objective_world = Some(world);
        for recipient in self
            .peers
            .values_mut()
            .chain(self.active.iter_mut())
            .filter(|a| a.initial_objects.is_some() && a.ready_for_updates)
        {
            recipient.queue_shared(packet.clone());
        }
        Ok(())
    }

    /// Opt-in server fixture for a real defender's normal C defuse interaction.
    pub(crate) fn seed_development_demolition(
        &mut self,
        reservation_id: &str,
        world_id: u32,
        player_id: u16,
        object_index: u16,
        now: Duration,
        wall: u64,
    ) -> Result<Vec<OutboundMessage>, &'static str> {
        if !self.development_control {
            return Err("development_control_disabled");
        }
        let active = self.active.as_ref().ok_or("no_reservation")?;
        if active.reservation.id != reservation_id
            || active.reservation.world != world_id
            || active.reservation.player != player_id
        {
            return Err("identity_mismatch");
        }
        if active.claimed.is_none() || !self.match_started || actor(active).is_none() {
            return Err("development_demolition_requires_ready_defender");
        }
        let rule = self
            .objective_catalog
            .as_ref()
            .and_then(|catalog| {
                catalog
                    .objects
                    .iter()
                    .find(|o| o.object_index == object_index)
            })
            .and_then(|object| object.demolition.as_ref())
            .ok_or("development_demolition_not_configured")?;
        if active.team() != rule.owner_team || !rule.can_defuse {
            return Err("development_demolition_requires_ready_defender");
        }
        if self.maximum_players == 1 && active.replication_history.len() >= MAX_REPLICATION_HISTORY
        {
            return Err("replication_history_limit");
        }
        let mut world = self
            .objective_world
            .clone()
            .ok_or("objective_world_unavailable")?;
        let delta = world.development_arm(object_index, now)?;
        let packet = replication::ObjectiveControlSnapshot {
            sequence: 1,
            timestamp_ms: now.as_millis() as u32,
            objectives: vec![delta],
        }
        .encode_with_profile(self.wire_profile)?;
        self.record(
            json!({"op":"development_demolition_seed",
            "reservation_id":reservation_id,"world_id":world_id,"player_id":player_id,
            "object_index":object_index,"state":delta.state_actor.state as u8,
            "actor_index":delta.state_actor.actor_index,"timestamp_ms":now.as_millis() as u32,
            "expiration_time_ms":delta.expiration_time_ms}),
            wall,
        )?;
        self.objective_world = Some(world);
        for recipient in self.peers.values_mut().chain(self.active.iter_mut()) {
            if recipient.initial_objects.is_some() && recipient.ready_for_updates {
                recipient.queue_shared(packet.clone());
            }
        }
        Ok(Vec::new())
    }

    /// Advances one shared clock from normal traffic, including idle time requests.
    /// Each recipient gets its own reliable sequence through the shared queue.
    pub(crate) fn tick_objectives(
        &mut self,
        now: Duration,
        wall: u64,
    ) -> Result<Vec<OutboundMessage>, &'static str> {
        if self.match_terminal.is_some() || !self.match_started {
            return Ok(Vec::new());
        }
        let Some(mut world) = self.objective_world.clone() else {
            return Ok(Vec::new());
        };
        let mut actors: std::collections::BTreeMap<_, _> = self
            .peers
            .values()
            .chain(self.active.iter())
            .filter_map(actor)
            .map(|actor| (actor.player, actor))
            .collect();
        // Bot character IDs occupy real world slots, without reservation/account aliases.
        for (&id, bot) in &self.bot_registry.bots {
            let r = bot.target.runtime.unwrap();
            if bot.health > 0 {
                actors.entry(id).or_insert(ObjectiveActor {
                    player: id,
                    team: bot.target.squad.team(),
                    generation: r.generation,
                    position: r.position,
                });
            }
        }
        let prior_owners: std::collections::BTreeMap<_, _> = world
            .snapshot(1, now.as_millis() as u32)
            .objectives
            .iter()
            .filter_map(|d| {
                world
                    .interaction_owner(d.object_index)
                    .map(|owner| (d.object_index, owner))
            })
            .collect();
        let previous_assignments: std::collections::BTreeMap<_, _> = self
            .peers
            .values()
            .chain(self.active.iter())
            .map(|a| {
                (
                    a.reservation.id.clone(),
                    self.initial_objective_index(a.squad_catalog.squad_index())
                        .ok(),
                )
            })
            .collect();
        let mut changes = std::collections::BTreeMap::new();
        let enabled = self.bot_registry.config.as_ref().is_some_and(|c| {
            c.autonomous
                && c.objective_policy
                    .as_ref()
                    .is_some_and(|p| p.interactions_enabled)
        });
        let mut receipts = Vec::new();
        let mut holding = Vec::new();
        if let Some(catalog) = &self.objective_catalog {
            for object in &catalog.objects {
                let index = object.object_index;
                if let Some((id, generation, trigger)) = world.interaction_owner(index)
                    && let Some(bot) = self.bot_registry.bots.get(&id)
                    && (!enabled || !bot.task.operator || bot.task.objective != Some(index))
                {
                    // Retasking is a real release, not synthetic actor disappearance.
                    if let Some(actor) = actors.get(&id).filter(|a| a.generation == generation) {
                        let outcome = world.interact(
                            protocol::ObjectiveInteraction {
                                target_object_id: 0x0c80_0000 | u32::from(index),
                                kind: protocol::ObjectiveInteractionKind::Release,
                                actor_index: id,
                                trigger_index: trigger,
                            },
                            *actor,
                            now,
                        );
                        if let Some(delta) = outcome.changed {
                            changes.insert(index, delta);
                        }
                        receipts.push(json!({"op":"bot_objective_action","object_index":id,"generation":generation,"objective":index,"action":"release","accepted":outcome.accepted,"reason":outcome.reason,"timestamp_ms":now.as_millis() as u64}));
                    }
                }
                if !enabled || world.interaction_owner(index).is_some() {
                    continue;
                }
                for (&id, bot) in &self.bot_registry.bots {
                    if !bot.task.operator
                        || bot.task.objective != Some(index)
                        || bot.task.recovering
                        || bot.drop_waiting.is_some()
                        || bot.drop_committed
                        || bot.drop_holding_goal.is_some()
                        || bot.drop_resume
                        || bot.drop_egress
                        || bot.drop_retreat
                        || bot.drop_reroute
                        || bot.traffic_yield.is_some()
                    {
                        continue;
                    }
                    let Some(actor) = actors.get(&id) else {
                        continue;
                    };
                    let r = bot.target.runtime.unwrap();
                    if !r.physics_motion.is_some_and(|m| m.grounded) || r.navmesh_state.on_player {
                        continue;
                    }
                    let Some((kind, center, radius)) = crate::bot_objectives::operator_action(
                        object,
                        world.state(index).unwrap(),
                        actor.team,
                    ) else {
                        continue;
                    };
                    let d2 = (actor.position.x - center[0]).powi(2)
                        + (actor.position.y - center[1]).powi(2)
                        + (actor.position.z - center[2]).powi(2);
                    if d2 > radius * radius {
                        continue;
                    }
                    let outcome = world.interact(
                        protocol::ObjectiveInteraction {
                            target_object_id: 0x0c80_0000 | u32::from(index),
                            kind,
                            actor_index: id,
                            trigger_index: 0,
                        },
                        *actor,
                        now,
                    );
                    if let Some(delta) = outcome.changed {
                        changes.insert(index, delta);
                    }
                    if outcome.accepted {
                        holding.push(id);
                    }
                    receipts.push(json!({"op":"bot_objective_action","object_index":id,"generation":actor.generation,"objective":index,"action":kind as u8,"accepted":outcome.accepted,"reason":outcome.reason,"team":actor.team,"position":actor.position,"distance_m":d2.sqrt(),"maximum_distance_m":radius,"timestamp_ms":now.as_millis() as u64}));
                }
            }
        }
        for id in holding {
            let bot = self.bot_registry.bots.get_mut(&id).unwrap();
            let r = bot.target.runtime.as_mut().unwrap();
            // Accepted holds stop at the actual supported position. No boundary
            // crossing or arrival correction is needed to keep a timer running.
            r.phase = development_target::BotPhase::Complete;
            bot.target.pending_move = None;
            bot.task.goal = Some(r.position);
            bot.task.role_goal = Some(r.position);
        }
        for delta in world.tick(now, &actors) {
            changes.insert(delta.object_index, delta);
        }
        for receipt in receipts {
            self.record(receipt, wall)?;
        }
        let changed: Vec<_> = changes.into_values().collect();
        let rewards: Vec<_> = world
            .take_reward_actions()
            .into_iter()
            .filter(|action| {
                self.peers.values().chain(self.active.iter()).any(|actor| {
                    actor.reservation.id == action.recipient.reservation_id
                        && actor.reservation.world == action.recipient.world_id
                        && actor.reservation.player == action.recipient.player_id
                })
            })
            .map(|action| {
                let id = format!(
                    "objective-{:016x}-{}-{}-{:?}",
                    self.round_number,
                    action.object_index,
                    action.started_at.as_nanos(),
                    action.kind
                );
                action.recipient.event(id, action.kind, 1)
            })
            .collect();
        self.check_reward_capacity(&rewards)?;
        if changed.is_empty() {
            self.advance_forward_spawns(now)?;
            return Ok(Vec::new());
        }
        let packet = replication::ObjectiveControlSnapshot {
            sequence: 1,
            timestamp_ms: now.as_millis() as u32,
            objectives: changed.clone(),
        }
        .encode_with_profile(self.wire_profile)?;
        let mut messages = Vec::new();
        for delta in &changed {
            if let Some(&(player, generation, trigger)) = prior_owners.get(&delta.object_index)
                && let Some(recipient) = self.peers.values().chain(self.active.iter()).find(|a| {
                    a.reservation.player == player
                        && matches!(a.life, CharacterLife::Alive {generation:g,..} if g==generation)
                })
            {
                messages.push(exit_message(recipient, delta.object_index, player, trigger));
            }
        }
        self.record(
            json!({"op":"objective_tick","messages":messages,"rewards":rewards,"timestamp_ms":now.as_millis() as u32,
                "objectives":changed.iter().map(|d| json!({
                    "object_index":d.object_index,"state":d.state_actor.state as u8,
                    "actor_index":d.state_actor.actor_index,
                    "expiration_time_ms":d.expiration_time_ms})).collect::<Vec<_>>() }),
            wall,
        )?;
        self.objective_world = Some(world);
        self.commit_rewards(rewards);
        self.advance_forward_spawns(now)?;
        for recipient in self.peers.values_mut().chain(self.active.iter_mut()) {
            if recipient.initial_objects.is_some() && recipient.ready_for_updates {
                recipient.queue_shared(packet.clone());
            }
            let next = self.objective_catalog.as_ref().and_then(|catalog| {
                self.objective_world.as_ref().and_then(|world| {
                    catalog
                        .dynamic_assignment(recipient.squad_catalog.squad_index(), |index| {
                            world.state(index)
                        })
                        .ok()
                })
            });
            if previous_assignments
                .get(&recipient.reservation.id)
                .copied()
                .flatten()
                != next
            {
                recipient.objective_assigned = false;
                recipient.objective_needs_clear = true;
            }
        }
        Ok(messages)
    }
}

fn exit_message(active: &Active, index: u16, player: u16, trigger: u8) -> OutboundMessage {
    let notification = protocol::ObjectiveInteraction {
        target_object_id: 0x0c80_0000 | u32::from(index),
        kind: protocol::ObjectiveInteractionKind::ActivatingOnExit,
        actor_index: player,
        trigger_index: trigger,
    };
    OutboundMessage {
        reservation_id: active.reservation.id.clone(),
        world_id: active.reservation.world,
        player_id: active.reservation.player,
        plugin_id: 1,
        payload_hex: protocol::encode_hex(&notification.encode()),
    }
}

impl<W: Write> Engine<W> {
    pub(crate) fn tick_match_clock(
        &mut self,
        now: Duration,
        wall: u64,
    ) -> Result<Vec<OutboundMessage>, &'static str> {
        if self
            .objective_catalog
            .as_ref()
            .is_some_and(|c| c.match_rules.is_some())
        {
            return self.tick_sabotage_round(now, wall);
        }
        let Some(rule) = self
            .objective_catalog
            .as_ref()
            .and_then(|c| c.score_bleed.as_ref())
        else {
            return Ok(Vec::new());
        };
        let Some(epoch) = self
            .match_epoch
            .or_else(|| self.active.as_ref().and_then(|a| a.match_clock_started))
        else {
            return Ok(Vec::new());
        };
        let paused = self
            .objective_world
            .as_ref()
            .and_then(|world| world.state(rule.pause_objective))
            .is_some_and(|state| {
                if rule.pause_states.is_empty() {
                    state as u8 == rule.pause_state
                } else {
                    rule.pause_states.contains(&(state as u8))
                }
            });
        let next_clock = self
            .score_clock
            .advance(epoch, now, paused, rule.interval_ms);
        let points = next_clock.points(
            self.match_profile().maximum_points[usize::from(rule.team)],
            rule.interval_ms,
            rule.points,
        );
        let mut messages = Vec::new();
        for active in self.peers.values().chain(self.active.iter()).filter(|a| {
            a.ready_for_updates && a.initial_objects.is_some() && points < a.last_clock_points
        }) {
            let score = protocol::ScoreChange {
                team: rule.team,
                points,
                reason: protocol::ScoreChangeReason::TimedBleed,
            };
            messages.push(OutboundMessage {
                reservation_id: active.reservation.id.clone(),
                world_id: active.reservation.world,
                player_id: active.reservation.player,
                plugin_id: 1,
                payload_hex: protocol::encode_hex(&score.encode_with_profile(self.wire_profile)?),
            });
        }
        if !messages.is_empty() {
            self.record(json!({"op":"match_clock_tick","team":rule.team,"points":points,"paused":paused,"messages":messages}), wall)?;
        }
        self.score_clock = next_clock;
        for active in self
            .peers
            .values_mut()
            .chain(self.active.iter_mut())
            .filter(|a| {
                a.ready_for_updates && a.initial_objects.is_some() && points < a.last_clock_points
            })
        {
            active.last_clock_points = points;
        }
        Ok(messages)
    }
}
