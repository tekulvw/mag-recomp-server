//! Source-bound contextual reward eligibility; XP arithmetic belongs to Horizon.
use super::*;
use std::collections::BTreeSet;

pub(crate) const CAPABILITY: &str = "frago_kill_bonus_v1";
pub(crate) const ACTION_CAPABILITY: &str = "frago_action_bonus_v1";

/// Accepted event poses precede queued damage, but install after recording.
#[derive(Clone, Copy, Default)]
pub(crate) struct Frame<'a> {
    pub active_reservation: Option<&'a str>,
    pub position: Option<[f32; 3]>,
    pub ground: Option<&'a acquisition_ground_runtime::Runtime>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    authored_catalog_sha256: String,
    progression_catalog_sha256: String,
    source_image_sha256: String,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Objective {
    object_index: u16,
    position: [f32; 3],
    radius: f32,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Rules {
    schema_version: u8,
    map_name: String,
    game_type: i8,
    source: Source,
    vehicle_range: f32,
    objectives: Vec<Objective>,
}
impl Rules {
    pub(crate) fn validate(
        &self,
        source: &objective_catalog::ObjectiveSource,
        indices: &BTreeSet<u16>,
        map_name: &str,
        game_type: i8,
    ) -> Result<(), &'static str> {
        let sha = |v: &str| v.len() == 64 && v.bytes().all(|b| b.is_ascii_hexdigit());
        let mut found = BTreeSet::new();
        if self.map_name != map_name
            || self.game_type != game_type
            || self.schema_version != 1
            || self.source.authored_catalog_sha256 != source.authored_catalog_sha256
            || self.source.source_image_sha256 != source.source_image_sha256
            || !sha(&self.source.progression_catalog_sha256)
            || !self.vehicle_range.is_finite()
            || self.vehicle_range < 0.
            || self.objectives.iter().any(|o| {
                !found.insert(o.object_index)
                    || o.position.iter().any(|x| !x.is_finite())
                    || !o.radius.is_finite()
                    || o.radius < 0.
            })
            || &found != indices
        {
            return Err("invalid_frago_geometry");
        }
        Ok(())
    }
}

pub(crate) fn point(p: replication::CharacterPosition) -> [f32; 3] {
    [p.x, p.y, p.z]
}
fn in_range(a: [f32; 3], b: [f32; 3], radius: f32) -> bool {
    if a.iter().chain(&b).any(|x| !x.is_finite()) {
        return false;
    }
    let [x, y, z] = std::array::from_fn(|i| a[i] - b[i]);
    ((x * x + y * y) + z * z) <= radius * radius
}
impl<W: Write> Engine<W> {
    pub(crate) fn frago_character_position(
        &self,
        actor: &Active,
        frame: Frame<'_>,
    ) -> Option<[f32; 3]> {
        // Ground is already bound to the current authoritative vehicle root.
        // Other mounted character root conventions remain a separate scope.
        if let Some(seat) = actor.ground_seat {
            return frame
                .ground
                .or(self.ground_runtime.as_ref())?
                .get(seat.vehicle_index)
                .map(|v| point(v.pose.position));
        }
        if actor.is_mounted() {
            return None;
        }
        frame
            .position
            .filter(|_| frame.active_reservation == Some(actor.reservation.id.as_str()))
            .or_else(|| actor.tracking.sample.map(|s| point(s.position)))
    }
    fn frago_target(
        &self,
        rules: &Rules,
        target: u32,
        frame: Frame<'_>,
    ) -> Option<([f32; 3], f32)> {
        let index = target as u16;
        match target & 0xffff_0000 {
            0x0c80_0000 => {
                self.objective_world.as_ref()?.state(index)?;
                rules
                    .objectives
                    .iter()
                    .find(|o| o.object_index == index)
                    .map(|o| (o.position, o.radius))
            }
            0x0100_0000 => frame
                .ground
                .or(self.ground_runtime.as_ref())?
                .get(index)
                .map(|v| (point(v.pose.position), rules.vehicle_range)),
            0x0280_0000 => self
                .domination_gameplay
                .as_ref()?
                .state
                .helo
                .as_ref()?
                .carriers
                .iter()
                .find(|c| c.source.object_index == index)?
                .current_pose
                .map(|p| (point(p.position), rules.vehicle_range)),
            0x0d80_0000 => self
                .bunker_world
                .as_ref()?
                .domination_bunker(index)
                .map(|b| (point(b.position), rules.vehicle_range)),
            _ => None,
        }
    }
    pub(crate) fn frago_kill_reward(
        &self,
        event: progression::RewardEvent,
        killer: &Active,
        context_position: Option<[f32; 3]>,
        frame: Frame<'_>,
    ) -> progression::RewardEvent {
        if event.kind != progression::RewardKind::EnemyKilled
            || event.units != 1
            || !killer
                .reservation
                .character
                .frago_bonus_enabled
                .unwrap_or(false)
            || killer.wire_profile.application_id() != 20231
        {
            return event;
        }
        self.frago_contextual_reward(event, killer, context_position, frame)
    }
    pub(crate) fn frago_action_reward(
        &self,
        event: progression::RewardEvent,
        actor: &Active,
        context_position: Option<[f32; 3]>,
        frame: Frame<'_>,
    ) -> progression::RewardEvent {
        if !event.kind.is_frago_action(event.units)
            || !actor
                .reservation
                .character
                .frago_action_bonus_enabled
                .unwrap_or(false)
            || actor.wire_profile.application_id() != 20231
        {
            return event;
        }
        self.frago_contextual_reward(event, actor, context_position, frame)
    }
    fn frago_contextual_reward(
        &self,
        mut event: progression::RewardEvent,
        actor: &Active,
        context_position: Option<[f32; 3]>,
        frame: Frame<'_>,
    ) -> progression::RewardEvent {
        let Some(rules) = self
            .objective_catalog
            .as_ref()
            .and_then(|c| c.frago.as_ref())
        else {
            return event;
        };
        let squad = actor.squad_catalog.squad_index();
        let assignment = self
            .acquisition_gameplay
            .as_ref()
            .and_then(|r| r.state.squad(squad).map(|s| s.assignment))
            .or_else(|| {
                self.domination_gameplay
                    .as_ref()
                    .and_then(|r| r.squad_assignment(squad))
            });
        let Some(assignment) = assignment else {
            return event;
        };
        let primary = if assignment.primary_index == u16::MAX {
            u32::MAX
        } else {
            0x0c80_0000 | u32::from(assignment.primary_index)
        };
        let actor_position = self.frago_character_position(actor, frame);
        for target in [assignment.tagged_object, primary] {
            if target == u32::MAX {
                continue;
            }
            if let Some((position, radius)) = self.frago_target(rules, target, frame)
                && actor_position
                    .into_iter()
                    .chain(context_position)
                    .any(|p| in_range(p, position, radius))
            {
                event.frago = Some(true);
                break;
            }
        }
        event
    }
}

pub(crate) fn validate_snapshot(points: Option<u32>, match_xp: u32) -> Result<(), &'static str> {
    if points.is_some_and(|p| p > i32::MAX as u32 || p > match_xp) {
        Err("invalid_match_frago_points")
    } else {
        Ok(())
    }
}
