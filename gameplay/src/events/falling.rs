//! Transactional owner-state landing detection; no inferred client collision.
use super::*;

pub(super) fn plan_report<W: Write>(
    ctx: &PlanningContext<'_, W>,
    plan: &mut EventPlan,
    header: replication::UpdateHeader,
    fields: &player_tracking::ClientCharacterFields,
) -> Result<(), &'static str> {
    let active = ctx.active;
    let Some(rules) = &ctx.engine.fall_damage_rules else {
        return Ok(());
    };
    if !ctx.accepted_owner_route
        || !active.tracking.accepts(header.sequence)
        || active.wire_profile.application_id() != rules.application_id
        || header.client_index != 0
        || header.nak_sequence != 0
        || header.sequence <= active.latest_owner_sequence
        || !active.ready_for_updates
        || !active.entered_game
        || active.mounted_seat.is_some()
        || (fields.state.is_none() && fields.position.is_none())
    {
        return Ok(());
    }
    let CharacterLife::Alive { generation, .. } = active.life else {
        return Ok(());
    };
    let mut tracker = active.fall_tracker.clone();
    let hit = tracker.observe(
        rules,
        generation,
        header.sequence,
        fields.state.map(|s| s.state),
        fields.position.map(|p| p.y),
    )?;
    plan.session.fall_tracker = Some(tracker);
    if let Some(hit) = hit {
        let character_id = 0x03800000 | u32::from(active.reservation.player);
        plan.audit.falling_damage = Some(hit);
        plan.combat.queued_fall = Some(PendingDamage {
            target_character_id: character_id,
            target_reservation_id: Some(active.reservation.id.clone()),
            id: format!("fall-{}-{}", generation, header.sequence),
            generation,
            falling: true,
            damage: damage::ResolvedDamage {
                amount_hp: hit.amount,
                kind: damage::DamageKind::ExternalKinetic,
                kinetic_response: false,
                location: 0,
                attacker_id: character_id,
                source_object_id: u32::MAX,
                user_data: 0,
            },
        });
    }
    Ok(())
}
