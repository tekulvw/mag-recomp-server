//! Revisioned saved-armory refreshes do not change the current physical kit.
use crate::*;
use sha2::{Digest, Sha256};

pub const CAPABILITY: &str = "active_loadout_refresh_v1";

/// Only the latest accepted request is retained for exact idempotent retries.
pub(super) struct AppliedRefresh {
    revision: u64,
    requested: Box<loadouts::SavedLoadouts>,
}

impl<W: Write> Engine<W> {
    fn validate_refresh_loadouts(
        &self,
        actor: &Active,
        saved: &loadouts::SavedLoadouts,
    ) -> Result<(), &'static str> {
        if actor.reservation.character.faction > 2 || saved.selected_index >= 5 {
            return Err("invalid_saved_inventory_index");
        }
        self.item_catalog.validate_saved_item_bindings(saved)?;
        if actor.wire_profile.application_id() == 20231 {
            self.item_catalog
                .retail_armory()
                .ok_or("retail_armory_unavailable")?
                .validate_saved(actor.reservation.character.faction, saved)?;
        } else {
            saved.validate(&self.item_catalog)?;
        }
        for row in &saved.inventories {
            if row.iter().any(Option::is_some) {
                let primary = row[0].ok_or("saved_inventory_missing_primary")?;
                if self.item_catalog.primary_magazine_capacity(primary)? == 0 {
                    return Err("unsupported_saved_primary_magazine");
                }
            }
        }
        Ok(())
    }

    pub(super) fn refresh_loadouts(
        &mut self,
        request: Request,
        now: Duration,
        wall: u64,
    ) -> Result<Vec<OutboundMessage>, &'static str> {
        if self.failed {
            return Err("recorder_unavailable");
        }
        let Request::RefreshLoadouts {
            reservation_id,
            world_id,
            player_id,
            account_id,
            character_id,
            revision,
            loadouts,
            ..
        } = request
        else {
            return Err("wrong_loadout_refresh_operation");
        };
        let actor = self
            .active
            .iter()
            .chain(self.peers.values())
            .find(|actor| actor.reservation.id == reservation_id)
            .ok_or("refresh_loadouts_owner_unavailable")?;
        let character = &actor.reservation.character;
        if actor.reservation.world != world_id
            || actor.reservation.player != player_id
            || character
                .loadout_account_id
                .as_ref()
                .or(character.account_id.as_ref())
                != Some(&account_id)
            || character.character_id != Some(character_id)
        {
            return Err("refresh_loadouts_identity_mismatch");
        }
        if now.saturating_sub(actor.last_event) >= IDLE_TIMEOUT {
            return Err("refresh_loadouts_reservation_expired");
        }
        let selected_index = character
            .loadouts
            .as_ref()
            .ok_or("refresh_loadouts_saved_inventory_unavailable")?
            .selected_index;
        let retry = if let Some(previous) = &actor.last_loadout_refresh {
            if revision < previous.revision {
                return Err("refresh_loadouts_stale_revision");
            }
            if revision == previous.revision {
                if loadouts != previous.requested {
                    return Err("refresh_loadouts_revision_conflict");
                }
                true
            } else {
                false
            }
        } else {
            false
        };
        // Validate the complete caller snapshot before normalizing the one field
        // this operation does not own. The preserved preference must remain valid
        // against the replacement rows as well.
        self.validate_refresh_loadouts(actor, &loadouts)?;
        let mut applied = loadouts.clone();
        applied.selected_index = selected_index;
        self.validate_refresh_loadouts(actor, &applied)?;
        let digest = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&loadouts).map_err(|_| "record_encode_failed")?)
        );
        self.record(json!({"op":"loadouts_refreshed","reservation_id":reservation_id,
            "world_id":world_id,"player_id":player_id,"account_id":account_id,"character_id":character_id,
            "revision":revision,"retry":retry,"loadouts_sha256":digest}),wall)?;
        if !retry {
            let actor = self
                .active
                .iter_mut()
                .chain(self.peers.values_mut())
                .find(|actor| actor.reservation.id == reservation_id)
                .ok_or("refresh_loadouts_owner_unavailable")?;
            actor.reservation.character.loadouts = Some(applied);
            actor.last_loadout_refresh = Some(AppliedRefresh {
                revision,
                requested: loadouts,
            });
        }
        Ok(Vec::new())
    }
}
