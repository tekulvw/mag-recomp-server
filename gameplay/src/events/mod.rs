//! Authenticated event planning. Native handlers see immutable world/session state.
//! Replies and state changes leave this module only after evidence recording succeeds.
use super::*;

mod abilities;
mod combat;
mod control;
mod falling;
mod objectives;
mod owner;
mod seats;
mod statistics;
pub(crate) use statistics::bot_killed_human;
mod update_actions;

pub(super) struct EventInput {
    pub reservation_id: String,
    pub world_id: u32,
    pub player_id: u16,
    pub transport: Transport,
    pub direction: Direction,
    pub rt_type: u8,
    pub plugin_id: Option<u8>,
    pub payload_hex: String,
}

struct PlanningContext<'a, W: Write> {
    engine: &'a Engine<W>,
    active: &'a Active,
    decoded: &'a DecodedEvent<'a>,
    now: Duration,
    next_sequence: u32,
    history_available: bool,
    accepted_owner_route: bool,
    accepted_objective_route: bool,
}

/// Changes are accumulated independently from authoritative state. Keeping the
/// plan separate makes the record/reject/commit order explicit at the call site.
#[derive(Default)]
struct EventPlan {
    session: SessionChanges,
    objective_world: Option<objective_control::ObjectiveWorld>,
    combat: CombatChanges,
    leadership: leadership_runtime::LeadershipPlan,
    statistics: statistics::StatisticsPlan,
    replication: ReplicationChanges,
    audit: EventAudit,
    messages: Vec<OutboundMessage>,
    outgoing: Option<Vec<u8>>,
    rejection: Option<&'static str>,
    gameplay_denial: Option<&'static str>,
}
impl EventPlan {
    fn new(active: &Active) -> Self {
        Self {
            combat: CombatChanges {
                projectile_replicas: active.projectile_replicas.clone(),
                projectile_cleanup: active.projectile_cleanup.clone(),
                ..Default::default()
            },
            ..Default::default()
        }
    }
}

#[derive(Default)]
struct SessionChanges {
    new_claim: Option<Claimed>,
    new_ready: Option<bool>,
    new_spawn_deadline: Option<Duration>,
    new_team_wave: Option<TeamWaveClock>,
    published_wave_deadline: Option<Duration>,
    cancel_pending_spawn: bool,
    new_mounted_seat: Option<Option<u8>>,
    last_exit: Option<protocol::DismountSeat>,
    exit_yaw: Option<u16>,
    clock_points: Option<i16>,
    selected_loadout: Option<protocol::SelectLoadout>,
    new_inventory_handle: Option<replication::InventoryHandle>,
    selected_spawn: Option<protocol::CharacterSpawnSelection>,
    squad_spawn: Option<u16>,
    squad_catalog: Option<std::sync::Arc<spawn_catalog::SpawnCatalog>>,
    new_nominations: Option<u8>,
    entered_game: bool,
    client_position: Option<(replication::CharacterPosition, u32)>,
    fall_tracker: Option<fall_damage::Tracker>,
    server_position: Option<(
        replication::CharacterPosition,
        player_tracking::PositionOrigin,
    )>,
    new_owner_sequence: Option<u32>,
    new_turret_fields: Option<turret::ClientTurretFields>,
}

#[derive(Default)]
struct CombatChanges {
    new_life: Option<CharacterLife>,
    peer_damage: Vec<(String, u32, replication::CharacterDamage)>,
    bot_damage: Vec<(u16, u32, replication::CharacterDamage)>,
    death_notifications: Vec<death_notifications::Plan>,
    new_damage_model: Option<replication::CharacterDamage>,
    new_healing: Option<Option<progression::HealingSession>>,
    peer_healing: Vec<(String, u32, progression::HealingSession)>,
    new_healing_sequence: Option<u64>,
    new_rewards: Vec<progression::RewardEvent>,
    new_heal_allowed: Option<Duration>,
    new_projectiles: Option<projectile::ProjectileFlights>,
    projectile_replicas: projectile_replication::ProjectileReplicas,
    projectile_cleanup: [Option<ProjectileCleanup>; 3],
    projectile_publications: Vec<ProjectilePublication>,
    new_grenade_remaining: Option<u8>,
    new_grenade_damage: Option<std::collections::VecDeque<PendingDamage>>,
    queued_fall: Option<PendingDamage>,
    dummy_created: bool,
    dummy_health: Option<u16>,
    bot_runtime: Option<development_target::BotRuntime>,
    bot_roster_published: bool,
    observed_primary: Option<replication::InventoryItem>,
    observed_secondary: Option<replication::InventoryItem>,
    primary_equipped: Option<bool>,
    clear_pending_damage: bool,
}

