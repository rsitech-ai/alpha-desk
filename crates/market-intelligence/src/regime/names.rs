use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum RegimeName {
    QuietRange,
    VolatileRange,
    OrderlyUptrend,
    OrderlyDowntrend,
    LeveragedUptrend,
    LeveragedDowntrend,
    LiquidityStress,
    PostLiquidationRecovery,
}

pub type MarketRegime = RegimeName;

impl RegimeName {
    pub const ALL: [Self; 8] = [
        Self::QuietRange,
        Self::VolatileRange,
        Self::OrderlyUptrend,
        Self::OrderlyDowntrend,
        Self::LeveragedUptrend,
        Self::LeveragedDowntrend,
        Self::LiquidityStress,
        Self::PostLiquidationRecovery,
    ];

    #[must_use]
    pub const fn as_wire_name(self) -> &'static str {
        match self {
            Self::QuietRange => "quiet_range",
            Self::VolatileRange => "volatile_range",
            Self::OrderlyUptrend => "orderly_uptrend",
            Self::OrderlyDowntrend => "orderly_downtrend",
            Self::LeveragedUptrend => "leveraged_uptrend",
            Self::LeveragedDowntrend => "leveraged_downtrend",
            Self::LiquidityStress => "liquidity_stress",
            Self::PostLiquidationRecovery => "post_liquidation_recovery",
        }
    }

    pub fn parse_wire(value: &str) -> Result<Self, crate::MarketError> {
        match value {
            "quiet_range" => Ok(Self::QuietRange),
            "volatile_range" => Ok(Self::VolatileRange),
            "orderly_uptrend" => Ok(Self::OrderlyUptrend),
            "orderly_downtrend" => Ok(Self::OrderlyDowntrend),
            "leveraged_uptrend" => Ok(Self::LeveragedUptrend),
            "leveraged_downtrend" => Ok(Self::LeveragedDowntrend),
            "liquidity_stress" => Ok(Self::LiquidityStress),
            "post_liquidation_recovery" => Ok(Self::PostLiquidationRecovery),
            _ => Err(crate::MarketError::Unsupported {
                what: "regime_name",
            }),
        }
    }
}
