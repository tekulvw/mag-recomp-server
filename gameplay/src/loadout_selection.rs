//! Committed desired-kit persistence work, separate from native kit equipping.
use crate::*;
use sha2::{Digest, Sha256};

pub const CAPABILITY: &str = "committed_loadout_selection_v1";
pub const MAX_BATCH: usize = 32;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CommittedLoadoutSelection {
    pub selection_id: String,
    pub reservation_id: String,
    pub world_id: u32,
    pub player_id: u16,
    pub account_id: String,
    pub character_id: u32,
    pub revision: u64,
    pub inventory_index: u8,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SelectionStatus {
    Persisted,
    Rejected,
}
pub(crate) fn deserialize_batch<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<CommittedLoadoutSelection>, D::Error> {
    let batch = Vec::<CommittedLoadoutSelection>::deserialize(deserializer)?;
    if batch.len() > MAX_BATCH {
        return Err(serde::de::Error::custom(
            "loadout selection batch exceeds bound",
        ));
    }
    Ok(batch)
}
impl<W: Write> Engine<W> {
    pub(super) fn plan_loadout_selection(
        &self,
        actor: &Active,
        request: protocol::SelectLoadout,
    ) -> Result<Option<CommittedLoadoutSelection>, &'static str> {
        let character = &actor.reservation.character;
        let Some(account) = character
            .loadout_account_id
            .as_ref()
            .or(character.account_id.as_ref())
            .filter(|account| !account.is_empty())
        else {
            return Ok(None);
        };
        let Some(character_id) = character.character_id.filter(|id| *id != 0) else {
            return Ok(None);
        };
        let Some(saved) = character.loadouts.as_ref() else {
            return Ok(None);
        };
        let index = request.desired_inventory_index();
        let Some(row) = saved.inventories.get(usize::from(index)) else {
            return Ok(None);
        };
        let Some(primary) = row[0] else {
            return Ok(None);
        };
        if character.faction > 2
            || self
                .item_catalog
                .primary_magazine_capacity(primary)
                .is_err()
            || self
                .item_catalog
                .validate_saved_item_bindings(saved)
                .is_err()
        {
            return Ok(None);
        }
        let valid = if actor.wire_profile.application_id() == 20231 {
            self.item_catalog
                .retail_armory()
                .is_some_and(|armory| armory.validate_saved(character.faction, saved).is_ok())
        } else {
            saved.validate(&self.item_catalog).is_ok()
        };
        if !valid {
            return Ok(None);
        }
        let revision = actor
            .loadout_selection_revision
            .checked_add(1)
            .ok_or("loadout_selection_revision_exhausted")?;
        let selection_id = format!(
            "{:x}",
            Sha256::digest(
                format!(
                    "MAG-loadout-selection-v1\0{}\0{}\0{}\0{}\0{}\0{}\0{revision}\0{index}",
                    actor.reservation.id,
                    actor.reservation.world,
                    actor.reservation.player,
                    actor.reservation.account,
                    account,
                    character_id
                )
                .as_bytes()
            )
        );
        Ok(Some(CommittedLoadoutSelection {
            selection_id,
            reservation_id: actor.reservation.id.clone(),
            world_id: actor.reservation.world,
            player_id: actor.reservation.player,
            account_id: account.clone(),
            character_id,
            revision,
            inventory_index: index,
        }))
    }
    pub(super) fn pending_loadout_selections(&self) -> Vec<CommittedLoadoutSelection> {
        self.active
            .iter()
            .chain(self.peers.values())
            .filter_map(|actor| actor.pending_loadout_selection.clone())
            .chain(self.retired_loadout_selections.values().cloned())
            .take(MAX_BATCH)
            .collect()
    }
    pub(super) fn settle_loadout_selection(
        &mut self,
        request: Request,
        wall: u64,
    ) -> Result<Vec<OutboundMessage>, &'static str> {
        if self.failed {
            return Err("recorder_unavailable");
        }
        let Request::LoadoutSelectionSettled {
            reservation_id,
            world_id,
            player_id,
            selection_id,
            revision,
            status,
            ..
        } = request
        else {
            return Err("wrong_loadout_selection_operation");
        };
        let matched = self
            .active
            .iter()
            .chain(self.peers.values())
            .find(|actor| {
                actor.reservation.id == reservation_id
                    && actor.reservation.world == world_id
                    && actor.reservation.player == player_id
            })
            .and_then(|actor| actor.pending_loadout_selection.as_ref())
            .is_some_and(|receipt| {
                receipt.selection_id == selection_id && receipt.revision == revision
            });
        let retired_matched = self
            .retired_loadout_selections
            .get(&selection_id)
            .is_some_and(|receipt| {
                receipt.reservation_id == reservation_id
                    && receipt.world_id == world_id
                    && receipt.player_id == player_id
                    && receipt.revision == revision
            });
        self.record(json!({"op":"loadout_selection_settled","reservation_id":reservation_id,
            "world_id":world_id,"player_id":player_id,"selection_id":selection_id,"revision":revision,
            "status":status,"matched":matched || retired_matched}),wall)?;
        if retired_matched {
            self.retired_loadout_selections.remove(&selection_id);
        }
        if matched {
            let actor = self
                .active
                .iter_mut()
                .chain(self.peers.values_mut())
                .find(|actor| actor.reservation.id == reservation_id)
                .ok_or("stale_loadout_selection_owner")?;
            actor.pending_loadout_selection = None;
        }
        Ok(Vec::new())
    }
}
