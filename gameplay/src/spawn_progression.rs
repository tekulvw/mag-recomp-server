//! Objective completion changes human spawn policy without moving live actors.
use super::*;

#[cfg(test)]
#[path = "spawn_progression_tests.rs"]
mod tests;

impl<W: Write> Engine<W> {
    pub(crate) fn advance_forward_spawns(&mut self, now: Duration) -> Result<(), &'static str> {
        let (Some(catalog), Some(world)) = (&self.spawn_catalog, &self.objective_world) else {
            return Ok(());
        };
        let rules: Vec<_> = catalog
            .map_pool()
            .forward_spawns
            .iter()
            .copied()
            .filter(|rule| {
                world.state(rule.objective_index) == Some(replication::ObjectiveState::Completed)
                    && !catalog.forward_applied(*rule)
            })
            .collect();
        if rules.is_empty() {
            return Ok(());
        }
        let catalog = self.spawn_catalog.as_mut().unwrap();
        for rule in &rules {
            catalog.activate_forward(*rule)?;
        }
        let availability = replication::SpawnAvailability::encode_update_with_profile(
            1,
            now.as_millis() as u32,
            catalog,
            self.wire_profile,
        )?;
        let assignments: Vec<_> = rules
            .iter()
            .map(|rule| {
                replication::SquadSpawnAssignment {
                    sequence: 2,
                    timestamp_ms: now.as_millis() as u32,
                    spawn_point_index: rule.spawn_point_index,
                }
                .encode_with_character_for_player_and_catalog_with_profile(
                    false,
                    0,
                    &catalog.for_squad(rule.squad_index)?,
                    self.wire_profile,
                )
            })
            .collect::<Result<_, _>>()?;
        for actor in self.peers.values_mut().chain(self.active.iter_mut()) {
            if actor.initial_objects.is_some() {
                actor.queue_shared(availability.clone());
                for packet in &assignments {
                    actor.queue_shared(packet.clone());
                }
            }
            for rule in &rules {
                // Every per-player catalog sees all progression, including a later
                // squad transfer. Only the affected squad's shared default changes.
                let follows = actor
                    .selected_spawn
                    .is_none_or(|selection| selection.spawn_point_index == actor.squad_spawn);
                std::sync::Arc::make_mut(&mut actor.squad_catalog).activate_forward(*rule)?;
                if actor.squad_catalog.squad_index() != rule.squad_index {
                    continue;
                }
                actor.squad_spawn = rule.spawn_point_index;
                if follows {
                    actor.selected_spawn = Some(protocol::CharacterSpawnSelection {
                        spawn_point_index: rule.spawn_point_index,
                    });
                    let packet = replication::CharacterSpawnAssignment {
                        sequence: 2,
                        timestamp_ms: now.as_millis() as u32,
                        spawn_point_index: rule.spawn_point_index,
                    }
                    .encode_for_player_with_catalog_with_profile(
                        actor.reservation.player,
                        &actor.squad_catalog,
                        self.wire_profile,
                    )?;
                    if actor.initial_objects.is_some() {
                        actor.queue_shared(packet);
                    }
                }
            }
        }
        Ok(())
    }

    pub(crate) fn reset_spawn_progression(&mut self) {
        self.parachute_waves.clear();
        if let Some(catalog) = &mut self.spawn_catalog {
            catalog.reset_runtime();
        }
        for actor in self.peers.values_mut().chain(self.active.iter_mut()) {
            std::sync::Arc::make_mut(&mut actor.squad_catalog).reset_runtime();
            actor.squad_spawn = actor.squad_catalog.default_spawn();
            // Personal selections referencing a retired forward point must not survive.
            if actor.selected_spawn.is_some_and(|selection| {
                actor
                    .squad_catalog
                    .alpha(selection.spawn_point_index)
                    .is_none()
            }) {
                actor.selected_spawn = None;
            }
        }
    }

    pub(crate) fn parachute_ordinal(
        &self,
        point: u16,
        deadline: Duration,
    ) -> Result<usize, &'static str> {
        let ordinal = self
            .parachute_waves
            .get(&(point, deadline))
            .copied()
            .unwrap_or(0);
        if ordinal >= self.match_profile().squad_capacity {
            return Err("parachute_wave_full");
        }
        Ok(ordinal)
    }

    pub(crate) fn prune_parachute_waves(&mut self) {
        let mut newest = std::collections::BTreeMap::new();
        for &(point, deadline) in self.parachute_waves.keys() {
            newest.insert(point, deadline);
        }
        let pending: std::collections::BTreeSet<_> = self
            .peers
            .values()
            .chain(self.active.iter())
            .filter_map(|actor| actor.pending_spawn_deadline)
            .collect();
        // Retain each point's current cycle and any older in-flight wave. Late
        // actors cannot reset an already consumed ordinal by arriving out of order.
        self.parachute_waves.retain(|(point, deadline), _| {
            newest.get(point) == Some(deadline) || pending.contains(deadline)
        });
    }
}
