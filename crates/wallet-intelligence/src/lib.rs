#![forbid(unsafe_code)]

mod cashflows;
mod change_point;
mod copyability;
mod error;
mod hedge;
mod intent;
mod markout;
mod math;
mod performance;
mod skill;
mod style;
mod subject;
mod whale;

pub use cashflows::{CashFlowKind, ExternalCashFlow};
pub use change_point::{BehaviorRegime, BehaviorSample, ChangePointDetector, ChangeReason};
pub use copyability::{
    CapacitySummary, CopyabilityClass, CopyabilityInputs, CopyabilityRequest, CopyabilitySummary,
    MarkoutHorizon, PortfolioContextSummary, WalletIntelligenceVector, estimate_copyability,
};
pub use error::IntelligenceError;
pub use hedge::{HedgeAssessment, HedgeEvidence, assess_hedge};
pub use intent::{IntentClass, IntentFeatures, IntentSnapshot, classify_intent};
pub use markout::{ActionSide, LiquidityRole, MarkoutPoint, MarkoutResult};
pub use performance::{
    ConcentrationBreakdown, ConcentrationInput, DEFAULT_RETURN_SCALE, DEFAULT_USD_SCALE,
    EquityObservation, PerformanceLedger, PerformanceSnapshot, concentration_breakdown,
    long_short_beta, maker_taker_mix, performance_before_after_capital_change,
};
pub use skill::{
    SkillEstimate, SkillObservation, SkillPrior, SkillVector, effective_sample_size_milli,
    estimate_skill,
};
pub use style::{StyleClass, StyleFeatures, StyleSnapshot, classify_style};
pub use subject::{Applicability, ApplicabilitySupport, IntelligenceSubject};
pub use whale::{WhaleComponents, WhaleInputs};
