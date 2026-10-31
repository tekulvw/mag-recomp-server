//! Prototype armor and unobstructed grenade radius scalar, before HP quantization.
//!
//! separate f32 arithmetic at 003d9650..003d9718), 008fdf48.c (inventory slot12,
//! 007692f0.asm.txt (f32 division followed by vmaddfp interpolation).
//! Authored resources under the prototype's scripts directory:
//! damagemodel.lua SHA256 5d876822c4866afb271022dd6fa3d4b6473b0d87266f26a16a28cb44d426eba9;
//! armor_items.lua SHA256 22e0f478cd5e03d254a9c817d05aafd121c87d77c025500f0921442a8a7f1040;
//! collision_info.lua SHA256 9189738c2a33267bd15a0848dc856b8b9a939efecd4e4cc07ad2da04b7005e5d.
//! This supports ordinary living-character damage only; no incap/revive. Owned incoming resistance is applied before armor.
//! Radius distance is an input from an upstream hit resolver: this module does
//! not find targets, simulate bounce, or establish visibility/penetration.
//! Already-resolved development damage must not be sent through this module.
use crate::item_catalog::{self, ArmorParameters, BodyParameters, GrenadeParameters, ItemCatalog};
use crate::replication::CharacterLoadout;
use serde::{Deserialize, Serialize};

/// Authored armor and body rows copied from the configured startup catalog.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ArmorProfile {
    parameters: ArmorParameters,
    body: [BodyParameters; 16],
    damage_taken: f32,
    explosive_taken: f32,
    impact_taken: f32,
    chemical_taken: f32,
}

/// A raw source amount with the two native armor penetration inputs. Location
/// is a body index (0..15), not an armor-area index. Zero damage is a valid result.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RawCharacterDamage {
    pub amount_hp: f32,
    pub armor_rating_redux: f32,
    pub armor_prevention_redux: f32,
    pub hit_location: u8,
}

