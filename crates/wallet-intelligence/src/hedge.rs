use domain_types::ProbabilityPpm;
use feature_core::EvidenceRef;
use serde::{Deserialize, Serialize};

use crate::{IntelligenceError, math::require_ppm};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HedgeEvidence {
    pub opposing_spot_perp: bool,
    pub correlated_opposite_positions: bool,
    pub synchronized_changes: bool,
    pub funding_sensitivity: bool,
    pub low_net_beta_high_turnover: bool,
    pub market_maker_inventory_reversion: bool,
    pub linked_account_activity: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HedgeAssessment {
    pub on_platform_hedge_probability: ProbabilityPpm,
    pub external_hedge_uncertainty: ProbabilityPpm,
    pub evidence: Vec<EvidenceRef>,
    pub limitations: Vec<String>,
}

pub fn assess_hedge(
    evidence: HedgeEvidence,
    evidence_refs: Vec<EvidenceRef>,
) -> Result<HedgeAssessment, IntelligenceError> {
    let flags = [
        evidence.opposing_spot_perp,
        evidence.correlated_opposite_positions,
        evidence.synchronized_changes,
        evidence.funding_sensitivity,
        evidence.low_net_beta_high_turnover,
        evidence.market_maker_inventory_reversion,
        evidence.linked_account_activity,
    ];
    let hits = flags.iter().filter(|flag| **flag).count();
    let on_platform = u32::try_from(hits)
        .ok()
        .and_then(|count| count.checked_mul(120_000))
        .ok_or(IntelligenceError::Overflow)?
        .min(950_000);
    let mut limitations = vec!["off_platform_hedges_unobservable".to_owned()];
    if hits == 0 {
        limitations.push("no_on_platform_hedge_evidence".to_owned());
    }
    Ok(HedgeAssessment {
        on_platform_hedge_probability: require_ppm(on_platform)?,
        external_hedge_uncertainty: require_ppm(1_000_000 - on_platform.min(800_000))?,
        evidence: evidence_refs,
        limitations,
    })
}
