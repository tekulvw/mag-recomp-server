//! Retail command availability, per-round cooldowns and native state fields.
use crate::{
    active_abilities::{Ability, CommandRole, Rules, Side},
    replication::{BitWriter, SquadAddress, UpdateHeader},
    wire_profile::WireProfile,
};

const ABILITY_COUNT: usize = 6;
const SQUAD_COUNT: usize = 32;
const PLATOON_COUNT: usize = 8;

/// Retail LevelLoaded uses the attacker 128-player columns for both sides
/// in Acquisition and Faction Exercise. Other unsupported modes register none.
pub fn native_command_availability(
    rules: &Rules,
    mode: u16,
    side: Side,
    role: CommandRole,
) -> Option<&[Ability]> {
    match mode {
        0 => rules.allowed(64, side, role),
        1 | 3 => rules.allowed(128, Side::Attacker, role),
        2 => rules.allowed(256, side, role),
        _ => Some(&[]),
    }
}

fn ability_slot(ability: Ability) -> Result<usize, &'static str> {
    ability
        .code()
        .checked_sub(1)
        .map(usize::from)
        .ok_or("invalid_native_ability_type")
}

pub fn native_ability_mask(ability: Ability) -> Result<u16, &'static str> {
    Ok(1 << ability_slot(ability)?)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeAbilityScope {
    Squad(u16),
    Platoon(u16),
}

impl NativeAbilityScope {
    fn platoon(self) -> Result<usize, &'static str> {
        match self {
            Self::Squad(index) => {
                SquadAddress::from_index(index)?;
                Ok(usize::from(index / 4))
            }
            Self::Platoon(index) if usize::from(index) < PLATOON_COUNT => Ok(usize::from(index)),
            Self::Platoon(_) => Err("invalid_native_platoon_index"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeActivationOutcome {
    Activated,
    /// The failure effect is part of the normal command path and spends both cooldowns.
    AcceptedFailure,
    /// Ability data was stored before the success effect factory failed.
    FactoryFailed,
}

#[derive(Clone, Debug)]
pub struct NativeCooldowns {
    shared_duration_ms: u32,
    durations_ms: [Option<u32>; ABILITY_COUNT],
    shared_ms: [u32; PLATOON_COUNT],
    squad_ms: [[u32; ABILITY_COUNT]; SQUAD_COUNT],
    platoon_ms: [[u32; ABILITY_COUNT]; PLATOON_COUNT],
}

impl NativeCooldowns {
    pub fn new(rules: &Rules) -> Result<Self, &'static str> {
        let shared_duration_ms = u32::try_from(rules.command_global_cooldown_ms)
            .ok()
            .filter(|&ms| ms > 0 && ms <= u32::MAX / 100)
            .ok_or("invalid_native_shared_cooldown")?;
        let mut durations_ms = [None; ABILITY_COUNT];
        for property in &rules.commands {
            let slot = ability_slot(property.ability)?;
            let duration_ms = u32::try_from(property.cooldown_ms)
                .ok()
                .filter(|&ms| ms <= u32::MAX / 200)
                .ok_or("invalid_native_ability_cooldown")?;
            if durations_ms[slot].replace(duration_ms).is_some() {
                return Err("duplicate_native_ability_cooldown");
            }
        }
        Ok(Self {
            shared_duration_ms,
            durations_ms,
            shared_ms: [0; PLATOON_COUNT],
            squad_ms: [[0; ABILITY_COUNT]; SQUAD_COUNT],
            platoon_ms: [[0; ABILITY_COUNT]; PLATOON_COUNT],
        })
    }

    /// The shared timer starts at native game time zero. Only individual
    /// ability data uses zero as a sentinel for immediate readiness. A squad
    /// ability with a prior activation and zero effective duration remains
    /// unavailable, represented by `u32::MAX`.
    pub fn remaining_ms(
        &self,
        scope: NativeAbilityScope,
        ability: Ability,
        now_ms: u32,
        shadow_modifier: f32,
    ) -> Result<u32, &'static str> {
        let platoon = scope.platoon()?;
        let slot = ability_slot(ability)?;
        let base_ms = self.durations_ms[slot].ok_or("missing_native_ability_cooldown")?;
        let modifier = clamped_shadow(shadow_modifier)?;
        let individual_duration_ms = modifier.mul_add(base_ms as f32, base_ms as f32) as u32;
        let last_ms = match scope {
            NativeAbilityScope::Squad(index) => self.squad_ms[usize::from(index)][slot],
            NativeAbilityScope::Platoon(_) => self.platoon_ms[platoon][slot],
        };
        if last_ms != 0
            && individual_duration_ms == 0
            && matches!(scope, NativeAbilityScope::Squad(_))
        {
            return Ok(u32::MAX);
        }
        let shared = remaining(self.shared_duration_ms, self.shared_ms[platoon], now_ms);
        let individual = if last_ms == 0 {
            0
        } else {
            remaining(individual_duration_ms, last_ms, now_ms)
        };
        Ok(shared.max(individual))
    }

    /// Install a server-observed outcome after the caller records the
    /// transaction. Denial paths have no outcome to install.
    pub fn record(
        &mut self,
        scope: NativeAbilityScope,
        ability: Ability,
        now_ms: u32,
        outcome: NativeActivationOutcome,
    ) -> Result<(), &'static str> {
        let platoon = scope.platoon()?;
        let slot = ability_slot(ability)?;
        if self.durations_ms[slot].is_none() {
            return Err("missing_native_ability_cooldown");
        }
        let last_ms = match scope {
            NativeAbilityScope::Squad(index) => &mut self.squad_ms[usize::from(index)][slot],
            NativeAbilityScope::Platoon(_) => &mut self.platoon_ms[platoon][slot],
        };
        *last_ms = now_ms;
        if outcome != NativeActivationOutcome::FactoryFailed {
            self.shared_ms[platoon] = now_ms;
        }
        Ok(())
    }
}

