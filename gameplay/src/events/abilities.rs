//! Plan authenticated ability requests. The recorded runtime candidate publishes
//! its native state through the regular world update and bootstrap paths.
use super::*;

pub(super) fn command<W: Write>(
    ctx: &PlanningContext<'_, W>,
    plan: &mut EventPlan,
    request: crate::active_abilities::CommandRequest,
) -> Result<Value, &'static str> {
    let (runtime, decision, denial) = ctx
        .engine
        .plan_command_ability(ctx.active, request, ctx.now)?;
    let Value::Object(mut decoded) = decision else {
        return Err("invalid_command_ability_decision");
    };
    decoded.insert("type".into(), json!("command_ability"));
    decoded.insert("squad_index".into(), json!(request.squad_index));
    decoded.insert("ability".into(), json!(request.ability));
    decoded.insert("position".into(), json!(request.position));
    decoded.insert("request_time".into(), json!(request.request_time));
    decoded.insert("target_object_id".into(), json!(request.target_id));
    decoded.insert("actor_index".into(), json!(ctx.active.reservation.player));
    decoded.insert(
        "actor_squad_index".into(),
        json!(ctx.active.squad_catalog.squad_index()),
    );
    decoded.insert("recognized".into(), json!(true));
    decoded.insert("accepted".into(), json!(runtime.is_some()));
    plan.gameplay_denial = denial;
    decoded.insert(
        "effects_implemented".into(),
        json!(
            ctx.engine.active_abilities.is_some()
                && matches!(
                    request.ability,
                    crate::active_abilities::Ability::ClusterBomb
                        | crate::active_abilities::Ability::Sensor
                )
        ),
    );
    plan.active_ability_runtime = runtime;
    Ok(Value::Object(decoded))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        active_abilities::{Ability, CommandRequest},
        spawn_progression::dispatch,
    };

    fn request() -> [u8; 26] {
        CommandRequest {
            squad_index: 0,
            ability: Ability::ClusterBomb,
            position: [17., 5., 23.],
            request_time: 9000,
            target_id: u32::MAX,
        }
        .to_bytes()
        .unwrap()
    }

    fn last_event(engine: &Engine<Vec<u8>>) -> Value {
        let line = std::str::from_utf8(&engine.writer)
            .unwrap()
            .lines()
            .last()
            .unwrap();
        serde_json::from_str(line).unwrap()
    }

    #[test]
    fn disabled_command_ability_preserves_pending_denial_without_emitting_a_reply() {
        let mut engine = crate::human_kill_reward_tests::fixture();
        assert!(dispatch(&mut engine, &request(), 10).unwrap().is_empty());
        let event = last_event(&engine);
        assert_eq!(
            event["gameplay_denial"],
            "command_ability_execution_pending"
        );
        assert_eq!(event["decoded"]["type"], "command_ability");
        assert_eq!(event["decoded"]["accepted"], false);
        assert_eq!(event["decoded"]["effects_implemented"], false);
        assert_eq!(event["decoded"]["request_time"], 9000);
    }

    #[test]
    fn finished_round_rejects_command_ability_before_planning() {
        let mut engine = crate::human_kill_reward_tests::fixture();
        engine.match_terminal = Some(match_flow::MatchTerminal {
            world_id: 1,
            match_id: "synthetic-finished-round".into(),
            reason: "synthetic".into(),
            reservation_ids: vec![],
        });
        engine.round.outcome = Some(match_flow::Outcome {
            winner: 0,
            major: false,
            deadline: Duration::ZERO,
            finalized: true,
        });
        assert!(dispatch(&mut engine, &request(), 10).unwrap().is_empty());
        let event = last_event(&engine);
        assert_eq!(event["gameplay_denial"], "match_finished");
        assert_eq!(event["decoded"]["type"], "postgame_action");
    }
}
