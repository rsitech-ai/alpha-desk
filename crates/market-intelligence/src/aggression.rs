use domain_types::{EntityId, UsdAmount};
use serde::{Deserialize, Serialize};

use crate::{
    MarketError,
    flow::{RiskFlowKind, SmartFlowContribution},
    math::{product_ppm, scale_usd_by_ppm},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AggressionLeg {
    OpenLong,
    CloseShort,
    OpenShort,
    CloseLong,
}

impl AggressionLeg {
    #[must_use]
    pub const fn as_wire_name(self) -> &'static str {
        match self {
            Self::OpenLong => "open_long",
            Self::CloseShort => "close_short",
            Self::OpenShort => "open_short",
            Self::CloseLong => "close_long",
        }
    }

    #[must_use]
    pub const fn from_kind(kind: RiskFlowKind) -> Option<Self> {
        match kind {
            RiskFlowKind::OpenLong | RiskFlowKind::AddLong => Some(Self::OpenLong),
            RiskFlowKind::CloseShort => Some(Self::CloseShort),
            RiskFlowKind::OpenShort | RiskFlowKind::AddShort => Some(Self::OpenShort),
            RiskFlowKind::CloseLong => Some(Self::CloseLong),
            RiskFlowKind::ReduceLong | RiskFlowKind::ReduceShort | RiskFlowKind::Static => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AggressionTotals {
    pub open_long: UsdAmount,
    pub close_short: UsdAmount,
    pub open_short: UsdAmount,
    pub close_long: UsdAmount,
    pub informed_open_minus_open: UsdAmount,
}

pub fn informed_taker_aggression(
    contributions: &[SmartFlowContribution],
) -> Result<AggressionTotals, MarketError> {
    if contributions.is_empty() {
        return Err(MarketError::InsufficientHistory { what: "aggression" });
    }
    let scale = contributions[0].notional_usd.scale();
    let mut open_long = UsdAmount::from_raw(0, scale)?;
    let mut close_short = UsdAmount::from_raw(0, scale)?;
    let mut open_short = UsdAmount::from_raw(0, scale)?;
    let mut close_long = UsdAmount::from_raw(0, scale)?;
    for contribution in contributions {
        let Some(leg) = AggressionLeg::from_kind(contribution.kind) else {
            continue;
        };
        let weight = product_ppm(&[
            contribution.skill_probability,
            contribution.independence_weight,
            contribution.data_confidence,
            contribution.intent_adjustment,
        ])?;
        let weighted = scale_usd_by_ppm(contribution.notional_usd, weight)?;
        match leg {
            AggressionLeg::OpenLong => open_long = open_long.checked_add(weighted)?,
            AggressionLeg::CloseShort => close_short = close_short.checked_add(weighted)?,
            AggressionLeg::OpenShort => open_short = open_short.checked_add(weighted)?,
            AggressionLeg::CloseLong => close_long = close_long.checked_add(weighted)?,
        }
    }
    let informed = open_long.checked_sub(open_short)?;
    Ok(AggressionTotals {
        open_long,
        close_short,
        open_short,
        close_long,
        informed_open_minus_open: informed,
    })
}

#[must_use]
pub fn aggression_subjects(contributions: &[SmartFlowContribution]) -> Vec<EntityId> {
    contributions
        .iter()
        .map(|contribution| contribution.subject.clone())
        .collect()
}
