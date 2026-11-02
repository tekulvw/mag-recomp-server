//! Record-before-commit owner field5 correction ACK and pending data release.
use super::*;

pub(super) fn plan<W: Write>(
    ctx: &PlanningContext<'_, W>,
    plan: &mut EventPlan,
    ack: crate::correction_ack::CorrectionAck,
) -> Result<Value, &'static str> {
    if !ctx.accepted_client_route || !ctx.active.life.is_alive() {
        plan.gameplay_denial = Some("correction_ack_not_current_owner");
        return Ok(json!({"type":"correction_ack","accepted":false}));
    }
    let active = ctx.active;
    let mut tracking = active.tracking.clone();
    let mut results = Vec::new();
    let mut changed = false;
    for item in ack.items {
        let accepted = tracking.acknowledge_correction(
            item,
            active.reservation.player,
            active.latest_owner_sequence,
        );
        changed |= accepted;
        results.push(json!({"server_sequence":item.server_sequence,"client_sequence":item.client_sequence,
            "object_type":item.object_type,"object_index":item.object_index,"field_index":item.field_index,"accepted":accepted}));
    }
    if changed {
        if let Some(sample) = tracking
            .sample
            .filter(|s| matches!(s.origin, player_tracking::PositionOrigin::ClientReported))
        {
            plan.session.corrected_owner_position = Some(sample.position);
            let fields = player_tracking::ClientCharacterFields {
                position: Some(sample.position),
                ..Default::default()
            };
            plan.session.correction_publication = Some(
                replication::multiplayer::CharacterOwnerDelta {
                    sequence: 1,
                    timestamp_ms: ctx.now.as_millis() as u32,
                    player: active.reservation.player,
                    fields: &fields,
                }
                .encode_with_profile(ctx.engine.wire_profile)?,
            );
        }
        plan.session.correction_tracking = Some(tracking);
    }
    Ok(json!({"type":"correction_ack","accepted":changed,"items":results}))
}
