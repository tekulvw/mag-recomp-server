//! Character-owned persisted inventory supplied by authenticated Horizon admission.
use crate::{
    item_catalog::ItemCatalog,
    replication::{CharacterInventory, CharacterLoadout, InventoryHandle, InventoryItem},
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SavedLoadouts {
    pub selected_index: u8,
    pub update_ids: [u8; 5],
    pub inventories: [[Option<u16>; 22]; 5],
}
impl SavedLoadouts {
    pub fn validate(&self, catalog: &ItemCatalog) -> Result<(), &'static str> {
        if self.selected_index >= 5 {
            return Err("invalid_saved_inventory_index");
        }
        for row in &self.inventories {
            for &item in row.iter().flatten() {
                catalog.validate_item(item)?;
            }
            if row.iter().any(Option::is_some) {
                catalog
                    .primary_magazine_capacity(row[0].ok_or("saved_inventory_missing_primary")?)?;
            }
        }
        if self.inventories[usize::from(self.selected_index)][0].is_none() {
            return Err("saved_inventory_missing_primary");
        }
        Ok(())
    }
    pub fn loadout(&self, catalog: &ItemCatalog, index: u8) -> Option<CharacterLoadout> {
        let row = self.inventories.get(usize::from(index))?;
        let ammo_count = catalog.primary_magazine_capacity(row[0]?).ok()?;
        let version = self.update_ids[usize::from(index)];
        Some(CharacterLoadout {
            inventories: std::array::from_fn(|i| CharacterInventory {
                update_id: self.update_ids[i],
                slots: self.inventories[i],
            }),
            handle: InventoryHandle {
                update_id: version,
                inventory_index: index,
            },
            primary: InventoryItem {
                inventory_update_id: version,
                gear_slot: 0,
                ammo_pool: 0,
                ammo_count,
                reloading: false,
            },
            primary_equipped: true,
        })
    }
}
