use std::{fs, os::unix::fs::PermissionsExt, path::Path, path::PathBuf, time::Instant};

use canonical_archive::{ArchiveConfig, LocalParquetArchive};
use canonical_events::{
    AssetContextUpdated, BlockEnvelope, CanonicalEventEnvelope, CanonicalEventInput,
    ConfirmationClass, DexCreated, EventPayload, FundingRateUpdated, MarginTableChanged,
    MarketCreated, MarketHalted, MarketMetadataChanged, MarketResumed, OpenInterestCapChanged,
    OracleUpdated, OutcomeCreated, OutcomeResolved, SourceEvidence,
};
use canonical_ledger::{
    AssetContextCurrentRecordV1, CanonicalLedger, CanonicalMarketReducerV1, CheckpointArtifact,
    CheckpointCompatibility, DexCurrentRecordV1, LedgerLimits, MarketCurrentRecordV1,
    MarketFactRecordV1, MarketMetadataResolutionV1, MarketMetadataVersionRecordV1, MarketStatusV1,
    OutcomeCurrentRecordV1, StateImageLimits,
};
use canonical_state_store::LocalCheckpointStore;
use domain_types::{
    Address, AssetId, BlockHeight, ChainId, DexId, FundingRate, KnownTime, MarketId, OutcomeId,
    Price, ProtocolTime, Quantity, QuoteAmount, SourceId, TransactionId,
};
use replay_engine::{ReplayLimits, ReplayOutcome, SerialReplayEngine};
use serde::Serialize;
use storage_ports::{CanonicalArchive, StateCheckpointStore};

use super::{
    CHAIN, FIXTURE_EPOCH_MICROS, FixtureRunError, NeverCancel, REPORT_FILE, RejectionReport,
    START_HEIGHT, create_private_output_root, fixture_time, publish_report, rejection_report,
    replay_request, source_hashes, validate_atomic_rejection, validate_replay_counts,
};

const MARKET_REPORT_SCHEMA: &str = "hyperliquid-alpha-desk/state-replay-market-e2e-report/v1";
const MARKET_EVIDENCE_CLASS: &str = "synthetic_canonical_market";
const MARKET_ID: &str = "perp:BTC";
const DEX_ID: &str = "hyperliquid";
const BASE_ASSET_ID: &str = "BTC";
const QUOTE_ASSET_ID: &str = "USDC";
const OUTCOME_ID: &str = "BTC-ABOVE-60000";
const CREATION_METADATA_VERSION: &str = "creation@1.0.0";
const HASH_ONLY_METADATA_VERSION: &str = "metadata@1.0.1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarketRunConfig {
    pub output_root: PathBuf,
    pub block_count: u64,
    pub checkpoint_after: u64,
    pub iterations: u64,
}

