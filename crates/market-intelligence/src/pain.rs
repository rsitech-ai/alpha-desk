use domain_types::{Direction, ProbabilityPpm};
use serde::{Deserialize, Serialize};

use crate::MarketError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PainState {
    Profitable,
    NearBreakEven,
    Underwater,
    VoluntaryExitPressure,
    NearLiquidation,
    Unknown,
}

impl PainState {
    #[must_use]
    pub const fn as_wire_name(self) -> &'static str {
        match self {
            Self::Profitable => "profitable",
            Self::NearBreakEven => "near_break_even",
            Self::Underwater => "underwater",
            Self::VoluntaryExitPressure => "voluntary_exit_pressure",
            Self::NearLiquidation => "near_liquidation",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PainThresholds {
    pub near_break_even_bps: i64,
    pub voluntary_age_micros: u64,
    pub near_liquidation_bps: i64,
}

impl PainThresholds {
    pub fn from_toml(text: &str) -> Result<Self, MarketError> {
        let raw: RawThresholds = toml::from_str(text).map_err(|_| MarketError::Malformed {
            what: "pain_thresholds",
            reason: "toml parse failed",
        })?;
        if raw.near_break_even_bps <= 0
            || raw.near_liquidation_bps <= 0
            || raw.voluntary_age_micros == 0
        {
            return Err(MarketError::Malformed {
                what: "pain_thresholds",
                reason: "thresholds must be positive",
            });
        }
        Ok(Self {
            near_break_even_bps: raw.near_break_even_bps,
            voluntary_age_micros: raw.voluntary_age_micros,
            near_liquidation_bps: raw.near_liquidation_bps,
        })
    }
}

#[derive(Deserialize)]
struct RawThresholds {
    near_break_even_bps: i64,
    voluntary_age_micros: u64,
    near_liquidation_bps: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PainObservation {
    pub side: Direction,
    pub pnl_bps: Option<i64>,
    pub liquidation_distance_bps: Option<i64>,
    pub age_micros: u64,
    pub margin_known: bool,
}

pub fn classify_pain(observation: PainObservation, thresholds: &PainThresholds) -> PainState {
    if !observation.margin_known {
        return PainState::Unknown;
    }
    let Some(distance) = observation.liquidation_distance_bps else {
        return PainState::Unknown;
    };
    if distance <= thresholds.near_liquidation_bps {
        return PainState::NearLiquidation;
    }
    let Some(pnl) = observation.pnl_bps else {
        return PainState::Unknown;
    };
    let signed = match observation.side {
        Direction::Long | Direction::Flat => pnl,
        Direction::Short => pnl,
    };
    if signed.abs() <= thresholds.near_break_even_bps {
        return PainState::NearBreakEven;
    }
    if signed < 0 {
        if observation.age_micros >= thresholds.voluntary_age_micros {
            PainState::VoluntaryExitPressure
        } else {
            PainState::Underwater
        }
    } else {
        PainState::Profitable
    }
}

pub fn pain_confidence(state: PainState) -> Result<ProbabilityPpm, MarketError> {
    let ppm = match state {
        PainState::Unknown => 0,
        PainState::NearBreakEven | PainState::Underwater => 600_000,
        PainState::Profitable | PainState::VoluntaryExitPressure => 750_000,
        PainState::NearLiquidation => 850_000,
    };
    ProbabilityPpm::from_ppm(ppm).map_err(Into::into)
}
