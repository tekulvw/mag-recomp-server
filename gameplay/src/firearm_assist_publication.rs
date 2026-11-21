//! Coalesced human assist rows share the recorded, bounded replication drain.
use crate::*;
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Pending {
    allocation: u64,
    through: u64,
}

#[derive(Clone, Copy)]
pub(super) enum Statistic {
    Bot(bot_stat_publication::Candidate),
    Human {
        slot: u16,
        allocation: u64,
        value: i32,
    },
}

impl Statistic {
    pub(super) fn slot(self) -> u16 {
        match self {
            Self::Bot(row) => row.slot,
            Self::Human { slot, .. } => slot,
        }
    }

    pub(super) fn encode<W: Write>(
        self,
        engine: &Engine<W>,
        profile: wire_profile::WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        match self {
            Self::Bot(row) => {
                row.totals
                    .encode(row.slot, profile, engine.firearm_assists.enabled())
            }
            Self::Human { slot, value, .. } => protocol::StatUpdate {
                character_index: slot,
                stats: vec![protocol::StatValue {
                    stat: firearm_assists::STAT,
                    value,
                }],
            }
            .encode_with_profile(profile),
        }
    }
}

impl<W: Write> Engine<W> {
    /// Only metadata is queued. Selection reads the latest authoritative value,
    /// so a later full statistics row cannot be overtaken by an older total.
    pub(super) fn queue_human_firearm_assist(&mut self, slot: u16, allocation: u64, world: u32) {
        for recipient in self
            .active
            .iter_mut()
            .chain(self.peers.values_mut())
            .filter(|actor| {
                actor.reservation.world == world
                    && actor.ready_for_updates
                    && actor.initial_objects.is_some()
            })
        {
            let through = recipient
                .shared_frames_drained
                .saturating_add(recipient.pending_shared.len() as u64);
            recipient
                .pending_firearm_assist_rows
                .entry(slot)
                .and_modify(|pending| {
                    if pending.allocation == allocation {
                        pending.through = pending.through.min(through);
                    } else {
                        *pending = Pending {
                            allocation,
                            through,
                        };
                    }
                })
                .or_insert(Pending {
                    allocation,
                    through,
                });
        }
    }

    fn current_human_assist(
        &self,
        recipient: &Active,
        slot: u16,
        pending: Pending,
    ) -> Option<Statistic> {
        if !self.firearm_assists.enabled() || recipient.wire_profile.application_id() != 20231 {
            return None;
        }
        let actor = self
            .active
            .iter()
            .chain(self.peers.values())
            .find(|actor| {
                actor.reservation.world == recipient.reservation.world
                    && actor.reservation.player == slot
                    && actor.allocation_sequence == pending.allocation
                    && actor.initial_objects.is_some()
            })?;
        Some(Statistic::Human {
            slot,
            allocation: pending.allocation,
            value: actor
                .reward_statistics
                .get(&firearm_assists::STAT)
                .copied()
                .unwrap_or(0),
        })
    }

    pub(super) fn human_assists_pending(&self, recipient: &Active) -> bool {
        recipient
            .pending_firearm_assist_rows
            .iter()
            .any(|(&slot, &pending)| {
                self.current_human_assist(recipient, slot, pending)
                    .is_some()
            })
    }

    pub(super) fn next_shared_statistic(
        &self,
        recipient: &Active,
        drained: u64,
        cursor: u16,
        selected: &BTreeSet<u16>,
    ) -> Option<Statistic> {
        let bot = self
            .bot_registry
            .stats_publication
            .next(
                recipient,
                &self.bot_registry.bots,
                drained,
                cursor,
                selected,
            )
            .map(Statistic::Bot);
        let human = recipient
            .pending_firearm_assist_rows
            .range(cursor..)
            .chain(recipient.pending_firearm_assist_rows.range(..cursor))
            .find_map(|(&slot, &pending)| {
                if pending.through > drained || selected.contains(&slot) {
                    return None;
                }
                self.current_human_assist(recipient, slot, pending)
            });
        bot.into_iter()
            .chain(human)
            .min_by_key(|row| row.slot().wrapping_sub(cursor))
    }

