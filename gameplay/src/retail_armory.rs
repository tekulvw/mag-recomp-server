//! Disk-backed retail equipment IDs for saved kits. Combat authority remains in
//! the separate retail combat catalog; no prototype item lookup is used here.
use crate::{loadouts::SavedLoadouts, replication::*};
use serde::Deserialize;
use std::{collections::BTreeMap, io, path::Path};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    schema_version: u8,
    application_id: u32,
    source_sha256: BTreeMap<String, String>,
    items: Vec<Item>,
    factions: Vec<Faction>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Item {
    wire_id: u16,
    name: String,
    authored_ref: String,
    available_bit: u16,
    factions: Vec<u8>,
    primary_ammo: Option<u8>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Faction {
    faction: u8,
    available_item_bits: Vec<u16>,
    skill_ids: Vec<u16>,
    loadouts: Vec<DefaultKit>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DefaultKit {
    update_id: u8,
    items: [Option<u16>; 22],
}
#[derive(Debug)]
pub struct RetailArmory {
    items: BTreeMap<u16, Item>,
}
fn invalid(reason: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, reason)
}
impl RetailArmory {
    pub fn load(path: impl AsRef<Path>) -> io::Result<Self> {
        Self::from_json(&std::fs::read_to_string(path)?)
    }
    pub fn from_json(json: &str) -> io::Result<Self> {
        let file: File =
            serde_json::from_str(json).map_err(|_| invalid("invalid_retail_armory"))?;
        if file.schema_version != 1
            || file.application_id != 20231
            || file.items.is_empty()
            || file.source_sha256.is_empty()
            || file
                .source_sha256
                .values()
                .any(|hash| hash.len() != 64 || !hash.bytes().all(|c| c.is_ascii_hexdigit()))
        {
            return Err(invalid("invalid_retail_armory_identity"));
        }
        let mut result = Self {
            items: BTreeMap::new(),
        };
        for item in file.items {
            if item.wire_id >= 4096
                || item.available_bit >= 768
                || item.name.is_empty()
                || !item.authored_ref.starts_with("FactionItemList")
                || item.factions.is_empty()
                || item.factions.iter().any(|&f| f > 2)
                || item.primary_ammo == Some(0)
                || result.items.contains_key(&item.wire_id)
            {
                return Err(invalid("invalid_retail_armory_item"));
            }
            result.items.insert(item.wire_id, item);
        }
        let mut seen = [false; 3];
        for faction in file.factions {
            let index = usize::from(faction.faction);
            if index >= 3
                || seen[index]
                || faction.loadouts.len() != 5
                || faction.available_item_bits.iter().any(|&bit| bit >= 768)
                || faction.skill_ids.iter().any(|&bit| bit >= 160)
            {
                return Err(invalid("invalid_retail_armory_faction"));
            }
            seen[index] = true;
            for row in faction.loadouts {
                if row.update_id == 0 {
                    return Err(invalid("invalid_retail_armory_default_version"));
                }
                result
                    .validate_row(faction.faction, &row.items)
                    .map_err(invalid)?;
            }
        }
        if seen != [true; 3] {
            return Err(invalid("missing_retail_armory_factions"));
        }
        Ok(result)
    }
    fn validate_row(&self, faction: u8, row: &[Option<u16>; 22]) -> Result<(), &'static str> {
        for id in row.iter().flatten() {
            let item = self.items.get(id).ok_or("unknown_retail_saved_item")?;
            if !item.factions.contains(&faction) {
                return Err("retail_saved_item_wrong_faction");
            }
        }
        if row.iter().any(Option::is_some) {
            let primary = row[0].ok_or("saved_inventory_missing_primary")?;
            if self
                .items
                .get(&primary)
                .and_then(|item| item.primary_ammo)
                .is_none()
            {
                return Err("retail_saved_primary_not_weapon");
            }
        }
        Ok(())
    }
    pub fn validate_saved(&self, faction: u8, saved: &SavedLoadouts) -> Result<(), &'static str> {
        if faction > 2 || saved.selected_index >= 5 {
            return Err("invalid_saved_inventory_index");
        }
        for row in &saved.inventories {
            self.validate_row(faction, row)?;
        }
        if saved.inventories[0][0].is_none()
            || saved.inventories[usize::from(saved.selected_index)][0].is_none()
        {
            return Err("saved_inventory_missing_primary");
        }
        Ok(())
    }
    pub fn saved_loadout(
        &self,
        faction: u8,
        saved: &SavedLoadouts,
        index: u8,
    ) -> Option<CharacterLoadout> {
        self.validate_saved(faction, saved).ok()?;
        let row = saved.inventories.get(usize::from(index))?;
        let ammo_count = self.items.get(&row[0]?)?.primary_ammo?;
        Some(CharacterLoadout {
            inventories: std::array::from_fn(|i| CharacterInventory {
                update_id: saved.update_ids[i],
                slots: saved.inventories[i],
            }),
            handle: InventoryHandle {
                update_id: saved.update_ids[usize::from(index)],
                inventory_index: index,
            },
            primary: InventoryItem {
                inventory_update_id: saved.update_ids[usize::from(index)],
                gear_slot: 0,
                ammo_pool: 0,
                ammo_count,
                reloading: false,
            },
            primary_equipped: true,
        })
    }
}
