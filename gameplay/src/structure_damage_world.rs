//! Qualified objective-trigger colliders, separate from network objective bodies.
use crate::{
    domination_structural_effects::InstantaneousBlast,
    dynamic_collision::{Geometry, Pose, QueryLimits, geometry},
    objective_range::{AUTHORED_TRIGGER_BOX, ObjectiveRange, TriggerBox},
    structure_damage::{StructureDamage, StructureDamageOutcome},
};
use serde::Deserialize;
use std::collections::BTreeMap;

pub(crate) type Key = (u16, u8);

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub object_index: u16,
    pub trigger_index: u8,
    pub trigger_type: u8,
    pub start_active: bool,
    pub damage_threshold: f32,
    pub position: [f32; 3],
    #[serde(rename = "box")]
    pub volume: TriggerBox,
}
impl Rule {
    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        StructureDamage::new(self.trigger_type, self.damage_threshold)?;
        if self.object_index >= 128
            || self.trigger_index == u8::MAX
            || self.position != self.volume.centre
            || !ObjectiveRange::validate(AUTHORED_TRIGGER_BOX, 1., 1, &[self.volume])
        {
            return Err("invalid_structure_damage_collider");
        }
        Ok(())
    }
    fn key(&self) -> Key {
        (self.object_index, self.trigger_index)
    }
}
#[derive(Clone)]
struct Trigger {
    rule: Rule,
    receiver: StructureDamage,
    phantom_present: bool,
}
#[derive(Clone, Default)]
pub(crate) struct World {
    triggers: BTreeMap<Key, Trigger>,
}
impl World {
    pub(crate) fn new(rules: &[Rule]) -> Result<Self, &'static str> {
        if rules.len() > 512 {
            return Err("structure_damage_trigger_limit");
        }
        let mut triggers = BTreeMap::new();
        for rule in rules {
            rule.validate()?;
            let trigger = Trigger {
                rule: rule.clone(),
                receiver: StructureDamage::new(rule.trigger_type, rule.damage_threshold)?,
                phantom_present: true,
            };
            if triggers.insert(rule.key(), trigger).is_some() {
                return Err("duplicate_structure_damage_trigger");
            }
        }
        Ok(Self { triggers })
    }
    /// Family56 receivers use a positive radial gate and raw channel6 damage.
    /// Their simple-shape phantoms do not use the actor material-cover query.
    #[cfg(test)]
    pub(crate) fn collect(
        &self,
        blast: &InstantaneousBlast,
        filter_row: u32,
        limits: QueryLimits,
    ) -> Result<Vec<Key>, &'static str> {
        self.collect_with_scale(blast, filter_row, limits, |distance| blast.scale(distance))
    }

    pub(crate) fn collect_with_scale(
        &self,
        blast: &InstantaneousBlast,
        filter_row: u32,
        limits: QueryLimits,
        scale: impl Fn(f32) -> Result<f32, &'static str>,
    ) -> Result<Vec<Key>, &'static str> {
        if filter_row & (1 << 6) == 0 || blast.damage == 0. {
            return Ok(Vec::new());
        }
        let sphere = Geometry::Sphere {
            radius: f64::from(blast.outer_radius),
        };
        let mut budget = limits.max_support_points;
        let mut hits = Vec::new();
        for (key, target) in &self.triggers {
            if !target.phantom_present {
                continue;
            }
            let v = target.rule.volume;
            let shape = Geometry::Box {
                half_extents: v.half_extents.map(f64::from),
                radius: 0.05,
            };
            let pose = Pose {
                translation: v.centre.map(f64::from),
                rotation: std::array::from_fn(|i| std::array::from_fn(|j| f64::from(v.axes[j][i]))),
            };
            for origin in &blast.origins {
                let delta = std::array::from_fn::<_, 3, _>(|i| origin[i] - target.rule.position[i]);
                let distance =
                    ((delta[0] * delta[0] + delta[1] * delta[1]) + delta[2] * delta[2]).sqrt();
                if scale(distance)? > 0.
                    && geometry::penetrates(
                        &sphere,
                        Pose::at(origin.map(f64::from)),
                        &shape,
                        pose,
                        limits,
                        &mut budget,
                    )?
                {
                    hits.push(*key);
                }
            }
        }
        Ok(hits)
    }
    pub(crate) fn has_targets(&self) -> bool {
        !self.triggers.is_empty()
    }
    pub(crate) fn apply(
        &mut self,
        key: Key,
        damage: f32,
        owner_is_active: bool,
    ) -> Result<StructureDamageOutcome, &'static str> {
        let target = self
            .triggers
            .get_mut(&key)
            .ok_or("unknown_structure_damage_trigger")?;
        Ok(target
            .receiver
            .apply(4, damage, target.rule.start_active, owner_is_active))
    }
    pub(crate) fn remove_phantom(&mut self, key: Key) -> Result<(), &'static str> {
        self.triggers
            .get_mut(&key)
            .ok_or("unknown_structure_damage_trigger")?
            .phantom_present = false;
        Ok(())
    }
}

