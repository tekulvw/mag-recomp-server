//! Optional retail bootstrap equipment. These IDs are never combat authority.

use crate::replication::{CharacterInventory, CharacterLoadout, InventoryHandle, InventoryItem};
use serde::Deserialize;
use serde_json::Value;
use std::{collections::BTreeMap, io, path::Path};

#[derive(Debug)]
pub struct BootstrapLoadouts {
    factions: BTreeMap<u8, Faction>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    schema_version: u8,
    application_id: u32,
    provenance: Value,
    factions: Vec<Faction>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Faction {
    faction: u8,
    class_ref: String,
    loadouts: [Loadout; 5],
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Loadout {
    update_id: u8,
    items: [Option<u16>; 22],
    primary_ammo: u8,
}

impl BootstrapLoadouts {
    pub fn load(path: impl AsRef<Path>) -> io::Result<Self> {
        Self::from_json(&std::fs::read_to_string(path)?)
    }
    pub fn from_json(json: &str) -> io::Result<Self> {
        let invalid = || {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid_retail_bootstrap_loadouts",
            )
        };
        let file: File = serde_json::from_str(json).map_err(|_| invalid())?;
        if file.schema_version != 1
            || file.application_id != 20231
            || file.factions.is_empty()
            || file.factions.len() > 3
        {
            return Err(invalid());
        }
        let proof = &file.provenance["verified_binding"];
        if proof["validated"].as_bool() != Some(true)
            || proof["positions_compared"].as_u64() != Some(667)
            || proof["blank_positions_compared"].as_u64() != Some(25)
            || proof["blank_zero_empirically_verified"].as_bool() != Some(true)
        {
            return Err(invalid());
        }
        for name in [
            "original_retail_elf_sha256",
            "snapshot_sha256",
            "catalog_sha256",
            "rules_sha256",
            "owner_attestation_sha256",
            "gear_slots_sha256",
            "comparator_sha256",
        ] {
            if !file.provenance[name].as_str().is_some_and(|value| {
                value.len() == 64
                    && value
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            }) {
                return Err(invalid());
            }
        }
        let mut factions = BTreeMap::new();
        for faction in file.factions {
            if faction.faction > 2
                || !faction
                    .class_ref
                    .strip_prefix("CharacterClassInfo")
                    .is_some_and(|value| {
                        !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit())
                    })
            {
                return Err(invalid());
            }
            for (index, row) in faction.loadouts.iter().enumerate() {
                if row.update_id == 0 || row.items.iter().flatten().any(|&id| id >= 4096) {
                    return Err(invalid());
                }
                if index < 3 {
                    if row.primary_ammo == 0 || row.items[..3].iter().any(Option::is_none) {
                        return Err(invalid());
                    }
                } else if row.primary_ammo != 0 || row.items.iter().any(Option::is_some) {
                    return Err(invalid());
                }
            }
            if factions.insert(faction.faction, faction).is_some() {
                return Err(invalid());
            }
        }
        Ok(Self { factions })
    }
    pub fn default_inventory_index(&self, faction: u8) -> Option<u8> {
        self.factions.contains_key(&faction).then_some(0)
    }
    /// Replication only: never resolve these IDs through the prototype authority tables.
    pub fn loadout(&self, faction: u8, index: u8) -> Option<CharacterLoadout> {
        let rows = &self.factions.get(&faction)?.loadouts;
        let selected = rows.get(usize::from(index))?;
        selected.items[0]?;
        Some(CharacterLoadout {
            inventories: std::array::from_fn(|i| CharacterInventory {
                update_id: rows[i].update_id,
                slots: rows[i].items,
            }),
            handle: InventoryHandle {
                update_id: selected.update_id,
                inventory_index: index,
            },
            primary: InventoryItem {
                inventory_update_id: selected.update_id,
                gear_slot: 0,
                ammo_pool: 0,
                ammo_count: selected.primary_ammo,
                reloading: false,
            },
            primary_equipped: true,
        })
    }
}
