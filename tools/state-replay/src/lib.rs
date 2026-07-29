#![forbid(unsafe_code)]

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
    CanonicalLedger, CheckpointArtifact, CheckpointCompatibility, LedgerLimits, StateImageLimits,
    WatermarkOnlyReducerV1,
};
use canonical_state_store::LocalCheckpointStore;
use domain_types::{
    Address, BlockHeight, BlockRange, ChainId, KnownTime, MarketId, Price, ProtocolTime, Quantity,
    SourceId, TransactionId,
};
use replay_engine::{
    ReplayCancellation, ReplayLimits, ReplayOutcome, ReplayRequest, SerialReplayEngine,
};
use serde::Serialize;
use storage_ports::{CanonicalArchive, StateCheckpointStore, VerifiedManifest};

const REPORT_SCHEMA: &str = "hyperliquid-alpha-desk/state-replay-e2e-report/v1";
const ARCHIVE_REPORT_SCHEMA: &str = "hyperliquid-alpha-desk/state-replay-archive-e2e-report/v1";
const EVIDENCE_CLASS: &str = "synthetic_fixture";
const ARCHIVE_EVIDENCE_CLASS: &str = "operator_archive";
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
            Self::Json(_) => "state_replay.json",
            Self::Invariant(_) => "state_replay.invariant",
        }
    }
}

