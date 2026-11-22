//! Zero-XP objective counters committed with their owning gameplay callback.
//! Call only inside the character-effect candidate: outer recording owns replay
//! and installation, rather than a second account reward receipt.
use crate::{
    acquisition_objective_runtime::Assignment, objective_catalog::NativeClass,
    objective_control::NativeEntry, replication::ObjectiveState as State, *,
};
use std::collections::BTreeMap;

#[derive(Default)]
pub(crate) struct Effects {
    pub audit: Vec<Value>,
    pub messages: Vec<OutboundMessage>,
}

fn repair_counter(entry: NativeEntry, subtype: u8) -> Option<u8> {
    if entry.class != NativeClass::Structural
        || entry.previous != State::Repairing
        || entry.delta.state_actor.state != State::Repaired
    {
        return None;
    }
    match subtype {
        3 => Some(43),
        4 => Some(45),
        5 => Some(47),
        7 => Some(49),
        8 => Some(51),
        11 => Some(53),
        13 => Some(55),
        14 => Some(57),
        _ => None,
    }
}
fn tagged_callback(entry: NativeEntry) -> bool {
    matches!(
        (entry.class, entry.previous, entry.delta.state_actor.state),
        (
            NativeClass::Structural,
            State::Activated | State::Deactivating,
            State::Completed
        ) | (
            NativeClass::Structural,
            State::Deactivating,
            State::Deactivated
        ) | (
            NativeClass::Structural,
            State::Deactivating | State::Repairing,
            State::Repaired
        ) | (
            NativeClass::SecureHold,
            State::Activated | State::CancelRetaken,
            State::Completed
        ) | (
            NativeClass::SecureHold,
            State::Deactivating | State::Retaken,
            State::Deactivated
        )
    )
}

impl<W: Write> Engine<W> {
    pub(crate) fn objective_script_character(
        &self,
        player: u16,
        world: u32,
    ) -> Option<domination_objective_runtime::Actor> {
        let team = self.current_objective_source_team(player, world)?;
        let generation = if let Some(actor) = self
            .active
            .iter()
            .chain(self.peers.values())
            .find(|a| a.reservation.player == player && a.reservation.world == world)
        {
            match actor.life {
                CharacterLife::AwaitingFirstSpawn => 0,
                CharacterLife::Alive { generation, .. }
                | CharacterLife::Incapacitated { generation, .. }
                | CharacterLife::AwaitingRespawn { generation } => generation,
            }
        } else {
            self.bot_registry
                .bots
                .get(&player)?
                .target
                .runtime
                .map_or(0, |r| r.generation)
        };
        Some(domination_objective_runtime::Actor {
            player,
            generation: u64::from(generation),
            team,
        })
    }

