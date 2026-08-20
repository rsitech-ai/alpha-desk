#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ResearchError {
    #[error("experiment manifest is incomplete and remains exploratory")]
    IncompleteManifest { field: &'static str },
    #[error("registered experiments are immutable")]
    ImmutableExperiment,
    #[error("input contains information known after the evaluation cutoff")]
    FutureData { field: &'static str },
    #[error("walk-forward validation is not implemented")]
    WalkForwardNotImplemented,
    #[error("locked holdout evaluation is not implemented")]
    HoldoutNotImplemented,
    #[error("shadow-live capture is not implemented")]
    ShadowLiveNotImplemented,
    #[error("holdout partition leaked into {field}")]
    HoldoutLeakage { field: &'static str },
    #[error("validation split is invalid at {field}")]
    SplitInvalid { field: &'static str },
    #[error("shadow-live capture refused leaked or unordered evidence at {field}")]
    ShadowLeakage { field: &'static str },
    #[error("research CLI usage is invalid")]
    Usage,
    #[error("synthetic fixture is invalid")]
    InvalidFixture,
    #[error("execution simulation failed: {0}")]
    Simulation(String),
    #[error("model runtime failed: {0}")]
    Model(String),
    #[error("research cannot sign or place live orders")]
    TradingSignerForbidden,
    #[error("research cannot load a live corpus")]
    LiveCorpusForbidden,
    #[error("research cannot load a locked holdout corpus")]
    LockedCorpusForbidden,
    #[error("research cannot claim replica command usage")]
    ReplicaCmdsUsedForbidden,
    #[error("observation is missing {field}")]
    MissingObservation { field: &'static str },
    #[error("training sample is insufficient to fit {field}")]
    InsufficientTrain { field: &'static str },
    #[error("estimator variance is unmodeled at {field}")]
    UnmodeledVariance { field: &'static str },
    #[error("estimator class is not an approved research adapter")]
    UnsupportedEstimator,
    #[error("research metric calculation failed at {field}")]
    Metric { field: &'static str },
    #[error("statistical significance is not claimed")]
    SignificanceNotClaimed,
    #[error("variant ledger records are immutable")]
    ImmutableVariant,
}

impl ResearchError {
    #[must_use]
    pub const fn reason_code(&self) -> &'static str {
        match self {
            Self::IncompleteManifest { .. } => "hl_research.incomplete_manifest",
            Self::ImmutableExperiment => "hl_research.immutable_experiment",
            Self::FutureData { .. } => "hl_research.future_data",
            Self::WalkForwardNotImplemented => "hl_research.walk_forward_not_implemented",
            Self::HoldoutNotImplemented => "hl_research.holdout_not_implemented",
            Self::ShadowLiveNotImplemented => "hl_research.shadow_live_not_implemented",
            Self::HoldoutLeakage { .. } => "hl_research.holdout_leakage",
            Self::SplitInvalid { .. } => "hl_research.split_invalid",
            Self::ShadowLeakage { .. } => "hl_research.shadow_leakage",
            Self::Usage => "hl_research.usage",
            Self::InvalidFixture => "hl_research.invalid_fixture",
            Self::Simulation(_) => "hl_research.simulation",
            Self::Model(_) => "hl_research.model",
            Self::TradingSignerForbidden => "hl_research.trading_signer_forbidden",
            Self::LiveCorpusForbidden => "hl_research.live_corpus",
            Self::LockedCorpusForbidden => "hl_research.locked_corpus",
            Self::ReplicaCmdsUsedForbidden => "hl_research.replica_cmds_used",
            Self::MissingObservation { .. } => "hl_research.missing_observation",
            Self::InsufficientTrain { .. } => "hl_research.insufficient_train",
            Self::UnmodeledVariance { .. } => "hl_research.unmodeled_variance",
            Self::UnsupportedEstimator => "hl_research.unsupported_estimator",
            Self::Metric { .. } => "hl_research.metric",
            Self::SignificanceNotClaimed => "hl_research.significance_not_claimed",
            Self::ImmutableVariant => "hl_research.immutable_variant",
        }
    }
}

impl From<execution_sim::SimError> for ResearchError {
    fn from(error: execution_sim::SimError) -> Self {
        match error {
            execution_sim::SimError::FutureData { field } => Self::FutureData { field },
            other => Self::Simulation(other.reason_code().to_owned()),
        }
    }
}

impl From<model_runtime::ModelError> for ResearchError {
    fn from(error: model_runtime::ModelError) -> Self {
        match error {
            model_runtime::ModelError::HoldoutNotImplemented => Self::HoldoutNotImplemented,
            model_runtime::ModelError::ShadowLiveNotImplemented => Self::ShadowLiveNotImplemented,
            other => Self::Model(other.reason_code().to_owned()),
        }
    }
}
