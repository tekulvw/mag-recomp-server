//! Read-only, recorded FIFO publication of committed retail inherited tiers.
use crate::{
    Active, Engine,
    replication::{BitWriter, UpdateHeader},
    wire_profile::WireProfile,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write,
};

pub(crate) fn level_rows(
    rows: &[(u16, u8)],
    sequence: u32,
    timestamp: u32,
    profile: WireProfile,
) -> Result<Vec<u8>, &'static str> {
    if profile.application_id() != 20231 || rows.is_empty() {
        return Err("invalid_inherited_leadership_wire");
    }
    let mut sorted = BTreeMap::new();
    for &(index, level) in rows {
        if index == u16::MAX || level > 3 || sorted.insert(index, level).is_some() {
            return Err("invalid_inherited_leadership_wire");
        }
    }
    let mut bits = BitWriter::with_profile(profile);
    UpdateHeader {
        sequence,
        nak_sequence: 0,
        client_index: 0,
        object_type_mask: 1 << 3,
        timestamp_ms: timestamp,
    }
    .encode(&mut bits)?;
    for (index, level) in sorted {
        bits.write(1, 1)?;
        bits.write(index.into(), 16)?;
        for field in 0..22 {
            bits.write(u32::from(field == 19), 1)?;
            if field == 19 {
                bits.write(level.into(), 8)?;
            }
        }
    }
    bits.write(0, 1)?;
    Ok(bits.finish())
}

