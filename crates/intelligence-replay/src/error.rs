use canonical_events::{BlockError, ContractError, MappingError};
use canonical_ledger::{AccountStateError, CanonicalStateError, LedgerError, MarketStateError};
use domain_types::ValueError;
use entity_graph::GraphError;
use feature_core::FeatureError;
use market_intelligence::MarketError;
use thiserror::Error;
use wallet_intelligence::IntelligenceError;

#[derive(Debug, Error)]
pub enum IntelligenceReplayError {
    #[error("qualification claim {what} is refused on the synthetic replay path")]
    QualificationClaim { what: &'static str },
    #[error("reconstructed intelligence state is missing")]
    MissingState,
    #[error("action-bearing committed block is rejected ({action_bundles} bundles)")]
    ActionBearingRejected { action_bundles: usize },
    #[error(transparent)]
    Feature(FeatureError),
    #[error(transparent)]
    Graph(#[from] GraphError),
    #[error(transparent)]
    Market(#[from] MarketError),
    #[error(transparent)]
    Wallet(#[from] IntelligenceError),
    #[error("canonical ledger failed: {reason_code}")]
    Ledger { reason_code: &'static str },
    #[error("committed mapper failed: {reason_code}")]
    Mapping { reason_code: &'static str },
    #[error("canonical block failed: {reason_code}")]
    Block { reason_code: &'static str },
    #[error("canonical event contract failed")]
    Contract,
    #[error("canonical state reducer configuration is invalid")]
    CanonicalState,
    #[error("reconstructed {what} record is invalid")]
    StateRecord { what: &'static str },
    #[error(transparent)]
    Domain(#[from] ValueError),
}

impl IntelligenceReplayError {
    #[must_use]
    pub const fn reason_code(&self) -> &'static str {
        match self {
            Self::QualificationClaim { .. } => "intelligence_replay.qualification_claim",
            Self::MissingState => "intelligence_replay.missing_state",
            Self::ActionBearingRejected { .. } => "intelligence_replay.action_bearing_rejected",
            Self::Feature(_) => "intelligence_replay.feature",
            Self::Graph(_) => "intelligence_replay.graph",
            Self::Market(_) => "intelligence_replay.market",
            Self::Wallet(_) => "intelligence_replay.wallet",
            Self::Ledger { reason_code } => reason_code,
            Self::Mapping { reason_code } => reason_code,
            Self::Block { reason_code } => reason_code,
            Self::Contract => "intelligence_replay.contract",
            Self::CanonicalState => "intelligence_replay.canonical_state",
            Self::StateRecord { .. } => "intelligence_replay.state_record",
            Self::Domain(_) => "intelligence_replay.domain",
        }
    }
}

impl From<FeatureError> for IntelligenceReplayError {
    fn from(error: FeatureError) -> Self {
        match error {
            FeatureError::MissingState => Self::MissingState,
            other => Self::Feature(other),
        }
    }
}

impl From<LedgerError> for IntelligenceReplayError {
    fn from(error: LedgerError) -> Self {
        Self::Ledger {
            reason_code: error.reason_code(),
        }
    }
}

impl From<MappingError> for IntelligenceReplayError {
    fn from(error: MappingError) -> Self {
        match error {
            MappingError::UnsupportedCommittedActions { action_bundles } => {
                Self::ActionBearingRejected { action_bundles }
            }
            other => Self::Mapping {
                reason_code: other.reason_code(),
            },
        }
    }
}

impl From<BlockError> for IntelligenceReplayError {
    fn from(error: BlockError) -> Self {
        Self::Block {
            reason_code: error.reason_code(),
        }
    }
}

impl From<ContractError> for IntelligenceReplayError {
    fn from(_error: ContractError) -> Self {
        Self::Contract
    }
}

impl From<CanonicalStateError> for IntelligenceReplayError {
    fn from(_error: CanonicalStateError) -> Self {
        Self::CanonicalState
    }
}

impl From<AccountStateError> for IntelligenceReplayError {
    fn from(_error: AccountStateError) -> Self {
        Self::StateRecord { what: "account" }
    }
}

impl From<MarketStateError> for IntelligenceReplayError {
    fn from(_error: MarketStateError) -> Self {
        Self::StateRecord { what: "market" }
    }
}
