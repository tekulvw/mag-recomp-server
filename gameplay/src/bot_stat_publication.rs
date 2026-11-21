//! Recipient-local creation fences for unsequenced absolute bot statistics.
use crate::{Active, bot_firearm_kd::Totals, bot_registry::WorldBot};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
pub(super) struct State {
    recipients: BTreeMap<String, Recipient>,
}

#[derive(Default)]
struct Recipient {
    entries: BTreeMap<u16, Entry>,
}

struct Entry {
    allocation: u32,
    through: u64,
    sent: Option<Totals>,
}

#[derive(Clone, Copy)]
pub(super) struct Candidate {
    pub(super) slot: u16,
    allocation: u32,
    pub(super) totals: Totals,
}

impl State {
    pub(super) fn register(&mut self, recipient: &str, slot: u16, allocation: u32, through: u64) {
        self.recipients
            .entry(recipient.to_owned())
            .or_default()
            .entries
            .insert(
                slot,
                Entry {
                    allocation,
                    through,
                    sent: None,
                },
            );
    }

    pub(super) fn retire(&mut self, slot: u16) {
        for recipient in self.recipients.values_mut() {
            recipient.entries.remove(&slot);
        }
    }

    pub(super) fn retain_recipients(&mut self, recipients: &[String]) {
        self.recipients.retain(|id, _| recipients.contains(id));
    }

    pub(super) fn pending(&self, actor: &Active, bots: &BTreeMap<u16, WorldBot>) -> bool {
        self.recipients
            .get(&actor.reservation.id)
            .is_some_and(|recipient| {
                actor.initial_objects.is_some()
                    && actor.wire_profile.application_id() == 20231
                    && recipient.entries.iter().any(|(slot, entry)| {
                        bots.get(slot).is_some_and(|bot| {
                            bot.allocation_generation == entry.allocation
                                && entry.sent != Some(bot.firearm_kd)
                        })
                    })
            })
    }

    pub(super) fn next(
        &self,
        actor: &Active,
        bots: &BTreeMap<u16, WorldBot>,
        drained: u64,
        cursor: u16,
        selected: &BTreeSet<u16>,
    ) -> Option<Candidate> {
        if actor.initial_objects.is_none() || actor.wire_profile.application_id() != 20231 {
            return None;
        }
        let recipient = self.recipients.get(&actor.reservation.id)?;
        recipient
            .entries
            .range(cursor..)
            .chain(recipient.entries.range(..cursor))
            .find_map(|(&slot, entry)| {
                let bot = bots.get(&slot)?;
                (entry.through <= drained
                    && entry.allocation == bot.allocation_generation
                    && entry.sent != Some(bot.firearm_kd)
                    && !selected.contains(&slot))
                .then_some(Candidate {
                    slot,
                    allocation: entry.allocation,
                    totals: bot.firearm_kd,
                })
            })
    }

