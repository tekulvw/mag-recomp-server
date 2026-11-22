//! Retained retail smoke projectile and volume snapshots.
use std::time::Duration;

use crate::{
    projectile::{
        ProjectileCreation, ProjectileOwnerFields, ProjectileOwnerUpdate, projectile_slot_count,
    },
    projectile_replication::ProjectileOwnerDelta,
    replication::UpdateHeader,
    smoke_volume_replication::{VolumeCreation, VolumeRemoval},
    wire_profile::WireProfile,
};

#[derive(Clone, Debug)]
pub(crate) struct Volume {
    pub object_index: u16,
    pub generation: u32,
    pub creation: ProjectileCreation,
    pub expires: Duration,
    pub poison: Option<Poison>,
}

#[derive(Clone, Debug)]
pub(crate) struct Poison {
    pub projectile_id: u32,
    pub source: crate::active_ability_gas_world::GasSource,
    pub round: u64,
    pub timing: crate::active_ability_gas_world::PulseTiming,
    pub falloff: crate::active_ability_effects::BlastFalloff,
    pub raw_hp: f32,
}
impl Volume {
    fn expired(&self, now: Duration) -> bool {
        if self.poison.is_some() {
            now > self.expires
        } else {
            now >= self.expires
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct State {
    volumes: Vec<Volume>,
}

impl State {
    pub(crate) fn volumes(&self) -> &[Volume] {
        &self.volumes
    }

    pub(crate) fn push(&mut self, volume: Volume) {
        debug_assert!(volume.generation > 0);
        debug_assert!(self.volumes.iter().all(|current| {
            current.object_index != volume.object_index
                && current.creation.slot != volume.creation.slot
        }));
        self.volumes.push(volume);
    }

    pub(crate) fn advance_poison(
        &mut self,
        now: Duration,
        round: u64,
    ) -> Vec<crate::active_ability_gas_world::GasPulse> {
        self.volumes
            .iter_mut()
            .filter_map(|volume| {
                use crate::active_ability_gas_world::{GasPulse, Pulse};
                let poison = volume.poison.as_mut()?;
                (poison.round == round && poison.timing.advance(now) == Pulse::Damage).then(|| {
                    let p = volume.creation.position;
                    GasPulse {
                        projectile_id: Some(poison.projectile_id),
                        origin: [p.x, p.y, p.z],
                        falloff: poison.falloff,
                        raw_hp: poison.raw_hp,
                        source: poison.source.clone(),
                        operation: "gas_grenade_pulse",
                    }
                })
            })
            .collect()
    }

    pub(crate) fn take_due(&mut self, now: Duration) -> Vec<Volume> {
        self.volumes
            .extract_if(.., |volume| volume.expired(now))
            .collect()
    }

    pub(crate) fn snapshot_packets(
        &self,
        now_ms: u32,
        now: Duration,
        profile: WireProfile,
    ) -> Result<Vec<Vec<u8>>, &'static str> {
        let mut packets = Vec::new();
        for volume in self.volumes.iter().filter(|volume| !volume.expired(now)) {
            let creation = &volume.creation;
            if creation.slot >= projectile_slot_count(profile) {
                return Err("invalid_projectile_type");
            }
            let header = UpdateHeader {
                sequence: 1,
                nak_sequence: 0,
                client_index: 0,
                object_type_mask: 1 << (16 + creation.slot),
                timestamp_ms: now_ms,
            };
            packets.push(creation.encode_with_profile(header, profile)?);
            // A newly created remote projectile starts its local smoke effect
            // when it consumes state 2. The native row carries no effect age.
            packets.push(
                ProjectileOwnerDelta {
                    header,
                    update: &ProjectileOwnerUpdate {
                        slot: creation.slot,
                        object_index: creation.object_index,
                        fields: ProjectileOwnerFields {
                            state: Some(2),
                            ..Default::default()
                        },
                    },
                }
                .encode_with_profile(profile)?,
            );
            packets.push(
                VolumeCreation {
                    object_index: volume.object_index,
                    position: creation.position,
                }
                .encode_with_profile(1, now_ms, profile)?,
            );
        }
        Ok(packets)
    }

    pub(crate) fn removal_packets(
        &self,
        now_ms: u32,
        profile: WireProfile,
    ) -> Result<Vec<Vec<u8>>, &'static str> {
        self.volumes
            .iter()
            .map(|volume| {
                VolumeRemoval {
                    object_index: volume.object_index,
                }
                .encode_with_profile(1, now_ms, profile)
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
        smoke_volume_replication::is_removal,
        wire_profile::WireProfileCatalog,
    };

    fn volume(index: u16, slot: u8, expires_ms: u64) -> Volume {
        Volume {
            object_index: index,
            generation: 20 + u32::from(index),
            creation: ProjectileCreation {
                slot,
                object_index: 17,
                owner_index: 17,
                ammo_token: 0xA1B2_C3D4,
                position: CharacterPosition {
                    x: 12.5,
                    y: -6.25,
                    z: 8.75,
                },
                rotation_xyzw: [0., 0., 0., 1.],
                shooter_id: 0x0380_0011,
            },
            expires: Duration::from_millis(expires_ms),
            poison: None,
        }
    }

    fn retail() -> WireProfile {
        WireProfileCatalog::embedded().profile(20231).unwrap()
    }

    #[test]
    fn simultaneous_expiry_drains_only_due_volumes_and_never_repeats() {
        let mut state = State::default();
        state.push(volume(6, 3, 100));
        state.push(volume(7, 1, 200));
        state.push(volume(8, 2, 100));

        assert!(state.take_due(Duration::from_millis(99)).is_empty());
        let due = state.take_due(Duration::from_millis(100));
        assert_eq!(
            due.iter()
                .map(|volume| (volume.object_index, volume.generation))
                .collect::<Vec<_>>(),
            [(6, 26), (8, 28)]
        );
        assert_eq!(state.volumes().len(), 1);
        assert_eq!(state.volumes()[0].object_index, 7);
        assert!(state.take_due(Duration::from_millis(100)).is_empty());
        assert_eq!(
            state.take_due(Duration::from_millis(200))[0].object_index,
            7
        );
        assert!(state.volumes().is_empty());
    }

    #[test]
    fn late_join_gets_terminal_parent_then_state_two_then_active_volume() {
        let mut state = State::default();
        state.push(volume(6, 3, 100));
        state.push(volume(7, 1, 200));
        let profile = retail();
        let packets = state
            .snapshot_packets(101, Duration::from_millis(100), profile)
            .unwrap();
        assert_eq!(packets.len(), 3);
        let updates = packets
            .iter()
            .map(|packet| ObjectUpdate::decode_with_profile(packet, profile).unwrap())
            .collect::<Vec<_>>();
        let headers = updates
            .iter()
            .map(|update| (update.header.object_type_mask, update.header.timestamp_ms))
            .collect::<Vec<_>>();
        assert_eq!(headers, [(1 << 17, 101), (1 << 17, 101), (1 << 5, 101)]);
        let mut creation = BitReader::with_profile(&packets[0], profile);
        creation.position = profile.header_bits();
        assert_eq!(creation.read(1).unwrap(), 1);
        assert_eq!(creation.read(16).unwrap(), 17);
        assert_eq!(creation.read(2).unwrap(), 0b11);
        assert_eq!(creation.read(1).unwrap(), 1);
        assert_eq!(creation.read(16).unwrap(), 17);
        assert_eq!(creation.read(2).unwrap(), 0);
        assert_eq!(creation.read(1).unwrap(), 1);
        assert_eq!(creation.read(32).unwrap(), 0xA1B2_C3D4);
        assert_eq!(creation.read(1).unwrap(), 1);
        assert_eq!(creation.read(16).unwrap(), 400);
        assert_eq!(creation.read(1).unwrap(), 1);
        assert_eq!(creation.read(16).unwrap(), 200);
        assert_eq!(creation.read(16).unwrap(), 280);
        let terminal = decode_projectile_updates_with_profile(&updates[1], profile).unwrap();
        assert_eq!(terminal.len(), 1);
        assert_eq!(
            (
                terminal[0].slot,
                terminal[0].object_index,
                terminal[0].fields.state
            ),
            (1, 17, Some(2))
        );
        let mut volume = BitReader::with_profile(&packets[2], profile);
        volume.position = profile.header_bits();
        assert_eq!(volume.read(1).unwrap(), 1);
        assert_eq!(volume.read(16).unwrap(), 7);
        assert_eq!(volume.read(2).unwrap(), 0b11);
        assert_eq!(volume.read(1).unwrap(), 1);
        assert_eq!(volume.read(16).unwrap(), 0x0105);
        assert_eq!(volume.read(1).unwrap(), 1);
        assert_eq!(volume.read(16).unwrap(), 400);
        assert_eq!(volume.read(1).unwrap(), 1);
        assert_eq!(volume.read(16).unwrap(), 200);
        assert_eq!(volume.read(16).unwrap(), 280);
    }

    #[test]
    fn teardown_emits_only_volume_removals_in_retained_order() {
        let mut state = State::default();
        state.push(volume(6, 3, 100));
        state.push(volume(7, 1, 200));
        let profile = retail();
        let packets = state.removal_packets(250, profile).unwrap();
        assert_eq!(
            packets
                .iter()
                .map(|packet| is_removal(packet, profile))
                .collect::<Vec<_>>(),
            [Some(6), Some(7)]
        );
        assert_eq!(state.volumes().len(), 2);
    }
}