pub fn run_fixture_e2e(config: &FixtureRunConfig) -> Result<FixtureEvidence, FixtureRunError> {
    validate_config(config)?;
    let output_root = create_private_output_root(&config.output_root)?;
    let archive_root = output_root.join("archive");
    let checkpoint_root = output_root.join("checkpoints");
    let archive = LocalParquetArchive::open(
        &archive_root,
        ArchiveConfig::deterministic_fixture(
            "state-replay-e2e-v1",
            KnownTime::from_unix_micros(FIXTURE_EPOCH_MICROS)?,
        )?,
    )?;
    let chain = ChainId::new(CHAIN)?;
    let end_height = START_HEIGHT
        .checked_add(config.block_count - 1)
        .ok_or(FixtureRunError::InvalidConfig)?;
    let mut manifests = Vec::with_capacity(
        usize::try_from(config.block_count).map_err(|_| FixtureRunError::InvalidConfig)?,
    );
    for height in START_HEIGHT..=end_height {
        manifests.push(
            archive
                .append_block(&empty_block(height, &chain)?)?
                .manifest_id()
                .clone(),
        );
    }
    let verified = archive.verify_manifest(
        manifests
            .first()
            .ok_or(FixtureRunError::Invariant("fixture manifest set is empty"))?,
    )?;
    let schema_fingerprint = *verified
        .schema_fingerprints()
        .get("canonical_events")
        .ok_or(FixtureRunError::Invariant(
            "canonical schema fingerprint is absent",
        ))?;

    let replay_started = Instant::now();
    let mut expected_state_hash = None;
    let mut expected_receipt_hash = None;
    for _ in 0..config.iterations {
        let mut ledger = empty_ledger(chain.clone())?;
        let request = replay_request(
            &chain,
            START_HEIGHT,
            end_height,
            manifests.clone(),
            ledger.state_hash(),
            schema_fingerprint,
        )?;
        let outcome = SerialReplayEngine::new(&archive, &mut ledger, ReplayLimits::production())
            .run(&request, &NeverCancel)?;
        let ReplayOutcome::Completed(receipt) = outcome else {
            return Err(FixtureRunError::Invariant(
                "uncancelled fixture replay was cancelled",
            ));
        };
        match (expected_state_hash, expected_receipt_hash) {
            (Some(state_hash), Some(receipt_hash)) => {
                if ledger.state_hash() != state_hash || receipt.receipt_hash() != receipt_hash {
                    return Err(FixtureRunError::Invariant(
                        "independent fixture replays diverged",
                    ));
                }
            }
            (None, None) => {
                expected_state_hash = Some(ledger.state_hash());
                expected_receipt_hash = Some(receipt.receipt_hash());
            }
            _ => {
                return Err(FixtureRunError::Invariant(
                    "replay expectation initialization is inconsistent",
                ));
            }
        }
    }
    let replay_elapsed_micros =
        u64::try_from(replay_started.elapsed().as_micros()).unwrap_or(u64::MAX);
    let expected_state_hash = expected_state_hash.ok_or(FixtureRunError::Invariant(
        "no full replay expectation was produced",
    ))?;
    let expected_receipt_hash = expected_receipt_hash.ok_or(FixtureRunError::Invariant(
        "no full replay receipt was produced",
    ))?;

    let checkpoint_count = config.checkpoint_after;
    let checkpoint_end = START_HEIGHT
        .checked_add(checkpoint_count - 1)
        .ok_or(FixtureRunError::InvalidConfig)?;
    let checkpoint_len =
        usize::try_from(checkpoint_count).map_err(|_| FixtureRunError::InvalidConfig)?;
    let mut partial = empty_ledger(chain.clone())?;
    let partial_request = replay_request(
        &chain,
        START_HEIGHT,
        checkpoint_end,
        manifests[..checkpoint_len].to_vec(),
        partial.state_hash(),
        schema_fingerprint,
    )?;
    let ReplayOutcome::Completed(_) =
        SerialReplayEngine::new(&archive, &mut partial, ReplayLimits::production())
            .run(&partial_request, &NeverCancel)?
    else {
        return Err(FixtureRunError::Invariant(
            "uncancelled checkpoint replay was cancelled",
        ));
    };
    let checkpoint_manifest = archive.verify_manifest(&manifests[checkpoint_len - 1])?;
    let artifact = CheckpointArtifact::try_new(
        partial
            .checkpoint()
            .ok_or(FixtureRunError::Invariant("checkpoint watermark is absent"))?,
        partial.state_image().clone(),
        checkpoint_manifest.manifest_id().clone(),
        checkpoint_manifest.manifest_sha256(),
        schema_fingerprint,
    )?;
    let checkpoint_store =
        LocalCheckpointStore::open(&checkpoint_root, StateImageLimits::production())?;
    let published = checkpoint_store.publish(&artifact)?;
    let compatibility = CheckpointCompatibility::try_new(
        chain.clone(),
        artifact.checkpoint().reducer_set_version(),
        artifact.archive_manifest_id().clone(),
        artifact.archive_manifest_sha256(),
        artifact.schema_fingerprint(),
    )?;
    let loaded = checkpoint_store.load(
        published.receipt().checkpoint_id(),
        &compatibility,
        StateImageLimits::production(),
    )?;
    let mut resumed = CanonicalLedger::try_from_state_image(
        loaded.state_image().clone(),
        WatermarkOnlyReducerV1,
        LedgerLimits::production(),
    )?;
    let resume_start = checkpoint_end
        .checked_add(1)
        .ok_or(FixtureRunError::InvalidConfig)?;
    let resume_request = replay_request(
        &chain,
        resume_start,
        end_height,
        manifests[checkpoint_len..].to_vec(),
        resumed.state_hash(),
        schema_fingerprint,
    )?;
    let ReplayOutcome::Completed(resume_receipt) =
        SerialReplayEngine::new(&archive, &mut resumed, ReplayLimits::production())
            .run(&resume_request, &NeverCancel)?
    else {
        return Err(FixtureRunError::Invariant(
            "uncancelled checkpoint resume was cancelled",
        ));
    };
    if resumed.state_hash() != expected_state_hash {
        return Err(FixtureRunError::Invariant(
            "checkpoint resume final state diverged",
        ));
    }

    let poison_height = end_height
        .checked_add(1)
        .ok_or(FixtureRunError::InvalidConfig)?;
    let poison_manifest = archive
        .append_block(&poison_block(poison_height, &chain)?)?
        .manifest_id()
        .clone();
    let poison_before = resumed.state_hash();
    let poison_request = replay_request(
        &chain,
        poison_height,
        poison_height,
        vec![poison_manifest],
        poison_before,
        schema_fingerprint,
    )?;
    let poison_error =
        match SerialReplayEngine::new(&archive, &mut resumed, ReplayLimits::production())
            .run(&poison_request, &NeverCancel)
        {
            Err(error) => error,
            Ok(_) => {
                return Err(FixtureRunError::Invariant(
                    "watermark-only reducer accepted a poison trade",
                ));
            }
        };
    let poison_after = resumed.state_hash();
    if poison_error.reason_code() != "replay.block_quarantined"
        || poison_error.source_reason_code() != Some("ledger.unsupported_event")
        || poison_error.progress().applied_block_count() != 0
        || poison_after != poison_before
    {
        return Err(FixtureRunError::Invariant(
            "poison block did not preserve the pre-block state",
        ));
    }

    let report = FixtureReport {
        schema_version: REPORT_SCHEMA,
        evidence_class: EVIDENCE_CLASS,
        stage_2_qualified: false,
        live_source_qualified: false,
        chain_id: CHAIN,
        start_height: START_HEIGHT,
        end_height,
        block_count: config.block_count,
        checkpoint_after: config.checkpoint_after,
        iterations_completed: config.iterations,
        expected_final_state_hash: hex::encode(expected_state_hash),
        deterministic_replay_receipt_hash: hex::encode(expected_receipt_hash),
        checkpoint_id: artifact.checkpoint_id().as_str(),
        resumed_final_state_hash: hex::encode(resumed.state_hash()),
        resume_receipt_hash: hex::encode(resume_receipt.receipt_hash()),
        replay_elapsed_micros,
        poison: PoisonReport {
            height: poison_height,
            reason_code: poison_error.reason_code(),
            source_reason_code: poison_error
                .source_reason_code()
                .ok_or(FixtureRunError::Invariant("poison source reason is absent"))?,
            applied_block_count: poison_error.progress().applied_block_count(),
            state_hash_before: hex::encode(poison_before),
            state_hash_after: hex::encode(poison_after),
        },
    };
    let report_path = output_root.join(REPORT_FILE);
    publish_report(&report_path, &report)?;
    Ok(FixtureEvidence { report_path })
}

