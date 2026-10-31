//! Native projectile deltas and per-actor presentation cache.
//! Admission, life and owner-sequence authority remain in ProjectileFlights.
//! Deactivation evidence: GRENADE-REMOTE-LIFECYCLE.md.
use crate::projectile::{ProjectileCreation, ProjectileOwnerFields, ProjectileOwnerUpdate};
use crate::replication::{BitWriter, UpdateHeader};

fn header(slot: u8, sequence: u32, timestamp_ms: u32) -> Result<UpdateHeader, &'static str> {
    if slot >= 3 {
        return Err("invalid_projectile_type");
    }
    Ok(UpdateHeader {
        sequence,
        nak_sequence: 0,
        client_index: 0,
        object_type_mask: 1 << (16 + slot),
        timestamp_ms,
    })
}
fn validate_header(slot: u8, actual: UpdateHeader) -> Result<(), &'static str> {
    if actual.object_type_mask
        != header(slot, actual.sequence, actual.timestamp_ms)?.object_type_mask
    {
        return Err("invalid_projectile_type");
    }
    Ok(())
}
/// Already authenticated/accepted owner fields, reserialized for peer recipients.
/// Recipient sequence assignment is performed by the shared replication queue.
pub struct ProjectileOwnerDelta<'a> {
    pub header: UpdateHeader,
    pub update: &'a ProjectileOwnerUpdate,
}
impl ProjectileOwnerDelta<'_> {
    pub fn encode(&self) -> Result<Vec<u8>, &'static str> {
        self.encode_with_profile(crate::wire_profile::WireProfile::prototype())
    }
    pub fn encode_with_profile(
        &self,
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        validate_header(self.update.slot, self.header)?;
        let mut bits = BitWriter::with_profile(profile);
        self.header.encode(&mut bits)?;
        bits.write(1, 1)?;
        bits.write(self.update.object_index.into(), 16)?;
        self.update.fields.encode_fields(&mut bits)?;
        bits.write(0, 1)?;
        Ok(bits.finish())
    }
}
/// The inherited NetObject active field is field0 for projectile types16..18.
#[derive(Clone, Copy, Debug)]
pub struct ProjectileRemoval {
    pub slot: u8,
    pub object_index: u16,
}
impl ProjectileRemoval {
    pub fn encode(&self, sequence: u32, timestamp_ms: u32) -> Result<Vec<u8>, &'static str> {
        self.encode_with_profile(
            sequence,
            timestamp_ms,
            crate::wire_profile::WireProfile::prototype(),
        )
    }
    pub fn encode_with_profile(
        &self,
        sequence: u32,
        timestamp_ms: u32,
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        let mut bits = BitWriter::with_profile(profile);
        header(self.slot, sequence, timestamp_ms)?.encode(&mut bits)?;
        bits.write(1, 1)?;
        bits.write(self.object_index.into(), 16)?;
        for field in 0..8 {
            bits.write(u32::from(field == 0), 1)?;
            if field == 0 {
                bits.write(0, 1)?;
            }
        }
        bits.write(0, 1)?;
        Ok(bits.finish())
    }
}
#[derive(Clone, Copy)]
struct Replica {
    generation: u32,
    creation: ProjectileCreation,
    fields: ProjectileOwnerFields,
}
/// One cache belongs to one Active reservation. A replaced reservation receives
/// a fresh cache; the wire identity stays type16+slot/Character low16 index.
#[derive(Clone, Default)]
pub struct ProjectileReplicas {
    profile: crate::wire_profile::WireProfile,
    slots: [Option<Replica>; 3],
}
impl ProjectileReplicas {
    pub fn with_profile(profile: crate::wire_profile::WireProfile) -> Self {
        Self {
            profile,
            slots: [None; 3],
        }
    }
    pub fn remember_creation(
        &mut self,
        generation: u32,
        creation: ProjectileCreation,
    ) -> Result<(), &'static str> {
        if generation == 0
            || usize::from(creation.object_index) >= crate::MAX_MATCH_PLAYERS
            || creation.owner_index != creation.object_index
            || creation.shooter_id != (0x03800000 | u32::from(creation.object_index))
        {
            return Err("invalid_projectile_replica_identity");
        }
        // Validate the complete native creation before caching it.
        creation.encode_with_profile(header(creation.slot, 1, 0)?, self.profile)?;
        let entry = &mut self.slots[usize::from(creation.slot)];
        if let Some(previous) = entry {
            return Err(if previous.generation >= generation {
                "stale_projectile_replica_generation"
            } else {
                "projectile_replica_slot_in_use"
            });
        }
        *entry = Some(Replica {
            generation,
            creation,
            fields: ProjectileOwnerFields::default(),
        });
        Ok(())
    }
    /// Call only after aggregate body validation and ProjectileFlights acceptance.
    /// A rejected/stale report must never be published or alter late-join state.
    pub fn accept_owner(
        &mut self,
        generation: u32,
        report: ProjectileOwnerUpdate,
    ) -> Result<bool, &'static str> {
        let Some(replica) = self
            .slots
            .get_mut(usize::from(report.slot))
            .ok_or("invalid_projectile_type")?
            .as_mut()
        else {
            return Ok(false);
        };
        if replica.generation != generation {
            return Ok(false);
        }
        if report.object_index != replica.creation.object_index {
            return Err("unowned_projectile_update");
        }
        ProjectileOwnerDelta {
            header: header(report.slot, 1, 0)?,
            update: &report,
        }
        .encode_with_profile(self.profile)?;
        if let Some(state) = report.fields.state {
            replica.fields.state = Some(state)
        }
        if let Some(position) = report.fields.position {
            replica.creation.position = position;
            replica.fields.position = Some(position);
        }
        if let Some(rotation) = report.fields.rotation_xyzw {
            replica.creation.rotation_xyzw = rotation;
            replica.fields.rotation_xyzw = Some(rotation);
        }
        Ok(true)
    }
    /// Late joins receive only live objects, at the last accepted owner transform.
    /// Sequence1 is a queue placeholder and must be rebased for each recipient.
    pub fn snapshot_packets(&self, timestamp: u32) -> Result<Vec<Vec<u8>>, &'static str> {
        let mut packets = Vec::new();
        for replica in self.slots.iter().flatten() {
            let creation = replica.creation;
            packets.push(
                creation.encode_with_profile(header(creation.slot, 1, timestamp)?, self.profile)?,
            );
            let report = ProjectileOwnerUpdate {
                slot: creation.slot,
                object_index: creation.object_index,
                fields: replica.fields,
            };
            packets.push(
                ProjectileOwnerDelta {
                    header: header(creation.slot, 1, timestamp)?,
                    update: &report,
                }
                .encode_with_profile(self.profile)?,
            );
        }
        Ok(packets)
    }
    /// Completion/expiry/reuse removes only the current authoritative flight.
    pub fn take_removal(
        &mut self,
        slot: u8,
        generation: u32,
    ) -> Result<Option<ProjectileRemoval>, &'static str> {
        let entry = self
            .slots
            .get_mut(usize::from(slot))
            .ok_or("invalid_projectile_type")?;
        if !entry.is_some_and(|replica| replica.generation == generation) {
            return Ok(None);
        }
        let replica = entry.take().unwrap();
        Ok(Some(ProjectileRemoval {
            slot,
            object_index: replica.creation.object_index,
        }))
    }
    /// Disconnect, idle expiry and life replacement must queue these before
    /// clearing the actor or admitting a new creation with the same wire index.
    pub fn take_all_removals(&mut self) -> Vec<ProjectileRemoval> {
        self.slots
            .iter_mut()
            .filter_map(Option::take)
            .map(|replica| ProjectileRemoval {
                slot: replica.creation.slot,
                object_index: replica.creation.object_index,
            })
            .collect()
    }
}
