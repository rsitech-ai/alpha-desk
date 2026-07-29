use domain_types::BlockHeight;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplayProgress {
    applied_block_count: u64,
    last_applied_height: Option<BlockHeight>,
    current_state_hash: [u8; 32],
}

impl ReplayProgress {
    pub(crate) const fn new(
        applied_block_count: u64,
        last_applied_height: Option<BlockHeight>,
        current_state_hash: [u8; 32],
    ) -> Self {
        Self {
            applied_block_count,
            last_applied_height,
            current_state_hash,
        }
    }

    #[must_use]
    pub const fn applied_block_count(&self) -> u64 {
        self.applied_block_count
    }

    #[must_use]
    pub const fn last_applied_height(&self) -> Option<BlockHeight> {
        self.last_applied_height
    }

    #[must_use]
    pub const fn current_state_hash(&self) -> [u8; 32] {
        self.current_state_hash
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ReplayRequestError {
    #[error("replay limits must be nonzero")]
    InvalidLimits,
    #[error("replay request is invalid")]
    InvalidRequest,
    #[error("replay request repeats an immutable manifest")]
    DuplicateManifest,
}

impl ReplayRequestError {
    #[must_use]
    pub const fn reason_code(&self) -> &'static str {
        match self {
            Self::InvalidLimits => "replay.invalid_limits",
            Self::InvalidRequest => "replay.invalid_request",
            Self::DuplicateManifest => "replay.duplicate_manifest",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ReplayError {
    #[error("replay request exceeds configured bounds")]
    LimitExceeded { progress: ReplayProgress },
    #[error("replay start state does not match the request")]
    StartStateMismatch { progress: ReplayProgress },
    #[error("replay ledger is positioned at another height")]
    StartHeightMismatch { progress: ReplayProgress },
    #[error("immutable archive manifest plan is invalid")]
    ManifestPlan {
        source_reason_code: &'static str,
        progress: ReplayProgress,
    },
    #[error("immutable archive read failed")]
    Archive {
        source_reason_code: &'static str,
        progress: ReplayProgress,
    },
    #[error("immutable archive content violates the replay plan")]
    ArchiveContent { progress: ReplayProgress },
    #[error("canonical block was quarantined at {height:?}")]
    BlockQuarantined {
        height: BlockHeight,
        source_reason_code: &'static str,
        progress: ReplayProgress,
    },
}

impl ReplayError {
    #[must_use]
    pub const fn reason_code(&self) -> &'static str {
        match self {
            Self::LimitExceeded { .. } => "replay.limit_exceeded",
            Self::StartStateMismatch { .. } => "replay.start_state_mismatch",
            Self::StartHeightMismatch { .. } => "replay.start_height_mismatch",
            Self::ManifestPlan { .. } => "replay.manifest_plan",
            Self::Archive { .. } => "replay.archive",
            Self::ArchiveContent { .. } => "replay.archive_content",
            Self::BlockQuarantined { .. } => "replay.block_quarantined",
        }
    }

    #[must_use]
    pub const fn progress(&self) -> &ReplayProgress {
        match self {
            Self::LimitExceeded { progress }
            | Self::StartStateMismatch { progress }
            | Self::StartHeightMismatch { progress }
            | Self::ManifestPlan { progress, .. }
            | Self::Archive { progress, .. }
            | Self::ArchiveContent { progress }
            | Self::BlockQuarantined { progress, .. } => progress,
        }
    }

    #[must_use]
    pub const fn source_reason_code(&self) -> Option<&'static str> {
        match self {
            Self::ManifestPlan {
                source_reason_code, ..
            }
            | Self::Archive {
                source_reason_code, ..
            }
            | Self::BlockQuarantined {
                source_reason_code, ..
            } => Some(*source_reason_code),
            _ => None,
        }
    }

    #[must_use]
    pub const fn quarantine_height(&self) -> Option<BlockHeight> {
        match self {
            Self::BlockQuarantined { height, .. } => Some(*height),
            _ => None,
        }
    }
}
