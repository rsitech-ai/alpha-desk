#![forbid(unsafe_code)]

mod account;
mod checkpoint;
mod error;
mod ledger;
mod market;
mod order;
mod reducer;
mod state;
mod trade;
mod watermark_only;

pub use account::{
    AccountFactRecordV1, AccountModeCurrentRecordV1, AccountQuantityFlowCurrentRecordV1,
    AccountQuantityFlowScopeV1, AccountQuoteFlowCurrentRecordV1, AccountQuoteFlowScopeV1,
    AccountStateError, AccountVaultRelationCurrentRecordV1, CanonicalAccountReducerV1,
    LeverageCurrentRecordV1, MarginModeCurrentRecordV1, SubaccountMasterCurrentRecordV1,
    VaultPrincipalFlowCurrentRecordV1, VaultShareFlowCurrentRecordV1,
};
pub use checkpoint::{CheckpointArtifact, CheckpointCompatibility};
pub use error::{CheckpointError, LedgerError, ReducerError, StateImageError, StateKeyError};
pub use ledger::{
    ApplyOutcome, CanonicalLedger, LedgerLimits, PrepareOutcome, PreparedBlock, StateCheckpoint,
    StateDelta,
};
pub use market::{
    AssetContextCurrentRecordV1, CanonicalMarketReducerV1, DexCurrentRecordV1,
    MarketCurrentRecordV1, MarketFactRecordV1, MarketMetadataResolutionV1,
    MarketMetadataVersionRecordV1, MarketStateError, MarketStatusV1, OutcomeCurrentRecordV1,
};
pub use order::{
    CanonicalOrderReducerV1, OrderCurrentRecordV1, OrderFactRecordV1, OrderLifecycleV1,
    OrderStateError, OrderTransitionRecordV1, OrderTransitionStatusV1,
};
pub use reducer::{ApplyContext, EventReducer};
pub use state::{
    AppliedMutation, StateImage, StateImageLimits, StateKey, StateMutation, StateView,
};
pub use trade::{
    CanonicalTradeReducerV1, TradeParticipantRecordV1, TradeReconciliationRecordV1,
    TradeStateError, TradeStateRecordV1,
};
pub use watermark_only::WatermarkOnlyReducerV1;

pub const CRATE_BOOTSTRAPPED: bool = true;
