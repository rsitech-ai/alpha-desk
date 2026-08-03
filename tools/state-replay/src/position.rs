use std::{collections::BTreeMap, path::PathBuf, str::FromStr, time::Instant};

use canonical_archive::LocalParquetArchive;
use canonical_events::{
    BackstopLiquidation, BlockEnvelope, CanonicalEventEnvelope, CanonicalEventInput,
    ConfirmationClass, EventPayload, FundingPaid, LiquidationFill, LiquidationStarted,
    OrderAccepted, PositionSettled, SourceEvidence, TradeMatched, TradeParticipantRoleV1,
    TradeParticipantV1,
};
use canonical_ledger::{
    BackstopLiquidationFactRecordV1, CanonicalLedger, CanonicalStateReducerV1, CheckpointArtifact,
    CheckpointCompatibility, EpisodeAttributionResolutionV1, EpisodeCloseCauseV1,
    EpisodeCompletenessV1, EpisodeEffectKindV1, EpisodeStatusV1, LedgerLimits,
    LiquidationCurrentRecordV1, LiquidationFillFactRecordV1, LiquidationMarketFlowCurrentRecordV1,
    LiquidationObservedStatusV1, LiquidationSourceValueResolutionV1, LiquidationStartFactRecordV1,
    PositionAnchorTransitionV1, PositionEffectFactRecordV1, PositionEpisodeCurrentRecordV1,
    PositionEpisodeEffectFactRecordV1, PositionEpisodeRecordV1, PositionQuantityCurrentRecordV1,
    PositionSettlementFactRecordV1, PositionUnresolvedCauseFactRecordV1, PositionUnresolvedCauseV1,
    StateImageLimits, StateKey, derive_position_episode_id,
};
use canonical_state_store::LocalCheckpointStore;
use domain_types::{
    Address, BlockHeight, ChainId, EventId, FundingRate, KnownTime, LiquidationId, MarketId,
    OrderId, OrderSide, PositionEpisodeId, PositionQuantity, Price, Quantity, QuoteAmount,
    SourceId, TradeId, TransactionId, UsdAmount,
};
use replay_engine::{ReplayLimits, ReplayOutcome, SerialReplayEngine};
use serde::Serialize;
use storage_ports::{CanonicalArchive, StateCheckpointStore};

use super::{
    CHAIN, FixtureRunError, NeverCancel, REPORT_FILE, RejectionReport, START_HEIGHT, account,
    append_fixture_blocks, canonical_events_schema_fingerprint, create_private_output_root,
    fixture_time, harden_private_tree, open_deterministic_archive, publish_report,
    rejection_report, replay_request, validate_atomic_rejection, validate_replay_counts,
};

