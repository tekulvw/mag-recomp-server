//! Round-owned Company strategic state. World work commits with its owning engine candidate.
use crate::{
    active_abilities::{Rules, Side},
    active_ability_state::NativeAbilityData,
};

pub const STRATEGIC_SLOTS: usize = 9;
const TEAMS: usize = 2;
const SQUADS_PER_TEAM: u16 = 16;
pub const SENSOR_HOLD_SECONDS: f32 = 5.;

#[repr(u16)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StrategicAbility {
    SensorSweep = 1,
    Blockade = 3,
    SignalsJamming = 6,
    RapidRedeploy = 7,
    TacticalStrike = 8,
}

impl StrategicAbility {
    const ALL: [Self; 5] = [
        Self::SensorSweep,
        Self::Blockade,
        Self::SignalsJamming,
        Self::RapidRedeploy,
        Self::TacticalStrike,
    ];

    pub fn from_code(code: u16) -> Result<Self, &'static str> {
        Self::ALL
            .into_iter()
            .find(|ability| ability.code() == code)
            .ok_or("unsupported_strategic_ability")
    }

    pub const fn code(self) -> u16 {
        self as u16
    }

    const fn slot(self) -> usize {
        self as usize - 1
    }

    pub const fn mask(self) -> u16 {
        1 << self.slot()
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::SensorSweep => "SensorSweep",
            Self::Blockade => "Blockade",
            Self::SignalsJamming => "SignalsJamming",
            Self::RapidRedeploy => "RapidRedeploy",
            Self::TacticalStrike => "TacticalStrike",
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Property {
    cooldown_ms: u32,
    duration_ms: u32,
    spawn_modifier: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CompanySnapshot {
    pub active_mask: u16,
    pub available_mask: u16,
    pub data: [Option<NativeAbilityData>; STRATEGIC_SLOTS],
}

#[derive(Clone, Copy, Debug, Default)]
struct Company {
    last_ms: u32,
    scan_slot: u16,
    snapshot: CompanySnapshot,
}

impl Company {
    fn active(self, ability: StrategicAbility) -> bool {
        self.snapshot.active_mask & ability.mask() != 0
    }

    fn last_ability_ms(self, ability: StrategicAbility) -> u32 {
        self.snapshot.data[ability.slot()].map_or(0, |data| data.activate_time_ms)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SensorScan {
    pub squad: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StrategicStart {
    None,
    RefreshCommands { team: u8 },
}

#[derive(Clone, Debug)]
pub struct StrategicState {
    companies: [Company; TEAMS],
    properties: [Option<Property>; STRATEGIC_SLOTS],
    global_cooldown_ms: u32,
}

impl StrategicState {
    /// Enable the native Company rules only when the complete retail strategic
    /// tables, including both signed spawn modifiers, are present.
    pub fn new(rules: &Rules) -> Result<Self, &'static str> {
        let global_cooldown_ms = cooldown(rules.strategic_global_cooldown_ms)?;
        let mut properties = [None; STRATEGIC_SLOTS];
        for ability in StrategicAbility::ALL {
            let mut matching = rules
                .strategic
                .iter()
                .filter(|row| row.ability == ability.name());
            let row = matching
                .next()
                .ok_or("missing_strategic_effect_properties")?;
            if matching.next().is_some() {
                return Err("duplicate_strategic_effect_properties");
            }
            let cooldown_ms = cooldown(row.cooldown_ms)?;
            let duration_ms = u32::try_from(row.duration_ms)
                .ok()
                .filter(|&ms| ms > 0 && ms <= cooldown_ms)
                .ok_or("invalid_strategic_duration")?;
            let spawn_modifier = match (ability, row.spawn_interval_modifier) {
                (StrategicAbility::Blockade, Some(value))
                    if value.is_finite() && value > 0. && value <= 1. =>
                {
                    value
                }
                (StrategicAbility::RapidRedeploy, Some(value))
                    if value.is_finite() && value > -1. && value < 0. =>
                {
                    value
                }
                (StrategicAbility::Blockade | StrategicAbility::RapidRedeploy, _) => {
                    return Err("invalid_strategic_spawn_modifier");
                }
                (_, None) => 0.,
                _ => return Err("invalid_strategic_spawn_modifier"),
            };
            properties[ability.slot()] = Some(Property {
                cooldown_ms,
                duration_ms,
                spawn_modifier,
            });
        }

        let mut companies = [Company::default(); TEAMS];
        let common = StrategicAbility::SignalsJamming.mask()
            | StrategicAbility::SensorSweep.mask()
            | StrategicAbility::TacticalStrike.mask();
        for mode in &rules.modes {
            if mode.players != 256 {
                if !mode.strategic.is_empty() {
                    return Err("unsupported_strategic_mode_inventory");
                }
                continue;
            }
            let (team, exclusive) = match mode.side {
                Side::Attacker => (0, StrategicAbility::Blockade),
                Side::Defender => (1, StrategicAbility::RapidRedeploy),
            };
            let mask = mode.strategic.iter().try_fold(0, |mask, name| {
                let bit = StrategicAbility::ALL
                    .iter()
                    .find(|ability| name == ability.name())
                    .ok_or("unsupported_strategic_mode_inventory")?
                    .mask();
                if mask & bit != 0 {
                    return Err("duplicate_strategic_mode_ability");
                }
                Ok(mask | bit)
            })?;
            if mask != common | exclusive.mask() || companies[team].snapshot.available_mask != 0 {
                return Err("unsupported_strategic_mode_inventory");
            }
            companies[team].snapshot.available_mask = mask;
        }
        if companies
            .iter()
            .any(|company| company.snapshot.available_mask == 0)
        {
            return Err("missing_strategic_mode_inventory");
        }
        Ok(Self {
            companies,
            properties,
            global_cooldown_ms,
        })
    }

    pub fn company(&self, team: u8) -> Option<CompanySnapshot> {
        self.companies
            .get(usize::from(team))
            .map(|company| company.snapshot)
    }

    pub fn commands_locked(&self, team: u8) -> bool {
        self.companies
            .get(usize::from(team ^ 1))
            .is_some_and(|company| company.active(StrategicAbility::SignalsJamming))
    }

    pub fn spawn_interval_modifier(&self, team: u8) -> f32 {
        let Some(company) = self.companies.get(usize::from(team)) else {
            return 0.;
        };
        let mut value = 0.;
        if company.active(StrategicAbility::RapidRedeploy) {
            value += self
                .property(StrategicAbility::RapidRedeploy)
                .spawn_modifier;
        }
        if self.companies[usize::from(team ^ 1)].active(StrategicAbility::Blockade) {
            value += self.property(StrategicAbility::Blockade).spawn_modifier;
        }
        value
    }

    /// Validate the Company clock and accept one authenticated order. The
    /// caller installs this state and any immediate work atomically.
    pub fn activate(
        &mut self,
        game_type: u16,
        team: u8,
        ability: StrategicAbility,
        now_ms: u32,
    ) -> Result<StrategicStart, &'static str> {
        if game_type != 2 {
            return Err("strategic_mode_unavailable");
        }
        let index = usize::from(team);
        let company = *self.companies.get(index).ok_or("invalid_strategic_team")?;
        if company.snapshot.available_mask & ability.mask() == 0 {
            return Err("strategic_ability_unavailable");
        }
        if self.commands_locked(team) {
            return Err("strategic_commands_jammed");
        }
        if !ready(company.last_ms, now_ms, self.global_cooldown_ms) {
            return Err("strategic_global_cooldown");
        }
        let last_ms = company.last_ability_ms(ability);
        if last_ms != 0
            && last_ms < now_ms
            && !ready(last_ms, now_ms, self.property(ability).cooldown_ms)
        {
            return Err("strategic_ability_cooldown");
        }
        let company = &mut self.companies[index];
        company.last_ms = now_ms;
        company.snapshot.active_mask |= ability.mask();
        company.snapshot.data[ability.slot()] = Some(NativeAbilityData {
            activate_time_ms: now_ms,
            enabled_at_start: true,
            ..Default::default()
        });
        if ability == StrategicAbility::SensorSweep {
            company.scan_slot = 0;
        }
        Ok(if ability == StrategicAbility::TacticalStrike {
            StrategicStart::RefreshCommands { team }
        } else {
            StrategicStart::None
        })
    }

    /// Each native update scans at most one opposing squad per Company.
    /// Sensor Sweep can scan on the invocation that stops its active instance.
    pub fn prepare_tick(
        &self,
        now_ms: u32,
    ) -> Result<(Self, [Option<SensorScan>; TEAMS]), &'static str> {
        let mut next = self.clone();
        let mut scans = [None; TEAMS];
        for (team, scan) in scans.iter_mut().enumerate() {
            let company = &mut next.companies[team];
            for ability in StrategicAbility::ALL {
                if company.active(ability)
                    && now_ms
                        > company
                            .last_ability_ms(ability)
                            .wrapping_add(self.property(ability).duration_ms)
                {
                    company.snapshot.active_mask &= !ability.mask();
                }
            }
            if self.companies[team].active(StrategicAbility::SensorSweep) {
                *scan = Some(SensorScan {
                    squad: (team as u16 ^ 1) * SQUADS_PER_TEAM + company.scan_slot,
                });
                company.scan_slot = (company.scan_slot + 1) % SQUADS_PER_TEAM;
            }
        }
        Ok((next, scans))
    }

    fn property(&self, ability: StrategicAbility) -> Property {
        self.properties[ability.slot()].expect("complete strategic property table")
    }
}

pub fn spawn_interval_ms(base_ms: u32, modifier: f32) -> u32 {
    let base = base_ms as f32;
    base.mul_add(modifier, base) as u32
}

fn cooldown(value: u64) -> Result<u32, &'static str> {
    u32::try_from(value)
        .ok()
        .filter(|&ms| ms > 0 && ms <= u32::MAX / 100)
        .ok_or("invalid_strategic_cooldown")
}

fn ready(last_ms: u32, now_ms: u32, duration_ms: u32) -> bool {
    now_ms.wrapping_sub(last_ms).wrapping_mul(100) / duration_ms >= 100
}