impl MarketRunConfig {
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
pub struct MarketEvidence {
    pub report_path: PathBuf,
}

pub fn run_market_e2e(config: &MarketRunConfig) -> Result<MarketEvidence, FixtureRunError> {
    validate_replay_counts(
        config.block_count,
        config.checkpoint_after,
        config.iterations,
        4,
    )?;
    let output_root = create_private_output_root(&config.output_root)?;
    let archive = LocalParquetArchive::open(
        output_root.join("archive"),
        ArchiveConfig::deterministic_fixture(
            "state-replay-market-e2e-v1",
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
        let offset = height
            .checked_sub(START_HEIGHT)
            .ok_or(FixtureRunError::InvalidConfig)?;
        manifests.push(
            archive
                .append_block(&market_block(height, offset, &chain, "1.0.0")?)?
                .manifest_id()
                .clone(),
        );
    }
    let verified = archive.verify_manifest(
        manifests
            .first()
            .ok_or(FixtureRunError::Invariant("market manifest set is empty"))?,
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
        let mut ledger = empty_market_ledger(chain.clone())?;
        let request = replay_request(
            &chain,
            START_HEIGHT,
            end_height,
            manifests.clone(),
            ledger.state_hash(),
            schema_fingerprint,
        )?;
        let ReplayOutcome::Completed(receipt) =
            SerialReplayEngine::new(&archive, &mut ledger, ReplayLimits::production())
                .run(&request, &NeverCancel)?
        else {
            return Err(FixtureRunError::Invariant(
                "uncancelled market replay was cancelled",
            ));
        };
        match (expected_state_hash, expected_receipt_hash) {
            (Some(state_hash), Some(receipt_hash)) => {
                if ledger.state_hash() != state_hash || receipt.receipt_hash() != receipt_hash {
                    return Err(FixtureRunError::Invariant(
                        "independent market replays diverged",
                    ));
                }
            }
            (None, None) => {
                expected_state_hash = Some(ledger.state_hash());
                expected_receipt_hash = Some(receipt.receipt_hash());
            }
            _ => {
                return Err(FixtureRunError::Invariant(
                    "market replay expectation initialization is inconsistent",
                ));
            }
        }
    }
    let replay_elapsed_micros =
        u64::try_from(replay_started.elapsed().as_micros()).unwrap_or(u64::MAX);
    let expected_state_hash = expected_state_hash.ok_or(FixtureRunError::Invariant(
        "no market replay expectation was produced",
    ))?;
    let expected_receipt_hash = expected_receipt_hash.ok_or(FixtureRunError::Invariant(
        "no market replay receipt was produced",
    ))?;

    let checkpoint_end = START_HEIGHT
        .checked_add(config.checkpoint_after - 1)
        .ok_or(FixtureRunError::InvalidConfig)?;
    let checkpoint_len =
        usize::try_from(config.checkpoint_after).map_err(|_| FixtureRunError::InvalidConfig)?;
    let mut partial = empty_market_ledger(chain.clone())?;
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
            "uncancelled market checkpoint replay was cancelled",
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
    let checkpoint_store = LocalCheckpointStore::open(
        output_root.join("checkpoints"),
        StateImageLimits::production(),
    )?;
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
        CanonicalMarketReducerV1,
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
            "uncancelled market checkpoint resume was cancelled",
        ));
    };
    if resumed.state_hash() != expected_state_hash {
        return Err(FixtureRunError::Invariant(
            "market checkpoint resume final state diverged",
        ));
    }
    let state_summary = summarize_market_state(&resumed, config.block_count)?;

    let metadata_height = end_height
        .checked_add(1)
        .ok_or(FixtureRunError::InvalidConfig)?;
    let metadata_manifest = archive
        .append_block(&metadata_change_block(metadata_height, &chain)?)?
        .manifest_id()
        .clone();
    let mut metadata_ledger = CanonicalLedger::try_from_state_image(
        resumed.state_image().clone(),
        CanonicalMarketReducerV1,
        LedgerLimits::production(),
    )?;
    let metadata_request = replay_request(
        &chain,
        metadata_height,
        metadata_height,
        vec![metadata_manifest],
        metadata_ledger.state_hash(),
        schema_fingerprint,
    )?;
    let ReplayOutcome::Completed(_) =
        SerialReplayEngine::new(&archive, &mut metadata_ledger, ReplayLimits::production())
            .run(&metadata_request, &NeverCancel)?
    else {
        return Err(FixtureRunError::Invariant(
            "uncancelled metadata replay was cancelled",
        ));
    };
    let mut metadata_evidence =
        summarize_hash_only_metadata(&metadata_ledger, end_height, metadata_height)?;

    let suppressed_height = metadata_height
        .checked_add(1)
        .ok_or(FixtureRunError::InvalidConfig)?;
    let suppressed = suppressed_value_block(suppressed_height, &chain)?;
    let mut suppressed_direct = CanonicalLedger::try_from_state_image(
        metadata_ledger.state_image().clone(),
        CanonicalMarketReducerV1,
        LedgerLimits::production(),
    )?;
    let suppressed_direct_before = suppressed_direct.state_hash();
    let suppressed_direct_error = match suppressed_direct.apply_block(&suppressed) {
        Err(error) => error,
        Ok(_) => {
            return Err(FixtureRunError::Invariant(
                "unresolved metadata accepted a value update before replay",
            ));
        }
    };
    if suppressed_direct.state_hash() != suppressed_direct_before {
        return Err(FixtureRunError::Invariant(
            "unresolved metadata changed state before archive replay",
        ));
    }
    let suppressed_reason =
        suppressed_direct_error
            .reducer_reason_code()
            .ok_or(FixtureRunError::Invariant(
                "unresolved metadata reducer reason is absent",
            ))?;
    let suppressed_manifest = archive.append_block(&suppressed)?.manifest_id().clone();
    let suppressed_before = metadata_ledger.state_hash();
    let suppressed_request = replay_request(
        &chain,
        suppressed_height,
        suppressed_height,
        vec![suppressed_manifest],
        suppressed_before,
        schema_fingerprint,
    )?;
    let suppressed_error =
        match SerialReplayEngine::new(&archive, &mut metadata_ledger, ReplayLimits::production())
            .run(&suppressed_request, &NeverCancel)
        {
            Err(error) => error,
            Ok(_) => {
                return Err(FixtureRunError::Invariant(
                    "unresolved market metadata accepted a value update",
                ));
            }
        };
    let suppressed_after = metadata_ledger.state_hash();
    validate_atomic_rejection(
        &suppressed_error,
        "ledger.reducer_failed",
        suppressed_before,
        suppressed_after,
    )?;
    if suppressed_reason != "market_state.metadata_unresolved" {
        return Err(FixtureRunError::Invariant(
            "unresolved market metadata returned the wrong rejection",
        ));
    }
    metadata_evidence.suppressed_value_update = Some(rejection_report(
        suppressed_height,
        &suppressed_error,
        Some(suppressed_reason.to_owned()),
        suppressed_before,
        suppressed_after,
    )?);

    let malformed_height = metadata_height;
    let malformed_archive = LocalParquetArchive::open(
        output_root.join("malformed-archive"),
        ArchiveConfig::deterministic_fixture(
            "state-replay-market-malformed-v1",
            KnownTime::from_unix_micros(FIXTURE_EPOCH_MICROS)?,
        )?,
    )?;
    let malformed = malformed_transition_block(malformed_height, &chain)?;
    let mut malformed_direct = CanonicalLedger::try_from_state_image(
        resumed.state_image().clone(),
        CanonicalMarketReducerV1,
        LedgerLimits::production(),
    )?;
    let malformed_direct_before = malformed_direct.state_hash();
    let malformed_direct_error = match malformed_direct.apply_block(&malformed) {
        Err(error) => error,
        Ok(_) => {
            return Err(FixtureRunError::Invariant(
                "market reducer accepted a late invalid transition before replay",
            ));
        }
    };
    if malformed_direct.state_hash() != malformed_direct_before {
        return Err(FixtureRunError::Invariant(
            "late invalid transition changed state before archive replay",
        ));
    }
    let malformed_reason =
        malformed_direct_error
            .reducer_reason_code()
            .ok_or(FixtureRunError::Invariant(
                "late invalid transition reducer reason is absent",
            ))?;
    let malformed_manifest = malformed_archive
        .append_block(&malformed)?
        .manifest_id()
        .clone();
    let mut malformed_ledger = CanonicalLedger::try_from_state_image(
        resumed.state_image().clone(),
        CanonicalMarketReducerV1,
        LedgerLimits::production(),
    )?;
    let malformed_before = malformed_ledger.state_hash();
    let malformed_request = replay_request(
        &chain,
        malformed_height,
        malformed_height,
        vec![malformed_manifest],
        malformed_before,
        schema_fingerprint,
    )?;
    let malformed_error = match SerialReplayEngine::new(
        &malformed_archive,
        &mut malformed_ledger,
        ReplayLimits::production(),
    )
    .run(&malformed_request, &NeverCancel)
    {
        Err(error) => error,
        Ok(_) => {
            return Err(FixtureRunError::Invariant(
                "market reducer accepted a late invalid transition",
            ));
        }
    };
    let malformed_after = malformed_ledger.state_hash();
    validate_atomic_rejection(
        &malformed_error,
        "ledger.reducer_failed",
        malformed_before,
        malformed_after,
    )?;
    if malformed_reason != "market_state.invalid_status_transition" {
        return Err(FixtureRunError::Invariant(
            "late market transition returned the wrong rejection",
        ));
    }

    let unsupported_archive = LocalParquetArchive::open(
        output_root.join("unsupported-archive"),
        ArchiveConfig::deterministic_fixture(
            "state-replay-market-unsupported-v1",
            KnownTime::from_unix_micros(FIXTURE_EPOCH_MICROS)?,
        )?,
    )?;
    let unsupported_manifest = unsupported_archive
        .append_block(&valuation_block(malformed_height, 2, &chain, "1.1.0")?)?
        .manifest_id()
        .clone();
    let mut unsupported_ledger = CanonicalLedger::try_from_state_image(
        resumed.state_image().clone(),
        CanonicalMarketReducerV1,
        LedgerLimits::production(),
    )?;
    let unsupported_before = unsupported_ledger.state_hash();
    let unsupported_request = replay_request(
        &chain,
        malformed_height,
        malformed_height,
        vec![unsupported_manifest],
        unsupported_before,
        schema_fingerprint,
    )?;
    let unsupported_error = match SerialReplayEngine::new(
        &unsupported_archive,
        &mut unsupported_ledger,
        ReplayLimits::production(),
    )
    .run(&unsupported_request, &NeverCancel)
    {
        Err(error) => error,
        Ok(_) => {
            return Err(FixtureRunError::Invariant(
                "market reducer accepted an unsupported schema",
            ));
        }
    };
    let unsupported_after = unsupported_ledger.state_hash();
    validate_atomic_rejection(
        &unsupported_error,
        "ledger.unsupported_event",
        unsupported_before,
        unsupported_after,
    )?;

    let report = MarketReport {
        schema_version: MARKET_REPORT_SCHEMA,
        evidence_class: MARKET_EVIDENCE_CLASS,
        state_semantics: "exact_market_registry",
        source_qualification: "synthetic_unassessed",
        reducer_set_version: CanonicalMarketReducerV1::VERSION,
        synthetic_market_contract_proven: true,
        stage_1_qualified: false,
        stage_2_qualified: false,
        live_source_qualified: false,
        deployed_source_qualified: false,
        authoritative_metadata_qualified: false,
        external_oracle_reconciliation_qualified: false,
        account_state_qualified: false,
        position_state_qualified: false,
        margin_state_qualified: false,
        book_state_qualified: false,
        signal_state_qualified: false,
        execution_qualified: false,
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
        market_fact_count: state_summary.market_fact_count,
        dex_current_count: state_summary.dex_current_count,
        asset_context_current_count: state_summary.asset_context_current_count,
        market_current_count: state_summary.market_current_count,
        market_metadata_version_count: state_summary.market_metadata_version_count,
        outcome_current_count: state_summary.outcome_current_count,
        active_market_count: state_summary.active_market_count,
        halted_market_count: state_summary.halted_market_count,
        exact_metadata_count: state_summary.exact_metadata_count,
        unresolved_metadata_count: state_summary.unresolved_metadata_count,
        resolved_outcome_count: state_summary.resolved_outcome_count,
        unresolved_outcome_count: state_summary.unresolved_outcome_count,
        sample_market: state_summary.sample_market,
        hash_only_metadata: metadata_evidence,
        malformed_transition: rejection_report(
            malformed_height,
            &malformed_error,
            Some(malformed_reason.to_owned()),
            malformed_before,
            malformed_after,
        )?,
        unsupported_schema: rejection_report(
            malformed_height,
            &unsupported_error,
            None,
            unsupported_before,
            unsupported_after,
        )?,
    };
    harden_private_tree(&output_root)?;
    let report_path = output_root.join(REPORT_FILE);
    publish_report(&report_path, &report)?;
    Ok(MarketEvidence { report_path })
}

