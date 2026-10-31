//! Build primitives proven from original artifacts. Object body compatibility
//! and the retail stats runtime guard remain unvalidated.
use serde::Deserialize;
use std::{collections::BTreeMap, io, path::Path};

#[derive(Clone, Copy, Debug, Deserialize, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StatEncoding {
    FixedBeI32,
    UnsignedContinuationU32,
}

/// Retail NetDataMagInfo appends byte members at source offsets0x0A/0x0B.
/// Their meanings are unproved; neutral constructor values come from disk.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MagInfoExtensionDefaults {
    pub member_0a: u8,
    pub member_0b: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WireProfile {
    application_id: u32,
    object_type_bits: usize,
    inventory_item_bits: usize,
    stat_encoding: StatEncoding,
    mag_info_extension: Option<MagInfoExtensionDefaults>,
}
impl WireProfile {
    /// Legacy/test compatibility loads the same embedded JSON parameters.
    /// Production runtime loads a disk catalog once and passes its value explicitly.
    pub fn prototype() -> Self {
        WireProfileCatalog::embedded()
            .profile(20230)
            .expect("prototype wire profile")
    }
    pub fn application_id(self) -> u32 {
        self.application_id
    }
    pub fn object_type_bits(self) -> usize {
        self.object_type_bits
    }
    pub fn header_bits(self) -> usize {
        128 + self.object_type_bits
    }
    pub fn inventory_item_bits(self) -> usize {
        self.inventory_item_bits
    }
    pub fn stat_encoding(self) -> StatEncoding {
        self.stat_encoding
    }
    pub fn mag_info_extension(self) -> Option<MagInfoExtensionDefaults> {
        self.mag_info_extension
    }
    /// Retail inserted NetMessageKeepAlive at 0x0003, moving every DME game
    /// message from prototype 0x0003..=0x0010 up by one (original retail mage_g.elf;
    /// docs/protocol/retail-join/DME-MESSAGE-IDS.md). Gameplay messages from 0x01FB are unchanged.
    fn app_message_shift(self) -> u16 {
        u16::from(self.application_id == 20231)
    }
    /// Wire number of a prototype-numbered DME game message.
    pub fn wire_message_type(self, prototype: u16) -> u16 {
        if (0x0003..=0x0010).contains(&prototype) {
            prototype + self.app_message_shift()
        } else {
            prototype
        }
    }
    /// Prototype number of a client-to-server wire message, or `None` for numbers
    /// this build assigns to something other than a DME game message (retail 0x0003).
    pub fn prototype_message_type(self, wire: u16) -> Option<u16> {
        if self.app_message_shift() == 0 {
            return Some(wire);
        }
        match wire {
            0x0003 => None,
            0x0004..=0x0011 => Some(wire - 1),
            _ => Some(wire),
        }
    }
    /// Only the prototype catalogs have verified numeric item bindings. Retail
    /// authored list-local ordinals cannot be used as wire IDs. This contains an
    /// unproven crash hypothesis; it does not prove a safe empty-inventory fallback.
    pub fn catalog_item_ids_valid(self) -> bool {
        self.application_id == 20230
    }
    /// Retail requires a ProtocolInfo exchange (0x0002) before the client claims.
    pub fn requires_protocol_info(self) -> bool {
        self.app_message_shift() != 0
    }
}
impl Default for WireProfile {
    fn default() -> Self {
        Self::prototype()
    }
}

#[derive(Clone)]
pub struct WireProfileCatalog {
    profiles: BTreeMap<u32, WireProfile>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    schema_version: u8,
    profiles: Vec<Entry>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    build: String,
    application_id: u32,
    object_type_bits: usize,
    inventory_item_bits: usize,
    stat_encoding: StatEncoding,
    #[serde(default)]
    mag_info_extension: Option<MagInfoExtensionDefaults>,
    evidence: String,
}
impl WireProfileCatalog {
    pub fn load(path: impl AsRef<Path>) -> io::Result<Self> {
        Self::from_json(&std::fs::read_to_string(path)?)
    }
    pub fn from_json(json: &str) -> io::Result<Self> {
        let invalid = || io::Error::new(io::ErrorKind::InvalidData, "invalid_wire_profile_catalog");
        let file: File = serde_json::from_str(json).map_err(|_| invalid())?;
        if file.schema_version != 1 || file.profiles.is_empty() || file.profiles.len() > 2 {
            return Err(invalid());
        }
        let mut profiles = BTreeMap::new();
        for e in file.profiles {
            let matches = matches!(
                (
                    e.build.as_str(),
                    e.application_id,
                    e.object_type_bits,
                    e.inventory_item_bits,
                    e.stat_encoding
                ),
                ("prototype", 20230, 19, 10, StatEncoding::FixedBeI32)
                    | (
                        "retail",
                        20231,
                        20,
                        12,
                        StatEncoding::UnsignedContinuationU32
                    )
            );
            if !matches
                || (e.application_id == 20231) != e.mag_info_extension.is_some()
                || !(19..=20).contains(&e.object_type_bits)
                || !(10..=12).contains(&e.inventory_item_bits)
                || e.evidence.is_empty()
            {
                return Err(invalid());
            }
            let value = WireProfile {
                application_id: e.application_id,
                object_type_bits: e.object_type_bits,
                inventory_item_bits: e.inventory_item_bits,
                stat_encoding: e.stat_encoding,
                mag_info_extension: e.mag_info_extension,
            };
            if profiles.insert(e.application_id, value).is_some() {
                return Err(invalid());
            }
        }
        Ok(Self { profiles })
    }
    pub fn profile(&self, application_id: u32) -> Result<WireProfile, &'static str> {
        self.profiles
            .get(&application_id)
            .copied()
            .ok_or("unsupported_application_wire_profile")
    }
    pub(crate) fn embedded() -> Self {
        Self::from_json(include_str!("../data/synthetic-wire-profiles.json"))
            .expect("valid embedded compatibility catalog")
    }
}
pub fn default_application_id() -> u32 {
    20230
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retail_only_catalog_needs_no_prototype_entry() {
        let mut value: serde_json::Value =
            serde_json::from_str(include_str!("../data/synthetic-wire-profiles.json")).unwrap();
        let retail = value["profiles"][1].clone();
        value["profiles"] = serde_json::json!([retail]);
        let catalog = WireProfileCatalog::from_json(&value.to_string()).unwrap();
        assert_eq!(catalog.profile(20231).unwrap().inventory_item_bits(), 12);
        assert_eq!(
            catalog.profile(20230),
            Err("unsupported_application_wire_profile")
        );
    }
}
