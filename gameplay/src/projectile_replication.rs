//! Native projectile deltas and per-actor presentation cache.
//! Admission, life and owner-sequence authority remain in ProjectileFlights.
//! Deactivation evidence: GRENADE-REMOTE-LIFECYCLE.md.
use crate::projectile::{
    MAX_PROJECTILE_SLOTS, ProjectileCreation, ProjectileOwnerFields, ProjectileOwnerUpdate,
    projectile_slot_count,
};
use crate::replication::{BitWriter, UpdateHeader};
use crate::wire_profile::WireProfile;

fn header(
    slot: u8,
    sequence: u32,
    timestamp_ms: u32,
    profile: WireProfile,
) -> Result<UpdateHeader, &'static str> {
    if slot >= projectile_slot_count(profile) {
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
fn validate_header(
    slot: u8,
    actual: UpdateHeader,
    profile: WireProfile,
) -> Result<(), &'static str> {
    if actual.object_type_mask
        != header(slot, actual.sequence, actual.timestamp_ms, profile)?.object_type_mask
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
        validate_header(self.update.slot, self.header, profile)?;
        let mut bits = BitWriter::with_profile(profile);
        self.header.encode(&mut bits)?;
        bits.write(1, 1)?;
        bits.write(self.update.object_index.into(), 16)?;
        self.update.fields.encode_fields(&mut bits)?;
        bits.write(0, 1)?;
        Ok(bits.finish())
    }
}
/// The inherited NetObject active field is field 0 in each projectile family.
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
        header(self.slot, sequence, timestamp_ms, profile)?.encode(&mut bits)?;
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
    slots: [Option<Replica>; MAX_PROJECTILE_SLOTS],
}
impl ProjectileReplicas {
    pub fn with_profile(profile: crate::wire_profile::WireProfile) -> Self {
        Self {
            profile,
            slots: [None; MAX_PROJECTILE_SLOTS],
        }
    }
    fn slot_mut(&mut self, slot: u8) -> Result<&mut Option<Replica>, &'static str> {
        self.slots[..usize::from(projectile_slot_count(self.profile))]
            .get_mut(usize::from(slot))
            .ok_or("invalid_projectile_type")
    }

    pub fn remember_creation(
        &mut self,
        generation: u32,
        creation: ProjectileCreation,
    ) -> Result<(), &'static str> {
        if generation == 0
            || usize::from(creation.object_index) >= crate::world_capacity::MAX_WORLD_CHARACTERS
            || creation.owner_index != creation.object_index
            || creation.shooter_id != (0x03800000 | u32::from(creation.object_index))
        {
            return Err("invalid_projectile_replica_identity");
        }
        // Validate the complete native creation before caching it.
        creation.encode_with_profile(header(creation.slot, 1, 0, self.profile)?, self.profile)?;
        let entry = self.slot_mut(creation.slot)?;
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

    pub(crate) fn creation(&self, slot: u8, generation: u32) -> Option<ProjectileCreation> {
        self.slots
            .get(usize::from(slot))?
            .as_ref()
            .filter(|replica| replica.generation == generation)
            .map(|replica| replica.creation)
    }
    /// Call only after aggregate body validation and ProjectileFlights acceptance.
    /// A rejected/stale report must never be published or alter late-join state.
    pub fn accept_owner(
        &mut self,
        generation: u32,
        report: ProjectileOwnerUpdate,
    ) -> Result<bool, &'static str> {
        let profile = self.profile;
        let Some(replica) = self.slot_mut(report.slot)?.as_mut() else {
            return Ok(false);
        };
        if replica.generation != generation {
            return Ok(false);
        }
        if report.object_index != replica.creation.object_index {
            return Err("unowned_projectile_update");
        }
        ProjectileOwnerDelta {
            header: header(report.slot, 1, 0, profile)?,
            update: &report,
        }
        .encode_with_profile(profile)?;
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
            packets.push(creation.encode_with_profile(
                header(creation.slot, 1, timestamp, self.profile)?,
                self.profile,
            )?);
            let report = ProjectileOwnerUpdate {
                slot: creation.slot,
                object_index: creation.object_index,
                fields: replica.fields,
            };
            packets.push(
                ProjectileOwnerDelta {
                    header: header(creation.slot, 1, timestamp, self.profile)?,
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
        let entry = self.slot_mut(slot)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        projectile::decode_projectile_updates_with_profile,
        replication::{BitReader, CharacterPosition, ObjectUpdate},
        wire_profile::{WireProfile, WireProfileCatalog},
    };

    #[test]
    fn retail_fourth_projectile_creation_delta_snapshot_and_removal_are_profile_scoped() {
        let profile = WireProfileCatalog::embedded().profile(20231).unwrap();
        let prototype = WireProfile::prototype();
        let mut replicas = ProjectileReplicas::with_profile(profile);
        let mut prototype_replicas = ProjectileReplicas::with_profile(prototype);
        let creation = ProjectileCreation {
            slot: 3,
            object_index: 7,
            owner_index: 7,
            ammo_token: 0x0012_3456,
            position: CharacterPosition {
                x: 16.,
                y: -2.,
                z: 24.,
            },
            rotation_xyzw: [0., 0., 0., 1.],
            shooter_id: 0x0380_0007,
        };
        let header = UpdateHeader {
            sequence: 1,
            nak_sequence: 0,
            client_index: 0,
            object_type_mask: 1 << 19,
            timestamp_ms: 10,
        };
        assert_eq!(creation.encode(header), Err("invalid_projectile_type"));
        assert_eq!(
            prototype_replicas.remember_creation(1, creation),
            Err("invalid_projectile_type")
        );
        replicas.remember_creation(1, creation).unwrap();
        let initial = creation.encode_with_profile(header, profile).unwrap();
        assert_eq!(
            ObjectUpdate::decode_with_profile(&initial, profile)
                .unwrap()
                .header,
            header
        );
        assert!(ObjectUpdate::decode_with_profile(&initial, prototype).is_err());

        let position = CharacterPosition {
            x: 13.,
            y: -1.5,
            z: 20.,
        };
        let report = ProjectileOwnerUpdate {
            slot: 3,
            object_index: 7,
            fields: ProjectileOwnerFields {
                state: Some(2),
                position: Some(position),
                rotation_xyzw: Some([0., 0., 0., 1.]),
            },
        };
        assert_eq!(
            ProjectileOwnerDelta {
                header,
                update: &report
            }
            .encode(),
            Err("invalid_projectile_type")
        );
        assert_eq!(
            prototype_replicas.accept_owner(1, report),
            Err("invalid_projectile_type")
        );
        assert!(!replicas.accept_owner(2, report).unwrap());
        assert!(replicas.accept_owner(1, report).unwrap());

        let packets = replicas.snapshot_packets(25).unwrap();
        assert_eq!(packets.len(), 2);
        for wire in &packets {
            let update = ObjectUpdate::decode_with_profile(wire, profile).unwrap();
            assert_eq!(update.header.object_type_mask, 1 << 19);
            assert_eq!(update.header.timestamp_ms, 25);
        }
        let update = ObjectUpdate::decode_with_profile(&packets[1], profile).unwrap();
        let decoded = decode_projectile_updates_with_profile(&update, profile).unwrap();
        assert_eq!(decoded.len(), 1);
        assert_eq!(decoded[0].slot, 3);
        assert_eq!(decoded[0].object_index, 7);
        assert_eq!(decoded[0].fields.state, Some(2));
        assert_eq!(decoded[0].fields.position, Some(position));
        assert_eq!(decoded[0].fields.rotation_xyzw, Some([0., 0., 0., 1.]));

        assert!(replicas.take_removal(3, 2).unwrap().is_none());
        assert!(matches!(
            prototype_replicas.take_removal(3, 1),
            Err("invalid_projectile_type")
        ));
        let removal = replicas.take_removal(3, 1).unwrap().unwrap();
        assert_eq!(removal.encode(2, 26), Err("invalid_projectile_type"));
        let wire = removal.encode_with_profile(2, 26, profile).unwrap();
        let update = ObjectUpdate::decode_with_profile(&wire, profile).unwrap();
        assert_eq!(update.header.object_type_mask, 1 << 19);
        let mut bits = BitReader::with_profile(&wire, profile);
        bits.position = profile.header_bits();
        assert_eq!(bits.read(1), Ok(1));
        assert_eq!(bits.read(16), Ok(7));
        assert_eq!(bits.read(1), Ok(1));
        assert_eq!(bits.read(1), Ok(0));
        assert_eq!(bits.read(8), Ok(0)); // Seven absent fields, then end of list.
        assert!(replicas.snapshot_packets(27).unwrap().is_empty());
    }
}
