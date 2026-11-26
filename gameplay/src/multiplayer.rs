//! One shared world with independently authenticated actor and recipient state.
use super::*;
use replication::multiplayer::*;

/// Per-recipient reliable shared-frame bound for legacy worlds. A joiner
/// receives its world bootstrap while 64 bots keep publishing; 128 frames
/// evicted joiners on live matches, so allow a larger burst (frames are at most
/// 1397 bytes). Larger worlds raise each recipient's `shared_queue_limit`; see
/// `world_capacity::shared_queue_limit`.
#[cfg(test)]
pub(super) const MAX_SHARED_QUEUE: usize = world_capacity::LEGACY_SHARED_QUEUE;

/// Replies that left shared frames queued because the bridge reply budget
/// (messages and payload bytes per reply, shared by every recipient) was full.
/// The frames stay queued in order and drain in later replies; a recipient
/// whose queue reaches its bound is released as `replication_backlogged`.
/// Every deferral is counted; a receipt reports them at most once per second.
#[derive(Debug, Default)]
pub(crate) struct SharedDeferral {
    pub(crate) replies: u64,
    reported_replies: u64,
    last_report: Option<u64>,
}

impl Active {
    /// Reliable packets cannot be discarded or coalesced without traced ACK rules.
    /// Bound memory now; retire this recipient through recorded lifecycle cleanup
    /// on the next request rather than failing the healthy publisher's operation.
    pub(super) fn queue_shared(&mut self, packet: Vec<u8>) {
        if self.ordinary_shared_len() >= self.shared_queue_limit {
            self.replication_backlogged = true;
        } else if !self.replication_backlogged {
            self.pending_shared.push_back(packet);
        }
    }

    pub(super) fn ordinary_shared_len(&self) -> usize {
        self.pending_shared.len() - self.hospital_bootstrap.len() - self.smoke_bootstrap.len()
    }

    pub(super) fn queue_hospital_bootstrap(&mut self, packets: Vec<Vec<u8>>) {
        if !packets.is_empty() {
            debug_assert!(self.hospital_bootstrap.is_empty());
            debug_assert!(packets.len() <= crate::hospital_turrets::MAX_LATE_JOIN_PACKETS);
            let start = self.pending_shared.len();
            self.hospital_bootstrap = start..start + packets.len();
            self.pending_shared.extend(packets);
        }
    }

    pub(super) fn queue_smoke_bootstrap(&mut self, packets: Vec<Vec<u8>>) {
        if !packets.is_empty() {
            debug_assert!(self.smoke_bootstrap.is_empty());
            debug_assert!(packets.len() <= usize::from(smoke_volume_replication::CAPACITY) * 3);
            let start = self.pending_shared.len();
            self.smoke_bootstrap = start..start + packets.len();
            self.pending_shared.extend(packets);
        }
    }

    pub(super) fn pop_shared(&mut self) {
        if self.pending_shared.pop_front().is_some() {
            self.shared_frames_drained = self.shared_frames_drained.saturating_add(1);
        }
        self.hospital_bootstrap.start = self.hospital_bootstrap.start.saturating_sub(1);
        self.hospital_bootstrap.end = self.hospital_bootstrap.end.saturating_sub(1);
        self.smoke_bootstrap.start = self.smoke_bootstrap.start.saturating_sub(1);
        self.smoke_bootstrap.end = self.smoke_bootstrap.end.saturating_sub(1);
    }