pub fn run_archive_e2e(config: &ArchiveRunConfig) -> Result<ArchiveEvidence, FixtureRunError> {
    let (chain, range, block_count) = validate_archive_run_config(config)?;
    let archive_root = validate_archive_root(&config.archive_root)?;
    let output_candidate = resolve_output_path(&config.output_root)?;
    if output_candidate.starts_with(&archive_root) || archive_root.starts_with(&output_candidate) {
        return Err(FixtureRunError::UnsafeOutput);
    }
    let archive = LocalParquetArchive::open(
        &archive_root,
        ArchiveConfig::production("state-replay-archive-e2e-v1")?
            .with_read_limits(block_count, 2 * 1_024 * 1_024 * 1_024)?,
    )?;
    let verified_manifests = archive.plan_range(&chain, range)?;
    let (checkpoint_index, schema_fingerprint) =
        validate_archive_plan(config, &verified_manifests)?;
    let manifests = verified_manifests
        .iter()
        .map(|manifest| manifest.manifest_id().clone())
        .collect::<Vec<_>>();

    let output_root = create_private_output_root(&config.output_root)?;
    let checkpoint_root = output_root.join("checkpoints");
    let replay_started = Instant::now();
    let mut expected_state_hash = None;
    let mut expected_receipt_hash = None;
    for _ in 0..config.iterations {
        let mut ledger = empty_ledger_at(chain.clone(), BlockHeight::new(config.start_height))?;
        let request = replay_request(
            &chain,
            config.start_height,
            config.end_height,
            manifests.clone(),
            ledger.state_hash(),
            schema_fingerprint,
        )?;
        let ReplayOutcome::Completed(receipt) =
            SerialReplayEngine::new(&archive, &mut ledger, ReplayLimits::production())
                .run(&request, &NeverCancel)?
        else {
            return Err(FixtureRunError::Invariant(
                "uncancelled operator archive replay was cancelled",
            ));
        };
        match (expected_state_hash, expected_receipt_hash) {
            (Some(state_hash), Some(receipt_hash)) => {
                if ledger.state_hash() != state_hash || receipt.receipt_hash() != receipt_hash {
                    return Err(FixtureRunError::Invariant(
                        "independent operator archive replays diverged",
                    ));
                }
            }
            (None, None) => {
                expected_state_hash = Some(ledger.state_hash());
                expected_receipt_hash = Some(receipt.receipt_hash());
            }
            _ => {
                return Err(FixtureRunError::Invariant(
                    "operator replay expectation initialization is inconsistent",
                ));
            }
        }
    }
    let replay_elapsed_micros =
        u64::try_from(replay_started.elapsed().as_micros()).unwrap_or(u64::MAX);
    let expected_state_hash = expected_state_hash.ok_or(FixtureRunError::Invariant(
        "no operator archive replay expectation was produced",
    ))?;
    let expected_receipt_hash = expected_receipt_hash.ok_or(FixtureRunError::Invariant(
        "no operator archive replay receipt was produced",
    ))?;

    let prefix = &verified_manifests[..=checkpoint_index];
    let prefix_ids = prefix
        .iter()
        .map(|manifest| manifest.manifest_id().clone())
        .collect::<Vec<_>>();
    let mut partial = empty_ledger_at(chain.clone(), BlockHeight::new(config.start_height))?;
    let partial_request = replay_request(
        &chain,
        config.start_height,
        config.checkpoint_height,
        prefix_ids,
        partial.state_hash(),
        schema_fingerprint,
    )?;
    let ReplayOutcome::Completed(_) =
        SerialReplayEngine::new(&archive, &mut partial, ReplayLimits::production())
            .run(&partial_request, &NeverCancel)?
    else {
        return Err(FixtureRunError::Invariant(
            "uncancelled operator checkpoint replay was cancelled",
        ));
    };
    let checkpoint_manifest = &verified_manifests[checkpoint_index];
    let artifact = CheckpointArtifact::try_new(
        partial
            .checkpoint()
            .ok_or(FixtureRunError::Invariant("checkpoint watermark is absent"))?,
        partial.state_image().clone(),
        checkpoint_manifest.manifest_id().clone(),
        checkpoint_manifest.manifest_sha256(),
        schema_fingerprint,
    )?;
    let checkpoint_store =
        LocalCheckpointStore::open(&checkpoint_root, StateImageLimits::production())?;
    let published = checkpoint_store.publish(&artifact)?;
    let compatibility = CheckpointCompatibility::try_new(
        chain.clone(),
        artifact.checkpoint().reducer_set_version(),
        artifact.archive_manifest_id().clone(),
        artifact.archive_manifest_sha256(),
        artifact.schema_fingerprint(),
    )?;
    let loaded = checkpoint_store.load(
        published.receipt().checkpoint_id(),
        &compatibility,
        StateImageLimits::production(),
    )?;
    let mut resumed = CanonicalLedger::try_from_state_image(
        loaded.state_image().clone(),
        WatermarkOnlyReducerV1,
        LedgerLimits::production(),
    )?;
    let resume_start = config
        .checkpoint_height
        .checked_add(1)
        .ok_or(FixtureRunError::InvalidConfig)?;
    let suffix_ids = verified_manifests[checkpoint_index + 1..]
        .iter()
        .map(|manifest| manifest.manifest_id().clone())
        .collect::<Vec<_>>();
    let resume_request = replay_request(
        &chain,
        resume_start,
        config.end_height,
        suffix_ids,
        resumed.state_hash(),
        schema_fingerprint,
    )?;
    let ReplayOutcome::Completed(resume_receipt) =
        SerialReplayEngine::new(&archive, &mut resumed, ReplayLimits::production())
            .run(&resume_request, &NeverCancel)?
    else {
        return Err(FixtureRunError::Invariant(
            "uncancelled operator archive resume was cancelled",
        ));
    };
    if resumed.state_hash() != expected_state_hash {
        return Err(FixtureRunError::Invariant(
            "operator checkpoint resume final state diverged",
        ));
    }

    let report = ArchiveReport {
        schema_version: ARCHIVE_REPORT_SCHEMA,
        evidence_class: ARCHIVE_EVIDENCE_CLASS,
        state_semantics: "watermark_only",
        source_qualification: "unassessed",
        stage_2_qualified: false,
        live_source_qualified: false,
        chain_id: chain.as_str(),
        start_height: config.start_height,
        end_height: config.end_height,
        block_count,
        checkpoint_height: config.checkpoint_height,
        iterations_completed: config.iterations,
        expected_final_state_hash: hex::encode(expected_state_hash),
        deterministic_replay_receipt_hash: hex::encode(expected_receipt_hash),
        checkpoint_id: artifact.checkpoint_id().as_str(),
        resumed_final_state_hash: hex::encode(resumed.state_hash()),
        resume_receipt_hash: hex::encode(resume_receipt.receipt_hash()),
        replay_elapsed_micros,
        manifests: verified_manifests
            .iter()
            .map(ManifestReport::from)
            .collect(),
    };
    let report_path = output_root.join(REPORT_FILE);
    publish_report(&report_path, &report)?;
    Ok(ArchiveEvidence { report_path })
}

