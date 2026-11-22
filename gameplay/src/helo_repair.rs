//! Manual operational Helo repair using the ordinary vehicle repair request.
use crate::{
    domination_helo::FlightState,
    domination_helo_damage::{HullDamage, damage_delta},
    *,
};

struct Candidate {
    index: u16,
    damage: HullDamage,
    packet: Option<Vec<u8>>,
    leadership: leadership_runtime::LeadershipPlan,
    credit: Option<repair_rewards::Plan>,
    cooldown: Duration,
    health_before: f32,
    percent: f32,
    position: [f32; 3],
}

impl<W: Write> Engine<W> {
    fn prepare_helo_repair(
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
            return Err("helo_repair_requires_native_client_route");
        }
        let world = self
            .domination_gameplay
            .as_ref()
            .ok_or("helo_repair_requires_domination")?;
        if !self.match_started || self.match_terminal.is_some() || !self.round_combat_allowed(now) {
            return Err("helo_repair_requires_open_match");
        }
        if !matches!(
            actor.life,
            CharacterLife::Alive {
                health_quarter_points: 1..,
                ..
            }
        ) || actor.initial_objects.is_none()
            || !actor.helo_creation_sent
        {
            return Err("helo_repair_requires_created_live_actor");
        }
        let structures = self
            .bunker_world
            .as_ref()
            .ok_or("helo_repair_tool_source_required")?;
        let tool = structures.repair_tool_profile();
        let (_, leadership) = self.plan_structural_repair_authority(Some(tool), now)?;
        if self
            .next_repair
            .get(&actor.reservation.id)
            .is_some_and(|&due| now < due)
        {
            return Err("repair_cooldown");
        }

        let state = &world.state;
        let index = request.target_object_id as u16;
        let carrier = state
            .helo
            .as_ref()
            .and_then(|fleet| fleet.carriers.get(usize::from(index)))
            .filter(|carrier| {
                carrier.source.object_index == index
                    && request.target_object_id & 0xff7f_ffff == (0x0200_0000 | u32::from(index))
            })
            .ok_or("unknown_helo_repair_target")?;
        if actor.team() != carrier.creation(0, 0).base.team {
            return Err("helo_repair_requires_allied_vehicle");
        }
        let pose = carrier
            .current_pose
            .filter(|_| {
                carrier.published_state != FlightState::Dormant && carrier.live_basis.is_some()
            })
            .ok_or("helo_repair_target_not_operational")?;
        if !carrier.damage.accepts_manual_repair() {
            return Err("helo_repair_target_not_operational");
        }
        let rules = state
            .helo_damage
            .as_ref()
            .ok_or("helo_repair_damage_source_required")?;
        let profile = rules.profile(carrier.source.faction)?;
        let health_before = carrier.damage.health_hp();
        if health_before >= profile.max_health {
            return Err("helo_repair_target_already_healthy");
        }

        let modifier = leadership.modifiers(actor).value("kCharModRepairAmount");
        if !modifier.is_finite() || modifier < -1.0 {
            return Err("invalid_vehicle_repair_parameters");
        }
        let amount = tool.amount.mul_add(modifier, tool.amount);
        let percent = (amount * 100.0_f32) / profile.max_health.max(1.0);
        let interval = Duration::try_from_secs_f32(tool.cooldown_seconds)
            .map_err(|_| "invalid_vehicle_repair_cooldown")?;
        let cooldown = now
            .checked_add(interval)
            .ok_or("repair_cooldown_overflow")?;

        let mut damage = carrier.damage;
        let changed = damage.heal_percent(profile, structures.damage_catalog().rules(), percent)?;
        let packet = changed
            .then(|| damage_delta(index, None, Some(damage.wire), now, self.wire_profile))
            .transpose()?;
        let position = frago::point(pose.position);
        let credit = if changed
            && !self.attacker_is_friendly(damage.wire.last_attacker, event.world_id, actor.team())
        {
            Some(self.plan_repair_credit(actor, modifier, Some(position))?)
        } else {
            None
        };
        Ok(Candidate {
            index,
            damage,
            packet,
            leadership,
            credit,
            cooldown,
            health_before,
            percent,
            position,
        })
    }

    pub(crate) fn event_helo_repair(
        &mut self,
        event: &events::EventInput,
        request: protocol::Repair,
        now: Duration,
        wall: u64,
    ) -> Result<Vec<OutboundMessage>, &'static str> {
        let mut audit = json!({"op":"helo_repair","reservation_id":event.reservation_id,
            "world_id":event.world_id,"player_id":event.player_id,"target":request.target_object_id,
            "transport":event.transport,"direction":event.direction,"rt_type":event.rt_type,
            "plugin_id":event.plugin_id,"payload_hex":event.payload_hex.to_ascii_lowercase()});
        let candidate = match self.prepare_helo_repair(event, request, now) {
            Ok(candidate) => candidate,
            Err(code) => {
                audit["gameplay_denial"] = json!(code);
                self.record(audit, wall)?;
                return Ok(Vec::new());
            }
        };
        audit["health_before_hp"] = json!(candidate.health_before);
        audit["health_after_hp"] = json!(candidate.damage.health_hp());
        audit["last_attacker"] = json!(candidate.damage.wire.last_attacker);
        audit["percent"] = json!(candidate.percent);
        audit["position"] = json!(candidate.position);
        audit["leadership_changes"] = json!(candidate.leadership.audit());
        audit["cooldown_deadline_ms"] = json!(candidate.cooldown.as_millis());
        audit["credit"] = candidate
            .credit
            .as_ref()
            .map_or(Value::Null, repair_rewards::Plan::audit);
        audit["helo_update"] = json!(candidate.packet.as_ref().map(|p| protocol::encode_hex(p)));
        self.record(audit, wall)?;

        let fleet = self
            .domination_gameplay
            .as_mut()
            .unwrap()
            .state
            .helo
            .as_mut()
            .unwrap();
        fleet.carriers[usize::from(candidate.index)].damage = candidate.damage;
        self.next_repair
            .insert(event.reservation_id.clone(), candidate.cooldown);
        self.commit_leadership(candidate.leadership);
        self.active.as_mut().unwrap().last_event = now;
        let messages = candidate
            .credit
            .map_or_else(Vec::new, |plan| self.install_repair_credit(plan));
        if let Some(packet) = candidate.packet {
            for recipient in self
                .active
                .iter_mut()
                .chain(self.peers.values_mut())
                .filter(|actor| {
                    actor.reservation.world == event.world_id
                        && actor.ready_for_updates
                        && actor.initial_objects.is_some()
                        && actor.helo_creation_sent
                })
            {
                recipient.queue_shared(packet.clone());
            }
        }
        Ok(messages)
    }
}
