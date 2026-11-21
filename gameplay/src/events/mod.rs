//! Authenticated event planning. Native handlers see immutable world/session state.
//! Replies and state changes leave this module only after evidence recording succeeds.
use super::*;

mod abilities;
mod combat;
mod control;
mod corrections;
mod falling;
mod objectives;
mod orders;
pub(crate) use objectives::exit_message as objective_hold_exit;
mod bunker_seats;
mod owner;
mod resuscitate;
mod seats;
mod statistics;
pub(crate) use statistics::{
    CharacterEffectConsequences, CharacterEffectInput, bot_killed_human,
    plan_character_effect_consequences,
};
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
    accepted_client_route: bool,
}

/// Changes are accumulated independently from authoritative state. Keeping the
/// plan separate makes the record/reject/commit order explicit at the call site.
#[derive(Default)]
struct EventPlan {
    session: SessionChanges,
    objective_world: Option<objective_control::ObjectiveWorld>,
    ground_runtime: Option<acquisition_ground_runtime::Runtime>,
    helo_runtime: Option<domination_helo::FleetRuntime>,
    active_ability_runtime: Option<crate::active_ability_engine::Runtime>,
    strategic_waves: Option<[Option<TeamWaveClock>; 2]>,
    squad_order_candidate: Option<crate::squad_order_engine::Candidate>,
    helo_damage_relay: Vec<Vec<u8>>,
    ground_dynamics_relay: Option<(u16, Vec<u8>)>,
    ground_damage_relay: Option<Vec<u8>>,
    combat: CombatChanges,
    leadership: leadership_runtime::LeadershipPlan,
    statistics: statistics::StatisticsPlan,
    replication: ReplicationChanges,
    audit: EventAudit,
    messages: Vec<OutboundMessage>,
    outgoing: Option<Vec<u8>>,
    rejection: Option<&'static str>,
    gameplay_denial: Option<&'static str>,
    /// Consumed only after the physical spawn and evidence have committed.
    parachute_spawn: Option<(u16, Duration, usize)>,
}
impl EventPlan {
    fn frago_frame<'a, W: Write>(&'a self, engine: &'a Engine<W>) -> crate::frago::Frame<'a> {
        crate::frago::Frame {
            active_reservation: engine.active.as_ref().map(|a| a.reservation.id.as_str()),
            position: self
                .session
                .client_position
                .map(|(p, _)| crate::frago::point(p)),
            ground: self.ground_runtime.as_ref(),
        }
    }
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
    new_mounted_bunker: Option<u16>,
    new_ground_seat: Option<Option<ground_seats::PassengerSeat>>,
    new_helo_seat: Option<Option<domination_helo::PassengerSeat>>,
    last_exit: Option<protocol::DismountSeat>,
    exit_yaw: Option<u16>,
    spawn_pose: Option<(replication::CharacterState, u16)>,
    clock_points: Option<i16>,
    selected_loadout: Option<protocol::SelectLoadout>,
    loadout_selection: Option<loadout_selection::CommittedLoadoutSelection>,
    new_inventory_handle: Option<replication::InventoryHandle>,
    new_spawned_loadout: Option<replication::CharacterLoadout>,
    selected_spawn: Option<protocol::CharacterSpawnSelection>,
    squad_spawn: Option<u16>,
    squad_catalog: Option<std::sync::Arc<spawn_catalog::SpawnCatalog>>,
    new_nominations: Option<u8>,
    entered_game: bool,
    client_position: Option<(replication::CharacterPosition, u32)>,
    queued_correction_position: Option<(replication::CharacterPosition, u32)>,
    correction_tracking: Option<player_tracking::PlayerTracking>,
    correction_publication: Option<Vec<u8>>,
    corrected_owner_position: Option<replication::CharacterPosition>,
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
    peer_lives: Vec<(String, CharacterLife)>,
    peer_damage: Vec<(String, u32, replication::CharacterDamage)>,
    bot_damage: Vec<(u16, u32, replication::CharacterDamage)>,
    death_notifications: Vec<death_notifications::Plan>,
    new_damage_model: Option<replication::CharacterDamage>,
    revived_peer: Option<(String, u32, replication::CharacterDamage)>,
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
    /// Health/life bodies waiting behind already queued world publications.
    ordered_health: Option<Vec<u8>>,
    initial_objects: Option<Vec<u8>>,
    spawn_delta: Option<Vec<u8>>,
    wave_delta: Option<Vec<u8>>,
    damage_delta: Option<Vec<u8>>,
    projectile_delta: Option<Vec<u8>>,
    dummy_delta: Option<Vec<u8>>,
    objective_delta: Option<Vec<u8>>,
    domination_sequence_batches: Vec<Vec<u8>>,
    objective_assignment: Option<Vec<u8>>,
    objective_clear: Option<Vec<u8>>,
    objective_target: Option<u16>,
    objective_count: usize,
    emplacement_creation: Option<Vec<u8>>,
    ground_vehicle_creation: Option<Vec<u8>>,
    helo_creation: Option<Vec<u8>>,
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
    ground_grenade_checks: Vec<Value>,
    helo_grenade_checks: Vec<Value>,
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
            | protocol::GameMessage::Resuscitate(_)
            | protocol::GameMessage::KillSelf(_)
            | protocol::GameMessage::MountSeat(_)
            | protocol::GameMessage::DismountSeat(_)
            | protocol::GameMessage::Repair(_)
            | protocol::GameMessage::CommandAbility(_))
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
        Ok(protocol::GameMessage::CorrectionAck(ack)) => corrections::plan(ctx, plan, ack),
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
        Ok(protocol::GameMessage::Resuscitate(request)) => resuscitate::plan(ctx, plan, request),
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
        Ok(protocol::GameMessage::TaggedObject(request)) => orders::tagged(ctx, plan, request),
        Ok(protocol::GameMessage::SquadWaypoint(request)) => orders::waypoint(ctx, plan, request),
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
        if let Ok(protocol::GameMessage::Repair(request)) = &decoded.message
            && request.target_object_id & 0xffff_0000 == 0x0100_0000
        {
            return self.event_ground_repair(&event, *request, now, wall_secs);
        }
        if self.domination_gameplay.is_some()
            && event.rt_type == 0x3d
            && event.plugin_id == Some(1)
            && matches!(event.transport, Transport::Tcp)
            && matches!(event.direction, Direction::ClientToServer)
            && let Ok(protocol::GameMessage::Repair(request)) = &decoded.message
            && request.target_object_id & 0xffff_0000 == 0x0c80_0000
        {
            return self.event_domination_structural_repair(&event, *request, now, wall_secs);
        }
        if self.acquisition_gameplay.is_some()
            && event.rt_type == 0x3d
            && event.plugin_id == Some(1)
            && matches!(event.transport, Transport::Tcp)
            && matches!(event.direction, Direction::ClientToServer)
            && let Ok(protocol::GameMessage::Repair(request)) = &decoded.message
            && request.target_object_id & 0xffff_0000 == 0x0c80_0000
            && self.objective_world.as_ref().is_some_and(|world| {
                world.native_class(request.target_object_id as u16)
                    == Some(objective_catalog::NativeClass::Structural)
            })
        {
            let (authority, leadership) = self
                .plan_structural_repair_authority(self.acquisition_structural_repair_tool(), now)?;
            return self.event_acquisition_structural(
                &event,
                acquisition_gameplay_engine::StructuralInput::Repair(
                    request.target_object_id as u16,
                    authority,
                ),
                leadership,
                now,
                wall_secs,
            );
        }
        let hospital_route_allowed = self.domination_gameplay.is_none()
            || match &decoded.message {
                Ok(protocol::GameMessage::Hit(request)) if request.defender >> 24 == 13 => {
                    matches!(event.transport, Transport::Udp)
                        && matches!(event.direction, Direction::ClientToServer)
                }
                Ok(protocol::GameMessage::Repair(request))
                    if request.target_object_id >> 24 == 13 =>
                {
                    matches!(event.transport, Transport::Tcp)
                        && matches!(event.direction, Direction::ClientToServer)
                }
                _ => true,
            };
        if hospital_route_allowed
            && self.match_terminal.is_none()
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
        if (self.domination_gameplay.is_some() || self.acquisition_gameplay.is_some())
            && self.match_started
            && self.match_terminal.is_none()
            && event.rt_type == 0x3d
            && event.plugin_id == Some(1)
            && matches!(event.direction, Direction::ClientToServer)
            && (self.world_tick_enabled
                || self.maximum_players > 1
                || active.replication_history.len() < MAX_REPLICATION_HISTORY)
            && let Ok(protocol::GameMessage::ObjectiveInteraction(request)) = &decoded.message
            && matches!(
                request.kind,
                protocol::ObjectiveInteractionKind::Activate
                    | protocol::ObjectiveInteractionKind::Deactivate
                    | protocol::ObjectiveInteractionKind::Release
            )
            && (self.domination_gameplay.is_some()
                || (request.target_object_id & 0xffff_0000 == 0x0c80_0000
                    && self.objective_world.as_ref().is_some_and(|world| {
                        world.native_class(request.target_object_id as u16)
                            == Some(objective_catalog::NativeClass::Structural)
                    })))
            && let Some(actor) = objectives::actor(active)
        {
            let modifier = plan
                .leadership
                .modifiers(active)
                .value("kCharModDemolitionTime");
            if self.acquisition_gameplay.is_some() {
                return self.event_acquisition_structural(
                    &event,
                    acquisition_gameplay_engine::StructuralInput::Interaction(
                        *request, actor, modifier,
                    ),
                    plan.leadership,
                    now,
                    wall_secs,
                );
            }
            return self.event_domination_interaction(
                &event,
                *request,
                actor,
                modifier,
                plan.leadership,
                now,
                wall_secs,
            );
        }
        let decoded_log = if event.rt_type == 0x3d && event.plugin_id == Some(1) {
            let ctx = PlanningContext {
                engine: self,
                active,
                decoded,
                now,
                next_sequence,
                accepted_owner_route: matches!(event.transport, Transport::Udp)
                    && matches!(event.direction, Direction::ClientToServer),
                accepted_client_route: matches!(event.direction, Direction::ClientToServer),
                history_available: self.world_tick_enabled
                    || self.maximum_players > 1
                    || active.replication_history.len() < MAX_REPLICATION_HISTORY,
            };
            let decoded = plan_message(&ctx, &mut plan)?;
            if active.ordered_health_pending
                && !active.pending_shared.is_empty()
                && (plan.replication.damage_delta.is_some()
                    || plan.replication.spawn_delta.is_some()
                    || plan.replication.wave_delta.is_some())
            {
                // A newer health/life body must not overtake a scheduled
                // model already waiting in this recipient's reliable FIFO.
                // Assign its sequence only when that FIFO actually drains.
                plan.replication.ordered_health = plan.outgoing.take();
            }
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
        let gameplay_denial = plan.gameplay_denial.or_else(|| {
            if decoded_log["type"] == "command_ability" && decoded_log["accepted"] == false {
                decoded_log["reason"].as_str()
            } else {
                None
            }
        });
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
                "peer_lives":plan.combat.peer_lives.iter().map(|(id,life)|json!({"reservation_id":id,"life":format!("{life:?}")})).collect::<Vec<_>>(),
                "new_life":plan.combat.new_life.map(|life|format!("{life:?}")),
                "death_notifications": plan.combat.death_notifications.iter().map(|p| p.audit()).collect::<Vec<_>>(),
                "ordered_health": plan.replication.ordered_health.as_ref().map(|p| protocol::encode_hex(p)),
                "domination_sequence_batches": plan.replication.domination_sequence_batches.iter()
                    .map(|packet| protocol::encode_hex(packet)).collect::<Vec<_>>(),
                "messages": plan.messages,
                "rewards": plan.combat.new_rewards,
                "rejection": plan.rejection,
                "gameplay_denial": gameplay_denial,
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
        let previously_driving = self.active_ground_drivers();
        let seating_transition = plan.replication.seat_transition.is_some();
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
        if let Some((id, generation, damage)) = plan.combat.revived_peer {
            let peer = self.peers.get_mut(&id).ok_or("stale_revive_patient")?;
            peer.commit_revive(generation, damage)?;
        }
        for (id, life) in plan.combat.peer_lives {
            if let Some(peer) = self.peers.get_mut(&id) {
                peer.life = life;
                if !life.is_alive() {
                    peer.ground_seat = None;
                    peer.helo_seat = None;
                    peer.helo_creation_sent = false;
                    peer.mounted_seat = None;
                    peer.mounted_bunker = None;
                }
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
        if let Some(packet) = plan.session.correction_publication.take() {
            for peer in self
                .peers
                .values_mut()
                .filter(|a| a.initial_objects.is_some())
            {
                peer.queue_shared(packet.clone());
            }
        }
        let active = self.active.as_mut().unwrap();
        if let Some((update, report)) = decoded.owner_update()
            && update.header.client_index == 0
            && update.header.nak_sequence == 0
            && update.header.sequence > active.latest_owner_sequence
            && let Some(fields) = &report.character
        {
            let fields = active.owner_presentation(fields);
            if fields.state.is_some() {
                active.owner_fields.state = fields.state;
                active.revive_state_pending = false;
            }
            if fields.position.is_some() && !active.tracking.correction_pending() {
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
            .or(plan.replication.ground_vehicle_creation.as_ref())
            .or(plan.replication.helo_creation.as_ref())
            .or(plan.replication.seat_transition.as_ref())
            .or(plan.replication.damage_delta.as_ref())
            .or(plan.replication.projectile_delta.as_ref())
            .or(plan.replication.dummy_delta.as_ref())
            .filter(|_| plan.replication.ordered_health.is_none())
        {
            active.remember(packet.clone())?;
        }
        if let Some(tracker) = plan.session.fall_tracker {
            active.fall_tracker = tracker;
        }
        if let Some(tracking) = plan.session.correction_tracking.take() {
            active.tracking = tracking;
        }
        if let Some(position) = plan.session.corrected_owner_position {
            active.owner_fields.position = Some(position);
        }
        if let Some((position, sequence)) = plan.session.queued_correction_position {
            active
                .tracking
                .queue_correction_report(position, sequence, now.as_millis() as u64);
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
            if !life.is_alive() {
                active.ground_seat = None;
                active.helo_seat = None;
                active.helo_creation_sent = false;
                active.mounted_seat = None;
                active.mounted_bunker = None;
            }
            if !life.is_alive() || plan.replication.spawn_delta.is_some() {
                active.revive_state_pending = false;
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
                        .new_spawned_loadout
                        .clone()
                        .or_else(|| active.equipped_loadout())
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
        for packet in plan.replication.domination_sequence_batches {
            active.queue_shared(packet);
        }
        if plan.replication.objective_delta.is_some() {
            active.objective_delta = plan.replication.objective_delta;
        }
        if plan.replication.emplacement_creation.is_some() {
            active.emplacement_creation = plan.replication.emplacement_creation;
        }
        if plan.replication.ground_vehicle_creation.is_some() {
            active.ground_vehicle_batch += 1;
        }
        active.helo_creation_sent |= plan.replication.helo_creation.is_some();
        if let Some(seat) = plan.session.new_helo_seat {
            active.helo_seat = seat;
            if let Some(yaw) = plan.session.exit_yaw {
                active.last_exit = plan.session.last_exit;
                active.owner_fields.state = Some(replication::CharacterState {
                    state: 1,
                    stance: 0,
                    user_data: [0; 3],
                });
                active.owner_fields.yaw = Some(yaw);
            }
        }
        if let Some(delta) = plan.replication.seat_transition {
            active.seat_transitions.push(delta);
        }
        if let Some(seat) = plan.session.new_mounted_seat {
            active.mounted_seat = seat;
            active.mounted_bunker = seat.and(plan.session.new_mounted_bunker);
            active.last_exit = plan.session.last_exit;
            active.owner_fields.state = Some(match seat {
                Some(seat) => replication::CharacterState {
                    state: 14,
                    stance: 0,
                    user_data: [13, active.mounted_bunker.unwrap_or(0) as u8, seat],
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
        if let Some(seat) = plan.session.new_ground_seat {
            active.ground_seat = seat;
            active.last_exit = plan.session.last_exit;
            active.owner_fields.state = Some(seat.map_or(
                replication::CharacterState {
                    state: 1,
                    stance: 0,
                    user_data: [0; 3],
                },
                ground_seats::PassengerSeat::character_state,
            ));
            if let Some(yaw) = plan.session.exit_yaw {
                active.owner_fields.yaw = Some(yaw);
            }
        }
        if let Some(points) = plan.session.clock_points {
            active.last_clock_points = points;
        }
        if let Some((state, yaw)) = plan.session.spawn_pose {
            active.owner_fields.state = Some(state);
            active.owner_fields.pitch = Some(127);
            active.owner_fields.yaw = Some(yaw);
        }
        if plan.session.new_inventory_handle.is_some() {
            active.current_inventory_handle = plan.session.new_inventory_handle;
            active.spawned_loadout = plan.session.new_spawned_loadout;
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
            || plan.session.corrected_owner_position.is_some()
            || plan.session.new_mounted_seat.is_some()
            || plan.session.new_ground_seat.is_some()
            || plan.combat.new_life.is_some_and(|life| !life.is_alive())
        {
            let generation = match active.life {
                CharacterLife::Alive { generation, .. }
                | CharacterLife::Incapacitated { generation, .. }
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
        if let Some((point, deadline, ordinal)) = plan.parachute_spawn {
            self.parachute_waves.insert((point, deadline), ordinal + 1);
            self.prune_parachute_waves();
        }
        self.advance_forward_spawns(now)?;
        // Expose persistence work only once the complete event commit succeeds,
        // including its remaining fallible publication/progression steps.
        if let Some(receipt) = plan.session.loadout_selection {
            let active = self
                .active
                .as_mut()
                .ok_or("missing_loadout_selection_owner")?;
            active.loadout_selection_revision = receipt.revision;
            active.pending_loadout_selection = Some(receipt);
        }
        if let Some(mut runtime) = plan.ground_runtime {
            let accepts = plan
                .ground_dynamics_relay
                .as_ref()
                .is_none_or(|(index, _)| {
                    self.active
                        .as_ref()
                        .is_some_and(|actor| self.ground_driver_authority(actor, *index))
                });
            if !accepts
                && let Some((index, _)) = &plan.ground_dynamics_relay
                && let Some(before) = &self.ground_runtime
            {
                runtime.discard_driver_input(before, *index);
            }
            if accepts || plan.ground_damage_relay.is_some() {
                self.ground_runtime = Some(runtime);
                if let Some(packet) = plan.ground_damage_relay {
                    self.broadcast_bot_packet(packet);
                }
                if let Some((_, relay)) = plan.ground_dynamics_relay.filter(|_| accepts) {
                    for peer in self
                        .peers
                        .values_mut()
                        .filter(|peer| peer.initial_objects.is_some())
                    {
                        peer.queue_shared(relay.clone());
                    }
                }
            }
        }
        if let Some(runtime) = plan.helo_runtime {
            let world = self
                .domination_gameplay
                .as_mut()
                .ok_or("domination_helo_world_required")?;
            world.state.helo = Some(runtime);
        }
        for packet in plan.helo_damage_relay {
            self.broadcast_bot_packet(packet);
        }
        // Manual seat transitions already carry the same-frame vehicle
        // ownership correction. Incapacitation clears it with a shared delta.
        if !seating_transition {
            for (index, owner) in previously_driving {
                if self.ground_driver_owner(index) != Some(owner) {
                    self.queue_ground_driver_revocation(index, now)?;
                }
            }
        }
        if let Some(clocks) = plan.strategic_waves {
            self.team_waves = clocks;
        }
        if let Some(runtime) = plan.active_ability_runtime {
            self.active_abilities = Some(runtime);
        }
        if let Some(candidate) = plan.squad_order_candidate {
            self.install_squad_order_candidate(candidate);
        }
        if let Some(packet) = plan.replication.ordered_health {
            for recipient in self
                .active
                .iter_mut()
                .chain(self.peers.values_mut())
                .filter(|a| a.initial_objects.is_some())
            {
                recipient.queue_shared(packet.clone());
            }
        }
        Ok(plan.messages)
    }
}

#[cfg(test)]
mod ground_grenade_commit_tests {
    use super::*;

    #[test]
    fn accepted_ground_grenade_survives_same_event_driver_death_without_driver_motion() {
        let mut engine = crate::spawn_progression::tests::dynamic_bodies::acquisition_gameplay_tests::ground_grenade_driver_fixture();
        let active = engine.active.as_ref().unwrap();
        assert!(engine.ground_driver_authority(active, 4));
        let player = active.reservation.player;
        let event = EventInput {
            reservation_id: active.reservation.id.clone(),
            world_id: active.reservation.world,
            player_id: player,
            transport: Transport::Udp,
            direction: Direction::ClientToServer,
            rt_type: 0x3d,
            plugin_id: Some(1),
            payload_hex: String::new(),
        };
        let now = Duration::from_millis(3400);
        let initial = engine.ground_runtime.as_ref().unwrap();
        let truck = initial.get(4).unwrap();
        let original_pose = truck.pose;
        let mut candidate = initial.clone();
        let destination = crate::acquisition_ground_runtime::Pose {
            position: replication::CharacterPosition {
                x: original_pose.position.x + 1.0,
                ..original_pose.position
            },
            ..original_pose
        };
        candidate.force_pose(truck.key, destination).unwrap();
        let terminals = [projectile::ProjectileTerminal {
            item_id: 4,
            slot: 0,
            generation: 1,
            position: Some(original_pose.position),
            reason: projectile::ProjectileTerminalReason::ReportedDetonation,
        }];
        let (packet, _) = engine
            .stage_ground_grenade_damage(&mut candidate, active, &terminals, now)
            .unwrap();
        assert!(packet.is_some());
        let mut plan = EventPlan::new(active);
        plan.ground_runtime = Some(candidate);
        plan.ground_damage_relay = packet;
        // An independently resolved same-event death disqualifies driver input.
        plan.ground_dynamics_relay = Some((4, vec![0]));
        plan.combat.new_life = Some(CharacterLife::AwaitingRespawn { generation: 1 });
        let decoded = DecodedEvent::with_profile(&[], player, engine.wire_profile, true);
        engine.commit_event(&event, &decoded, plan, now).unwrap();
        let truck = engine.ground_runtime.as_ref().unwrap().get(4).unwrap();
        assert!(truck.pending_death);
        assert_eq!(truck.pose, original_pose);
        assert!(engine.active.as_ref().unwrap().ground_seat.is_none());
    }
}