impl ArmorProfile {
    pub fn from_retail_loadout(
        loadout: &CharacterLoadout,
        catalog: &crate::retail_combat::RetailCombatCatalog,
    ) -> Result<Self, &'static str> {
        let inventory = loadout
            .inventories
            .get(usize::from(loadout.handle.inventory_index))
            .ok_or("unsupported_inventory_index")?;
        let item = inventory.slots[12].ok_or("missing_armor_item")?;
        let armor = catalog
            .armor_for_reduction(item)
            .map_err(|_| "unsupported_retail_armor")?;
        let mut body = [BodyParameters {
            armor_area: 0,
            multiplier: 0.0,
        }; 16];
        for row in catalog.body() {
            body[usize::from(row.location)] = BodyParameters {
                armor_area: usize::from(row.armor_area),
                multiplier: row.multiplier as f32,
            };
        }
        Ok(Self {
            parameters: ArmorParameters {
                armor_type: armor.armor_type,
                rating: armor.rating as f32,
                prevention: armor.prevention as f32,
                coverage: armor.coverage.map(|v| v as f32),
            },
            body,
            damage_taken: 0.0,
            explosive_taken: 0.0,
            impact_taken: 0.0,
            chemical_taken: 0.0,
        })
    }
    pub fn from_item_id(item_id: u16) -> Result<Self, &'static str> {
        Self::from_item_id_with_catalog(item_id, item_catalog::bundled()?)
    }
    pub fn from_item_id_with_catalog(
        item_id: u16,
        catalog: &ItemCatalog,
    ) -> Result<Self, &'static str> {
        Ok(Self {
            parameters: catalog.armor(item_id)?,
            body: catalog.body(),
            damage_taken: 0.0,
            explosive_taken: 0.0,
            impact_taken: 0.0,
            chemical_taken: 0.0,
        })
    }
    pub fn from_loadout(loadout: &CharacterLoadout) -> Result<Self, &'static str> {
        Self::from_loadout_with_catalog(loadout, item_catalog::bundled()?)
    }
    pub fn from_loadout_with_catalog(
        loadout: &CharacterLoadout,
        catalog: &ItemCatalog,
    ) -> Result<Self, &'static str> {
        if let Ok(retail) = catalog.retail_combat() {
            return Self::from_retail_loadout(loadout, retail);
        }
        let inventory = loadout
            .inventories
            .get(usize::from(loadout.handle.inventory_index))
            .ok_or("unsupported_inventory_index")?;
        Self::from_item_id_with_catalog(inventory.slots[12].ok_or("missing_armor_item")?, catalog)
    }
    /// Public value is the authored one-based item type. Native stores type-1.
    pub fn armor_type(self) -> u8 {
        self.parameters.armor_type
    }

    pub fn with_modifiers(mut self, modifiers: &crate::skill_catalog::OwnedModifiers) -> Self {
        self.damage_taken = modifiers.value("kCharModDamageTaken");
        self.explosive_taken = modifiers.value("kCharModExplosiveDamageTaken");
        self.impact_taken = modifiers.value("kCharModImpactDamageTaken");
        self.chemical_taken = modifiers.value("kCharModChemicalDamageTaken");
        self
    }
    pub fn incoming_scale(self, kind: crate::damage::DamageKind) -> f32 {
        let resistance = match kind {
            crate::damage::DamageKind::Explosive => self.explosive_taken,
            crate::damage::DamageKind::ExternalKinetic => self.impact_taken,
            crate::damage::DamageKind::Poison => self.chemical_taken,
            _ => 0.0,
        };
        ((1.0_f32 + self.damage_taken) + resistance).max(0.0)
    }
    /// Recorded arithmetic inputs; no damage is applied by this audit method.
    pub fn calculation(
        self,
        raw: RawCharacterDamage,
        kind: crate::damage::DamageKind,
    ) -> Result<serde_json::Value, &'static str> {
        let amount = self.adjust_for_kind(raw, kind)?;
        let body = self.body[usize::from(raw.hit_location)];
        let scale = self.incoming_scale(kind);
        Ok(
            serde_json::json!({"raw_amount_hp":raw.amount_hp,"incoming_damage_scale":scale,
            "after_owned_modifier_hp":raw.amount_hp*scale,"armor_type":self.armor_type(),
            "armor_rating":self.parameters.rating,"armor_rating_reduction":raw.armor_rating_redux,
            "armor_prevention":self.parameters.prevention,"armor_prevention_reduction":raw.armor_prevention_redux,
            "coverage":self.parameters.coverage[body.armor_area],"body_multiplier":body.multiplier,
            "after_armor_hp":amount}),
        )
    }
    pub fn adjust(self, raw: RawCharacterDamage) -> Result<f32, &'static str> {
        self.adjust_for_kind(raw, crate::damage::DamageKind::Bullet)
    }
    /// Native SimCharacter::ApplyDamage multiplies incoming raw damage before
    /// MagDamageModel::AdjustDamage, including before armor rating subtraction.
    pub fn adjust_for_kind(
        self,
        raw: RawCharacterDamage,
        kind: crate::damage::DamageKind,
    ) -> Result<f32, &'static str> {
        let body = self
            .body
            .get(usize::from(raw.hit_location))
            .ok_or("invalid_damage_location")?;
        if !raw.amount_hp.is_finite() || raw.amount_hp < 0.0 {
            return Err("invalid_raw_damage_amount");
        }
        if !raw.armor_rating_redux.is_finite()
            || !raw.armor_prevention_redux.is_finite()
            || raw.armor_rating_redux < 0.0
            || raw.armor_prevention_redux < 0.0
        {
            return Err("invalid_armor_reduction");
        }
        // Keep native operation order with the configured rating/prevention: each subtraction/product rounds as f32, with no FMA.
        let scale = self.incoming_scale(kind);
        let incoming = raw.amount_hp * scale;
        let rating = (self.parameters.rating - raw.armor_rating_redux).max(0.0);
        let after_rating = (incoming - rating).max(0.0);
        let prevention = (self.parameters.prevention - raw.armor_prevention_redux).max(0.0);
        let coverage = self.parameters.coverage[body.armor_area].max(0.0);
        let coverage_product = prevention * coverage;
        let armor_scale = 1.0_f32 - coverage_product;
        let body_scale = body.multiplier * armor_scale;
        let result = (after_rating * body_scale).max(0.0);
        if !result.is_finite() {
            return Err("invalid_adjusted_damage_amount");
        }
        Ok(result)
    }
}