#[cfg(test)]
#[path = "structure_damage_world_tests.rs"]
mod tests;

impl<W: std::io::Write> crate::Engine<W> {
    /// Threshold damage is independent of the owner callback's acceptance.
    /// A missing source can spend trigger health without creating progression.
    pub(crate) fn stage_structure_strike(
        &self,
        triggers: &mut World,
        objectives: Option<&mut crate::objective_control::ObjectiveWorld>,
        hits: &[Key],
        raw_damage: f32,
        attacker: u32,
        now: std::time::Duration,
    ) -> Result<Vec<serde_json::Value>, &'static str> {
        use crate::{CharacterLife, replication::ObjectiveState};
        if hits.is_empty() {
            return Ok(Vec::new());
        }
        let objectives = objectives.ok_or("structure_damage_objective_world_required")?;
        let mut audit = Vec::new();
        for &key in hits {
            let state = objectives
                .state(key.0)
                .ok_or("structure_damage_owner_missing")?;
            let outcome = triggers.apply(key, raw_damage, state != ObjectiveState::Inactive)?;
            let mut progressed = false;
            if outcome == StructureDamageOutcome::Destroyed
                && matches!(
                    state,
                    ObjectiveState::Deactivated | ObjectiveState::Repaired
                )
                && let Some(actor) = self
                    .active
                    .iter()
                    .chain(self.peers.values())
                    .find(|a| 0x0380_0000 | u32::from(a.reservation.player) == attacker)
            {
                let generation = match actor.life {
                    CharacterLife::AwaitingFirstSpawn => 0,
                    CharacterLife::Alive { generation, .. }
                    | CharacterLife::Incapacitated { generation, .. }
                    | CharacterLife::AwaitingRespawn { generation } => generation,
                };
                let modifier = self
                    .plan_leadership(now)?
                    .modifiers(actor)
                    .value("kCharModDemolitionTime");
                let source = crate::structure_damage::Source {
                    player: actor.reservation.player,
                    generation,
                };
                let source_allocated = self
                    .current_objective_source_team(source.player, actor.reservation.world)
                    .is_some();
                progressed = objectives
                    .activate_destroyed_trigger_with_source(
                        key.0,
                        key.1,
                        source,
                        source_allocated,
                        now,
                        modifier,
                    )?
                    .is_some();
                if progressed {
                    triggers.remove_phantom(key)?;
                }
            }
            audit.push(
                serde_json::json!({"op":"structure_strike_damage","objective_index":key.0,
                "trigger_index":key.1,"raw_damage":raw_damage,"attacker":attacker,
                "outcome":format!("{outcome:?}"),"owner_progressed":progressed}),
            );
        }
        Ok(audit)
    }
}
