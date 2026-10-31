//! Authored first-aid and medkit healing with owned character modifiers.
//!
//! Native MagSimCharacter::Update accumulates time and applies one healing tick
//! only when the accumulator is strictly greater than one second, then expires
//! the effect after its authored duration. Its frame cadence can lose ticks.
//! This server instead guarantees the authored three/eight one-second ticks,
//! with bounded catch-up when publication is delayed. Each tick still uses the
//! native quarter-HP truncation and self-heal maximum-HP scaling. Retail uses
//! this same explicit server timing policy; authored durations alone do not prove
//! the retail native tick schedule. Recipient IDs remain client-reported inputs
//! admitted by the event handler, without a claimed medkit sphere/range test.
use crate::item_catalog::{self, HealingParameters, ItemCatalog};
use crate::replication::{CharacterDamage, CharacterLoadout};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HealItem {
    FirstAidPack,
    Medkit,
    /// Actual configured retail item ID; no prototype ordinal translation.
    Retail(u16),
}
impl HealItem {
    /// Resolve the requested gear slot in the selected inventory. Item ordinals
    /// are from the bundled ItemListData, rather than client-supplied amounts.
    pub fn from_loadout(loadout: &CharacterLoadout, slot: u8) -> Result<Self, &'static str> {
        let inventory = loadout
            .inventories
            .get(usize::from(loadout.handle.inventory_index))
            .ok_or("unsupported_healing_inventory")?;
        match inventory.slots.get(usize::from(slot)).copied().flatten() {
            Some(125) => Ok(Self::FirstAidPack),
            Some(98) => Ok(Self::Medkit),
            _ => Err("unsupported_healing_item"),
        }
    }
    /// Resolve the selected inventory slot through the active startup catalog.
    /// The catalog chooses the supported application and healing parameters.
    pub fn from_loadout_with_catalog(
        loadout: &CharacterLoadout,
        slot: u8,
        catalog: &ItemCatalog,
    ) -> Result<Self, &'static str> {
        let inventory = loadout
            .inventories
            .get(usize::from(loadout.handle.inventory_index))
            .ok_or("unsupported_healing_inventory")?;
        let id = inventory
            .slots
            .get(usize::from(slot))
            .copied()
            .flatten()
            .ok_or("unsupported_healing_item")?;
        catalog.healing(id)?;
        Ok(match id {
            125 => Self::FirstAidPack,
            98 => Self::Medkit,
            id => Self::Retail(id),
        })
    }
    pub fn item_id(self) -> u16 {
        match self {
            Self::FirstAidPack => 125,
            Self::Medkit => 98,
            Self::Retail(id) => id,
        }
    }
    pub fn cooldown(self) -> Duration {
        self.cooldown_with_catalog(
            item_catalog::bundled().expect("validated disk item catalog required"),
        )
        .expect("supported healing item")
    }
    pub fn cooldown_with_catalog(self, catalog: &ItemCatalog) -> Result<Duration, &'static str> {
        // Authored cooldown enforced as local admission policy.
        Duration::try_from_secs_f32(catalog.healing(self.item_id())?.cooldown_seconds)
            .map_err(|_| "invalid_healing_cooldown")
    }
}