#[derive(Default)]
struct ReplicationChanges {
    initial_objects: Option<Vec<u8>>,
    spawn_delta: Option<Vec<u8>>,
    wave_delta: Option<Vec<u8>>,
    damage_delta: Option<Vec<u8>>,
    projectile_delta: Option<Vec<u8>>,
    dummy_delta: Option<Vec<u8>>,
    objective_delta: Option<Vec<u8>>,
    objective_assignment: Option<Vec<u8>>,
    objective_clear: Option<Vec<u8>>,
    objective_target: Option<u16>,
    objective_count: usize,
    emplacement_creation: Option<Vec<u8>>,
    seat_transition: Option<Vec<u8>>,
    squad_spawn_delta: Option<Vec<u8>>,
}

#[derive(Default)]
struct EventAudit {
    bot_script: Option<Value>,
    applied_healing: Option<Value>,
    projectile_reports: Vec<Value>,
    projectile_terminals: Vec<projectile::ProjectileTerminal>,
    grenade_blast_checks: Vec<blast_visibility::GrenadeBlastCheck>,
    applied_damage: Option<Value>,
    falling_damage: Option<fall_damage::FallDamage>,
    spawned_point: Option<u16>,
}

fn plan_message<W: Write>(
    ctx: &PlanningContext<'_, W>,
    plan: &mut EventPlan,
) -> Result<Value, &'static str> {
    let spawn_or_selection = matches!(
        &ctx.decoded.message,
        Ok(protocol::GameMessage::ConfirmRespawn(_)
            | protocol::GameMessage::CancelRespawn(_)
            | protocol::GameMessage::CharacterSpawnSelection(_)
            | protocol::GameMessage::SquadSpawnSelection(_)
            | protocol::GameMessage::ChangePlayerSquad(_)
            | protocol::GameMessage::SelectLoadout(_))
    );
    let combat = matches!(
        &ctx.decoded.message,
        Ok(protocol::GameMessage::Hit(_)
            | protocol::GameMessage::FireGrenade(_)
            | protocol::GameMessage::Heal(_)
            | protocol::GameMessage::KillSelf(_)
            | protocol::GameMessage::MountSeat(_)
            | protocol::GameMessage::DismountSeat(_)
            | protocol::GameMessage::Repair(_))
    );
    if ctx.engine.match_terminal.is_some()
        && (spawn_or_selection || (combat && !ctx.engine.round_combat_allowed(ctx.now)))
    {
        plan.gameplay_denial = Some("match_finished");
        return Ok(json!({"type":"postgame_action","accepted":false,"reason":"match_finished"}));
    }
    match ctx.decoded.message.clone() {
        Ok(protocol::GameMessage::TimeRequest(request)) => {
            control::time_request(ctx, plan, request)
        }
        Ok(protocol::GameMessage::ClaimReservation(c)) => control::claim_reservation(ctx, plan, c),
        Ok(protocol::GameMessage::ClientReady(request)) => {
            control::client_ready(ctx, plan, request)
        }
        Ok(protocol::GameMessage::ObjectUpdate(update)) => owner::object_update(ctx, plan, update),
        Ok(protocol::GameMessage::EnterGame(_)) => control::enter_game(ctx, plan),
        Ok(protocol::GameMessage::SelectLoadout(request)) => {
            control::select_loadout(ctx, plan, request)
        }
        Ok(protocol::GameMessage::ChangePlayerSquad(request)) => {
            control::change_player_squad(ctx, plan, request)
        }
        Ok(protocol::GameMessage::ChangePlayerNominations(request)) => {
            control::change_player_nominations(ctx, plan, request)
        }
        Ok(protocol::GameMessage::SquadSpawnSelection(request)) => {
            control::squad_spawn_selection(ctx, plan, request)
        }
        Ok(protocol::GameMessage::CharacterSpawnSelection(request)) => {
            control::character_spawn_selection(ctx, plan, request)
        }
        Ok(protocol::GameMessage::CancelRespawn(_)) => control::cancel_respawn(ctx, plan),
        Ok(protocol::GameMessage::ConfirmRespawn(_)) => control::confirm_respawn(ctx, plan),
        Ok(protocol::GameMessage::ObjectiveInteraction(request)) => {
            objectives::interaction(ctx, plan, request)
        }
        Ok(protocol::GameMessage::Hit(request)) => combat::hit(ctx, plan, request),
        Ok(protocol::GameMessage::FireGrenade(request)) => combat::fire_grenade(ctx, plan, request),
        Ok(protocol::GameMessage::Heal(request)) => combat::heal(ctx, plan, request),
        Ok(protocol::GameMessage::KillSelf(request)) => combat::kill_self(ctx, plan, request),
        Ok(protocol::GameMessage::MountSeat(request)) => seats::mount_seat(ctx, plan, request),
        Ok(protocol::GameMessage::DismountSeat(request)) => {
            seats::dismount_seat(ctx, plan, request)
        }
        Ok(protocol::GameMessage::Repair(request)) => Ok({
            plan.gameplay_denial = Some("repair_target_unavailable");
            json!({"type":"repair","target_object_id":request.target_object_id})
        }),
        Ok(protocol::GameMessage::PingRequest(request)) => {
            control::ping_request(ctx, plan, request)
        }
        Ok(protocol::GameMessage::CommandAbility(request)) => {
            abilities::command(ctx, plan, request)
        }
        Ok(protocol::GameMessage::StrategicAbility(request)) => {
            abilities::strategic(ctx, plan, request)
        }
        Ok(protocol::GameMessage::PluginGreeting) => Ok(json!({"type":"plugin_greeting"})),
        Ok(protocol::GameMessage::ProtocolInfo(info)) => Ok(
            json!({"type":"protocol_info","protocol":info.protocol,"build":info.build,
            "accepted":info.protocol == protocol::ProtocolInfo::RETAIL_PROTOCOL}),
        ),
        Ok(protocol::GameMessage::Unknown { message_type, .. }) => {
            Ok(json!({"type":"unknown","message_type":message_type}))
        }
        Err(code) => Ok(json!({"type":"malformed","error":code})),
    }
}