fn validate_config(config: &FixtureRunConfig) -> Result<(), FixtureRunError> {
    let passes = config
        .iterations
        .checked_add(1)
        .ok_or(FixtureRunError::InvalidConfig)?;
    let total_blocks = config
        .block_count
        .checked_mul(passes)
        .and_then(|count| count.checked_add(1))
        .ok_or(FixtureRunError::InvalidConfig)?;
    if config.block_count < 2
        || config.block_count > MAX_BLOCKS
        || config.checkpoint_after == 0
        || config.checkpoint_after >= config.block_count
        || config.iterations == 0
        || config.iterations > MAX_ITERATIONS
        || total_blocks > MAX_TOTAL_REPLAY_BLOCKS
    {
        return Err(FixtureRunError::InvalidConfig);
    }
    Ok(())
}

fn validate_archive_run_config(
    config: &ArchiveRunConfig,
) -> Result<(ChainId, BlockRange, u64), FixtureRunError> {
    if config.iterations == 0
        || config.iterations > MAX_ITERATIONS
        || config.start_height >= config.end_height
        || config.checkpoint_height < config.start_height
        || config.checkpoint_height >= config.end_height
    {
        return Err(FixtureRunError::InvalidConfig);
    }
    let block_count = config
        .end_height
        .checked_sub(config.start_height)
        .and_then(|span| span.checked_add(1))
        .ok_or(FixtureRunError::InvalidConfig)?;
    let passes = config
        .iterations
        .checked_add(1)
        .ok_or(FixtureRunError::InvalidConfig)?;
    let total_blocks = block_count
        .checked_mul(passes)
        .ok_or(FixtureRunError::InvalidConfig)?;
    if block_count > MAX_BLOCKS || total_blocks > MAX_TOTAL_REPLAY_BLOCKS {
        return Err(FixtureRunError::InvalidConfig);
    }
    let chain = ChainId::new(config.chain_id.clone())?;
    let range = BlockRange::new(
        BlockHeight::new(config.start_height),
        BlockHeight::new(config.end_height),
    )?;
    Ok((chain, range, block_count))
}

