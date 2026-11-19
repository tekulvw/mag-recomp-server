//! Acquisition score state and the common score-depletion lifecycle.
use crate::acquisition_score_rate::Rates;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rules {
    pub starting_points: [i16; 2],
    pub major_percent: i32,
    pub end_delay_ms: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScoreIntent {
    pub team: u8,
    pub delta: i16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Playing,
    Ending { deadline_ms: u32 },
    Finished,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResultNotice {
    pub winner: u8,
    pub major: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transition {
    EndGame(ResultNotice),
    Finished,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Round {
    rules: Rules,
    points: [i16; 2],
    rates: Rates,
    phase: Phase,
}

impl Round {
    pub fn new(rules: Rules) -> Self {
        Self {
            rules,
            points: rules.starting_points,
            rates: Rates::default(),
            phase: Phase::Playing,
        }
    }

    pub fn points(&self) -> [i16; 2] {
        self.points
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn rates_mut(&mut self) -> &mut Rates {
        &mut self.rates
    }

    /// Return the absolute score for publication when an intent is accepted.
    pub fn apply_score(&mut self, intent: ScoreIntent) -> Result<Option<i16>, &'static str> {
        let points = self
            .points
            .get_mut(usize::from(intent.team))
            .ok_or("invalid_acquisition_score_team")?;
        Ok(change_points(points, intent.delta))
    }

    /// Process one score-bleed update. The caller supplies the schedule relative
    /// to the separate match-state update.
    pub fn update_bleed(&mut self, now_ms: u32) -> [Option<i16>; 2] {
        let mut changes = [None; 2];
        for deduction in self.rates.due(now_ms).into_iter().flatten() {
            let team = usize::from(deduction.team);
            let delta = (deduction.amount as i16).wrapping_neg();
            changes[team] = change_points(&mut self.points[team], delta);
        }
        changes
    }

    /// A depletion at nonzero network time starts Ending in this update. At
    /// network time zero the terminal epoch retains its unset sentinel.
    pub fn update_end(&mut self, now_ms: u32) -> Option<Transition> {
        match self.phase {
            Phase::Playing if now_ms != 0 && self.points.contains(&0) => {
                let winner = usize::from(self.points[0] == 0);
                let fraction =
                    f32::from(self.points[winner]) / f32::from(self.rules.starting_points[winner]);
                let major = fraction >= self.rules.major_percent as f32 * 0.01_f32;
                self.phase = Phase::Ending {
                    deadline_ms: now_ms.wrapping_add(self.rules.end_delay_ms),
                };
                Some(Transition::EndGame(ResultNotice {
                    winner: winner as u8,
                    major,
                }))
            }
            Phase::Ending { deadline_ms } if now_ms >= deadline_ms => {
                self.phase = Phase::Finished;
                Some(Transition::Finished)
            }
            _ => None,
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new(self.rules);
    }
}

fn change_points(points: &mut i16, delta: i16) -> Option<i16> {
    if delta == 0 || (delta > 0 && *points == 0) {
        return None;
    }
    *points = points.wrapping_add(delta).max(0);
    Some(*points)
}