fn empty_market_ledger(
    chain: ChainId,
) -> Result<CanonicalLedger<CanonicalMarketReducerV1>, FixtureRunError> {
    Ok(CanonicalLedger::try_new(
        chain,
        BlockHeight::new(START_HEIGHT),
        CanonicalMarketReducerV1,
        LedgerLimits::production(),
    )?)
}

fn market_block(
    height: u64,
    offset: u64,
    chain: &ChainId,
    schema_version: &str,
) -> Result<BlockEnvelope, FixtureRunError> {
    if offset == 0 {
        initial_market_block(height, chain, schema_version)
    } else {
        valuation_block(height, offset, chain, schema_version)
    }
}

fn initial_market_block(
    height: u64,
    chain: &ChainId,
    schema_version: &str,
) -> Result<BlockEnvelope, FixtureRunError> {
    let market = market_id()?;
    let outcome = outcome_id()?;
    let time = fixture_time(height)?;
    market_events_block(
        height,
        chain,
        schema_version,
        vec![
            EventPayload::DexCreated(DexCreated {
                dex_id: DexId::new(DEX_ID)?,
                name: "Hyperliquid synthetic fixture".to_owned(),
                operator_account_id: operator_account(),
            }),
            EventPayload::AssetContextUpdated(AssetContextUpdated {
                asset_id: AssetId::new(BASE_ASSET_ID)?,
                context_version: "asset-context@1.0.0".to_owned(),
                context_hash: fixture_hash("BTC-context"),
            }),
            EventPayload::AssetContextUpdated(AssetContextUpdated {
                asset_id: AssetId::new(QUOTE_ASSET_ID)?,
                context_version: "asset-context@1.0.0".to_owned(),
                context_hash: fixture_hash("USDC-context"),
            }),
            EventPayload::MarketCreated(MarketCreated {
                market_id: market.clone(),
                dex_id: DexId::new(DEX_ID)?,
                base_asset_id: AssetId::new(BASE_ASSET_ID)?,
                quote_asset_id: AssetId::new(QUOTE_ASSET_ID)?,
                tick_size: Price::parse_at_scale("0.1", 6)?,
                lot_size: Quantity::parse_at_scale("0.00001", 8)?,
            }),
            EventPayload::OutcomeCreated(OutcomeCreated {
                market_id: market.clone(),
                outcome_id: outcome,
                description: "Synthetic BTC closes above 60000".to_owned(),
            }),
            EventPayload::OracleUpdated(OracleUpdated {
                market_id: market.clone(),
                oracle_price: Price::from_raw(65_000_000_000, 6)?,
                source: "synthetic-oracle".to_owned(),
                effective_at: time,
            }),
            EventPayload::FundingRateUpdated(FundingRateUpdated {
                market_id: market.clone(),
                funding_rate: FundingRate::from_raw(10_000, 8)?,
                effective_at: time,
            }),
            EventPayload::OpenInterestCapChanged(OpenInterestCapChanged {
                market_id: market.clone(),
                previous_cap: QuoteAmount::from_raw(0, 6)?,
                new_cap: QuoteAmount::from_raw(100_000_000, 6)?,
            }),
            EventPayload::MarginTableChanged(MarginTableChanged {
                market_id: market,
                previous_table_hash: "uninitialized".to_owned(),
                new_table_hash: "margin-0".to_owned(),
            }),
        ],
    )
}