fn validate_archive_plan(
    config: &ArchiveRunConfig,
    manifests: &[VerifiedManifest],
) -> Result<(usize, [u8; 32]), FixtureRunError> {
    let first = manifests
        .first()
        .ok_or(FixtureRunError::Invariant("archive plan is empty"))?;
    let last = manifests
        .last()
        .ok_or(FixtureRunError::Invariant("archive plan is empty"))?;
    let schema_fingerprint =
        *first
            .schema_fingerprints()
            .get("canonical_events")
            .ok_or(FixtureRunError::Invariant(
                "canonical schema fingerprint is absent",
            ))?;
    let mut expected_start = config.start_height;
    let mut checkpoint_index = None;
    for (index, manifest) in manifests.iter().enumerate() {
        let range = manifest.block_range();
        if manifest.chain_id().as_str() != config.chain_id
            || range.start_inclusive.get() != expected_start
            || manifest.schema_fingerprints().get("canonical_events") != Some(&schema_fingerprint)
            || range.end_inclusive.get() > config.end_height
        {
            return Err(FixtureRunError::InvalidConfig);
        }
        if range.end_inclusive.get() == config.checkpoint_height {
            checkpoint_index = Some(index);
        }
        expected_start = range
            .end_inclusive
            .get()
            .checked_add(1)
            .ok_or(FixtureRunError::InvalidConfig)?;
    }
    if first.block_range().start_inclusive.get() != config.start_height
        || last.block_range().end_inclusive.get() != config.end_height
    {
        return Err(FixtureRunError::InvalidConfig);
    }
    let checkpoint_index = checkpoint_index.ok_or(FixtureRunError::InvalidConfig)?;
    Ok((checkpoint_index, schema_fingerprint))
}

fn validate_archive_root(path: &Path) -> Result<PathBuf, FixtureRunError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| FixtureRunError::InvalidArchive)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(FixtureRunError::InvalidArchive);
    }
    path.canonicalize()
        .map_err(|_| FixtureRunError::InvalidArchive)
}