fn remaining(duration_ms: u32, last_ms: u32, now_ms: u32) -> u32 {
    let elapsed_percent_numerator = now_ms.wrapping_sub(last_ms).wrapping_mul(100);
    (duration_ms * 100)
        .saturating_sub(elapsed_percent_numerator)
        .div_ceil(100)
}

fn clamped_shadow(modifier: f32) -> Result<f32, &'static str> {
    if !modifier.is_finite() {
        return Err("invalid_native_shadow_modifier");
    }
    Ok(modifier.clamp(-1.0, 1.0))
}

/// The native net object writes the low byte of trunc(value * 100).
/// Its simulation manager separately clamps the effective f32 value.
pub fn native_shadow_modifier_percent(value: f32) -> Result<i8, &'static str> {
    let percent = value * 100.0;
    if !percent.is_finite() || !(-2_147_483_648.0..2_147_483_648.0).contains(&percent) {
        return Err("invalid_native_shadow_modifier");
    }
    Ok(percent as i32 as i8)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NativeAbilityData {
    pub position: [f32; 3],
    pub squad_index: u16,
    pub activate_time_ms: u32,
    pub enabled_at_start: bool,
    pub target_id: u32,
}

impl Default for NativeAbilityData {
    fn default() -> Self {
        Self {
            position: [0.; 3],
            squad_index: 0,
            activate_time_ms: 0,
            enabled_at_start: false,
            target_id: u32::MAX,
        }
    }
}

impl NativeAbilityData {
    pub const WIRE_BITS: usize = 177;

    fn encode(self, bits: &mut BitWriter) -> Result<(), &'static str> {
        if !self
            .position
            .iter()
            .all(|coordinate| coordinate.is_finite())
        {
            return Err("invalid_native_ability_position");
        }
        for coordinate in self.position {
            bits.write(coordinate.to_bits(), 32)?;
        }
        bits.write(self.squad_index.into(), 16)?;
        bits.write(self.activate_time_ms, 32)?;
        bits.write(self.enabled_at_start.into(), 1)?;
        bits.write(self.target_id, 32)
    }
}

/// Ability fields shared by type8 Squad and type10 Platoon. Supply the
/// active and owner fields when creating a Platoon object. Squad members
/// continue to be created by the standard roster snapshot.
#[derive(Clone, Debug, PartialEq)]
pub struct NativeAbilityUpdate {
    pub scope: NativeAbilityScope,
    pub owner_index: Option<u16>,
    pub active: Option<bool>,
    pub leader_index: Option<u16>,
    pub data: [Option<NativeAbilityData>; ABILITY_COUNT],
    pub enabled_mask: Option<u16>,
    pub active_mask: Option<u16>,
    pub available_mask: Option<u16>,
    pub shadow_modifier_percent: Option<i8>,
}

#[derive(Clone, Copy)]
enum Value {
    Bit(bool),
    Byte(i8),
    U16(u16),
    Ability(NativeAbilityData),
}

impl Value {
    fn write_optional(value: Option<Self>, bits: &mut BitWriter) -> Result<(), &'static str> {
        bits.write(value.is_some().into(), 1)?;
        match value {
            None => Ok(()),
            Some(Self::Bit(value)) => bits.write(value.into(), 1),
            Some(Self::Byte(value)) => bits.write((value as u8).into(), 8),
            Some(Self::U16(value)) => bits.write(value.into(), 16),
            Some(Self::Ability(data)) => data.encode(bits),
        }
    }
}

impl NativeAbilityUpdate {
    pub fn new(scope: NativeAbilityScope) -> Self {
        Self {
            scope,
            owner_index: None,
            active: None,
            leader_index: None,
            data: [None; ABILITY_COUNT],
            enabled_mask: None,
            active_mask: None,
            available_mask: None,
            shadow_modifier_percent: None,
        }
    }

