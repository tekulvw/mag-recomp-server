//! Opt-in varied bot equipment and appearance (`mag_bot_kits_v1`).
//!
//! A kit catalog is an explicit operator input. Each faction lists concrete
//! kits, each tagged with a squad role, and per-armor apparel pools (uniforms,
//! head apparel and face apparel). The catalog also lists the head and voice
//! identities bots may present. No equipment or identity is invented here: a
//! catalog that is absent leaves every bot on its faction's bootstrap kit with
//! head and voice zero.
//!
//! Selection is deterministic. A bot's head and voice follow from its faction
//! and name slot alone, so a named bot keeps the same face and voice across
//! lives and matches. Its kit and apparel also take the match formation as a
//! salt, so the same bot may carry different equipment in another match. The
//! kit is chosen once when the bot joins the world and kept for every life;
//! the medic and engineer kits follow the squad role slots that the healing
//! and repair policies already assign, and the other roles are drawn by weight
//! with a per-squad cap.
use crate::item_catalog::ItemCatalog;
use crate::replication::{CharacterInventory, CharacterLoadout, InventoryHandle, InventoryItem};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const FORMAT: &str = "mag_bot_kits_v1";
const MAX_BYTES: u64 = 1024 * 1024;
const MAX_KITS_PER_FACTION: usize = 512;
/// Retail character creation offers heads 0..=14 for every faction.
pub const HEAD_COUNT: u8 = 15;
/// Retail character creation offers nine voices (0..=8); higher vocal IDs
/// belong to the faction commanders.
pub const VOICE_COUNT: u8 = 9;