impl<W: Write> Engine<W> {
    pub(super) fn handle_event(
        &mut self,
        event: EventInput,
        decoded: &DecodedEvent<'_>,
        now: Duration,
        wall_secs: u64,
    ) -> Result<Vec<OutboundMessage>, &'static str> {
        let active = self.active.as_ref().ok_or("no_reservation")?;
        if active.reservation.id != event.reservation_id
            || active.reservation.world != event.world_id
            || active.reservation.player != event.player_id
        {
            return Err("identity_mismatch");
        }
        if self.match_terminal.is_none()
            && event.rt_type == 0x3d
            && event.plugin_id == Some(1)
            && match (event.transport, &decoded.message) {
                // The real repair tool uses TCP; hits and owner updates
                // keep their previously accepted UDP route.
                (Transport::Tcp, Ok(protocol::GameMessage::Repair(_))) => true,
                (Transport::Udp, Ok(protocol::GameMessage::ObjectUpdate(_))) => false,
                (Transport::Udp, _) => true,
                _ => false,
            }
            && let Some(result) = self.structure_event(&decoded.message, now, wall_secs)?
        {
            return Ok(result);
        }
        let active = self.active.as_ref().ok_or("no_reservation")?;
        // SCERT authentication and reservation binding remain in Horizon.
        // Only this proven route carries raw native gameplay messages.
        let mut plan = EventPlan::new(active);
        plan.leadership = self.plan_leadership(now)?;
        // Preserve the sequence-exhaustion gate for every authenticated event,
        // including opaque transport payloads that produce no native reply.
        let next_sequence = active.next_sequence()?;
        let decoded_log = if event.rt_type == 0x3d && event.plugin_id == Some(1) {
            let ctx = PlanningContext {
                engine: self,
                active,
                decoded,
                now,
                next_sequence,
                accepted_owner_route: matches!(event.transport, Transport::Udp)
                    && matches!(event.direction, Direction::ClientToServer),
                accepted_objective_route: matches!(event.direction, Direction::ClientToServer),
                history_available: self.world_tick_enabled
                    || self.maximum_players > 1
                    || active.replication_history.len() < MAX_REPLICATION_HISTORY,
            };
            let decoded = plan_message(&ctx, &mut plan)?;
            if let Some(payload) = plan.outgoing.take() {
                plan.messages.push(OutboundMessage {
                    reservation_id: event.reservation_id.clone(),
                    world_id: event.world_id,
                    player_id: event.player_id,
                    plugin_id: 1,
                    payload_hex: protocol::encode_hex(&payload),
                });
            }
            decoded
        } else {
            json!({"type":"opaque_transport_payload"})
        };
        statistics::stage(self, &mut plan)?;
        if let Some(points) = plan.session.clock_points {
            let score = protocol::ScoreChange {
                team: 0,
                points,
                reason: protocol::ScoreChangeReason::TimedBleed,
            };
            plan.messages.push(OutboundMessage {
                reservation_id: event.reservation_id.clone(),
                world_id: event.world_id,
                player_id: event.player_id,
                plugin_id: 1,
                payload_hex: protocol::encode_hex(&score.encode_with_profile(self.wire_profile)?),
            });
        }
        // Capture outgoing bytes with the triggering request before returning them
        // or committing the claim. A recording failure never releases replies.
        // The result is already final; grace-period damage does not create new rewards.
        if self.match_terminal.is_some() {
            plan.combat.new_rewards.clear();
        }
        self.check_reward_capacity(&plan.combat.new_rewards)?;
        self.record(
            json!({
                "op": "event",
                "reservation_id": event.reservation_id,
                "world_id": event.world_id,
                "player_id": event.player_id,
                "transport": event.transport,
                "direction": event.direction,
                "rt_type": event.rt_type,
                "plugin_id": event.plugin_id,
                "payload_hex": event.payload_hex.to_ascii_lowercase(),
                "decoded": decoded_log,
                "leadership_changes": plan.leadership.audit(),
                "death_notifications": plan.combat.death_notifications.iter().map(|p| p.audit()).collect::<Vec<_>>(),
                "messages": plan.messages,
                "rewards": plan.combat.new_rewards,
                "rejection": plan.rejection,
                "gameplay_denial": plan.gameplay_denial,
                "clock_points": plan.session.clock_points,
                "representation": if event.rt_type == 0x3d {
                    "plugin_application"
                } else {
                    "scert_body_canonical"
                },
            }),
            wall_secs,
        )?;
        if let Some(code) = plan.rejection {
            return Err(code);
        }
        self.commit_event(&event, decoded, plan, now)
    }

    /// Called only after recording and validating the planned event. Keep this
    /// commit order stable: peer effects, owner fields, history, then actor state.
    fn commit_event(
        &mut self,
        event: &EventInput,
        decoded: &DecodedEvent<'_>,
        mut plan: EventPlan,
        now: Duration,
    ) -> Result<Vec<OutboundMessage>, &'static str> {
        if let Some(clock) = plan.session.new_team_wave {
            let team = self.active.as_ref().ok_or("no_reservation")?.team();
            self.team_waves[usize::from(team)] = Some(clock);
        }
        if let Some(world) = plan.objective_world {
            self.objective_world = Some(world);
        }
        self.commit_leadership(plan.leadership);
        statistics::commit(self, plan.statistics);
        self.commit_rewards(plan.combat.new_rewards);
        for (id, generation, session) in plan.combat.peer_healing {
            if let Some(peer) = self.peers.get_mut(&id)
                && matches!(peer.life,CharacterLife::Alive{generation:g,..} if g==generation)
            {
                peer.healing = Some(session);
            }
        }
        for (id, generation, damage) in plan.combat.peer_damage {
            let notification = self.peers.get(&id).and_then(|peer| {
                plan.combat
                    .death_notifications
                    .iter()
                    .position(|p| p.victim_matches(peer.reservation.player, generation, Some(&id)))
            });
            if let Some(index) = notification {
                self.commit_firearm_death(
                    plan.combat.death_notifications.remove(index),
                    damage,
                    now,
                )?;
            } else if let Some(peer) = self.peers.get_mut(&id) {
                peer.apply_peer_damage(generation, damage, now)?;
            }
        }
        for (id, generation, damage) in plan.combat.bot_damage {
            if let Some(index) = plan
                .combat
                .death_notifications
                .iter()
                .position(|p| p.victim_matches(id, generation, None))
            {
                self.commit_firearm_death(
                    plan.combat.death_notifications.remove(index),
                    damage,
                    now,
                )?;
            } else {
                self.commit_bot_damage(id, generation, damage, now)?;
            }
        }
        if let Some(fields) = plan.session.new_turret_fields {
            self.alpha_turret.merge(&fields);
            if fields.primary.is_some()
                && let CharacterLife::Alive { generation, .. } = self.active.as_ref().unwrap().life
            {
                self.alpha_turret_reporter = Some((event.reservation_id.clone(), generation));
            }
        }
        let squad_changed = plan.session.squad_catalog.is_some();
        let nominations_changed = plan.session.new_nominations.is_some();
        let shared_spawn_changed = plan.session.squad_spawn;
        let active = self.active.as_mut().unwrap();
        if let Some((update, report)) = decoded.owner_update()
            && update.header.client_index == 0
            && update.header.nak_sequence == 0
            && update.header.sequence > active.latest_owner_sequence
            && let Some(fields) = &report.character
        {
            if fields.state.is_some() {
                active.owner_fields.state = fields.state;
            }
            if fields.position.is_some() {
                active.owner_fields.position = fields.position;
            }
            if fields.pitch.is_some() {
                active.owner_fields.pitch = fields.pitch;
            }
            if fields.yaw.is_some() {
                active.owner_fields.yaw = fields.yaw;
            }
        }
        // Only newly encoded object updates enter history. Replays and time/score
        // replies consume no replication sequence; commit follows record+flush.
        if let Some(packet) = plan
            .replication
            .initial_objects
            .as_ref()
            .or(plan.replication.spawn_delta.as_ref())
            .or(plan.replication.wave_delta.as_ref())
            .or(plan.replication.squad_spawn_delta.as_ref())
            .or(plan.replication.objective_delta.as_ref())
            .or(plan.replication.objective_assignment.as_ref())
            .or(plan.replication.objective_clear.as_ref())
            .or(plan.replication.emplacement_creation.as_ref())
            .or(plan.replication.seat_transition.as_ref())
            .or(plan.replication.damage_delta.as_ref())
            .or(plan.replication.projectile_delta.as_ref())
            .or(plan.replication.dummy_delta.as_ref())
        {
            active.remember(packet.clone())?;
        }
        if let Some(tracker) = plan.session.fall_tracker {
            active.fall_tracker = tracker;
        }
        if let Some((position, sequence)) = plan.session.client_position {
            active
                .tracking
                .accept_client(position, sequence, now.as_millis() as u64);
        }
        if let Some((position, origin)) = plan.session.server_position {
            active
                .tracking
                .server_position(position, origin, now.as_millis() as u64);
        }
        if plan.combat.clear_pending_damage || plan.replication.spawn_delta.is_some() {
            active.pending_damage = None;
        }
        if plan.combat.bot_roster_published
            && let Some(target) = active.development_target.as_mut()
        {
            target.roster_published = true;
        }
        if let Some(runtime) = plan.combat.bot_runtime
            && let Some(target) = active.development_target.as_mut()
        {
            target.runtime = Some(runtime);
            if matches!(runtime.phase, development_target::BotPhase::Dead) {
                target.pending_move = None;
            }
        }
        active.dummy_created |= plan.combat.dummy_created;
        if let Some(health) = plan.combat.dummy_health {
            active.dummy_health = Some(health);
        }
        if plan.replication.spawn_delta.is_some() {
            active.observed_secondary = None;
        }
        if let Some(item) = plan.combat.observed_primary {
            active.observed_primary = Some(item);
        }
        if let Some(item) = plan.combat.observed_secondary {
            active.observed_secondary = Some(item);
        }
        if let Some(flag) = plan.combat.primary_equipped {
            active.primary_equipped = flag;
        }
        if let Some(flights) = plan.combat.new_projectiles {
            active.projectiles = flights;
        }
        active.projectile_replicas = plan.combat.projectile_replicas;
        active.projectile_cleanup = plan.combat.projectile_cleanup;
        active
            .projectile_publications
            .extend(plan.combat.projectile_publications);
        if let Some(remaining) = plan.combat.new_grenade_remaining {
            active.grenade_remaining = remaining;
        }
        if let Some(sequence) = plan.session.new_owner_sequence {
            active.latest_owner_sequence = sequence;
        }
        if let Some(queue) = plan.combat.new_grenade_damage {
            active.grenade_damage = queue;
        }
        if let Some(model) = plan.combat.new_damage_model {
            active.last_damage = model;
        }
        if let Some(effect) = plan.combat.new_healing {
            active.healing = effect;
        }
        if let Some(sequence) = plan.combat.new_healing_sequence {
            active.healing_sequence = sequence;
        }
        if let Some(deadline) = plan.combat.new_heal_allowed {
            active.next_heal_allowed = deadline;
        }
        if let Some(life) = plan.combat.new_life {
            active.life = life;
            if !life.is_alive() || plan.replication.spawn_delta.is_some() {
                active.healing = None;
                // An owner's death/new life invalidates its own queued hits,
                // not damage already owed to independent world characters.
                active.grenade_damage.retain(|hit| {
                    hit.target_character_id != (0x03800000 | u32::from(active.reservation.player))
                });
                active.clear_live_projectiles(now)?;
                if plan.replication.spawn_delta.is_some() {
                    active.last_damage = replication::CharacterDamage {
                        last_location_and_type: 0,
                        health_quarter_points: active.modifiers.max_health_quarter_points()?,
                        conditions: 0,
                        last_attacker: 0,
                        last_source_object: 0,
                        last_user_data: 0,
                    };
                    active.next_heal_allowed = Duration::ZERO;
                    active.grenade_remaining = plan
                        .session
                        .new_inventory_handle
                        .or(active.current_inventory_handle)
                        .and_then(|h| {
                            active.reservation.character.replication_loadout(
                                &self.item_catalog,
                                h.inventory_index,
                                active.wire_profile,
                            )
                        })
                        .and_then(|kit| {
                            kit.inventories
                                .get(usize::from(kit.handle.inventory_index))
                                .and_then(|i| i.slots[4])
                        })
                        .and_then(|id| self.item_catalog.grenade(id).ok())
                        .map_or(0, |p| p.initial_count);
                }
            }
            if !life.is_alive() || plan.replication.spawn_delta.is_some() {
                active.last_exit = None;
                active.pending_spawn_deadline = None;
            }
        }
        if plan.session.cancel_pending_spawn {
            // Commit dequeue only after the reply/evidence record was flushed.
            // Keep team timing, selection, loadout and history unchanged.
            active.pending_spawn_deadline = None;
        }
        if let Some(deadline) = plan.session.new_spawn_deadline {
            active.pending_spawn_deadline = Some(deadline);
        }
        if let Some(deadline) = plan.session.published_wave_deadline {
            active.published_wave_deadline = Some(deadline);
        }
        active.last_event = now;
        if plan.session.new_claim.is_some() {
            active.claimed = plan.session.new_claim;
        }
        if let Some(ready) = plan.session.new_ready {
            active.ready_for_updates = ready;
        }
        if plan.replication.initial_objects.is_some() {
            active.initial_objects = plan.replication.initial_objects;
        }
        if plan.replication.spawn_delta.is_some() {
            active.spawn_delta = plan.replication.spawn_delta;
            active.match_clock_started.get_or_insert(now);
            active.objective_assigned = false;
            active.objective_needs_clear = true;
        }
        if plan.replication.objective_clear.is_some() {
            active.objective_needs_clear = false;
        }
        active.objective_assigned |= plan.replication.objective_assignment.is_some();
        if plan.replication.objective_delta.is_some() {
            active.objective_delta = plan.replication.objective_delta;
        }
        if plan.replication.emplacement_creation.is_some() {
            active.emplacement_creation = plan.replication.emplacement_creation;
        }
        if let Some(delta) = plan.replication.seat_transition {
            active.seat_transitions.push(delta);
        }
        if let Some(seat) = plan.session.new_mounted_seat {
            active.mounted_seat = seat;
            active.last_exit = plan.session.last_exit;
            active.owner_fields.state = Some(match seat {
                Some(seat) => replication::CharacterState {
                    state: 14,
                    stance: 0,
                    user_data: [13, 0, seat],
                },
                None => replication::CharacterState {
                    state: 1,
                    stance: 0,
                    user_data: [0; 3],
                },
            });
            if plan.combat.new_life.is_some_and(|life| life.is_alive()) {
                active.owner_fields.pitch = Some(127);
                active.owner_fields.yaw = Some(0);
            } else if let Some(yaw) = plan.session.exit_yaw {
                active.owner_fields.yaw = Some(yaw);
            }
        }
        if let Some(points) = plan.session.clock_points {
            active.last_clock_points = points;
        }
        if plan.session.new_inventory_handle.is_some() {
            active.current_inventory_handle = plan.session.new_inventory_handle;
        }
        if plan.session.selected_loadout.is_some() {
            active.selected_loadout = plan.session.selected_loadout;
        }
        if let Some(nominations) = plan.session.new_nominations {
            active.nominations = nominations;
        }
        if let Some(catalog) = plan.session.squad_catalog {
            // A new squad needs its own HUD order; world state/history remain intact.
            active.objective_assigned = false;
            active.objective_needs_clear = active.objective_delta.is_some();
            active.squad_spawn = catalog.default_spawn();
            active.squad_catalog = catalog;
            active.selected_spawn = None;
        }
        if let Some(index) = plan.session.squad_spawn {
            active.squad_spawn = index;
            std::sync::Arc::make_mut(&mut active.squad_catalog).set_runtime_default(index)?;
        }
        if plan.session.selected_spawn.is_some() {
            active.selected_spawn = plan.session.selected_spawn;
        }
        active.entered_game |= plan.session.entered_game;
        if plan.session.server_position.is_some()
            || plan.session.new_mounted_seat.is_some()
            || plan.combat.new_life.is_some_and(|life| !life.is_alive())
        {
            let generation = match active.life {
                CharacterLife::Alive { generation, .. }
                | CharacterLife::AwaitingRespawn { generation } => generation,
                CharacterLife::AwaitingFirstSpawn => 0,
            };
            if let Some(sample) = active.tracking.sample {
                active.fall_tracker.reset(
                    generation,
                    active.latest_owner_sequence,
                    sample.position.y,
                )?;
            }
        }
        if let Some(point) = shared_spawn_changed {
            self.publish_squad_spawn_change(point, now)?;
        }
        if squad_changed {
            self.publish_squad_change(now)?;
        }
        if nominations_changed {
            self.publish_nominations(now)?;
        }
        Ok(plan.messages)
    }
}
