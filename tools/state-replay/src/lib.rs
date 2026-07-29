#![forbid(unsafe_code)]

mod archive;
mod fixture;
mod order;
mod shared;
mod trade;

use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::Write,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
    path::{Component, Path, PathBuf},
    time::Instant,
};

use canonical_archive::{ArchiveConfig, LocalParquetArchive};
use canonical_events::{
    BlockEnvelope, CanonicalEventEnvelope, CanonicalEventInput, ConfirmationClass, EventPayload,
    SourceEvidence, TradeMatched,
};
use canonical_ledger::{
    CanonicalLedger, CanonicalTradeReducerV1, CheckpointArtifact, CheckpointCompatibility,
    LedgerLimits, StateImageLimits, TradeParticipantRecordV1, TradeReconciliationRecordV1,
    TradeStateRecordV1, WatermarkOnlyReducerV1,
};
use canonical_state_store::LocalCheckpointStore;
use domain_types::{
    Address, BlockHeight, BlockRange, ChainId, KnownTime, MarketId, Price, ProtocolTime, Quantity,
    SourceId, TradeId, TransactionId,
};
use replay_engine::{
    ReplayCancellation, ReplayLimits, ReplayOutcome, ReplayRequest, SerialReplayEngine,
};
use serde::Serialize;
use storage_ports::{CanonicalArchive, StateCheckpointStore, VerifiedManifest};

use shared::*;

pub use archive::run_archive_e2e;
pub use fixture::run_fixture_e2e;
pub use order::{OrderEvidence, OrderRunConfig, run_order_e2e};
pub use trade::run_trade_e2e;

const REPORT_SCHEMA: &str = "hyperliquid-alpha-desk/state-replay-e2e-report/v1";
const ARCHIVE_REPORT_SCHEMA: &str = "hyperliquid-alpha-desk/state-replay-archive-e2e-report/v1";
const TRADE_REPORT_SCHEMA: &str = "hyperliquid-alpha-desk/state-replay-trade-e2e-report/v1";
const EVIDENCE_CLASS: &str = "synthetic_fixture";
const ARCHIVE_EVIDENCE_CLASS: &str = "operator_archive";
const TRADE_EVIDENCE_CLASS: &str = "synthetic_canonical_trade";
const CHAIN: &str = "mainnet";
const START_HEIGHT: u64 = 1_000_000;
const FIXTURE_EPOCH_MICROS: i64 = 1_721_779_200_000_000;
const MAX_BLOCKS: u64 = 100_000;
const MAX_ITERATIONS: u64 = 100_000;
const MAX_TOTAL_REPLAY_BLOCKS: u64 = 100_000_000;
const MAX_OUTPUT_PATH_BYTES: usize = 4_096;
const REPORT_FILE: &str = "report.json";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixtureRunConfig {
    pub output_root: PathBuf,
    pub block_count: u64,
    pub checkpoint_after: u64,
    pub iterations: u64,
}

impl FixtureRunConfig {
    #[must_use]
    pub fn new(
        output_root: impl AsRef<Path>,
        block_count: u64,
        checkpoint_after: u64,
        iterations: u64,
    ) -> Self {
        Self {
            output_root: output_root.as_ref().to_path_buf(),
            block_count,
            checkpoint_after,
            iterations,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TradeRunConfig {
    pub output_root: PathBuf,
    pub block_count: u64,
    pub checkpoint_after: u64,
    pub iterations: u64,
}

impl TradeRunConfig {
    #[must_use]
    pub fn new(
        output_root: impl AsRef<Path>,
        block_count: u64,
        checkpoint_after: u64,
        iterations: u64,
    ) -> Self {
        Self {
            output_root: output_root.as_ref().to_path_buf(),
            block_count,
            checkpoint_after,
            iterations,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveRunConfig {
    pub archive_root: PathBuf,
    pub output_root: PathBuf,
    pub chain_id: String,
    pub start_height: u64,
    pub end_height: u64,
    pub checkpoint_height: u64,
    pub iterations: u64,
}

impl ArchiveRunConfig {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn new(
        archive_root: impl AsRef<Path>,
        output_root: impl AsRef<Path>,
        chain_id: impl Into<String>,
        start_height: u64,
        end_height: u64,
        checkpoint_height: u64,
        iterations: u64,
    ) -> Self {
        Self {
            archive_root: archive_root.as_ref().to_path_buf(),
            output_root: output_root.as_ref().to_path_buf(),
            chain_id: chain_id.into(),
            start_height,
            end_height,
            checkpoint_height,
            iterations,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixtureEvidence {
    pub report_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveEvidence {
    pub report_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TradeEvidence {
    pub report_path: PathBuf,
}

#[derive(Debug, thiserror::Error)]
pub enum FixtureRunError {
    #[error("fixture replay configuration is invalid")]
    InvalidConfig,
    #[error("fixture replay output path is unsafe")]
    UnsafeOutput,
    #[error("fixture replay output already exists")]
    OutputExists,
    #[error("operator archive is absent or unsafe")]
    InvalidArchive,
    #[error("fixture replay I/O failed while {0}")]
    Io(&'static str),
    #[error("fixture replay archive failed")]
    Archive(#[from] storage_ports::ArchiveError),
    #[error("fixture replay ledger failed")]
    Ledger(#[from] canonical_ledger::LedgerError),
    #[error("fixture replay engine failed")]
    Replay(#[from] replay_engine::ReplayError),
    #[error("fixture replay checkpoint contract failed")]
    Checkpoint(#[from] canonical_ledger::CheckpointError),
    #[error("fixture replay checkpoint store failed")]
    CheckpointStore(#[from] storage_ports::CheckpointStoreError),
    #[error("fixture replay canonical block construction failed")]
    Block(#[from] canonical_events::BlockError),
    #[error("fixture replay canonical event construction failed")]
    Contract(#[from] canonical_events::ContractError),
    #[error("fixture replay domain value is invalid")]
    Domain(#[from] domain_types::ValueError),
    #[error("fixture replay trade-state record is invalid")]
    TradeState(#[from] canonical_ledger::TradeStateError),
    #[error("fixture replay order-state record is invalid")]
    OrderState(#[from] canonical_ledger::OrderStateError),
    #[error("fixture replay report serialization failed")]
    Json(#[from] serde_json::Error),
    #[error("fixture replay invariant failed: {0}")]
    Invariant(&'static str),
}

impl FixtureRunError {
    #[must_use]
    pub const fn reason_code(&self) -> &'static str {
        match self {
            Self::InvalidConfig => "state_replay.invalid_config",
            Self::UnsafeOutput => "state_replay.unsafe_output",
            Self::OutputExists => "state_replay.output_exists",
            Self::InvalidArchive => "state_replay.invalid_archive",
            Self::Io(_) => "state_replay.io",
            Self::Archive(_) => "state_replay.archive",
            Self::Ledger(_) => "state_replay.ledger",
            Self::Replay(_) => "state_replay.replay",
            Self::Checkpoint(_) => "state_replay.checkpoint",
            Self::CheckpointStore(_) => "state_replay.checkpoint_store",
            Self::Block(_) => "state_replay.block",
            Self::Contract(_) => "state_replay.contract",
            Self::Domain(_) => "state_replay.domain",
            Self::TradeState(_) => "state_replay.trade_state",
            Self::OrderState(_) => "state_replay.order_state",
            Self::Json(_) => "state_replay.json",
            Self::Invariant(_) => "state_replay.invariant",
        }
    }
}