fn valuation_block(
    height: u64,
    offset: u64,
    chain: &ChainId,
    schema_version: &str,
) -> Result<BlockEnvelope, FixtureRunError> {
    let market = market_id()?;
    let time = fixture_time(height)?;
    let offset_i128 = i128::from(offset);
    let oracle_whole = 65_000_i128
        .checked_add(offset_i128)
        .ok_or(FixtureRunError::InvalidConfig)?;
    let cap_whole = 100_i128
        .checked_add(
            offset_i128
                .checked_mul(10)
                .ok_or(FixtureRunError::InvalidConfig)?,
        )
        .ok_or(FixtureRunError::InvalidConfig)?;
    let previous_cap_whole = cap_whole
        .checked_sub(10)
        .ok_or(FixtureRunError::InvalidConfig)?;
    let funding_raw = 10_000_i128
        .checked_add(
            offset_i128
                .checked_mul(100)
                .ok_or(FixtureRunError::InvalidConfig)?,
        )
        .ok_or(FixtureRunError::InvalidConfig)?;
    let previous_offset = offset
        .checked_sub(1)
        .ok_or(FixtureRunError::InvalidConfig)?;
    let mut payloads = vec![
        EventPayload::OracleUpdated(OracleUpdated {
            market_id: market.clone(),
            oracle_price: Price::from_raw(
                oracle_whole
                    .checked_mul(1_000_000)
                    .ok_or(FixtureRunError::InvalidConfig)?,
                6,
            )?,
            source: "synthetic-oracle".to_owned(),
            effective_at: time,
        }),
        EventPayload::FundingRateUpdated(FundingRateUpdated {
            market_id: market.clone(),
            funding_rate: FundingRate::from_raw(funding_raw, 8)?,
            effective_at: time,
        }),
        EventPayload::OpenInterestCapChanged(OpenInterestCapChanged {
            market_id: market.clone(),
            previous_cap: QuoteAmount::from_raw(
                previous_cap_whole
                    .checked_mul(1_000_000)
                    .ok_or(FixtureRunError::InvalidConfig)?,
                6,
            )?,
            new_cap: QuoteAmount::from_raw(
                cap_whole
                    .checked_mul(1_000_000)
                    .ok_or(FixtureRunError::InvalidConfig)?,
                6,
            )?,
        }),
        EventPayload::MarginTableChanged(MarginTableChanged {
            market_id: market.clone(),
            previous_table_hash: format!("margin-{previous_offset}"),
            new_table_hash: format!("margin-{offset}"),
        }),
        EventPayload::MarketHalted(MarketHalted {
            market_id: market.clone(),
            reason: format!("synthetic halt {offset}"),
        }),
        EventPayload::MarketResumed(MarketResumed {
            market_id: market.clone(),
            reason: format!("synthetic resume {offset}"),
        }),
    ];
    if offset == 1 {
        payloads.push(EventPayload::OutcomeResolved(OutcomeResolved {
            market_id: market,
            outcome_id: outcome_id()?,
            settlement_value: Price::parse_at_scale("1", 6)?,
            resolved_at: time,
        }));
    }
    market_events_block(height, chain, schema_version, payloads)
}