/// Inventory slots, in the client's item slot order.
pub(crate) mod slot {
    pub const PRIMARY: usize = 0;
    pub const BACKUP: usize = 1;
    pub const SWIPE: usize = 2;
    pub const HEAVY: usize = 3;
    pub const GEAR: [usize; 2] = [4, 5];
    pub const APPAREL_HEAD: usize = 10;
    pub const APPAREL_FACE: usize = 11;
    pub const ARMOR: usize = 12;
    pub const UNIFORM: usize = 13;
    pub const PRIMARY_SIGHT: usize = 14;
    pub const PRIMARY_MAGAZINE: usize = 15;
    pub const PRIMARY_MUZZLE: usize = 16;
    pub const PRIMARY_UNDERBARREL: usize = 17;
    pub const BACKUP_SIGHT: usize = 18;
    pub const BACKUP_MAGAZINE: usize = 19;
    pub const BACKUP_MUZZLE: usize = 20;
    pub const HEAVY_SIGHT: usize = 21;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Rifleman,
    Medic,
    Engineer,
    Support,
    Marksman,
    AntiArmor,
    CloseQuarters,
}
impl Role {
    pub fn code(self) -> &'static str {
        match self {
            Self::Rifleman => "rifleman",
            Self::Medic => "medic",
            Self::Engineer => "engineer",
            Self::Support => "support",
            Self::Marksman => "marksman",
            Self::AntiArmor => "anti_armor",
            Self::CloseQuarters => "close_quarters",
        }
    }
    /// Medic and engineer kits follow squad role slots and are never drawn.
    fn assigned(self) -> bool {
        matches!(self, Self::Medic | Self::Engineer)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    format: String,
    application_id: u32,
    provenance: Value,
    heads: Vec<u8>,
    voices: Vec<u8>,
    apparel_head_per_mille: u16,
    apparel_face_per_mille: u16,
    roles: Vec<RoleWeight>,
    factions: Vec<FactionRow>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RoleWeight {
    role: Role,
    weight: u16,
    #[serde(default)]
    max_per_squad: Option<u8>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FactionRow {
    faction: u8,
    kits: Vec<KitRow>,
    apparel: Vec<ApparelRow>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KitRow {
    id: String,
    role: Role,
    items: [Option<u16>; 22],
    #[serde(default)]
    weight: Option<u16>,
    /// Projection audit values; never used at runtime.
    #[serde(default)]
    #[allow(dead_code)]
    cost: Option<u32>,
    #[serde(default)]
    #[allow(dead_code)]
    skill_points: Option<u32>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ApparelRow {
    armor: u16,
    uniforms: Vec<u16>,
    #[serde(default)]
    head: Vec<u16>,
    #[serde(default)]
    face: Vec<u16>,
}

#[derive(Clone, Debug)]
struct Kit {
    id: String,
    role: Role,
    items: [Option<u16>; 22],
    weight: u16,
}
#[derive(Clone, Debug)]
struct Apparel {
    uniforms: Vec<u16>,
    head: Vec<u16>,
    face: Vec<u16>,
}
#[derive(Clone, Debug)]
struct Faction {
    kits: Vec<Kit>,
    apparel: BTreeMap<u16, Apparel>,
}

/// A validated-shape kit catalog; `validate_items` checks it against the
/// worker's item catalog before any bot uses it.
#[derive(Clone, Debug)]
pub struct Catalog {
    sha256: String,
    heads: Vec<u8>,
    voices: Vec<u8>,
    head_per_mille: u16,
    face_per_mille: u16,
    roles: Vec<RoleWeight>,
    factions: BTreeMap<u8, Faction>,
}

/// Who is asking for a kit.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Request<'a> {
    pub faction: u8,
    pub name_index: u16,
    /// Varies kits between matches; empty outside a selected match.
    pub salt: &'a str,
    /// Medic or engineer when the bot's squad role slot carries that duty.
    pub assigned: Option<Role>,
    /// Drawn roles already present in the bot's squad.
    pub squad_roles: &'a [Role],
}
#[derive(Clone, Debug)]
pub(crate) struct Chosen {
    pub kit_id: String,
    pub role: Role,
    pub loadout: CharacterLoadout,
    pub head: u8,
    pub voice: u8,
}

fn invalid(code: &'static str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, code)
}

impl Catalog {
    pub fn load(path: &std::path::Path) -> std::io::Result<Self> {
        use std::io::Read;
        let mut bytes = Vec::new();
        std::fs::File::open(path)?
            .take(MAX_BYTES + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err(invalid("bot_kits_file_limit"));
        }
        Self::from_json(&bytes)
    }
    pub fn from_json(bytes: &[u8]) -> std::io::Result<Self> {
        let file: File = serde_json::from_slice(bytes).map_err(|_| invalid("invalid_bot_kits"))?;
        let sha256 = format!("{:x}", Sha256::digest(bytes));
        Self::from_file(file, sha256).map_err(invalid)
    }
    fn from_file(file: File, sha256: String) -> Result<Self, &'static str> {
        if file.format != FORMAT || file.application_id != 20231 || !file.provenance.is_object() {
            return Err("invalid_bot_kits");
        }
        let unique = |values: &[u8], limit: u8| {
            !values.is_empty()
                && values.iter().all(|&v| v < limit)
                && values.iter().collect::<BTreeSet<_>>().len() == values.len()
        };
        if !unique(&file.heads, HEAD_COUNT) {
            return Err("invalid_bot_kit_heads");
        }
        if !unique(&file.voices, VOICE_COUNT) {
            return Err("invalid_bot_kit_voices");
        }
        if file.apparel_head_per_mille > 1000 || file.apparel_face_per_mille > 1000 {
            return Err("invalid_bot_kit_apparel_chance");
        }
        let mut drawn = BTreeSet::new();
        for row in &file.roles {
            if row.role.assigned()
                || !(1..=1000).contains(&row.weight)
                || row.max_per_squad.is_some_and(|cap| !(1..=8).contains(&cap))
                || !drawn.insert(row.role)
            {
                return Err("invalid_bot_kit_role_weights");
            }
        }
        if drawn.is_empty() {
            return Err("invalid_bot_kit_role_weights");
        }
        let mut factions = BTreeMap::new();
        for row in file.factions {
            if row.faction > 2 || factions.contains_key(&row.faction) {
                return Err("invalid_bot_kit_faction");
            }
            let mut apparel = BTreeMap::new();
            for a in row.apparel {
                let lists = [&a.uniforms, &a.head, &a.face];
                if a.armor >= 4096
                    || a.uniforms.is_empty()
                    || lists
                        .iter()
                        .any(|l| l.len() > 256 || l.iter().any(|&id| id >= 4096))
                    || apparel
                        .insert(
                            a.armor,
                            Apparel {
                                uniforms: a.uniforms,
                                head: a.head,
                                face: a.face,
                            },
                        )
                        .is_some()
                {
                    return Err("invalid_bot_kit_apparel");
                }
            }
            if row.kits.is_empty() || row.kits.len() > MAX_KITS_PER_FACTION {
                return Err("invalid_bot_kit_count");
            }
            let mut ids = BTreeSet::new();
            let mut kits = Vec::with_capacity(row.kits.len());
            for k in row.kits {
                let weight = k.weight.unwrap_or(1);
                if k.id.is_empty()
                    || k.id.len() > 64
                    || !k.id.bytes().all(|b| b.is_ascii_graphic())
                    || !ids.insert(k.id.clone())
                    || !(1..=1000).contains(&weight)
                {
                    return Err("invalid_bot_kit_id");
                }
                if k.items.iter().flatten().any(|&id| id >= 4096)
                    || [slot::PRIMARY, slot::BACKUP, slot::SWIPE, slot::ARMOR]
                        .iter()
                        .any(|&s| k.items[s].is_none())
                {
                    return Err("invalid_bot_kit_items");
                }
                // Apparel comes from the armor's pools; magazines are unused.
                if [
                    slot::APPAREL_HEAD,
                    slot::APPAREL_FACE,
                    slot::UNIFORM,
                    slot::PRIMARY_MAGAZINE,
                    slot::BACKUP_MAGAZINE,
                ]
                .iter()
                .any(|&s| k.items[s].is_some())
                {
                    return Err("invalid_bot_kit_reserved_slot");
                }
                if !apparel.contains_key(&k.items[slot::ARMOR].unwrap()) {
                    return Err("bot_kit_armor_without_apparel");
                }
                kits.push(Kit {
                    id: k.id,
                    role: k.role,
                    items: k.items,
                    weight,
                });
            }
            let has = |role: Role| kits.iter().any(|k| k.role == role);
            if !has(Role::Medic) || !has(Role::Engineer) || !drawn.iter().any(|&role| has(role)) {
                return Err("bot_kit_faction_missing_roles");
            }
            factions.insert(row.faction, Faction { kits, apparel });
        }
        if factions.is_empty() {
            return Err("invalid_bot_kit_faction");
        }
        Ok(Self {
            sha256,
            heads: file.heads,
            voices: file.voices,
            head_per_mille: file.apparel_head_per_mille,
            face_per_mille: file.apparel_face_per_mille,
            roles: file.roles,
            factions,
        })
    }
    pub fn sha256(&self) -> &str {
        &self.sha256
    }
    pub fn kit_count(&self) -> usize {
        self.factions.values().map(|f| f.kits.len()).sum()
    }
    pub fn covers(&self, faction: u8) -> bool {
        self.factions.contains_key(&faction)
    }

    /// Check every kit, and every apparel item it may wear, against the item
    /// catalog, the Armory and the retail combat rules bots rely on.
    pub fn validate_items(&self, items: &ItemCatalog) -> Result<(), &'static str> {
        let armory = items
            .retail_armory()
            .ok_or("bot_kits_require_retail_armory")?;
        let team = crate::bot_healing::team_item(items).map(|t| t.id);
        for (&faction, f) in &self.factions {
            for (&armor, apparel) in &f.apparel {
                let armor_name = definition(items, armory, faction, armor, "kArmorItem")?["id"]
                    .as_str()
                    .ok_or("bot_kit_armor_unresolved")?
                    .to_owned();
                for &uniform in &apparel.uniforms {
                    let p = definition(items, armory, faction, uniform, "kUniformItem")?;
                    if p["armor_dependency"].as_str() != Some(armor_name.as_str()) {
                        return Err("bot_kit_uniform_armor_mismatch");
                    }
                }
                for (list, kind) in [
                    (&apparel.head, "kCharAttachTypeApparelHead"),
                    (&apparel.face, "kCharAttachTypeApparelFace"),
                ] {
                    for &id in list {
                        let p = definition(items, armory, faction, id, "kSimpleItem")?;
                        let fits = p["simpleitem_type"].as_str() == Some("kSimpleItemApparel")
                            && p["character_attachment_type"].as_str() == Some(kind)
                            && p["valid_armor_list"].as_array().is_some_and(|list| {
                                list.iter()
                                    .any(|row| row["name"].as_str() == Some(armor_name.as_str()))
                            });
                        if !fits {
                            return Err("bot_kit_apparel_armor_mismatch");
                        }
                    }
                }
            }
            for kit in &f.kits {
                validate_kit(items, armory, faction, kit, team)?;
            }
        }
        Ok(())
    }

    /// The kit, apparel, head and voice for one bot.
    pub(crate) fn choose(
        &self,
        items: &ItemCatalog,
        request: Request<'_>,
    ) -> Result<Chosen, &'static str> {
        let faction = self
            .factions
            .get(&request.faction)
            .ok_or("bot_kits_missing_faction")?;
        let mut look = Rng::new(&[
            b"appearance",
            &[request.faction],
            &request.name_index.to_le_bytes(),
        ]);
        let head = self.heads[look.below(self.heads.len())];
        let voice = self.voices[look.below(self.voices.len())];
        let role = match request.assigned {
            Some(role) => role,
            None => {
                let mut rng = Rng::new(&[
                    b"role",
                    request.salt.as_bytes(),
                    &[request.faction],
                    &request.name_index.to_le_bytes(),
                ]);
                let present = |role| faction.kits.iter().any(|k| k.role == role);
                let open: Vec<_> = self
                    .roles
                    .iter()
                    .filter(|r| present(r.role))
                    .filter(|r| {
                        r.max_per_squad.is_none_or(|cap| {
                            request.squad_roles.iter().filter(|&&s| s == r.role).count()
                                < usize::from(cap)
                        })
                    })
                    .collect();
                let pool: Vec<_> = if open.is_empty() {
                    self.roles.iter().filter(|r| present(r.role)).collect()
                } else {
                    open
                };
                let weights: Vec<_> = pool.iter().map(|r| u64::from(r.weight)).collect();
                pool[rng.weighted(&weights)].role
            }
        };
        let candidates: Vec<_> = faction.kits.iter().filter(|k| k.role == role).collect();
        if candidates.is_empty() {
            return Err("bot_kits_missing_role");
        }
        let mut rng = Rng::new(&[
            b"kit",
            request.salt.as_bytes(),
            &[request.faction],
            &request.name_index.to_le_bytes(),
            role.code().as_bytes(),
        ]);
        let weights: Vec<_> = candidates.iter().map(|k| u64::from(k.weight)).collect();
        let kit = candidates[rng.weighted(&weights)];
        let mut row = kit.items;
        let apparel = &faction.apparel[&row[slot::ARMOR].unwrap()];
        row[slot::UNIFORM] = Some(apparel.uniforms[rng.below(apparel.uniforms.len())]);
        if !apparel.head.is_empty() && rng.per_mille(self.head_per_mille) {
            row[slot::APPAREL_HEAD] = Some(apparel.head[rng.below(apparel.head.len())]);
        }
        if !apparel.face.is_empty() && rng.per_mille(self.face_per_mille) {
            row[slot::APPAREL_FACE] = Some(apparel.face[rng.below(apparel.face.len())]);
        }
        Ok(Chosen {
            kit_id: kit.id.clone(),
            role,
            loadout: loadout(items, row)?,
            head,
            voice,
        })
    }
}