fn create_private_output_root(path: &Path) -> Result<PathBuf, FixtureRunError> {
    let output = resolve_output_path(path)?;
    if fs::symlink_metadata(&output).is_ok() {
        return Err(FixtureRunError::OutputExists);
    }
    let canonical_parent = output.parent().ok_or(FixtureRunError::UnsafeOutput)?;
    let mut builder = fs::DirBuilder::new();
    builder.mode(0o700);
    builder
        .create(&output)
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::AlreadyExists => FixtureRunError::OutputExists,
            _ => FixtureRunError::Io("creating evidence root"),
        })?;
    fs::set_permissions(&output, fs::Permissions::from_mode(0o700))
        .map_err(|_| FixtureRunError::Io("setting evidence root permissions"))?;
    OpenOptions::new()
        .read(true)
        .open(canonical_parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| FixtureRunError::Io("syncing evidence root parent"))?;
    Ok(output)
}

fn resolve_output_path(path: &Path) -> Result<PathBuf, FixtureRunError> {
    if path.as_os_str().is_empty()
        || path.as_os_str().len() > MAX_OUTPUT_PATH_BYTES
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(FixtureRunError::UnsafeOutput);
    }
    let name = path
        .file_name()
        .filter(|name| !name.is_empty())
        .ok_or(FixtureRunError::UnsafeOutput)?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let canonical_parent = parent
        .canonicalize()
        .map_err(|_| FixtureRunError::UnsafeOutput)?;
    Ok(canonical_parent.join(name))
}

fn empty_ledger(
    chain: ChainId,
) -> Result<CanonicalLedger<WatermarkOnlyReducerV1>, FixtureRunError> {
    empty_ledger_at(chain, BlockHeight::new(START_HEIGHT))
}

fn empty_ledger_at(
    chain: ChainId,
    first_height: BlockHeight,
) -> Result<CanonicalLedger<WatermarkOnlyReducerV1>, FixtureRunError> {
    Ok(CanonicalLedger::try_new(
        chain,
        first_height,
        WatermarkOnlyReducerV1,
        LedgerLimits::production(),
    )?)
}

fn replay_request(
    chain: &ChainId,
    start: u64,
    end: u64,
    manifests: Vec<domain_types::ManifestId>,
    expected_start_state_hash: [u8; 32],
    schema_fingerprint: [u8; 32],
) -> Result<ReplayRequest, FixtureRunError> {
    ReplayRequest::try_new(
        chain.clone(),
        BlockRange::new(BlockHeight::new(start), BlockHeight::new(end))?,
        manifests,
        expected_start_state_hash,
        "canonical_events",
        schema_fingerprint,
    )
    .map_err(|_| FixtureRunError::InvalidConfig)
}

fn empty_block(height: u64, chain: &ChainId) -> Result<BlockEnvelope, FixtureRunError> {
    Ok(BlockEnvelope::try_new(
        chain.clone(),
        BlockHeight::new(height),
        fixture_time(height)?,
        ConfirmationClass::CommittedPrimary,
        Vec::new(),
        source_hashes(height)?,
    )?)
}

fn poison_block(height: u64, chain: &ChainId) -> Result<BlockEnvelope, FixtureRunError> {
    let time = fixture_time(height)?;
    let payload = EventPayload::TradeMatched(TradeMatched::without_identities(
        Price::parse_at_scale("65000", 6)?,
        Quantity::parse_at_scale("0.01", 8)?,
        1,
    ));
    let payload_hash = *blake3::hash(&payload.encode_to_vec()?).as_bytes();
    let event = CanonicalEventEnvelope::from_input(CanonicalEventInput {
        schema_version: "1.0.0".to_owned(),
        chain_id: chain.clone(),
        block_height: BlockHeight::new(height),
        block_time: time,
        transaction_id: TransactionId::new(format!("state-replay-poison-{height}"))?,
        transaction_index: 0,
        canonical_event_index: 0,
        market_ids: vec![MarketId::new("perp:BTC")?],
        account_ids: vec![
            Address::from_bytes([0x11; 20]),
            Address::from_bytes([0x22; 20]),
        ],
        source_evidence: vec![SourceEvidence::try_new(
            SourceId::new("state-replay-fixture")?,
            "v1",
            height.to_string(),
            payload_hash,
        )?],
        confirmation_class: ConfirmationClass::CommittedPrimary,
        observed_at: KnownTime::from_unix_micros(time.unix_micros())?,
        ingested_at: KnownTime::from_unix_micros(time.unix_micros())?,
        canonicalized_at: KnownTime::from_unix_micros(time.unix_micros())?,
        parser_version: "state-replay-fixture-v1".to_owned(),
        payload,
    })?;
    Ok(BlockEnvelope::try_new(
        chain.clone(),
        BlockHeight::new(height),
        time,
        ConfirmationClass::CommittedPrimary,
        vec![event],
        source_hashes(height)?,
    )?)
}

