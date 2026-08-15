use domain_types::ProbabilityPpm;
use serde::{Deserialize, Serialize};

use crate::MarketError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ConvictionComponent {
    PositionEquityDelta,
    LeverageChange,
    AggressiveSpreadCrossing,
    PrecedingCapitalActivation,
    AdditionsThroughAdverseMove,
    ConcentrationChange,
    PersistenceVersusHold,
    VisibleHedgeEvidence,
}

impl ConvictionComponent {
    pub const ALL: [Self; 8] = [
        Self::PositionEquityDelta,
        Self::LeverageChange,
        Self::AggressiveSpreadCrossing,
        Self::PrecedingCapitalActivation,
        Self::AdditionsThroughAdverseMove,
        Self::ConcentrationChange,
        Self::PersistenceVersusHold,
        Self::VisibleHedgeEvidence,
    ];

    #[must_use]
    pub const fn as_wire_name(self) -> &'static str {
        match self {
            Self::PositionEquityDelta => "position_equity_delta",
            Self::LeverageChange => "leverage_change",
            Self::AggressiveSpreadCrossing => "aggressive_spread_crossing",
            Self::PrecedingCapitalActivation => "preceding_capital_activation",
            Self::AdditionsThroughAdverseMove => "additions_through_adverse_move",
            Self::ConcentrationChange => "concentration_change",
            Self::PersistenceVersusHold => "persistence_versus_hold",
            Self::VisibleHedgeEvidence => "visible_hedge_evidence",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConvictionSnapshot {
    pub components: Vec<(ConvictionComponent, ProbabilityPpm)>,
    pub combined: ProbabilityPpm,
}

impl ConvictionSnapshot {
    pub fn try_new(
        components: Vec<(ConvictionComponent, ProbabilityPpm)>,
    ) -> Result<Self, MarketError> {
        if components.len() != ConvictionComponent::ALL.len() {
            return Err(MarketError::Malformed {
                what: "conviction",
                reason: "all eight components are required",
            });
        }
        let mut seen = [false; 8];
        let mut acc = 0_u64;
        for (component, value) in &components {
            let index = ConvictionComponent::ALL
                .iter()
                .position(|candidate| candidate == component)
                .ok_or(MarketError::Unsupported {
                    what: "conviction_component",
                })?;
            if seen[index] {
                return Err(MarketError::Malformed {
                    what: "conviction",
                    reason: "duplicate component",
                });
            }
            seen[index] = true;
            acc = acc
                .checked_add(u64::from(value.ppm()))
                .ok_or(MarketError::Overflow)?;
        }
        if seen.iter().any(|flag| !*flag) {
            return Err(MarketError::Malformed {
                what: "conviction",
                reason: "missing component",
            });
        }
        let combined =
            ProbabilityPpm::from_ppm(u32::try_from(acc / 8).map_err(|_| MarketError::Overflow)?)?;
        Ok(Self {
            components,
            combined,
        })
    }
}