    pub(super) fn commit_shared_statistic(&mut self, recipient: &str, row: Statistic) {
        if let Statistic::Bot(candidate) = row {
            self.bot_registry
                .stats_publication
                .sent(recipient, candidate);
        }
        let actor = self
            .active
            .as_mut()
            .filter(|a| a.reservation.id == recipient)
            .or_else(|| self.peers.get_mut(recipient))
            .unwrap();
        if let Statistic::Human {
            slot, allocation, ..
        } = row
            && actor
                .pending_firearm_assist_rows
                .get(&slot)
                .is_some_and(|p| p.allocation == allocation)
        {
            actor.pending_firearm_assist_rows.remove(&slot);
        }
        actor.statistics_cursor = row.slot().wrapping_add(1);
        actor.prefer_statistics = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Engine<Vec<u8>> {
        let mut engine = crate::human_kill_reward_tests::fixture()
            .with_firearm_assist_rules(firearm_assists::Rules {
                schema_version: 1,
                application_id: 20231,
                assist_timeout_ms: 37,
            })
            .unwrap();
        engine.maximum_players = 8;
        engine.drain_shared(Vec::new(), 1).unwrap();
        assert!(!engine.bot_registry.enabled());
        engine
    }
    fn source(engine: &mut Engine<Vec<u8>>) -> &mut Active {
        engine
            .peers
            .values_mut()
            .find(|a| a.reservation.player == 0)
            .unwrap()
    }
    fn queue(engine: &mut Engine<Vec<u8>>, value: i32) {
        let actor = source(engine);
        actor.reward_statistics.insert(11, value);
        let allocation = actor.allocation_sequence;
        engine.queue_human_firearm_assist(0, allocation, 1);
    }
    fn rows(messages: &[OutboundMessage], player: u16) -> Vec<&str> {
        messages
            .iter()
            .filter(|m| m.player_id == player && m.payload_hex.starts_with("0204"))
            .map(|m| m.payload_hex.as_str())
            .collect()
    }
    fn fill(engine: &mut Engine<Vec<u8>>, count: usize) {
        let wire = replication::ObjectUpdate {
            header: replication::UpdateHeader {
                sequence: 1,
                nak_sequence: 0,
                client_index: 0,
                object_type_mask: 0,
                timestamp_ms: 0,
            },
            wire: &[],
        }
        .encode_empty_with_profile(engine.wire_profile)
        .unwrap();
        for _ in 0..count {
            engine.active.as_mut().unwrap().queue_shared(wire.clone());
        }
    }

    #[test]
    fn human_assist_rows_coalesce_without_bots_or_replication_sequences() {
        let mut engine = fixture();
        queue(&mut engine, 1);
        queue(&mut engine, 2);
        let before = engine.active.as_ref().unwrap().outbound_sequence;
        let reply = engine.drain_shared(Vec::new(), 1).unwrap();
        assert_eq!(rows(&reply, 1), ["02040000010b02"]);
        assert_eq!(rows(&reply, 0), ["02040000010b02"]);
        assert_eq!(engine.active.as_ref().unwrap().outbound_sequence, before);
        assert!(
            engine
                .active
                .as_ref()
                .unwrap()
                .pending_firearm_assist_rows
                .is_empty()
        );
        assert!(engine.drain_shared(Vec::new(), 1).unwrap().is_empty());
    }

    #[test]
    fn human_assist_fence_preserves_creation_prefix_but_not_unrelated_later_backlog() {
        let mut engine = fixture();
        fill(&mut engine, 40);
        queue(&mut engine, 1);
        fill(&mut engine, 60);
        queue(&mut engine, 2);
        let first = engine.drain_shared(Vec::new(), 1).unwrap();
        assert!(rows(&first, 1).is_empty());
        let second = engine.drain_shared(Vec::new(), 1).unwrap();
        assert_eq!(rows(&second, 1), ["02040000010b02"]);
        assert!(!engine.active.as_ref().unwrap().pending_shared.is_empty());
        assert!(first.len() <= 32 && second.len() <= 32);
    }

    #[test]
    fn human_assist_record_refusal_preserves_pending_allocation_and_fence() {
        let mut engine = fixture();
        queue(&mut engine, 2);
        let before = engine
            .active
            .as_ref()
            .unwrap()
            .pending_firearm_assist_rows
            .clone();
        engine.max_log = engine.written;
        assert_eq!(
            engine.drain_shared(Vec::new(), 1).unwrap_err(),
            "log_limit_reached"
        );
        assert_eq!(
            engine.active.as_ref().unwrap().pending_firearm_assist_rows,
            before
        );
        assert_eq!(source(&mut engine).reward_statistics.get(&11), Some(&2));
    }

    #[test]
    fn human_assist_pending_row_cannot_alias_replacement_admission() {
        let mut engine = fixture();
        queue(&mut engine, 1);
        source(&mut engine).allocation_sequence += 1;
        source(&mut engine).reward_statistics.insert(11, 7);
        assert!(engine.drain_shared(Vec::new(), 1).unwrap().is_empty());
        queue(&mut engine, 7);
        let reply = engine.drain_shared(Vec::new(), 1).unwrap();
        assert_eq!(rows(&reply, 1), ["02040000010b07"]);
    }
}
