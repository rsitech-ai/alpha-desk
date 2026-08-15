use domain_types::CalibrationStatus;
use serde::{Deserialize, Serialize};

use crate::IntelligenceError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillPrior {
    pub version: String,
    pub mu0_bps: i64,
    pub kappa0_milli: u64,
    pub min_ess_milli: u64,
    pub half_life_micros: u64,
}

impl SkillPrior {
    pub fn from_toml(text: &str) -> Result<Self, IntelligenceError> {
        let prior: Self = toml::from_str(text).map_err(|_| IntelligenceError::Malformed {
            what: "skill_prior",
            reason: "toml parse failed",
        })?;
        prior.validate()?;
        Ok(prior)
    }

    pub fn validate(&self) -> Result<(), IntelligenceError> {
        if self.version.trim().is_empty() {
            return Err(IntelligenceError::EmptyIdentifier {
                field: "skill_prior.version",
            });
        }
        if self.kappa0_milli == 0 || self.min_ess_milli == 0 || self.half_life_micros == 0 {
            return Err(IntelligenceError::Malformed {
                what: "skill_prior",
                reason: "kappa, min ess, and half-life must be positive",
            });
        }
        Ok(())
    }

    #[must_use]
    pub fn calibration(&self, ess_milli: u64) -> CalibrationStatus {
        if ess_milli < self.min_ess_milli {
            CalibrationStatus::InsufficientEvidence
        } else {
            CalibrationStatus::UnderReview
        }
    }
}
