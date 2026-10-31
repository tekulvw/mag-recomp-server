//! Validated authored equipment, loaded once before accepting clients.
//! The raw catalog retains every extracted item; typed views cover only equipment
//! whose runtime authority and native formulas have already been established.
use serde_json::Value;
use std::{
    collections::BTreeMap,
    io,
    path::Path,
    sync::{Arc, OnceLock},
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DamageParameters {
    pub character: f32,
    pub armor_pierce: f32,
    pub armor_redux: f32,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TargetDamageParameters {
    pub amount: f32,
    pub armor_pierce: f32,
    pub armor_redux: f32,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BulletParameters {
    pub damage: DamageParameters,
    pub ammo_token: u32,
    pub effective_range: f32,
    pub max_range: f32,
    pub magazine_capacity: u8,
    pub initial_magazine_count: u8,
}
/// Native bipods with no replacement WeaponData preserve the host bullet profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BulletAttachmentKind {
    Bipod,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BodyParameters {
    pub armor_area: usize,
    pub multiplier: f32,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ArmorParameters {
    pub armor_type: u8,
    pub rating: f32,
    pub prevention: f32,
    pub coverage: [f32; 4],
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HealingParameters {
    pub cooldown_seconds: f32,
    pub heal_percent: f32,
    pub ticks: u8,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GrenadeParameters {
    pub damage: DamageParameters,
    pub ammo_token: u32,
    pub full_radius: f32,
    pub half_radius: f32,
    pub fuse_ms: u64,
    pub authored_lifetime_ms: u64,
    pub initial_count: u8,
}
#[derive(Debug)]
pub struct ItemCatalog {
    raw: Value,
    character_blast_rules: crate::blast_rules::CharacterBlastRules,
    valor_defaults: Option<crate::loadouts::SavedLoadouts>,
    retail_item_refs: BTreeMap<u16, String>,
    bootstrap_loadouts: Option<crate::bootstrap_loadouts::BootstrapLoadouts>,
    retail_combat: Option<crate::retail_combat::RetailCombatCatalog>,
    retail_armory: Option<crate::retail_armory::RetailArmory>,
    bullets: BTreeMap<u16, BulletParameters>,
    vehicle_bullets: BTreeMap<u16, TargetDamageParameters>,
    bullet_attachments: BTreeMap<(u16, u16), BulletAttachmentKind>,
    armor: BTreeMap<u16, ArmorParameters>,
    body: [BodyParameters; 16],
    healing: BTreeMap<u16, HealingParameters>,
    grenades: BTreeMap<u16, GrenadeParameters>,
}
fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
fn field<'a>(value: &'a Value, name: &str) -> io::Result<&'a Value> {
    value
        .get(name)
        .ok_or_else(|| invalid(format!("item catalog missing {name}")))
}
fn number(value: &Value, name: &str) -> io::Result<f32> {
    let source = field(value, name)?
        .as_f64()
        .ok_or_else(|| invalid(format!("item catalog invalid {name}")))?;
    let result = source as f32;
    if !result.is_finite() || result < 0.0 {
        return Err(invalid(format!(
            "item catalog nonfinite or negative {name}"
        )));
    }
    Ok(result)
}
fn byte(value: &Value, name: &str) -> io::Result<u8> {
    field(value, name)?
        .as_u64()
        .and_then(|v| u8::try_from(v).ok())
        .filter(|v| *v > 0)
        .ok_or_else(|| invalid(format!("invalid item catalog {name}")))
}
fn damage(value: &Value) -> io::Result<DamageParameters> {
    let value = field(value, "damage")?;
    Ok(DamageParameters {
        character: number(value, "character")?,
        armor_pierce: number(value, "characterAPierce")?,
        armor_redux: number(value, "characterARedux")?,
    })
}
fn token(value: &Value) -> io::Result<u32> {
    let value = field(value, "native_token_u32")?
        .as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(|| invalid("invalid native ammo token"))?;
    if value == 0 {
        return Err(invalid("zero native ammo token"));
    }
    Ok(value)
}
fn milliseconds(seconds: f32) -> io::Result<u64> {
    let ms = f64::from(seconds) * 1000.0;
    if ms <= 0.0 || ms.fract() != 0.0 || ms >= u64::MAX as f64 {
        return Err(invalid("item catalog invalid millisecond duration"));
    }
    Ok(ms as u64)
}
impl ItemCatalog {
    pub fn load(path: impl AsRef<Path>) -> io::Result<Self> {
        Self::load_with_blast_rules(
            path,
            Path::new(env!("CARGO_MANIFEST_DIR")).join("data/character-blast-rules.json"),
        )
    }
    pub fn load_with_blast_rules(
        path: impl AsRef<Path>,
        rules_path: impl AsRef<Path>,
    ) -> io::Result<Self> {
        let path = path.as_ref();
        let raw = serde_json::from_reader(io::BufReader::new(std::fs::File::open(path)?))
            .map_err(|e| invalid(e.to_string()))?;
        let defaults_path = path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("prototype-loadouts.json");
        Self::from_values(
            raw,
            Self::read_defaults(defaults_path)?,
            crate::blast_rules::CharacterBlastRules::load(rules_path)?,
        )
    }
    /// Select the catalog schema explicitly. Retail never reads prototype defaults.
    pub fn load_for_application(
        path: impl AsRef<Path>,
        rules_path: impl AsRef<Path>,
        application_id: u32,
    ) -> io::Result<Self> {
        match application_id {
            20230 => Self::load_with_blast_rules(path, rules_path),
            20231 => {
                let raw = serde_json::from_reader(io::BufReader::new(std::fs::File::open(path)?))
                    .map_err(|e| invalid(e.to_string()))?;
                Self::from_value_for_application(
                    raw,
                    crate::blast_rules::CharacterBlastRules::load(rules_path)?,
                    application_id,
                )
            }
            _ => Err(invalid("unsupported item catalog application")),
        }
    }
    pub fn from_value_for_application(
        raw: Value,
        character_blast_rules: crate::blast_rules::CharacterBlastRules,
        application_id: u32,
    ) -> io::Result<Self> {
        let schema = match application_id {
            20230 => 1,
            20231 => 2,
            _ => return Err(invalid("unsupported item catalog application")),
        };
        if raw.get("schema_version").and_then(Value::as_u64) != Some(schema)
            || raw.get("application_id").and_then(Value::as_u64) != Some(u64::from(application_id))
        {
            return Err(invalid("unsupported item catalog schema/application"));
        }
        if application_id == 20230 {
            return Self::from_values(
                raw,
                Self::read_defaults(
                    Path::new(env!("CARGO_MANIFEST_DIR")).join("data/prototype-loadouts.json"),
                )?,
                character_blast_rules,
            );
        }
        let items = field(&raw, "items")?
            .as_object()
            .filter(|items| !items.is_empty())
            .ok_or_else(|| invalid("invalid retail item table"))?;
        let definitions = field(&raw, "definitions")?
            .as_object()
            .ok_or_else(|| invalid("invalid retail definition table"))?;
        let mut retail_item_refs = BTreeMap::new();
        for (reference, item) in items {
            let (namespace, ordinal) = reference
                .split_once(':')
                .ok_or_else(|| invalid("invalid retail authored reference"))?;
            let list = namespace
                .strip_prefix("FactionItemList")
                .and_then(|s| s.parse::<u16>().ok())
                .filter(|&list| list < 4 && namespace == format!("FactionItemList{list}"))
                .ok_or_else(|| invalid("invalid retail item namespace"))?;
            let ordinal = ordinal
                .parse::<u16>()
                .ok()
                .filter(|&ordinal| ordinal < 1024 && reference == &format!("{namespace}:{ordinal}"))
                .ok_or_else(|| invalid("invalid retail local ordinal"))?;
            if item.get("authored_ref").and_then(Value::as_str) != Some(reference)
                || item.get("faction_list").and_then(Value::as_str) != Some(namespace)
                || item.get("local_ordinal").and_then(Value::as_u64) != Some(u64::from(ordinal))
                || item.get("wire_item_id") != Some(&Value::Null)
            {
                return Err(invalid("inconsistent retail item identity"));
            }
            // Native ordinary inventory flag 0, also used by the offline native
            // comparator: faction namespaces carry ten ordinal bits plus offset.
            // This binds inventory IDs only; it proves no ammo-token algorithm.
            let wire_id = ((list << 10) | ordinal) + if list == 0 { 0 } else { 50 };
            if wire_id >= 4096
                || retail_item_refs
                    .insert(wire_id, reference.clone())
                    .is_some()
            {
                return Err(invalid("invalid or duplicate retail wire item ID"));
            }
            if item.get("item_type") == Some(&Value::Null) {
                if item.get("name") != Some(&Value::Null)
                    || item.get("definition") != Some(&Value::Null)
                {
                    return Err(invalid("malformed reserved retail item"));
                }
                continue;
            }
            field(item, "item_type")?
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| invalid("invalid retail item type"))?;
            let name = field(item, "name")?
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| invalid("invalid retail item name"))?;
            let binding = field(item, "definition")?;
            // Preserve incomplete extraction coverage without treating it as
            // supported equipment. validate_item rejects these assigned rows.
            if binding.is_null() {
                continue;
            }
            let group = field(binding, "group")?
                .as_str()
                .ok_or_else(|| invalid("invalid retail definition group"))?;
            let definition_name = field(binding, "name")?
                .as_str()
                .ok_or_else(|| invalid("invalid retail definition name"))?;
            if name != definition_name
                || definitions
                    .get(group)
                    .and_then(|group| group.get(name))
                    .is_none()
            {
                return Err(invalid("unresolved retail item definition"));
            }
        }
        Ok(Self {
            raw,
            character_blast_rules,
            valor_defaults: None,
            retail_item_refs,
            bootstrap_loadouts: None,
            retail_combat: None,
            retail_armory: None,
            bullets: BTreeMap::new(),
            vehicle_bullets: BTreeMap::new(),
            bullet_attachments: BTreeMap::new(),
            armor: BTreeMap::new(),
            body: [BodyParameters {
                armor_area: 0,
                multiplier: 0.0,
            }; 16],
            healing: BTreeMap::new(),
            grenades: BTreeMap::new(),
        })
    }
    pub fn application_id(&self) -> u32 {
        self.raw["application_id"]
            .as_u64()
            .expect("validated application") as u32
    }
    pub fn from_value(raw: Value) -> io::Result<Self> {
        Self::from_values(
            raw,
            Self::read_defaults(
                Path::new(env!("CARGO_MANIFEST_DIR")).join("data/prototype-loadouts.json"),
            )?,
            crate::blast_rules::CharacterBlastRules::load(
                Path::new(env!("CARGO_MANIFEST_DIR")).join("data/character-blast-rules.json"),
            )?,
        )
    }
    fn read_defaults(path: impl AsRef<Path>) -> io::Result<crate::loadouts::SavedLoadouts> {
        let raw: Value = serde_json::from_reader(io::BufReader::new(std::fs::File::open(path)?))
            .map_err(|e| invalid(e.to_string()))?;
        if raw.get("application_id").and_then(Value::as_u64) != Some(20230) {
            return Err(invalid("unsupported loadout catalog application"));
        }
        let faction = raw
            .get("factions")
            .and_then(Value::as_array)
            .and_then(|rows| {
                rows.iter()
                    .find(|row| row.get("faction").and_then(Value::as_u64) == Some(0))
            })
            .ok_or_else(|| invalid("missing authored Valor loadouts"))?;
        let rows = field(faction, "loadouts")?
            .as_array()
            .filter(|rows| !rows.is_empty() && rows.len() <= 5)
            .ok_or_else(|| invalid("invalid authored loadout count"))?;
        let initial_version = rows[0]
            .get("update_id")
            .and_then(Value::as_u64)
            .and_then(|v| u8::try_from(v).ok())
            .ok_or_else(|| invalid("invalid authored inventory version"))?;
        let mut defaults = crate::loadouts::SavedLoadouts {
            selected_index: 0,
            update_ids: [initial_version; 5],
            inventories: [[None; 22]; 5],
        };
        for (i, row) in rows.iter().enumerate() {
            defaults.update_ids[i] = field(row, "update_id")?
                .as_u64()
                .and_then(|v| u8::try_from(v).ok())
                .ok_or_else(|| invalid("invalid authored inventory version"))?;
            let items = field(row, "items")?
                .as_array()
                .filter(|items| items.len() == 22)
                .ok_or_else(|| invalid("invalid authored inventory width"))?;
            for (slot, item) in items.iter().enumerate() {
                defaults.inventories[i][slot] = match item.as_i64() {
                    Some(-1) => None,
                    Some(id) => {
                        Some(u16::try_from(id).map_err(|_| invalid("invalid authored item ID"))?)
                    }
                    None => return Err(invalid("invalid authored item ID")),
                };
            }
        }
        Ok(defaults)
    }
    fn from_values(
        raw: Value,
        valor_defaults: crate::loadouts::SavedLoadouts,
        character_blast_rules: crate::blast_rules::CharacterBlastRules,
    ) -> io::Result<Self> {
        if raw.get("schema_version").and_then(Value::as_u64) != Some(1)
            || raw.get("application_id").and_then(Value::as_u64) != Some(20230)
        {
            return Err(invalid("unsupported item catalog schema/application"));
        }
        let definition = |id: u16, group: &str, name: &str| -> io::Result<&Value> {
            let binding = raw
                .pointer(&format!("/items/{id}/definition"))
                .ok_or_else(|| invalid(format!("missing item {id}")))?;
            if binding.get("group").and_then(Value::as_str) != Some(group)
                || binding.get("name").and_then(Value::as_str) != Some(name)
            {
                return Err(invalid(format!("unexpected supported item binding {id}")));
            }
            raw.get("definitions")
                .and_then(|v| v.get(group))
                .and_then(|v| v.get(name))
                .ok_or_else(|| invalid(format!("unresolved item definition {id}")))
        };
        let linked_ammo = |weapon: &Value, group: &str| -> io::Result<&Value> {
            let name = field(field(weapon, "parameters")?, "ammo_name")?
                .as_str()
                .ok_or_else(|| invalid("invalid ammo name"))?;
            raw.get("definitions")
                .and_then(|v| v.get(group))
                .and_then(|v| v.get(name))
                .ok_or_else(|| invalid(format!("unresolved ammo {name}")))
        };
        let mut bullets = BTreeMap::new();
        let mut vehicle_bullets = BTreeMap::new();
        let mut bullet_attachments = BTreeMap::new();
        for (id, name) in [
            (0, "M9"),
            (13, "m4a1"),
            (22, "mk14_mod0"),
            (31, "mk46_mod0"),
            (53, "minigun_green"),
        ] {
            let weapon = definition(id, "weapons", name)?;
            let p = field(weapon, "parameters")?;
            let ammo = linked_ammo(weapon, "hitscan_ammo")?;
            // Resolve compatibility from the authored weapon list and global item
            // bindings. Native SimWeaponKit ignores damage-neutral attachments:
            // SetInventory00848b1c requires a replacement WeaponData pointer.
            if let Some(list) = p.get("attachment_list") {
                let list = list
                    .as_object()
                    .ok_or_else(|| invalid("invalid weapon attachment list"))?;
                for row in list.values() {
                    let name = field(field(row, "WeaponAttachment")?, "name")?
                        .as_str()
                        .ok_or_else(|| invalid("invalid weapon attachment name"))?;
                    let attachment = raw
                        .get("definitions")
                        .and_then(|v| v.get("attachments"))
                        .and_then(|v| v.get(name))
                        .ok_or_else(|| invalid("unresolved weapon attachment"))?;
                    let parameters = field(attachment, "parameters")?;
                    if parameters.get("attachment_type").and_then(Value::as_str)
                        != Some("kAttachTypeBipod")
                        || parameters.get("weapon_name").and_then(Value::as_str) != Some("")
                    {
                        continue;
                    }
                    let items = field(&raw, "items")?
                        .as_object()
                        .ok_or_else(|| invalid("invalid global item bindings"))?;
                    for (ordinal, item) in items {
                        if item.get("item_type").and_then(Value::as_str) == Some("kAttachmentItem")
                            && item.pointer("/definition/group").and_then(Value::as_str)
                                == Some("attachments")
                            && item.pointer("/definition/name").and_then(Value::as_str)
                                == Some(name)
                        {
                            let attachment_id =
                                ordinal
                                    .parse::<u16>()
                                    .ok()
                                    .filter(|v| *v < 1024)
                                    .ok_or_else(|| invalid("invalid supported attachment ID"))?;
                            bullet_attachments
                                .insert((id, attachment_id), BulletAttachmentKind::Bipod);
                        }
                    }
                }
            }
            let effective_range = number(p, "effective_range")?;
            let max_range = number(p, "max_range")?;
            if max_range == 0.0 || effective_range > max_range {
                return Err(invalid("invalid bullet range ordering"));
            }
            let vehicle_damage = field(field(ammo, "parameters")?, "damage")?;
            vehicle_bullets.insert(
                id,
                TargetDamageParameters {
                    amount: number(vehicle_damage, "vehicle")?,
                    armor_pierce: number(vehicle_damage, "vehicleAPierce")?,
                    armor_redux: number(vehicle_damage, "vehicleARedux")?,
                },
            );
            bullets.insert(
                id,
                BulletParameters {
                    damage: damage(field(ammo, "parameters")?)?,
                    ammo_token: token(ammo)?,
                    effective_range,
                    max_range,
                    magazine_capacity: byte(p, "magazine_capacity")?,
                    initial_magazine_count: byte(p, "initial_magazine_count")?,
                },
            );
        }
        let mut body = [BodyParameters {
            armor_area: 0,
            multiplier: 0.0,
        }; 16];
        for (location, entry) in body.iter_mut().enumerate() {
            let p = raw.pointer(&format!("/resources/collision_info.lua/parameters/CharacterCollisionData/CharacterStyles/CharacterStyle0/CharacterStyle/m_bodyData/CollisionBodyInfo{location}/CollisionBodyInfo")).ok_or_else(|| invalid("missing character body parameters"))?;
            let area = field(p, "armorArea")?
                .as_u64()
                .filter(|n| *n < 4)
                .ok_or_else(|| invalid("invalid body armor area"))?;
            *entry = BodyParameters {
                armor_area: area as usize,
                multiplier: number(p, "hitpointMult")?,
            };
        }
        let mut armor = BTreeMap::new();
        for id in (116..=121).chain(131..=142) {
            let binding = raw
                .pointer(&format!("/items/{id}/definition"))
                .ok_or_else(|| invalid("missing armor item"))?;
            let name = field(binding, "name")?
                .as_str()
                .ok_or_else(|| invalid("invalid armor item name"))?;
            let item = definition(id, "armor_items", name)?;
            let armor_type = field(field(item, "parameters")?, "armor_type")?
                .as_u64()
                .filter(|n| (1..=6).contains(n))
                .ok_or_else(|| invalid("invalid armor type"))? as u8;
            let p = raw
                .pointer(&format!("/definitions/armor_types/{armor_type}/parameters"))
                .ok_or_else(|| invalid("unresolved armor type"))?;
            let values = field(p, "m_coverage")?
                .as_array()
                .filter(|v| v.len() == 4)
                .ok_or_else(|| invalid("invalid armor coverage array"))?;
            let mut coverage = [0.0; 4];
            for (value, out) in values.iter().zip(&mut coverage) {
                *out = value
                    .as_f64()
                    .map(|v| v as f32)
                    .filter(|v| v.is_finite() && (0.0..=1.0).contains(v))
                    .ok_or_else(|| invalid("invalid armor coverage"))?;
            }
            armor.insert(
                id,
                ArmorParameters {
                    armor_type,
                    rating: number(p, "rating")?,
                    prevention: number(p, "prevention")?,
                    coverage,
                },
            );
        }
        let globals = raw
            .pointer("/resources/active_items.lua/parameters/ActiveItemDataTable")
            .ok_or_else(|| invalid("missing healing globals"))?;
        let mut healing = BTreeMap::new();
        for (id, name) in [(125, "first_aid_pack"), (98, "medkit")] {
            let p = field(definition(id, "active_items", name)?, "parameters")?;
            let duration = number(globals, &format!("{name}_heal_duration"))?;
            if duration < 1.0 || duration > u8::MAX as f32 || duration.fract() != 0.0 {
                return Err(invalid("invalid healing tick duration"));
            }
            let cooldown_seconds = number(p, "cooldown_time")?;
            std::time::Duration::try_from_secs_f32(cooldown_seconds)
                .map_err(|_| invalid("invalid healing cooldown duration"))?;
            healing.insert(
                id,
                HealingParameters {
                    cooldown_seconds,
                    heal_percent: number(globals, &format!("{name}_heal_percent"))?,
                    ticks: duration as u8,
                },
            );
        }
        let weapon = definition(42, "weapons", "grenade_frag_green")?;
        let ammo = linked_ammo(weapon, "projectile_ammo")?;
        let p = field(ammo, "parameters")?;
        let full_radius = number(p, "full_damage_radius")?;
        let half_radius = number(p, "half_damage_radius")?;
        if half_radius == 0.0 || full_radius <= half_radius {
            return Err(invalid("invalid grenade radius ordering"));
        }
        let fuse_ms = milliseconds(number(p, "detonation_timer")?)?;
        let authored_lifetime_ms = milliseconds(number(p, "m_fLifetimeMax")?)?;
        if authored_lifetime_ms
            .checked_add(crate::projectile::PROJECTILE_AGE_GRACE_MS)
            .is_none()
        {
            return Err(invalid("grenade lifetime allowance overflows"));
        }
        if fuse_ms > authored_lifetime_ms {
            return Err(invalid("grenade fuse exceeds authored lifetime"));
        }
        let initial_count = field(field(weapon, "parameters")?, "initial_magazine_count")?
            .as_u64()
            .and_then(|v| u8::try_from(v).ok())
            .filter(|v| *v > 0)
            .ok_or_else(|| invalid("invalid initial grenade count"))?;
        let grenades = BTreeMap::from([(
            42,
            GrenadeParameters {
                damage: damage(p)?,
                ammo_token: token(ammo)?,
                full_radius,
                half_radius,
                fuse_ms,
                authored_lifetime_ms,
                initial_count,
            },
        )]);
        let catalog = Self {
            raw,
            character_blast_rules,
            valor_defaults: Some(valor_defaults),
            retail_item_refs: BTreeMap::new(),
            bootstrap_loadouts: None,
            retail_combat: None,
            retail_armory: None,
            bullets,
            vehicle_bullets,
            bullet_attachments,
            armor,
            body,
            healing,
            grenades,
        };
        catalog
            .valor_defaults
            .as_ref()
            .expect("prototype defaults")
            .validate(&catalog)
            .map_err(invalid)?;
        Ok(catalog)
    }
    pub fn with_retail_combat(
        mut self,
        catalog: crate::retail_combat::RetailCombatCatalog,
    ) -> Self {
        self.retail_combat = Some(catalog);
        self
    }
    pub fn retail_combat(
        &self,
    ) -> Result<&crate::retail_combat::RetailCombatCatalog, &'static str> {
        self.retail_combat
            .as_ref()
            .ok_or("retail_combat_unavailable")
    }
    pub fn with_retail_armory(mut self, catalog: crate::retail_armory::RetailArmory) -> Self {
        self.retail_armory = Some(catalog);
        self
    }
    pub fn retail_armory(&self) -> Option<&crate::retail_armory::RetailArmory> {
        self.retail_armory.as_ref()
    }
    pub fn with_bootstrap_loadouts(
        mut self,
        catalog: crate::bootstrap_loadouts::BootstrapLoadouts,
    ) -> Self {
        self.bootstrap_loadouts = Some(catalog);
        self
    }
    pub fn bootstrap_loadout(
        &self,
        faction: u8,
        index: u8,
    ) -> Option<crate::replication::CharacterLoadout> {
        self.bootstrap_loadouts.as_ref()?.loadout(faction, index)
    }
    pub fn bootstrap_default_inventory_index(&self, faction: u8) -> Option<u8> {
        self.bootstrap_loadouts
            .as_ref()?
            .default_inventory_index(faction)
    }
    pub fn validate_bootstrap_faction(&self, faction: u8) -> Result<(), &'static str> {
        if self.bootstrap_loadouts.is_some()
            && self.bootstrap_default_inventory_index(faction).is_none()
        {
            return Err("bootstrap_catalog_missing_match_faction");
        }
        Ok(())
    }
    /// Runtime authored defaults, used only when no character edits were saved.
    pub fn valor_loadout(&self, index: u8) -> Option<crate::replication::CharacterLoadout> {
        self.valor_defaults.as_ref()?.loadout(self, index)
    }
    fn item(&self, id: u16) -> Result<&Value, &'static str> {
        let retail = self.application_id() == 20231;
        if id >= if retail { 4096 } else { 1024 } {
            return Err("saved_item_out_of_wire_range");
        }
        let key = if retail {
            self.retail_item_refs
                .get(&id)
                .ok_or("unknown_saved_item")?
                .clone()
        } else {
            id.to_string()
        };
        self.raw
            .get("items")
            .and_then(|items| items.get(key))
            .ok_or("unknown_saved_item")
    }
    pub fn validate_item(&self, id: u16) -> Result<(), &'static str> {
        let item = self.item(id)?;
        if item.get("item_type").and_then(Value::as_str).is_none() {
            return Err("reserved_saved_item");
        }
        let binding = item.get("definition").ok_or("unresolved_saved_item")?;
        let group = binding
            .get("group")
            .and_then(Value::as_str)
            .ok_or("unresolved_saved_item")?;
        let name = binding
            .get("name")
            .and_then(Value::as_str)
            .ok_or("unresolved_saved_item")?;
        self.raw
            .get("definitions")
            .and_then(|definitions| definitions.get(group))
            .and_then(|definitions| definitions.get(name))
            .ok_or("unresolved_saved_item")?;
        Ok(())
    }
    /// The Armory proves saved equipment identity/faction eligibility separately.
    /// Every populated slot must also resolve through this raw item catalog,
    /// including items in inventories that are not currently selected.
    pub(crate) fn validate_saved_item_bindings(
        &self,
        saved: &crate::loadouts::SavedLoadouts,
    ) -> Result<(), &'static str> {
        for row in &saved.inventories {
            for &item in row.iter().flatten() {
                self.validate_item(item)?;
            }
        }
        Ok(())
    }
    pub fn primary_magazine_capacity(&self, id: u16) -> Result<u8, &'static str> {
        self.validate_item(id)?;
        let item = self.item(id)?;
        if item["item_type"].as_str() != Some("kWeapon") {
            return Err("saved_primary_not_weapon");
        }
        let binding = &item["definition"];
        let group = binding["group"].as_str().ok_or("unresolved_saved_item")?;
        let name = binding["name"].as_str().ok_or("unresolved_saved_item")?;
        self.raw["definitions"][group][name]["parameters"]["magazine_capacity"]
            .as_u64()
            .and_then(|capacity| u8::try_from(capacity).ok())
            .ok_or("unsupported_saved_primary_magazine")
    }
    pub fn character_blast_rules(&self) -> crate::blast_rules::CharacterBlastRules {
        self.character_blast_rules
    }
    pub fn raw(&self) -> &Value {
        &self.raw
    }
    pub fn bullet(&self, id: u16) -> Result<BulletParameters, &'static str> {
        self.bullets
            .get(&id)
            .copied()
            .ok_or("unsupported_bullet_weapon")
    }
    pub fn vehicle_bullet(&self, id: u16) -> Result<TargetDamageParameters, &'static str> {
        self.vehicle_bullets
            .get(&id)
            .copied()
            .ok_or("unsupported_vehicle_bullet")
    }
    pub fn bullet_attachment(
        &self,
        weapon_id: u16,
        attachment_id: u16,
    ) -> Result<BulletAttachmentKind, &'static str> {
        self.bullet_attachments
            .get(&(weapon_id, attachment_id))
            .copied()
            .ok_or("unsupported_bullet_attachment")
    }
    pub fn armor(&self, id: u16) -> Result<ArmorParameters, &'static str> {
        if let Some(retail) = &self.retail_combat {
            let p = retail
                .armor_for_reduction(id)
                .map_err(|_| "unsupported_retail_armor")?;
            return Ok(ArmorParameters {
                armor_type: p.armor_type,
                rating: p.rating as f32,
                prevention: p.prevention as f32,
                coverage: p.coverage.map(|v| v as f32),
            });
        }
        self.armor.get(&id).copied().ok_or("unsupported_armor_item")
    }
    pub fn body(&self) -> [BodyParameters; 16] {
        if let Some(retail) = &self.retail_combat {
            let mut result = self.body;
            for p in retail.body() {
                result[usize::from(p.location)] = BodyParameters {
                    armor_area: usize::from(p.armor_area),
                    multiplier: p.multiplier as f32,
                };
            }
            return result;
        }
        self.body
    }
    pub fn healing(&self, id: u16) -> Result<HealingParameters, &'static str> {
        if let Some(retail) = &self.retail_combat {
            let p = retail
                .healing_for_schedule(id)
                .map_err(|_| "unsupported_retail_healing_item")?;
            return Ok(HealingParameters {
                cooldown_seconds: p.cooldown_seconds as f32,
                heal_percent: p.heal_percent as f32,
                ticks: p.duration_seconds as u8,
            });
        }
        self.healing
            .get(&id)
            .copied()
            .ok_or("unsupported_healing_item")
    }
    pub fn default_grenade_item(&self) -> Result<u16, &'static str> {
        if let Some(retail) = &self.retail_combat {
            return retail
                .grenades()
                .keys()
                .copied()
                .find(|id| retail.grenade_for_damage(*id).is_ok())
                .ok_or("retail_grenade_authority_unavailable");
        }
        if self.application_id() == 20231 {
            return Err("retail_grenade_authority_unavailable");
        }
        Ok(42)
    }
    pub fn grenade_expiry_grace_ms(&self) -> u64 {
        self.retail_combat
            .as_ref()
            .and_then(|r| r.runtime_policy())
            .map_or(crate::projectile::PROJECTILE_AGE_GRACE_MS, |p| {
                p.projectile_expiry_grace_ms
            })
    }
    pub fn grenade(&self, id: u16) -> Result<GrenadeParameters, &'static str> {
        if let Some(retail) = &self.retail_combat {
            let p = retail
                .grenade_for_damage(id)
                .map_err(|_| "unsupported_retail_grenade_item")?;
            return Ok(GrenadeParameters {
                damage: DamageParameters {
                    character: p.damage.character as f32,
                    armor_pierce: p.damage.character_armor_pierce as f32,
                    armor_redux: p.damage.character_armor_reduction as f32,
                },
                ammo_token: retail
                    .projectile_creation_token(id)
                    .map_err(|_| "retail_grenade_token_unavailable")?,
                full_radius: p.full_radius as f32,
                half_radius: p.half_radius as f32,
                fuse_ms: p.fuse_ms,
                authored_lifetime_ms: p.authored_lifetime_ms,
                initial_count: u8::try_from(p.initial_count)
                    .map_err(|_| "invalid_retail_grenade_count")?,
            });
        }
        self.grenades
            .get(&id)
            .copied()
            .ok_or("unsupported_grenade_item")
    }
}
/// Compatibility constructors load the same disk catalog once. Production uses
/// its explicit startup path and passes that catalog to every runtime resolver.
pub fn bundled() -> Result<&'static Arc<ItemCatalog>, &'static str> {
    static CATALOG: OnceLock<io::Result<Arc<ItemCatalog>>> = OnceLock::new();
    CATALOG
        .get_or_init(|| {
            ItemCatalog::load(
                Path::new(env!("CARGO_MANIFEST_DIR")).join("data/prototype-items.json"),
            )
            .map(Arc::new)
        })
        .as_ref()
        .map_err(|_| "item_catalog_unavailable")
}
