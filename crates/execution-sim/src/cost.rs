use serde::{Deserialize, Serialize};

use crate::error::SimError;
use crate::fees::{FeeSchedule, FundingSchedule};
use crate::impact::{ImpactModel, SlippageModel};
use crate::latency::LatencyAssumptions;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CostModel {
    version: String,
    fees: FeeSchedule,
    funding: FundingSchedule,
    slippage: SlippageModel,
    impact: ImpactModel,
    latency: LatencyAssumptions,
}

impl CostModel {
    pub fn new(
        version: impl Into<String>,
        fees: FeeSchedule,
        funding: FundingSchedule,
        slippage: SlippageModel,
        impact: ImpactModel,
        latency: LatencyAssumptions,
    ) -> Result<Self, SimError> {
        let version = version.into();
        if version.trim().is_empty() {
            return Err(SimError::UnmodeledCost {
                component: "cost_model_version",
            });
        }
        Ok(Self {
            version,
            fees,
            funding,
            slippage,
            impact,
            latency,
        })
    }

    pub fn validate(&self) -> Result<(), SimError> {
        if self.version.trim().is_empty() {
            return Err(SimError::UnmodeledCost {
                component: "cost_model_version",
            });
        }
        FeeSchedule::new(self.fees.taker_fee_rate(), self.fees.maker_fee_rate())?;
        FundingSchedule::new(self.funding.interval_micros(), self.funding.rate())?;
        SlippageModel::new(self.slippage.extra_bps())?;
        ImpactModel::new(self.impact.bps_at_full_depth())?;
        Ok(())
    }

    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }

    #[must_use]
    pub const fn fees(&self) -> &FeeSchedule {
        &self.fees
    }

    #[must_use]
    pub const fn funding(&self) -> &FundingSchedule {
        &self.funding
    }

    #[must_use]
    pub const fn slippage(&self) -> SlippageModel {
        self.slippage
    }

    #[must_use]
    pub const fn impact(&self) -> ImpactModel {
        self.impact
    }

    #[must_use]
    pub const fn latency(&self) -> LatencyAssumptions {
        self.latency
    }
}