/// Character allocation creation persists across life and squad changes.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Creation {
    world: u32,
    round: u64,
    reservation: String,
    allocation: u64,
    player: u16,
}
impl Creation {
    fn capture(a: &Active, round: u64) -> Self {
        Self {
            world: a.reservation.world,
            round,
            reservation: a.reservation.id.clone(),
            allocation: a.allocation_sequence,
            player: a.reservation.player,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Subject {
    creation: Creation,
    life: u32,
    squad: u16,
}
impl Subject {
    fn capture(a: &Active, round: u64) -> Self {
        let (world, life, squad) = a.leadership_scope();
        debug_assert_eq!(world, a.reservation.world);
        Self {
            creation: Creation::capture(a, round),
            life,
            squad,
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct View {
    observer: Option<Subject>,
    rows: BTreeMap<u16, (Subject, u8)>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct State {
    introduced: BTreeMap<Creation, BTreeSet<Creation>>,
    last_queued: BTreeMap<Creation, View>,
}
impl<W: Write> Engine<W> {
    /// Called only after the normal Character snapshot creation record succeeds.
    /// The snapshots are already in each observer's FIFO before any tier row.
    pub(crate) fn note_leadership_character_created(&mut self) {
        if self.leadership.is_none() || self.wire_profile.application_id() != 20231 {
            return;
        }
        let Some(new) = self.active.as_ref().filter(|a| a.initial_objects.is_some()) else {
            return;
        };
        let key = Creation::capture(new, self.round_number);
        // Creation is the only point at which another allocation can grow this
        // map. Prune here even when every committed tier is zero and no frame
        // needs publication.
        let current = self
            .active
            .iter()
            .chain(self.peers.values())
            .map(|a| Creation::capture(a, self.round_number))
            .collect::<BTreeSet<_>>();
        self.inherited_leadership
            .introduced
            .retain(|observer, _| current.contains(observer));
        for known in self.inherited_leadership.introduced.values_mut() {
            known.retain(|subject| current.contains(subject));
        }
        self.inherited_leadership
            .last_queued
            .retain(|observer, _| current.contains(observer));
        let peers = self
            .peers
            .values()
            .filter(|a| a.initial_objects.is_some() && !a.replication_backlogged)
            .map(|a| Creation::capture(a, self.round_number))
            .collect::<Vec<_>>();
        if !new.replication_backlogged {
            self.inherited_leadership
                .introduced
                .entry(key.clone())
                .or_default()
                .extend(peers.iter().cloned());
        }
        for peer in peers {
            self.inherited_leadership
                .introduced
                .entry(peer)
                .or_default()
                .insert(key.clone());
        }
    }
    pub(crate) fn publish_inherited_leadership(&mut self, wall: u64) -> Result<(), &'static str> {
        if self.leadership.is_none() || self.wire_profile.application_id() != 20231 {
            return Ok(());
        }
        let actors = self
            .active
            .iter()
            .chain(self.peers.values())
            .filter(|a| a.ready_for_updates && a.initial_objects.is_some())
            .collect::<Vec<_>>();
        let current = self
            .active
            .iter()
            .chain(self.peers.values())
            .filter(|a| a.initial_objects.is_some())
            .map(|a| (Creation::capture(a, self.round_number), a))
            .collect::<BTreeMap<_, _>>();
        let mut next = self.inherited_leadership.clone();
        next.introduced.retain(|o, _| current.contains_key(o));
        for known in next.introduced.values_mut() {
            known.retain(|s| current.contains_key(s));
        }
        next.last_queued.retain(|o, _| current.contains_key(o));
        let mut packets = Vec::new();
        for (&actor, key) in actors.iter().zip(
            actors
                .iter()
                .map(|a| Creation::capture(a, self.round_number)),
        ) {
            if actor.replication_backlogged
                || actor.ordinary_shared_len() >= actor.shared_queue_limit
            {
                continue;
            }
            let before = next.last_queued.get(&key);
            let mut after = View {
                observer: Some(Subject::capture(actor, self.round_number)),
                rows: BTreeMap::new(),
            };
            let mut rows = Vec::new();
            for (subject_key, subject) in &current {
                if subject_key.world != key.world
                    || (subject_key != &key
                        && !next
                            .introduced
                            .get(&key)
                            .is_some_and(|known| known.contains(subject_key)))
                {
                    continue;
                }
                let identity = Subject::capture(subject, self.round_number);
                let level = subject.committed_inherited_leadership();
                let previous = before.and_then(|v| v.rows.get(&subject_key.player));
                if (level != 0 || previous.is_some_and(|(_, v)| *v != 0))
                    && (previous != Some(&(identity.clone(), level))
                        || before.is_some_and(|v| v.observer != after.observer))
                {
                    rows.push((subject_key.player, level));
                }
                after.rows.insert(subject_key.player, (identity, level));
            }
            if !rows.is_empty() {
                packets.push((
                    key.clone(),
                    level_rows(
                        &rows,
                        1,
                        actor.last_event.as_millis() as u32,
                        self.wire_profile,
                    )?,
                ));
                next.last_queued.insert(key, after);
            }
        }
        if packets.is_empty() {
            return Ok(());
        }
        self.record(serde_json::json!({"op":"inherited_leadership_queued","packets":packets.iter().map(|(o,p)|serde_json::json!({"reservation":o.reservation,"allocation":o.allocation,"packet":crate::protocol::encode_hex(p)})).collect::<Vec<_>>()}),wall)?;
        self.inherited_leadership = next;
        for (key, packet) in packets {
            if let Some(actor) = self
                .active
                .iter_mut()
                .chain(self.peers.values_mut())
                .find(|a| Creation::capture(a, self.round_number) == key)
            {
                actor.queue_shared(packet);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inherited_field19_independent_unsigned_byte_literals() {
        let profile = crate::wire_profile::WireProfileCatalog::embedded()
            .profile(20231)
            .unwrap();
        for (level, literal) in [
            (0, "00080000000700000000000000008000004d28001800008000"),
            (1, "00080000000700000000000000008000004d28001800008080"),
            (2, "00080000000700000000000000008000004d28001800008100"),
            (3, "00080000000700000000000000008000004d28001800008180"),
        ] {
            assert_eq!(
                crate::protocol::encode_hex(&level_rows(&[(3, level)], 7, 1234, profile).unwrap()),
                literal
            );
        }
    }
}

#[cfg(test)]
mod runtime_tests {
    use super::*;
    use crate::{
        CharacterLife,
        replication::{BitReader, ObjectUpdate},
    };
    use std::time::Duration;

    pub(super) fn fixture() -> Engine<Vec<u8>> {
        let mut engine = crate::active_ability_engine_tests::configured();
        for slot in 0..2 {
            crate::active_ability_engine_tests::add_ready(&mut engine, slot);
        }
        for actor in engine.active.iter_mut().chain(engine.peers.values_mut()) {
            actor.nominations = u8::from(actor.reservation.player == 0);
            actor.claimed = Some(crate::Claimed {
                wire: Vec::new(),
                np_identity: [1; 36],
                game_info: crate::protocol::GameInfo {
                    client_index: actor.reservation.player,
                    level_name: "synthetic_ability_arena".into(),
                    start_time_ms: 0,
                    attacking_faction: 0,
                    defending_faction: 1,
                    maximum_points: [20; 2],
                    current_points: [20; 2],
                    game_type: 2,
                },
            });
            actor.owner_fields.state = Some(crate::replication::CharacterState {
                state: 1,
                stance: 0,
                user_data: [0; 3],
            });
        }
        let squad = current(&engine, 0).squad_catalog.clone();
        current_mut(&mut engine, 1).squad_catalog = squad;
        engine.match_started = true;
        engine.reconcile_squad_leaders().unwrap();
        engine
    }
    pub(super) fn refresh(e: &mut Engine<Vec<u8>>, ms: u64) {
        e.tick_leadership_auto_heal(Duration::from_millis(ms), 1)
            .unwrap();
    }
    fn values(bytes: &[u8]) -> Vec<(u16, u8)> {
        let p = crate::wire_profile::WireProfileCatalog::embedded()
            .profile(20231)
            .unwrap();
        assert_eq!(
            ObjectUpdate::decode_with_profile(bytes, p)
                .unwrap()
                .header
                .object_type_mask,
            8
        );
        let mut bits = BitReader::with_profile(bytes, p);
        for width in [16, 32, 32, 16, 20, 32] {
            bits.read(width).unwrap();
        }
        let mut out = Vec::new();
        while bits.read(1).unwrap() != 0 {
            let index = bits.read(16).unwrap() as u16;
            let mut value = None;
            for field in 0..22 {
                if bits.read(1).unwrap() != 0 {
                    assert_eq!(field, 19);
                    value = Some(bits.read(8).unwrap() as u8);
                }
            }
            out.push((index, value.unwrap()));
        }
        out
    }
    pub(super) fn current(e: &Engine<Vec<u8>>, slot: u16) -> &Active {
        e.active
            .iter()
            .chain(e.peers.values())
            .find(|a| a.reservation.player == slot)
            .unwrap()
    }
    pub(super) fn current_mut(e: &mut Engine<Vec<u8>>, slot: u16) -> &mut Active {
        e.active
            .iter_mut()
            .chain(e.peers.values_mut())
            .find(|a| a.reservation.player == slot)
            .unwrap()
    }
    pub(super) fn take(e: &mut Engine<Vec<u8>>, slot: u16) -> Vec<(u16, u8)> {
        values(&current_mut(e, slot).pending_shared.pop_back().unwrap())
    }

    #[test]
    fn inherited_field19_only_current_known_characters_after_snapshot_fifo() {
        let mut e = fixture();
        refresh(&mut e, 10);
        assert_eq!(current(&e, 0).committed_inherited_leadership(), 1);
        assert_eq!(current(&e, 1).committed_inherited_leadership(), 1);
        e.publish_inherited_leadership(1).unwrap();
        assert_eq!(take(&mut e, 0), vec![(0, 1)]);
        assert_eq!(take(&mut e, 1), vec![(1, 1)]);
        // Explicit synthetic existing snapshots, matching the normal creation seam.
        let snapshot = current(&e, 0).snapshot(Duration::ZERO).unwrap();
        current_mut(&mut e, 1).queue_shared(snapshot.clone());
        e.note_leadership_character_created();
        e.publish_inherited_leadership(1).unwrap();
        assert_eq!(current(&e, 1).pending_shared.front(), Some(&snapshot));
        assert_eq!(take(&mut e, 0), vec![(1, 1)]);
        assert_eq!(take(&mut e, 1), vec![(0, 1)]);
        current_mut(&mut e, 1).pending_shared.clear();
        e.publish_inherited_leadership(1).unwrap();
        assert!(current(&e, 1).pending_shared.is_empty());
    }
    #[test]
    fn inherited_field19_expiry_uses_existing_hold_grace_and_clears_peer_snapshot() {
        let mut e = fixture();
        e.note_leadership_character_created();
        refresh(&mut e, 10);
        e.publish_inherited_leadership(1).unwrap();
        assert_eq!(take(&mut e, 1), vec![(0, 1), (1, 1)]);
        take(&mut e, 0);
        current_mut(&mut e, 0).tracking.server_position(
            crate::replication::CharacterPosition {
                x: 150.,
                y: 3.,
                z: 100.,
            },
            crate::player_tracking::PositionOrigin::ServerSpawn,
            11,
        );
        refresh(&mut e, 309);
        e.publish_inherited_leadership(1).unwrap();
        assert!(current(&e, 1).pending_shared.is_empty());
        refresh(&mut e, 360);
        e.publish_inherited_leadership(1).unwrap();
        assert_eq!(take(&mut e, 1), vec![(1, 0)]);
        assert_eq!(take(&mut e, 0), vec![(1, 0)]);
        assert_eq!(current(&e, 1).committed_inherited_leadership(), 0);
    }
    #[test]
    fn inherited_field19_life_and_slot_replacement_clear_without_duplicate_rows() {
        let mut e = fixture();
        e.note_leadership_character_created();
        refresh(&mut e, 10);
        e.publish_inherited_leadership(1).unwrap();
        take(&mut e, 0);
        take(&mut e, 1);
        current_mut(&mut e, 0).life = CharacterLife::Alive {
            generation: 2,
            health_quarter_points: 400,
        };
        e.publish_inherited_leadership(1).unwrap();
        assert_eq!(take(&mut e, 1), vec![(0, 0)]);
        assert_eq!(take(&mut e, 0), vec![(0, 0), (1, 1)]);
        refresh(&mut e, 11);
        e.publish_inherited_leadership(1).unwrap();
        assert_eq!(take(&mut e, 1), vec![(0, 1)]);
        take(&mut e, 0);
        let a = current_mut(&mut e, 0);
        a.reservation.id = "synthetic-replacement".into();
        a.allocation_sequence += 100;
        a.leadership = None;
        // Until the replacement snapshot is queued, this slot is not introduced.
        e.publish_inherited_leadership(1).unwrap();
        assert!(current(&e, 1).pending_shared.is_empty());
        let replacement = Creation::capture(current(&e, 0), e.round_number);
        let observer = Creation::capture(current(&e, 1), e.round_number);
        e.inherited_leadership
            .introduced
            .entry(observer)
            .or_default()
            .insert(replacement);
        e.publish_inherited_leadership(1).unwrap();
        assert_eq!(take(&mut e, 1), vec![(0, 0)]);
    }
    #[test]
    fn inherited_field19_publication_record_refusal_preserves_committed_cache_queue_and_retry() {
        let mut e = fixture();
        e.note_leadership_character_created();
        refresh(&mut e, 10);
        let before = e.inherited_leadership.clone();
        let written = e.written;
        let max = e.max_log;
        e.max_log = written;
        assert_eq!(e.publish_inherited_leadership(1), Err("log_limit_reached"));
        assert_eq!(e.inherited_leadership, before);
        for a in e.active.iter().chain(e.peers.values()) {
            assert!(a.pending_shared.is_empty());
            assert_eq!(a.committed_inherited_leadership(), 1);
        }
        e.failed = false;
        e.max_log = max;
        e.publish_inherited_leadership(1).unwrap();
        assert_eq!(take(&mut e, 1), vec![(0, 1), (1, 1)]);
    }
    #[test]
    fn inherited_field19_primary_record_refusal_does_not_install_cache_or_publish() {
        let mut e = fixture();
        e.note_leadership_character_created();
        let before = e.inherited_leadership.clone();
        e.max_log = e.written;
        assert_eq!(
            e.tick_leadership_auto_heal(Duration::from_millis(10), 1),
            Err("log_limit_reached")
        );
        assert_eq!(e.inherited_leadership, before);
        for a in e.active.iter().chain(e.peers.values()) {
            assert!(a.pending_shared.is_empty());
            assert_eq!(a.committed_inherited_leadership(), 0);
        }
    }
    #[test]
    fn inherited_field19_full_fifo_defers_without_marking_backlog_or_advancing_ledger() {
        let mut e = fixture();
        e.note_leadership_character_created();
        refresh(&mut e, 10);
        let before = e.inherited_leadership.clone();
        let filler = level_rows(&[(0, 0)], 1, 0, e.wire_profile).unwrap();
        for a in e.active.iter_mut().chain(e.peers.values_mut()) {
            for _ in 0..crate::multiplayer::MAX_SHARED_QUEUE {
                a.queue_shared(filler.clone());
            }
        }
        e.publish_inherited_leadership(1).unwrap();
        assert_eq!(e.inherited_leadership, before);
        assert!(
            e.active
                .iter()
                .chain(e.peers.values())
                .all(|a| !a.replication_backlogged)
        );
        let first = e.drain_shared(Vec::new(), 1).unwrap();
        assert!(!first.is_empty());
        assert_eq!(e.inherited_leadership, before);
        let next = e.drain_shared(Vec::new(), 1).unwrap();
        assert!(!next.is_empty());
        assert_ne!(e.inherited_leadership, before);
        assert!(
            e.active
                .iter()
                .chain(e.peers.values())
                .all(|a| !a.replication_backlogged)
        );
    }
    #[test]
    fn inherited_field19_disabled_prototype_and_owner_authority_remain_unchanged() {
        let mut e = fixture();
        refresh(&mut e, 10);
        e.leadership = None;
        let bytes = e.written;
        e.publish_inherited_leadership(1).unwrap();
        assert_eq!(e.written, bytes);
        assert!(
            e.active
                .iter()
                .chain(e.peers.values())
                .all(|a| a.pending_shared.is_empty())
        );
        assert!(level_rows(&[(3, 1)], 1, 0, WireProfile::prototype()).is_err());
        let p = e.wire_profile;
        let bytes = level_rows(&[(1, 3)], 1, 0, p).unwrap();
        let update = ObjectUpdate::decode_with_profile(&bytes, p).unwrap();
        assert_eq!(
            crate::owner_update::TypedOwnerUpdateFields::decode_with_profile(&update, 1, p)
                .unwrap_err(),
            "unsupported_client_character_field"
        );
        assert!(level_rows(&[(3, 4)], 1, 0, p).is_err());
        assert!(level_rows(&[(3, 1), (3, 2)], 1, 0, p).is_err());
        assert_eq!(
            level_rows(&(0..64).map(|i| (i, 3)).collect::<Vec<_>>(), 1, 0, p)
                .unwrap()
                .len(),
            395
        );
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::runtime_tests::{current, current_mut, fixture, refresh, take};
    use crate::CharacterLife;

    #[test]
    fn inherited_field19_squad_world_and_incapacitation_follow_existing_cache_scope() {
        let mut e = fixture();
        e.note_leadership_character_created();
        refresh(&mut e, 10);
        e.publish_inherited_leadership(1).unwrap();
        take(&mut e, 0);
        take(&mut e, 1);
        // Incapacitation is not an invented publication immunity or scope reset.
        let a = current_mut(&mut e, 1);
        a.life = CharacterLife::Incapacitated {
            generation: 1,
            downed: crate::incapacitation::Downed {
                deadline: std::time::Duration::from_secs(20),
                accumulated_damage_hp: 0.,
            },
        };
        e.publish_inherited_leadership(1).unwrap();
        assert!(current(&e, 1).pending_shared.is_empty());
        let squad = current(&e, 1).squad_catalog.for_squad(17).unwrap();
        current_mut(&mut e, 1).squad_catalog = std::sync::Arc::new(squad);
        e.publish_inherited_leadership(1).unwrap();
        assert_eq!(take(&mut e, 0), vec![(1, 0)]);
        assert_eq!(take(&mut e, 1), vec![(0, 1), (1, 0)]);
        current_mut(&mut e, 1).reservation.world = 2;
        e.publish_inherited_leadership(1).unwrap();
        assert!(current(&e, 0).pending_shared.is_empty());
        assert!(current(&e, 1).pending_shared.is_empty());
    }
    #[test]
    fn inherited_field19_quiet_zero_and_current_roster_pruning_remain_bounded() {
        let mut e = fixture();
        let written = e.written;
        e.note_leadership_character_created();
        e.publish_inherited_leadership(1).unwrap();
        assert_eq!(e.written, written);
        assert!(
            e.active
                .iter()
                .chain(e.peers.values())
                .all(|a| a.pending_shared.is_empty())
        );
        for ordinal in 0..80 {
            let a = e.active.as_mut().unwrap();
            a.reservation.id = format!("synthetic-zero-replacement-{ordinal}");
            a.allocation_sequence += 1;
            e.note_leadership_character_created();
        }
        assert_eq!(e.inherited_leadership.introduced.len(), 2);
        assert!(
            e.inherited_leadership
                .introduced
                .values()
                .all(|known| known.len() <= 1)
        );
    }
}
