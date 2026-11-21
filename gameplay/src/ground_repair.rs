//! Manual live Ground repair; HP and credit share one recorded transaction.
use crate::*;

struct Candidate {
    ground: acquisition_ground_runtime::Runtime,
    packet: Option<Vec<u8>>,
    leadership: leadership_runtime::LeadershipPlan,
    cooldown: Duration,
    modifier: f32,
    position: [f32; 3],
    last_attacker: u32,
    before_hp: f32,
    after_hp: f32,
    percent: f32,
    source: Value,
}
impl<W: Write> Engine<W> {
    fn prepare_ground_repair(
        &self,
        event: &events::EventInput,
        request: protocol::Repair,
        now: Duration,
    ) -> Result<Candidate, &'static str> {
        let actor = self.active.as_ref().ok_or("no_reservation")?;
        if actor.wire_profile.application_id() != 20231
            || !matches!(event.transport, Transport::Tcp)
            || !matches!(event.direction, Direction::ClientToServer)
            || event.rt_type != 0x3d
            || event.plugin_id != Some(1)
        {
            return Err("ground_repair_requires_native_client_route");
        }
        if (self.acquisition_gameplay.is_none() && self.domination_gameplay.is_none())
            || !self.match_started
            || self.match_terminal.is_some()
            || !self.round_combat_allowed(now)
        {
            return Err("ground_repair_requires_open_match");
        }
        if !matches!(
            actor.life,
            CharacterLife::Alive {
                health_quarter_points: 1..,
                ..
            }
        ) || actor.initial_objects.is_none()
            || !self.ground_vehicle_creation_complete(actor.ground_vehicle_batch)
        {
            return Err("ground_repair_requires_created_live_actor");
        }
        let tool = self
            .acquisition_structural_repair_tool()
            .or_else(|| {
                self.bunker_world
                    .as_ref()
                    .map(structures::BunkerWorld::repair_tool_profile)
            })
            .ok_or("ground_repair_tool_source_required")?;
        let (_, leadership) = self.plan_structural_repair_authority(Some(tool), now)?;
        if now
            < self
                .next_repair
                .get(&actor.reservation.id)
                .copied()
                .unwrap_or_default()
        {
            return Err("repair_cooldown");
        }
        let index = request.target_object_id as u16;
        let catalog = self
            .ground_vehicles
            .as_ref()
            .ok_or("ground_vehicle_catalog_required")?;
        let creation = catalog
            .vehicle(index)
            .filter(|c| c.object_id() == request.target_object_id)
            .ok_or("unknown_ground_repair_target")?;
        let source = catalog
            .vehicle_source(index)
            .ok_or("ground_repair_source_required")?;
        let supported = if self.domination_gameplay.is_some() {
            matches!(source.spawn_type, 3 | 7)
        } else {
            source.spawn_type == 9
        };
        if !supported || actor.team() != creation.team {
            return Err("ground_repair_requires_allied_supported_vehicle");
        }
        let before = self
            .ground_runtime
            .as_ref()
            .ok_or("ground_vehicle_runtime_required")?;
        let vehicle = before.get(index).ok_or("unknown_ground_repair_target")?;
        if vehicle.state != 2
            || vehicle.body != acquisition_ground_runtime::Body::Operational
            || vehicle.is_dead()
            || vehicle.is_dying()
        {
            return Err("ground_repair_target_not_operational");
        }
        let maximum_hp = f32::from(vehicle.maximum_health_quarter_points) / 4.0;
        if vehicle.health_hp() >= maximum_hp {
            return Err("ground_repair_target_already_healthy");
        }
        let modifier = leadership.modifiers(actor).value("kCharModRepairAmount");
        if !modifier.is_finite() || modifier < -1.0 {
            return Err("invalid_vehicle_repair_parameters");
        }
        // The native Ground time virtual returns -1: preserve the instant kit
        // FMA and percent conversion before the common vehicle Heal operation.
        let amount = tool.amount.mul_add(modifier, tool.amount);
        let percent = (amount * 100.0_f32) / maximum_hp.max(1.0);
        if !percent.is_finite() {
            return Err("invalid_vehicle_repair_amount");
        }
        let interval = Duration::try_from_secs_f32(tool.cooldown_seconds)
            .map_err(|_| "invalid_vehicle_repair_cooldown")?;
        let cooldown = now
            .checked_add(interval)
            .ok_or("repair_cooldown_overflow")?;
        let mut ground = before.clone();
        let effects = ground.heal_percent(vehicle.key, percent)?;
        let after_hp = ground.get(index).expect("owned Ground clone").health_hp();
        let changes = if effects.is_empty() {
            Default::default()
        } else {
            [(index, 1 << 18)].into()
        };
        let packet = self.encode_ground_changes(&ground, &changes, now)?;
        let source = json!({"generation":vehicle.key.generation,"spawn_type":source.spawn_type,
            "spawn_point_token":source.spawn_point_token,"vehicle_type_token":creation.vehicle_type_token,
            "faction":creation.faction,"team":creation.team,"repair_item":tool.item_id,
            "base_amount_hp":tool.amount,"modifier":modifier,"last_attacker":vehicle.last_attacker});
        Ok(Candidate {
            ground,
            packet,
            leadership,
            cooldown,
            modifier,
            position: frago::point(vehicle.pose.position),
            last_attacker: vehicle.last_attacker,
            before_hp: vehicle.health_hp(),
            after_hp,
            percent,
            source,
        })
    }

    pub(crate) fn event_ground_repair(
        &mut self,
        event: &events::EventInput,
        request: protocol::Repair,
        now: Duration,
        wall: u64,
    ) -> Result<Vec<OutboundMessage>, &'static str> {
        let mut audit = json!({"op":"ground_repair","reservation_id":event.reservation_id,
            "world_id":event.world_id,"player_id":event.player_id,"target":request.target_object_id,
            "transport":event.transport,"direction":event.direction,"rt_type":event.rt_type,
            "plugin_id":event.plugin_id,"payload_hex":event.payload_hex.to_ascii_lowercase()});
        let candidate = match self.prepare_ground_repair(event, request, now) {
            Ok(candidate) => candidate,
            Err(code) => {
                audit["gameplay_denial"] = json!(code);
                self.record(audit, wall)?;
                return Ok(Vec::new());
            }
        };
        let actor = self.active.as_ref().ok_or("no_reservation")?;
        let increased = candidate.after_hp > candidate.before_hp;
        let credit = if increased
            && !self.attacker_is_friendly(candidate.last_attacker, event.world_id, actor.team())
        {
            Some(self.plan_repair_credit(actor, candidate.modifier, Some(candidate.position))?)
        } else {
            None
        };
        audit["health_before_hp"] = json!(candidate.before_hp);
        audit["health_after_hp"] = json!(candidate.after_hp);
        audit["percent"] = json!(candidate.percent);
        audit["source"] = candidate.source;
        audit["health_increased"] = json!(increased);
        audit["leadership_changes"] = json!(candidate.leadership.audit());
        audit["cooldown_deadline_ms"] = json!(candidate.cooldown.as_millis());
        audit["credit"] = credit
            .as_ref()
            .map_or(Value::Null, repair_rewards::Plan::audit);
        audit["ground_update"] = json!(candidate.packet.as_ref().map(|p| protocol::encode_hex(p)));
        self.record(audit, wall)?;
        // All candidate validation and serialization precede the durable record.
        self.ground_runtime = Some(candidate.ground);
        self.next_repair
            .insert(event.reservation_id.clone(), candidate.cooldown);
        self.commit_leadership(candidate.leadership);
        self.active
            .as_mut()
            .expect("authenticated repair actor")
            .last_event = now;
        let messages = credit.map_or_else(Vec::new, |plan| self.install_repair_credit(plan));
        if let Some(packet) = candidate.packet {
            for recipient in self
                .active
                .iter_mut()
                .chain(self.peers.values_mut())
                .filter(|a| {
                    a.reservation.world == event.world_id
                        && a.ready_for_updates
                        && a.initial_objects.is_some()
                })
            {
                recipient.queue_shared(packet.clone());
            }
        }
        Ok(messages)
    }
}