/// One inventory of a single kit; the other four are empty.
fn loadout(items: &ItemCatalog, row: [Option<u16>; 22]) -> Result<CharacterLoadout, &'static str> {
    let primary = row[slot::PRIMARY].ok_or("bot_kit_missing_primary")?;
    let ammo_count = items.primary_magazine_capacity(primary)?;
    Ok(CharacterLoadout {
        inventories: std::array::from_fn(|i| CharacterInventory {
            update_id: 1,
            slots: if i == 0 { row } else { [None; 22] },
        }),
        handle: InventoryHandle {
            update_id: 1,
            inventory_index: 0,
        },
        primary: InventoryItem {
            inventory_update_id: 1,
            gear_slot: 0,
            ammo_pool: 0,
            ammo_count,
            reloading: false,
        },
        primary_equipped: true,
    })
}

/// The definition parameters of an item the Armory offers this faction, of
/// the expected item type.
fn definition<'a>(
    items: &'a ItemCatalog,
    armory: &crate::retail_armory::RetailArmory,
    faction: u8,
    id: u16,
    item_type: &str,
) -> Result<&'a Value, &'static str> {
    if !armory.offers(faction, id) {
        return Err("bot_kit_item_not_offered");
    }
    let (kind, _, parameters) = items.item_definition(id).ok_or("bot_kit_item_unresolved")?;
    if kind != item_type {
        return Err("bot_kit_item_wrong_type");
    }
    Ok(parameters)
}

