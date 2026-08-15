use domain_types::{
    CalibrationStatus, Decimal, KnownTime, ModelVersion, ProbabilityPpm, ProtocolTime,
};
use feature_core::FeatureKey;
use serde::{Deserialize, Serialize};
use wallet_intelligence::ApplicabilitySupport;

use crate::{
    MarketError,
    regime::{RegimeAssessment, RegimeFeatureVector, RegimeName, probabilities_from_weights},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegimeModel {
    pub version: ModelVersion,
    pub min_dwell_micros: u64,
    pub vol_quiet_milli: i64,
    pub vol_stress_milli: i64,
    pub leverage_oi_milli: i64,
    pub liq_recovery_ppm: u32,
}

impl RegimeModel {
    pub fn from_toml(text: &str) -> Result<Self, MarketError> {
        let raw: RawModel = toml::from_str(text).map_err(|_| MarketError::Malformed {
            what: "regime_model",
            reason: "toml parse failed",
        })?;
        if raw.version.trim().is_empty() || raw.min_dwell_micros == 0 {
            return Err(MarketError::Malformed {
                what: "regime_model",
                reason: "version and min dwell are required",
            });
        }
        Ok(Self {
            version: ModelVersion::new(raw.version)?,
            min_dwell_micros: raw.min_dwell_micros,
            vol_quiet_milli: raw.vol_quiet_milli,
            vol_stress_milli: raw.vol_stress_milli,
            leverage_oi_milli: raw.leverage_oi_milli,
            liq_recovery_ppm: raw.liq_recovery_ppm,
        })
    }
}

#[derive(Deserialize)]
struct RawModel {
    version: String,
    min_dwell_micros: u64,
    vol_quiet_milli: i64,
    vol_stress_milli: i64,
    leverage_oi_milli: i64,
    liq_recovery_ppm: u32,
}

pub fn classify_regime(
    model: &RegimeModel,
    features: Option<&RegimeFeatureVector>,
    previous: Option<&RegimeAssessment>,
    effective_at: ProtocolTime,
    known_at: KnownTime,
) -> Result<RegimeAssessment, MarketError> {
    let Some(features) = features else {
        return unsupported(model, effective_at, known_at, "missing_required_inputs");
    };
    if features.liquidity_quality_ppm == 0 {
        return unsupported(model, effective_at, known_at, "outside_training_support");
    }
    let mut weights = [1_u128; 8];
    if features.realized_vol_milli <= model.vol_quiet_milli
        && features.trend_milli.unsigned_abs() < 250
    {
        weights[index(RegimeName::QuietRange)] = 40;
    }
    if features.realized_vol_milli >= model.vol_stress_milli
        && features.trend_milli.unsigned_abs() < 400
        && features.liquidation_intensity_ppm < 400_000
    {
        weights[index(RegimeName::VolatileRange)] = 35;
    }
    if features.trend_milli > 250 && features.oi_change_milli < model.leverage_oi_milli {
        weights[index(RegimeName::OrderlyUptrend)] = 30;
    }
    if features.trend_milli < -250 && features.oi_change_milli < model.leverage_oi_milli {
        weights[index(RegimeName::OrderlyDowntrend)] = 30;
    }
    if features.trend_milli > 250 && features.oi_change_milli >= model.leverage_oi_milli {
        weights[index(RegimeName::LeveragedUptrend)] = 35;
    }
    if features.trend_milli < -250 && features.oi_change_milli >= model.leverage_oi_milli {
        weights[index(RegimeName::LeveragedDowntrend)] = 35;
    }
    if features.liquidity_quality_ppm < 250_000 || features.correlation_stress_ppm > 700_000 {
        weights[index(RegimeName::LiquidityStress)] = 45;
    }
    if features.liquidation_intensity_ppm >= model.liq_recovery_ppm {
        weights[index(RegimeName::PostLiquidationRecovery)] = 40;
    }
    let mut probabilities = probabilities_from_weights(&weights)?;
    if let Some(previous) = previous {
        let elapsed = known_at
            .unix_micros()
            .checked_sub(previous.known_at.unix_micros())
            .ok_or(MarketError::Overflow)?;
        if elapsed < i64::try_from(model.min_dwell_micros).map_err(|_| MarketError::Overflow)? {
            let prior = previous.dominant()?;
            let boost = probabilities
                .get(&prior)
                .map(|value| u128::from(value.ppm()).saturating_add(200_000))
                .unwrap_or(200_000);
            weights[index(prior)] = weights[index(prior)].saturating_add(boost);
            probabilities = probabilities_from_weights(&weights)?;
        }
    }
    let ranked: Vec<_> = {
        let mut pairs: Vec<_> = probabilities.iter().collect();
        pairs.sort_by_key(|(_, ppm)| std::cmp::Reverse(ppm.ppm()));
        pairs
    };
    let top = ranked[0].1.ppm();
    let second = ranked[1].1.ppm();
    let change = ProbabilityPpm::from_ppm((1_000_000 - top.saturating_sub(second)).min(1_000_000))?;
    let contributions = vec![
        (
            FeatureKey::try_new("regime", "trend_milli", 1)?,
            Decimal::from_raw(i128::from(features.trend_milli), 0)?,
        ),
        (
            FeatureKey::try_new("regime", "realized_vol_milli", 1)?,
            Decimal::from_raw(i128::from(features.realized_vol_milli), 0)?,
        ),
        (
            FeatureKey::try_new("regime", "liquidation_intensity_ppm", 1)?,
            Decimal::from_raw(i128::from(features.liquidation_intensity_ppm), 0)?,
        ),
    ];
    RegimeAssessment::try_new(
        probabilities,
        change,
        CalibrationStatus::UnderReview,
        ApplicabilitySupport::Supported,
        contributions,
        effective_at,
        known_at,
        model.version.clone(),
    )
}

fn unsupported(
    model: &RegimeModel,
    effective_at: ProtocolTime,
    known_at: KnownTime,
    _reason: &'static str,
) -> Result<RegimeAssessment, MarketError> {
    let weights = [1_u128; 8];
    let probabilities = probabilities_from_weights(&weights)?;
    RegimeAssessment::try_new(
        probabilities,
        ProbabilityPpm::from_ppm(1_000_000)?,
        CalibrationStatus::InsufficientEvidence,
        ApplicabilitySupport::Unsupported,
        Vec::new(),
        effective_at,
        known_at,
        model.version.clone(),
    )
}

const fn index(name: RegimeName) -> usize {
    match name {
        RegimeName::QuietRange => 0,
        RegimeName::VolatileRange => 1,
        RegimeName::OrderlyUptrend => 2,
        RegimeName::OrderlyDowntrend => 3,
        RegimeName::LeveragedUptrend => 4,
        RegimeName::LeveragedDowntrend => 5,
        RegimeName::LiquidityStress => 6,
        RegimeName::PostLiquidationRecovery => 7,
    }
}
