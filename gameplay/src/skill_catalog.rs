//! Owned character modifiers loaded from the authored disk skill table.
//! Native Reset/ChangeModifier (003bf8d0/003bf8f8) zero slots and add f32
//! values in ascending skill order. Faction type/value fallback is independent
//! (008742a0/00874304); zero selects the corresponding Default field.
use serde_json::Value;
use std::{collections::BTreeMap, io, path::Path};
#[derive(Clone, Debug)]
struct Skill {
    available: [bool; 3],
    modifier: [Option<(usize, f32)>; 3],
}
#[derive(Clone, Debug)]
pub struct SkillCatalog {
    skills: BTreeMap<usize, Skill>,
}
#[derive(Clone, Copy, Debug)]
pub struct OwnedModifiers {
    values: [f32; 41],
}
impl Default for OwnedModifiers {
    fn default() -> Self {
        Self { values: [0.0; 41] }
    }
}
// Recovered native enum names, not authored modifier amounts.
const MODIFIER_TYPES: [&str; 41] = [
    "kCharModNone",
    "kCharModHealth",
    "kCharModSprintTime",
    "kCharModParachuteControl",
    "kCharModDamageTaken",
    "kCharModExplosiveDamageTaken",
    "kCharModImpactDamageTaken",
    "kCharModChemicalDamageTaken",
    "kCharModFlashBangTime",
    "kCharModResupplyRate",
    "kCharModAutoHeal",
    "kCharModAutoRepair",
    "kCharModMovementSpeed",
    "kCharModHoldBreathTime",
    "kCharModDetectionSpeed",
    "kCharModSecureAndHoldTime",
    "kCharModAcousticDetectionRange",
    "kCharModDemolitionTime",
    "kCharModIncapacitatedTime",
    "kCharModResuscitatePercent",
    "kCharModRepairAmount",
    "kCharModHealAmount",
    "kCharModStaminaRecovery",
    "kCharModReloadTime",
    "kCharModAssaultRifleReloadTime",
    "kCharModAssaultRifleBarrelWander",
    "kCharModAssaultRifleKnock",
    "kCharModGrenadeThrowPower",
    "kCharModGrenadeLauncherReloadTime",
    "kCharModKnifeReach",
    "kCharModKnifeSpeed",
    "kCharModMachineGunReloadTime",
    "kCharModMachineGunKnock",
    "kCharModRocketLauncherReloadTime",
    "kCharModSideArmRefireTime",
    "kCharModSideArmSwapTime",
    "kCharModSideArmReloadTime",
    "kCharModSniperRifleReloadTime",
    "kCharModSniperRifleBarrelWander",
    "kCharModCloseQuartersReloadTime",
    "kCharModCloseQuartersKnock",
];
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
fn modifier_type(row: &Value) -> io::Result<Option<usize>> {
    match row.get("Modifier") {
        None => Ok(None),
        Some(Value::String(value)) if value == "kCharModNone" => Ok(None),
        Some(Value::String(value)) => MODIFIER_TYPES
            .iter()
            .position(|v| *v == value)
            .map(Some)
            .ok_or_else(|| invalid("unknown skill modifier type")),
        _ => Err(invalid("invalid skill modifier type")),
    }
}
fn modifier_value(row: &Value) -> io::Result<f32> {
    let Some(value) = row.get("ModifierValue") else {
        return Ok(0.0);
    };
    let number = value
        .as_f64()
        .ok_or_else(|| invalid("invalid skill modifier value"))? as f32;
    if !number.is_finite() {
        return Err(invalid("nonfinite skill modifier value"));
    }
    Ok(number)
}
impl SkillCatalog {
    pub fn load(path: impl AsRef<Path>) -> io::Result<Self> {
        let raw = serde_json::from_reader(io::BufReader::new(std::fs::File::open(path)?))
            .map_err(|e| invalid(&e.to_string()))?;
        Self::from_value(raw)
    }
    pub fn from_value(raw: Value) -> io::Result<Self> {
        if !matches!(
            (
                raw.get("schema_version").and_then(Value::as_u64),
                raw.get("application_id").and_then(Value::as_u64)
            ),
            (Some(1), Some(20230)) | (Some(2), Some(20231))
        ) {
            return Err(invalid("unsupported skill catalog schema or application"));
        }
        let rows = raw
            .pointer("/table/Skills")
            .and_then(Value::as_object)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| invalid("missing skill table"))?;
        let mut skills = BTreeMap::new();
        for (key, row) in rows {
            let id = key
                .strip_prefix("MagCharacterSkill")
                .and_then(|v| v.parse::<usize>().ok())
                .filter(|v| *v < 160)
                .ok_or_else(|| invalid("invalid skill ordinal"))?;
            if key != &format!("MagCharacterSkill{id}") {
                return Err(invalid("invalid skill ordinal spelling"));
            }
            let row = row
                .get("MagCharacterSkill")
                .and_then(Value::as_object)
                .ok_or_else(|| invalid("invalid skill row"))?;
            if ["Default", "Green", "Black", "Red"]
                .iter()
                .any(|name| row.get(*name).is_some_and(|value| !value.is_object()))
            {
                return Err(invalid("invalid skill faction/default row"));
            }
            let default = row.get("Default").unwrap_or(&Value::Null);
            let default_type = modifier_type(default)?;
            let default_value = modifier_value(default)?;
            let mut skill = Skill {
                available: [false; 3],
                modifier: std::array::from_fn(|_| None),
            };
            for (f, name) in ["Green", "Black", "Red"].into_iter().enumerate() {
                let faction = row.get(name).unwrap_or(&Value::Null);
                skill.available[f] = match faction.get("Available") {
                    None => false,
                    Some(Value::Bool(v)) => *v,
                    _ => return Err(invalid("invalid skill availability")),
                };
                let kind = modifier_type(faction)?.or(default_type);
                let value = modifier_value(faction)?;
                let value = if value == 0.0 { default_value } else { value };
                skill.modifier[f] = kind.map(|kind| (kind, value));
            }
            if skills.insert(id, skill).is_some() {
                return Err(invalid("duplicate skill ordinal"));
            }
        }
        Ok(Self { skills })
    }
    pub fn resolve(&self, faction: u8, owned: &[u32; 5]) -> Result<OwnedModifiers, &'static str> {
        let faction = usize::from(faction);
        if faction >= 3 {
            return Err("unsupported_skill_faction");
        }
        let mut result = OwnedModifiers::default();
        for id in 0..160 {
            if owned[id / 32] & (1u32 << (id % 32)) == 0 {
                continue;
            }
            let skill = self.skills.get(&id).ok_or("undefined_owned_skill")?;
            if !skill.available[faction] {
                return Err("unavailable_owned_skill");
            }
            if let Some((kind, value)) = &skill.modifier[faction] {
                let total = &mut result.values[*kind];
                *total += value;
                if !total.is_finite() {
                    return Err("nonfinite_owned_modifier");
                }
            }
        }
        result.max_health_quarter_points()?;
        Ok(result)
    }
}
impl OwnedModifiers {
    /// Compose from immutable owned base slots. Calling this on an already
    /// combined value would double leadership; Engine keeps its base unchanged.
    pub fn with_leadership(
        self,
        passives: &crate::leadership_passives::PassiveModifiers,
    ) -> Result<Self, &'static str> {
        let mut result = self;
        for (kind, amount) in passives.iter() {
            let index = MODIFIER_TYPES
                .iter()
                .position(|name| *name == kind.as_str())
                .ok_or("unsupported_leadership_modifier")?;
            result.values[index] += amount;
            if !result.values[index].is_finite() {
                return Err("nonfinite_combined_modifier");
            }
        }
        result.max_health_quarter_points()?;
        Ok(result)
    }

    pub fn value(&self, name: &str) -> f32 {
        MODIFIER_TYPES
            .iter()
            .position(|kind| *kind == name)
            .map_or(0.0, |id| self.values[id])
    }
    /// Native ApplyModifiers 0078b3c0 uses fmadds, then a 1HP lower bound.
    pub fn max_health_hp(&self) -> f32 {
        100.0_f32
            .mul_add(self.value("kCharModHealth"), 100.0)
            .max(1.0)
    }
    pub fn max_health_quarter_points(&self) -> Result<u16, &'static str> {
        let hp = self.max_health_hp();
        let points = (hp * 4.0).trunc();
        if !hp.is_finite() || points >= 4096.0 {
            return Err("skill_health_exceeds_wire_bounds");
        }
        Ok(points as u16)
    }
}