fn fixture_time(height: u64) -> Result<ProtocolTime, FixtureRunError> {
    let offset = i64::try_from(height).map_err(|_| FixtureRunError::InvalidConfig)?;
    let micros = FIXTURE_EPOCH_MICROS
        .checked_add(offset)
        .ok_or(FixtureRunError::InvalidConfig)?;
    Ok(ProtocolTime::from_unix_micros(micros)?)
}

fn source_hashes(height: u64) -> Result<BTreeMap<SourceId, [u8; 32]>, FixtureRunError> {
    Ok(BTreeMap::from([(
        SourceId::new("state-replay-fixture")?,
        *blake3::hash(&height.to_be_bytes()).as_bytes(),
    )]))
}

fn publish_report(path: &Path, report: &impl Serialize) -> Result<(), FixtureRunError> {
    let mut bytes = serde_json::to_vec_pretty(report)?;
    bytes.push(b'\n');
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::AlreadyExists => FixtureRunError::OutputExists,
            _ => FixtureRunError::Io("creating evidence report"),
        })?;
    file.write_all(&bytes)
        .map_err(|_| FixtureRunError::Io("writing evidence report"))?;
    file.sync_all()
        .map_err(|_| FixtureRunError::Io("syncing evidence report"))?;
    let directory = OpenOptions::new()
        .read(true)
        .open(path.parent().ok_or(FixtureRunError::UnsafeOutput)?)
        .map_err(|_| FixtureRunError::Io("opening evidence root"))?;
    directory
        .sync_all()
        .map_err(|_| FixtureRunError::Io("syncing evidence root"))
}

#[derive(Debug, Serialize)]
struct FixtureReport<'a> {
    schema_version: &'static str,
    evidence_class: &'static str,
    stage_2_qualified: bool,
    live_source_qualified: bool,
    chain_id: &'static str,
    start_height: u64,
    end_height: u64,
    block_count: u64,
    checkpoint_after: u64,
    iterations_completed: u64,
    expected_final_state_hash: String,
    deterministic_replay_receipt_hash: String,
    checkpoint_id: &'a str,
    resumed_final_state_hash: String,
    resume_receipt_hash: String,
    replay_elapsed_micros: u64,
    poison: PoisonReport,
}

#[derive(Debug, Serialize)]
struct ArchiveReport<'a> {
    schema_version: &'static str,
    evidence_class: &'static str,
    state_semantics: &'static str,
    source_qualification: &'static str,
    stage_2_qualified: bool,
    live_source_qualified: bool,
    chain_id: &'a str,
    start_height: u64,
    end_height: u64,
    block_count: u64,
    checkpoint_height: u64,
    iterations_completed: u64,
    expected_final_state_hash: String,
    deterministic_replay_receipt_hash: String,
    checkpoint_id: &'a str,
    resumed_final_state_hash: String,
    resume_receipt_hash: String,
    replay_elapsed_micros: u64,
    manifests: Vec<ManifestReport>,
}

#[derive(Debug, Serialize)]
struct ManifestReport {
    manifest_id: String,
    manifest_sha256: String,
    start_height: u64,
    end_height: u64,
    row_count: u64,
}

impl From<&VerifiedManifest> for ManifestReport {
    fn from(manifest: &VerifiedManifest) -> Self {
        Self {
            manifest_id: manifest.manifest_id().as_str().to_owned(),
            manifest_sha256: hex::encode(manifest.manifest_sha256()),
            start_height: manifest.block_range().start_inclusive.get(),
            end_height: manifest.block_range().end_inclusive.get(),
            row_count: manifest.row_count(),
        }
    }
}

#[derive(Debug, Serialize)]
struct PoisonReport {
    height: u64,
    reason_code: &'static str,
    source_reason_code: &'static str,
    applied_block_count: u64,
    state_hash_before: String,
    state_hash_after: String,
}

#[derive(Debug, Clone, Copy)]
struct NeverCancel;

impl ReplayCancellation for NeverCancel {
    fn is_cancelled(&self) -> bool {
        false
    }
}
