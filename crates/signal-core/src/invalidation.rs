use domain_types::{ProbabilityPpm, ProtocolTime};
use feature_core::{FeatureKey, FeatureValue, HealthState};
use serde::{Deserialize, Serialize};

use crate::SignalError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum InvalidationRule {
    OriginatorExposureClosed {
        fraction: ProbabilityPpm,
    },
    FlowBelow {
        feature: FeatureKey,
        threshold: FeatureValue,
    },
    IndependenceBelow(ProbabilityPpm),
    CostAboveEdge,
    DataHealthNotGreen,
    BookHealthNotGreen,
    TimeExpired {
        at: ProtocolTime,
    },
    CustomApproved {
        rule_id: String,
        version: u32,
    },
}

impl InvalidationRule {
    #[must_use]
    pub const fn as_wire_name(&self) -> &'static str {
        match self {
            Self::OriginatorExposureClosed { .. } => "originator_exposure_closed",
            Self::FlowBelow { .. } => "flow_below",
            Self::IndependenceBelow(_) => "independence_below",
            Self::CostAboveEdge => "cost_above_edge",
            Self::DataHealthNotGreen => "data_health_not_green",
            Self::BookHealthNotGreen => "book_health_not_green",
            Self::TimeExpired { .. } => "time_expired",
            Self::CustomApproved { .. } => "custom_approved",
        }
    }

    pub fn try_custom(rule_id: impl Into<String>, version: u32) -> Result<Self, SignalError> {
        let rule_id = rule_id.into();
        if rule_id.trim().is_empty() || version == 0 {
            return Err(SignalError::ContractViolation("invalid custom rule"));
        }
        Ok(Self::CustomApproved { rule_id, version })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvalidationObservation {
    pub originator_closed_ppm: ProbabilityPpm,
    pub flow_value: Option<FeatureValue>,
    pub independence: ProbabilityPpm,
    pub cost_exceeds_edge: bool,
    pub data_health: HealthState,
    pub book_health: HealthState,
    pub now: ProtocolTime,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvalidationStatus {
    pub rule: InvalidationRule,
    pub triggered: bool,
    pub distance_ppm: ProbabilityPpm,
}

pub fn evaluate_rule(
    rule: &InvalidationRule,
    observation: &InvalidationObservation,
) -> Result<InvalidationStatus, SignalError> {
    let (triggered, distance) = match rule {
        InvalidationRule::OriginatorExposureClosed { fraction } => {
            let triggered = observation.originator_closed_ppm.ppm() >= fraction.ppm();
            let distance = fraction
                .ppm()
                .saturating_sub(observation.originator_closed_ppm.ppm());
            (triggered, distance)
        }
        InvalidationRule::FlowBelow { threshold, .. } => match (&observation.flow_value, threshold)
        {
            (Some(FeatureValue::SignedInteger(value)), FeatureValue::SignedInteger(limit)) => {
                (value < limit, 0)
            }
            (None, _) => (true, 0),
            _ => {
                return Err(SignalError::ContractViolation(
                    "flow comparison requires signed integers",
                ));
            }
        },
        InvalidationRule::IndependenceBelow(threshold) => {
            let triggered = observation.independence.ppm() < threshold.ppm();
            (
                triggered,
                observation
                    .independence
                    .ppm()
                    .saturating_sub(threshold.ppm()),
            )
        }
        InvalidationRule::CostAboveEdge => (observation.cost_exceeds_edge, 0),
        InvalidationRule::DataHealthNotGreen => (observation.data_health != HealthState::Green, 0),
        InvalidationRule::BookHealthNotGreen => (observation.book_health != HealthState::Green, 0),
        InvalidationRule::TimeExpired { at } => (observation.now >= *at, 0),
        InvalidationRule::CustomApproved { .. } => (false, 1_000_000),
    };
    Ok(InvalidationStatus {
        rule: rule.clone(),
        triggered,
        distance_ppm: ProbabilityPpm::from_ppm(distance.min(1_000_000))?,
    })
}

pub fn any_triggered(
    rules: &[InvalidationRule],
    observation: &InvalidationObservation,
) -> Result<bool, SignalError> {
    for rule in rules {
        if evaluate_rule(rule, observation)?.triggered {
            return Ok(true);
        }
    }
    Ok(false)
}
