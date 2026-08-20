use crate::estimator::EstimatorClass;

pub const SYNTHETIC_BASELINES: [EstimatorClass; 4] = [
    EstimatorClass::NoTrade,
    EstimatorClass::Momentum,
    EstimatorClass::MeanReversion,
    EstimatorClass::RawFeature,
];

pub const FOLD_ESTIMATOR_CLASSES: [EstimatorClass; 6] = [
    EstimatorClass::MeanOutcome,
    EstimatorClass::UnivariateLinear,
    EstimatorClass::NoTrade,
    EstimatorClass::Momentum,
    EstimatorClass::MeanReversion,
    EstimatorClass::RawFeature,
];

pub const UNMODELED_BASELINES: [&str; 3] = [
    "raw_whale_size",
    "equal_weight_top_wallet",
    "regime_conditioned_linear",
];