/// Verified Valor frag item42 only. Retained valor-frag-ammo.lua authors raw200,
/// radius9/half-radius5, APierce0/ARedux.3. Native DoDamage109d748 supplies missing
/// half-scale.5; full-scale defaults0. No other grenade kind is implied.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GrenadeProfile {
    parameters: GrenadeParameters,
    character_blast_rules: crate::blast_rules::CharacterBlastRules,
}
impl GrenadeProfile {
    pub fn from_item_id(item_id: u16) -> Result<Self, &'static str> {
        Self::from_item_id_with_catalog(item_id, item_catalog::bundled()?)
    }
    pub fn from_item_id_with_catalog(
        item_id: u16,
        catalog: &ItemCatalog,
    ) -> Result<Self, &'static str> {
        Ok(Self {
            parameters: catalog.grenade(item_id)?,
            character_blast_rules: catalog.character_blast_rules(),
        })
    }

    pub fn raw_amount(self, distance: f32) -> Result<f32, &'static str> {
        if !distance.is_finite() || distance < 0.0 {
            return Err("invalid_explosion_distance");
        }
        let scale = if distance <= self.parameters.half_radius {
            // Native lerp(1,.5,d/5) uses vmaddfp after fdivs.
            (-0.5_f32).mul_add(distance / self.parameters.half_radius, 1.0)
        } else if distance < self.parameters.full_radius {
            (-0.5_f32).mul_add(
                (distance - self.parameters.half_radius)
                    / (self.parameters.full_radius - self.parameters.half_radius),
                0.5,
            )
        } else {
            0.0
        };
        Ok(self.parameters.damage.character * scale)
    }

    pub fn native_damage_type(self) -> u8 {
        self.character_blast_rules.damage_type
    }
    /// The original callback overrides authored armor penetration and body location.
    pub fn native_character_damage(
        self,
        distance: f32,
    ) -> Result<RawCharacterDamage, &'static str> {
        Ok(RawCharacterDamage {
            amount_hp: self.raw_amount(distance)?,
            armor_rating_redux: self.character_blast_rules.armor_rating_reduction,
            armor_prevention_redux: self.character_blast_rules.armor_prevention_reduction,
            hit_location: self.character_blast_rules.hit_location,
        })
    }
    /// Low-level authored profile for comparisons; the live blast resolver uses
    /// native_character_damage, never these authored penetration parameters.
    pub fn raw_character_damage(
        self,
        distance: f32,
        hit_location: u8,
    ) -> Result<RawCharacterDamage, &'static str> {
        Ok(RawCharacterDamage {
            amount_hp: self.raw_amount(distance)?,
            armor_rating_redux: self.parameters.damage.armor_pierce,
            armor_prevention_redux: self.parameters.damage.armor_redux,
            hit_location,
        })
    }
    pub fn adjusted_amount(
        self,
        distance: f32,
        armor: ArmorProfile,
        hit_location: u8,
    ) -> Result<f32, &'static str> {
        armor.adjust_for_kind(
            RawCharacterDamage {
                amount_hp: self.raw_amount(distance)?,
                armor_rating_redux: self.parameters.damage.armor_pierce,
                armor_prevention_redux: self.parameters.damage.armor_redux,
                hit_location,
            },
            crate::damage::DamageKind::Explosive,
        )
    }
}
