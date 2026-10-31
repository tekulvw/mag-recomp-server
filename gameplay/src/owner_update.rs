//! Typed native owner reports, including interleaved Character3/Alpha13 lists.
//! Parse the complete body before applying any component. Character/projectile
//! indices bind to the authenticated actor; turret fields still need a separate
//! current living, exclusive gunner proof before mutation. Empty selected lists
//! confer no authority and may include registered types not otherwise supported.
use crate::{
    player_tracking::ClientCharacterFields,
    projectile::ProjectileOwnerUpdate,
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
    Invalid(&'static str),
}
impl OwnerUpdateError {
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::UnsupportedCharacterField => "unsupported_client_character_field",
            Self::UnsupportedObjectType => "unsupported_client_object_type",
            Self::UnsupportedProjectileField => "unsupported_client_projectile_field",
            Self::UnsupportedTurretField => "unsupported_client_turret_field",
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
            code => Self::Invalid(code),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct TypedOwnerUpdateFields {
    pub character: Option<ClientCharacterFields>,
    pub turret: Option<ClientTurretFields>,
    pub projectiles: Vec<ProjectileOwnerUpdate>,
}
impl TypedOwnerUpdateFields {
    pub(crate) fn decode_checked_with_profile(
        update: &ObjectUpdate<'_>,
        expected_character_index: u16,
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Self, OwnerUpdateError> {
        Self::decode_with_profile(update, expected_character_index, profile)
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
            if object_type == 13 {
                fields.turret = ClientTurretFields::decode_object(&mut bits)?;
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
                16..=18 => {
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
