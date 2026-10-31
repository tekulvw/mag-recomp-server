//! Retail slot-owned swipe profiles over the native NetMessageHit wire.
//! Facing uses the retained GetMeleeDamageModifier branches; scalar sin/cos
//! reconstructs the native vector approximation rather than matching its bits.
use crate::{
    armor, damage::DamageKind, replication::CharacterLoadout, retail_combat::RetailCombatCatalog,
};

#[derive(Clone, Copy, Debug)]
pub struct RetailMeleeProfile {
    item_id: u16,
    amount: f32,
    pierce: f32,
    redux: f32,
    effective_range: f32,
    max_range: f32,
    direction_multiplier: f32,
}

impl RetailMeleeProfile {
    #[allow(clippy::too_many_arguments)]
    pub fn from_kit(
        catalog: &RetailCombatCatalog,
        kit: &CharacterLoadout,
        gear: u8,
        mode: u8,
        reach_modifier: f32,
        attacker_yaw: u16,
        defender_yaw: u16,
    ) -> Result<Self, &'static str> {
        if attacker_yaw >= 1024 || defender_yaw >= 1024 {
            return Err("invalid_melee_yaw");
        }
        if !reach_modifier.is_finite() {
            return Err("invalid_melee_reach_modifier");
        }
        let inventory = kit
            .inventories
            .get(usize::from(kit.handle.inventory_index))
            .ok_or("unsupported_inventory_index")?;
        let item_id = inventory
            .slots
            .get(usize::from(gear))
            .copied()
            .flatten()
            .ok_or("missing_melee_weapon")?;
        let parameters = catalog
            .melee_for_damage(item_id)
            .map_err(|_| "unsupported_retail_melee_weapon")?;
        if gear != parameters.gear_slot || mode != parameters.fire_mode {
            return Err("unsupported_melee_gear_or_mode");
        }
        let effective = parameters.effective_range as f32;
        let maximum = parameters.max_range as f32;
        let effective_range = effective.mul_add(reach_modifier, effective);
        let max_range = maximum.mul_add(reach_modifier, maximum);
        if !effective_range.is_finite()
            || !max_range.is_finite()
            || effective_range < 0.0
            || max_range <= 0.0
            || effective_range > max_range
        {
            return Err("invalid_melee_range");
        }
        let rules = catalog
            .melee_direction()
            .map_err(|_| "retail_melee_direction_unavailable")?;
        let facing = |yaw| {
            // Original PackedYawToRadians uses two separately rounded products.
            let normalized = f32::from(yaw) * (1.0_f32 / 1024.0);
            (normalized * std::f32::consts::TAU).sin_cos()
        };
        let (attacker_sin, attacker_cos) = facing(attacker_yaw);
        let (defender_sin, defender_cos) = facing(defender_yaw);
        let dot = attacker_sin * defender_sin + attacker_cos * defender_cos;
        let direction_multiplier = if dot > rules.back_dot_threshold as f32 {
            rules.back_multiplier as f32
        } else if dot <= rules.front_dot_threshold as f32 {
            rules.front_multiplier as f32
        } else {
            rules.side_multiplier as f32
        };
        Ok(Self {
            item_id,
            amount: parameters.damage.character as f32,
            pierce: parameters.damage.character_armor_pierce as f32,
            redux: parameters.damage.character_armor_reduction as f32,
            effective_range,
            max_range,
            direction_multiplier,
        })
    }

    pub fn item_id(self) -> u16 {
        self.item_id
    }

    pub fn raw_character_damage(
        self,
        distance: f32,
        scale: f32,
        location: u8,
    ) -> Result<armor::RawCharacterDamage, &'static str> {
        if !distance.is_finite() || distance < 0.0 {
            return Err("invalid_melee_distance");
        }
        if !scale.is_finite() || !(0.0..=1.0).contains(&scale) {
            return Err("invalid_melee_scale");
        }
        if location > 15 {
            return Err("invalid_damage_location");
        }
        let range = if distance < self.effective_range {
            1.0
        } else if distance >= self.max_range {
            0.0
        } else {
            (-0.5_f32).mul_add(
                (distance - self.effective_range) / (self.max_range - self.effective_range),
                1.0,
            )
        };
        let source = self.amount * scale;
        let factor = range * self.direction_multiplier;
        let amount_hp = source * factor;
        if !amount_hp.is_finite() || amount_hp < 0.0 {
            return Err("invalid_melee_damage");
        }
        Ok(armor::RawCharacterDamage {
            amount_hp,
            armor_rating_redux: self.pierce,
            armor_prevention_redux: self.redux,
            hit_location: location,
        })
    }

    pub fn adjusted_amount(
        self,
        distance: f32,
        scale: f32,
        armor: armor::ArmorProfile,
        location: u8,
    ) -> Result<f32, &'static str> {
        armor.adjust_for_kind(
            self.raw_character_damage(distance, scale, location)?,
            DamageKind::Swipe,
        )
    }
}
