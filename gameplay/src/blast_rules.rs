//! Original character-blast callback inputs, separate from authored ammo data.
use serde::Deserialize;
use serde_json::Value;
use std::{io, path::Path};

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CharacterBlastRules {
    pub armor_rating_reduction: f32,
    pub armor_prevention_reduction: f32,
    pub hit_location: u8,
    pub damage_type: u8,
}
impl CharacterBlastRules {
    pub fn load(path: impl AsRef<Path>) -> io::Result<Self> {
        let raw = serde_json::from_reader(io::BufReader::new(std::fs::File::open(path)?))?;
        Self::from_value(raw)
    }
    pub fn from_value(raw: Value) -> io::Result<Self> {
        let invalid = || {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid native character-blast rules",
            )
        };
        if raw["schema_version"].as_u64() != Some(1)
            || raw["applications"].as_array().is_none_or(|apps| {
                apps.len() != 2
                    || !apps.contains(&Value::from(20230))
                    || !apps.contains(&Value::from(20231))
            })
        {
            return Err(invalid());
        }
        let rules: Self =
            serde_json::from_value(raw["character_damage"].clone()).map_err(|_| invalid())?;
        if !rules.armor_rating_reduction.is_finite()
            || rules.armor_rating_reduction < 0.0
            || !rules.armor_prevention_reduction.is_finite()
            || rules.armor_prevention_reduction < 0.0
            || rules.hit_location >= 16
            // Native type10 is explosive base2 together with KineticResponse8.
            || rules.damage_type != 10
        {
            return Err(invalid());
        }
        Ok(rules)
    }
}
