//! Already-resolved ordinary damage. Hit finding/modifiers remain upstream.
//! Native ApplyDamage quantizes remaining HP, not the damage amount.
use crate::replication::CharacterDamage;
use serde::{Deserialize, Serialize};
/// Native damage quantizes the remaining health, rather than rounding damage.
/// Falling prediction and actual application share this exact f32 operation.
pub(crate) fn remaining_health_quarter_points(
    health: u16,
    amount: f32,
) -> Result<u16, &'static str> {
    if health >= 4096 {
        return Err("invalid_damage_health");
    }
    if !amount.is_finite() || amount < 0.0 {
        return Err("invalid_damage_amount");
    }
    Ok((((health as f32 * 0.25 - amount).max(0.0)) * 4.0).trunc() as u16)
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum DamageKind {
    Bullet,
    Explosive,
    Swipe,
    Poison,
    ExternalKinetic,
}
impl DamageKind {
    fn native(self) -> u8 {
        match self {
            Self::Bullet => 1,
            Self::Explosive => 2,
            Self::Swipe => 3,
            Self::Poison => 4,
            Self::ExternalKinetic => 5,
        }
    }
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ResolvedDamage {
    pub amount_hp: f32,
    pub kind: DamageKind,
    pub kinetic_response: bool,
    pub location: u8,
    pub attacker_id: u32,
    pub source_object_id: u32,
    pub user_data: u32,
}
impl ResolvedDamage {
    pub fn validate(self) -> Result<(), &'static str> {
        if !self.amount_hp.is_finite() || self.amount_hp <= 0.0 || self.amount_hp > 1000.0 {
            return Err("invalid_damage_amount");
        }
        if self.location > 15 {
            return Err("invalid_damage_location");
        }
        Ok(())
    }
    pub fn apply(self, health_quarter_points: u16) -> Result<CharacterDamage, &'static str> {
        self.validate()?;
        self.apply_validated(health_quarter_points)
    }
    /// Original falling formula can exceed the ordinary1000HP command limit.
    /// Only the internal state-bound falling path may use this entry point.
    pub(crate) fn apply_falling(
        self,
        health_quarter_points: u16,
    ) -> Result<CharacterDamage, &'static str> {
        if self.kind != DamageKind::ExternalKinetic
            || !self.amount_hp.is_finite()
            || self.amount_hp <= 0.0
            || self.location > 15
        {
            return Err("invalid_falling_damage");
        }
        self.apply_validated(health_quarter_points)
    }
    fn apply_validated(self, health_quarter_points: u16) -> Result<CharacterDamage, &'static str> {
        Ok(CharacterDamage {
            last_location_and_type: ((self.kind.native()
                | if self.kinetic_response { 8 } else { 0 })
                << 4)
                | self.location,
            health_quarter_points: remaining_health_quarter_points(
                health_quarter_points,
                self.amount_hp,
            )?,
            conditions: 0,
            last_attacker: self.attacker_id,
            last_source_object: self.source_object_id,
            last_user_data: self.user_data,
        })
    }
    /// Owned suicide is an explicit terminal lifecycle action. Keep the normal
    /// damage validation/attribution, then force zero HP independently of the
    /// ordinary damage amount limit and the admitted maximum-health bonus.
    pub fn kill_self(
        character_id: u32,
        health_quarter_points: u16,
    ) -> Result<CharacterDamage, &'static str> {
        let mut damage = Self {
            amount_hp: 1000.0,
            kind: DamageKind::Bullet,
            kinetic_response: false,
            location: 0,
            attacker_id: character_id,
            source_object_id: u32::MAX,
            user_data: 0,
        }
        .apply(health_quarter_points)?;
        damage.health_quarter_points = 0;
        Ok(damage)
    }
}

/// Common character-hit authority for reported human hits and server bot shots.
/// Target acquisition and ammunition admission remain the caller's responsibility.
pub(crate) fn resolve_character_hit(
    raw: crate::armor::RawCharacterDamage,
    armor: crate::armor::ArmorProfile,
    kind: DamageKind,
    attacker_id: u32,
    user_data: u32,
    health: u16,
) -> Result<(serde_json::Value, f32, Option<CharacterDamage>), &'static str> {
    let calculation = armor.calculation(raw, kind)?;
    let amount = armor.adjust_for_kind(raw, kind)?;
    let damage = if amount == 0.0 {
        None
    } else {
        Some(
            ResolvedDamage {
                amount_hp: amount,
                kind,
                kinetic_response: false,
                location: raw.hit_location,
                attacker_id,
                source_object_id: u32::MAX,
                user_data,
            }
            .apply(health)?,
        )
    };
    Ok((calculation, amount, damage))
}
