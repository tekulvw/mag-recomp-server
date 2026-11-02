//! Explicit selected-match incoming damage policy. Tuning is loaded from disk.
use crate::*;
use serde::Deserialize;
use std::{io::Read, path::Path};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rules {
    schema_version: u8,
    application_id: u32,
    map_name: String,
    game_type: i8,
    friendly_fire_multiplier: f32,
    provenance: serde_json::Value,
}
impl Rules {
    pub fn load(path: impl AsRef<Path>) -> io::Result<Self> {
        let mut bytes = Vec::new();
        std::fs::File::open(path)?
            .take(65_537)
            .read_to_end(&mut bytes)?;
        let text = std::str::from_utf8(&bytes).map_err(io::Error::other)?;
        Self::from_json(text)
    }
    pub fn from_json(text: &str) -> io::Result<Self> {
        if text.len() > 65_536 {
            return Err(io::Error::other("character_damage_rules_too_large"));
        }
        let value: Self = serde_json::from_str(text).map_err(io::Error::other)?;
        value.validate().map_err(io::Error::other)?;
        Ok(value)
    }
    fn validate(&self) -> Result<(), &'static str> {
        let digest = self
            .provenance
            .get("spawn_catalog_sha256")
            .and_then(Value::as_str);
        if self.schema_version != 1
            || self.application_id != 20231
            || !(0..=3).contains(&self.game_type)
            || self.map_name.is_empty()
            || self.map_name.len() > 64
            || !self.friendly_fire_multiplier.is_finite()
            || !(0.0..=100.0).contains(&self.friendly_fire_multiplier)
            || !self.provenance.is_object()
            || !digest.is_some_and(|s| {
                s.len() == 64
                    && s.bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            })
        {
            return Err("invalid_character_damage_rules");
        }
        Ok(())
    }
    pub fn validate_match(
        &self,
        application_id: u32,
        catalog: &spawn_catalog::SpawnCatalog,
    ) -> Result<(), &'static str> {
        self.validate()?;
        if application_id != self.application_id
            || catalog.map_name() != self.map_name
            || catalog.match_profile().game_type != self.game_type
            || catalog.source_sha256() != self.provenance["spawn_catalog_sha256"].as_str()
        {
            return Err("character_damage_rules_match_mismatch");
        }
        Ok(())
    }
    pub fn friendly_fire_multiplier(&self) -> f32 {
        self.friendly_fire_multiplier
    }
}

