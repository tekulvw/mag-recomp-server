//! One immutable native decode per validated engine dispatch. Keep the original
//! bytes for exact evidence/replay, and cache the complete owner body together.
use crate::{
    owner_update::{OwnerUpdateError, TypedOwnerUpdateFields},
    protocol, replication,
};

pub(crate) struct DecodedEvent<'a> {
    pub(crate) bytes: &'a [u8],
    pub(crate) message: Result<protocol::GameMessage<'a>, &'static str>,
    pub(crate) owner_fields: Option<Result<TypedOwnerUpdateFields, OwnerUpdateError>>,
}
impl<'a> DecodedEvent<'a> {
    #[cfg(test)]
    pub(crate) fn with_profile(
        bytes: &'a [u8],
        player: u16,
        profile: crate::wire_profile::WireProfile,
        allow_ground_owner: bool,
    ) -> Self {
        Self::with_hospital_ownership(bytes, player, profile, allow_ground_owner, false)
    }

    pub(crate) fn with_hospital_ownership(
        bytes: &'a [u8],
        player: u16,
        profile: crate::wire_profile::WireProfile,
        allow_ground_owner: bool,
        allow_hospital_owner: bool,
    ) -> Self {
        let message = protocol::GameMessage::decode_with_profile(bytes, profile);
        let owner_fields = match &message {
            Ok(protocol::GameMessage::ObjectUpdate(update)) => {
                Some(TypedOwnerUpdateFields::decode_checked_with_profile(
                    update,
                    player,
                    profile,
                    allow_ground_owner,
                    allow_hospital_owner,
                ))
            }
            _ => None,
        };
        Self {
            bytes,
            message,
            owner_fields,
        }
    }
    pub(crate) fn owner_update(
        &self,
    ) -> Option<(&replication::ObjectUpdate<'a>, &TypedOwnerUpdateFields)> {
        match (&self.message, &self.owner_fields) {
            (Ok(protocol::GameMessage::ObjectUpdate(update)), Some(Ok(fields))) => {
                Some((update, fields))
            }
            _ => None,
        }
    }
}