    pub(super) fn apply_peer_damage(
        &mut self,
        generation: u32,
        damage: replication::CharacterDamage,
        now: Duration,
    ) -> Result<(), &'static str> {
        if !self
            .life
            .damage_health()
            .is_some_and(|(g, _)| g == generation)
        {
            return Err("stale_peer_damage");
        }
        self.last_damage = damage;
        if damage.health_quarter_points == 0 {
            self.flash_yaw = None;
            self.revive_state_pending = false;
        }
        self.life = if damage.health_quarter_points == 0 {
            self.healing = None;
            self.pending_damage = None;
            self.static_resupply.clear();
            self.clear_live_projectiles(now)?;
            self.last_exit = None;
            self.pending_spawn_deadline = None;
            self.mounted_seat = None;
            self.mounted_bunker = None;
            self.ground_seat = None;
            self.helo_seat = None;
            CharacterLife::AwaitingRespawn { generation }
        } else {
            CharacterLife::Alive {
                generation,
                health_quarter_points: damage.health_quarter_points,
            }
        };
        Ok(())
    }

    pub(super) fn clear_live_projectiles(&mut self, now: Duration) -> Result<(), &'static str> {
        self.projectiles = projectile::ProjectileFlights::default();
        for removal in self.projectile_replicas.take_all_removals() {
            self.projectile_publications.push(ProjectilePublication {
                include_owner: true,
                wire: removal.encode_with_profile(1, now.as_millis() as u32, self.wire_profile)?,
            });
        }
        // Already detonated objects retain their separate500ms cleanup deadline.
        Ok(())
    }
    fn projectile_removal_packets(&self, now: Duration) -> Result<Vec<Vec<u8>>, &'static str> {
        let mut packets = self
            .projectile_replicas
            .clone()
            .take_all_removals()
            .into_iter()
            .chain(
                self.projectile_cleanup
                    .iter()
                    .flatten()
                    .map(|entry| entry.removal),
            )
            .chain(self.hospital_rocket_flights.removals())
            .chain(self.infantry_rocket_state.removals())
            .map(|removal| {
                removal.encode_with_profile(1, now.as_millis() as u32, self.wire_profile)
            })
            .collect::<Result<Vec<_>, _>>()?;
        packets.extend(
            self.smoke_volumes
                .removal_packets(now.as_millis() as u32, self.wire_profile)?,
        );
        Ok(packets)
    }
    pub(super) fn next_sequence(&self) -> Result<u32, &'static str> {
        self.outbound_sequence
            .checked_add(1)
            .ok_or("replication_sequence_exhausted")
    }
    pub(super) fn remember(&mut self, packet: Vec<u8>) -> Result<(), &'static str> {
        let sequence = replication::ObjectUpdate::decode_with_profile(&packet, self.wire_profile)?
            .header
            .sequence;
        if sequence != self.next_sequence()? {
            return Err("replication_sequence_conflict");
        }
        self.outbound_sequence = sequence;
        self.replication_history.push(packet);
        if self.replication_history.len() > MAX_REPLICATION_HISTORY {
            self.replication_history.remove(0);
        }
        Ok(())
    }
    pub(super) fn snapshot(&self, now: Duration) -> Result<Vec<u8>, &'static str> {
        let claimed = self.claimed.as_ref().ok_or("unclaimed_actor")?;
        let c = &self.reservation.character;
        let character = replication::CharacterCreation {
            object_index: self.reservation.player,
            owner_index: self.reservation.player,
            info: replication::CharacterInfo {
                team: replication::SquadAddress::from_index(self.squad_catalog.squad_index())?
                    .team(),
                faction: c.faction,
                character_class: 21,
                name: c.name.clone(),
                client_index: self.reservation.player,
                squad_index: self.squad_catalog.squad_index(),
                account_id: self.reservation.account,
                np_identity: claimed.np_identity,
            },
            mag_info: self.mag_info(c.progression.unwrap_or_default().level),
        };
        let mut inventory = self.equipped_loadout();
        if let (Some(kit), Some(handle)) = (&mut inventory, self.current_inventory_handle) {
            kit.handle = handle;
            kit.primary.inventory_update_id = handle.update_id;
            if let Some(primary) = self.observed_primary {
                kit.primary = primary;
            }
            kit.primary_equipped = self.primary_equipped;
        }
        let physical = self
            .spawn_delta
            .as_ref()
            .and(self.tracking.sample)
            .map(|sample| replication::CharacterRespawn {
                sequence: 1,
                timestamp_ms: now.as_millis() as u32,
                object_index: self.reservation.player,
                state: if !self.life.is_alive() {
                    replication::CharacterState {
                        state: 7,
                        stance: 0,
                        user_data: [0; 3],
                    }
                } else if let Some(seat) = self.ground_seat {
                    seat.character_state()
                } else if let Some(seat) = self.helo_seat {
                    seat.character_state()
                } else if let Some(seat) = self.mounted_seat {
                    replication::CharacterState {
                        state: 14,
                        stance: 0,
                        user_data: [13, self.mounted_bunker.unwrap_or(0) as u8, seat],
                    }
                } else {
                    self.owner_fields
                        .state
                        .unwrap_or(replication::CharacterState {
                            state: 1,
                            stance: 0,
                            user_data: [0; 3],
                        })
                },
                position: if self.helo_seat.is_some() {
                    domination_helo::mounted_wire_position(sample.position)
                } else {
                    sample.position
                },
                pitch: self.owner_fields.pitch.unwrap_or(127),
                yaw: self.owner_fields.yaw.unwrap_or(0),
                damage: self.last_damage,
            });
        RemoteCharacterSnapshot {
            sequence: 1,
            timestamp_ms: now.as_millis() as u32,
            character: &character,
            inventory: inventory.as_ref(),
            physical: physical.as_ref(),
        }
        .encode_with_profile(self.wire_profile)
    }
}
impl<W: Write> Engine<W> {
    pub fn with_match_limits(
        mut self,
        minimum: usize,
        maximum: usize,
    ) -> Result<Self, &'static str> {
        if minimum == 0
            || minimum > maximum
            || maximum > self.human_admission_capacity_for(self.wire_profile.application_id())
        {
            return Err("invalid_match_limits");
        }
        if self.development_dummy_enabled() && maximum > 1 {
            return Err("dummy_slot_conflicts_with_multiplayer");
        }
        self.validate_team_quorum(self.minimum_players_per_team, maximum)?;
        self.minimum_players = minimum;
        self.maximum_players = maximum;
        self.match_started = self.starts_without_quorum();
        Ok(self)
    }
    pub(super) fn starts_without_quorum(&self) -> bool {
        self.minimum_players == 1 && self.minimum_players_per_team == [0; 2]
    }
    pub(super) fn validate_team_quorum(
        &self,
        minimum: [usize; 2],
        maximum: usize,
    ) -> Result<(), &'static str> {
        if minimum == [0; 2] {
            return Ok(());
        }
        let policy = self.match_profile();
        let team_capacity = |team: usize| {
            self.spawn_catalog.as_ref().map_or(0, |catalog| {
                (catalog
                    .map_pool()
                    .squads
                    .iter()
                    .filter(|s| usize::from(s.team) == team)
                    .count()
                    * policy.squad_capacity)
                    .min(self.human_team_capacity())
            })
        };
        if minimum[0].saturating_add(minimum[1]) > maximum
            || minimum
                .iter()
                .enumerate()
                .any(|(team, n)| *n > team_capacity(team))
            || !policy.auto_assign_teams
            || self.development_dummy_enabled()
        {
            return Err("invalid_team_quorum");
        }
        Ok(())
    }
    /// Only authenticated, current human actors count. Bots/Targets never have
    /// a reservation here. A pending Ready replaces that actor's prior state,
    /// so initial snapshots and the subsequent start transition use one rule.
    pub(super) fn ready_humans_by_team(
        &self,
        now: Duration,
        pending: Option<&Active>,
    ) -> [usize; 2] {
        let mut counts = [0; 2];
        let mut accounts = std::collections::BTreeSet::new();
        let mut owners = std::collections::BTreeSet::new();
        for actor in self.peers.values().chain(self.active.iter()).chain(pending) {
            let is_pending = pending.is_some_and(|p| std::ptr::eq(p, actor));
            if !is_pending && pending.is_some_and(|p| p.reservation.id == actor.reservation.id) {
                continue;
            }
            if actor.claimed.is_none()
                || actor.replication_backlogged
                || now.saturating_sub(actor.last_event) >= IDLE_TIMEOUT
                || (!is_pending && !(actor.ready_for_updates && actor.initial_objects.is_some()))
                || !accounts.insert(actor.reservation.account)
            {
                continue;
            }
            if let Some(owner) = actor.reservation.character.account_id.as_deref()
                && !owners.insert(owner)
            {
                continue;
            }
            counts[usize::from(actor.team())] += 1;
        }
        counts
    }
    pub(super) fn quorum_satisfied(&self, now: Duration, pending: Option<&Active>) -> bool {
        let counts = self.ready_humans_by_team(now, pending);
        counts.iter().sum::<usize>() >= self.minimum_players
            && counts
                .iter()
                .zip(self.minimum_players_per_team)
                .all(|(count, minimum)| *count >= minimum)
    }
    pub(super) fn squad_members(&self, squad: u16, include: Option<u16>) -> [u16; 8] {
        let mut ids: Vec<u16> = self
            .peers
            .values()
            .filter(|p| p.initial_objects.is_some() && p.squad_catalog.squad_index() == squad)
            .map(|p| p.reservation.player)
            .collect();
        if let Some(p) = include {
            ids.push(p);
        } else if let Some(p) = self
            .active
            .as_ref()
            .filter(|p| p.initial_objects.is_some() && p.squad_catalog.squad_index() == squad)
        {
            ids.push(p.reservation.player);
        }
        // The one-bot proof has no authenticated bot peer. Keep its committed
        // membership when an ordinary squad snapshot/roster is rebuilt.
        if self.maximum_players == 1 {
            for actor in self.active.iter().chain(self.peers.values()) {
                if actor.dummy_created
                    && let Some(target) = &actor.development_target
                    && target.script.is_some()
                    && target.roster_published
                    && target.squad.squad_index() == squad
                {
                    ids.push(dummy::OBJECT_INDEX);
                }
            }
        }
        ids.extend(self.bot_registry.members(squad));
        ids.sort_unstable();
        ids.dedup();
        let mut members = [u16::MAX; 8];
        for (slot, id) in members.iter_mut().zip(ids) {
            *slot = id;
        }
        members
    }
    pub(super) fn apply_shared(
        &mut self,
        request: Request,
        decoded: Option<&DecodedEvent<'_>>,
        now: Duration,
        wall: u64,
    ) -> Result<Vec<OutboundMessage>, &'static str> {
        if let Request::SelectMatchProfile {
            formation_id,
            profile_id,
            ..
        } = request
        {
            return self.select_match_profile(formation_id, profile_id, wall);
        }
        if matches!(&request, Request::Reserve { .. })
            && self.match_profiles.is_some()
            && self.selected_match.is_none()
        {
            return Err("match_profile_not_selected");
        }
        if let Some(reservation_id) = match &request {
            Request::Event { reservation_id, .. }
            | Request::Release { reservation_id, .. }
            | Request::DevelopmentDamage { reservation_id, .. }
            | Request::DevelopmentArmDemolition { reservation_id, .. }
            | Request::DevelopmentSpawnTarget { reservation_id, .. }
            | Request::DevelopmentMoveBot { reservation_id, .. }
            | Request::DevelopmentStructureDamage { reservation_id, .. } => Some(reservation_id),
            _ => None,
        } && self.results_owner(reservation_id).is_some()
        {
            let messages = self.handle_results_request(&request, decoded, now, wall)?;
            return if self.world_tick_enabled {
                Ok(messages)
            } else {
                self.tick_results_cohorts(now, wall, messages)
            };
        }
        if let Request::Release { reservation_id, .. } = &request
            && self.released_reservations.contains(reservation_id)
        {
            self.record(
                json!({"op":"released_replay","reservation_id":reservation_id}),
                wall,
            )?;
            return Ok(Vec::new());
        }
        if let Request::Release { reservation_id, .. } = &request
            && !self
                .active
                .as_ref()
                .is_some_and(|a| a.reservation.id == *reservation_id)
            && !self.peers.contains_key(reservation_id)
        {
            // A cancel may overtake its in-flight reserve on another control
            // connection. A successful release is definitive for that identity.
            self.record(
                json!({"op":"released_before_admission","reservation_id":reservation_id}),
                wall,
            )?;
            self.retire_reservation_id(reservation_id);
            return Ok(Vec::new());
        }
        if let Request::Reserve { world_id, .. } = &request
            && self
                .results_cohorts
                .values()
                .any(|c| c.terminal.world_id != *world_id)
        {
            return Err("reservation_conflict");
        }
        if let Request::Reserve { reservation_id, .. } = &request {
            if self.released_reservations.contains(reservation_id) {
                return Err("reservation_id_retired");
            }
            if self.results_owner(reservation_id).is_some() {
                return Err("reservation_id_retired");
            }
            if self.match_terminal.is_some() {
                if self.match_profiles.is_some() {
                    return Err("match_finished");
                }
                return self.reserve_fixed_cohort(request, now, wall);
            }
        }
        if matches!(&request, Request::RefreshProgression { .. }) {
            return self.refresh_progression(request, now, wall);
        }
        if matches!(&request, Request::RefreshLoadouts { .. }) {
            return self.refresh_loadouts(request, now, wall);
        }
        if matches!(&request, Request::LoadoutSelectionSettled { .. }) {
            return self.settle_loadout_selection(request, wall);
        }
        if matches!(&request, Request::ProgressionCommitted { .. }) {
            return self.acknowledge_progression(request, now, wall);
        }
        // Private move commands queue state only. Their admin response is not a
        // human transport, so leave clock/replication draining to normal events.
        if matches!(&request, Request::DevelopmentMoveBot { .. }) {
            return self.apply(request, decoded, now, wall);
        }
        let development_demolition = matches!(&request, Request::DevelopmentArmDemolition { .. });
        if self.maximum_players == 1 && self.minimum_players_per_team == [0; 2] {
            let was_created = self
                .active
                .as_ref()
                .is_some_and(|a| a.initial_objects.is_some());
            let applied = self.apply(request, decoded, now, wall);
            let is_created = self
                .active
                .as_ref()
                .is_some_and(|a| a.initial_objects.is_some());
            // The single-player path skips shared actor publication below. It
            // still needs native promotion on entry and removal on release/expiry,
            // including expiry followed by a rejected event from the old client.
            if self.native_leader_election() && was_created != is_created {
                self.queue_roster(now)?;
            }
            let mut messages = applied?;
            // The private admin socket cannot deliver client publications. Keep
            // the seeded snapshot queued for the next normal bridge request.
            if development_demolition {
                return Ok(messages);
            }
            if !self.world_tick_enabled {
                messages.extend(self.tick_objectives(now, wall)?);
                self.queue_squad_orders(now, wall)?;
                messages.extend(self.tick_match_clock(now, wall)?);
                messages.extend(self.tick_hospital_rockets(now, wall)?);
                self.queue_projectile_publications(now)?;
            }
            return self.drain_shared(messages, wall);
        }
        if self.development_dummy_enabled() {
            return Err("dummy_slot_conflicts_with_multiplayer");
        }
        let id = match &request {
            Request::Hello { .. }
            | Request::SelectMatchProfile { .. }
            | Request::ReserveParty { .. }
            | Request::ReleaseParty { .. }
            | Request::ConfigureMatch { .. }
            | Request::ProgressionCommitted { .. }
            | Request::RefreshProgression { .. }
            | Request::LoadoutSelectionSettled { .. }
            | Request::RefreshLoadouts { .. } => None,
            Request::Reserve { reservation_id, .. }
            | Request::Event { reservation_id, .. }
            | Request::DevelopmentSpawnTarget { reservation_id, .. }
            | Request::DevelopmentMoveBot { reservation_id, .. }
            | Request::DevelopmentDamage { reservation_id, .. }
            | Request::DevelopmentArmDemolition { reservation_id, .. }
            | Request::DevelopmentStructureDamage { reservation_id, .. }
            | Request::Release { reservation_id, .. } => Some(reservation_id.clone()),
        };
        // The active slot is only the current request's actor. Other participants
        // stay in the same engine, sharing geometry and world clocks.
        if let Some(actor) = self.active.take() {
            self.peers.insert(actor.reservation.id.clone(), actor);
        }
        self.expire_world_peers(now, wall)?;
        if let Request::Reserve {
            reservation_id,
            world_id,
            player_id,
            account_id,
            character,
            ..
        } = &request
            && !self.peers.contains_key(reservation_id)
        {
            if self.peers.len() >= self.maximum_players {
                return Err("match_full");
            }
            if usize::from(*player_id) >= self.maximum_players {
                return Err("invalid_player_slot");
            }
            let policy = self.match_profile();
            if !(character.faction == policy.defending_faction
                || (policy.auto_assign_teams && character.faction == policy.attacking_faction))
                || character.universe_id != 26
            {
                return Err("unsupported_match_character");
            }
            if self.peers.values().any(|p| {
                p.reservation.world != *world_id
                    || p.reservation.player == *player_id
                    || p.reservation.account == *account_id
            }) {
                return Err("reservation_conflict");
            }
        }
        let native_event = matches!(
            &request,
            Request::Event {
                rt_type: 0x3d,
                plugin_id: Some(1),
                ..
            }
        );
        let owner_report = decoded
            .filter(|_| native_event)
            .and_then(DecodedEvent::owner_update)
            .filter(|(update, _)| {
                update.header.nak_sequence == 0 && update.header.client_index == 0
            })
            .and_then(|(update, report)| {
                report
                    .character
                    .clone()
                    .map(|fields| (update.header.sequence, fields))
            });
        if let Some(id) = id.as_ref() {
            self.active = self.peers.remove(id);
        }
        let was_created = self
            .active
            .as_ref()
            .is_some_and(|a| a.initial_objects.is_some());
        let had_helo_creation = self
            .active
            .as_ref()
            .is_some_and(|actor| actor.helo_creation_sent);
        let before_sequence = self.active.as_ref().map_or(0, |a| a.outbound_sequence);
        let had_objectives = self
            .active
            .as_ref()
            .is_some_and(|a| a.objective_delta.is_some());
        let before_owner = self.active.as_ref().map_or(0, |a| a.latest_owner_sequence);
        let before_alive_generation = self
            .active
            .as_ref()
            .and_then(|a| a.life.damage_health().map(|(g, _)| g));
        let had_emplacement = self
            .active
            .as_ref()
            .is_some_and(|a| a.emplacement_creation.is_some());
        let is_release = matches!(&request, Request::Release { .. });
        let released_driver = self
            .active
            .as_ref()
            .filter(|_| is_release)
            .and_then(|actor| self.ground_driver_seat_index(actor));
        let released_gunner = self
            .active
            .as_ref()
            .filter(|actor| is_release && self.is_ground_gunner(actor))
            .and_then(|actor| actor.ground_seat)
            .map(|seat| {
                let index = seat.vehicle_index;
                self.ground_turret_revocation_packet(index, now)
                    .map(|packets| (index, packets))
            })
            .transpose()?;
        let removed = if is_release {
            self.active
                .as_ref()
                .filter(|a| a.initial_objects.is_some())
                .map(|a| {
                    a.projectile_removal_packets(now).map(|packets| {
                        (
                            a.reservation.player,
                            a.mounted_seat.map(|_| a.mounted_bunker.unwrap_or(0)),
                            packets,
                        )
                    })
                })
                .transpose()?
        } else {
            None
        };
        if matches!(&request, Request::Release { .. })
            && self.active.is_none()
            && !self.peers.is_empty()
        {
            return Err("identity_mismatch");
        }
        let incoming_human = match &request {
            Request::Reserve { player_id, .. } => Some(*player_id),
            _ => None,
        };
        let mut messages = self.apply(request, decoded, now, wall)?;
        if is_release {
            self.revoke_inactive_hospital_turrets();
        }
        if let Some((index, packets)) = released_gunner {
            self.revoke_ground_gunner(index);
            for actor in self
                .peers
                .values_mut()
                .filter(|actor| actor.initial_objects.is_some())
            {
                for packet in &packets {
                    actor.queue_shared(packet.clone());
                }
            }
        }
        if let Some(index) = released_driver {
            self.queue_ground_driver_revocation(index, now)?;
        }
        if development_demolition {
            return Ok(messages);
        }
        if let Some(index) = incoming_human {
            self.retire_bot_for_human(index, now, wall)?;
        }
        if let Some(active) = self.active.as_ref() {
            if active.claimed.is_some() {
                self.world_epoch.get_or_insert(now);
            }
            if let Some(epoch) = active.match_clock_started {
                self.match_epoch.get_or_insert(epoch);
            }
        }
        let created = self
            .active
            .as_ref()
            .is_some_and(|a| a.initial_objects.is_some())
            && !was_created;
        if created {
            let new_id = id.as_ref().unwrap();
            // A scheduler can leave activations staged between requests. Old
            // recipients need those originals, while this new recipient gets
            // only the current snapshot below, once.
            self.queue_projectile_publications_except(now, Some(new_id))?;
            let snapshot = self.active.as_ref().unwrap().snapshot(now)?;
            let existing: Vec<Vec<u8>> = self
                .peers
                .values()
                .filter(|a| a.initial_objects.is_some())
                .map(|a| {
                    let mut packets = vec![a.snapshot(now)?];
                    packets.extend(
                        a.projectile_replicas
                            .snapshot_packets(now.as_millis() as u32)?,
                    );
                    packets.extend(
                        a.hospital_rocket_flights
                            .snapshot_packets(now.as_millis() as u32, self.wire_profile)?,
                    );
                    packets.extend(
                        a.infantry_rocket_state
                            .snapshot_packets(now.as_millis() as u32, self.wire_profile)?,
                    );
                    Ok(packets)
                })
                .collect::<Result<Vec<Vec<Vec<u8>>>, &'static str>>()?
                .into_iter()
                .flatten()
                .collect();
            let smoke_snapshot = self
                .peers
                .values()
                .filter(|actor| actor.initial_objects.is_some())
                .map(|actor| {
                    actor.smoke_volumes.snapshot_packets(
                        now.as_millis() as u32,
                        now,
                        self.wire_profile,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .flatten()
                .collect();
            for peer in self
                .peers
                .values_mut()
                .filter(|a| a.initial_objects.is_some())
            {
                peer.queue_shared(snapshot.clone());
            }
            // Remote snapshots can already reference Helos. Deliver the
            // type-2 instances to this new peer before those snapshots.
            if let Some(fleet) = self
                .domination_gameplay
                .as_ref()
                .and_then(|world| world.state.helo.as_ref())
            {
                let carriers = fleet
                    .carriers
                    .iter()
                    .map(|carrier| carrier.creation(2, now.as_millis() as u32))
                    .collect::<Vec<_>>();
                let packet =
                    domination_helo::HeloCreation::encode_list(&carriers, self.wire_profile)?;
                let actor = self.active.as_mut().unwrap();
                actor.queue_shared(packet);
                actor.helo_creation_sent = true;
            }
            for packet in existing {
                self.active.as_mut().unwrap().queue_shared(packet);
            }
            self.active
                .as_mut()
                .unwrap()
                .queue_smoke_bootstrap(smoke_snapshot);
            self.record(
                json!({"op":"shared_actor_created","reservation_id":new_id}),
                wall,
            )?;
            self.note_leadership_character_created();
            self.queue_roster(now)?;
        } else if let Some(actor) = self.active.as_ref()
            && actor.outbound_sequence > before_sequence
        {
            let packet = actor.replication_history.last().unwrap().clone();
            let mask = replication::ObjectUpdate::decode_with_profile(&packet, self.wire_profile)?
                .header
                .object_type_mask;
            // Per-character spawn assignment remains private; shared positions,
            // life, vehicles, deadlines and objectives are published to peers.
            let own_helo_creation =
                mask == 1 << 2 && !had_helo_creation && actor.helo_creation_sent;
            // Multi-packet objective bootstrap belongs to the joining recipient.
            // Relaying the last history packet would send only its private suffix.
            let own_objective_bootstrap = mask == 1 << 12
                && !had_objectives
                && actor.objective_delta.is_some()
                && actor.outbound_sequence - before_sequence > 1;
            if mask != ((1 << 3) | (1 << 7) | (1 << 8) | (1 << 13))
                && !own_helo_creation
                && !own_objective_bootstrap
                && (mask != 8 || !is_spawn_assignment(&packet, self.wire_profile))
            {
                for peer in self
                    .peers
                    .values_mut()
                    .filter(|a| a.initial_objects.is_some())
                {
                    peer.queue_shared(packet.clone());
                }
            }
        }
        if native_event
            && let Some(actor) = self.active.as_ref()
            && let Some((update, report)) = decoded.and_then(DecodedEvent::owner_update)
            && update.header.client_index == 0
            && update.header.nak_sequence == 0
            && update.header.sequence > before_owner
            && actor.latest_owner_sequence == update.header.sequence
            && self.alpha_turret_authority(actor).is_ok()
            && let Some(fields) = report.turret.as_ref().filter(|f| f.has_changes())
        {
            let packet =
                fields.encode_update_with_profile(1, now.as_millis() as u32, self.wire_profile)?;
            for peer in self
                .peers
                .values_mut()
                .filter(|a| a.initial_objects.is_some())
            {
                peer.queue_shared(packet.clone());
            }
        }
        if !had_emplacement
            && self
                .active
                .as_ref()
                .is_some_and(|a| a.emplacement_creation.is_some())
            && (self.alpha_turret.yaw.is_some()
                || self.alpha_turret.pitch.is_some()
                || self.alpha_turret.primary.is_some()
                || self.alpha_turret.secondary.is_some()
                || self.alpha_turret.overheat_info.is_some())
        {
            let packet = self.alpha_turret.encode_update_with_profile(
                1,
                now.as_millis() as u32,
                self.wire_profile,
            )?;
            self.active.as_mut().unwrap().queue_shared(packet);
        }
        if is_release
            && self
                .alpha_turret_reporter
                .as_ref()
                .is_some_and(|(reporter, _)| Some(reporter) == id.as_ref())
        {
            self.alpha_turret_reporter = None;
        }
        if let Some((player, bunker_index, projectiles)) = removed {
            for packet in projectiles {
                for peer in self
                    .peers
                    .values_mut()
                    .filter(|a| a.initial_objects.is_some())
                {
                    peer.queue_shared(packet.clone());
                }
            }
            let packet = CharacterRemoval {
                sequence: 1,
                timestamp_ms: now.as_millis() as u32,
                player,
            }
            .encode_with_profile(self.wire_profile)?;
            for peer in self
                .peers
                .values_mut()
                .filter(|a| a.initial_objects.is_some())
            {
                peer.queue_shared(packet.clone());
            }
            self.queue_roster(now)?;
            if let Some(index) = bunker_index {
                self.queue_bunker_ownership(index, now)?;
            }
        }
        let owner_report = owner_report.and_then(|(seq, fields)| {
            self.active.as_ref().map(|a| {
                let mut fields = a.owner_presentation(&fields);
                if a.tracking.correction_pending() {
                    fields.position = None;
                }
                (seq, fields)
            })
        });
        if let Some((sequence, fields)) = owner_report
            && sequence > before_owner
            && self.active.as_ref().is_some_and(|a| {
                // A report from before spawn must not overwrite the new life
                // with its old pose or inventory after the authoritative packet.
                a.life
                    .damage_health()
                    .is_some_and(|(generation, _)| Some(generation) == before_alive_generation)
                    && a.ready_for_updates
                    && !a.is_mounted()
            })
            && (fields.state.is_some()
                || fields.position.is_some()
                || fields.pitch.is_some()
                || fields.yaw.is_some()
                || fields.primary.is_some()
                || fields.secondary.is_some()
                || fields.primary_equipped.is_some())
        {
            let packet = CharacterOwnerDelta {
                sequence: 1,
                timestamp_ms: now.as_millis() as u32,
                player: self.active.as_ref().unwrap().reservation.player,
                fields: &fields,
            }
            .encode_with_profile(self.wire_profile)?;
            for peer in self
                .peers
                .values_mut()
                .filter(|a| a.initial_objects.is_some())
            {
                peer.queue_shared(packet.clone());
            }
        }
        if !self.match_started && self.quorum_satisfied(now, None) {
            self.record(json!({"op":"match_started","ready_players":self.ready_humans_by_team(now, None).iter().sum::<usize>(),"ready_players_per_team":self.ready_humans_by_team(now, None),"minimum_players":self.minimum_players,"minimum_players_per_team":self.minimum_players_per_team}),wall)?;
            self.match_started = true;
            let next_waves = [
                self.team_waves[0]
                    .map(|clock| clock.next_after(now))
                    .transpose()?,
                self.team_waves[1]
                    .map(|clock| clock.next_after(now))
                    .transpose()?,
            ];
            let mut deadline_packets = [None, None];
            for team in [0_u8, 1] {
                deadline_packets[usize::from(team)] = next_waves[usize::from(team)]
                    .map(|deadline| {
                        replication::TeamSpawnDeadline {
                            sequence: 1,
                            timestamp_ms: now.as_millis() as u32,
                            object_index: 0,
                            team,
                            deadline_ms: deadline.as_millis() as u32,
                        }
                        .encode_with_profile(self.wire_profile)
                    })
                    .transpose()?;
            }
            let packet = MatchPhaseUpdate {
                sequence: 1,
                timestamp_ms: now.as_millis() as u32,
                phase: replication::MatchPhase::Playing,
            }
            .encode_with_profile(self.wire_profile)?;
            for actor in self
                .peers
                .values_mut()
                .chain(self.active.iter_mut())
                .filter(|a| a.initial_objects.is_some())
            {
                actor.queue_shared(packet.clone());
                let team = usize::from(actor.team());
                if let Some(deadline) = next_waves[team] {
                    if actor.pending_spawn_deadline.is_some_and(|d| d <= now) {
                        actor.pending_spawn_deadline = Some(deadline);
                    }
                    actor.published_wave_deadline = Some(deadline);
                    actor.queue_shared(deadline_packets[team].as_ref().unwrap().clone());
                }
            }
            // Native StartGame runs FullBattleFieldPromotion before play.
            if self.native_leader_election() {
                self.queue_roster(now)?;
            }
        }
        // A reserve is a control request. Horizon's bridge codec accepts reliable
        // messages only on event and release replies and rejects any other reply
        // that carries them as malformed, which refused a second human's join
        // whenever another ready human had packets queued. Leave peers' queues to
        // the world stream or their next event instead of piggybacking on admission.
        if incoming_human.is_some() {
            return Ok(messages);
        }
        if !self.world_tick_enabled {
            self.queue_team_wave_deadlines(now, wall)?;
            messages.extend(self.tick_objectives(now, wall)?);
            self.queue_squad_orders(now, wall)?;
            messages.extend(self.tick_match_clock(now, wall)?);
            messages.extend(self.tick_hospital_rockets(now, wall)?);
            self.queue_projectile_publications(now)?;
        }
        self.drain_shared(messages, wall)
    }
    /// The scheduler and event path share expiry; no fabricated client event.
    pub(super) fn expire_world_peers(
        &mut self,
        now: Duration,
        wall: u64,
    ) -> Result<Vec<OutboundMessage>, &'static str> {
        let expired: Vec<_> = self
            .peers
            .iter()
            .chain(self.active.iter().map(|a| (&a.reservation.id, a)))
            .filter(|(_, a)| {
                a.replication_backlogged || now.saturating_sub(a.last_event) >= IDLE_TIMEOUT
            })
            .map(|(id, _)| id.clone())
            .collect();
        let any_expired = !expired.is_empty();
        let mut expired_bunkers = std::collections::BTreeSet::new();
        for id in expired {
            let actor = self
                .peers
                .get(&id)
                .or_else(|| self.active.as_ref().filter(|a| a.reservation.id == id))
                .unwrap();
            if actor.replication_backlogged {
                // Name the full queue so a live eviction can be attributed.
                self.record(
                    json!({"op":"replication_backlog_released","reservation_id":id,
                        "pending_shared":actor.ordinary_shared_len(),
                        "pending_notifications":actor.pending_notifications.len(),
                        "ready_for_updates":actor.ready_for_updates}),
                    wall,
                )?;
            } else {
                self.record(json!({"op":"expired","reservation_id":id}), wall)?;
            }
            // Horizon learns of this eviction only through its next bridge call.
            // Retiring the identity makes its later release replay as acknowledged
            // instead of failing as an identity mismatch while other peers remain,
            // which would otherwise leave the account's release pending in Horizon.
            self.retire_reservation_id(&id);
            let removed = self
                .peers
                .remove(&id)
                .or_else(|| {
                    if self.active.as_ref().is_some_and(|a| a.reservation.id == id) {
                        self.active.take()
                    } else {
                        None
                    }
                })
                .unwrap();
            self.remember_round_participant(&removed);
            if let Some(index) = self.ground_driver_seat_index(&removed) {
                self.queue_ground_driver_revocation(index, now)?;
            }
            if self
                .alpha_turret_reporter
                .as_ref()
                .is_some_and(|(id, _)| *id == removed.reservation.id)
            {
                self.alpha_turret_reporter = None;
            }
            if removed.mounted_seat.is_some() {
                expired_bunkers.insert(removed.mounted_bunker.unwrap_or(0));
            }
            if removed.initial_objects.is_some() {
                for packet in removed.projectile_removal_packets(now)? {
                    for peer in self
                        .peers
                        .values_mut()
                        .chain(self.active.iter_mut())
                        .filter(|a| a.initial_objects.is_some())
                    {
                        peer.queue_shared(packet.clone());
                    }
                }
                let packet = CharacterRemoval {
                    sequence: 1,
                    timestamp_ms: now.as_millis() as u32,
                    player: removed.reservation.player,
                }
                .encode_with_profile(self.wire_profile)?;
                for peer in self.peers.values_mut().chain(self.active.iter_mut()) {
                    peer.queue_shared(packet.clone());
                }
            }
        }
        if any_expired {
            self.queue_roster(now)?;
            for index in &expired_bunkers {
                self.queue_bunker_ownership(*index, now)?;
            }
        }
        if any_expired && self.peers.is_empty() && self.active.is_none() {
            self.clear_bot_world();
            self.reset_empty_gameplay_world();
            self.team_waves = [None; 2];
            self.last_published_rosters.clear();
            self.shared_drain_cursor = 0;
            self.world_epoch = None;
            self.match_epoch = None;
            self.alpha_turret = turret::ClientTurretFields::default();
            self.alpha_turret_reporter = None;
            self.match_started = self.starts_without_quorum();
        }
        Ok(Vec::new())
    }
    /// Slot reuse also waits for previously queued cleanup packets to enter every
    /// recipient's immutable history, including when the32-message budget defers them.
    pub(super) fn projectile_cleanup_pending(&self, owner: &Active, slot: u8) -> bool {
        owner
            .projectile_cleanup
            .get(usize::from(slot))
            .is_some_and(Option::is_some)
            || self
                .peers
                .values()
                .chain(self.active.iter())
                .any(|recipient| {
                    recipient.pending_shared.iter().any(|packet| {
                        replication::ObjectUpdate::decode_with_profile(packet, self.wire_profile)
                            .is_ok_and(|update| {
                                projectile_replication::ProjectileRemoval {
                                    slot,
                                    object_index: owner.reservation.player,
                                }
                                .encode_with_profile(
                                    update.header.sequence,
                                    update.header.timestamp_ms,
                                    self.wire_profile,
                                )
                                .is_ok_and(|cleanup| cleanup == *packet)
                            })
                    })
                })
    }
    pub(super) fn queued_projectile_terminals(&self) -> std::collections::BTreeSet<(u8, u16)> {
        self.peers
            .values()
            .chain(self.active.iter())
            .flat_map(|recipient| &recipient.pending_shared)
            .filter_map(|wire| {
                replication::ObjectUpdate::decode_with_profile(wire, self.wire_profile).ok()
            })
            .filter_map(|update| {
                projectile::decode_projectile_updates_with_profile(&update, self.wire_profile).ok()
            })
            .flatten()
            .filter(|report| report.fields.state == Some(2))
            .map(|report| (report.slot, report.object_index))
            .collect()
    }
    /// This runs after admission: a launch on the cleanup deadline is denied;
    /// the same request flushes removal, permitting the next launch request.
    pub(crate) fn queue_projectile_publications(
        &mut self,
        now: Duration,
    ) -> Result<(), &'static str> {
        self.queue_projectile_publications_except(now, None)
    }

    fn queue_projectile_publications_except(
        &mut self,
        now: Duration,
        excluded: Option<&str>,
    ) -> Result<(), &'static str> {
        // Hold cleanup for the slowest recipient. This check runs before drain:
        // the last queued state2 grants500ms from its actual delivery request,
        // even when the32-message budget deferred it past the original deadline.
        let queued_terminals = self.queued_projectile_terminals();
        let held_until = (!queued_terminals.is_empty())
            .then(|| {
                now.checked_add(Duration::from_millis(500))
                    .ok_or("projectile_cleanup_time_overflow")
            })
            .transpose()?;
        let mut publications = Vec::new();
        for actor in self.peers.values_mut().chain(self.active.iter_mut()) {
            for entry in &mut actor.projectile_cleanup {
                if let Some(cleanup) = entry.as_mut()
                    && queued_terminals
                        .contains(&(cleanup.removal.slot, cleanup.removal.object_index))
                {
                    cleanup.due = cleanup.due.max(held_until.unwrap());
                }
                if entry.as_ref().is_some_and(|cleanup| cleanup.due <= now) {
                    let cleanup = entry.take().unwrap();
                    actor.projectile_publications.push(ProjectilePublication {
                        include_owner: true,
                        wire: cleanup.removal.encode_with_profile(
                            1,
                            now.as_millis() as u32,
                            self.wire_profile,
                        )?,
                    });
                }
            }
            for volume in actor.smoke_volumes.take_due(now) {
                actor.projectile_publications.push(ProjectilePublication {
                    include_owner: true,
                    wire: smoke_volume_replication::VolumeRemoval {
                        object_index: volume.object_index,
                    }
                    .encode_with_profile(
                        1,
                        now.as_millis() as u32,
                        self.wire_profile,
                    )?,
                });
            }
            for publication in actor.projectile_publications.drain(..) {
                publications.push((actor.reservation.id.clone(), publication));
            }
        }
        for (owner, publication) in publications {
            for recipient in self
                .peers
                .values_mut()
                .chain(self.active.iter_mut())
                .filter(|a| {
                    a.initial_objects.is_some() && excluded != Some(a.reservation.id.as_str())
                })
            {
                if publication.include_owner || recipient.reservation.id != owner {
                    recipient.queue_shared(publication.wire.clone());
                }
            }
        }
        Ok(())
    }
    /// Validate the final direct replies before any owner-event record/commit.
    /// Shared FIFO planning uses precisely the same admission rule below.
    pub(super) fn reply_budget(
        &self,
        messages: &[OutboundMessage],
    ) -> Result<(usize, usize), &'static str> {
        let limit = if self.maximum_players > 8 { 128 } else { 32 };
        let total = messages
            .iter()
            .try_fold(0usize, |total, message| {
                total.checked_add(message.payload_hex.len() / 2)
            })
            .ok_or("shared_reply_budget_exceeded")?;
        if messages.len() > limit || total > 24_000 {
            return Err("shared_reply_budget_exceeded");
        }
        Ok((limit, total))
    }

    pub(super) fn drain_shared(
        &mut self,
        mut messages: Vec<OutboundMessage>,
        wall: u64,
    ) -> Result<Vec<OutboundMessage>, &'static str> {
        // Preserve reliable FIFO packets and recipient sequence numbers. A pass
        // takes one packet per recipient so a busy low slot cannot starve peers.
        // Larger match admission explicitly opts into the expanded message count;
        // the aggregate raw-byte budget is the same for every match size.
        let (limit, mut total) = self.reply_budget(&messages)?;
        self.publish_inherited_leadership(wall)?;
        let mut planned = Vec::new();
        // Shared character frames can already refer to an indexed hospital.
        // Its direct creation reply precedes the existing FIFO and bootstrap.
        let requires_hospital_creation = self.has_hospital_turrets();
        let mut ready: Vec<_> = self
            .peers
            .values()
            .chain(self.active.iter())
            .filter(|a| {
                a.ready_for_updates
                    && (!a.pending_shared.is_empty()
                        || self
                            .bot_registry
                            .stats_publication
                            .pending(a, &self.bot_registry.bots)
                        || self.human_assists_pending(a))
                    && self.ground_vehicle_creation_complete(a.ground_vehicle_batch)
                    && (!requires_hospital_creation || a.emplacement_creation.is_some())
            })
            .collect();
        ready.sort_unstable_by_key(|a| a.reservation.player);
        if !ready.is_empty() {
            let start = ready.partition_point(|a| a.reservation.player < self.shared_drain_cursor);
            let count = ready.len();
            ready.rotate_left(start % count);
        }
        let mut offsets = vec![0; ready.len()];
        let mut sequences = ready
            .iter()
            .map(|a| a.outbound_sequence)
            .collect::<Vec<_>>();
        let mut scheduling = ready
            .iter()
            .map(|a| (a.statistics_cursor, a.prefer_statistics))
            .collect::<Vec<_>>();
        let mut selected_stats = vec![std::collections::BTreeSet::new(); ready.len()];
        let mut next_cursor = self.shared_drain_cursor;
        'drain: loop {
            let mut progressed = false;
            for (index, actor) in ready.iter().enumerate() {
                if messages.len() + planned.len() >= limit {
                    break 'drain;
                }
                let candidate = self.next_shared_statistic(
                    actor,
                    actor
                        .shared_frames_drained
                        .saturating_add(offsets[index] as u64),
                    scheduling[index].0,
                    &selected_stats[index],
                );
                let stat_packet = candidate
                    .map(|candidate| candidate.encode(self, actor.wire_profile))
                    .transpose()?;
                let shared = actor.pending_shared.get(offsets[index]);
                // Alternate eligible application rows with reliable FIFO frames.
                // A busy scoreboard and a busy replication stream both advance.
                let prefer_stat = scheduling[index].1 || shared.is_none();
                let mut chosen = None;
                for statistics in [prefer_stat, !prefer_stat] {
                    if statistics {
                        if let Some(packet) = &stat_packet
                            && total + packet.len() <= 24_000
                        {
                            chosen = Some((packet.clone(), candidate, 0));
                            break;
                        }
                    } else if let Some(wire) = shared {
                        let sequence = sequences[index]
                            .checked_add(1)
                            .ok_or("replication_sequence_exhausted")?;
                        let (packet, frames) = self.next_shared_packet(
                            actor,
                            offsets[index],
                            wire,
                            sequence,
                            24_000 - total,
                        )?;
                        if total + packet.len() <= 24_000 {
                            chosen = Some((packet, None, frames));
                            break;
                        }
                    }
                }
                let Some((packet, stat, frames)) = chosen else {
                    continue;
                };
                total += packet.len();
                planned.push((actor.reservation.id.clone(), packet, stat, frames));
                if let Some(stat) = stat {
                    selected_stats[index].insert(stat.slot());
                    scheduling[index] = (stat.slot().wrapping_add(1), false);
                } else {
                    offsets[index] += frames;
                    sequences[index] += 1;
                    scheduling[index].1 = true;
                }
                next_cursor = actor.reservation.player.wrapping_add(1);
                progressed = true;
            }
            if !progressed {
                break;
            }
        }
        // Whatever the budget left behind, per recipient: (queued, bound).
        let deferred: Vec<_> = ready
            .iter()
            .zip(&offsets)
            .map(|(actor, offset)| {
                (
                    actor.pending_shared.len().saturating_sub(*offset),
                    actor.shared_queue_limit,
                )
            })
            .filter(|(queued, _)| *queued > 0)
            .collect();
        let recipients: Vec<_> = planned
            .iter()
            .map(|(id, wire, _, _)| {
                let actor = self
                    .active
                    .as_ref()
                    .filter(|a| a.reservation.id == *id)
                    .or_else(|| self.peers.get(id))
                    .unwrap();
                OutboundMessage {
                    reservation_id: id.clone(),
                    world_id: actor.reservation.world,
                    player_id: actor.reservation.player,
                    plugin_id: 1,
                    payload_hex: protocol::encode_hex(wire),
                }
            })
            .collect();
        if !recipients.is_empty() {
            self.record(
                json!({"op":"shared_replication","messages":recipients,"next_drain_cursor":next_cursor}),
                wall,
            )?;
        }
        for (id, packet, stat, frames) in planned {
            if let Some(stat) = stat {
                self.commit_shared_statistic(&id, stat);
                continue;
            }
            let actor = if self.active.as_ref().is_some_and(|a| a.reservation.id == id) {
                self.active.as_mut().unwrap()
            } else {
                self.peers.get_mut(&id).unwrap()
            };
            actor.prefer_statistics = true;
            // A joined update never carries the recipient's own Character.
            let correction = (frames == 1)
                .then(|| {
                    crate::correction_ack::owner_position_packet(
                        &packet,
                        actor.reservation.player,
                        actor.wire_profile,
                    )
                })
                .flatten();
            actor.remember(packet)?;
            if let Some((sequence, position)) = correction {
                actor.tracking.bind_correction(sequence, position);
            }
            for _ in 0..frames {
                actor.pop_shared();
            }
            if actor.pending_shared.is_empty() {
                actor.ordered_health_pending = false;
            }
        }
        self.shared_drain_cursor = next_cursor;
        messages.extend(recipients);
        if !deferred.is_empty() {
            self.report_shared_deferral(&deferred, messages.len(), total, limit, wall)?;
        }
        self.drain_notifications(messages, wall)
    }
    /// The next reliable frame for `actor` from its queue position `offset`
    /// (`wire`), with `sequence`, and how many queued frames it consumes.
    ///
    /// With `--coalesce-character-replication`, consecutive single-Character
    /// deltas of one tick join into one update that fits `budget` bytes (see
    /// `character_coalescing`). The recipient's own Character, bootstrap
    /// frames and anything unmeasured go out alone, exactly as without it.
    fn next_shared_packet(
        &self,
        actor: &Active,
        offset: usize,
        wire: &[u8],
        sequence: u32,
        budget: usize,
    ) -> Result<(Vec<u8>, usize), &'static str> {
        use crate::character_coalescing::{CharacterRow, join};
        let single = || {
            Ok((
                for_recipient_with_profile(wire, sequence, self.wire_profile)?,
                1,
            ))
        };
        if !self.coalesce_character_replication {
            return single();
        }
        let profile = self.wire_profile;
        let joinable = |position: usize, frame: &[u8]| {
            (!actor.hospital_bootstrap.contains(&position)
                && !actor.smoke_bootstrap.contains(&position))
            .then(|| CharacterRow::measure(frame, profile))
            .flatten()
            .filter(|row| row.object != actor.reservation.player)
        };
        let Some(first) = joinable(offset, wire) else {
            return single();
        };
        let limit = budget.min(crate::replication::MAX_UPDATE_BYTES);
        let mut rows = vec![first];
        let mut wires = vec![wire];
        let mut row_bits = first.bits();
        for (position, frame) in actor.pending_shared.iter().enumerate().skip(offset + 1) {
            let Some(row) = joinable(position, frame) else {
                break;
            };
            if row.timestamp_ms != first.timestamp_ms
                || rows.iter().any(|seen| seen.object == row.object)
                || CharacterRow::update_bytes(row_bits + row.bits(), profile) > limit
            {
                break;
            }
            row_bits += row.bits();
            rows.push(row);
            wires.push(frame);
        }
        if rows.len() == 1 {
            return single();
        }
        Ok((join(&wires, &rows, sequence, profile)?, rows.len()))
    }
    /// Count one budget-limited reply and report the running count at most
    /// once per wall second, with the deepest queue against its bound.
    fn report_shared_deferral(
        &mut self,
        deferred: &[(usize, usize)],
        messages: usize,
        bytes: usize,
        message_limit: usize,
        wall: u64,
    ) -> Result<(), &'static str> {
        let deferral = &mut self.shared_deferral;
        deferral.replies = deferral.replies.saturating_add(1);
        if deferral.last_report == Some(wall) {
            return Ok(());
        }
        let since = deferral.replies - deferral.reported_replies;
        deferral.reported_replies = deferral.replies;
        deferral.last_report = Some(wall);
        let replies = deferral.replies;
        let (deepest, bound) = deferred.iter().copied().max().unwrap_or_default();
        self.record(
            json!({"op":"shared_replication_deferred","recipients":deferred.len(),
                "queued_frames":deferred.iter().map(|(queued, _)| queued).sum::<usize>(),
                "deepest_queue":deepest,"queue_bound":bound,"messages":messages,
                "payload_bytes":bytes,"message_limit":message_limit,"byte_limit":24_000,
                "deferred_replies":since,"deferred_replies_total":replies}),
            wall,
        )
    }
    pub(super) fn alpha_passenger_seat(&self, player: u16) -> Option<u8> {
        // The native bunker has distinct physical seats; descending selection
        // matches GetFirstAvailableSeat(-1), preserving seat8 for an empty bunker.
        (1..=8).rev().find(|seat| {
            !self.peers.values().chain(self.active.iter()).any(|actor| {
                actor.reservation.player != player
                    && actor.mounted_bunker.is_none()
                    && actor.mounted_seat == Some(*seat)
            })
        })
    }
    pub(super) fn alpha_owner(&self) -> Option<(u16, u8)> {
        self.peers
            .values()
            .chain(self.active.iter())
            .filter(|p| p.mounted_bunker.is_none())
            .filter_map(|p| p.mounted_seat.map(|seat| (p.reservation.player, seat)))
            .min_by_key(|(player, seat)| (*seat != 0, *player))
    }
    pub(super) fn alpha_ownership_after(
        &self,
        player: u16,
        seat: Option<u8>,
    ) -> replication::AlphaVehicleOwnership {
        let occupants = self
            .peers
            .values()
            .chain(self.active.iter())
            .filter(|p| p.reservation.player != player && p.mounted_bunker.is_none())
            .filter_map(|p| p.mounted_seat.map(|s| (p.reservation.player, s)))
            .chain(seat.map(|s| (player, s)));
        let owner = occupants.min_by_key(|(p, s)| (*s != 0, *p));
        replication::AlphaVehicleOwnership {
            owner: owner.map_or(0x105, |(p, _)| p),
            turret_owner: owner.filter(|(_, s)| *s == 0).map_or(u16::MAX, |(p, _)| p),
        }
    }
    pub(super) fn squad_spawn_for(&self, squad: u16) -> Option<u16> {
        self.peers
            .values()
            .chain(self.active.iter())
            .filter(|a| a.squad_catalog.squad_index() == squad)
            .min_by_key(|a| a.reservation.player)
            .map(|a| a.squad_spawn)
            .or_else(|| self.spawn_catalog.as_ref()?.default_for_squad(squad))
            .or_else(|| {
                self.spawn_catalog
                    .as_ref()
                    .filter(|c| c.squad_index() == squad)
                    .map(|c| c.default_spawn())
            })
    }
    pub(super) fn publish_squad_spawn_change(
        &mut self,
        point: u16,
        now: Duration,
    ) -> Result<(), &'static str> {
        let squad = self
            .active
            .as_ref()
            .ok_or("no_reservation")?
            .squad_catalog
            .squad_index();
        for peer in self
            .peers
            .values_mut()
            .filter(|p| p.squad_catalog.squad_index() == squad)
        {
            let follows = peer
                .selected_spawn
                .is_none_or(|s| s.spawn_point_index == peer.squad_spawn);
            peer.squad_spawn = point;
            std::sync::Arc::make_mut(&mut peer.squad_catalog).set_runtime_default(point)?;
            if follows {
                peer.selected_spawn = Some(protocol::CharacterSpawnSelection {
                    spawn_point_index: point,
                });
                let packet = replication::CharacterSpawnAssignment {
                    sequence: 2,
                    timestamp_ms: now.as_millis() as u32,
                    spawn_point_index: point,
                }
                .encode_for_player_with_catalog_with_profile(
                    peer.reservation.player,
                    &peer.squad_catalog,
                    self.wire_profile,
                )?;
                if peer.initial_objects.is_some() {
                    peer.queue_shared(packet);
                }
            }
        }
        Ok(())
    }
    pub(super) fn squad_creations(&self, include: Option<u16>) -> Vec<replication::SquadCreation> {
        let catalog = self.spawn_catalog.as_ref().expect("loaded spawn catalog");
        let selected = self
            .active
            .as_ref()
            .map_or(catalog.squad_index(), |a| a.squad_catalog.squad_index());
        let platoon = (selected % 16) / 4;
        let indices: Vec<u16> = if catalog.map_pool().squads.is_empty() {
            vec![selected]
        } else {
            catalog
                .map_pool()
                .squads
                .iter()
                .filter(|b| {
                    catalog.match_profile().auto_assign_teams
                        || (u16::from(b.platoon) == platoon && u16::from(b.team) == selected / 16)
                })
                .map(|b| b.index)
                .collect()
        };
        indices
            .into_iter()
            .map(|index| {
                let include = include.filter(|_| index == selected);
                let members = self.squad_members(index, include);
                replication::SquadCreation {
                    object_index: index,
                    owner_index: 0x105,
                    leader_index: self.squad_leader_index(index, &members),
                    members,
                }
            })
            .collect()
    }
    pub(super) fn queue_roster(&mut self, now: Duration) -> Result<(), &'static str> {
        self.reconcile_squad_leaders()?;
        for squad in self.squad_creations(None) {
            let published = (squad.members, squad.leader_index);
            if self
                .last_published_rosters
                .get(&squad.object_index)
                .copied()
                .unwrap_or(([u16::MAX; 8], u16::MAX))
                == published
            {
                continue;
            }
            let packet = SquadRoster {
                sequence: 1,
                timestamp_ms: now.as_millis() as u32,
                squad: squad.object_index,
                members: squad.members,
                leader: squad.leader_index,
            }
            .encode_with_order_state(
                self.wire_profile,
                self.squad_roster_orders(squad.object_index),
            )?;
            self.last_published_rosters
                .insert(squad.object_index, published);
            for actor in self
                .peers
                .values_mut()
                .chain(self.active.iter_mut())
                .filter(|a| a.initial_objects.is_some())
            {
                actor.queue_shared(packet.clone());
            }
        }
        Ok(())
    }
    /// Character field18 carries nominations (native
    /// `MagNetObjectCharacter::SetNominations`); promotion follows in the roster.
    pub(super) fn publish_nominations(&mut self, now: Duration) -> Result<(), &'static str> {
        let actor = self.active.as_ref().ok_or("no_reservation")?;
        let packet = replication::CharacterLevelUpdate {
            sequence: 1,
            timestamp_ms: now.as_millis() as u32,
            object_index: actor.reservation.player,
            mag_info: actor.mag_info(
                actor
                    .reservation
                    .character
                    .progression
                    .unwrap_or_default()
                    .level,
            ),
        }
        .encode_with_profile(self.wire_profile)?;
        for recipient in self
            .peers
            .values_mut()
            .chain(self.active.iter_mut())
            .filter(|a| a.initial_objects.is_some())
        {
            recipient.queue_shared(packet.clone());
        }
        self.queue_roster(now)
    }
    pub(super) fn publish_squad_change(&mut self, now: Duration) -> Result<(), &'static str> {
        let actor = self.active.as_ref().ok_or("no_reservation")?;
        let c = &actor.reservation.character;
        let claimed = actor.claimed.as_ref().ok_or("unclaimed_actor")?;
        let info = replication::CharacterInfo {
            team: replication::SquadAddress::from_index(actor.squad_catalog.squad_index())?.team(),
            faction: c.faction,
            character_class: 21,
            name: c.name.clone(),
            client_index: actor.reservation.player,
            squad_index: actor.squad_catalog.squad_index(),
            account_id: actor.reservation.account,
            np_identity: claimed.np_identity,
        };
        let packet = CharacterSquadAssignment {
            sequence: 1,
            timestamp_ms: now.as_millis() as u32,
            player: actor.reservation.player,
            info: &info,
        }
        .encode_with_profile(self.wire_profile)?;
        let assignment = replication::SquadSpawnAssignment {
            sequence: 2,
            timestamp_ms: now.as_millis() as u32,
            spawn_point_index: actor.squad_spawn,
        }
        .encode_with_character_for_player_and_catalog_with_profile(
            false,
            actor.reservation.player,
            &actor.squad_catalog,
            self.wire_profile,
        )?;
        let private = replication::CharacterSpawnAssignment {
            sequence: 2,
            timestamp_ms: now.as_millis() as u32,
            spawn_point_index: actor.squad_spawn,
        }
        .encode_for_player_with_catalog_with_profile(
            actor.reservation.player,
            &actor.squad_catalog,
            self.wire_profile,
        )?;
        for recipient in self
            .peers
            .values_mut()
            .chain(self.active.iter_mut())
            .filter(|a| a.initial_objects.is_some())
        {
            recipient.queue_shared(packet.clone());
            recipient.queue_shared(assignment.clone());
        }
        self.active.as_mut().unwrap().queue_shared(private);
        self.queue_roster(now)
    }
}
fn is_spawn_assignment(packet: &[u8], profile: crate::wire_profile::WireProfile) -> bool {
    // Typed body discrimination; Character field14 is the private preview selection.
    let mut bits = replication::BitReader::with_profile(packet, profile);
    bits.position = profile.header_bits();
    if bits.read(1) != Ok(1) || bits.read(16).is_err() {
        return false;
    }
    // A field14-only delta starts with fourteen absent fields.
    (0..14).all(|_| bits.read(1) == Ok(0)) && bits.read(1) == Ok(1)
}

