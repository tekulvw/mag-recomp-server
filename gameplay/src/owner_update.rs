//! Typed native owner reports, including interleaved Character3/Alpha13 lists.
//! Parse the complete body before applying any component. Character/projectile
//! indices bind to the authenticated actor; turret fields still need a separate
//! current living, exclusive gunner proof before mutation. Empty selected lists
//! confer no authority and may include registered types not otherwise supported.
use crate::{
    ground_vehicles::OwnerDynamics,
    player_tracking::ClientCharacterFields,
    projectile::{ProjectileOwnerUpdate, projectile_slot_count},
    replication::{BitReader, ObjectUpdate},
    turret::ClientTurretFields,
};

/// Coverage gaps retain the acknowledged session; invalid input rejects it.
/// Legacy component codecs expose stable strings, translated at this boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OwnerUpdateError {
    UnsupportedCharacterField,
    UnsupportedObjectType,
    UnsupportedProjectileField,
    UnsupportedTurretField,
    UnsupportedGroundVehicleField,
    Invalid(&'static str),
}
impl OwnerUpdateError {
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::UnsupportedCharacterField => "unsupported_client_character_field",
            Self::UnsupportedObjectType => "unsupported_client_object_type",
            Self::UnsupportedProjectileField => "unsupported_client_projectile_field",
            Self::UnsupportedTurretField => "unsupported_client_turret_field",
            Self::UnsupportedGroundVehicleField => "unsupported_client_ground_vehicle_field",
            Self::Invalid(code) => code,
        }
    }
}
impl From<&'static str> for OwnerUpdateError {
    fn from(code: &'static str) -> Self {
        match code {
            "unsupported_client_character_field" => Self::UnsupportedCharacterField,
            "unsupported_client_object_type" => Self::UnsupportedObjectType,
            "unsupported_client_projectile_field" => Self::UnsupportedProjectileField,
            "unsupported_client_turret_field" => Self::UnsupportedTurretField,
            "unsupported_client_ground_vehicle_field" => Self::UnsupportedGroundVehicleField,
            code => Self::Invalid(code),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct TypedOwnerUpdateFields {
    pub character: Option<ClientCharacterFields>,
    pub ground_vehicles: Vec<OwnerDynamics>,
    pub(crate) ground_turrets: Vec<crate::ground_vehicles::OwnerUpdate>,
    pub turret: Option<ClientTurretFields>,
    pub(crate) hospital_turrets: Vec<crate::hospital_turret_wire::ClientUpdate>,
    pub projectiles: Vec<ProjectileOwnerUpdate>,
}
impl TypedOwnerUpdateFields {
    pub(crate) fn decode_checked_with_profile(
        update: &ObjectUpdate<'_>,
        expected_character_index: u16,
        profile: crate::wire_profile::WireProfile,
        allow_ground_owner: bool,
        allow_hospital_owner: bool,
        allow_ground_turret_owner: bool,
    ) -> Result<Self, OwnerUpdateError> {
        Self::decode_with_ownership(
            update,
            expected_character_index,
            profile,
            allow_ground_owner,
            allow_hospital_owner,
            allow_ground_turret_owner,
        )
        .map_err(OwnerUpdateError::from)
    }

    pub fn decode(
        update: &ObjectUpdate<'_>,
        expected_character_index: u16,
    ) -> Result<Self, &'static str> {
        Self::decode_with_profile(
            update,
            expected_character_index,
            crate::wire_profile::WireProfile::prototype(),
        )
    }
    pub fn decode_with_profile(
        update: &ObjectUpdate<'_>,
        expected_character_index: u16,
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Self, &'static str> {
        Self::decode_with_ownership(
            update,
            expected_character_index,
            profile,
            false,
            false,
            false,
        )
    }

    fn decode_with_ownership(
        update: &ObjectUpdate<'_>,
        expected_character_index: u16,
        profile: crate::wire_profile::WireProfile,
        allow_ground_owner: bool,
        allow_hospital_owner: bool,
        allow_ground_turret_owner: bool,
    ) -> Result<Self, &'static str> {
        update.validate_profile(profile)?;
        let mut bits = BitReader::with_profile(update.wire, profile);
        bits.position = profile.header_bits();
        let mut fields = Self::default();
        // Lists use ascending type order within the selected build's type mask.
        // An unknown type is safe only when its list explicitly contains no object.
        for object_type in 0..profile.object_type_bits() {
            if update.header.object_type_mask & (1 << object_type) == 0 {
                continue;
            }
            if object_type == 1 && allow_ground_owner && profile.application_id() == 20231 {
                if allow_ground_turret_owner {
                    let rows = crate::ground_vehicles::decode_owner_updates(&mut bits)?;
                    fields.ground_vehicles = rows.iter().map(|row| row.dynamics).collect();
                    fields.ground_turrets = rows
                        .into_iter()
                        .filter(|row| row.turret.has_changes())
                        .collect();
                } else {
                    fields.ground_vehicles = OwnerDynamics::decode_list(&mut bits)?;
                }
                continue;
            }
            if object_type == 13 {
                if allow_hospital_owner {
                    fields.hospital_turrets =
                        crate::hospital_turret_wire::decode_objects(&mut bits)?;
                } else {
                    fields.turret = ClientTurretFields::decode_object(&mut bits)?;
                }
                continue;
            }
            if bits.read(1)? == 0 {
                continue;
            }
            match object_type {
                3 => {
                    fields.character = Some(ClientCharacterFields::decode_present_object(
                        &mut bits,
                        expected_character_index,
                    )?)
                }
                16..=19 if object_type - 16 < usize::from(projectile_slot_count(profile)) => {
                    if bits.read(16)? != u32::from(expected_character_index) {
                        return Err("unowned_projectile_update");
                    }
                    fields.projectiles.push(ProjectileOwnerUpdate {
                        slot: (object_type - 16) as u8,
                        object_index: expected_character_index,
                        fields: crate::projectile::decode_owner_fields(&mut bits)?,
                    });
                    if bits.read(1)? != 0 {
                        return Err("multiple_client_projectile_updates");
                    }
                }
                _ => return Err("unsupported_client_object_type"),
            }
        }
        let remaining = update.wire.len() * 8 - bits.position;
        if remaining >= 8 || bits.read(remaining)? != 0 {
            return Err("trailing_client_update_bits");
        }
        Ok(fields)
    }
}