fn metadata_change_block(height: u64, chain: &ChainId) -> Result<BlockEnvelope, FixtureRunError> {
    market_events_block(
        height,
        chain,
        "1.0.0",
        vec![EventPayload::MarketMetadataChanged(MarketMetadataChanged {
            market_id: market_id()?,
            metadata_version: HASH_ONLY_METADATA_VERSION.to_owned(),
            metadata_hash: fixture_hash(HASH_ONLY_METADATA_VERSION),
        })],
    )
}

fn suppressed_value_block(height: u64, chain: &ChainId) -> Result<BlockEnvelope, FixtureRunError> {
    market_events_block(
        height,
        chain,
        "1.0.0",
        vec![EventPayload::OracleUpdated(OracleUpdated {
            market_id: market_id()?,
            oracle_price: Price::parse_at_scale("70000", 6)?,
            source: "synthetic-oracle".to_owned(),
            effective_at: fixture_time(height)?,
        })],
    )
}

fn malformed_transition_block(
    height: u64,
    chain: &ChainId,
) -> Result<BlockEnvelope, FixtureRunError> {
    let market = market_id()?;
    market_events_block(
        height,
        chain,
        "1.0.0",
        vec![
            EventPayload::OracleUpdated(OracleUpdated {
                market_id: market.clone(),
                oracle_price: Price::parse_at_scale("70000", 6)?,
                source: "synthetic-oracle".to_owned(),
                effective_at: fixture_time(height)?,
            }),
            EventPayload::MarketResumed(MarketResumed {
                market_id: market,
                reason: "invalid resume while active".to_owned(),
            }),
        ],
    )
}

fn market_events_block(
    height: u64,
    chain: &ChainId,
    schema_version: &str,
    payloads: Vec<EventPayload>,
) -> Result<BlockEnvelope, FixtureRunError> {
    let time = fixture_time(height)?;
    let mut events = Vec::with_capacity(payloads.len());
    for (index, payload) in payloads.into_iter().enumerate() {
        let event_index = u32::try_from(index).map_err(|_| FixtureRunError::InvalidConfig)?;
        let (market_ids, account_ids) = event_scope(&payload);
        let payload_hash = *blake3::hash(&payload.encode_to_vec()?).as_bytes();
        events.push(CanonicalEventEnvelope::from_input(CanonicalEventInput {
            schema_version: schema_version.to_owned(),
            chain_id: chain.clone(),
            block_height: BlockHeight::new(height),
            block_time: time,
            transaction_id: TransactionId::new(format!("state-replay-market-tx-{height}"))?,
            transaction_index: 0,
            canonical_event_index: event_index,
            market_ids,
            account_ids,
            source_evidence: vec![SourceEvidence::try_new_indexed(
                SourceId::new("state-replay-fixture")?,
                "v1",
                height.to_string(),
                payload_hash,
                event_index,
            )?],
            confirmation_class: ConfirmationClass::CommittedPrimary,
            observed_at: KnownTime::from_unix_micros(time.unix_micros())?,
            ingested_at: KnownTime::from_unix_micros(time.unix_micros())?,
            canonicalized_at: KnownTime::from_unix_micros(time.unix_micros())?,
            parser_version: "state-replay-market-fixture-v1".to_owned(),
            payload,
        })?);
    }
    Ok(BlockEnvelope::try_new(
        chain.clone(),
        BlockHeight::new(height),
        time,
        ConfirmationClass::CommittedPrimary,
        events,
        source_hashes(height)?,
    )?)
}

fn event_scope(payload: &EventPayload) -> (Vec<MarketId>, Vec<Address>) {
    match payload {
        EventPayload::DexCreated(value) => (Vec::new(), vec![value.operator_account_id]),
        EventPayload::AssetContextUpdated(_) => (Vec::new(), Vec::new()),
        EventPayload::MarketCreated(value) => (vec![value.market_id.clone()], Vec::new()),
        EventPayload::MarketMetadataChanged(value) => (vec![value.market_id.clone()], Vec::new()),
        EventPayload::MarketHalted(value) => (vec![value.market_id.clone()], Vec::new()),
        EventPayload::MarketResumed(value) => (vec![value.market_id.clone()], Vec::new()),
        EventPayload::OpenInterestCapChanged(value) => (vec![value.market_id.clone()], Vec::new()),
        EventPayload::MarginTableChanged(value) => (vec![value.market_id.clone()], Vec::new()),
        EventPayload::OracleUpdated(value) => (vec![value.market_id.clone()], Vec::new()),
        EventPayload::FundingRateUpdated(value) => (vec![value.market_id.clone()], Vec::new()),
        EventPayload::OutcomeCreated(value) => (vec![value.market_id.clone()], Vec::new()),
        EventPayload::OutcomeResolved(value) => (vec![value.market_id.clone()], Vec::new()),
        _ => (Vec::new(), Vec::new()),
    }
}

