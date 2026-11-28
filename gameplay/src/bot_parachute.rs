//! Bot parachute descents (docs/bot-active-spawns.md, "Parachute points").
//!
//! A bot life that arrives at a parachute drop zone starts in the air, as a
//! human parachutist does, instead of standing on its landing spot:
//!
//! - Its respawn carries the parachute character state (17, the state a human
//!   parachute respawn carries), the drop zone's parachute yaw, and a
//!   position straight above its landing spot at the drop zone's authored
//!   airborne height.
//! - It comes down at a steady `DESCENT_MPS`, publishing its position every
//!   `PUBLISH_INTERVAL` to the recipients that hold its snapshot. A recipient
//!   that takes its snapshot mid-descent gets the parachute state and the
//!   current height in it.
//! - It touches down on its landing spot when the descent time is up and
//!   publishes standing state 1 there. Its life then goes on as usual.
//!
//! While descending its body is not grounded: it does not walk, fire,
//! operate objectives, take cover or react to threats, and other bots do not
//! aim at it. Its body is reserved at its landing spot from the start, so no
//! other life lands on top of it. Damage still applies; a bot killed in the
//! air dies there like any other bot.
//!
//! The server has no steering model: the descent is vertical. Human
//! parachutists drift forward as they steer; bots land where the drop zone's
//! landing placements are.
use super::*;

/// Vertical speed of a bot under its parachute. Human parachute drops in
/// live Domination came down 39-41 m in 5.8-6.9 s (5.9-6.7 m/s).
pub(crate) const DESCENT_MPS: f64 = 6.4;
/// How often a descending bot publishes its height.
pub(crate) const PUBLISH_INTERVAL: Duration = Duration::from_millis(250);
/// A drop zone this close above the landing spot is arrived at standing.
const MIN_DROP_M: f64 = 1.0;
/// The character state a human parachute respawn carries.
pub(crate) const PARACHUTE_STATE: u8 = 17;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Descent {
    /// The airborne height the descent starts at.
    pub(crate) top: f32,
    pub(crate) landing: replication::CharacterPosition,
    pub(crate) started: Duration,
    pub(crate) lands: Duration,
    pub(crate) published: Duration,
    /// The grounded body at the landing spot, restored on touchdown.
    pub(crate) motion: bot_physics::Motion,
}
impl Descent {
    /// The descent onto `landing` from `airborne_y`, or `None` when the drop
    /// zone is not above it.
    pub(crate) fn plan(
        landing: replication::CharacterPosition,
        airborne_y: f32,
        now: Duration,
        motion: bot_physics::Motion,
    ) -> Option<Self> {
        let drop = f64::from(airborne_y) - f64::from(landing.y);
        (drop.is_finite() && drop > MIN_DROP_M).then(|| Self {
            top: airborne_y,
            landing,
            started: now,
            lands: now + Duration::from_secs_f64(drop / DESCENT_MPS),
            published: now,
            motion,
        })
    }
    pub(crate) fn landed(&self, now: Duration) -> bool {
        now >= self.lands
    }
    /// The wire position at `now`: straight above the landing spot.
    pub(crate) fn position(&self, now: Duration) -> replication::CharacterPosition {
        let total = self.lands.saturating_sub(self.started).as_secs_f64();
        let left = self.lands.saturating_sub(now).as_secs_f64();
        let fraction = if total > 0. { left / total } else { 0. };
        let y = f64::from(self.landing.y)
            + (f64::from(self.top) - f64::from(self.landing.y)) * fraction.clamp(0., 1.);
        replication::CharacterPosition {
            y: y as f32,
            ..self.landing
        }
    }
    /// The body while in the air: at the landing spot, not grounded.
    pub(crate) fn airborne_motion(&self) -> bot_physics::Motion {
        bot_physics::Motion {
            grounded: false,
            vertical_velocity: 0.,
            slide_velocity: [0.; 2],
            ..self.motion
        }
    }
}

pub(crate) fn parachute_state() -> replication::CharacterState {
    replication::CharacterState {
        state: PARACHUTE_STATE,
        stance: 0,
        user_data: [0; 3],
    }
}

impl<W: Write> Engine<W> {
    /// Advance bot `id`'s descent, if it is descending: its height every
    /// `PUBLISH_INTERVAL`, then its touchdown.
    pub(crate) fn tick_bot_descent(
        &mut self,
        id: u16,
        now: Duration,
        packets: &mut Vec<(u16, Vec<u8>)>,
        receipts: &mut Vec<Value>,
    ) -> Result<(), &'static str> {
        let profile = self.wire_profile;
        let bot = self
            .bot_registry
            .bots
            .get_mut(&id)
            .ok_or("bot_index_missing")?;
        let Some(mut descent) = bot.descent else {
            return Ok(());
        };
        let runtime = bot.target.runtime.as_mut().ok_or("bot_runtime_missing")?;
        if bot.health == 0 {
            // Killed in the air: the death is the body's own publication.
            bot.descent = None;
            return Ok(());
        }
        let landed = descent.landed(now);
        if !landed && now < descent.published + PUBLISH_INTERVAL {
            return Ok(());
        }
        let fields = player_tracking::ClientCharacterFields {
            state: landed.then_some(replication::CharacterState {
                state: 1,
                stance: 0,
                user_data: [0; 3],
            }),
            position: Some(descent.position(now)),
            ..Default::default()
        };
        let packet = replication::multiplayer::CharacterOwnerDelta {
            sequence: 2,
            timestamp_ms: now.as_millis() as u32,
            player: id,
            fields: &fields,
        }
        .encode_with_profile(profile)?;
        if landed {
            runtime.physics_motion = Some(descent.motion);
            runtime.position = descent.landing;
            runtime.last_update = now;
            bot.descent = None;
            receipts.push(json!({"op":"bot_parachute_landed","object_index":id,
                "generation":runtime.generation,"position":descent.landing,
                "descent_ms":now.saturating_sub(descent.started).as_millis() as u64}));
        } else {
            descent.published = now;
            bot.descent = Some(descent);
        }
        packets.push((id, packet));
        Ok(())
    }
}

#[cfg(test)]
#[path = "bot_parachute_tests.rs"]
mod tests;
