use domain_types::{KnownTime, ModelVersion, ProbabilityPpm, ProtocolTime};
use feature_core::FeatureKey;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use wallet_intelligence::ApplicabilitySupport;

use crate::{MarketError, math::allocate_ppm};

mod features;
mod model;
mod names;

pub use features::RegimeFeatureVector;
pub use model::{RegimeModel, classify_regime};
pub use names::{MarketRegime, RegimeName};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegimeAssessment {
    pub probabilities: BTreeMap<RegimeName, ProbabilityPpm>,
    pub change_probability: ProbabilityPpm,
    pub calibration: domain_types::CalibrationStatus,
    pub support: ApplicabilitySupport,
    pub contributions: Vec<(FeatureKey, domain_types::Decimal)>,
    pub effective_at: ProtocolTime,
    pub known_at: KnownTime,
    pub model_version: ModelVersion,
}

impl RegimeAssessment {
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        probabilities: BTreeMap<RegimeName, ProbabilityPpm>,
        change_probability: ProbabilityPpm,
        calibration: domain_types::CalibrationStatus,
        support: ApplicabilitySupport,
        contributions: Vec<(FeatureKey, domain_types::Decimal)>,
        effective_at: ProtocolTime,
        known_at: KnownTime,
        model_version: ModelVersion,
    ) -> Result<Self, MarketError> {
        if known_at.unix_micros() < effective_at.unix_micros() {
            return Err(MarketError::Malformed {
                what: "regime_assessment",
                reason: "known_at precedes effective_at",
            });
        }
        if probabilities.len() != RegimeName::ALL.len() {
            return Err(MarketError::Malformed {
                what: "regime_assessment",
                reason: "all eight regimes required",
            });
        }
        let mut sum = 0_u64;
        for name in RegimeName::ALL {
            let ppm = probabilities.get(&name).ok_or(MarketError::Malformed {
                what: "regime_assessment",
                reason: "missing regime probability",
            })?;
            sum = sum
                .checked_add(u64::from(ppm.ppm()))
                .ok_or(MarketError::Overflow)?;
        }
        if sum != 1_000_000 {
            return Err(MarketError::Malformed {
                what: "regime_assessment",
                reason: "probabilities must sum to 1000000 ppm",
            });
        }
        Ok(Self {
            probabilities,
            change_probability,
            calibration,
            support,
            contributions,
            effective_at,
            known_at,
            model_version,
        })
    }

    pub fn dominant(&self) -> Result<RegimeName, MarketError> {
        self.probabilities
            .iter()
            .max_by_key(|(_, ppm)| ppm.ppm())
            .map(|(name, _)| *name)
            .ok_or(MarketError::Malformed {
                what: "regime_assessment",
                reason: "empty probabilities",
            })
    }
}

pub fn probabilities_from_weights(
    weights: &[u128; 8],
) -> Result<BTreeMap<RegimeName, ProbabilityPpm>, MarketError> {
    let allocated = allocate_ppm(weights)?;
    Ok(RegimeName::ALL.into_iter().zip(allocated).collect())
}