#[derive(Clone, Copy, Debug)]
pub struct HealingEffect {
    pub generation: u32,
    parameters: HealingParameters,
    started: Duration,
    max_health_hp: f32,
    healer_max_health_hp: f32,
    heal_amount_modifier: f32,
    max_health_quarter_points: u16,
    tick_amount_hp: f32,
    completed_ticks: u8,
}
pub type HealingAdvance = (CharacterDamage, Option<HealingEffect>, Vec<(u8, u32)>);
impl HealingEffect {
    pub fn new(item: HealItem, generation: u32, now: Duration) -> Self {
        Self::new_with_catalog(
            item,
            generation,
            now,
            item_catalog::bundled().expect("validated disk item catalog required"),
        )
        .expect("supported healing item")
    }
    pub fn new_with_catalog(
        item: HealItem,
        generation: u32,
        now: Duration,
        catalog: &ItemCatalog,
    ) -> Result<Self, &'static str> {
        Self::new_with_modifiers(
            item,
            generation,
            now,
            catalog,
            &crate::skill_catalog::OwnedModifiers::default(),
        )
    }
    pub fn new_with_modifiers(
        item: HealItem,
        generation: u32,
        now: Duration,
        catalog: &ItemCatalog,
        modifiers: &crate::skill_catalog::OwnedModifiers,
    ) -> Result<Self, &'static str> {
        Self::new_for_recipient(item, generation, now, catalog, modifiers, modifiers)
    }
    pub fn new_for_recipient(
        item: HealItem,
        generation: u32,
        now: Duration,
        catalog: &ItemCatalog,
        healer: &crate::skill_catalog::OwnedModifiers,
        recipient: &crate::skill_catalog::OwnedModifiers,
    ) -> Result<Self, &'static str> {
        let parameters = catalog.healing(item.item_id())?;
        let max_health_hp = recipient.max_health_hp();
        let max_health_quarter_points = recipient.max_health_quarter_points()?;
        let healer_max_health_hp = healer.max_health_hp();
        let heal_amount_modifier = healer.value("kCharModHealAmount");
        // The native receiver uses healer bonus/maxHP, then each recipient's
        // Heal scales the stored tick by that recipient's maxHP/100.
        let base_percent = parameters.heal_percent / 100.0;
        let percent = base_percent.mul_add(heal_amount_modifier, base_percent);
        let stored_tick = (percent * healer_max_health_hp) / f32::from(parameters.ticks);
        let tick_amount_hp = max_health_hp * (stored_tick / 100.0);
        if !tick_amount_hp.is_finite() || tick_amount_hp < 0.0 {
            return Err("invalid_owned_healing_amount");
        }
        Ok(Self {
            generation,
            parameters,
            max_health_hp,
            healer_max_health_hp,
            heal_amount_modifier,
            max_health_quarter_points,
            tick_amount_hp,
            started: now,
            completed_ticks: 0,
        })
    }
    pub fn healer_max_health_hp(&self) -> f32 {
        self.healer_max_health_hp
    }
    pub fn recipient_max_health_hp(&self) -> f32 {
        self.max_health_hp
    }
    pub fn heal_amount_modifier(&self) -> f32 {
        self.heal_amount_modifier
    }
    pub fn tick_amount_hp(&self) -> f32 {
        self.tick_amount_hp
    }
    pub fn is_due(&self, now: Duration) -> bool {
        now.saturating_sub(self.started)
            .as_secs()
            .min(u64::from(self.parameters.ticks))
            > u64::from(self.completed_ticks)
    }
    /// None means no tick is due and the caller keeps the effect unchanged.
    /// Some returns a health snapshot and either the remaining effect or None
    /// when all authored ticks have run, maximum health is reached, or the
    /// character is dead. Admission/life generation checks belong to the caller.
    pub fn advance(
        self,
        now: Duration,
        damage: CharacterDamage,
    ) -> Option<(CharacterDamage, Option<Self>)> {
        self.advance_with_receipts(now, damage)
            .map(|(damage, remaining, _)| (damage, remaining))
    }
    /// Native heal stats truncate actual post-quantization HP separately per
    /// tick. Preserve those quantities when delayed publication catches up.
    pub fn advance_with_receipts(
        mut self,
        now: Duration,
        mut damage: CharacterDamage,
    ) -> Option<HealingAdvance> {
        if damage.health_quarter_points == 0
            || damage.health_quarter_points >= self.max_health_quarter_points
        {
            return Some((damage, None, Vec::new()));
        }
        let due = now
            .saturating_sub(self.started)
            .as_secs()
            .min(u64::from(self.parameters.ticks)) as u8;
        if due <= self.completed_ticks {
            return None;
        }
        let mut receipts = Vec::new();
        while self.completed_ticks < due {
            let previous = damage.health_quarter_points;
            let health_hp = previous as f32 * 0.25;
            damage.health_quarter_points =
                ((health_hp + self.tick_amount_hp).min(self.max_health_hp) * 4.0).trunc() as u16;
            damage.last_location_and_type = 0x60 | (damage.last_location_and_type & 0x0f);
            self.completed_ticks += 1;
            receipts.push((
                self.completed_ticks,
                u32::from(damage.health_quarter_points - previous) / 4,
            ));
            if damage.health_quarter_points == self.max_health_quarter_points {
                return Some((damage, None, receipts));
            }
        }
        let remaining = (self.completed_ticks < self.parameters.ticks).then_some(self);
        Some((damage, remaining, receipts))
    }
}