fn market_id() -> Result<MarketId, FixtureRunError> {
    Ok(MarketId::new(MARKET_ID)?)
}

fn outcome_id() -> Result<OutcomeId, FixtureRunError> {
    Ok(OutcomeId::new(OUTCOME_ID)?)
}

const fn operator_account() -> Address {
    Address::from_bytes([0x44; 20])
}

fn fixture_hash(value: &str) -> [u8; 32] {
    *blake3::hash(value.as_bytes()).as_bytes()
}

fn harden_private_tree(path: &Path) -> Result<(), FixtureRunError> {
    let metadata =
        fs::symlink_metadata(path).map_err(|_| FixtureRunError::Io("reading evidence metadata"))?;
    if metadata.file_type().is_symlink() {
        return Err(FixtureRunError::UnsafeOutput);
    }
    if metadata.is_dir() {
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|_| FixtureRunError::Io("setting evidence directory permissions"))?;
        for entry in
            fs::read_dir(path).map_err(|_| FixtureRunError::Io("reading evidence directory"))?
        {
            harden_private_tree(
                &entry
                    .map_err(|_| FixtureRunError::Io("reading evidence directory entry"))?
                    .path(),
            )?;
        }
    } else if metadata.is_file() {
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|_| FixtureRunError::Io("setting evidence file permissions"))?;
    } else {
        return Err(FixtureRunError::UnsafeOutput);
    }
    Ok(())
}

fn summarize_market_state(
    ledger: &CanonicalLedger<CanonicalMarketReducerV1>,
    expected_block_count: u64,
) -> Result<MarketStateSummary, FixtureRunError> {
    let mut market_fact_count = 0_u64;
    let mut dex_current_count = 0_u64;
    let mut asset_context_current_count = 0_u64;
    let mut market_current_count = 0_u64;
    let mut market_metadata_version_count = 0_u64;
    let mut outcome_current_count = 0_u64;
    let mut active_market_count = 0_u64;
    let mut halted_market_count = 0_u64;
    let mut exact_metadata_count = 0_u64;
    let mut unresolved_metadata_count = 0_u64;
    let mut resolved_outcome_count = 0_u64;
    let mut unresolved_outcome_count = 0_u64;

    for (key, bytes) in ledger.state_image().entries() {
        match key.namespace() {
            "market-fact.v1" => {
                MarketFactRecordV1::decode_at(key, bytes)?;
                market_fact_count = checked_count(market_fact_count)?;
            }
            "dex-current.v1" => {
                DexCurrentRecordV1::decode_at(key, bytes)?;
                dex_current_count = checked_count(dex_current_count)?;
            }
            "asset-context-current.v1" => {
                AssetContextCurrentRecordV1::decode_at(key, bytes)?;
                asset_context_current_count = checked_count(asset_context_current_count)?;
            }
            "market-current.v1" => {
                let current = MarketCurrentRecordV1::decode_at(key, bytes)?;
                market_current_count = checked_count(market_current_count)?;
                match current.status() {
                    MarketStatusV1::Active => {
                        active_market_count = checked_count(active_market_count)?;
                    }
                    MarketStatusV1::Halted => {
                        halted_market_count = checked_count(halted_market_count)?;
                    }
                }
                match current.metadata_resolution() {
                    MarketMetadataResolutionV1::Exact => {
                        exact_metadata_count = checked_count(exact_metadata_count)?;
                    }
                    MarketMetadataResolutionV1::Unresolved => {
                        unresolved_metadata_count = checked_count(unresolved_metadata_count)?;
                    }
                }
            }
            "market-metadata-version.v1" => {
                MarketMetadataVersionRecordV1::decode_at(key, bytes)?;
                market_metadata_version_count = checked_count(market_metadata_version_count)?;
            }
            "market-outcome-current.v1" => {
                let outcome = OutcomeCurrentRecordV1::decode_at(key, bytes)?;
                outcome_current_count = checked_count(outcome_current_count)?;
                if outcome.settlement_value().is_some() && outcome.resolved_at().is_some() {
                    resolved_outcome_count = checked_count(resolved_outcome_count)?;
                } else if outcome.settlement_value().is_none() && outcome.resolved_at().is_none() {
                    unresolved_outcome_count = checked_count(unresolved_outcome_count)?;
                } else {
                    return Err(FixtureRunError::Invariant(
                        "market outcome resolution fields are inconsistent",
                    ));
                }
            }
            _ => {
                return Err(FixtureRunError::Invariant(
                    "market state contains an unexpected namespace",
                ));
            }
        }
    }
    let expected_fact_count = expected_block_count
        .checked_mul(6)
        .and_then(|value| value.checked_add(4))
        .ok_or(FixtureRunError::InvalidConfig)?;
    if market_fact_count != expected_fact_count
        || dex_current_count != 1
        || asset_context_current_count != 2
        || market_current_count != 1
        || market_metadata_version_count != 1
        || outcome_current_count != 1
        || active_market_count != 1
        || halted_market_count != 0
        || exact_metadata_count != 1
        || unresolved_metadata_count != 0
        || resolved_outcome_count != 1
        || unresolved_outcome_count != 0
    {
        return Err(FixtureRunError::Invariant(
            "market state record cardinality is inconsistent",
        ));
    }

    let market = market_id()?;
    let market_key = MarketCurrentRecordV1::state_key(&market)?;
    let current = MarketCurrentRecordV1::decode_at(
        &market_key,
        ledger
            .state_image()
            .entries()
            .get(&market_key)
            .ok_or(FixtureRunError::Invariant(
                "final market current sample is absent",
            ))?,
    )?;
    let outcome = outcome_id()?;
    let outcome_key = OutcomeCurrentRecordV1::state_key(&market, &outcome)?;
    let outcome_current = OutcomeCurrentRecordV1::decode_at(
        &outcome_key,
        ledger
            .state_image()
            .entries()
            .get(&outcome_key)
            .ok_or(FixtureRunError::Invariant(
                "final market outcome sample is absent",
            ))?,
    )?;
    let sample_market = MarketSample {
        market_id: current.market_id().as_str().to_owned(),
        dex_id: current.dex_id().as_str().to_owned(),
        base_asset_id: current.base_asset_id().as_str().to_owned(),
        quote_asset_id: current.quote_asset_id().as_str().to_owned(),
        status: status_name(current.status()),
        metadata_resolution: resolution_name(current.metadata_resolution()),
        metadata_version: current.metadata_version().to_owned(),
        metadata_hash: hex::encode(current.metadata_hash()),
        tick_size: current.tick_size().map(|value| value.to_string()),
        lot_size: current.lot_size().map(|value| value.to_string()),
        price_scale: current.price_scale(),
        quantity_scale: current.quantity_scale(),
        open_interest_cap: current.open_interest_cap().map(|value| value.to_string()),
        margin_table_hash: current.margin_table_hash().map(str::to_owned),
        oracle_price: current.oracle_price().map(|value| value.to_string()),
        oracle_source: current.oracle_source().map(str::to_owned),
        oracle_effective_at_micros: current.oracle_effective_at().map(ProtocolTime::unix_micros),
        funding_rate: current.funding_rate().map(|value| value.to_string()),
        funding_effective_at_micros: current
            .funding_effective_at()
            .map(ProtocolTime::unix_micros),
        outcome_id: outcome_current.outcome_id().as_str().to_owned(),
        outcome_description: outcome_current.description().to_owned(),
        outcome_resolution: if outcome_current.resolved_at().is_some() {
            "resolved"
        } else {
            "unresolved"
        },
        settlement_value: outcome_current
            .settlement_value()
            .map(|value| value.to_string()),
        resolved_at_micros: outcome_current.resolved_at().map(ProtocolTime::unix_micros),
    };

    Ok(MarketStateSummary {
        market_fact_count,
        dex_current_count,
        asset_context_current_count,
        market_current_count,
        market_metadata_version_count,
        outcome_current_count,
        active_market_count,
        halted_market_count,
        exact_metadata_count,
        unresolved_metadata_count,
        resolved_outcome_count,
        unresolved_outcome_count,
        sample_market,
    })
}

