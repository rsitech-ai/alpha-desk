use serde::Serialize;

use crate::claims::serialize_unclaimed;
use crate::error::ResearchError;
use crate::ledger::VariantLedger;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MultipleTestingReport {
    pub schema_version: &'static str,
    pub attempted_variants: usize,
    pub bonferroni_divisor: usize,
    pub significance: &'static str,
    pub false_discovery: &'static str,
    pub deflated_performance: &'static str,
    pub withheld_reason: &'static str,
    #[serde(serialize_with = "serialize_unclaimed")]
    pub alpha_quality_claimed: bool,
    #[serde(serialize_with = "serialize_unclaimed")]
    pub alpha_qualified: bool,
    #[serde(serialize_with = "serialize_unclaimed")]
    pub significance_claimed: bool,
}

pub fn diagnose_family(ledger: &VariantLedger) -> MultipleTestingReport {
    let attempted = ledger.len();
    let reason = if attempted == 0 {
        "empty_family"
    } else {
        "no_locked_holdout"
    };
    MultipleTestingReport {
        schema_version: "hl.research.multiple-testing.v1",
        attempted_variants: attempted,
        bonferroni_divisor: attempted.max(1),
        significance: "not_claimed",
        false_discovery: "not_claimed",
        deflated_performance: "not_claimed",
        withheld_reason: reason,
        alpha_quality_claimed: false,
        alpha_qualified: false,
        significance_claimed: false,
    }
}

pub fn claim_discovery(_ledger: &VariantLedger) -> Result<(), ResearchError> {
    Err(ResearchError::SignificanceNotClaimed)
}
