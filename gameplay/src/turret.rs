//! Captured prototype Alpha turret owner reports and its authored minigun.
//! Native Vehicle fields8/9 use FloatMarkup sign+integer3+fraction4 and
//! sign+integer1+fraction6; fields11/12 use the ordinary24-bit InventoryItem.
//! Field21 is the native turret0 overheat byte. No server ownership, health,
//! inventory definition or second-turret field is accepted from the client.
//! See docs/protocol/dme-probe/TURRET-MULTIPLAYER.md for the native binder.
use crate::armor::{ArmorProfile, RawCharacterDamage};
use crate::item_catalog::{self, BulletParameters, ItemCatalog};
use crate::replication::{BitReader, BitWriter, InventoryItem, ObjectUpdate, UpdateHeader};

pub const TYPE_MASK: u32 = 1 << 13;
pub const ALPHA_OBJECT_ID: u32 = 0x0d800000;

/// Construct from authenticated match state, never from the packet owner/index.
/// A proof is per event; do not retain it across seat or life transitions.
#[derive(Clone, Copy, Debug)]
pub struct AlphaTurretAuthority {
    player: u16,
}
impl AlphaTurretAuthority {
    pub fn authorize(
        player: u16,
        entered_game: bool,
        ready_for_updates: bool,
        living: bool,
        mounted_seat: Option<u8>,
        exclusive_turret_owner: Option<u16>,
    ) -> Result<Self, &'static str> {
        if !entered_game
            || !ready_for_updates
            || !living
            || mounted_seat != Some(0)
            || exclusive_turret_owner != Some(player)
        {
            return Err("turret_requires_living_exclusive_owner");
        }
        Ok(Self { player })
    }
    pub fn player(self) -> u16 {
        self.player
    }

    /// The seat proves the property binding; the last accepted native item
    /// report proves the gunner is using its initialized primary gear/pool.
    pub fn minigun_profile_from_equipped(
        self,
        equipped: InventoryItem,
        gear_index: u8,
        fire_mode: u8,
    ) -> Result<AlphaMinigunProfile, &'static str> {
        self.minigun_profile_from_equipped_with_catalog(
            equipped,
            gear_index,
            fire_mode,
            item_catalog::bundled()?,
        )
    }
    pub fn minigun_profile_from_equipped_with_catalog(
        self,
        equipped: InventoryItem,
        gear_index: u8,
        fire_mode: u8,
        catalog: &ItemCatalog,
    ) -> Result<AlphaMinigunProfile, &'static str> {
        if equipped.inventory_update_id == 0 || equipped.gear_slot != 0 || equipped.ammo_pool != 0 {
            return Err("unsupported_alpha_turret_equipped_item");
        }
        self.minigun_profile_with_catalog(gear_index, fire_mode, catalog)
    }

    pub fn minigun_profile(
        self,
        gear_index: u8,
        fire_mode: u8,
    ) -> Result<AlphaMinigunProfile, &'static str> {
        self.minigun_profile_with_catalog(gear_index, fire_mode, item_catalog::bundled()?)
    }
    pub fn minigun_profile_with_catalog(
        self,
        gear_index: u8,
        fire_mode: u8,
        catalog: &ItemCatalog,
    ) -> Result<AlphaMinigunProfile, &'static str> {
        // Turret0's authored Weapon1 is minigun_green (global item53), gear0.
        // The captured mounted hit reports use gear0/mode0. Other turret guns
        // require their own native evidence and must not fall back to this gun.
        if gear_index != 0 || fire_mode != 0 {
            return Err("unsupported_alpha_turret_weapon");
        }
        Ok(AlphaMinigunProfile {
            parameters: catalog.bullet(53)?,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeTurretYaw(u8);
impl NativeTurretYaw {
    pub fn packed(self) -> u8 {
        self.0
    }
    pub fn radians(self) -> f32 {
        let magnitude = (self.0 & 127) as f32 / 16.0;
        if self.0 & 128 != 0 {
            -magnitude
        } else {
            magnitude
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeTurretPitch(u8);
impl NativeTurretPitch {
    pub fn packed(self) -> u8 {
        self.0
    }
    pub fn radians(self) -> f32 {
        let magnitude = (self.0 & 127) as f32 / 64.0;
        if self.0 & 128 != 0 {
            -magnitude
        } else {
            magnitude
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct ClientTurretFields {
    pub yaw: Option<NativeTurretYaw>,
    pub pitch: Option<NativeTurretPitch>,
    pub primary: Option<InventoryItem>,
    pub secondary: Option<InventoryItem>,
    /// Exact native byte; preserving it does not author a heat simulation.
    pub overheat_info: Option<u8>,
}
impl ClientTurretFields {
    pub fn decode(
        update: &ObjectUpdate<'_>,
        _authority: AlphaTurretAuthority,
    ) -> Result<Self, &'static str> {
        Self::decode_with_profile(
            update,
            _authority,
            crate::wire_profile::WireProfile::prototype(),
        )
    }
    pub fn decode_with_profile(
        update: &ObjectUpdate<'_>,
        _authority: AlphaTurretAuthority,
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Self, &'static str> {
        update.validate_profile(profile)?;
        if update.header.object_type_mask != TYPE_MASK {
            return Err("unsupported_client_turret_type");
        }
        let mut bits = BitReader::with_profile(update.wire, profile);
        bits.position = profile.header_bits();
        let fields = Self::decode_object(&mut bits)?.unwrap_or_default();
        let remaining = update.wire.len() * 8 - bits.position;
        if remaining >= 8 || bits.read(remaining)? != 0 {
            return Err("trailing_client_turret_bits");
        }
        Ok(fields)
    }
    pub(crate) fn decode_object(bits: &mut BitReader<'_>) -> Result<Option<Self>, &'static str> {
        let mut fields = Self::default();
        if bits.read(1)? == 0 {
            return Ok(None);
        }
        // Network allocation0 is fixed for Alpha. It is not a player identity.
        if bits.read(16)? != 0 {
            return Err("unowned_client_turret_object");
        }
        for field in 0..25 {
            if bits.read(1)? == 0 {
                continue;
            }
            match field {
                8 => fields.yaw = Some(NativeTurretYaw(bits.read(8)? as u8)),
                9 => fields.pitch = Some(NativeTurretPitch(bits.read(8)? as u8)),
                11 => fields.primary = Some(crate::player_tracking::inventory(bits)?),
                12 => fields.secondary = Some(crate::player_tracking::inventory(bits)?),
                21 => fields.overheat_info = Some(bits.read(8)? as u8),
                _ => return Err("unsupported_client_turret_field"),
            }
        }
        if bits.read(1)? != 0 {
            return Err("multiple_client_turret_updates");
        }
        Ok(Some(fields))
    }
    /// Empty selected lists and zero-field objects request no turret mutation.
    pub fn has_changes(&self) -> bool {
        self.yaw.is_some()
            || self.pitch.is_some()
            || self.primary.is_some()
            || self.secondary.is_some()
            || self.overheat_info.is_some()
    }
    /// Retain latest values for a late recipient's current turret snapshot.
    pub fn merge(&mut self, newer: &Self) {
        if newer.yaw.is_some() {
            self.yaw = newer.yaw;
        }
        if newer.pitch.is_some() {
            self.pitch = newer.pitch;
        }
        if newer.primary.is_some() {
            self.primary = newer.primary;
        }
        if newer.secondary.is_some() {
            self.secondary = newer.secondary;
        }
        if newer.overheat_info.is_some() {
            self.overheat_info = newer.overheat_info;
        }
    }
    pub fn encode_update(&self, sequence: u32, timestamp_ms: u32) -> Result<Vec<u8>, &'static str> {
        self.encode_update_with_profile(
            sequence,
            timestamp_ms,
            crate::wire_profile::WireProfile::prototype(),
        )
    }
    pub fn encode_update_with_profile(
        &self,
        sequence: u32,
        timestamp_ms: u32,
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        if sequence == 0 {
            return Err("invalid_turret_sequence");
        }
        let mut bits = BitWriter::with_profile(profile);
        UpdateHeader {
            sequence,
            nak_sequence: 0,
            client_index: 0,
            object_type_mask: TYPE_MASK,
            timestamp_ms,
        }
        .encode(&mut bits)?;
        bits.write(1, 1)?;
        bits.write(0, 16)?;
        for field in 0..25 {
            let present = match field {
                8 => self.yaw.is_some(),
                9 => self.pitch.is_some(),
                11 => self.primary.is_some(),
                12 => self.secondary.is_some(),
                21 => self.overheat_info.is_some(),
                _ => false,
            };
            bits.write(u32::from(present), 1)?;
            if !present {
                continue;
            }
            match field {
                8 => bits.write(self.yaw.unwrap().packed().into(), 8)?,
                9 => bits.write(self.pitch.unwrap().packed().into(), 8)?,
                11 => self.primary.unwrap().encode(&mut bits)?,
                12 => self.secondary.unwrap().encode(&mut bits)?,
                21 => bits.write(self.overheat_info.unwrap().into(), 8)?,
                _ => unreachable!(),
            }
        }
        bits.write(0, 1)?;
        Ok(bits.finish())
    }
}
/// Authored minigun_green => minigun_ammo: character30, APierce0/ARedux0,
/// effective/max300. No arbitrary source damage enters this profile.
#[derive(Clone, Copy, Debug)]
pub struct AlphaMinigunProfile {
    parameters: BulletParameters,
}
impl AlphaMinigunProfile {
    pub fn item_id(self) -> u16 {
        53
    }
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
        let scaled_source = self.parameters.damage.character * reported_scale;
        let falloff = if distance < self.parameters.effective_range {
            1.0
        } else if distance >= self.parameters.max_range {
            0.0
        } else {
            (-0.5_f32).mul_add(
                (distance - self.parameters.effective_range)
                    / (self.parameters.max_range - self.parameters.effective_range),
                1.0,
            )
        };
        let amount_hp = scaled_source * falloff;
        Ok(RawCharacterDamage {
            amount_hp,
            armor_rating_redux: self.parameters.damage.armor_pierce,
            armor_prevention_redux: self.parameters.damage.armor_redux,
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