#[cfg(test)]
mod projectile_family_tests {
    use super::*;
    use crate::{
        projectile::ProjectileOwnerFields,
        replication::{BitWriter, UpdateHeader},
        wire_profile::{WireProfile, WireProfileCatalog},
    };

    #[test]
    fn retail_fourth_projectile_owner_report_uses_the_common_fields() {
        let retail = WireProfileCatalog::embedded().profile(20231).unwrap();
        let header = UpdateHeader {
            sequence: 1,
            nak_sequence: 0,
            client_index: 0,
            object_type_mask: (1 << 16) | (1 << 19),
            timestamp_ms: 41,
        };
        let mut bits = BitWriter::with_profile(retail);
        header.encode(&mut bits).unwrap();
        for state in [1, 2] {
            bits.write(1, 1).unwrap();
            bits.write(7, 16).unwrap();
            ProjectileOwnerFields {
                state: Some(state),
                ..Default::default()
            }
            .encode_fields(&mut bits)
            .unwrap();
            bits.write(0, 1).unwrap();
        }
        let wire = bits.finish();
        let update = ObjectUpdate::decode_with_profile(&wire, retail).unwrap();
        assert!(matches!(
            TypedOwnerUpdateFields::decode_with_profile(&update, 8, retail),
            Err("unowned_projectile_update")
        ));
        let fields = TypedOwnerUpdateFields::decode_with_profile(&update, 7, retail).unwrap();
        assert_eq!(
            fields
                .projectiles
                .iter()
                .map(|row| (row.slot, row.object_index, row.fields.state))
                .collect::<Vec<_>>(),
            [(0, 7, Some(1)), (3, 7, Some(2))]
        );

        let legacy =
            ClientCharacterFields::decode_for_owner_with_profile(&update, 7, retail).unwrap();
        assert_eq!(legacy.projectiles.len(), 2);
        assert_eq!(legacy.projectiles[1].slot, 3);
        assert_eq!(legacy.projectiles[1].fields.state, Some(2));

        let prototype = WireProfile::prototype();
        assert!(
            header
                .encode(&mut BitWriter::with_profile(prototype))
                .is_err()
        );
        assert!(TypedOwnerUpdateFields::decode_with_profile(&update, 7, prototype).is_err());
    }

    #[test]
    fn retail_fourth_projectile_report_cannot_create_an_unadmitted_flight() {
        use crate::{
            spawn_progression::tests::dispatch,
            spawn_progression::tests::ground_vehicle_runtime::domination_ground_driver::{
                driver_engine, ready_on_foot,
            },
        };

        let mut engine = driver_engine(8);
        ready_on_foot(&mut engine, 0, 100);
        let profile = engine.wire_profile;
        assert_eq!(profile.application_id(), 20231);
        let actor = engine.active.as_ref().unwrap();
        let sequence = actor.latest_owner_sequence + 1;
        let mut bits = BitWriter::with_profile(profile);
        UpdateHeader {
            sequence,
            nak_sequence: 0,
            client_index: 0,
            object_type_mask: 1 << 19,
            timestamp_ms: 110,
        }
        .encode(&mut bits)
        .unwrap();
        bits.write(1, 1).unwrap();
        bits.write(u32::from(actor.reservation.player), 16).unwrap();
        ProjectileOwnerFields {
            state: Some(2),
            ..Default::default()
        }
        .encode_fields(&mut bits)
        .unwrap();
        bits.write(0, 1).unwrap();

        let rewards = engine.pending_reward_batch().len();
        dispatch(&mut engine, &bits.finish(), 110).unwrap();
        let actor = engine.active.as_ref().unwrap();
        assert_eq!(actor.latest_owner_sequence, sequence);
        assert!(actor.projectiles.get(3).is_none());
        assert!(actor.projectile_cleanup.iter().all(Option::is_none));
        assert!(
            actor
                .projectile_replicas
                .snapshot_packets(110)
                .unwrap()
                .is_empty()
        );
        assert_eq!(engine.pending_reward_batch().len(), rewards);
    }
}
