//! Sabotage launcher and frag Character doses for registry profiles.
//!
//! A direct retail Sabotage launch that loads `--collision-world` and
//! `--character-damage-rules` flies stock launcher rockets and doses frag
//! blasts against that static world. A registry worker loads those inputs per
//! profile instead, so each Sabotage profile qualifies on its own: it needs a
//! non-empty collision world for its own map and its own friendly-fire rules,
//! and the worker's item catalog must be the retail catalog its combat
//! metadata names. Selecting a qualified profile installs both; selecting any
//! other profile clears them. A profile that cannot qualify keeps its refusal
//! for the startup and selection reports.
use super::*;
use serde_json::Value;

/// One profile's qualification for the static-world Character weapons.
/// `None` means the path does not apply to the profile at all.
#[derive(Clone, Default)]
pub(super) struct Weapons {
    rockets: Option<Result<Arc<crate::infantry_rockets::Catalog>, &'static str>>,
    frags: Option<Result<Vec<u16>, &'static str>>,
}

impl Weapons {
    /// Launcher rows to install when this profile is selected.
    pub(super) fn rockets(&self) -> Option<Arc<crate::infantry_rockets::Catalog>> {
        self.rockets.clone()?.ok()
    }

    /// Whether frag blasts dose Characters when this profile is selected.
    pub(super) fn frags(&self) -> bool {
        matches!(self.frags, Some(Ok(_)))
    }
}

/// Why a Sabotage profile with a collision world cannot use it, if it can't.
fn static_world_refusal(profile: &Loaded) -> Option<&'static str> {
    let world = profile.collision.as_ref()?;
    if world.stats().triangles == 0 {
        return Some("static_world_collision_empty");
    }
    if world.metadata().map_name != profile.spawns.map_name() {
        return Some("static_world_collision_map_mismatch");
    }
    None
}

impl Registry {
    /// Qualify every Sabotage profile that loads its own static collision
    /// world for item-catalog launcher rows and frag Character blast. Other
    /// modes keep their represented projectile scenes; their launcher rows
    /// come from [`Registry::bind_infantry_rockets`].
    pub fn bind_static_world_weapons(&mut self, items: &item_catalog::ItemCatalog) {
        self.static_world.clear();
        let mut derived = None;
        for (id, profile) in &self.profiles {
            if profile.summary.game_type != 0
                || profile.acquisition.is_some()
                || profile.domination.is_some()
                || profile.infantry_rocket_source.is_some()
                || profile.collision.is_none()
            {
                continue;
            }
            let refusal = static_world_refusal(profile);
            let rockets = match (refusal, profile.damage.is_some()) {
                (Some(reason), _) => Err(reason),
                // Character rocket doses resolve friendly fire through these rules.
                (None, false) => Err("character_damage_rules_required"),
                (None, true) => derived
                    .get_or_insert_with(|| {
                        crate::infantry_rockets::Catalog::from_item_catalog(items).map(Arc::new)
                    })
                    .clone(),
            };
            let frags = match refusal {
                Some(reason) => Err(reason),
                None => crate::static_world_frags::qualified_items(items, profile.damage.is_some()),
            };
            self.static_world.insert(
                id.clone(),
                Weapons {
                    rockets: Some(rockets),
                    frags: Some(frags),
                },
            );
        }
    }

    /// Whether any profile flies launcher rockets against its static world.
    pub fn has_static_world_rockets(&self) -> bool {
        self.static_world
            .values()
            .any(|weapons| weapons.rockets().is_some())
    }

    /// Per-profile launcher and frag Character dose status for the startup
    /// report, keyed by profile id: `(infantry_rockets, frag_blast)`.
    pub fn character_weapon_reports(&self) -> (Value, Value) {
        let mut rockets = serde_json::Map::new();
        let mut frags = serde_json::Map::new();
        for id in self.profiles.keys() {
            let report = self.character_weapon_report(id);
            rockets.insert(id.clone(), report["infantry_rockets"].clone());
            frags.insert(id.clone(), report["frag_blast"].clone());
        }
        (json!({"per_profile":rockets}), json!({"per_profile":frags}))
    }

    /// One profile's launcher and frag Character dose status.
    pub(super) fn character_weapon_report(&self, profile_id: &str) -> Value {
        let Some(profile) = self.profiles.get(profile_id) else {
            return Value::Null;
        };
        let weapons = self.static_world.get(profile_id);
        let rockets = match (
            weapons.and_then(|w| w.rockets.as_ref()),
            &profile.infantry_rockets,
        ) {
            (Some(Ok(catalog)), _) => json!({"source":"item_catalog",
                "scene":"static_collision_world","items":items(catalog)}),
            (Some(Err(reason)), _) => json!({"enabled":false,"reason":reason}),
            (None, Some(catalog)) => json!({"source":"profile_rules",
                "scene":"represented_projectile_scene","items":items(catalog)}),
            (None, None) => Value::Null,
        };
        let represented = profile
            .acquisition
            .as_ref()
            .map(|setup| setup.frag_character_blast)
            .or_else(|| {
                profile
                    .domination
                    .as_ref()
                    .map(|world| world.state.frag_character_blast)
            });
        let frags = match (weapons.and_then(|w| w.frags.as_ref()), represented) {
            (Some(Ok(items)), _) => json!({"enabled":true,"scene":"static_collision_world",
                "cover":"any_static_crossing","items":items}),
            (Some(Err(reason)), _) => json!({"enabled":false,"reason":reason}),
            (None, Some(enabled)) => {
                json!({"enabled":enabled,"scene":"represented_projectile_scene"})
            }
            (None, None) => Value::Null,
        };
        json!({"infantry_rockets":rockets,"frag_blast":frags})
    }
}

fn items(catalog: &crate::infantry_rockets::Catalog) -> Vec<u16> {
    catalog.variants().iter().map(|v| v.item_id).collect()
}

#[cfg(test)]
#[path = "match_profile_static_world_tests.rs"]
pub(crate) mod tests;