fn validate_kit(
    items: &ItemCatalog,
    armory: &crate::retail_armory::RetailArmory,
    faction: u8,
    kit: &Kit,
    team: Option<u16>,
) -> Result<(), &'static str> {
    use crate::item_catalog::BulletAttachmentKind as Kind;
    let row = &kit.items;
    let get = |s: usize| row[s];
    let primary = get(slot::PRIMARY).unwrap();
    let backup = get(slot::BACKUP).unwrap();
    for id in row.iter().flatten() {
        if !armory.offers(faction, *id) {
            return Err("bot_kit_item_not_offered");
        }
        items.validate_item(*id)?;
    }
    items.retail_firearm_gear_role(0, primary)?;
    items.retail_firearm_gear_role(1, backup)?;
    let knife = definition(items, armory, faction, get(slot::SWIPE).unwrap(), "kWeapon")?;
    if knife["category"].as_str() != Some("kWepCategoryKnife") {
        return Err("bot_kit_swipe_not_knife");
    }
    definition(
        items,
        armory,
        faction,
        get(slot::ARMOR).unwrap(),
        "kArmorItem",
    )?;
    for (host, gear, sight, muzzle, underbarrel) in [
        (
            primary,
            0,
            slot::PRIMARY_SIGHT,
            slot::PRIMARY_MUZZLE,
            Some(slot::PRIMARY_UNDERBARREL),
        ),
        (backup, 1, slot::BACKUP_SIGHT, slot::BACKUP_MUZZLE, None),
    ] {
        let sight = get(sight).ok_or("bot_kit_missing_sight")?;
        if items.retail_firearm_attachment(gear, host, sight)? != Kind::Sight {
            return Err("bot_kit_sight_slot");
        }
        if let Some(id) = get(muzzle)
            && items.retail_firearm_attachment(gear, host, id)? != Kind::Suppressor
        {
            return Err("bot_kit_muzzle_slot");
        }
        if let Some(id) = underbarrel.and_then(get)
            && !matches!(
                items.retail_firearm_attachment(gear, host, id)?,
                Kind::Bipod | Kind::ForwardGrip | Kind::Launcher
            )
        {
            return Err("bot_kit_underbarrel_slot");
        }
    }
    match (get(slot::HEAVY), get(slot::HEAVY_SIGHT)) {
        (None, None) => {}
        (Some(heavy), Some(sight)) => {
            let weapon = definition(items, armory, faction, heavy, "kWeapon")?;
            let sight = definition(items, armory, faction, sight, "kAttachmentItem")?;
            if weapon["category"].as_str() != Some("kWepCategoryRocketLauncher")
                || weapon["required_sight_attachment"].as_str().is_none()
                || weapon["required_sight_attachment"] != sight["id"]
            {
                return Err("bot_kit_heavy_slot");
            }
        }
        _ => return Err("bot_kit_heavy_slot"),
    }
    let mut heals = false;
    for gear in slot::GEAR {
        let Some(id) = get(gear) else { continue };
        let (_, _, p) = items.item_definition(id).ok_or("bot_kit_item_unresolved")?;
        if p["menu_selection_slot"].as_str() != Some("kSelectionMenuSlotEquipment") {
            return Err("bot_kit_gear_slot");
        }
        heals |= items.healing(id).is_ok();
    }
    if !heals {
        return Err("bot_kit_without_healing");
    }
    let medkit = team.is_some_and(|t| row.contains(&Some(t)));
    let repair = row
        .iter()
        .flatten()
        .any(|&id| items.is_retail_repair_tool(id));
    if medkit != (kit.role == Role::Medic) {
        return Err("bot_kit_medic_item_role");
    }
    if repair != (kit.role == Role::Engineer) {
        return Err("bot_kit_repair_item_role");
    }
    let kit = loadout(items, *row)?;
    crate::retail_combat::RetailBulletProfile::from_equipped_with_items(
        items,
        &kit,
        kit.primary,
        0,
        0,
        true,
    )?;
    crate::bot_firearms::validate_weapon_recoil(items, &kit)?;
    crate::armor::ArmorProfile::from_retail_loadout(&kit, items.retail_combat()?)?;
    Ok(())
}

/// SplitMix64 seeded from a domain-separated SHA-256 digest.
struct Rng(u64);
impl Rng {
    fn new(parts: &[&[u8]]) -> Self {
        let mut h = Sha256::new();
        h.update(b"mag-bot-kits-v1");
        for part in parts {
            h.update((part.len() as u32).to_le_bytes());
            h.update(part);
        }
        Self(u64::from_le_bytes(h.finalize()[..8].try_into().unwrap()))
    }
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: usize) -> usize {
        ((u128::from(self.next()) * n as u128) >> 64) as usize
    }
    fn per_mille(&mut self, chance: u16) -> bool {
        self.below(1000) < usize::from(chance)
    }
    fn weighted(&mut self, weights: &[u64]) -> usize {
        let total: u64 = weights.iter().sum();
        let mut pick = self.below(total as usize) as u64;
        for (i, &w) in weights.iter().enumerate() {
            if pick < w {
                return i;
            }
            pick -= w;
        }
        weights.len() - 1
    }
}

#[cfg(test)]
#[path = "bot_kits_tests.rs"]
pub(crate) mod tests;
