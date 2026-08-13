use std::path::Path;

use serde::Serialize;

use crate::baselines::UNMODELED_BASELINES;
use crate::claims::serialize_unclaimed;
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

/// External locked-holdout evidence.
///
/// This type has no public constructor. `open` and `from_bytes` fail closed,
/// including for any path inside this repository, so in-repo fixtures cannot
/// mint a lock or stamp `HOLDOUT_PASSED`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HoldoutLock {
    _private: (),
}

impl HoldoutLock {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, ResearchError> {
        let _in_repo = lock_path_is_in_repo(path.as_ref());
        let _ = path.as_ref().exists();
        Err(ResearchError::HoldoutNotImplemented)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ResearchError> {
        let _ = bytes;
        Err(ResearchError::HoldoutNotImplemented)
    }
}

/// Returns true when `path` resolves inside this workspace or a research fixture
/// tree. Those locations cannot mint a holdout lock.
#[must_use]
pub fn lock_path_is_in_repo(path: &Path) -> bool {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let workspace = workspace.canonicalize().unwrap_or(workspace);
    let mut candidates = vec![path.to_path_buf()];
    if let Ok(canonical) = path.canonicalize() {
        candidates.push(canonical);
    }
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join(path));
    }
    candidates.into_iter().any(|candidate| {
        candidate.starts_with(&workspace)
            || candidate.to_string_lossy().contains("fixtures/research")
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromotionEvidence<'a> {
    pub outcome_count: usize,
    pub holdout_lock: Option<&'a HoldoutLock>,
    pub holdout_outcome_count: usize,
    pub calendar_days: Option<u64>,
    pub bootstrap: &'a BootstrapReport,
    pub calibration: &'a CalibrationReport,
    pub metrics: Option<&'a PerformanceMetrics>,
    pub shadow_live: bool,
    pub episode_shares_ppm: &'a [u32],
}

impl PromotionEvidence<'_> {
    #[must_use]
    pub const fn holdout_locked(&self) -> bool {
        self.holdout_lock.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PromotionReport {
    pub schema_version: &'static str,
    pub decision: &'static str,
    #[serde(serialize_with = "serialize_unclaimed")]
    pub promoted: bool,
    #[serde(serialize_with = "serialize_unclaimed")]
    pub holdout_passed: bool,
    #[serde(serialize_with = "serialize_unclaimed")]
    pub alpha_quality_claimed: bool,
    #[serde(serialize_with = "serialize_unclaimed")]
    pub alpha_qualified: bool,
    #[serde(serialize_with = "serialize_unclaimed")]
    pub significance_claimed: bool,
    #[serde(serialize_with = "serialize_unclaimed")]
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
    gates.push(if evidence.holdout_locked() {
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
        alpha_qualified: false,
        significance_claimed: false,
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
