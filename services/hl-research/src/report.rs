use serde::Serialize;

use crate::experiment::ExperimentStatus;
use execution_sim::SimulationResult;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResearchReport {
    pub schema_version: &'static str,
    pub mode: &'static str,
    pub experiment_status: ExperimentStatus,
    pub experiment_id: String,
    pub walk_forward: &'static str,
    pub holdout: &'static str,
    pub shadow_live: &'static str,
    pub alpha_quality_claimed: bool,
    pub stage_pass_claimed: bool,
    pub net_pnl: String,
    pub filled_quantity: String,
    pub missed_quantity: String,
    pub entry_fees: String,
    pub exit_fees: String,
    pub funding: String,
    pub slippage: String,
    pub impact: String,
    pub simulation_trace_hash: String,
    pub model_score: Option<String>,
}

impl ResearchReport {
    pub fn from_synthetic(
        status: ExperimentStatus,
        experiment_id: String,
        result: &SimulationResult,
        model_score: Option<String>,
    ) -> Self {
        Self {
            schema_version: "hl.research.report.v1",
            mode: "synthetic",
            experiment_status: status,
            experiment_id,
            walk_forward: "not_evaluated",
            holdout: "not_evaluated",
            shadow_live: "not_evaluated",
            alpha_quality_claimed: false,
            stage_pass_claimed: false,
            net_pnl: result.net_pnl().to_string(),
            filled_quantity: result.filled_quantity().to_string(),
            missed_quantity: result.missed_quantity().to_string(),
            entry_fees: result.entry_fees().to_string(),
            exit_fees: result.exit_fees().to_string(),
            funding: result.funding().to_string(),
            slippage: result.slippage().to_string(),
            impact: result.impact().to_string(),
            simulation_trace_hash: hex::encode(result.trace_hash()),
            model_score,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResearchStatus {
    pub schema_version: &'static str,
    pub service: &'static str,
    pub walk_forward: bool,
    pub holdout: bool,
    pub shadow_live: bool,
    pub synthetic_walk_forward: bool,
    pub holdout_isolation: bool,
    pub shadow_capture: bool,
    pub onnx_production: bool,
    pub trading_signer: bool,
    pub alpha_quality_claimed: bool,
    pub stage_pass_claimed: bool,
}

impl ResearchStatus {
    #[must_use]
    pub const fn current() -> Self {
        Self {
            schema_version: "hl.research.status.v1",
            service: "hl-research",
            walk_forward: false,
            holdout: false,
            shadow_live: false,
            synthetic_walk_forward: true,
            holdout_isolation: true,
            shadow_capture: true,
            onnx_production: false,
            trading_signer: false,
            alpha_quality_claimed: false,
            stage_pass_claimed: false,
        }
    }
}
