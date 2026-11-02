//! Target-only resuscitation. The authenticated reservation supplies the healer.
use super::*;
pub(super) fn plan<W: Write>(
    ctx: &PlanningContext<'_, W>,
    plan: &mut EventPlan,
    request: protocol::Resuscitate,
) -> Result<Value, &'static str> {
    let healer = ctx.active;
    let patient = ctx.engine.peers.values().find(|p| {
        request.patient == (0x0380_0000 | u32::from(p.reservation.player))
            && p.reservation.world == healer.reservation.world
    });
    let denial = if !ctx.accepted_client_route {
        Some("revive_requires_authenticated_client_route")
    } else if ctx.engine.incapacitation_rules.is_none() {
        Some("incapacitation_not_enabled")
    } else if !healer.entered_game
        || !healer.ready_for_updates
        || !healer.life.is_alive()
        || healer.mounted_seat.is_some()
    {
        Some("revive_requires_living_healer")
    } else if !ctx.history_available {
        Some("replication_history_limit")
    } else {
        None
    };
    if let Some(code) = denial {
        plan.gameplay_denial = Some(code);
        return Ok(
            json!({"type":"resuscitate","accepted":false,"patient":request.patient,"reason":code}),
        );
    }
    let Some(patient) = patient else {
        plan.gameplay_denial = Some("revive_patient_unavailable");
        return Ok(json!({"type":"resuscitate","accepted":false,"patient":request.patient}));
    };
    let CharacterLife::Incapacitated { generation, downed } = patient.life else {
        plan.gameplay_denial = Some("revive_patient_not_incapacitated");
        return Ok(json!({"type":"resuscitate","accepted":false,"patient":request.patient}));
    };
    if patient.team() != healer.team()
        || !patient.ready_for_updates
        || !patient.entered_game
        || downed.is_expired(ctx.now)
    {
        plan.gameplay_denial = Some("revive_patient_ineligible");
        return Ok(json!({"type":"resuscitate","accepted":false,"patient":request.patient}));
    }
    let source = plan.leadership.modifiers(healer);
    let recipient = plan.leadership.modifiers(patient);
    let health = match incapacitation::resuscitated_health_quarter_points(
        source.max_health_hp(),
        recipient.max_health_hp(),
        source.value("kCharModResuscitatePercent"),
    ) {
        Ok(health) if health > 0 => health,
        _ => {
            plan.gameplay_denial = Some("revive_requires_owned_modifier");
            return Ok(json!({"type":"resuscitate","accepted":false,"patient":request.patient}));
        }
    };
    let damage = replication::CharacterDamage {
        last_location_and_type: (7 << 4) | (patient.last_damage.last_location_and_type & 15),
        health_quarter_points: health,
        conditions: patient.last_damage.conditions & !2,
        ..patient.last_damage
    };
    let packet = replication::CharacterDamageUpdate {
        sequence: ctx.next_sequence,
        timestamp_ms: ctx.now.as_millis() as u32,
        object_index: patient.reservation.player,
        damage,
        forced_state: None,
    }
    .encode_with_profile(ctx.engine.wire_profile)?;
    plan.combat.revived_peer = Some((patient.reservation.id.clone(), generation, damage));
    plan.replication.damage_delta = Some(packet.clone());
    plan.outgoing = Some(packet);
    let notice = protocol::RetailResuscitationNotification {
        healer: healer.reservation.player,
        patient: patient.reservation.player,
    }
    .encode()?;
    for member in ctx
        .engine
        .active
        .iter()
        .chain(ctx.engine.peers.values())
        .filter(|a| {
            a.reservation.world == healer.reservation.world
                && a.ready_for_updates
                && a.initial_objects.is_some()
                && [
                    healer.squad_catalog.squad_index(),
                    patient.squad_catalog.squad_index(),
                ]
                .contains(&a.squad_catalog.squad_index())
        })
    {
        plan.messages.push(OutboundMessage {
            reservation_id: member.reservation.id.clone(),
            world_id: member.reservation.world,
            player_id: member.reservation.player,
            plugin_id: 1,
            payload_hex: protocol::encode_hex(&notice),
        });
    }
    statistics::resuscitated(plan, healer);
    let sequence = healer
        .healing_sequence
        .checked_add(1)
        .ok_or("healing_sequence_exhausted")?;
    plan.combat.new_healing_sequence = Some(sequence);
    if let Some(recipient) = progression::RewardRecipient::from_active(healer) {
        plan.combat.new_rewards.push(recipient.event(
            format!("revive-{}-{sequence:016x}", healer.reservation.id),
            progression::RewardKind::TeammateResuscitated,
            1,
        ));
    }
    Ok(
        json!({"type":"resuscitate","accepted":true,"healer":healer.reservation.player,
        "patient":request.patient,"generation":generation,"health_quarter_points":health,
        "notification_hex":protocol::encode_hex(&notice),"forced_state":false}),
    )
}