    pub fn encode(
        &self,
        sequence: u32,
        timestamp_ms: u32,
        profile: WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        Self::encode_batch(std::slice::from_ref(self), sequence, timestamp_ms, profile)
    }

    /// Encode up to eight squads or eight platoons under a common object-type header.
    pub fn encode_batch(
        updates: &[Self],
        sequence: u32,
        timestamp_ms: u32,
        profile: WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        let first = updates
            .first()
            .ok_or("invalid_native_ability_batch_count")?;
        if updates.len() > 8 {
            return Err("invalid_native_ability_batch_count");
        }
        let object_type = match first.scope {
            NativeAbilityScope::Squad(_) => 8,
            NativeAbilityScope::Platoon(_) => 10,
        };
        let mut bits = begin_update(object_type, sequence, timestamp_ms, profile)?;
        for update in updates {
            if std::mem::discriminant(&update.scope) != std::mem::discriminant(&first.scope) {
                return Err("mixed_native_ability_batch_scopes");
            }
            update.write_fields(&mut bits)?;
        }
        bits.write(0, 1)?;
        Ok(bits.finish())
    }

    fn write_fields(&self, bits: &mut BitWriter) -> Result<(), &'static str> {
        self.scope.platoon()?;
        let (object_type, index, field_count, data_start, leader, active_mask, shadow) =
            match self.scope {
                NativeAbilityScope::Squad(index) => {
                    if self.enabled_mask.is_some() {
                        return Err("native_squad_has_no_enabled_mask");
                    }
                    (8, index, 26, 16, 10, 22, 24)
                }
                NativeAbilityScope::Platoon(index) => (10, index, 13, 2, 8, 10, 12),
            };
        write_object_index(bits, index)?;
        let mut fields = [None; 26];
        fields[0] = self.active.map(Value::Bit);
        fields[1] = self.owner_index.map(Value::U16);
        fields[leader] = self.leader_index.map(Value::U16);
        if object_type == 10 {
            fields[9] = self.enabled_mask.map(Value::U16);
        }
        for (field, data) in fields[data_start..data_start + ABILITY_COUNT]
            .iter_mut()
            .zip(self.data)
        {
            *field = data.map(Value::Ability);
        }
        fields[active_mask] = self.active_mask.map(Value::U16);
        fields[active_mask + 1] = self.available_mask.map(Value::U16);
        fields[shadow] = self.shadow_modifier_percent.map(Value::Byte);
        for value in fields.into_iter().take(field_count) {
            Value::write_optional(value, bits)?;
        }
        Ok(())
    }
}

/// Type3 Character fields for the common detection bit and the MAG jammer bit.
/// The caller publishes deltas only when the corresponding native value changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeDetectionUpdate {
    pub character_index: u16,
    pub globally_detected: Option<bool>,
    pub undetected_via_satellite_jammer: Option<bool>,
}

impl NativeDetectionUpdate {
    pub fn encode(
        &self,
        sequence: u32,
        timestamp_ms: u32,
        profile: WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        Self::encode_batch(std::slice::from_ref(self), sequence, timestamp_ms, profile)
    }

    /// Encode up to 48 character deltas in a single type-3 object list.
    pub fn encode_batch(
        updates: &[Self],
        sequence: u32,
        timestamp_ms: u32,
        profile: WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        if updates.is_empty() || updates.len() > 48 {
            return Err("invalid_native_detection_batch_count");
        }
        let mut bits = begin_update(3, sequence, timestamp_ms, profile)?;
        for update in updates {
            write_object_index(&mut bits, update.character_index)?;
            update.write_fields(&mut bits)?;
        }
        bits.write(0, 1)?;
        Ok(bits.finish())
    }

    fn write_fields(&self, bits: &mut BitWriter) -> Result<(), &'static str> {
        for field in 0..22 {
            let value = match field {
                10 => self.globally_detected,
                21 => self.undetected_via_satellite_jammer,
                _ => None,
            };
            Value::write_optional(value.map(Value::Bit), bits)?;
        }
        Ok(())
    }
}

fn begin_update(
    object_type: u32,
    sequence: u32,
    timestamp_ms: u32,
    profile: WireProfile,
) -> Result<BitWriter, &'static str> {
    if profile.application_id() != 20231 || sequence <= 1 {
        return Err("invalid_native_ability_replication_context");
    }
    let mut bits = BitWriter::with_profile(profile);
    UpdateHeader {
        sequence,
        nak_sequence: 0,
        client_index: 0,
        object_type_mask: 1 << object_type,
        timestamp_ms,
    }
    .encode(&mut bits)?;
    Ok(bits)
}

fn write_object_index(bits: &mut BitWriter, object_index: u16) -> Result<(), &'static str> {
    if object_index == u16::MAX {
        return Err("invalid_native_ability_replication_context");
    }
    bits.write(1, 1)?;
    bits.write(object_index.into(), 16)
}
