use domain_types::ProtocolTime;
use serde::{Deserialize, Serialize};

use crate::IntelligenceError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservedHoldInterval {
    pub opened_at: ProtocolTime,
    pub closed_at: ProtocolTime,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HoldingTimeDistribution {
    pub sample_count: u64,
    pub min_micros: u64,
    pub max_micros: u64,
    pub median_micros: u64,
    pub total_micros: u128,
}

impl ObservedHoldInterval {
    pub fn try_new(
        opened_at: ProtocolTime,
        closed_at: ProtocolTime,
    ) -> Result<Self, IntelligenceError> {
        let interval = Self {
            opened_at,
            closed_at,
        };
        interval.duration_micros()?;
        Ok(interval)
    }

    pub fn duration_micros(&self) -> Result<u64, IntelligenceError> {
        let delta = self
            .closed_at
            .unix_micros()
            .checked_sub(self.opened_at.unix_micros())
            .ok_or(IntelligenceError::Overflow)?;
        if delta < 0 {
            return Err(IntelligenceError::Malformed {
                what: "holding_time",
                reason: "close before open",
            });
        }
        u64::try_from(delta).map_err(|_| IntelligenceError::Overflow)
    }
}

pub fn holding_time_distribution(
    intervals: &[ObservedHoldInterval],
) -> Result<HoldingTimeDistribution, IntelligenceError> {
    if intervals.is_empty() {
        return Err(IntelligenceError::InsufficientHistory {
            what: "holding_time",
        });
    }
    let mut durations = Vec::with_capacity(intervals.len());
    let mut total = 0_u128;
    for interval in intervals {
        let duration = interval.duration_micros()?;
        durations.push(duration);
        total = total
            .checked_add(u128::from(duration))
            .ok_or(IntelligenceError::Overflow)?;
    }
    durations.sort_unstable();
    let last = durations.len() - 1;
    Ok(HoldingTimeDistribution {
        sample_count: u64::try_from(durations.len()).map_err(|_| IntelligenceError::Overflow)?,
        min_micros: durations[0],
        max_micros: durations[last],
        median_micros: durations[last / 2],
        total_micros: total,
    })
}
