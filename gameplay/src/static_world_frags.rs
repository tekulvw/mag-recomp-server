//! Ordinary frag Character blast in Sabotage, against the static map world.
//!
//! Sabotage loads no represented projectile scene: no dynamic body catalog,
//! collision filter rows or material bindings. A frag that the owner reports
//! detonated there still owes its authored Character dose. Every living,
//! on-foot Character whose stance anchor lies inside the authored outer radius
//! takes that dose with the authored falloff, unless a static triangle crosses
//! the straight line from the blast to the anchor. Any crossing counts as
//! cover: the world carries no materials to penetrate. The thrower is a
//! candidate like anyone else. Armour, friendly fire, incapacitation,
//! statistics and rewards follow the ordinary queued frag dose.
use crate::{
    Engine, acquisition_character_effects as character, armor,
    collision::CollisionWorld,
    development_target::BotPhase,
    domination_structural_world::{
        CHARACTER_BLAST_ANCHOR_HEIGHT_M, CharacterBlastObservation, OnFootBlastPose,
    },
    item_catalog::ItemCatalog,
};
use std::io::Write;

/// Static triangle and node work allowed for one blast's cover rays.
const QUERY_BUDGET: usize = 1_000_000;

/// The frag items a static-world blast can dose, when the loaded inputs
/// qualify it: retail grenade damage authority and friendly-fire rules.
pub fn qualified_items(
    items: &ItemCatalog,
    character_damage_rules: bool,
) -> Result<Vec<u16>, &'static str> {
    if !character_damage_rules {
        return Err("static_world_frag_blast_requires_character_damage_rules");
    }
    items
        .retail_combat()
        .map_err(|_| "static_world_frag_blast_requires_retail_combat")?
        .frag_damage_items()
        .map_err(|_| "static_world_frag_blast_requires_grenade_damage")
}

impl<W: Write> Engine<W> {
    /// The static collision world a frag blast uses when the match has no
    /// represented projectile scene: retail Sabotage with a loaded world for
    /// its map, after [`Engine::with_static_world_frag_blast`].
    pub(crate) fn static_frag_world(&self) -> Option<&CollisionWorld> {
        if !self.static_world_frag_blast
            || self.acquisition_gameplay.is_some()
            || self.domination_gameplay.is_some()
            || self.wire_profile.application_id() != 20231
            || self.match_profile().game_type != 0
        {
            return None;
        }
        self.collision_world.as_ref().filter(|world| {
            world.stats().triangles != 0 && world.metadata().map_name == self.map_name()
        })
    }

    /// An ordinary frag doses Characters through either qualified scene.
    pub(crate) fn frag_character_blast_enabled(&self) -> bool {
        self.frag_character_blast || self.static_frag_world().is_some()
    }

    /// Dose Characters from reported Sabotage frag detonations against the
    /// loaded static collision world. Requires that world, friendly-fire rules
    /// and retail grenade damage metadata.
    pub fn with_static_world_frag_blast(mut self) -> Result<Self, &'static str> {
        qualified_items(&self.item_catalog, self.character_damage_rules.is_some())?;
        self.static_world_frag_blast = true;
        if self.static_frag_world().is_none() {
            return Err("static_world_frag_blast_requires_sabotage_collision");
        }
        Ok(self)
    }

    /// Characters in `world` reached by a blast at `origin`. The event's
    /// Character uses its accepted same-event pose; others use their latest
    /// tracked position and owner stance.
    pub(crate) fn collect_static_frag_blast(
        &self,
        scene: &CollisionWorld,
        world: u32,
        origin: [f64; 3],
        half_radius_m: f32,
        outer_radius_m: f32,
        accepted: OnFootBlastPose,
    ) -> Result<Vec<CharacterBlastObservation>, &'static str> {
        if !origin.into_iter().all(f64::is_finite)
            || !half_radius_m.is_finite()
            || !outer_radius_m.is_finite()
            || half_radius_m <= 0.
            || outer_radius_m < half_radius_m
        {
            return Err("invalid_character_blast_shape");
        }
        let anchor_height = |stance: u8| {
            CHARACTER_BLAST_ANCHOR_HEIGHT_M
                .get(usize::from(stance))
                .copied()
        };
        let mut anchors = Vec::new();
        let actors = self
            .active
            .iter()
            .map(|actor| (actor, true))
            .chain(self.peers.values().map(|actor| (actor, false)));
        for (actor, event_owner) in actors {
            if actor.reservation.world != world
                || actor.is_mounted()
                || !actor.entered_game
                || !actor.ready_for_updates
                || actor.initial_objects.is_none()
            {
                continue;
            }
            let Some((generation, _)) = actor.life.damage_health() else {
                continue;
            };
            let pose = if event_owner {
                accepted
            } else {
                OnFootBlastPose::default()
            };
            let Some(feet) = pose
                .position
                .or(actor.tracking.sample.map(|sample| sample.position))
            else {
                continue;
            };
            // Until the first owner state, a spawned Character stands.
            let stance = pose
                .stance
                .or(actor.owner_fields.state.map(|state| state.stance))
                .unwrap_or(0);
            let Some(height) = anchor_height(stance) else {
                continue;
            };
            anchors.push((
                character::Target {
                    character_id: 0x0380_0000 | u32::from(actor.reservation.player),
                    generation,
                    world,
                },
                [feet.x, feet.y + height, feet.z],
            ));
        }
        if self.accepts_subscription_world(world) {
            for (&id, bot) in &self.bot_registry.bots {
                let Some(runtime) = bot
                    .target
                    .runtime
                    .filter(|runtime| bot.health > 0 && !matches!(runtime.phase, BotPhase::Dead))
                else {
                    continue;
                };
                let [x, y, z] = runtime.physical_position().map(|axis| axis as f32);
                anchors.push((
                    character::Target {
                        character_id: 0x0380_0000 | u32::from(id),
                        generation: runtime.generation,
                        world,
                    },
                    [x, y + CHARACTER_BLAST_ANCHOR_HEIGHT_M[0], z],
                ));
            }
        }
        let origin_f32 = origin.map(|axis| axis as f32);
        let mut budget = QUERY_BUDGET;
        let mut observations = Vec::new();
        for (target, anchor) in anchors {
            let displacement: [f32; 3] =
                std::array::from_fn(|axis| anchor[axis] - origin_f32[axis]);
            let distance = displacement
                .iter()
                .map(|axis| axis * axis)
                .sum::<f32>()
                .sqrt();
            let scale = armor::native_radial_scale(distance, half_radius_m, outer_radius_m)?;
            if scale <= 0. {
                continue;
            }
            let end = anchor.map(f64::from);
            if end != origin
                && scene
                    .raycast_segment_bounded(origin, end, &mut budget)
                    .map_err(|_| {
                        if budget == 0 {
                            "static_world_frag_blast_work_limit"
                        } else {
                            "static_world_frag_blast_query_invalid"
                        }
                    })?
                    .nearest
                    .is_some()
            {
                continue;
            }
            observations.push(CharacterBlastObservation {
                target,
                distance,
                scale,
                direction: if distance == 0. {
                    [0.; 3]
                } else {
                    displacement.map(|axis| axis / distance)
                },
            });
        }
        Ok(observations)
    }
}

#[cfg(test)]
#[path = "static_world_frags_tests.rs"]
mod tests;
