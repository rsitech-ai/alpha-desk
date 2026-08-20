use domain_types::{
    BasisPoints, BlockHeight, ClosedInterval, Direction, FeatureSetVersion, Horizon, KnownTime,
    MarketId, ModelVersion, ProbabilityPpm, ProtocolTime, SignalId, UsdAmount,
};
use feature_core::HealthAssessment;
use serde::{Deserialize, Serialize};

use crate::SignalError;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SignalType {
    IndependentSmartFlowAcceleration,
    SmartCrowdDivergence,
    LiquidationFragilityAsymmetry,
    ResearchOnly(String),
}

impl SignalType {
    #[must_use]
    pub const fn as_wire_name(&self) -> &'static str {
        match self {
            Self::IndependentSmartFlowAcceleration => "independent_smart_flow_acceleration",
            Self::SmartCrowdDivergence => "smart_crowd_divergence",
            Self::LiquidationFragilityAsymmetry => "liquidation_fragility_asymmetry",
            Self::ResearchOnly(_) => "research_only",
        }
    }

    #[must_use]
    pub fn can_enter_live(&self) -> bool {
        match self {
            Self::IndependentSmartFlowAcceleration
            | Self::SmartCrowdDivergence
            | Self::LiquidationFragilityAsymmetry => true,
            Self::ResearchOnly(_) => false,
        }
    }

    pub fn research_only(identifier: impl Into<String>) -> Result<Self, SignalError> {
        let identifier = identifier.into();
        if identifier.trim().is_empty() || identifier.trim() != identifier {
            return Err(SignalError::EmptyIdentifier {
                field: "research_only_id",
            });
        }
        Ok(Self::ResearchOnly(identifier))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SignalLifecycleState {
    Candidate,
    Validated,
    Live,
    Decaying,
    Invalidated,
    Expired,
    Resolved,
}

impl SignalLifecycleState {
    #[must_use]
    pub const fn as_wire_name(self) -> &'static str {
        match self {
            Self::Candidate => "candidate",
            Self::Validated => "validated",
            Self::Live => "live",
            Self::Decaying => "decaying",
            Self::Invalidated => "invalidated",
            Self::Expired => "expired",
            Self::Resolved => "resolved",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SignalConfirmationClass {
    CommittedPrimary,
    CommittedIndependent,
    ProvisionalSource,
    SyntheticUnqualified,
}

impl SignalConfirmationClass {
    #[must_use]
    pub const fn as_wire_name(self) -> &'static str {
        match self {
            Self::CommittedPrimary => "committed_primary",
            Self::CommittedIndependent => "committed_independent",
            Self::ProvisionalSource => "provisional_source",
            Self::SyntheticUnqualified => "synthetic_unqualified",
        }
    }

    #[must_use]
    pub const fn can_enter_live(self) -> bool {
        match self {
            Self::CommittedPrimary | Self::CommittedIndependent => true,
            Self::ProvisionalSource | Self::SyntheticUnqualified => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SignalActor {
    System,
    ResearchRole,
    RiskRole,
    PlatformRole,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signal {
    pub signal_id: SignalId,
    pub signal_type: SignalType,
    pub market_id: MarketId,
    pub direction: Direction,
    pub created_at: KnownTime,
    pub effective_at: ProtocolTime,
    pub as_of_block: BlockHeight,
    pub confirmation_class: SignalConfirmationClass,
    pub horizon: Horizon,
    pub expected_return_bps: BasisPoints,
    pub expected_cost_bps: BasisPoints,
    pub net_edge_bps: BasisPoints,
    pub confidence: ProbabilityPpm,
    pub confidence_interval_bps: ClosedInterval<BasisPoints>,
    pub capacity: UsdAmount,
    pub half_life: Horizon,
    pub crowding: ProbabilityPpm,
    pub tail_risk_bps: BasisPoints,
    pub data_health: HealthAssessment,
    pub model_version: ModelVersion,
    pub feature_set_version: FeatureSetVersion,
    pub evidence_bundle_hash: [u8; 32],
    pub invalidation_rules_hash: [u8; 32],
    pub lifecycle_state: SignalLifecycleState,
}

impl Signal {
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        signal_id: SignalId,
        signal_type: SignalType,
        market_id: MarketId,
        direction: Direction,
        created_at: KnownTime,
        effective_at: ProtocolTime,
        as_of_block: BlockHeight,
        confirmation_class: SignalConfirmationClass,
        horizon: Horizon,
        expected_return_bps: BasisPoints,
        expected_cost_bps: BasisPoints,
        confidence: ProbabilityPpm,
        confidence_interval_bps: ClosedInterval<BasisPoints>,
        capacity: UsdAmount,
        half_life: Horizon,
        crowding: ProbabilityPpm,
        tail_risk_bps: BasisPoints,
        data_health: HealthAssessment,
        model_version: ModelVersion,
        feature_set_version: FeatureSetVersion,
        evidence_bundle_hash: [u8; 32],
        invalidation_rules_hash: [u8; 32],
        lifecycle_state: SignalLifecycleState,
    ) -> Result<Self, SignalError> {
        if created_at.unix_micros() < effective_at.unix_micros() {
            return Err(SignalError::ContractViolation(
                "created_at precedes effective_at",
            ));
        }
        if evidence_bundle_hash.iter().all(|byte| *byte == 0)
            || invalidation_rules_hash.iter().all(|byte| *byte == 0)
        {
            return Err(SignalError::IncompleteEvidence(vec![
                "zero_evidence_or_rule_hash".to_owned(),
            ]));
        }
        match direction {
            Direction::Flat => {
                return Err(SignalError::ContractViolation(
                    "signals must be directional",
                ));
            }
            Direction::Long | Direction::Short => {}
        }
        if expected_cost_bps.raw() < 0 {
            return Err(SignalError::ContractViolation("cost must be non-negative"));
        }
        let net_edge_bps = BasisPoints::from_raw(
            expected_return_bps
                .raw()
                .checked_sub(expected_cost_bps.raw())
                .ok_or(SignalError::Overflow)?,
            expected_return_bps.scale(),
        )?;
        match lifecycle_state {
            SignalLifecycleState::Live => {
                if !signal_type.can_enter_live() {
                    return Err(SignalError::ResearchOnlyCannotGoLive);
                }
                if !confirmation_class.can_enter_live() {
                    return Err(SignalError::ContractViolation(
                        "synthetic or provisional confirmation cannot enter live",
                    ));
                }
            }
            SignalLifecycleState::Candidate
            | SignalLifecycleState::Validated
            | SignalLifecycleState::Decaying
            | SignalLifecycleState::Invalidated
            | SignalLifecycleState::Expired
            | SignalLifecycleState::Resolved => {}
        }
        Ok(Self {
            signal_id,
            signal_type,
            market_id,
            direction,
            created_at,
            effective_at,
            as_of_block,
            confirmation_class,
            horizon,
            expected_return_bps,
            expected_cost_bps,
            net_edge_bps,
            confidence,
            confidence_interval_bps,
            capacity,
            half_life,
            crowding,
            tail_risk_bps,
            data_health,
            model_version,
            feature_set_version,
            evidence_bundle_hash,
            invalidation_rules_hash,
            lifecycle_state,
        })
    }
}