    pub(crate) fn stage_native_objective_callbacks(
        &mut self,
        entry: NativeEntry,
        assignment: Option<Assignment>,
        assignments: &[(u16, Assignment)],
        frame: frago::Frame<'_>,
    ) -> Result<Effects, &'static str> {
        let counter = self
            .objective_catalog
            .as_ref()
            .and_then(|c| {
                c.objects
                    .iter()
                    .find(|o| o.object_index == entry.delta.object_index)
            })
            .and_then(|o| o.reward.as_ref())
            .and_then(|r| repair_counter(entry, r.subtype));
        let mut effects = if let Some(origin) = entry.bot_plant {
            if origin.round != self.round_number || !self.accepts_subscription_world(origin.world) {
                return Err("stale_bot_plant_objective_source");
            }
            self.stage_objective_counters_for_team(
                origin.world,
                origin.team,
                None,
                entry.delta.object_index,
                &[],
                tagged_callback(entry),
                assignments,
            )?
        } else if let Some(player) = entry.reward_actor {
            self.stage_objective_counters(
                player,
                entry.delta.object_index,
                counter.as_slice(),
                tagged_callback(entry),
                assignments,
            )?
        } else {
            Effects::default()
        };
        effects
            .audit
            .extend(self.stage_native_objective_rewards(entry, assignment, frame)?);
        Ok(effects)
    }

    /// Counters use current allocated characters and current published squad
    /// leaders. XP admission, account IDs and life state are unrelated gates.
    pub(crate) fn stage_objective_counters(
        &mut self,
        player: u16,
        object_index: u16,
        counters: &[u8],
        tagged: bool,
        assignments: &[(u16, Assignment)],
    ) -> Result<Effects, &'static str> {
        if counters
            .iter()
            .any(|s| !matches!(s, 43 | 45 | 47 | 49 | 51 | 53 | 55 | 57 | 58))
        {
            return Err("unsupported_objective_counter");
        }
        let humans = || self.active.iter().chain(self.peers.values());
        let Some(world) = humans().next().map(|a| a.reservation.world) else {
            return Ok(Effects::default());
        };
        let Some(team) = self
            .current_objective_source_team(player, world)
            .filter(|t| *t <= 1)
        else {
            return Ok(Effects::default());
        };
        self.stage_objective_counters_for_team(
            world,
            team,
            Some(player),
            object_index,
            counters,
            tagged,
            assignments,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn stage_objective_counters_for_team(
        &mut self,
        world: u32,
        team: u8,
        player: Option<u16>,
        object_index: u16,
        counters: &[u8],
        tagged: bool,
        assignments: &[(u16, Assignment)],
    ) -> Result<Effects, &'static str> {
        let humans = || self.active.iter().chain(self.peers.values());
        let mut increments: BTreeMap<(String, u8), i32> = BTreeMap::new();
        if let Some(actor) = humans().find(|a| {
            Some(a.reservation.player) == player
                && a.reservation.world == world
                && a.wire_profile.application_id() == 20231
        }) {
            for &stat in counters {
                *increments
                    .entry((actor.reservation.id.clone(), stat))
                    .or_default() += 1;
            }
        }
        if tagged {
            let target = 0x0c00_0000 | u32::from(object_index);
            for &(squad, assignment) in assignments {
                if squad / 16 != u16::from(team) || assignment.tagged_object & 0xff00_ffff != target
                {
                    continue;
                }
                if let Some(leader) = self.objective_counter_leader(squad)
                    && leader.reservation.world == world
                    && leader.wire_profile.application_id() == 20231
                {
                    *increments
                        .entry((leader.reservation.id.clone(), 41))
                        .or_default() += 1;
                }
            }
        }
        let mut updates: BTreeMap<String, BTreeMap<u8, i32>> = BTreeMap::new();
        for ((id, stat), delta) in increments {
            let actor = humans()
                .find(|a| a.reservation.id == id)
                .expect("allocated objective counter recipient");
            updates.entry(id).or_default().insert(
                stat,
                actor
                    .reward_statistics
                    .get(&stat)
                    .copied()
                    .unwrap_or(0)
                    .saturating_add(delta),
            );
        }
        let mut effects = Effects::default();
        for (id, stats) in &updates {
            let actor = humans()
                .find(|a| a.reservation.id == *id)
                .expect("allocated objective counter recipient");
            let wire = protocol::StatUpdate {
                character_index: actor.reservation.player,
                stats: stats
                    .iter()
                    .map(|(&stat, &value)| protocol::StatValue { stat, value })
                    .collect(),
            }
            .encode_with_profile(actor.wire_profile)?;
            for recipient in humans().filter(|a| {
                a.reservation.world == world && a.ready_for_updates && a.initial_objects.is_some()
            }) {
                effects.messages.push(OutboundMessage {
                    reservation_id: recipient.reservation.id.clone(),
                    world_id: world,
                    player_id: recipient.reservation.player,
                    plugin_id: 1,
                    payload_hex: protocol::encode_hex(&wire),
                });
            }
            effects.audit.push(
                json!({"op":"objective_counters","object_index":object_index,
                "source_player":player,"reservation_id":id,"statistics":stats}),
            );
        }
        for actor in self.active.iter_mut().chain(self.peers.values_mut()) {
            if let Some(stats) = updates.remove(&actor.reservation.id) {
                actor.reward_statistics.extend(stats);
            }
        }
        Ok(effects)
    }
}