impl<W: Write> Engine<W> {
    pub fn with_character_damage_rules(mut self, rules: Rules) -> Result<Self, &'static str> {
        let catalog = self
            .spawn_catalog
            .as_ref()
            .ok_or("character_damage_rules_require_spawns")?;
        rules.validate_match(self.wire_profile.application_id(), catalog)?;
        self.character_damage_rules = Some(rules);
        Ok(self)
    }
    /// Native allocated character relationship, including retained dead lives.
    /// Caller context supplies the validated world; wire/source-object aliases
    /// must not be truncated into a different character handle.
    pub(crate) fn character_team_in_world(&self, character: u32, world: u32) -> Option<u8> {
        if let Some(actor) = self
            .peers
            .values()
            .chain(self.active.iter())
            .find(|a| (0x0380_0000 | u32::from(a.reservation.player)) == character)
        {
            return (actor.reservation.world == world).then(|| actor.team());
        }
        if character & 0xffff_0000 != 0x0380_0000 {
            return None;
        }
        if self
            .active
            .iter()
            .chain(self.peers.values())
            .any(|a| a.reservation.world == world)
            && self.accepts_subscription_world(world)
            && let Some(bot) = self.bot_registry.bots.get(&(character as u16))
        {
            return Some(bot.target.squad.team());
        }
        self.peers
            .values()
            .chain(self.active.iter())
            .filter(|a| a.reservation.world == world && a.dummy_created)
            .find_map(|a| {
                a.development_target
                    .as_ref()
                    .filter(|t| {
                        t.runtime.is_some()
                            && (0x0380_0000 | u32::from(t.object_index)) == character
                    })
                    .map(|t| t.squad.team())
            })
    }
    /// Ordinary typed damage only. Heal/Resuscitate do not enter armor damage
    /// calculation, and self falling retains its existing separate base-one path.
    pub(crate) fn character_incoming_base(&self, attacker: u32, victim: u32, world: u32) -> f32 {
        let Some(rules) = &self.character_damage_rules else {
            return 1.0;
        };
        if attacker == victim {
            return 1.0;
        }
        match (
            self.character_team_in_world(attacker, world),
            self.character_team_in_world(victim, world),
        ) {
            (Some(a), Some(v)) if a == v => rules.friendly_fire_multiplier(),
            _ => 1.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture() -> (crate::Engine<Vec<u8>>, serde_json::Value) {
        let engine = crate::spawn_progression::engine();
        let catalog = engine.spawn_catalog.as_ref().unwrap();
        let value = json!({"schema_version":1,"application_id":20231,
            "map_name":catalog.map_name(),"game_type":catalog.match_profile().game_type,
            "friendly_fire_multiplier":0.375,"provenance":{
                "source":"synthetic","spawn_catalog_sha256":catalog.source_sha256().unwrap()}});
        (engine, value)
    }
    #[test]
    fn external_rules_bind_exact_selected_spawn_map_mode_and_retail_profile() {
        let (engine, value) = fixture();
        let rules = Rules::from_json(&value.to_string()).unwrap();
        let catalog = engine.spawn_catalog.as_ref().unwrap();
        assert!(rules.validate_match(20231, catalog).is_ok());
        assert!(rules.validate_match(20230, catalog).is_err());
        for (field, wrong) in [("map_name", json!("other_map")), ("game_type", json!(3))] {
            let mut bad = value.clone();
            bad[field] = wrong;
            assert!(
                Rules::from_json(&bad.to_string())
                    .unwrap()
                    .validate_match(20231, catalog)
                    .is_err()
            );
        }
        let mut bad = value.clone();
        bad["provenance"]["spawn_catalog_sha256"] = json!("b".repeat(64));
        assert!(
            Rules::from_json(&bad.to_string())
                .unwrap()
                .validate_match(20231, catalog)
                .is_err()
        );
        assert!(engine.with_character_damage_rules(rules).is_ok());
    }
    #[test]
    fn external_rules_are_strict_finite_and_use_native_float_bounds() {
        let (engine, value) = fixture();
        for n in [0.0, 100.0] {
            let mut v = value.clone();
            v["friendly_fire_multiplier"] = json!(n);
            assert!(Rules::from_json(&v.to_string()).is_ok());
        }
        for (field, bad) in [
            ("schema_version", json!(2)),
            ("application_id", json!(20230)),
            ("game_type", json!(-1)),
            ("game_type", json!(4)),
            ("friendly_fire_multiplier", json!(-0.01)),
            ("friendly_fire_multiplier", json!(101.0)),
            ("friendly_fire_multiplier", json!("NaN")),
            ("provenance", json!(null)),
            ("extra", json!(true)),
        ] {
            let mut v = value.clone();
            v[field] = bad;
            assert!(Rules::from_json(&v.to_string()).is_err(), "{field}");
        }
        let mut missing = value.clone();
        missing["provenance"]
            .as_object_mut()
            .unwrap()
            .remove("spawn_catalog_sha256");
        assert!(Rules::from_json(&missing.to_string()).is_err());
        let mut bypass = value.clone();
        bypass["friendly_fire_multiplier"] = json!(101.0);
        let direct: Rules = serde_json::from_value(bypass).unwrap();
        assert!(
            direct
                .validate_match(20231, engine.spawn_catalog.as_ref().unwrap())
                .is_err()
        );
    }
}
