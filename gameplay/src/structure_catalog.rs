//! Authored emplacement durability, vehicle armor and repair-tool settings.
//! Native evidence supplies the one-based armor index and timed repair branch;
//! profile values remain on disk. This catalog does not authorize a target,
//! trace collision, apply damage, or drive objective/vehicle state transitions.
use crate::item_catalog::{ArmorParameters, BodyParameters};
use serde_json::Value;
use std::{collections::BTreeMap, io, path::Path};

#[derive(Clone, Debug, PartialEq)]
pub struct StructureProfile {
    pub faction: u8,
    pub model: String,
    pub properties_source: String,
    pub max_health: f32,
    pub armor_type: u8,
    pub critical_loss_percent_per_second: f32,
    pub can_repair_destroyed: bool,
    pub repaired_health_percent: f32,
    pub repair_seconds: f32,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RepairParameters {
    pub item_id: u16,
    pub amount: f32,
    pub distance: f32,
    pub radius: f32,
    pub cooldown_seconds: f32,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NativeStructureRules {
    pub light_percent: f32,
    pub heavy_percent: f32,
    pub critical_percent: f32,
    pub dead_health_floor: f32,
    pub min_rebuild_percent: f32,
}
#[derive(Clone, Debug)]
pub struct StructureCatalog {
    profiles: BTreeMap<u8, StructureProfile>,
    health_rules: NativeStructureRules,
    friendly_fire_multiplier: f32,
    armor: Vec<ArmorParameters>,
    body: [BodyParameters; 16],
    repair: RepairParameters,
    repair_modifiers: BTreeMap<u8, f32>,
}
fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
fn field<'a>(value: &'a Value, name: &str) -> io::Result<&'a Value> {
    value
        .get(name)
        .ok_or_else(|| invalid(format!("missing structure parameter {name}")))
}
fn scalar(value: &Value, name: &str) -> io::Result<f32> {
    let number = field(value, name)?
        .as_f64()
        .ok_or_else(|| invalid(format!("invalid structure parameter {name}")))?
        as f32;
    if !number.is_finite() {
        return Err(invalid(format!("nonfinite structure parameter {name}")));
    }
    Ok(number)
}
fn number(value: &Value, name: &str) -> io::Result<f32> {
    let result = scalar(value, name)?;
    if result < 0.0 {
        return Err(invalid(format!("negative structure parameter {name}")));
    }
    Ok(result)
}
fn positive(value: &Value, name: &str) -> io::Result<f32> {
    let result = number(value, name)?;
    if result == 0.0 {
        return Err(invalid(format!("zero structure parameter {name}")));
    }
    Ok(result)
}
fn integer(value: &Value, name: &str) -> io::Result<u8> {
    field(value, name)?
        .as_u64()
        .and_then(|n| u8::try_from(n).ok())
        .ok_or_else(|| invalid(format!("invalid structure integer {name}")))
}
fn string(value: &Value, name: &str) -> io::Result<String> {
    field(value, name)?
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| invalid(format!("invalid structure string {name}")))
}
impl StructureCatalog {
    pub fn load(path: impl AsRef<Path>) -> io::Result<Self> {
        Self::from_value(
            serde_json::from_reader(io::BufReader::new(std::fs::File::open(path)?))
                .map_err(|e| invalid(e.to_string()))?,
        )
    }
    pub fn from_value(raw: Value) -> io::Result<Self> {
        if raw.get("schema_version").and_then(Value::as_u64) != Some(1)
            || raw.get("application_id").and_then(Value::as_u64) != Some(20230)
        {
            return Err(invalid("unsupported structure catalog schema/application"));
        }
        let rules = field(&raw, "native_rules")?;
        let thresholds = field(rules, "health_condition_threshold_percent")?
            .as_array()
            .filter(|v| v.len() == 3)
            .ok_or_else(|| invalid("invalid vehicle health thresholds"))?;
        let mut condition_threshold_percent = [0.0; 3];
        for (target, source) in condition_threshold_percent.iter_mut().zip(thresholds) {
            *target = source
                .as_f64()
                .ok_or_else(|| invalid("invalid vehicle health threshold"))?
                as f32;
            if !target.is_finite() || *target <= 0.0 || *target > 100.0 {
                return Err(invalid("vehicle health threshold outside percent bounds"));
            }
        }
        if condition_threshold_percent.windows(2).any(|v| v[0] <= v[1]) {
            return Err(invalid("vehicle health thresholds must descend"));
        }
        let health_rules = NativeStructureRules {
            light_percent: condition_threshold_percent[0],
            heavy_percent: condition_threshold_percent[1],
            critical_percent: condition_threshold_percent[2],
            dead_health_floor: positive(rules, "dead_below_health")?,
            min_rebuild_percent: positive(rules, "minimum_rebuilt_health_percent")?,
        };
        if health_rules.min_rebuild_percent > 100.0 {
            return Err(invalid("invalid rebuilt health minimum percentage"));
        }
        let game_rules = raw
            .pointer("/resources/settings~1mag_game_settings.lua/MAGGameRules")
            .ok_or_else(|| invalid("missing authored game rules"))?;
        let friendly_fire_multiplier = number(game_rules, "FriendlyFireDamageMultiplier")?;
        if friendly_fire_multiplier > 1.0 {
            return Err(invalid("invalid friendly-fire multiplier"));
        }
        // All currently authored creation modes agree with the server default.
        // Reject ambiguity rather than silently choosing a different game mode.
        let modes = raw.pointer("/resources/settings~1gameproperties.zpr/GameCreationPropertiesTable/m_gameCreationTypes").and_then(Value::as_object).filter(|v| !v.is_empty()).ok_or_else(|| invalid("missing authored game creation modes"))?;
        for wrapper in modes.values() {
            let rules = wrapper
                .pointer("/GameCreationType/m_gameRules")
                .ok_or_else(|| invalid("missing game creation rules"))?;
            if number(rules, "FriendlyFireDamageMultiplier")? != friendly_fire_multiplier {
                return Err(invalid("ambiguous mode-specific friendly-fire multiplier"));
            }
        }
        let armor_rows = raw
            .pointer("/resources/vehicle_damagemodel.lua/ArmorTypeInfoList/ArmorTypes")
            .and_then(Value::as_object)
            .filter(|rows| !rows.is_empty() && rows.len() < 256)
            .ok_or_else(|| invalid("missing vehicle armor table"))?;
        let mut armor = Vec::with_capacity(armor_rows.len());
        for index in 0..armor_rows.len() {
            let p = armor_rows
                .get(&format!("ArmorTypeInfo{index}"))
                .and_then(|v| v.get("ArmorTypeInfo"))
                .ok_or_else(|| invalid("noncontiguous vehicle armor table"))?;
            let coverage_values = field(p, "m_coverage")?
                .as_array()
                .filter(|values| values.len() == 4)
                .ok_or_else(|| invalid("invalid vehicle armor coverage width"))?;
            let mut coverage = [0.0; 4];
            for (target, source) in coverage.iter_mut().zip(coverage_values) {
                *target = source
                    .as_f64()
                    .ok_or_else(|| invalid("invalid vehicle armor coverage"))?
                    as f32;
                if !target.is_finite() || !(0.0..=1.0).contains(target) {
                    return Err(invalid("vehicle armor coverage outside probability bounds"));
                }
            }
            let prevention = number(p, "prevention")?;
            if prevention > 1.0 {
                return Err(invalid(
                    "vehicle armor prevention outside probability bounds",
                ));
            }
            armor.push(ArmorParameters {
                armor_type: (index + 1) as u8,
                rating: number(p, "rating")?,
                prevention,
                coverage,
            });
        }
        let body_rows = raw
            .pointer(
                "/resources/vehicle_collision_info.lua/VehicleCollisionData/CollisionBodyMapping",
            )
            .and_then(Value::as_object)
            .filter(|v| v.len() == 16)
            .ok_or_else(|| invalid("invalid vehicle body table width"))?;
        let mut body = [BodyParameters {
            armor_area: 0,
            multiplier: 0.0,
        }; 16];
        for (index, entry) in body.iter_mut().enumerate() {
            let p = body_rows
                .get(&format!("CollisionBodyInfo{index}"))
                .and_then(|v| v.get("CollisionBodyInfo"))
                .ok_or_else(|| invalid("noncontiguous vehicle body table"))?;
            let area = integer(p, "armorArea")?;
            if area >= 4 || integer(p, "hitpointArea")? != 0 {
                return Err(invalid("unsupported vehicle armor/hitpoint area"));
            }
            *entry = BodyParameters {
                armor_area: usize::from(area),
                multiplier: number(p, "hitpointMult")?,
            };
        }
        let repair_row = field(&raw, "repair")?;
        let item_id = field(repair_row, "item_id")?
            .as_u64()
            .and_then(|n| u16::try_from(n).ok())
            .filter(|n| *n < 1024)
            .ok_or_else(|| invalid("invalid repair item ID"))?;
        let p = field(repair_row, "parameters")?;
        if p.get("activeitem_type").and_then(Value::as_str) != Some("kActiveItemRepairkit") {
            return Err(invalid("unexpected repair item type"));
        }
        let globals = field(repair_row, "globals")?;
        let repair = RepairParameters {
            item_id,
            amount: positive(globals, "repairkit_repair_amount")?,
            distance: positive(globals, "repairkit_repair_distance")?,
            radius: positive(globals, "repairkit_repair_radius")?,
            cooldown_seconds: positive(p, "cooldown_time")?,
        };
        let mut repair_modifiers = BTreeMap::new();
        for (key, wrapper) in field(repair_row, "skill_modifiers")?
            .as_object()
            .ok_or_else(|| invalid("invalid repair skill table"))?
        {
            let id = key
                .strip_prefix("MagCharacterSkill")
                .and_then(|s| s.parse::<u8>().ok())
                .filter(|id| *id < 128)
                .ok_or_else(|| invalid("invalid repair skill ID"))?;
            let p = wrapper
                .pointer("/MagCharacterSkill/Default")
                .ok_or_else(|| invalid("missing repair skill default"))?;
            if p.get("Modifier").and_then(Value::as_str) != Some("kCharModRepairAmount") {
                return Err(invalid("unexpected repair skill modifier"));
            }
            repair_modifiers.insert(id, scalar(p, "ModifierValue")?);
        }
        let mut profiles = BTreeMap::new();
        for row in field(&raw, "profiles")?
            .as_array()
            .ok_or_else(|| invalid("invalid structure profiles"))?
        {
            if row.get("vehicle_type").and_then(Value::as_str) != Some("DefensiveEmplacement") {
                continue;
            }
            let faction = integer(row, "faction")?;
            if faction > 2 {
                return Err(invalid("invalid structure faction"));
            }
            let damage = field(row, "damage_properties")?;
            let max_health = positive(damage, "MaxHitpoints")?;
            // Shared damage wire has 12 quarter-health bits. No saturating catalog repair.
            if max_health < health_rules.dead_health_floor || max_health > 1023.75 {
                return Err(invalid("structure health exceeds wire bounds"));
            }
            let armor_type = integer(damage, "ArmorType")?;
            if armor_type != 0 && usize::from(armor_type) > armor.len() {
                return Err(invalid("unresolved structure armor type"));
            }
            let repaired_health_percent = number(damage, "RepairedHealthPct")?;
            if repaired_health_percent > 100.0 {
                return Err(invalid("invalid repaired health percentage"));
            }
            let repair_seconds = scalar(damage, "TimeToRepairInSeconds")?;
            if repair_seconds == 0.0 {
                return Err(invalid("zero structure repair duration"));
            }
            let profile = StructureProfile {
                faction,
                model: string(row, "model")?,
                properties_source: string(row, "properties_source")?,
                max_health,
                armor_type,
                critical_loss_percent_per_second: number(damage, "DamageCriticalHealthLossRate")?,
                can_repair_destroyed: field(damage, "CanBeRepairedFromDestroyed")?
                    .as_bool()
                    .ok_or_else(|| invalid("invalid repair-from-destroyed flag"))?,
                repaired_health_percent,
                repair_seconds,
            };
            if profiles.insert(faction, profile).is_some() {
                return Err(invalid("duplicate structure faction"));
            }
        }
        if profiles.len() != 3 {
            return Err(invalid("missing authored faction emplacement profile"));
        }
        Ok(Self {
            profiles,
            health_rules,
            friendly_fire_multiplier,
            armor,
            body,
            repair,
            repair_modifiers,
        })
    }
    pub fn profile(&self, faction: u8) -> Result<&StructureProfile, &'static str> {
        self.profiles
            .get(&faction)
            .ok_or("unsupported_structure_faction")
    }
    pub fn profile_for_source(&self, source: &str) -> Result<&StructureProfile, &'static str> {
        self.profiles
            .values()
            .find(|p| p.properties_source == source)
            .ok_or("unsupported_structure_properties")
    }
    pub fn friendly_fire_multiplier(&self) -> f32 {
        self.friendly_fire_multiplier
    }
    pub fn rules(&self) -> NativeStructureRules {
        self.health_rules
    }
    pub fn repair(&self) -> RepairParameters {
        self.repair
    }
    pub fn repair_modifiers(&self) -> &BTreeMap<u8, f32> {
        &self.repair_modifiers
    }
    pub fn armor(&self, armor_type: u8) -> Result<ArmorParameters, &'static str> {
        // Native initialization444330 decrements nonzero types; zero stays row0.
        self.armor
            .get(usize::from(armor_type.saturating_sub(1)))
            .copied()
            .ok_or("unsupported_vehicle_armor_type")
    }
    pub fn body(&self, hit_location: u8) -> Result<BodyParameters, &'static str> {
        self.body
            .get(usize::from(hit_location))
            .copied()
            .ok_or("unsupported_vehicle_hit_location")
    }
    pub fn repair_amount(
        &self,
        profile: &StructureProfile,
        modifier: f32,
    ) -> Result<f32, &'static str> {
        if !modifier.is_finite()
            || modifier < -1.0
            || !profile.max_health.is_finite()
            || profile.max_health <= 0.0
            || !profile.repair_seconds.is_finite()
            || profile.repair_seconds == 0.0
        {
            return Err("invalid_vehicle_repair_parameters");
        }
        // SimCharacterRepair scales cooldown with vmaddfp before converting
        // timed repairs to HP. SimVehicleHeal then converts percent back to HP.
        let amount = if profile.repair_seconds < 0.0 {
            self.repair.amount.mul_add(modifier, self.repair.amount)
        } else {
            let rate = self
                .repair
                .cooldown_seconds
                .mul_add(modifier, self.repair.cooldown_seconds);
            (profile.max_health * rate) / profile.repair_seconds
        };
        let percent = (amount * 100.0_f32) / profile.max_health;
        let amount = profile.max_health * (percent / 100.0_f32);
        if !amount.is_finite() || amount < 0.0 {
            return Err("invalid_vehicle_repair_amount");
        }
        Ok(amount)
    }
}