fn summarize_hash_only_metadata(
    ledger: &CanonicalLedger<CanonicalMarketReducerV1>,
    expected_prior_end: u64,
    expected_next_start: u64,
) -> Result<HashOnlyMetadataEvidence, FixtureRunError> {
    let market = market_id()?;
    let current_key = MarketCurrentRecordV1::state_key(&market)?;
    let current = MarketCurrentRecordV1::decode_at(
        &current_key,
        ledger
            .state_image()
            .entries()
            .get(&current_key)
            .ok_or(FixtureRunError::Invariant(
                "hash-only market current state is absent",
            ))?,
    )?;
    let prior_key = MarketMetadataVersionRecordV1::state_key(&market, CREATION_METADATA_VERSION)?;
    let prior = MarketMetadataVersionRecordV1::decode_at(
        &prior_key,
        ledger
            .state_image()
            .entries()
            .get(&prior_key)
            .ok_or(FixtureRunError::Invariant(
                "prior market metadata version is absent",
            ))?,
    )?;
    let next_key = MarketMetadataVersionRecordV1::state_key(&market, HASH_ONLY_METADATA_VERSION)?;
    let next = MarketMetadataVersionRecordV1::decode_at(
        &next_key,
        ledger
            .state_image()
            .entries()
            .get(&next_key)
            .ok_or(FixtureRunError::Invariant(
                "hash-only market metadata version is absent",
            ))?,
    )?;
    if current.metadata_resolution() != MarketMetadataResolutionV1::Unresolved
        || current.metadata_version() != HASH_ONLY_METADATA_VERSION
        || current.metadata_hash() != fixture_hash(HASH_ONLY_METADATA_VERSION)
        || prior.effective_until_block() != Some(BlockHeight::new(expected_prior_end))
        || next.effective_from_block() != BlockHeight::new(expected_next_start)
        || next.effective_until_block().is_some()
        || prior.resolution() != MarketMetadataResolutionV1::Exact
        || next.resolution() != MarketMetadataResolutionV1::Unresolved
        || current.tick_size().is_some()
        || current.lot_size().is_some()
        || current.price_scale().is_some()
        || current.quantity_scale().is_some()
        || current.open_interest_cap().is_some()
        || current.margin_table_hash().is_some()
        || current.oracle_price().is_some()
        || current.oracle_source().is_some()
        || current.oracle_effective_at().is_some()
        || current.funding_rate().is_some()
        || current.funding_effective_at().is_some()
    {
        return Err(FixtureRunError::Invariant(
            "hash-only metadata did not clear exact applicability",
        ));
    }
    Ok(HashOnlyMetadataEvidence {
        prior_version: prior.metadata_version().to_owned(),
        next_version: next.metadata_version().to_owned(),
        prior_effective_until_block: prior.effective_until_block().map(BlockHeight::get),
        next_effective_from_block: next.effective_from_block().get(),
        next_resolution: resolution_name(next.resolution()),
        metadata_hash: hex::encode(current.metadata_hash()),
        tick_size: current.tick_size().map(|value| value.to_string()),
        lot_size: current.lot_size().map(|value| value.to_string()),
        price_scale: current.price_scale(),
        quantity_scale: current.quantity_scale(),
        open_interest_cap: current.open_interest_cap().map(|value| value.to_string()),
        margin_table_hash: current.margin_table_hash().map(str::to_owned),
        oracle_price: current.oracle_price().map(|value| value.to_string()),
        oracle_source: current.oracle_source().map(str::to_owned),
        oracle_effective_at_micros: current.oracle_effective_at().map(ProtocolTime::unix_micros),
        funding_rate: current.funding_rate().map(|value| value.to_string()),
        funding_effective_at_micros: current
            .funding_effective_at()
            .map(ProtocolTime::unix_micros),
        suppressed_value_update: None,
    })
}

