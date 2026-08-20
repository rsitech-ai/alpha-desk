use domain_types::Direction;
use feature_core::HealthState;
use market_intelligence::MarketFeatureSnapshot;

use crate::{
    SignalError,
    families::{
        FamilyThresholds, SignalContext, SignalEvaluation, SignalEvaluator,
        smart_flow_acceleration::candidate_signal, suppress_if_red,
    },
    signal::SignalType,
};

pub struct FragilityAsymmetryEvaluator {
    pub thresholds: FamilyThresholds,
}

impl FragilityAsymmetryEvaluator {
    pub fn from_toml(text: &str) -> Result<Self, SignalError> {
        Ok(Self {
            thresholds: FamilyThresholds::from_toml(text)?,
        })
    }
}

impl SignalEvaluator for FragilityAsymmetryEvaluator {
    fn signal_type(&self) -> SignalType {
        SignalType::LiquidationFragilityAsymmetry
    }

    fn evaluate(
        &self,
        snapshot: &MarketFeatureSnapshot,
        context: &SignalContext,
    ) -> Result<SignalEvaluation, SignalError> {
        if let Some(suppressed) = suppress_if_red(snapshot, context) {
            return Ok(suppressed);
        }
        if context
            .fragility
            .missing_inputs
            .iter()
            .any(|item| item == "book" || item == "margin_model")
            || context.fragility.base.health.state == HealthState::Red
        {
            return Ok(SignalEvaluation::Suppressed {
                health: context.fragility.base.health.clone(),
                reasons: vec!["unsupported_margin_or_book".to_owned()],
            });
        }
        let mut reasons = Vec::new();
        let long = context.fragility.base.total_forced_notional.raw();
        let short = context
            .fragility
            .high
            .total_forced_notional
            .raw()
            .max(context.fragility.low.total_forced_notional.raw());
        let asymmetry = (long - short).abs();
        if asymmetry < i128::from(self.thresholds.min_asymmetry_usd_raw) {
            reasons.push("asymmetry_below_policy".to_owned());
        }
        if context.fragility.base.absorbed_notional.raw()
            >= context.fragility.base.total_forced_notional.raw()
            && context.fragility.base.total_forced_notional.raw() > 0
            && context.executable_capacity.raw()
                >= context.fragility.base.total_forced_notional.raw()
        {
            reasons.push("book_absorbs_first_wave".to_owned());
        }
        if context.fragility.low.waves.is_empty()
            && context.fragility.base.waves.is_empty()
            && context.fragility.high.waves.is_empty()
        {
            reasons.push("no_scenario_path".to_owned());
        }
        if !reasons.is_empty() {
            return Ok(SignalEvaluation::NoSignal { reasons });
        }
        let direction = if long > short {
            Direction::Short
        } else {
            Direction::Long
        };
        Ok(SignalEvaluation::Candidate(Box::new(candidate_signal(
            SignalType::LiquidationFragilityAsymmetry,
            snapshot,
            context,
            direction,
        )?)))
    }
}
