use domain_types::Direction;
use market_intelligence::MarketFeatureSnapshot;

use crate::{
    SignalError,
    families::{
        FamilyThresholds, SignalContext, SignalEvaluation, SignalEvaluator, independent_vote_count,
        signed_feature, smart_flow_acceleration::candidate_signal, suppress_if_red,
    },
    signal::SignalType,
};

pub struct SmartCrowdDivergenceEvaluator {
    pub thresholds: FamilyThresholds,
}

impl SmartCrowdDivergenceEvaluator {
    pub fn from_toml(text: &str) -> Result<Self, SignalError> {
        Ok(Self {
            thresholds: FamilyThresholds::from_toml(text)?,
        })
    }
}

impl SignalEvaluator for SmartCrowdDivergenceEvaluator {
    fn signal_type(&self) -> SignalType {
        SignalType::SmartCrowdDivergence
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
        let smart = signed_feature(snapshot, "smart_flow_usd_milli")?;
        let crowd = signed_feature(snapshot, "crowd_flow_usd_milli")?;
        if smart.signum() == 0 || crowd.signum() == 0 || smart.signum() == crowd.signum() {
            reasons.push("flows_not_opposite".to_owned());
        }
        if smart.abs() < self.thresholds.min_divergence_usd_raw
            || crowd.abs() < self.thresholds.min_divergence_usd_raw
        {
            reasons.push("sample_too_small".to_owned());
        }
        if context.smart_intent_explained_by_mm {
            reasons.push("market_maker_explains_smart_side".to_owned());
        }
        if context.follower_dominated {
            reasons.push("smart_side_follower_dominated".to_owned());
        }
        if !reasons.is_empty() {
            return Ok(SignalEvaluation::NoSignal { reasons });
        }
        let direction = if smart > 0 {
            Direction::Long
        } else {
            Direction::Short
        };
        Ok(SignalEvaluation::Candidate(Box::new(candidate_signal(
            SignalType::SmartCrowdDivergence,
            snapshot,
            context,
            direction,
        )?)))
    }
}
