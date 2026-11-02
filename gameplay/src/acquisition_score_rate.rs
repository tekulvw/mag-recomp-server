//! Native score-rate request primitive, separate from winner rules/catch-up clocks.
use crate::acquisition_control::UnboundCall;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request {
    pub frequency_ms: u32,
    pub amount: u32,
    pub team: u8,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rate {
    pub frequency_ms: u32,
    pub amount: u32,
    pub deadline_ms: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Deduction {
    pub team: u8,
    pub amount: u32,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Rates {
    rates: [Rate; 2],
}
impl Request {
    /// Restricted admitted domain: integer graph arguments, nonnegative rate and
    /// amount, teams0/1, and a finite frequency fitting u32. Native accepts other
    /// Lua numbers; their out-of-range conversion behavior is not inferred here.
    pub fn from_call(call: &UnboundCall) -> Result<Self, &'static str> {
        if call.symbol != "Game_SetScoreBleed" {
            return Err("unsupported_acquisition_call");
        }
        let [seconds, amount, team] = call.arguments.as_slice() else {
            return Err("invalid_score_rate_arguments");
        };
        if *seconds < 0 || *amount < 0 || !(0..=1).contains(team) {
            return Err("unsupported_score_rate_arguments");
        }
        let frequency_ms = (*seconds as f32) * 1000.0_f32;
        if !frequency_ms.is_finite() || f64::from(frequency_ms) >= 4294967296.0 {
            return Err("unsupported_score_rate_conversion");
        }
        Ok(Self {
            frequency_ms: frequency_ms as u32,
            amount: (*amount as f32) as u32,
            team: *team as u8,
        })
    }
}
impl Rates {
    pub fn rates(&self) -> &[Rate; 2] {
        &self.rates
    }
    /// Configure only; never deduct immediately. Preserve native unsigned comparison
    /// and wrap, including zero's deadline-disabled meaning.
    pub fn configure(&mut self, request: Request, now_ms: u32) -> Result<(), &'static str> {
        let Some(rate) = self.rates.get_mut(usize::from(request.team)) else {
            return Err("invalid_score_rate_team");
        };
        rate.frequency_ms = request.frequency_ms;
        rate.amount = request.amount;
        if request.frequency_ms == 0 || request.amount == 0 {
            rate.deadline_ms = 0;
        } else {
            let candidate = now_ms.wrapping_add(request.frequency_ms);
            if rate.deadline_ms == 0 || candidate < rate.deadline_ms {
                rate.deadline_ms = candidate;
            }
        }
        Ok(())
    }
    /// One native Update opportunity. Produce at most one deduction per team,
    /// strictly after its nonzero deadline; do not coalesce elapsed intervals.
    pub fn due(&mut self, now_ms: u32) -> [Option<Deduction>; 2] {
        let mut deductions = [None, None];
        for (team, rate) in self.rates.iter_mut().enumerate() {
            if rate.deadline_ms != 0 && rate.deadline_ms < now_ms {
                deductions[team] = Some(Deduction {
                    team: team as u8,
                    amount: rate.amount,
                });
                rate.deadline_ms = rate.deadline_ms.wrapping_add(rate.frequency_ms);
            }
        }
        deductions
    }
}
