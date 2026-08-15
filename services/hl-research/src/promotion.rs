use serde::Serialize;

use crate::baselines::UNMODELED_BASELINES;
use crate::error::ResearchError;
use crate::metrics::{BootstrapReport, CalibrationReport, PerformanceMetrics};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GateDecision {
    Fail,
    Withheld,
}

impl GateDecision {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Fail => "fail",
            Self::Withheld => "withheld",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GateResult {
    pub name: &'static str,
    pub decision: GateDecision,
    pub reason: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PromotionPolicy {
    pub min_outcomes: usize,
    pub min_holdout_outcomes: usize,
    pub min_calendar_days: u64,
    pub max_episode_share_ppm: u32,
}

impl PromotionPolicy {
    #[must_use]
    pub const fn defaults() -> Self {
        Self {
            min_outcomes: 100,
            min_holdout_outcomes: 30,
            min_calendar_days: 90,
            max_episode_share_ppm: 200_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromotionEvidence<'a> {
    pub outcome_count: usize,
    pub holdout_locked: bool,
    pub holdout_outcome_count: usize,
    pub calendar_days: Option<u64>,
    pub bootstrap: &'a BootstrapReport,
    pub calibration: &'a CalibrationReport,
    pub metrics: Option<&'a PerformanceMetrics>,
    pub shadow_live: bool,
    pub episode_shares_ppm: &'a [u32],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PromotionReport {
    pub schema_version: &'static str,
    pub decision: &'static str,
    pub promoted: bool,
    pub holdout_passed: bool,
    pub alpha_quality_claimed: bool,
    pub stage_pass_claimed: bool,
    pub unmodeled_baselines: &'static [&'static str],
    pub gates: Vec<GateResult>,
}

pub fn evaluate_promotion(evidence: &PromotionEvidence<'_>) -> PromotionReport {
    let policy = PromotionPolicy::defaults();
    let mut gates = Vec::new();
    gates.push(if evidence.outcome_count < policy.min_outcomes {
        gate(
            "independent_outcomes",
            GateDecision::Fail,
            "insufficient_independent_outcomes",
        )
    } else {
        gate(
            "independent_outcomes",
            GateDecision::Withheld,
            "no_locked_holdout",
        )
    });
    gates.push(match evidence.calendar_days {
        None => gate(
            "calendar_coverage",
            GateDecision::Withheld,
            "calendar_unmodeled",
        ),
        Some(days) if days < policy.min_calendar_days => gate(
            "calendar_coverage",
            GateDecision::Fail,
            "insufficient_calendar_coverage",
        ),
        Some(_) => gate(
            "calendar_coverage",
            GateDecision::Withheld,
            "no_locked_holdout",
        ),
    });
    gates.push(gate(
        "net_expectancy_bootstrap",
        GateDecision::Withheld,
        "significance_not_claimed",
    ));
    gates.push(gate(
        "cost_stress",
        GateDecision::Withheld,
        "cost_stress_unmodeled",
    ));
    gates.push(gate(
        "latency_stress",
        GateDecision::Withheld,
        "latency_stress_unmodeled",
    ));
    gates.push(concentration_gate(evidence, &policy));
    gates.push(gate(
        "drawdown",
        GateDecision::Withheld,
        "no_preregistered_holdout_budget",
    ));
    gates.push(if evidence.calibration.serialized_as_probability {
        gate(
            "calibration",
            GateDecision::Fail,
            "probability_display_forbidden",
        )
    } else {
        gate("calibration", GateDecision::Withheld, "uncalibrated")
    });
    gates.push(gate(
        "capacity",
        GateDecision::Withheld,
        "synthetic_linear_only",
    ));
    gates.push(if evidence.shadow_live {
        gate(
            "shadow_live",
            GateDecision::Fail,
            "shadow_live_not_production",
        )
    } else {
        gate(
            "shadow_live",
            GateDecision::Withheld,
            "shadow_live_not_implemented",
        )
    });
    gates.push(gate(
        "reproducibility",
        GateDecision::Withheld,
        "two_builder_reproduction_unmodeled",
    ));
    gates.push(if evidence.holdout_locked {
        gate(
            "locked_holdout",
            GateDecision::Fail,
            "holdout_pass_not_implemented",
        )
    } else {
        gate(
            "locked_holdout",
            GateDecision::Withheld,
            "no_locked_holdout",
        )
    });
    let _ = (
        evidence.bootstrap,
        evidence.holdout_outcome_count,
        evidence.metrics,
    );
    PromotionReport {
        schema_version: "hl.research.promotion.v1",
        decision: "withheld",
        promoted: false,
        holdout_passed: false,
        alpha_quality_claimed: false,
        stage_pass_claimed: false,
        unmodeled_baselines: &UNMODELED_BASELINES,
        gates,
    }
}

pub fn promote(_report: &PromotionReport) -> Result<(), ResearchError> {
    Err(ResearchError::HoldoutNotImplemented)
}

pub fn stamp_holdout_passed(_report: &PromotionReport) -> Result<(), ResearchError> {
    Err(ResearchError::HoldoutNotImplemented)
}

fn concentration_gate(evidence: &PromotionEvidence<'_>, policy: &PromotionPolicy) -> GateResult {
    if evidence.episode_shares_ppm.is_empty() {
        return gate(
            "concentration",
            GateDecision::Withheld,
            "episode_shares_unmodeled",
        );
    }
    if evidence
        .episode_shares_ppm
        .iter()
        .any(|share| *share > policy.max_episode_share_ppm)
    {
        return gate(
            "concentration",
            GateDecision::Fail,
            "episode_share_exceeds_policy",
        );
    }
    gate("concentration", GateDecision::Withheld, "no_locked_holdout")
}

fn gate(name: &'static str, decision: GateDecision, reason: &'static str) -> GateResult {
    GateResult {
        name,
        decision,
        reason,
    }
}
