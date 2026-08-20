use domain_types::BasisPoints;
use serde::{Deserialize, Serialize};

use crate::error::SimError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlippageModel {
    extra_bps: BasisPoints,
}

impl SlippageModel {
    pub fn new(extra_bps: BasisPoints) -> Result<Self, SimError> {
        if extra_bps.raw() < 0 {
            return Err(SimError::UnmodeledCost {
                component: "negative_slippage",
            });
        }
        Ok(Self { extra_bps })
    }

    #[must_use]
    pub const fn extra_bps(self) -> BasisPoints {
        self.extra_bps
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImpactModel {
    bps_at_full_depth: BasisPoints,
}

impl ImpactModel {
    pub fn new(bps_at_full_depth: BasisPoints) -> Result<Self, SimError> {
        if bps_at_full_depth.raw() < 0 {
            return Err(SimError::UnmodeledCost {
                component: "negative_impact",
            });
        }
        Ok(Self { bps_at_full_depth })
    }

    #[must_use]
    pub const fn bps_at_full_depth(self) -> BasisPoints {
        self.bps_at_full_depth
    }
}
