//! Retail type-11 Company object update fields.
use crate::{
    active_ability_state::NativeAbilityData,
    replication::{BitWriter, UpdateHeader},
    strategic_abilities::STRATEGIC_SLOTS,
    wire_profile::WireProfile,
};

#[derive(Clone, Debug, PartialEq)]
pub struct CompanyUpdate {
    pub team: u8,
    pub owner_index: Option<u16>,
    pub active: Option<bool>,
    pub data: [Option<NativeAbilityData>; STRATEGIC_SLOTS],
    pub leader_index: Option<u16>,
    pub active_mask: Option<u16>,
    pub available_mask: Option<u16>,
    pub shadow_modifier_percent: Option<i8>,
}

impl CompanyUpdate {
    pub fn new(team: u8) -> Self {
        Self {
            team,
            owner_index: None,
            active: None,
            data: [None; STRATEGIC_SLOTS],
            leader_index: None,
            active_mask: None,
            available_mask: None,
            shadow_modifier_percent: None,
        }
    }

    pub fn encode_batch(
        updates: &[Self],
        sequence: u32,
        timestamp_ms: u32,
        profile: WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        if updates.is_empty()
            || updates.len() > 2
            || sequence <= 1
            || profile.application_id() != 20231
        {
            return Err("invalid_native_company_update_context");
        }
        let mut bits = BitWriter::with_profile(profile);
        UpdateHeader {
            sequence,
            nak_sequence: 0,
            client_index: 0,
            object_type_mask: 1 << 11,
            timestamp_ms,
        }
        .encode(&mut bits)?;

        let mut teams = 0;
        for update in updates {
            if update.team > 1 || teams & (1 << update.team) != 0 {
                return Err("invalid_native_company_update_team");
            }
            teams |= 1 << update.team;
            bits.write(1, 1)?;
            bits.write(update.team.into(), 16)?;
            optional(&mut bits, update.active.map(u32::from), 1)?;
            optional(&mut bits, update.owner_index.map(u32::from), 16)?;
            for data in update.data {
                bits.write(data.is_some().into(), 1)?;
                if let Some(data) = data {
                    data.encode(&mut bits)?;
                }
            }
            optional(&mut bits, update.leader_index.map(u32::from), 16)?;
            optional(&mut bits, update.active_mask.map(u32::from), 16)?;
            optional(&mut bits, update.available_mask.map(u32::from), 16)?;
            optional(
                &mut bits,
                update
                    .shadow_modifier_percent
                    .map(|value| u32::from(value as u8)),
                8,
            )?;
        }
        bits.write(0, 1)?;
        Ok(bits.finish())
    }
}

fn optional(bits: &mut BitWriter, value: Option<u32>, width: usize) -> Result<(), &'static str> {
    bits.write(value.is_some().into(), 1)?;
    if let Some(value) = value {
        bits.write(value, width)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "strategic_replication_tests.rs"]
mod tests;
