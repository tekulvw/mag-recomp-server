use super::*;

fn decide(
    plan: &mut EventPlan,
    kind: &str,
    candidate: Result<crate::squad_order_engine::Candidate, &'static str>,
) -> Value {
    match candidate {
        Ok(candidate) => {
            plan.squad_order_candidate = Some(candidate);
            json!({"type":kind,"accepted":true})
        }
        Err(reason) => {
            plan.gameplay_denial = Some(reason);
            json!({"type":kind,"accepted":false,"reason":reason})
        }
    }
}
pub(super) fn tagged<W: Write>(
    ctx: &PlanningContext<'_, W>,
    plan: &mut EventPlan,
    request: protocol::TaggedObject,
) -> Result<Value, &'static str> {
    let candidate = if ctx.accepted_client_route {
        ctx.engine
            .plan_squad_tag(ctx.active, request.target_id, ctx.now.as_millis() as u32)
    } else {
        Err("squad_order_wrong_route")
    };
    Ok(decide(plan, "tagged_object", candidate))
}
pub(super) fn waypoint<W: Write>(
    ctx: &PlanningContext<'_, W>,
    plan: &mut EventPlan,
    request: protocol::SquadWaypoint,
) -> Result<Value, &'static str> {
    let candidate = if ctx.accepted_client_route {
        ctx.engine.plan_squad_waypoint(
            ctx.active,
            request.position,
            request.clear,
            ctx.now.as_millis() as u32,
        )
    } else {
        Err("squad_order_wrong_route")
    };
    Ok(decide(plan, "squad_waypoint", candidate))
}
