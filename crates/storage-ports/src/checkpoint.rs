use canonical_ledger::{
    CheckpointArtifact, CheckpointCompatibility, CheckpointError, StateImageLimits,
};
use domain_types::{BlockHeight, CheckpointId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckpointReceipt {
    checkpoint_id: CheckpointId,
    block_height: BlockHeight,
    state_hash: [u8; 32],
    manifest_blake3: [u8; 32],
}

impl CheckpointReceipt {
    #[must_use]
    pub const fn new(
        checkpoint_id: CheckpointId,
        block_height: BlockHeight,
        state_hash: [u8; 32],
        manifest_blake3: [u8; 32],
    ) -> Self {
        Self {
            checkpoint_id,
            block_height,
            state_hash,
            manifest_blake3,
        }
    }

    #[must_use]
    pub const fn checkpoint_id(&self) -> &CheckpointId {
        &self.checkpoint_id
    }

    #[must_use]
    pub const fn block_height(&self) -> BlockHeight {
        self.block_height
    }

    #[must_use]
    pub const fn state_hash(&self) -> [u8; 32] {
        self.state_hash
    }

    #[must_use]
    pub const fn manifest_blake3(&self) -> [u8; 32] {
        self.manifest_blake3
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckpointPublishDisposition {
    Published(CheckpointReceipt),
    Identical(CheckpointReceipt),
}

impl CheckpointPublishDisposition {
    #[must_use]
    pub const fn receipt(&self) -> &CheckpointReceipt {
        match self {
            Self::Published(receipt) | Self::Identical(receipt) => receipt,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CheckpointStoreError {
    #[error("checkpoint store path is unsafe")]
    UnsafePath,
    #[error("checkpoint store object is absent")]
    NotFound,
    #[error("checkpoint store object exceeds its configured bound")]
    TooLarge,
    #[error("checkpoint store object has unsafe permissions or type")]
    UnsafeObject,
    #[error("checkpoint store I/O failed while {0}")]
    Io(&'static str),
    #[error("checkpoint store contains conflicting immutable content")]
    Conflict,
    #[error("checkpoint contract validation failed: {0}")]
    Contract(#[from] CheckpointError),
}

impl CheckpointStoreError {
    #[must_use]
    pub const fn reason_code(&self) -> &'static str {
        match self {
            Self::UnsafePath => "checkpoint_store.unsafe_path",
            Self::NotFound => "checkpoint_store.not_found",
            Self::TooLarge => "checkpoint_store.too_large",
            Self::UnsafeObject => "checkpoint_store.unsafe_object",
            Self::Io(_) => "checkpoint_store.io",
            Self::Conflict => "checkpoint_store.conflict",
            Self::Contract(_) => "checkpoint_store.contract",
        }
    }
}

pub trait StateCheckpointStore {
    fn publish(
        &self,
        artifact: &CheckpointArtifact,
    ) -> Result<CheckpointPublishDisposition, CheckpointStoreError>;

    fn load(
        &self,
        checkpoint_id: &CheckpointId,
        compatibility: &CheckpointCompatibility,
        limits: StateImageLimits,
    ) -> Result<CheckpointArtifact, CheckpointStoreError>;
}
