//! Authored prototype Valor bullet profiles, before health quantization.
//!
//! HandleMessageHit 007a06f8 obtains DamageInfo from the current kit's gear slot,
//! multiplies authored character HP by message damageScale, then separately by
//! range falloff (007a0ca4..007a0d94). Between effective/max, the range uses fdivs
//! followed by fmadds with -0.5 and +1 (007a16b8..007a16dc). At max it is zero.
//! Ordinary SimWeaponKit GetWeaponDamage 008471a8 forwards magazine to virtual
//! GetAmmoData 0084724c, which ignores that argument and obtains gear+0x8c.
//! This narrow resolver accepts the observed ordinary pool0 and no untraced
//! magazine/firepoint attachments. Authored weapon-compatible, non-weapon bipods
//! in the primary underbarrel slot retain the base bullet profile; native
//! SetInventory00848930/GetAmmoData0084724c proof lives in bipod-ballistics. Matching hit magazine to observed
//! pool is a conservative development policy, not a claim of native authority.
//! It does not consume ammo, count fired/missed shots, raycast, validate material
//! penetration or infer that a client hit is geometrically valid. The caller
//! supplies an accepted current equipped item and validates hit/life ownership.
//! Retail profiles and unsupported equipment require additional evidence.
use crate::armor::{ArmorProfile, RawCharacterDamage};
use crate::item_catalog::{self, ItemCatalog};
use crate::replication::{CharacterLoadout, InventoryItem};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BallisticProfile {
    item_id: u16,
    ammo_token: u32,
    amount_hp: f32,
    armor_pierce: f32,
    armor_redux: f32,
    effective_range: f32,
    max_range: f32,
}

impl BallisticProfile {
    pub fn from_equipped(
        loadout: &CharacterLoadout,
        equipped: InventoryItem,
        hit_gear_slot: u8,
        hit_magazine: u8,
    ) -> Result<Self, &'static str> {
        Self::from_equipped_with_catalog(
            loadout,
            equipped,
            hit_gear_slot,
            hit_magazine,
            item_catalog::bundled()?,
        )
    }
    pub fn from_equipped_with_catalog(
        loadout: &CharacterLoadout,
        equipped: InventoryItem,
        hit_gear_slot: u8,
        hit_magazine: u8,
        catalog: &ItemCatalog,
    ) -> Result<Self, &'static str> {
        let inventory = loadout
            .inventories
            .get(usize::from(loadout.handle.inventory_index))
            .ok_or("unsupported_inventory_index")?;
        if equipped.inventory_update_id != loadout.handle.update_id {
            return Err("stale_equipped_inventory");
        }
        if equipped.gear_slot != hit_gear_slot {
            return Err("hit_gear_not_equipped");
        }
        // Current authored normal bullet magazine uses pool0. AmmoData resolution
        // does not use the hit magazine to select a different damage profile.
        if equipped.ammo_pool != 0 || hit_magazine != equipped.ammo_pool {
            return Err("unsupported_bullet_magazine");
        }
        let attachment_slots: &[usize] = match hit_gear_slot {
            0 => &[15, 16, 17],
            1 => &[19, 20],
            _ => return Err("unsupported_bullet_weapon"),
        };
        let item_id = inventory.slots[usize::from(hit_gear_slot)].ok_or("missing_bullet_weapon")?;
        for &slot in attachment_slots {
            if let Some(attachment_id) = inventory.slots[slot] {
                match catalog.bullet_attachment(item_id, attachment_id)? {
                    item_catalog::BulletAttachmentKind::Bipod
                        if hit_gear_slot == 0 && slot == 17 => {}
                    _ => return Err("unsupported_bullet_attachment"),
                }
            }
        }
        match (hit_gear_slot, item_id) {
            (0, 13 | 22 | 31) | (1, 0) => (),
            _ => return Err("unsupported_bullet_weapon"),
        }
        let parameters = catalog.bullet(item_id)?;
        Ok(Self {
            item_id,
            ammo_token: parameters.ammo_token,
            amount_hp: parameters.damage.character,
            armor_pierce: parameters.damage.armor_pierce,
            armor_redux: parameters.damage.armor_redux,
            effective_range: parameters.effective_range,
            max_range: parameters.max_range,
        })
    }

    pub fn item_id(self) -> u16 {
        self.item_id
    }
    pub fn ammo_token(self) -> u32 {
        self.ammo_token
    }
    pub fn effective_range(self) -> f32 {
        self.effective_range
    }
    pub fn max_range(self) -> f32 {
        self.max_range
    }

    /// reported_scale is the client's bounded material/hit attenuation, not a
    /// damage amount. The caller must decide when that report is acceptable;
    /// this function only enforces finite0..1 and authors the base amount itself.
    pub fn raw_character_damage(
        self,
        distance: f32,
        reported_scale: f32,
        hit_location: u8,
    ) -> Result<RawCharacterDamage, &'static str> {
        if !distance.is_finite() || distance < 0.0 {
            return Err("invalid_bullet_distance");
        }
        if !reported_scale.is_finite() || !(0.0..=1.0).contains(&reported_scale) {
            return Err("invalid_bullet_scale");
        }
        if hit_location > 15 {
            return Err("invalid_damage_location");
        }
        let falloff = if distance < self.effective_range {
            1.0
        } else if distance >= self.max_range {
            0.0
        } else {
            let interval = self.max_range - self.effective_range;
            let past_effective = distance - self.effective_range;
            let ratio = past_effective / interval;
            (-0.5_f32).mul_add(ratio, 1.0)
        };
        // Native rounds these two products separately; do not reassociate them.
        let scaled_source = self.amount_hp * reported_scale;
        let amount_hp = scaled_source * falloff;
        Ok(RawCharacterDamage {
            amount_hp,
            armor_rating_redux: self.armor_pierce,
            armor_prevention_redux: self.armor_redux,
            hit_location,
        })
    }

    pub fn adjusted_amount(
        self,
        distance: f32,
        reported_scale: f32,
        armor: ArmorProfile,
        hit_location: u8,
    ) -> Result<f32, &'static str> {
        armor.adjust(self.raw_character_damage(distance, reported_scale, hit_location)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_range_ramp_and_separate_source_multiplication_keep_f32_order() {
        // Synthetic unequal ranges exercise the same native formula; all four
        // current authored weapons use 300/300 and cannot exercise this branch.
        let gun = BallisticProfile {
            item_id: 22,
            ammo_token: 0x3c850290,
            amount_hp: 50.0,
            armor_pierce: 0.0,
            armor_redux: 0.0,
            effective_range: 100.0,
            max_range: 300.0,
        };
        assert_eq!(
            gun.raw_character_damage(100.0, 1.0, 3).unwrap().amount_hp,
            50.0
        );
        assert_eq!(
            gun.raw_character_damage(200.0, 1.0, 3).unwrap().amount_hp,
            37.5
        );
        assert_eq!(
            gun.raw_character_damage(299.0, 1.0, 3)
                .unwrap()
                .amount_hp
                .to_bits(),
            0x41c90000
        );
        assert_eq!(
            gun.raw_character_damage(300.0, 1.0, 3).unwrap().amount_hp,
            0.0
        );
        // Re-associating raw*(reportedScale*falloff) would yield0x4147fae1.
        assert_eq!(
            gun.raw_character_damage(200.0, 0.3333, 3)
                .unwrap()
                .amount_hp
                .to_bits(),
            0x4147fae0
        );
    }
}
