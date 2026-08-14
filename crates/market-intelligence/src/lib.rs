#![forbid(unsafe_code)]

mod aggression;
mod cohort;
mod conviction;
mod cross_asset;
mod crowding;
mod entry_map;
mod errors;
mod flow;
mod fragility;
mod hash;
mod math;
mod memory;
mod normalization;
mod pain;
mod ratio;
mod regime;
mod sentiment;

pub use aggression::{
    AggressionLeg, AggressionTotals, aggression_subjects, informed_taker_aggression,
};
pub use cohort::{CohortDefinition, CohortMember, CohortPredicate, select_members};
pub use conviction::{ConvictionComponent, ConvictionSnapshot};
pub use cross_asset::{CrossAssetFeatures, CrossAssetInputs, cross_asset_features};
pub use crowding::{
    CrowdingComponents, CrowdingPosition, crowding_components, crowding_components_from_snapshot,
};
pub use entry_map::{EntryBin, EntryHistogram, break_even_bps, entry_histogram};
pub use errors::MarketError;
pub use flow::{
    RiskFlowKind, SmartFlowAggregate, SmartFlowContribution, WeightedContributionEvidence,
    accumulate_smart_flow,
};
pub use fragility::{
    DEFAULT_SHOCKS_BPS, FragilityResult, FragilityScenario, LiquidationWave, ScenarioPathResult,
    SimulatedAccount, SimulatedBook, SimulatedMarginMode, simulate_fragility,
    simulate_fragility_from_snapshot, simulate_path,
};
pub use math::{COUNT_SCALE, PPM_ONE, RATIO_SCALE, USD_SCALE, robust_z_milli};
pub use memory::{
    AnalogueMatch, AnalogueSet, ExactVectorIndex, MemoryEntry, MemoryQuery, MemorySupport,
    VECTOR_DIMENSION_COUNT, VectorIndex, VectorManifest,
};
pub use normalization::LiquidityNormalizer;
pub use pain::{PainObservation, PainState, PainThresholds, classify_pain, pain_confidence};
pub use ratio::{
    PositionedMember, RatioMeasure, RatioResult, RatioScope, RatioUnit, compute_ratio,
};
pub use regime::{
    MarketRegime, RegimeAssessment, RegimeFeatureVector, RegimeModel, RegimeName, classify_regime,
};
pub use sentiment::{
    BooleanObservationPurpose, DimensionUnit, MarketFeatureSnapshot, MarketSentimentVector,
    ObservationMintKind, ObservationStatus, ObservedBookAndFills, ScoredDimension,
    boolean_presence_from_decimal_depth, decimal_depth_from_boolean_presence, market_feature_key,
    mint_boolean_observation, missing_dimension,
};