    pub(super) fn sent(&mut self, id: &str, candidate: Candidate) {
        if let Some(recipient) = self.recipients.get_mut(id)
            && let Some(entry) = recipient.entries.get_mut(&candidate.slot)
            && entry.allocation == candidate.allocation
        {
            entry.sent = Some(candidate.totals);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::*;

    const RECIPIENT: &str = "synthetic-bot-stat-recipient";
    fn fixture() -> (Engine<Vec<u8>>, u16) {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut engine = crate::bot_registry::identity_tests::populated_engine(&format!(
            "Synthetic Bot Stats {id}"
        ));
        let reply = engine.handle(
            Request::Reserve {
                v: BRIDGE_VERSION,
                request_id: "synthetic-reserve-bot-stats".into(),
                reservation_id: RECIPIENT.into(),
                world_id: 1,
                player_id: 0,
                account_id: 1,
                session_key: "aaaaaaaaaaaaaaaa".into(),
                access_key: "bbbbbbbbbbbbbbbb".into(),
                character: serde_json::from_value(json!({"name":"Synthetic Observer","faction":2,
                "head":0,"voice":0,"universe_id":26,"character_id":1}))
                .unwrap(),
            },
            Duration::ZERO,
            1,
        );
        assert!(reply.ok, "{:?}", reply.error);
        crate::spawn_progression::ready(engine.active.as_mut().unwrap());
        let slot = *engine.bot_registry.bots.keys().next().unwrap();
        (engine, slot)
    }
    fn empty(engine: &Engine<Vec<u8>>) -> Vec<u8> {
        replication::ObjectUpdate {
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
        .unwrap()
    }
    fn fill(engine: &mut Engine<Vec<u8>>, count: usize) {
        let packet = empty(engine);
        for _ in 0..count {
            engine.active.as_mut().unwrap().queue_shared(packet.clone());
        }
    }
    fn stat_rows(messages: &[OutboundMessage]) -> Vec<&str> {
        messages
            .iter()
            .filter(|m| m.payload_hex.starts_with("0204"))
            .map(|m| m.payload_hex.as_str())
            .collect()
    }
    fn expect_row(slot: u16, kills: u8, deaths: u8) -> String {
        // Independently specified retail continuation wire: Character + two absolute counters.
        format!("0204{slot:04x}020a{kills:02x}0c{deaths:02x}")
    }

    #[test]
    fn bot_statistics_follow_actual_creation_drain_and_precede_unrelated_backlog() {
        let (mut engine, slot) = fixture();
        fill(&mut engine, 40);
        engine.tick_bots(Duration::ZERO, 1).unwrap();
        fill(&mut engine, 60);
        let first = engine.drain_shared(Vec::new(), 1).unwrap();
        assert_eq!(first.len(), 32);
        assert!(stat_rows(&first).is_empty());
        assert_eq!(engine.active.as_ref().unwrap().shared_frames_drained, 32);
        let before = engine.active.as_ref().unwrap().outbound_sequence;
        let next = engine.drain_shared(Vec::new(), 1).unwrap();
        assert_eq!(stat_rows(&next), [expect_row(slot, 0, 0)]);
        let stat_at = next
            .iter()
            .position(|m| m.payload_hex.starts_with("0204"))
            .unwrap();
        assert!(
            stat_at >= 9,
            "eight older frames and creation must leave first"
        );
        let actor = engine.active.as_ref().unwrap();
        assert!(
            !actor.pending_shared.is_empty(),
            "later traffic cannot starve bot statistics"
        );
        assert_eq!(actor.outbound_sequence - before, (next.len() - 1) as u32);
        assert_eq!(actor.shared_frames_drained, 63);
        assert!(
            actor
                .replication_history
                .iter()
                .all(|p| !p.starts_with(&[2, 4]))
        );
    }

    #[test]
    fn bot_statistics_coalesce_latest_totals_and_do_not_consume_replication_sequence() {
        let (mut engine, slot) = fixture();
        engine.tick_bots(Duration::ZERO, 1).unwrap();
        engine.bot_registry.bots.get_mut(&slot).unwrap().firearm_kd =
            crate::bot_firearm_kd::Totals {
                kills: 3,
                assists: 0,
                deaths: 2,
            };
        let reply = engine.drain_shared(Vec::new(), 1).unwrap();
        assert_eq!(stat_rows(&reply), [expect_row(slot, 3, 2)]);
        let actor = engine.active.as_ref().unwrap();
        let sequence = actor.outbound_sequence;
        let history = actor.replication_history.len();
        assert!(engine.drain_shared(Vec::new(), 1).unwrap().is_empty());
        engine
            .bot_registry
            .bots
            .get_mut(&slot)
            .unwrap()
            .firearm_kd
            .kills = 4;
        let reply = engine.drain_shared(Vec::new(), 1).unwrap();
        assert_eq!(stat_rows(&reply), [expect_row(slot, 4, 2)]);
        assert_eq!(engine.active.as_ref().unwrap().outbound_sequence, sequence);
        assert_eq!(
            engine.active.as_ref().unwrap().replication_history.len(),
            history
        );
    }

    #[test]
    fn firearm_assists_add_stat_11_only_after_creation_drains() {
        let (engine, slot) = fixture();
        let mut engine = engine
            .with_firearm_assist_rules(crate::firearm_assists::Rules {
                schema_version: 1,
                application_id: 20231,
                assist_timeout_ms: 17,
            })
            .unwrap();
        fill(&mut engine, 40);
        engine.tick_bots(Duration::ZERO, 1).unwrap();
        engine
            .bot_registry
            .bots
            .get_mut(&slot)
            .unwrap()
            .firearm_kd
            .assists = 1;
        assert!(stat_rows(&engine.drain_shared(Vec::new(), 1).unwrap()).is_empty());
        let updated = engine.drain_shared(Vec::new(), 1).unwrap();
        assert_eq!(
            stat_rows(&updated),
            [format!("0204{slot:04x}030a000b010c00")]
        );
        assert!(engine.drain_shared(Vec::new(), 1).unwrap().is_empty());
    }

    #[test]
    fn bot_statistics_record_refusal_preserves_creation_fence_and_sent_baseline() {
        let (mut engine, slot) = fixture();
        engine.tick_bots(Duration::ZERO, 1).unwrap();
        let queue = engine.active.as_ref().unwrap().pending_shared.clone();
        let sequence = engine.active.as_ref().unwrap().outbound_sequence;
        engine.max_log = engine.written;
        assert_eq!(
            engine.drain_shared(Vec::new(), 1).unwrap_err(),
            "log_limit_reached"
        );
        let actor = engine.active.as_ref().unwrap();
        assert_eq!(actor.pending_shared, queue);
        assert_eq!(actor.shared_frames_drained, 0);
        assert_eq!(actor.outbound_sequence, sequence);
        let ticket = &engine.bot_registry.stats_publication.recipients[RECIPIENT].entries[&slot];
        assert_eq!(ticket.sent, None);
        assert_eq!(
            engine.bot_registry.bots[&slot].firearm_kd,
            crate::bot_firearm_kd::Totals::default()
        );
        assert!(
            engine.failed,
            "a recorder failure permanently refuses this process"
        );
    }

    #[test]
    fn bot_statistics_defer_at_byte_budget_without_marking_totals_sent() {
        let (mut engine, slot) = fixture();
        engine.tick_bots(Duration::ZERO, 1).unwrap();
        engine.drain_shared(Vec::new(), 1).unwrap();
        engine
            .bot_registry
            .bots
            .get_mut(&slot)
            .unwrap()
            .firearm_kd
            .kills = 3;
        let large = OutboundMessage {
            reservation_id: RECIPIENT.into(),
            world_id: 1,
            player_id: 0,
            plugin_id: 1,
            payload_hex: "00".repeat(23_996),
        };
        let reply = engine.drain_shared(vec![large], 1).unwrap();
        assert_eq!(reply.len(), 1);
        assert!(stat_rows(&reply).is_empty());
        assert_eq!(
            stat_rows(&engine.drain_shared(Vec::new(), 1).unwrap()),
            [expect_row(slot, 3, 0)]
        );
    }

    #[test]
    fn bot_statistics_late_join_receives_current_absolute_totals_after_its_snapshot() {
        let (mut engine, _) = fixture();
        engine.maximum_players = 4;
        engine.bot_names = crate::bot_names::Catalog::from_json(&serde_json::to_vec(&json!({"entries":[
            {"name":"Synthetic One"},{"name":"Synthetic Two"},{"name":"Synthetic Three"},{"name":"Synthetic Four"}
        ]})).unwrap()).unwrap();
        let mut config = engine.bot_registry.config.clone().unwrap();
        for team in 0..2 {
            let mut slot = config.slots[team].clone();
            slot.position.x = 2. + team as f32 * 4.;
            slot.position.z = 6.;
            config.slots.push(slot);
        }
        engine = engine.with_bot_population(config, 3).unwrap();
        engine.active.as_mut().unwrap().life = CharacterLife::Alive {
            generation: 1,
            health_quarter_points: 400,
        };
        engine.tick_bots(Duration::ZERO, 1).unwrap();
        let team = engine.active.as_ref().unwrap().team();
        let slot = *engine
            .bot_registry
            .bots
            .iter()
            .find(|(_, bot)| bot.target.squad.team() != team)
            .unwrap()
            .0;
        engine.tick_bots(Duration::ZERO, 1).unwrap();
        engine.bot_registry.bots.get_mut(&slot).unwrap().firearm_kd =
            crate::bot_firearm_kd::Totals {
                kills: 3,
                assists: 0,
                deaths: 2,
            };
        engine.drain_shared(Vec::new(), 1).unwrap();
        let reply = engine.handle(
            Request::Reserve {
                v: BRIDGE_VERSION,
                request_id: "synthetic-late-bot-stat".into(),
                reservation_id: "synthetic-late-observer".into(),
                world_id: 1,
                player_id: 3,
                account_id: 4,
                session_key: "aaaaaaaaaaaaaaaa".into(),
                access_key: "bbbbbbbbbbbbbbbb".into(),
                character: serde_json::from_value(
                    json!({"name":"Synthetic Late Observer","faction":2,
                "head":0,"voice":0,"universe_id":26,"character_id":3}),
                )
                .unwrap(),
            },
            Duration::from_millis(1),
            1,
        );
        assert!(reply.ok, "{:?}", reply.error);
        let late = engine.active.as_mut().unwrap();
        assert_eq!(late.reservation.player, 3);
        crate::spawn_progression::ready(late);
        engine.tick_bots(Duration::from_millis(1), 1).unwrap();
        let reply = engine.drain_shared(Vec::new(), 1).unwrap();
        let late_rows: Vec<_> = reply.iter().filter(|m| m.player_id == 3).collect();
        assert!(late_rows.len() >= 2);
        assert!(!late_rows[0].payload_hex.starts_with("0204"));
        assert_eq!(
            late_rows
                .iter()
                .filter(|m| m.payload_hex.starts_with("0204"))
                .map(|m| m.payload_hex.as_str())
                .collect::<Vec<_>>(),
            [expect_row(slot, 3, 2)]
        );
        assert!(
            reply
                .iter()
                .filter(|m| m.player_id == 0)
                .all(|m| !m.payload_hex.starts_with("0204"))
        );
    }

    #[test]
    fn bot_statistics_revalidate_retired_or_replaced_allocation() {
        let (mut engine, slot) = fixture();
        engine.tick_bots(Duration::ZERO, 1).unwrap();
        let actor = engine.active.as_mut().unwrap();
        // Actual Character creation has drained; preserve the now-stale stat ticket.
        while let Some(wire) = actor.pending_shared.front().cloned() {
            let packet = replication::multiplayer::for_recipient_with_profile(
                &wire,
                actor.next_sequence().unwrap(),
                actor.wire_profile,
            )
            .unwrap();
            actor.remember(packet).unwrap();
            actor.pop_shared();
        }
        engine
            .bot_registry
            .bots
            .get_mut(&slot)
            .unwrap()
            .allocation_generation += 1;
        engine.bot_registry.bots.get_mut(&slot).unwrap().firearm_kd =
            crate::bot_firearm_kd::Totals {
                kills: 7,
                assists: 0,
                deaths: 4,
            };
        assert!(stat_rows(&engine.drain_shared(Vec::new(), 1).unwrap()).is_empty());
        engine.bot_registry.bots.remove(&slot);
        assert!(stat_rows(&engine.drain_shared(Vec::new(), 1).unwrap()).is_empty());
    }
}
