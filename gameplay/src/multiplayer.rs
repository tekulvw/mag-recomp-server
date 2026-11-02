//! One shared world with independently authenticated actor and recipient state.
use super::*;
use replication::multiplayer::*;

const MAX_SHARED_QUEUE: usize = 128;

impl Active {
    /// Reliable packets cannot be discarded or coalesced without traced ACK rules.
    /// Bound memory now; retire this recipient through recorded lifecycle cleanup
    /// on the next request rather than failing the healthy publisher's operation.
    pub(super) fn queue_shared(&mut self, packet: Vec<u8>) {
        if self.pending_shared.len() >= MAX_SHARED_QUEUE {
            self.replication_backlogged = true;
        } else if !self.replication_backlogged {
            self.pending_shared.push_back(packet);
        }
    }

    pub(super) fn apply_peer_damage(
        &mut self,
        generation: u32,
        damage: replication::CharacterDamage,
        now: Duration,
    ) -> Result<(), &'static str> {
        if !matches!(self.life,CharacterLife::Alive{generation:g,..} if g==generation) {
            return Err("stale_peer_damage");
        }
        self.last_damage = damage;
        self.life = if damage.health_quarter_points == 0 {
            self.healing = None;
            self.pending_damage = None;
            self.clear_live_projectiles(now)?;
            self.last_exit = None;
            self.pending_spawn_deadline = None;
            self.mounted_seat = None;
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
        self.projectile_replicas
            .clone()
            .take_all_removals()
            .into_iter()
            .chain(
                self.projectile_cleanup
                    .iter()
                    .flatten()
                    .map(|entry| entry.removal),
            )
            .map(|removal| {
                removal.encode_with_profile(1, now.as_millis() as u32, self.wire_profile)
            })
            .collect()
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
    fn snapshot(
        &self,
        now: Duration,
        catalog: &crate::item_catalog::ItemCatalog,
    ) -> Result<Vec<u8>, &'static str> {
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
        let mut inventory = self.current_inventory_handle.and_then(|h| {
            self.reservation.character.replication_loadout(
                catalog,
                h.inventory_index,
                self.wire_profile,
            )
        });
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
                } else if let Some(seat) = self.mounted_seat {
                    replication::CharacterState {
                        state: 14,
                        stance: 0,
                        user_data: [13, 0, seat],
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
                position: sample.position,
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
        if minimum == 0 || minimum > maximum || maximum > self.admission_capacity() {
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
                    .min(policy.team_capacity)
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
        if let Request::Release { reservation_id, .. } = &request
            && self.released_reservations.contains(reservation_id)
        {
            self.record(
                json!({"op":"released_replay","reservation_id":reservation_id}),
                wall,
            )?;
            return Ok(Vec::new());
        }
        if let Request::Reserve { reservation_id, .. } = &request {
            if self.released_reservations.contains(reservation_id) {
                return Err("reservation_id_retired");
            }
            if self.match_terminal.is_some() {
                return Err("match_finished");
            }
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
                messages.extend(self.tick_match_clock(now, wall)?);
                self.queue_projectile_publications(now)?;
            }
            return self.drain_shared(messages, wall);
        }
        if self.development_dummy_enabled() {
            return Err("dummy_slot_conflicts_with_multiplayer");
        }
        let id = match &request {
            Request::Hello { .. }
            | Request::ConfigureMatch { .. }
            | Request::ProgressionCommitted { .. } => None,
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
        let before_sequence = self.active.as_ref().map_or(0, |a| a.outbound_sequence);
        let before_owner = self.active.as_ref().map_or(0, |a| a.latest_owner_sequence);
        let before_alive_generation = self.active.as_ref().and_then(|a| match a.life {
            CharacterLife::Alive { generation, .. } => Some(generation),
            _ => None,
        });
        let had_emplacement = self
            .active
            .as_ref()
            .is_some_and(|a| a.emplacement_creation.is_some());
        let removed = if matches!(&request, Request::Release { .. }) {
            self.active
                .as_ref()
                .filter(|a| a.initial_objects.is_some())
                .map(|a| {
                    a.projectile_removal_packets(now)
                        .map(|packets| (a.reservation.player, a.mounted_seat, packets))
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
        let is_release = matches!(&request, Request::Release { .. });
        let mut messages = self.apply(request, decoded, now, wall)?;
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
            let snapshot = self
                .active
                .as_ref()
                .unwrap()
                .snapshot(now, &self.item_catalog)?;
            let existing: Vec<Vec<u8>> = self
                .peers
                .values()
                .filter(|a| a.initial_objects.is_some())
                .map(|a| {
                    let mut packets = vec![a.snapshot(now, &self.item_catalog)?];
                    packets.extend(
                        a.projectile_replicas
                            .snapshot_packets(now.as_millis() as u32)?,
                    );
                    Ok(packets)
                })
                .collect::<Result<Vec<Vec<Vec<u8>>>, &'static str>>()?
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
            for packet in existing {
                self.active.as_mut().unwrap().queue_shared(packet);
            }
            self.record(
                json!({"op":"shared_actor_created","reservation_id":new_id}),
                wall,
            )?;
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
            if mask != ((1 << 3) | (1 << 7) | (1 << 8) | (1 << 13))
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
        if let Some((player, seat, projectiles)) = removed {
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
            if seat.is_some() {
                self.queue_alpha_ownership(now)?;
            }
        }
        if let Some((sequence, fields)) = owner_report
            && sequence > before_owner
            && self.active.as_ref().is_some_and(|a| {
                // A report from before spawn must not overwrite the new life
                // with its old pose or inventory after the authoritative packet.
                matches!(a.life, CharacterLife::Alive { generation, .. }
                    if Some(generation) == before_alive_generation)
                    && a.ready_for_updates
                    && a.mounted_seat.is_none()
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
        if !self.world_tick_enabled {
            self.queue_team_wave_deadlines(now, wall)?;
            messages.extend(self.tick_objectives(now, wall)?);
            messages.extend(self.tick_match_clock(now, wall)?);
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
        let mut expired_occupant = false;
        for id in expired {
            let actor = self
                .peers
                .get(&id)
                .or_else(|| self.active.as_ref().filter(|a| a.reservation.id == id))
                .unwrap();
            let op = if actor.replication_backlogged {
                "replication_backlog_released"
            } else {
                "expired"
            };
            self.record(json!({"op":op,"reservation_id":id}), wall)?;
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
            if self
                .alpha_turret_reporter
                .as_ref()
                .is_some_and(|(id, _)| *id == removed.reservation.id)
            {
                self.alpha_turret_reporter = None;
            }
            expired_occupant |= removed.mounted_seat.is_some();
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
            if expired_occupant {
                self.queue_alpha_ownership(now)?;
            }
        }
        if any_expired && self.peers.is_empty() && self.active.is_none() {
            self.clear_bot_world();
            self.reset_objective_world();
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
        owner.projectile_cleanup[usize::from(slot)].is_some()
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
    /// This runs after admission: a launch on the cleanup deadline is denied;
    /// the same request flushes removal, permitting the next launch request.
    pub(crate) fn queue_projectile_publications(
        &mut self,
        now: Duration,
    ) -> Result<(), &'static str> {
        // Hold cleanup for the slowest recipient. This check runs before drain:
        // the last queued state2 grants500ms from its actual delivery request,
        // even when the32-message budget deferred it past the original deadline.
        let queued_terminals: std::collections::BTreeSet<_> = self
            .peers
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
            .collect();
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
            for publication in actor.projectile_publications.drain(..) {
                publications.push((actor.reservation.id.clone(), publication));
            }
        }
        for (owner, publication) in publications {
            for recipient in self
                .peers
                .values_mut()
                .chain(self.active.iter_mut())
                .filter(|a| a.initial_objects.is_some())
            {
                if publication.include_owner || recipient.reservation.id != owner {
                    recipient.queue_shared(publication.wire.clone());
                }
            }
        }
        Ok(())
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
        let limit = if self.maximum_players > 8 { 128 } else { 32 };
        let mut planned = Vec::new();
        let mut total = messages
            .iter()
            .map(|m| m.payload_hex.len() / 2)
            .sum::<usize>();
        if messages.len() > limit || total > 24_000 {
            return Err("shared_reply_budget_exceeded");
        }
        let mut ready: Vec<_> = self
            .peers
            .values()
            .chain(self.active.iter())
            .filter(|a| a.ready_for_updates && !a.pending_shared.is_empty())
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
            .map(|a| a.next_sequence())
            .collect::<Result<Vec<_>, _>>()?;
        let mut next_cursor = self.shared_drain_cursor;
        'drain: loop {
            let mut progressed = false;
            for (index, actor) in ready.iter().enumerate() {
                if messages.len() + planned.len() >= limit {
                    break 'drain;
                }
                let Some(wire) = actor.pending_shared.get(offsets[index]) else {
                    continue;
                };
                let packet = for_recipient_with_profile(wire, sequences[index], self.wire_profile)?;
                if total + packet.len() > 24_000 {
                    // Keep this recipient's head packet intact. Smaller heads
                    // may still fit; resume after the last served slot next time.
                    continue;
                }
                total += packet.len();
                planned.push((actor.reservation.id.clone(), packet));
                offsets[index] += 1;
                sequences[index] = sequences[index]
                    .checked_add(1)
                    .ok_or("replication_sequence_exhausted")?;
                next_cursor = actor.reservation.player.wrapping_add(1);
                progressed = true;
            }
            if !progressed {
                break;
            }
        }
        let recipients: Vec<_> = planned
            .iter()
            .map(|(id, wire)| {
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
        for (id, packet) in planned {
            let actor = if self.active.as_ref().is_some_and(|a| a.reservation.id == id) {
                self.active.as_mut().unwrap()
            } else {
                self.peers.get_mut(&id).unwrap()
            };
            actor.remember(packet)?;
            actor.pending_shared.pop_front();
        }
        self.shared_drain_cursor = next_cursor;
        messages.extend(recipients);
        self.drain_notifications(messages, wall)
    }
    pub(super) fn alpha_passenger_seat(&self, player: u16) -> Option<u8> {
        // The native bunker has distinct physical seats; descending selection
        // matches GetFirstAvailableSeat(-1), preserving seat8 for an empty bunker.
        (1..=8).rev().find(|seat| {
            !self.peers.values().chain(self.active.iter()).any(|actor| {
                actor.reservation.player != player && actor.mounted_seat == Some(*seat)
            })
        })
    }
    pub(super) fn alpha_owner(&self) -> Option<(u16, u8)> {
        self.peers
            .values()
            .chain(self.active.iter())
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
            .filter(|p| p.reservation.player != player)
            .filter_map(|p| p.mounted_seat.map(|s| (p.reservation.player, s)))
            .chain(seat.map(|s| (player, s)));
        let owner = occupants.min_by_key(|(p, s)| (*s != 0, *p));
        replication::AlphaVehicleOwnership {
            owner: owner.map_or(0x105, |(p, _)| p),
            turret_owner: owner.filter(|(_, s)| *s == 0).map_or(u16::MAX, |(p, _)| p),
        }
    }
    fn queue_alpha_ownership(&mut self, now: Duration) -> Result<(), &'static str> {
        let packet = self
            .alpha_ownership_after(u16::MAX, None)
            .encode_update_with_profile(1, now.as_millis() as u32, self.wire_profile)?;
        for actor in self
            .peers
            .values_mut()
            .chain(self.active.iter_mut())
            .filter(|a| a.initial_objects.is_some())
        {
            actor.queue_shared(packet.clone());
        }
        Ok(())
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
            .encode_with_profile(self.wire_profile)?;
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