fn checked_count(value: u64) -> Result<u64, FixtureRunError> {
    value
        .checked_add(1)
        .ok_or(FixtureRunError::Invariant("market record count overflow"))
}

const fn status_name(status: MarketStatusV1) -> &'static str {
    match status {
        MarketStatusV1::Active => "active",
        MarketStatusV1::Halted => "halted",
    }
}

const fn resolution_name(resolution: MarketMetadataResolutionV1) -> &'static str {
    match resolution {
        MarketMetadataResolutionV1::Exact => "exact",
        MarketMetadataResolutionV1::Unresolved => "unresolved",
    }
}

#[derive(Debug)]
struct MarketStateSummary {
    market_fact_count: u64,
    dex_current_count: u64,
    asset_context_current_count: u64,
    market_current_count: u64,
    market_metadata_version_count: u64,
    outcome_current_count: u64,
    active_market_count: u64,
    halted_market_count: u64,
    exact_metadata_count: u64,
    unresolved_metadata_count: u64,
    resolved_outcome_count: u64,
    unresolved_outcome_count: u64,
    sample_market: MarketSample,
}

#[derive(Debug, Serialize)]
struct MarketSample {
    market_id: String,
    dex_id: String,
    base_asset_id: String,
    quote_asset_id: String,
    status: &'static str,
    metadata_resolution: &'static str,
    metadata_version: String,
    metadata_hash: String,
    tick_size: Option<String>,
    lot_size: Option<String>,
    price_scale: Option<u32>,
    quantity_scale: Option<u32>,
    open_interest_cap: Option<String>,
    margin_table_hash: Option<String>,
    oracle_price: Option<String>,
    oracle_source: Option<String>,
    oracle_effective_at_micros: Option<i64>,
    funding_rate: Option<String>,
    funding_effective_at_micros: Option<i64>,
    outcome_id: String,
    outcome_description: String,
    outcome_resolution: &'static str,
    settlement_value: Option<String>,
    resolved_at_micros: Option<i64>,
}

#[derive(Debug, Serialize)]
struct HashOnlyMetadataEvidence {
    prior_version: String,
    next_version: String,
    prior_effective_until_block: Option<u64>,
    next_effective_from_block: u64,
    next_resolution: &'static str,
    metadata_hash: String,
    tick_size: Option<String>,
    lot_size: Option<String>,
    price_scale: Option<u32>,
    quantity_scale: Option<u32>,
    open_interest_cap: Option<String>,
    margin_table_hash: Option<String>,
    oracle_price: Option<String>,
    oracle_source: Option<String>,
    oracle_effective_at_micros: Option<i64>,
    funding_rate: Option<String>,
    funding_effective_at_micros: Option<i64>,
    suppressed_value_update: Option<RejectionReport>,
}

#[derive(Debug, Serialize)]
struct MarketReport<'a> {
    schema_version: &'static str,
    evidence_class: &'static str,
    state_semantics: &'static str,
    source_qualification: &'static str,
    reducer_set_version: &'static str,
    synthetic_market_contract_proven: bool,
    stage_1_qualified: bool,
    stage_2_qualified: bool,
    live_source_qualified: bool,
    deployed_source_qualified: bool,
    authoritative_metadata_qualified: bool,
    external_oracle_reconciliation_qualified: bool,
    account_state_qualified: bool,
    position_state_qualified: bool,
    margin_state_qualified: bool,
    book_state_qualified: bool,
    signal_state_qualified: bool,
    execution_qualified: bool,
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
    market_fact_count: u64,
    dex_current_count: u64,
    asset_context_current_count: u64,
    market_current_count: u64,
    market_metadata_version_count: u64,
    outcome_current_count: u64,
    active_market_count: u64,
    halted_market_count: u64,
    exact_metadata_count: u64,
    unresolved_metadata_count: u64,
    resolved_outcome_count: u64,
    unresolved_outcome_count: u64,
    sample_market: MarketSample,
    hash_only_metadata: HashOnlyMetadataEvidence,
    malformed_transition: RejectionReport,
    unsupported_schema: RejectionReport,
}
