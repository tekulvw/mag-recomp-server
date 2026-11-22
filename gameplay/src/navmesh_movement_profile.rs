//! Exact external movement rules projected from the supported retail source.
use super::{Error, sha256, valid_sha};
use serde::Deserialize;
use std::{
    fs::File,
    io::{self, Read},
    path::Path,
};

const MAX_BYTES: usize = 4096;
const SOURCE_RULES_SHA256: &str =
    "af01772b4d570e6e5c3bbf92a57101c5e066571cc6a586e8b9ed964cd512070a";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    retail_elf_sha256: String,
    game_archive_sha256: String,
    character_zpr_sha256: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    schema_version: u32,
    application_id: u32,
    source: Source,
    max_ascent_degrees: f64,
    max_no_slide_degrees: f64,
}

/// The opt-in slope policy retains the replacement runner's other body values.
#[derive(Debug)]
pub struct MovementProfile {
    sha256: String,
    slope_degrees: f64,
}

impl MovementProfile {
    #[cfg(test)]
    pub(crate) fn synthetic(slope_degrees: f64) -> Self {
        Self {
            sha256: "1".repeat(64),
            slope_degrees,
        }
    }

    pub fn load(path: impl AsRef<Path>) -> io::Result<Self> {
        let mut bytes = Vec::new();
        File::open(path)?
            .take(MAX_BYTES as u64 + 1)
            .read_to_end(&mut bytes)?;
        Self::from_bytes(&bytes).map_err(io::Error::other)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() > MAX_BYTES {
            return Err(Error("movement_profile_too_large"));
        }
        let document: Document = serde_json::from_slice(bytes)
            .map_err(|_| Error("movement_profile_invalid_document"))?;
        let slope = document.max_ascent_degrees;
        if document.schema_version != 1
            || document.application_id != 20231
            || !slope.is_finite()
            || slope <= 0.
            || slope > 60.
            || document.max_no_slide_degrees != slope
            || !valid_sha(&document.source.retail_elf_sha256)
            || !valid_sha(&document.source.game_archive_sha256)
            || !valid_sha(&document.source.character_zpr_sha256)
        {
            return Err(Error("movement_profile_invalid_rules"));
        }
        let digest = sha256(bytes);
        if digest != SOURCE_RULES_SHA256 {
            return Err(Error("movement_profile_source_mismatch"));
        }
        Ok(Self {
            sha256: digest,
            slope_degrees: slope,
        })
    }

    pub fn source_sha256(&self) -> &str {
        &self.sha256
    }

    pub fn slope_degrees(&self) -> f64 {
        self.slope_degrees
    }
}
