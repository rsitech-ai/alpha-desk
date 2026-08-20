use domain_types::{
    BasisPoints, ClosedInterval, Direction, FeatureSetVersion, Horizon, ModelVersion,
    ProbabilityPpm, SignalId,
};
use market_intelligence::MarketFeatureSnapshot;

use crate::{
    SignalError,
    families::{
        FamilyThresholds, SignalContext, SignalEvaluation, SignalEvaluator, independent_vote_count,
        signed_feature, suppress_if_red,
    },
    signal::{Signal, SignalConfirmationClass, SignalLifecycleState, SignalType},
};

pub struct SmartFlowAccelerationEvaluator {
    pub thresholds: FamilyThresholds,
}

impl SmartFlowAccelerationEvaluator {
    pub fn from_toml(text: &str) -> Result<Self, SignalError> {
        Ok(Self {
            thresholds: FamilyThresholds::from_toml(text)?,
        })
    }
}

impl SignalEvaluator for SmartFlowAccelerationEvaluator {
    fn signal_type(&self) -> SignalType {
        SignalType::IndependentSmartFlowAcceleration
    }

    fn evaluate(
        &self,
        snapshot: &MarketFeatureSnapshot,
        context: &SignalContext,
    ) -> Result<SignalEvaluation, SignalError> {
        if let Some(suppressed) = suppress_if_red(snapshot, context) {
            return Ok(suppressed);
        }
        let mut reasons = Vec::new();
        if independent_vote_count(context)? < self.thresholds.min_independent_entities {
            reasons.push("insufficient_independent_entities".to_owned());
        }
        if signed_feature(snapshot, "smart_flow_acceleration_milli")?
            < self.thresholds.min_acceleration_milli
        {
            reasons.push("acceleration_below_threshold".to_owned());
        }
        if signed_feature(snapshot, "historical_markout_bps")? < self.thresholds.min_markout_bps {
            reasons.push("historical_markout_not_supportive".to_owned());
        }
        if context.executable_capacity.raw() <= 0 {
            reasons.push("insufficient_capacity".to_owned());
        }
        if context.crowding.raw_value.raw() > i128::from(self.thresholds.max_crowding_ppm) {
            reasons.push("crowding_consumed".to_owned());
        }
        if context.execution_cost_bps.raw() >= i128::from(self.thresholds.min_markout_bps.max(0)) {
            reasons.push("cost_consumes_edge".to_owned());
        }
        if context.follower_dominated {
            reasons.push("follower_dominated".to_owned());
        }
        if !reasons.is_empty() {
            return Ok(SignalEvaluation::NoSignal { reasons });
        }
        Ok(SignalEvaluation::Candidate(Box::new(candidate_signal(
            SignalType::IndependentSmartFlowAcceleration,
            snapshot,
            context,
            Direction::Long,
        )?)))
    }
}

pub(crate) fn candidate_signal(
    signal_type: SignalType,
    snapshot: &MarketFeatureSnapshot,
    context: &SignalContext,
    direction: Direction,
) -> Result<Signal, SignalError> {
    let crowding = ProbabilityPpm::from_ppm(
        u32::try_from(context.crowding.raw_value.raw().clamp(0, 1_000_000))
            .map_err(|_| SignalError::Overflow)?,
    )?;
    Signal::try_new(
        SignalId::new("candidate")?,
        signal_type,
        snapshot.market_id.clone(),
        direction,
        snapshot.known_at,
        snapshot.effective_at,
        snapshot.input_watermark,
        SignalConfirmationClass::SyntheticUnqualified,
        snapshot.horizon,
        BasisPoints::from_raw(40, 0)?,
        context.execution_cost_bps,
        context.historical_support,
        ClosedInterval::new(BasisPoints::from_raw(10, 0)?, BasisPoints::from_raw(80, 0)?)?,
        context.executable_capacity,
        Horizon::MINUTES_5,
        crowding,
        BasisPoints::from_raw(25, 0)?,
        snapshot.health.clone(),
        ModelVersion::new("signals-v1")?,
        FeatureSetVersion::new(snapshot.feature_set_version.as_str())?,
        snapshot.provenance_hash,
        snapshot.provenance_hash,
        SignalLifecycleState::Candidate,
    )
}
