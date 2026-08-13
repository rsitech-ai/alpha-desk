use domain_types::{ProbabilityPpm, ProtocolTime};
use serde::{Deserialize, Serialize};

use crate::{IntelligenceError, math::allocate_ppm};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum StyleClass {
    DirectionalDiscretionary,
    MomentumTrend,
    MeanReversion,
    Scalping,
    SwingTrading,
    MarketMaking,
    BasisSpotPerpArbitrage,
    FundingCarryCapture,
    LiquidationTrading,
    PortfolioHedge,
    VaultStrategy,
    AutomatedFollower,
    UnclassifiedMixed,
}

impl StyleClass {
    pub const ALL: [Self; 13] = [
        Self::DirectionalDiscretionary,
        Self::MomentumTrend,
        Self::MeanReversion,
        Self::Scalping,
        Self::SwingTrading,
        Self::MarketMaking,
        Self::BasisSpotPerpArbitrage,
        Self::FundingCarryCapture,
        Self::LiquidationTrading,
        Self::PortfolioHedge,
        Self::VaultStrategy,
        Self::AutomatedFollower,
        Self::UnclassifiedMixed,
    ];

    #[must_use]
    pub const fn as_wire_name(self) -> &'static str {
        match self {
            Self::DirectionalDiscretionary => "directional_discretionary",
            Self::MomentumTrend => "momentum_trend",
            Self::MeanReversion => "mean_reversion",
            Self::Scalping => "scalping",
            Self::SwingTrading => "swing_trading",
            Self::MarketMaking => "market_making",
            Self::BasisSpotPerpArbitrage => "basis_spot_perp_arbitrage",
            Self::FundingCarryCapture => "funding_carry_capture",
            Self::LiquidationTrading => "liquidation_trading",
            Self::PortfolioHedge => "portfolio_hedge",
            Self::VaultStrategy => "vault_strategy",
            Self::AutomatedFollower => "automated_follower",
            Self::UnclassifiedMixed => "unclassified_mixed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StyleFeatures {
    pub maker_ratio_ppm: Option<u32>,
    pub turnover_ppm: Option<u32>,
    pub hold_period_micros: Option<u64>,
    pub inventory_reversion_ppm: Option<u32>,
    pub directional_beta_milli: Option<i32>,
    pub funding_sensitivity_ppm: Option<u32>,
    pub spot_perp_offset_ppm: Option<u32>,
    pub sync_activity_ppm: Option<u32>,
    pub response_lag_micros: Option<u64>,
    pub liquidation_flag: Option<bool>,
    pub vault_flag: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StyleSnapshot {
    pub effective_at: ProtocolTime,
    pub probabilities: Vec<(StyleClass, ProbabilityPpm)>,
    pub missing_critical_inputs: bool,
}

pub fn classify_style(
    features: &StyleFeatures,
    effective_at: ProtocolTime,
) -> Result<StyleSnapshot, IntelligenceError> {
    let missing = features.maker_ratio_ppm.is_none() || features.turnover_ppm.is_none();
    let mut weights = [1_u128; 13];
    if missing {
        weights[style_index(StyleClass::UnclassifiedMixed)] = 80;
    }
    if let Some(maker) = features.maker_ratio_ppm
        && maker >= 700_000
    {
        bump(&mut weights, StyleClass::MarketMaking, 40);
    }
    if let Some(turnover) = features.turnover_ppm {
        if turnover >= 800_000 {
            bump(&mut weights, StyleClass::Scalping, 30);
        } else if turnover <= 150_000 {
            bump(&mut weights, StyleClass::SwingTrading, 25);
        }
    }
    if let Some(hold) = features.hold_period_micros {
        if hold <= 5_000_000 {
            bump(&mut weights, StyleClass::Scalping, 20);
        } else if hold >= 3_600_000_000 {
            bump(&mut weights, StyleClass::SwingTrading, 20);
        }
    }
    if let Some(reversion) = features.inventory_reversion_ppm
        && reversion >= 700_000
    {
        bump(&mut weights, StyleClass::MarketMaking, 25);
        bump(&mut weights, StyleClass::MeanReversion, 10);
    }
    if let Some(beta) = features.directional_beta_milli
        && beta.unsigned_abs() >= 800
    {
        bump(&mut weights, StyleClass::DirectionalDiscretionary, 25);
        bump(&mut weights, StyleClass::MomentumTrend, 15);
    }
    if features.funding_sensitivity_ppm.unwrap_or(0) >= 700_000 {
        bump(&mut weights, StyleClass::FundingCarryCapture, 35);
    }
    if features.spot_perp_offset_ppm.unwrap_or(0) >= 700_000 {
        bump(&mut weights, StyleClass::BasisSpotPerpArbitrage, 35);
    }
    if features.sync_activity_ppm.unwrap_or(0) >= 700_000
        && features.response_lag_micros.unwrap_or(u64::MAX) <= 2_000_000
    {
        bump(&mut weights, StyleClass::AutomatedFollower, 40);
    }
    if features.liquidation_flag == Some(true) {
        bump(&mut weights, StyleClass::LiquidationTrading, 50);
    }
    if features.vault_flag == Some(true) {
        bump(&mut weights, StyleClass::VaultStrategy, 50);
    }
    let allocated = allocate_ppm(&weights)?;
    let probabilities = StyleClass::ALL
        .into_iter()
        .zip(allocated)
        .collect::<Vec<_>>();
    let total: u32 = probabilities.iter().map(|(_, ppm)| ppm.ppm()).sum();
    if total != 1_000_000 {
        return Err(IntelligenceError::Malformed {
            what: "style",
            reason: "probabilities must sum to 1_000_000 ppm",
        });
    }
    Ok(StyleSnapshot {
        effective_at,
        probabilities,
        missing_critical_inputs: missing,
    })
}

fn bump(weights: &mut [u128; 13], class: StyleClass, amount: u128) {
    weights[style_index(class)] = weights[style_index(class)].saturating_add(amount);
}

fn style_index(class: StyleClass) -> usize {
    match class {
        StyleClass::DirectionalDiscretionary => 0,
        StyleClass::MomentumTrend => 1,
        StyleClass::MeanReversion => 2,
        StyleClass::Scalping => 3,
        StyleClass::SwingTrading => 4,
        StyleClass::MarketMaking => 5,
        StyleClass::BasisSpotPerpArbitrage => 6,
        StyleClass::FundingCarryCapture => 7,
        StyleClass::LiquidationTrading => 8,
        StyleClass::PortfolioHedge => 9,
        StyleClass::VaultStrategy => 10,
        StyleClass::AutomatedFollower => 11,
        StyleClass::UnclassifiedMixed => 12,
    }
}