#[cfg(test)]
mod spawned_kit_tests {
    use super::*;
    use crate::spawn_progression::{dispatch, engine, ready, reserve};

    #[test]
    fn saved_kit_replacement_does_not_change_current_life_snapshot() {
        let mut engine = engine();
        reserve(&mut engine, 0);
        let actor = engine.active.as_mut().unwrap();
        ready(actor);
        actor.claimed = Some(Claimed {
            wire: Vec::new(),
            np_identity: [0; 36],
            game_info: protocol::GameInfo {
                client_index: 0,
                level_name: "synthetic_arena".into(),
                start_time_ms: 0,
                attacking_faction: 1,
                defending_faction: 0,
                maximum_points: [100; 2],
                current_points: [100; 2],
                game_type: 0,
            },
        });
        actor.life = CharacterLife::AwaitingRespawn { generation: 0 };
        dispatch(&mut engine, &0x0211u16.to_be_bytes(), 0).unwrap();
        let update = replication::ObjectUpdate {
            header: replication::UpdateHeader {
                sequence: 1,
                nak_sequence: 0,
                client_index: 0,
                object_type_mask: 0,
                timestamp_ms: 100,
            },
            wire: &[],
        }
        .encode_empty_with_profile(engine.wire_profile)
        .unwrap();
        dispatch(&mut engine, &update, 100).unwrap();
        let actor = engine.active.as_mut().unwrap();
        assert!(actor.life.is_alive());
        assert!(actor.current_inventory_handle.is_some());
        let now = Duration::from_millis(101);
        let before = actor.snapshot(now).unwrap();
        // A future validated armory refresh replaces saved rows, while this
        // life must retain the kit already published at its physical spawn.
        actor
            .reservation
            .character
            .loadouts
            .as_mut()
            .unwrap()
            .inventories[0][21] = Some(2);
        engine
            .item_catalog
            .validate_saved_item_bindings(actor.reservation.character.loadouts.as_ref().unwrap())
            .unwrap();
        let after = actor.snapshot(now).unwrap();
        assert_eq!(
            before, after,
            "saved rows leaked into the live kit snapshot"
        );
    }
}
