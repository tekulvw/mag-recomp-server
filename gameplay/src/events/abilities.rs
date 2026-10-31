//! Typed ability requests are observed without inventing an activation reply.
//! Native activation publishes ability state; effects/target predicates are a
//! separate next increment. No request here consumes or resets cooldown state.
use super::*;

pub(super) fn command<W: Write>(
    ctx: &PlanningContext<'_, W>,
    plan: &mut EventPlan,
    request: crate::active_abilities::CommandRequest,
) -> Result<Value, &'static str> {
    plan.gameplay_denial = Some("command_ability_execution_pending");
    Ok(
        json!({"type":"command_ability","squad_index":request.squad_index,
        "ability":request.ability,"position":request.position,"request_time":request.request_time,
        "target_object_id":request.target_id,"actor_index":ctx.active.reservation.player,
        "actor_squad_index":ctx.active.squad_catalog.squad_index(),
        "recognized":true,"accepted":false,"effects_implemented":false,
        "reason":"command_ability_execution_pending"}),
    )
}

pub(super) fn strategic<W: Write>(
    ctx: &PlanningContext<'_, W>,
    plan: &mut EventPlan,
    request: crate::active_abilities::StrategicRequest,
) -> Result<Value, &'static str> {
    plan.gameplay_denial = Some("strategic_ability_execution_pending");
    Ok(
        json!({"type":"strategic_ability","ability_code":request.ability_code,
        "actor_index":ctx.active.reservation.player,"recognized":true,"accepted":false,
        "effects_implemented":false,"reason":"strategic_ability_execution_pending"}),
    )
}