const REPORT_SCHEMA: &str = "hyperliquid-alpha-desk/state-replay-position-e2e-report/v1";
const EVIDENCE_CLASS: &str = "synthetic_canonical_position";
const STATE_SEMANTICS: &str = "exact_trade_anchored_quantity_and_analytical_episode_flows";
const SOURCE_QUALIFICATION: &str = "synthetic_unassessed";
const POSITION_ARCHIVE_ID: &str = "state-replay-position-e2e-v1";
const MARKET: &str = "perp:BTC";
const LIQUIDATION: &str = "liq-position-e2e";
const OPEN_TRADE: &str = "trd-position-open";
const REVERSAL_TRADE: &str = "trd-position-reversal";
const RECOVERY_TRADE: &str = "trd-position-recovery";
const BUYER: Address = Address::from_bytes([0x11; 20]);
const SELLER: Address = Address::from_bytes([0x22; 20]);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PositionRunConfig {
    pub output: PathBuf,
    pub blocks: u64,
    pub checkpoint_after: u64,
    pub iterations: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PositionEvidence {
    pub report_path: PathBuf,
}

pub fn run_position_e2e(config: &PositionRunConfig) -> Result<PositionEvidence, FixtureRunError> {
    if config.iterations < 2
        || config
            .checkpoint_after
            .checked_add(8)
            .is_none_or(|minimum| config.blocks < minimum)
    {
        return Err(FixtureRunError::InvalidConfig);
    }
    validate_replay_counts(
        config.blocks,
        config.checkpoint_after,
        config.iterations,
        2,
        3,
    )?;

    let output = create_private_output_root(&config.output)?;
    let chain = ChainId::new(CHAIN)?;
    let scenario = build_scenario(config, &chain, "-2.5")?;
    let archive = open_deterministic_archive(&output.join("archive"), POSITION_ARCHIVE_ID)?;
    let manifests = append_fixture_blocks(&archive, &scenario.blocks)?;
    let schema_fingerprint = canonical_events_schema_fingerprint(&archive, &manifests)?;

    let replay_started = Instant::now();
    let mut expected_state_hash = None;
    let mut expected_state_bytes = None;
    let mut expected_receipt_hash = None;
    let mut expected_ledger = None;
    for _ in 0..config.iterations {
        let (ledger, receipt_hash) = replay_all(
            &archive,
            &chain,
            &manifests,
            schema_fingerprint,
            scenario.end_height,
        )?;
        validate_final_entries(ledger.state_image().entries(), &scenario.expected)?;
        let bytes = ledger.state_image().canonical_bytes();
        match (
            expected_state_hash,
            expected_state_bytes.as_ref(),
            expected_receipt_hash,
        ) {
            (Some(hash), Some(prior_bytes), Some(prior_receipt))
                if hash == ledger.state_hash()
                    && prior_bytes == &bytes
                    && prior_receipt == receipt_hash => {}
            (None, None, None) => {
                expected_state_hash = Some(ledger.state_hash());
                expected_state_bytes = Some(bytes);
                expected_receipt_hash = Some(receipt_hash);
                expected_ledger = Some(ledger);
            }
            _ => {
                return Err(FixtureRunError::Invariant(
                    "independent position replays diverged",
                ));
            }
        }
    }
    let replay_elapsed_micros =
        u64::try_from(replay_started.elapsed().as_micros()).unwrap_or(u64::MAX);
    let expected_state_hash = expected_state_hash.ok_or(FixtureRunError::Invariant(
        "missing final position state hash",
    ))?;
    let expected_state_bytes = expected_state_bytes.ok_or(FixtureRunError::Invariant(
        "missing final position state bytes",
    ))?;
    let expected_receipt_hash = expected_receipt_hash.ok_or(FixtureRunError::Invariant(
        "missing full position replay receipt",
    ))?;
    let expected_ledger = expected_ledger.ok_or(FixtureRunError::Invariant(
        "missing final position replay ledger",
    ))?;

    let checkpoint_len =
        usize::try_from(config.checkpoint_after).map_err(|_| FixtureRunError::InvalidConfig)?;
    let checkpoint_end = scenario.checkpoint_height;
    let mut partial = empty_ledger(chain.clone())?;
    let checkpoint_request = replay_request(
        &chain,
        START_HEIGHT,
        checkpoint_end,
        manifests[..checkpoint_len].to_vec(),
        partial.state_hash(),
        schema_fingerprint,
    )?;
    let ReplayOutcome::Completed(_) =
        SerialReplayEngine::new(&archive, &mut partial, ReplayLimits::production())
            .run(&checkpoint_request, &NeverCancel)?
    else {
        return Err(FixtureRunError::Invariant(
            "position checkpoint replay was cancelled",
        ));
    };
    validate_checkpoint_entries(partial.state_image().entries(), &scenario.expected)?;
    let checkpoint_bytes_before_publish = partial.state_image().canonical_bytes();
    let checkpoint_hash_before_publish = partial.state_hash();

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
    let store =
        LocalCheckpointStore::open(output.join("checkpoints"), StateImageLimits::production())?;
    let published = store.publish(&artifact)?;
    let compatibility = CheckpointCompatibility::try_new(
        chain.clone(),
        artifact.checkpoint().reducer_set_version(),
        artifact.archive_manifest_id().clone(),
        artifact.archive_manifest_sha256(),
        artifact.schema_fingerprint(),
    )?;
    let loaded = store.load(
        published.receipt().checkpoint_id(),
        &compatibility,
        StateImageLimits::production(),
    )?;
    if loaded.state_image().canonical_bytes() != checkpoint_bytes_before_publish
        || loaded.state_image().state_hash() != checkpoint_hash_before_publish
    {
        return Err(FixtureRunError::Invariant(
            "published position checkpoint changed canonical state",
        ));
    }
    validate_checkpoint_entries(loaded.state_image().entries(), &scenario.expected)?;

    let mut resumed = CanonicalLedger::try_from_state_image(
        loaded.state_image().clone(),
        account::composite_reducer()?,
        LedgerLimits::production(),
    )?;
    let mut segmented_resume_receipt_hashes = Vec::new();
    for (offset, manifest) in manifests[checkpoint_len..].iter().enumerate() {
        let height = checkpoint_end
            .checked_add(1)
            .and_then(|start| start.checked_add(u64::try_from(offset).ok()?))
            .ok_or(FixtureRunError::InvalidConfig)?;
        let resume_request = replay_request(
            &chain,
            height,
            height,
            vec![manifest.clone()],
            resumed.state_hash(),
            schema_fingerprint,
        )?;
        let ReplayOutcome::Completed(resume_receipt) =
            SerialReplayEngine::new(&archive, &mut resumed, ReplayLimits::production())
                .run(&resume_request, &NeverCancel)?
        else {
            return Err(FixtureRunError::Invariant(
                "position checkpoint resume was cancelled",
            ));
        };
        segmented_resume_receipt_hashes.push(hex::encode(resume_receipt.receipt_hash()));
        if height == checkpoint_end + 5 || height == checkpoint_end + 6 {
            validate_interrupted_entries(resumed.state_image().entries(), &scenario.expected)?;
        }
    }
    validate_final_entries(resumed.state_image().entries(), &scenario.expected)?;
    if resumed.state_hash() != expected_state_hash
        || resumed.state_image().canonical_bytes() != expected_state_bytes
    {
        return Err(FixtureRunError::Invariant(
            "position checkpoint suffix diverged",
        ));
    }

    validate_semantic_variant(config, &output, &chain)?;

    let rejection_height = scenario.end_height + 1;
    let duplicate_trade_identity = run_rejection(
        &output,
        "duplicate-trade",
        &chain,
        &expected_ledger,
        rejection_height,
        duplicate_trade_block(rejection_height, &chain)?,
        schema_fingerprint,
        "ledger.reducer_failed",
        Some("trade_state.trade_id_collision"),
    )?;
    let start_position_mismatch = run_rejection(
        &output,
        "start-position-mismatch",
        &chain,
        &expected_ledger,
        rejection_height,
        start_mismatch_block(rejection_height, &chain)?,
        schema_fingerprint,
        "ledger.reducer_failed",
        Some("position_state.start_position_mismatch"),
    )?;
    let unsupported_schema = run_rejection(
        &output,
        "unsupported-schema",
        &chain,
        &expected_ledger,
        rejection_height,
        unsupported_schema_block(rejection_height, &chain)?,
        schema_fingerprint,
        "ledger.unsupported_event",
        None,
    )?;

    let report = PositionReport {
        schema_version: REPORT_SCHEMA,
        evidence_class: EVIDENCE_CLASS,
        state_semantics: STATE_SEMANTICS,
        source_qualification: SOURCE_QUALIFICATION,
        reducer_version: CanonicalStateReducerV1::VERSION,
        synthetic_position_contract_proven: true,
        stage_1_qualified: false,
        stage_2_qualified: false,
        deployed_source_qualified: false,
        live_source_qualified: false,
        authoritative_opening_position_qualified: false,
        authoritative_opening_balance_qualified: false,
        venue_position_reconciliation_qualified: false,
        protocol_entry_price_parity_qualified: false,
        source_closed_pnl_completeness_qualified: false,
        execution_fee_attribution_qualified: false,
        twap_position_completeness_qualified: false,
        backstop_cost_basis_qualified: false,
        standard_margin_qualified: false,
        unified_margin_qualified: false,
        portfolio_margin_qualified: false,
        liquidation_price_qualified: false,
        book_state_qualified: false,
        signal_state_qualified: false,
        execution_qualified: false,
        live_product_qualified: false,
        block_count: config.blocks,
        checkpoint_after: config.checkpoint_after,
        iterations_completed: config.iterations,
        expected_final_state_hash: hex::encode(expected_state_hash),
        resumed_final_state_hash: hex::encode(resumed.state_hash()),
        checkpoint_state_hash_before_publish: hex::encode(checkpoint_hash_before_publish),
        checkpoint_state_hash_after_load: hex::encode(loaded.state_image().state_hash()),
        deterministic_full_replay_receipt_hash: hex::encode(expected_receipt_hash),
        segmented_resume_receipt_hashes,
        checkpoint_id: artifact.checkpoint_id().as_str(),
        replay_elapsed_micros,
        namespace_counts: namespace_counts(resumed.state_image().entries()),
        fixture_oracle: fixture_oracle(&scenario.expected)?,
        duplicate_trade_identity,
        start_position_mismatch,
        unsupported_schema,
    };
    let report_path = output.join(REPORT_FILE);
    publish_report(&report_path, &report)?;
    harden_private_tree(&output)?;
    Ok(PositionEvidence { report_path })
}

mod fixture;
use fixture::{FrozenExpectation, build_scenario, trade_event};
fn empty_ledger(
    chain: ChainId,
) -> Result<CanonicalLedger<CanonicalStateReducerV1>, FixtureRunError> {
    Ok(CanonicalLedger::try_new(
        chain,
        BlockHeight::new(START_HEIGHT),
        account::composite_reducer()?,
        LedgerLimits::production(),
    )?)
}

fn replay_all(
    archive: &LocalParquetArchive,
    chain: &ChainId,
    manifests: &[domain_types::ManifestId],
    schema_fingerprint: [u8; 32],
    end_height: u64,
) -> Result<(CanonicalLedger<CanonicalStateReducerV1>, [u8; 32]), FixtureRunError> {
    let mut ledger = empty_ledger(chain.clone())?;
    let request = replay_request(
        chain,
        START_HEIGHT,
        end_height,
        manifests.to_vec(),
        ledger.state_hash(),
        schema_fingerprint,
    )?;
    let ReplayOutcome::Completed(receipt) =
        SerialReplayEngine::new(archive, &mut ledger, ReplayLimits::production())
            .run(&request, &NeverCancel)?
    else {
        return Err(FixtureRunError::Invariant(
            "full position replay was cancelled",
        ));
    };
    Ok((ledger, receipt.receipt_hash()))
}

fn validate_semantic_variant(
    config: &PositionRunConfig,
    output: &std::path::Path,
    chain: &ChainId,
) -> Result<(), FixtureRunError> {
    let scenario = build_scenario(config, chain, "-2.75")?;
    let archive = open_deterministic_archive(
        &output.join("semantic-variant"),
        "state-replay-position-semantic-v1",
    )?;
    let manifests = append_fixture_blocks(&archive, &scenario.blocks)?;
    let fingerprint = canonical_events_schema_fingerprint(&archive, &manifests)?;
    let (ledger, _) = replay_all(
        &archive,
        chain,
        &manifests,
        fingerprint,
        scenario.end_height,
    )?;
    match validate_final_entries(ledger.state_image().entries(), &scenario.expected) {
        Err(FixtureRunError::PositionSemanticMismatch) => Ok(()),
        Err(error) => Err(error),
        Ok(()) => Err(FixtureRunError::Invariant(
            "position semantic variant unexpectedly qualified",
        )),
    }
}

#[allow(clippy::too_many_arguments)]
fn run_rejection(
    output: &std::path::Path,
    name: &str,
    chain: &ChainId,
    source: &CanonicalLedger<CanonicalStateReducerV1>,
    height: u64,
    rejected: BlockEnvelope,
    schema_fingerprint: [u8; 32],
    expected_source_reason: &str,
    expected_reducer_reason: Option<&str>,
) -> Result<RejectionReport, FixtureRunError> {
    let archive = open_deterministic_archive(
        &output.join(format!("rejection-{name}")),
        &format!("state-replay-position-rejection-{name}-v1"),
    )?;
    let manifest = archive.append_block(&rejected)?.manifest_id().clone();
    let mut ledger = CanonicalLedger::try_from_state_image(
        source.state_image().clone(),
        account::composite_reducer()?,
        LedgerLimits::production(),
    )?;
    let before = ledger.state_hash();
    let request = replay_request(
        chain,
        height,
        height,
        vec![manifest],
        before,
        schema_fingerprint,
    )?;
    let error = SerialReplayEngine::new(&archive, &mut ledger, ReplayLimits::production())
        .run(&request, &NeverCancel)
        .expect_err("position rejection block must quarantine");
    let after = ledger.state_hash();
    validate_atomic_rejection(&error, expected_source_reason, before, after)?;
    if error.reducer_reason_code() != expected_reducer_reason {
        return Err(FixtureRunError::Invariant(
            "position rejection reducer precedence diverged",
        ));
    }
    rejection_report(
        height,
        &error,
        error.reducer_reason_code().map(str::to_owned),
        before,
        after,
    )
}

fn duplicate_trade_block(height: u64, chain: &ChainId) -> Result<BlockEnvelope, FixtureRunError> {
    account::block(
        height,
        chain,
        vec![trade_event(
            height,
            0,
            RECOVERY_TRADE,
            "position-duplicate-trade",
            BUYER,
            SELLER,
            "4.25000000",
            "-0.25000000",
            "position-recovery-buyer-order",
            "position-recovery-seller-order",
            "95.000000",
            "0.25000000",
            "1.0.0",
        )?],
    )
}

fn start_mismatch_block(height: u64, chain: &ChainId) -> Result<BlockEnvelope, FixtureRunError> {
    account::block(
        height,
        chain,
        vec![trade_event(
            height,
            0,
            "trd-position-start-mismatch",
            "position-start-mismatch",
            BUYER,
            SELLER,
            "0.00000000",
            "-0.25000000",
            "position-recovery-buyer-order",
            "position-recovery-seller-order",
            "95.000000",
            "0.25000000",
            "1.0.0",
        )?],
    )
}

fn unsupported_schema_block(
    height: u64,
    chain: &ChainId,
) -> Result<BlockEnvelope, FixtureRunError> {
    account::block(
        height,
        chain,
        vec![trade_event(
            height,
            0,
            "trd-position-schema-unsupported",
            "position-schema-unsupported",
            BUYER,
            SELLER,
            "4.25000000",
            "-0.25000000",
            "position-recovery-buyer-order",
            "position-recovery-seller-order",
            "95.000000",
            "0.25000000",
            "1.1.0",
        )?],
    )
}

mod validation;
use validation::{
    fixture_oracle, namespace_counts, validate_checkpoint_entries, validate_final_entries,
    validate_interrupted_entries,
};
#[derive(Debug, Serialize)]
struct PositionReport<'a> {
    schema_version: &'a str,
    evidence_class: &'a str,
    state_semantics: &'a str,
    source_qualification: &'a str,
    reducer_version: &'a str,
    synthetic_position_contract_proven: bool,
    stage_1_qualified: bool,
    stage_2_qualified: bool,
    deployed_source_qualified: bool,
    live_source_qualified: bool,
    authoritative_opening_position_qualified: bool,
    authoritative_opening_balance_qualified: bool,
    venue_position_reconciliation_qualified: bool,
    protocol_entry_price_parity_qualified: bool,
    source_closed_pnl_completeness_qualified: bool,
    execution_fee_attribution_qualified: bool,
    twap_position_completeness_qualified: bool,
    backstop_cost_basis_qualified: bool,
    standard_margin_qualified: bool,
    unified_margin_qualified: bool,
    portfolio_margin_qualified: bool,
    liquidation_price_qualified: bool,
    book_state_qualified: bool,
    signal_state_qualified: bool,
    execution_qualified: bool,
    live_product_qualified: bool,
    block_count: u64,
    checkpoint_after: u64,
    iterations_completed: u64,
    expected_final_state_hash: String,
    resumed_final_state_hash: String,
    checkpoint_state_hash_before_publish: String,
    checkpoint_state_hash_after_load: String,
    deterministic_full_replay_receipt_hash: String,
    segmented_resume_receipt_hashes: Vec<String>,
    checkpoint_id: &'a str,
    replay_elapsed_micros: u64,
    namespace_counts: BTreeMap<String, usize>,
    fixture_oracle: serde_json::Value,
    duplicate_trade_identity: RejectionReport,
    start_position_mismatch: RejectionReport,
    unsupported_schema: RejectionReport,
}
