//! Separate aim/flight warning contributions and ordinary field24 replication.
use crate::{
    Active, CharacterLife, Engine,
    replication::{BitWriter, UpdateHeader},
    wire_profile::WireProfile,
};
use std::{collections::BTreeMap, io::Write, time::Duration};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LockReport {
    pub target: u32,
    pub state: u8,
}
impl LockReport {
    pub(crate) fn decode(raw: &[u8]) -> Result<Self, &'static str> {
        if raw.len() != 7 || raw[..2] != [2, 0x29] || raw[6] > 2 {
            return Err("invalid_rocket_lock_state");
        }
        Ok(Self {
            target: u32::from_be_bytes(
                raw[2..6]
                    .try_into()
                    .map_err(|_| "invalid_rocket_lock_state")?,
            ),
            state: raw[6],
        })
    }
}
#[cfg(test)]
fn warning_wire(
    family: u8,
    index: u16,
    state: u8,
    sequence: u32,
    timestamp: u32,
    profile: WireProfile,
) -> Result<Vec<u8>, &'static str> {
    warning_rows(&[(family, index, state)], sequence, timestamp, profile)
}
fn warning_rows(
    rows: &[(u8, u16, u8)],
    sequence: u32,
    timestamp: u32,
    profile: WireProfile,
) -> Result<Vec<u8>, &'static str> {
    if profile.application_id() != 20231 || rows.is_empty() {
        return Err("invalid_guided_warning_wire");
    }
    let mut sorted = BTreeMap::new();
    let mut mask = 0;
    for &(family, index, state) in rows {
        if !matches!(family, 1 | 2 | 13)
            || state > 2
            || sorted.insert((family, index), state).is_some()
        {
            return Err("invalid_guided_warning_wire");
        }
        mask |= 1 << family;
    }
    let mut bits = BitWriter::with_profile(profile);
    UpdateHeader {
        sequence,
        nak_sequence: 0,
        client_index: 0,
        object_type_mask: mask,
        timestamp_ms: timestamp,
    }
    .encode(&mut bits)?;
    for family in [1, 2, 13] {
        if mask & (1 << family) == 0 {
            continue;
        }
        for (&(_, index), &state) in sorted.iter().filter(|((f, _), _)| *f == family) {
            bits.write(1, 1)?;
            bits.write(index.into(), 16)?;
            let count = match family {
                1 => 30,
                2 => 27,
                _ => 25,
            };
            for field in 0..count {
                bits.write(u32::from(field == 24), 1)?;
                if field == 24 {
                    bits.write(state.into(), 8)?;
                }
            }
        }
        bits.write(0, 1)?;
    }
    Ok(bits.finish())
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Target {
    pub world: u32,
    pub round: u64,
    pub incarnation: u64,
    pub family: u8,
    pub index: u16,
    pub generation: u32,
    pub source: String,
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Owner {
    world: u32,
    round: u64,
    reservation: String,
    allocation: u64,
    life: u32,
}
impl Owner {
    fn capture(actor: &Active, round: u64) -> Option<Self> {
        let CharacterLife::Alive {
            generation,
            health_quarter_points: 1..,
        } = actor.life
        else {
            return None;
        };
        (!actor.is_mounted()
            && actor.ready_for_updates
            && actor.entered_game
            && actor.initial_objects.is_some())
        .then(|| Self {
            world: actor.reservation.world,
            round,
            reservation: actor.reservation.id.clone(),
            allocation: actor.allocation_sequence,
            life: generation,
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Observer {
    reservation: String,
    allocation: u64,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Ledger {
    aims: BTreeMap<Owner, (Target, u8)>,
    last_queued: BTreeMap<Observer, BTreeMap<Target, u8>>,
}
pub(crate) struct Candidate {
    pub ledger: Ledger,
    publications: Vec<(Observer, Vec<u8>)>,
}
impl Candidate {
    pub(crate) fn audit(&self) -> serde_json::Value {
        serde_json::json!({"packets":self.publications.iter().map(|(o,p)|serde_json::json!({"reservation":o.reservation,"allocation":o.allocation,"packet":crate::protocol::encode_hex(p)})).collect::<Vec<_>>()})
    }
}
impl<W: Write> Engine<W> {
    pub(crate) fn guided_target(&self, world: u32, packed: u32) -> Result<Target, &'static str> {
        let first = self
            .active
            .iter()
            .chain(self.peers.values())
            .next()
            .ok_or("guided_target_world_unavailable")?;
        let key = crate::acquisition_atomic::WorldKey {
            world_id: first.reservation.world,
            incarnation: self.round_number.max(1),
        };
        if world != key.world_id
            || self
                .active
                .iter()
                .chain(self.peers.values())
                .any(|a| a.reservation.world != key.world_id)
        {
            return Err("guided_target_world_mismatch");
        }
        // Represented handles admit only the ordinary low16 index and known active bit.
        if !matches!(packed & 0x00ff_0000, 0 | 0x0080_0000) {
            return Err("guided_target_handle_bits_unsupported");
        }
        let family = (packed >> 24) as u8;
        let index = packed as u16;
        let spawn = self
            .spawn_catalog
            .as_ref()
            .and_then(|s| s.source_sha256())
            .ok_or("guided_target_source_required")?;
        let (generation, source) = match family {
            1 => {
                let v = self
                    .ground_runtime
                    .as_ref()
                    .and_then(|r| r.get(index))
                    .ok_or("guided_target_unavailable")?;
                (
                    v.key.generation,
                    format!("{spawn}:ground:{}:{}", v.vehicle_type_token, v.faction),
                )
            }
            2 => {
                let c = self
                    .domination_gameplay
                    .as_ref()
                    .and_then(|w| w.state.helo.as_ref())
                    .and_then(|f| f.carriers.iter().find(|c| c.source.object_index == index))
                    .ok_or("guided_target_unavailable")?;
                (
                    0,
                    format!(
                        "{spawn}:helo:{}:{}:{}",
                        c.source.spawn_index, c.source.spawn_token, c.source.vehicle_type_token
                    ),
                )
            }
            13 => {
                let b = self
                    .bunker_world
                    .as_ref()
                    .and_then(|w| w.domination_bunker(index))
                    .ok_or("guided_target_unavailable")?;
                (
                    0,
                    format!("{spawn}:hospital:{}:{}", b.spawn_index, b.properties_source),
                )
            }
            33 => return Err("guided_target_family33_unsupported"),
            _ => return Err("guided_target_family_unsupported"),
        };
        Ok(Target {
            world,
            round: self.round_number,
            incarnation: key.incarnation,
            family,
            index,
            generation,
            source,
        })
    }
    pub(crate) fn guided_anchor(&self, target: &Target) -> Option<[f64; 3]> {
        if self
            .guided_target(
                target.world,
                (u32::from(target.family) << 24) | u32::from(target.index),
            )
            .ok()
            .as_ref()
            != Some(target)
        {
            return None;
        }
        let anchors = &self.guided_launchers.as_ref()?.guidance.anchor_y_m;
        let (mut position, y) = match target.family {
            1 => {
                let v = self.ground_runtime.as_ref()?.get(target.index)?;
                (
                    [v.pose.position.x, v.pose.position.y, v.pose.position.z].map(f64::from),
                    anchors.ground,
                )
            }
            2 => {
                let c = self
                    .domination_gameplay
                    .as_ref()?
                    .state
                    .helo
                    .as_ref()?
                    .carriers
                    .iter()
                    .find(|c| c.source.object_index == target.index)?;
                let p = c.current_pose?;
                (
                    [p.position.x, p.position.y, p.position.z].map(f64::from),
                    anchors.helo,
                )
            }
            13 => {
                let b = self
                    .bunker_world
                    .as_ref()?
                    .domination_bunker(target.index)?;
                (
                    [b.position.x, b.position.y, b.position.z].map(f64::from),
                    anchors.hospital,
                )
            }
            _ => return None,
        };
        position[1] += f64::from(y);
        Some(position)
    }
    pub(crate) fn prepare_guided_warnings(
        &self,
        report: Option<(Owner, Target, u8)>,
        override_state: Option<(&str, &crate::infantry_rocket_runtime::State)>,
        invalidated: &[String],
        now: Duration,
    ) -> Result<Candidate, &'static str> {
        let mut ledger = self.guided_warnings.clone();
        ledger.aims.retain(|owner, (target, _)| {
            !invalidated.contains(&owner.reservation)
                && self
                    .active
                    .iter()
                    .chain(self.peers.values())
                    .any(|a| Owner::capture(a, self.round_number).as_ref() == Some(owner))
                && self
                    .guided_target(
                        target.world,
                        (u32::from(target.family) << 24) | u32::from(target.index),
                    )
                    .ok()
                    .as_ref()
                    == Some(target)
        });
        if let Some((owner, target, state)) = report {
            if state == 0 {
                ledger.aims.remove(&owner);
            } else {
                ledger.aims.insert(owner, (target, state));
            }
        }
        let mut values = BTreeMap::<Target, u8>::new();
        for (target, state) in ledger.aims.values() {
            values
                .entry(target.clone())
                .and_modify(|v| *v = (*v).max(*state))
                .or_insert(*state);
        }
        for actor in self.active.iter().chain(self.peers.values()) {
            let state = override_state
                .filter(|(id, _)| *id == actor.reservation.id)
                .map_or(&actor.infantry_rocket_state, |(_, state)| state);
            for target in state.guided_targets(actor, self.round_number) {
                if self
                    .guided_target(
                        target.world,
                        (u32::from(target.family) << 24) | u32::from(target.index),
                    )
                    .ok()
                    .as_ref()
                    == Some(target)
                {
                    values.insert(target.clone(), 2);
                }
            }
        }
        let mut publications = Vec::new();
        let mut last_queued = BTreeMap::new();
        for actor in self
            .active
            .iter()
            .chain(self.peers.values())
            .filter(|a| a.ready_for_updates && a.initial_objects.is_some())
        {
            let observer = Observer {
                reservation: actor.reservation.id.clone(),
                allocation: actor.allocation_sequence,
            };
            let before = ledger.last_queued.get(&observer);
            let mut after = BTreeMap::new();
            let mut rows = Vec::new();
            let known = |target: &Target| match target.family {
                1 => self.ground_vehicle_creation_complete(actor.ground_vehicle_batch),
                2 => actor.helo_creation_sent,
                13 => actor.emplacement_creation.is_some(),
                _ => false,
            };
            for (target, &value) in &values {
                if target.world == actor.reservation.world && known(target) {
                    if before.and_then(|b| b.get(target)).copied() != Some(value) {
                        rows.push((target.family, target.index, value));
                    }
                    after.insert(target.clone(), value);
                }
            }
            if let Some(before) = before {
                for target in before.keys().filter(|t| !values.contains_key(*t)) {
                    if known(target)
                        && self
                            .guided_target(
                                target.world,
                                (u32::from(target.family) << 24) | u32::from(target.index),
                            )
                            .ok()
                            .as_ref()
                            == Some(target)
                    {
                        rows.push((target.family, target.index, 0));
                    }
                }
            }
            if !rows.is_empty() {
                publications.push((
                    observer.clone(),
                    warning_rows(&rows, 1, now.as_millis() as u32, self.wire_profile)?,
                ));
            }
            last_queued.insert(observer, after);
        }
        ledger.last_queued = last_queued;
        Ok(Candidate {
            ledger,
            publications,
        })
    }
    pub(crate) fn check_guided_warning_capacity(
        &self,
        candidate: &Candidate,
        extra: usize,
    ) -> Result<(), &'static str> {
        for (observer, _) in &candidate.publications {
            let actor = self
                .active
                .iter()
                .chain(self.peers.values())
                .find(|a| {
                    a.reservation.id == observer.reservation
                        && a.allocation_sequence == observer.allocation
                })
                .ok_or("stale_guided_warning_observer")?;
            if actor.ordinary_shared_len().saturating_add(1 + extra) > actor.shared_queue_limit
                || actor.replication_backlogged
            {
                return Err("guided_warning_publication_budget_exceeded");
            }
        }
        Ok(())
    }
    pub(crate) fn commit_guided_warnings(&mut self, candidate: Candidate) {
        self.guided_warnings = candidate.ledger;
        for (observer, packet) in candidate.publications {
            if let Some(actor) = self
                .active
                .iter_mut()
                .chain(self.peers.values_mut())
                .find(|a| {
                    a.reservation.id == observer.reservation
                        && a.allocation_sequence == observer.allocation
                })
            {
                actor.queue_shared(packet);
            }
        }
    }
    pub(crate) fn tick_guided_warnings(
        &mut self,
        now: Duration,
        wall: u64,
    ) -> Result<(), &'static str> {
        if self.guided_launchers.is_none() {
            return Ok(());
        }
        let candidate = self.prepare_guided_warnings(None, None, &[], now)?;
        if self.check_guided_warning_capacity(&candidate, 0).is_err() {
            return Ok(());
        }
        if candidate.ledger == self.guided_warnings {
            return Ok(());
        }
        self.record(serde_json::json!({"op":"guided_lock_warnings","packets":candidate.publications.iter().map(|(o,p)|serde_json::json!({"reservation":o.reservation,"packet":crate::protocol::encode_hex(p)})).collect::<Vec<_>>()}),wall)?;
        self.commit_guided_warnings(candidate);
        Ok(())
    }
    pub(crate) fn plan_guided_lock(
        &self,
        actor: &Active,
        report: LockReport,
        now: Duration,
    ) -> Result<Candidate, &'static str> {
        if self.guided_launchers.is_none() {
            return Err("guided_launcher_rules_required");
        }
        let owner = Owner::capture(actor, self.round_number)
            .ok_or("rocket_lock_requires_current_living_owner")?;
        let target = self.guided_target(actor.reservation.world, report.target)?;
        self.prepare_guided_warnings(Some((owner, target, report.state)), None, &[], now)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn guided_lock_independent_native_ingress_and_field24_goldens() {
        assert_eq!(
            LockReport::decode(&[2, 0x29, 1, 0, 0, 0x51, 1]).unwrap(),
            LockReport {
                target: 0x01000051,
                state: 1
            }
        );
        let profile = crate::wire_profile::WireProfileCatalog::embedded()
            .profile(20231)
            .unwrap();
        for (family, index, expected) in [
            (1, 3, "00080000000700000000000000002000004d2800180000040800"),
            (2, 5, "00080000000700000000000000004000004d2800280000040800"),
            (13, 7, "00080000000700000000000002000000004d28003800000408"),
        ] {
            assert_eq!(
                crate::protocol::encode_hex(
                    &warning_wire(family, index, 2, 7, 1234, profile).unwrap()
                ),
                expected
            );
        }
        for raw in [
            &[2, 0x29, 1, 0, 0, 1, 3][..],
            &[2, 0x29, 1, 0, 0, 1][..],
            &[2, 0x29, 1, 0, 0, 1, 0, 0][..],
        ] {
            assert!(LockReport::decode(raw).is_err());
        }
    }
}

#[cfg(test)]
mod mixed_tests {
    use super::*;
    #[test]
    fn guided_mixed_warning_packet_matches_independent_literal_and_capacity() {
        let p = crate::wire_profile::WireProfileCatalog::embedded()
            .profile(20231)
            .unwrap();
        let bytes = warning_rows(&[(13, 7, 0), (1, 3, 2), (2, 5, 1)], 7, 1234, p).unwrap();
        assert_eq!(
            crate::protocol::encode_hex(&bytes),
            "00080000000700000000000002006000004d28001800000408080028000004044001c000002000"
        );
        let rows = (0..32)
            .map(|i| (1, i, 2))
            .chain((0..8).map(|i| (2, i, 1)))
            .chain((0..16).map(|i| (13, i, 0)))
            .collect::<Vec<_>>();
        assert_eq!(warning_rows(&rows, 7, 1234, p).unwrap().len(), 391);
    }
}
