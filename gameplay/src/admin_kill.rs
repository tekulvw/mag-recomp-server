//! Operator kill (`admin_kill`): Horizon asks the worker to end one admitted
//! human's current life, for example to free a player stuck in a parachute or
//! a vehicle seat that no longer exists. Players have no default key for Kill
//! Self, so the operator sends it for them.
//!
//! The worker applies the request exactly as if that player's client had sent
//! its own owned Kill Self (`0x0215`): the same event planner, death notice,
//! seat exit, downed give-up and respawn queue, and the same match statistics
//! (a death and a suicide) and no experience change. Only the
//! sender differs, and the journal records an `admin_kill` row naming the
//! operator and the reason before the event row.
//!
//! The request is refused, with nothing applied, when the reservation is not
//! a placed character of this world (`not_in_match`), when its match is over
//! (`match_finished`) or when the character is not alive or downed
//! (`not_alive`). A Kill Self the planner itself denies still answers `ok`
//! with its replies, because those replies may carry shared publications that
//! must be delivered; its outcome says `refused` and names the denial.
use super::*;

/// Discovery capability. Horizon sends `admin_kill` only to workers that
/// advertise it, so an older worker never sees an op it would reject.
pub const CAPABILITY: &str = "admin_kill_v1";
/// Longest accepted operator name and reason, in printable ASCII bytes.
pub const MAX_OPERATOR: usize = 254;
pub const MAX_REASON: usize = 256;

/// What an applied `admin_kill` did, sent only in its own reply.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    /// `killed` (an alive character died), `gave_up` (a downed character gave
    /// up its downed life) or `refused` (the Kill Self planner denied it).
    pub result: String,
    /// The planner's denial code for `refused`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

fn printable(value: &str) -> bool {
    value.bytes().all(|b| (0x20..=0x7e).contains(&b))
}

pub(crate) fn validate(operator: &str, reason: Option<&str>) -> Result<(), &'static str> {
    if operator.is_empty() || operator.len() > MAX_OPERATOR || !printable(operator) {
        return Err("invalid_admin_operator");
    }
    if reason.is_some_and(|r| r.len() > MAX_REASON || !printable(r)) {
        return Err("invalid_admin_reason");
    }
    Ok(())
}

/// The owned Kill Self message the player's own client would send.
pub(crate) fn kill_self_payload(player: u16) -> Vec<u8> {
    let mut wire = 0x0215u16.to_be_bytes().to_vec();
    wire.extend((0x0380_0000 | u32::from(player)).to_be_bytes());
    wire
}

impl<W: Write> Engine<W> {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn admin_kill(
        &mut self,
        request_id: String,
        reservation_id: String,
        world_id: u32,
        player_id: u16,
        operator: String,
        reason: Option<String>,
        now: Duration,
        wall_secs: u64,
    ) -> Response {
        if self.results_owner(&reservation_id).is_some() {
            return Response::error(request_id, "match_finished");
        }
        let Some(actor) = self
            .active
            .iter()
            .chain(self.peers.values())
            .find(|actor| actor.reservation.id == reservation_id)
        else {
            return Response::error(request_id, "not_in_match");
        };
        if actor.reservation.world != world_id
            || actor.reservation.player != player_id
            || !actor.entered_game
            || !actor.ready_for_updates
            || actor.initial_objects.is_none()
        {
            return Response::error(request_id, "not_in_match");
        }
        // Once the result is decided nothing is left to free: the player
        // moves to the results screen with the rest of the match.
        if self.match_terminal.is_some() {
            return Response::error(request_id, "match_finished");
        }
        let downed = match actor.life {
            CharacterLife::Alive { .. } => false,
            CharacterLife::Incapacitated { .. } => true,
            CharacterLife::AwaitingFirstSpawn | CharacterLife::AwaitingRespawn { .. } => {
                return Response::error(request_id, "not_alive");
            }
        };
        let generation = actor.life.damage_health().map(|(generation, _)| generation);
        if let Err(code) = self.record(
            json!({"op":"admin_kill","reservation_id":reservation_id,"world_id":world_id,
                "player_id":player_id,"operator":operator,"reason":reason,
                "life_before":if downed { "downed" } else { "alive" }}),
            wall_secs,
        ) {
            return Response::error(request_id, code);
        }
        let event = Request::Event {
            v: BRIDGE_VERSION,
            request_id: request_id.clone(),
            reservation_id: reservation_id.clone(),
            world_id,
            player_id,
            transport: Transport::Tcp,
            direction: Direction::ClientToServer,
            rt_type: 0x3d,
            plugin_id: Some(1),
            payload_hex: protocol::encode_hex(&kill_self_payload(player_id)),
        };
        self.last_gameplay_denial = None;
        let mut response = self.handle(event, now, wall_secs);
        if !response.ok {
            return response;
        }
        let died = self
            .active
            .iter()
            .chain(self.peers.values())
            .find(|actor| actor.reservation.id == reservation_id)
            .is_some_and(|actor| {
                matches!(actor.life, CharacterLife::AwaitingRespawn { generation: g } if Some(g) == generation)
            });
        response.admin_kill = Some(if died {
            Outcome {
                result: if downed { "gave_up" } else { "killed" }.into(),
                reason: None,
            }
        } else {
            Outcome {
                result: "refused".into(),
                reason: Some(
                    self.last_gameplay_denial
                        .unwrap_or("kill_self_not_applied")
                        .into(),
                ),
            }
        });
        response
    }
}

#[cfg(test)]
#[path = "admin_kill_tests.rs"]
mod tests;
