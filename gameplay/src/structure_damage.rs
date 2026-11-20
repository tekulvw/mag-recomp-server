//! Damage state for retail collidables that resolve to a damage objective trigger.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Source {
    pub player: u16,
    pub generation: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StructureDamageKind {
    Damage,
    AirStrike,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StructureDamageOutcome {
    Ignored,
    Accumulated,
    Destroyed,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StructureDamage {
    kind: StructureDamageKind,
    threshold: f32,
    accumulated_damage: f32,
}

impl StructureDamage {
    pub fn new(kind: u8, threshold: f32) -> Result<Self, &'static str> {
        let kind = match kind {
            4 => StructureDamageKind::Damage,
            5 => StructureDamageKind::AirStrike,
            _ => return Err("invalid_structure_damage_kind"),
        };
        if !threshold.is_finite() || threshold <= 0.0 {
            return Err("invalid_structure_damage_threshold");
        }
        Ok(Self {
            kind,
            threshold,
            accumulated_damage: 0.0,
        })
    }

    pub fn accumulated_damage(&self) -> f32 {
        self.accumulated_damage
    }

    pub fn is_destroyed(&self) -> bool {
        self.accumulated_damage >= self.threshold
    }

    /// Processes raw receiver damage after the caller has qualified the blast geometry.
    /// Destroyed signals the threshold crossing. The owner decides whether to accept
    /// the progression request and remove the collider.
    pub fn apply(
        &mut self,
        source_type: u32,
        damage: f32,
        active: bool,
        owner_is_active: bool,
    ) -> StructureDamageOutcome {
        const AIR_STRIKE_SOURCE_TYPE: u32 = 4;

        if !active
            || !owner_is_active
            || self.is_destroyed()
            || (self.kind == StructureDamageKind::AirStrike
                && source_type != AIR_STRIKE_SOURCE_TYPE)
            || !damage.is_finite()
            || damage <= 0.0
        {
            return StructureDamageOutcome::Ignored;
        }
        self.accumulated_damage += damage;
        if self.is_destroyed() {
            StructureDamageOutcome::Destroyed
        } else {
            StructureDamageOutcome::Accumulated
        }
    }
}

#[cfg(test)]
#[path = "structure_damage_tests.rs"]
mod structure_damage_tests;
