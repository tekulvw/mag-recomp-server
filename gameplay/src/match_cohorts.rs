//! Finished actors retain result delivery, never another live simulation.
use crate::*;
use std::collections::BTreeMap;

pub(crate) const CAPABILITY: &str = "independent_match_cohorts_v1";
const MAX_RESULTS_COHORTS: usize = 8;

pub(crate) struct ResultsCohort {
    pub(crate) actors: BTreeMap<String, Active>,
    pub(crate) round: match_flow::Round,
    pub(crate) terminal: match_flow::MatchTerminal,
    pub(crate) reward_match_id: String,
    pub(crate) participants: BTreeMap<progression::ParticipantKey, progression::RoundParticipation>,
    pub(crate) profile: wire_profile::WireProfile,
    pub(crate) game_type: i8,
    pub(crate) cursor: u16,
}

impl ResultsCohort {
    fn remove_actor(&mut self, id: &str) {
        if let Some(actor) = self.actors.remove(id)
            && let Some(key) = progression::participant_key(&actor.reservation)
        {
            self.participants.insert(
                key,
                progression::RoundParticipation {
                    match_experience: actor.match_experience,
                    statistics: actor.reward_statistics,
                    repair_credit: actor.repair_credit,
                },
            );
        }
    }
}

impl<W: Write> Engine<W> {
    pub(crate) fn check_cohort_rollover(&self) -> Result<(), &'static str> {
        if self.match_terminal.is_none() {
            return Err("match_not_terminal");
        }
        if self.reward_match_id.is_none() || self.round.outcome.is_none() {
            return Err("terminal_cohort_identity_missing");
        }
        let retained = self
            .results_cohorts
            .values()
            .filter(|c| {
                !c.actors.is_empty()
                    || self
                        .pending_rewards
                        .values()
                        .any(|r| r.match_id == c.reward_match_id)
            })
            .count();
        if retained >= MAX_RESULTS_COHORTS {
            return Err("results_cohort_limit");
        }
        Ok(())
    }

    /// The caller records the containing selection/rollover before this install.
    pub(crate) fn archive_terminal_cohort(&mut self) {
        let game_type = self.match_profile().game_type;
        let terminal = self.match_terminal.take().expect("checked terminal cohort");
        let reward_match_id = self
            .reward_match_id
            .take()
            .expect("checked reward identity");
        let mut actors = std::mem::take(&mut self.peers);
        if let Some(actor) = self.active.take() {
            actors.insert(actor.reservation.id.clone(), actor);
        }
        for actor in actors.values_mut() {
            if let Some(receipt) = actor.pending_loadout_selection.take() {
                self.retired_loadout_selections
                    .insert(receipt.selection_id.clone(), receipt);
            }
            // Keep already committed world packets in their own bounded FIFO,
            // ahead of result notices. Never combine two full queues into one.
            actor.pending_spawn_deadline = None;
            actor.pending_damage = None;
            actor.healing = None;
        }
        self.results_cohorts.retain(|_, c| {
            !c.actors.is_empty()
                || self
                    .pending_rewards
                    .values()
                    .any(|r| r.match_id == c.reward_match_id)
        });
        self.results_cohorts.insert(
            reward_match_id.clone(),
            ResultsCohort {
                actors,
                terminal,
                reward_match_id,
                round: std::mem::take(&mut self.round),
                participants: std::mem::take(&mut self.round_participants),
                profile: self.wire_profile,
                game_type,
                cursor: self.shared_drain_cursor,
            },
        );
    }

    pub(crate) fn results_owner(&self, reservation: &str) -> Option<&ResultsCohort> {
        self.results_cohorts
            .values()
            .find(|c| c.actors.contains_key(reservation))
    }

    pub(crate) fn reserve_fixed_cohort(
        &mut self,
        request: Request,
        now: Duration,
        wall: u64,
    ) -> Result<Vec<OutboundMessage>, &'static str> {
        let Request::Reserve {
            reservation_id,
            world_id,
            player_id,
            account_id,
            session_key,
            access_key,
            character,
            ..
        } = request
        else {
            return Err("wrong_cohort_admission_operation");
        };
        self.check_cohort_rollover()?;
        self.validate_objective_configuration(self.wire_profile)?;
        if usize::from(player_id) >= self.maximum_players {
            return Err("invalid_player_slot");
        }
        if self
            .match_terminal
            .as_ref()
            .is_some_and(|t| t.world_id != world_id)
        {
            return Err("reservation_conflict");
        }
        if character.universe_id != 26 {
            return Err("unsupported_match_character");
        }
        let modifiers = self.validate_admission_character(&character)?;
        let squad_catalog = self.allocate_fresh_squad(character.faction)?;
        let shared_spawn = squad_catalog.default_spawn();
        let candidate = Reservation {
            id: reservation_id,
            world: world_id,
            player: player_id,
            account: account_id,
            session_key,
            access_key,
            character,
        };
        let round_number = self
            .round_number
            .checked_add(1)
            .ok_or("round_number_exhausted")?;
        // Selection, admission and rollover install only after this single record.
        self.record(json!({"op":"reserved","reservation_id":candidate.id,"world_id":world_id,"player_id":player_id,"account_id":account_id,"character":candidate.character,
            "team":squad_catalog.squad_index()/16,"squad_index":squad_catalog.squad_index(),"archived_match_terminal":self.match_terminal}),wall)?;
        self.archive_terminal_cohort();
        self.reset_objective_world();
        self.clear_bot_world();
        if let Some(world) = &mut self.bunker_world {
            world.reset();
        }
        self.next_repair.clear();
        self.pending_bunker_damage.clear();
        self.team_waves = [None; 2];
        self.parachute_waves.clear();
        self.shared_drain_cursor = 0;
        self.last_published_rosters.clear();
        self.world_epoch = None;
        self.match_epoch = None;
        self.alpha_turret = Default::default();
        self.alpha_turret_reporter = None;
        self.match_started = self.starts_without_quorum();
        self.install_reserved(
            admission::PreparedAdmission {
                candidate,
                modifiers,
                squad_catalog,
                shared_spawn,
                round_number,
            },
            now,
        );
        Ok(Vec::new())
    }

    /// Empty terminal rounds still own deferred participation until rollover.
    pub(crate) fn reset_empty_gameplay_world(&mut self) {
        if self.match_terminal.is_none() {
            self.reset_objective_world();
        } else {
            self.retire_match_selection();
        }
    }

    pub(crate) fn handle_results_request(
        &mut self,
        request: &Request,
        decoded: Option<&DecodedEvent<'_>>,
        now: Duration,
        wall: u64,
    ) -> Result<Vec<OutboundMessage>, &'static str> {
        let (reservation, release) = match request {
            Request::Event { reservation_id, .. } => (reservation_id, false),
            Request::Release { reservation_id, .. } => (reservation_id, true),
            _ => return Err("results_only_reservation"),
        };
        let cohort_key = self
            .results_cohorts
            .iter()
            .find(|(_, c)| c.actors.contains_key(reservation))
            .map(|(key, _)| key.clone())
            .ok_or("no_reservation")?;
        let cohort = &self.results_cohorts[&cohort_key];
        let actor = &cohort.actors[reservation];
        let mut outgoing = None;
        if let Request::Event {
            world_id,
            player_id,
            transport,
            direction,
            rt_type,
            plugin_id,
            ..
        } = request
        {
            if actor.reservation.world != *world_id || actor.reservation.player != *player_id {
                return Err("identity_mismatch");
            }
            if now.saturating_sub(actor.last_event) >= IDLE_TIMEOUT {
                return Err("results_reservation_expired");
            }
            if matches!(direction, Direction::ClientToServer)
                && *rt_type == 0x3d
                && *plugin_id == Some(1)
            {
                match decoded.map(|d| &d.message) {
                    Some(Ok(protocol::GameMessage::TimeRequest(r))) => {
                        outgoing = Some(
                            protocol::TimeResponse {
                                server_time_ms: now.as_millis() as u32,
                                client_time_ms: r.client_time_ms,
                            }
                            .encode_with_profile(cohort.profile),
                        )
                    }
                    Some(Ok(protocol::GameMessage::PingRequest(r))) => {
                        outgoing = Some(
                            protocol::PingResponse {
                                client_time_ms: r.client_time_ms,
                            }
                            .encode(),
                        )
                    }
                    Some(Ok(protocol::GameMessage::ObjectUpdate(r)))
                        if matches!(transport, Transport::Udp)
                            && r.header.client_index == 0
                            && r.header.nak_sequence != 0 =>
                    {
                        outgoing = actor
                            .replication_history
                            .iter()
                            .find(|wire| {
                                replication::ObjectUpdate::decode_with_profile(wire, cohort.profile)
                                    .is_ok_and(|packet| {
                                        packet.header.sequence == r.header.nak_sequence
                                    })
                            })
                            .cloned();
                    }
                    _ => {} // Results transports never invoke gameplay handlers.
                }
            }
        }
        let messages: Vec<_> = outgoing
            .into_iter()
            .map(|wire| OutboundMessage {
                reservation_id: reservation.clone(),
                world_id: actor.reservation.world,
                player_id: actor.reservation.player,
                plugin_id: 1,
                payload_hex: protocol::encode_hex(&wire),
            })
            .collect();
        self.record(json!({"op":if release {"results_released"} else {"results_event"},"reservation_id":reservation,"match_id":cohort_key,"messages":messages}),wall)?;
        let cohort = self.results_cohorts.get_mut(&cohort_key).unwrap();
        if release {
            cohort.remove_actor(reservation);
            self.retire_reservation_id(reservation);
        } else {
            cohort.actors.get_mut(reservation).unwrap().last_event = now;
        }
        Ok(messages)
    }

    /// Advance only terminal delivery; no simulation, owner reports or combat.
    pub(crate) fn tick_results_cohorts(
        &mut self,
        now: Duration,
        wall: u64,
        mut messages: Vec<OutboundMessage>,
    ) -> Result<Vec<OutboundMessage>, &'static str> {
        let expired: Vec<_> = self
            .results_cohorts
            .iter()
            .flat_map(|(key, c)| {
                c.actors
                    .values()
                    .filter(|a| {
                        a.replication_backlogged || now.saturating_sub(a.last_event) >= IDLE_TIMEOUT
                    })
                    .map(|a| (key.clone(), a.reservation.id.clone()))
            })
            .collect();
        for (key, id) in expired {
            self.record(
                json!({"op":"results_expired","reservation_id":id,"match_id":key}),
                wall,
            )?;
            self.results_cohorts
                .get_mut(&key)
                .unwrap()
                .remove_actor(&id);
            self.retire_reservation_id(&id);
        }
        let keys: Vec<_> = self.results_cohorts.keys().cloned().collect();
        for key in keys {
            let settled = !self.pending_rewards.values().any(|r| r.match_id == key);
            let c = self.results_cohorts.get_mut(&key).unwrap();
            if let Some(end) = &mut c.round.outcome {
                end.finalized |= now >= end.deadline;
            }
            match_flow::queue_round_publications(
                &c.round,
                c.profile,
                c.actors.values_mut().collect(),
                settled,
                now,
                match_flow::ScorePublication::Sabotage {
                    objective_effect: false,
                },
            )?;
            // One FIFO per old actor, independently budgeted and sequenced. Leave
            // heads untouched once the combined bridge budget is consumed.
            let mut planned = Vec::new();
            let mut size: usize = messages.iter().map(|m| m.payload_hex.len() / 2).sum();
            let mut actors: Vec<_> = c.actors.values().filter(|a| a.ready_for_updates).collect();
            actors.sort_unstable_by_key(|a| a.reservation.player.wrapping_sub(c.cursor));
            for a in actors {
                let mut sequence = a.next_sequence()?;
                let notices = a.pending_shared.iter().map(|w| (w, true, true)).chain(
                    a.pending_notifications.iter().map(|n| match n {
                        match_flow::Publication::Application(w) => (w, false, false),
                        match_flow::Publication::Replication(w) => (w, true, false),
                    }),
                );
                for (source, replicated, shared) in notices {
                    let wire = if replicated {
                        replication::multiplayer::for_recipient_with_profile(
                            source, sequence, c.profile,
                        )?
                    } else {
                        source.clone()
                    };
                    if messages.len() + planned.len() >= 128 || size + wire.len() > 24_000 {
                        break;
                    }
                    size += wire.len();
                    if replicated {
                        sequence = sequence
                            .checked_add(1)
                            .ok_or("replication_sequence_exhausted")?;
                    }
                    planned.push((
                        a.reservation.id.clone(),
                        wire,
                        replicated,
                        a.reservation.world,
                        a.reservation.player,
                        shared,
                    ));
                }
            }
            let publications: Vec<_> = planned
                .iter()
                .map(|(id, w, _, world, player, _)| OutboundMessage {
                    reservation_id: id.clone(),
                    world_id: *world,
                    player_id: *player,
                    plugin_id: 1,
                    payload_hex: protocol::encode_hex(w),
                })
                .collect();
            if !publications.is_empty() {
                self.record(
                    json!({"op":"results_publications","match_id":key,"messages":publications}),
                    wall,
                )?;
            }
            let c = self.results_cohorts.get_mut(&key).unwrap();
            for (id, wire, replicated, _, player, shared) in planned {
                let a = c.actors.get_mut(&id).unwrap();
                if replicated {
                    a.remember(wire)?;
                }
                if shared {
                    a.pending_shared.pop_front();
                } else {
                    a.pending_notifications.pop_front();
                }
                c.cursor = player.wrapping_add(1);
            }
            messages.extend(publications);
        }
        self.results_cohorts.retain(|key, c| {
            !c.actors.is_empty() || self.pending_rewards.values().any(|r| r.match_id == *key)
        });
        Ok(messages)
    }
}
