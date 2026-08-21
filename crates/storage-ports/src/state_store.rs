use canonical_ledger::{StateDelta, StateImage, StateImageLimits};
use domain_types::BlockHeight;

pub const STATE_STORE_SCHEMA: &str = "hyperliquid-alpha-desk/file-atomic-state-store/v1";
pub const STATE_STORE_ENGINE: &str = "file-atomic";
pub const LEGACY_ROCKSDB_STATE_STORE_SCHEMA: &str = "hyperliquid-alpha-desk/rocksdb-state-store/v1";
pub const STATE_STORE_CFS: &[&str] = &[
    "meta",
    "market_state",
    "l2_book",
    "l4_orders",
    "account_state",
    "balances",
    "positions",
    "orders",
    "twap",
    "vaults",
    "staking",
    "borrow_lend",
    "evm_heads",
    "reconciliation",
    "event_seen",
    "checkpoints",
];

pub fn admit_column_family_schema(observed: &[&str]) -> Result<(), StateStoreError> {
    if observed == STATE_STORE_CFS {
        Ok(())
    } else {
        Err(StateStoreError::RebuildRequired)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct AtomicStateCommit<'a> {
    delta: &'a StateDelta,
    state_image: &'a StateImage,
}

impl<'a> AtomicStateCommit<'a> {
    pub fn try_new(
        delta: &'a StateDelta,
        state_image: &'a StateImage,
    ) -> Result<Self, StateStoreError> {
        let checkpoint = delta.checkpoint();
        if state_image.state_hash() != delta.after_state_hash()
            || state_image.chain_id() != checkpoint.chain_id()
            || state_image.block_height() != Some(checkpoint.block_height())
            || state_image.canonical_block_hash() != Some(checkpoint.canonical_block_hash())
            || state_image.reducer_set_version() != checkpoint.reducer_set_version()
        {
            return Err(StateStoreError::InvalidCommit);
        }
        Ok(Self { delta, state_image })
    }

    #[must_use]
    pub const fn delta(&self) -> &StateDelta {
        self.delta
    }

    #[must_use]
    pub const fn state_image(&self) -> &StateImage {
        self.state_image
    }

    #[must_use]
    pub const fn before_state_hash(&self) -> [u8; 32] {
        self.delta.before_state_hash()
    }

    #[must_use]
    pub const fn after_state_hash(&self) -> [u8; 32] {
        self.delta.after_state_hash()
    }

    #[must_use]
    pub const fn block_height(&self) -> BlockHeight {
        self.delta.checkpoint().block_height()
    }

    #[must_use]
    pub const fn canonical_block_hash(&self) -> [u8; 32] {
        self.delta.checkpoint().canonical_block_hash()
    }

    #[must_use]
    pub fn reducer_set_version(&self) -> &str {
        self.delta.checkpoint().reducer_set_version()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateCommitReceipt {
    block_height: BlockHeight,
    canonical_block_hash: [u8; 32],
    state_hash: [u8; 32],
}

impl StateCommitReceipt {
    #[must_use]
    pub const fn new(
        block_height: BlockHeight,
        canonical_block_hash: [u8; 32],
        state_hash: [u8; 32],
    ) -> Self {
        Self {
            block_height,
            canonical_block_hash,
            state_hash,
        }
    }

    #[must_use]
    pub const fn block_height(&self) -> BlockHeight {
        self.block_height
    }

    #[must_use]
    pub const fn canonical_block_hash(&self) -> [u8; 32] {
        self.canonical_block_hash
    }

    #[must_use]
    pub const fn state_hash(&self) -> [u8; 32] {
        self.state_hash
    }

    #[must_use]
    pub fn matches(&self, commit: &AtomicStateCommit<'_>) -> bool {
        self.block_height.get() == commit.block_height().get()
            && self.canonical_block_hash == commit.canonical_block_hash()
            && self.state_hash == commit.after_state_hash()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateCommitDisposition {
    Committed(StateCommitReceipt),
    AlreadyCommitted(StateCommitReceipt),
}

impl StateCommitDisposition {
    #[must_use]
    pub const fn receipt(&self) -> &StateCommitReceipt {
        match self {
            Self::Committed(receipt) | Self::AlreadyCommitted(receipt) => receipt,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum StateStoreError {
    #[error("state-store commit contract is invalid")]
    InvalidCommit,
    #[error("state store is owned by another process")]
    Locked,
    #[error("state store contains corrupt or incomplete durable state")]
    Corrupt,
    #[error("state store conflicts with the requested canonical transition")]
    Conflict,
    #[error("state store exceeds a configured resource bound")]
    ResourceLimit,
    #[error("state store schema requires a rebuild")]
    RebuildRequired,
    #[error("state store I/O failed while {0}")]
    Io(&'static str),
}

impl StateStoreError {
    #[must_use]
    pub const fn reason_code(&self) -> &'static str {
        match self {
            Self::InvalidCommit => "state_store.invalid_commit",
            Self::Locked => "state_store.locked",
            Self::Corrupt => "state_store.corrupt",
            Self::Conflict => "state_store.conflict",
            Self::ResourceLimit => "state_store.resource_limit",
            Self::RebuildRequired => "state_store.rebuild_required",
            Self::Io(_) => "state_store.io",
        }
    }
}

pub trait AtomicStateStore {
    /// Atomically persists every state mutation and its block checkpoint.
    ///
    /// Returning success means a subsequent `load_latest` must observe the
    /// returned checkpoint and complete state image after process restart.
    fn commit(
        &self,
        commit: &AtomicStateCommit<'_>,
    ) -> Result<StateCommitDisposition, StateStoreError>;

    /// Loads the latest complete durable state, or `None` for a new store.
    ///
    /// Implementations must reject partial, corrupt, or oversized state
    /// instead of returning a best-effort image.
    fn load_latest(&self, limits: StateImageLimits) -> Result<Option<StateImage>, StateStoreError>;
}
