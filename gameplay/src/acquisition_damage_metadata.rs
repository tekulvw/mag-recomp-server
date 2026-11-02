//! Narrow native volume-lethal request construction, not damage application.
//! Ordinary character notifications, dead/protection gates, modifiers, armor and
//! lifecycle remain required. Neither constructor can prove terminal death.
#[derive(Clone, Copy, Debug)]
pub struct Metadata {
    damage_type: u8,
    source_type: u32,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CharacterLethal {
    pub amount_hp: f32,
    pub damage_type: u8,
    pub source_type: u32,
    pub source_index: i32,
    pub attacker: u32,
    pub source_object: u32,
    pub location: u8,
    pub user_data: u32,
    pub armor_reduction: [f32; 2],
    /// Direct copy of the qualified victim's vector at native +20; do not infer
    /// a normalized direction from the DamageData member's name.
    pub from_direction: [f32; 3],
    /// Clear the native bool before calling ordinary ApplyDamage only if set.
    pub clear_invulnerability: bool,
    /// After ApplyDamage native restores canonical1 only when initially nonzero.
    pub restore_invulnerability: Option<u8>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GroundLethal {
    pub attacker: u32,
    pub vaporize: bool,
}
impl Metadata {
    /// Only this original Lua enum pair is currently bound. Reject unknown
    /// symbols instead of fabricating firearm metadata or source attribution.
    pub fn from_symbols(damage_type: &str, source_type: &str) -> Result<Self, &'static str> {
        if damage_type != "DamageType.ExternalKinetic"
            || source_type != "DamageSourceType.eDamageSourceTypeCrushed"
        {
            return Err("unsupported_acquisition_damage_metadata");
        }
        Ok(Self {
            damage_type: 5,
            source_type: 10,
        })
    }
    /// The caller supplies a resolved current victim and its native field vector.
    /// This finite admitted domain does not establish identity/liveness itself.
    /// Raw1000 still passes through normal modifier/armor and dead checks: do
    /// not replace it with kill_self or an unconditional zero-health write.
    pub fn character_lethal(
        self,
        victim: u32,
        victim_vector: [f32; 3],
        invulnerability: u8,
    ) -> Result<CharacterLethal, &'static str> {
        if victim == u32::MAX || !victim_vector.iter().all(|v| v.is_finite()) {
            return Err("unsupported_acquisition_lethal_victim");
        }
        Ok(CharacterLethal {
            amount_hp: 1000.,
            damage_type: self.damage_type,
            source_type: self.source_type,
            source_index: -1,
            attacker: victim,
            source_object: u32::MAX,
            location: 0,
            user_data: 0,
            armor_reduction: [0.; 2],
            from_direction: victim_vector,
            clear_invulnerability: invulnerability != 0,
            restore_invulnerability: (invulnerability != 0).then_some(1),
        })
    }
    /// Native callback arguments only. The model applies its dead guard and
    /// SetHealth(0,-1), and the vehicle then stores vaporize. OnDeath, occupant
    /// handling, objective notification and removal are later distinct effects.
    pub fn ground_lethal(self) -> GroundLethal {
        GroundLethal {
            attacker: u32::MAX,
            vaporize: true,
        }
    }
}
