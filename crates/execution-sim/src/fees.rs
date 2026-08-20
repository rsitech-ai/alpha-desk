use domain_types::{FeeRate, FundingRate};
use serde::{Deserialize, Serialize};

use crate::error::SimError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeeSchedule {
    taker_fee_rate: FeeRate,
    maker_fee_rate: FeeRate,
}

impl FeeSchedule {
    pub fn new(taker_fee_rate: FeeRate, maker_fee_rate: FeeRate) -> Result<Self, SimError> {
        if taker_fee_rate.raw() < 0 || maker_fee_rate.raw() < 0 {
            return Err(SimError::UnmodeledCost {
                component: "negative_fee_rate",
            });
        }
        Ok(Self {
            taker_fee_rate,
            maker_fee_rate,
        })
    }

    #[must_use]
    pub const fn taker_fee_rate(&self) -> FeeRate {
        self.taker_fee_rate
    }

    #[must_use]
    pub const fn maker_fee_rate(&self) -> FeeRate {
        self.maker_fee_rate
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FundingSchedule {
    interval_micros: u64,
    rate: FundingRate,
}

impl FundingSchedule {
    pub fn new(interval_micros: u64, rate: FundingRate) -> Result<Self, SimError> {
        if interval_micros == 0 {
            return Err(SimError::UnmodeledCost {
                component: "funding_interval",
            });
        }
        Ok(Self {
            interval_micros,
            rate,
        })
    }

    #[must_use]
    pub const fn interval_micros(&self) -> u64 {
        self.interval_micros
    }

    #[must_use]
    pub const fn rate(&self) -> FundingRate {
        self.rate
    }
}
