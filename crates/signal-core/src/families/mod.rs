use domain_types::{BasisPoints, EntityId, ProbabilityPpm, UsdAmount};
use feature_core::{FeatureValue, HealthAssessment, HealthState};
use market_intelligence::{
    FragilityResult, MarketFeatureSnapshot, ObservationAdmission, ObservationMintKind,
    RegimeAssessment, ScoredDimension,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use wallet_intelligence::WalletIntelligenceVector;

use crate::{SignalError, signal::Signal};

mod fragility_asymmetry;
mod smart_crowd_divergence;
mod smart_flow_acceleration;

pub use fragility_asymmetry::FragilityAsymmetryEvaluator;
pub use smart_crowd_divergence::SmartCrowdDivergenceEvaluator;
pub use smart_flow_acceleration::SmartFlowAccelerationEvaluator;

use crate::signal::SignalType;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignalContext {
    pub wallet_intelligence: Vec<WalletIntelligenceVector>,
    pub independence_weights: BTreeMap<EntityId, ProbabilityPpm>,
    pub execution_cost_bps: BasisPoints,
    pub executable_capacity: UsdAmount,
    pub regime: RegimeAssessment,
    pub crowding: ScoredDimension,
    pub fragility: FragilityResult,
    pub historical_support: ProbabilityPpm,
    pub required_health: HealthAssessment,
    pub book_health: HealthAssessment,
    pub originator_ids: Vec<EntityId>,
    pub smart_intent_explained_by_mm: bool,
    pub follower_dominated: bool,
}

pub trait SignalEvaluator {
    fn signal_type(&self) -> SignalType;
    fn evaluate(
        &self,
        snapshot: &MarketFeatureSnapshot,
        context: &SignalContext,
    ) -> Result<SignalEvaluation, SignalError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SignalEvaluation {
    Candidate(Box<Signal>),
    NoSignal {
        reasons: Vec<String>,
    },
    Suppressed {
        health: HealthAssessment,
        reasons: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FamilyThresholds {
    pub min_independent_entities: u32,
    pub min_acceleration_milli: i64,
    pub min_markout_bps: i64,
    pub max_crowding_ppm: u32,
    pub min_divergence_usd_raw: i64,
    pub min_asymmetry_usd_raw: i64,
}

impl FamilyThresholds {
    pub fn from_toml(text: &str) -> Result<Self, SignalError> {
        toml::from_str(text).map_err(|_| SignalError::ContractViolation("family toml"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProofWithholdReason {
    MissingBookOrFills,
    MissingInventory,
    MalformedInventory,
}

impl ProofWithholdReason {
    #[must_use]
    pub const fn as_wire_name(self) -> &'static str {
        match self {
            Self::MissingBookOrFills => "missing_book_or_fills",
            Self::MissingInventory => "missing_inventory",
            Self::MalformedInventory => "malformed_inventory",
        }
    }
}

#[must_use]
pub fn proof_withhold_reason(snapshot: &MarketFeatureSnapshot) -> Option<ProofWithholdReason> {
    match snapshot.admit_observation("book", ObservationMintKind::DecimalDepth) {
        ObservationAdmission::Observed => {}
        ObservationAdmission::Missing | ObservationAdmission::Malformed { .. } => {
            return Some(ProofWithholdReason::MissingBookOrFills);
        }
    }
    match snapshot.admit_observation("fills", ObservationMintKind::BooleanPresence) {
        ObservationAdmission::Observed => {}
        ObservationAdmission::Missing | ObservationAdmission::Malformed { .. } => {
            return Some(ProofWithholdReason::MissingBookOrFills);
        }
    }
    match snapshot.admit_observation("inventory", ObservationMintKind::DecimalDepth) {
        ObservationAdmission::Observed => None,
        ObservationAdmission::Missing => Some(ProofWithholdReason::MissingInventory),
        ObservationAdmission::Malformed { .. } => Some(ProofWithholdReason::MalformedInventory),
    }
}

pub fn suppress_missing_book_or_fills(
    snapshot: &MarketFeatureSnapshot,
) -> Option<SignalEvaluation> {
    let reason = proof_withhold_reason(snapshot)?;
    Some(SignalEvaluation::Suppressed {
        health: snapshot.health.clone(),
        reasons: vec![reason.as_wire_name().to_owned()],
    })
}

pub fn suppress_if_red(
    snapshot: &MarketFeatureSnapshot,
    context: &SignalContext,
) -> Option<SignalEvaluation> {
    if snapshot.health.state == HealthState::Red
        || context.book_health.state == HealthState::Red
        || context.required_health.state == HealthState::Red
        || context.fragility.low.health.state == HealthState::Red
    {
        return Some(SignalEvaluation::Suppressed {
            health: snapshot.health.clone(),
            reasons: vec!["red_required_dependency".to_owned()],
        });
    }
    suppress_missing_book_or_fills(snapshot)
}

pub fn signed_feature(snapshot: &MarketFeatureSnapshot, name: &str) -> Result<i64, SignalError> {
    let key = market_intelligence::market_feature_key(name)?;
    match snapshot.values.get(&key) {
        Some(FeatureValue::SignedInteger(value)) => Ok(*value),
        Some(FeatureValue::Missing(_)) | None => {
            Err(SignalError::IncompleteEvidence(vec![name.to_owned()]))
        }
        Some(_) => Err(SignalError::ContractViolation("unexpected feature kind")),
    }
}

pub fn independent_vote_count(context: &SignalContext) -> Result<u32, SignalError> {
    let sum = context
        .independence_weights
        .values()
        .try_fold(0_u64, |acc, weight| {
            acc.checked_add(u64::from(weight.ppm()))
        })
        .ok_or(SignalError::Overflow)?;
    u32::try_from((sum + 500_000) / 1_000_000).map_err(|_| SignalError::Overflow)
}
