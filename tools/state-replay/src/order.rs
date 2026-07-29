use std::{path::Path, path::PathBuf, time::Instant};

use canonical_archive::{ArchiveConfig, LocalParquetArchive};
use canonical_events::{
    BlockEnvelope, CanonicalEventEnvelope, CanonicalEventInput, ConfirmationClass, EventPayload,
    OrderAccepted, OrderCancelled, OrderFilled, OrderModified, OrderPartiallyFilled, OrderRejected,
    OrderRested, SourceEvidence,
};
use canonical_ledger::{
    CanonicalLedger, CanonicalOrderReducerV1, CheckpointArtifact, CheckpointCompatibility,
    LedgerLimits, OrderCurrentRecordV1, OrderFactRecordV1, OrderLifecycleV1,
    OrderTransitionRecordV1, StateImageLimits,
};
use canonical_state_store::LocalCheckpointStore;
use domain_types::{
    Address, BlockHeight, ChainId, ClientOrderId, KnownTime, MarketId, OrderId, OrderSide, Price,
    Quantity, SourceId, TradeId, TransactionId,
};
use replay_engine::{ReplayLimits, ReplayOutcome, SerialReplayEngine};
use serde::Serialize;
use storage_ports::{CanonicalArchive, StateCheckpointStore};

use super::{
    CHAIN, FIXTURE_EPOCH_MICROS, FixtureRunError, NeverCancel, REPORT_FILE, RejectionReport,
    START_HEIGHT, create_private_output_root, fixture_time, publish_report, rejection_report,
    replay_request, source_hashes, validate_atomic_rejection, validate_replay_counts,
};

const ORDER_REPORT_SCHEMA: &str = "hyperliquid-alpha-desk/state-replay-order-e2e-report/v1";
const ORDER_EVIDENCE_CLASS: &str = "synthetic_canonical_order";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderRunConfig {
    pub output_root: PathBuf,
    pub block_count: u64,
    pub checkpoint_after: u64,
    pub iterations: u64,
}

impl OrderRunConfig {
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
pub struct OrderEvidence {
    pub report_path: PathBuf,
}

pub fn run_order_e2e(config: &OrderRunConfig) -> Result<OrderEvidence, FixtureRunError> {
    validate_replay_counts(
        config.block_count,
        config.checkpoint_after,
        config.iterations,
        2,
    )?;
    let output_root = create_private_output_root(&config.output_root)?;
    let archive_root = output_root.join("archive");
    let checkpoint_root = output_root.join("checkpoints");
    let archive = LocalParquetArchive::open(
        &archive_root,
        ArchiveConfig::deterministic_fixture(
            "state-replay-order-e2e-v1",
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
                .append_block(&order_block(height, &chain, "1.0.0")?)?
                .manifest_id()
                .clone(),
        );
    }
    let verified = archive.verify_manifest(
        manifests
            .first()
            .ok_or(FixtureRunError::Invariant("order manifest set is empty"))?,
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
        let mut ledger = empty_order_ledger(chain.clone())?;
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
                "uncancelled order replay was cancelled",
            ));
        };
        match (expected_state_hash, expected_receipt_hash) {
            (Some(state_hash), Some(receipt_hash)) => {
                if ledger.state_hash() != state_hash || receipt.receipt_hash() != receipt_hash {
                    return Err(FixtureRunError::Invariant(
                        "independent order replays diverged",
                    ));
                }
            }
            (None, None) => {
                expected_state_hash = Some(ledger.state_hash());
                expected_receipt_hash = Some(receipt.receipt_hash());
            }
            _ => {
                return Err(FixtureRunError::Invariant(
                    "order replay expectation initialization is inconsistent",
                ));
            }
        }
    }
    let replay_elapsed_micros =
        u64::try_from(replay_started.elapsed().as_micros()).unwrap_or(u64::MAX);
    let expected_state_hash = expected_state_hash.ok_or(FixtureRunError::Invariant(
        "no order replay expectation was produced",
    ))?;
    let expected_receipt_hash = expected_receipt_hash.ok_or(FixtureRunError::Invariant(
        "no order replay receipt was produced",
    ))?;

    let checkpoint_end = START_HEIGHT
        .checked_add(config.checkpoint_after - 1)
        .ok_or(FixtureRunError::InvalidConfig)?;
    let checkpoint_len =
        usize::try_from(config.checkpoint_after).map_err(|_| FixtureRunError::InvalidConfig)?;
    let mut partial = empty_order_ledger(chain.clone())?;
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
            "uncancelled order checkpoint replay was cancelled",
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
        CanonicalOrderReducerV1,
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
            "uncancelled order checkpoint resume was cancelled",
        ));
    };
    if resumed.state_hash() != expected_state_hash {
        return Err(FixtureRunError::Invariant(
            "order checkpoint resume final state diverged",
        ));
    }
    let state_summary = summarize_order_state(&resumed, end_height, config.block_count)?;

    let rejection_height = end_height
        .checked_add(1)
        .ok_or(FixtureRunError::InvalidConfig)?;
    let malformed = malformed_order_block(rejection_height, &chain)?;
    let mut malformed_ledger = CanonicalLedger::try_from_state_image(
        resumed.state_image().clone(),
        CanonicalOrderReducerV1,
        LedgerLimits::production(),
    )?;
    let malformed_before = malformed_ledger.state_hash();
    let direct_error = match malformed_ledger.apply_block(&malformed) {
        Err(error) => error,
        Ok(_) => {
            return Err(FixtureRunError::Invariant(
                "order reducer accepted a malformed order before replay",
            ));
        }
    };
    if malformed_ledger.state_hash() != malformed_before {
        return Err(FixtureRunError::Invariant(
            "malformed order changed state before archive replay",
        ));
    }
    let reducer_reason_code = direct_error
        .reducer_reason_code()
        .ok_or(FixtureRunError::Invariant(
            "malformed order reducer reason is absent",
        ))?
        .to_owned();
    let malformed_manifest = archive.append_block(&malformed)?.manifest_id().clone();
    let malformed_request = replay_request(
        &chain,
        rejection_height,
        rejection_height,
        vec![malformed_manifest],
        malformed_before,
        schema_fingerprint,
    )?;
    let malformed_error =
        match SerialReplayEngine::new(&archive, &mut malformed_ledger, ReplayLimits::production())
            .run(&malformed_request, &NeverCancel)
        {
            Err(error) => error,
            Ok(_) => {
                return Err(FixtureRunError::Invariant(
                    "order reducer accepted a malformed order",
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

    let unsupported_archive = LocalParquetArchive::open(
        output_root.join("unsupported-archive"),
        ArchiveConfig::deterministic_fixture(
            "state-replay-order-unsupported-v1",
            KnownTime::from_unix_micros(FIXTURE_EPOCH_MICROS)?,
        )?,
    )?;
    let unsupported_manifest = unsupported_archive
        .append_block(&order_block(rejection_height, &chain, "1.1.0")?)?
        .manifest_id()
        .clone();
    let mut unsupported_ledger = CanonicalLedger::try_from_state_image(
        resumed.state_image().clone(),
        CanonicalOrderReducerV1,
        LedgerLimits::production(),
    )?;
    let unsupported_before = unsupported_ledger.state_hash();
    let unsupported_request = replay_request(
        &chain,
        rejection_height,
        rejection_height,
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
                "order reducer accepted an unsupported schema",
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

    let report = OrderReport {
        schema_version: ORDER_REPORT_SCHEMA,
        evidence_class: ORDER_EVIDENCE_CLASS,
        state_semantics: "exact_order_lifecycle",
        source_qualification: "synthetic_unassessed",
        reducer_set_version: CanonicalOrderReducerV1::VERSION,
        synthetic_order_contract_proven: true,
        stage_1_qualified: false,
        stage_2_qualified: false,
        live_source_qualified: false,
        deployed_source_qualified: false,
        position_state_qualified: false,
        margin_state_qualified: false,
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
        order_fact_count: state_summary.order_fact_count,
        order_current_count: state_summary.order_current_count,
        order_transition_count: state_summary.order_transition_count,
        filled_order_count: state_summary.filled_order_count,
        cancelled_order_count: state_summary.cancelled_order_count,
        rejection_fact_count: state_summary.rejection_fact_count,
        sample_order: state_summary.sample_order,
        malformed_order: rejection_report(
            rejection_height,
            &malformed_error,
            Some(reducer_reason_code),
            malformed_before,
            malformed_after,
        )?,
        unsupported_schema: rejection_report(
            rejection_height,
            &unsupported_error,
            None,
            unsupported_before,
            unsupported_after,
        )?,
    };
    let report_path = output_root.join(REPORT_FILE);
    publish_report(&report_path, &report)?;
    Ok(OrderEvidence { report_path })
}

fn empty_order_ledger(
    chain: ChainId,
) -> Result<CanonicalLedger<CanonicalOrderReducerV1>, FixtureRunError> {
    Ok(CanonicalLedger::try_new(
        chain,
        BlockHeight::new(START_HEIGHT),
        CanonicalOrderReducerV1,
        LedgerLimits::production(),
    )?)
}

fn order_block(
    height: u64,
    chain: &ChainId,
    schema_version: &str,
) -> Result<BlockEnvelope, FixtureRunError> {
    let account = Address::from_bytes([0x11; 20]);
    let market = MarketId::new("perp:BTC")?;
    let order_id = OrderId::new(format!("state-replay-order-{height}"))?;
    let full_lifecycle = (height - START_HEIGHT).is_multiple_of(2);
    let payloads = if full_lifecycle {
        vec![
            EventPayload::OrderAccepted(OrderAccepted {
                order_id: order_id.clone(),
                account_id: account,
                market_id: market.clone(),
                side: OrderSide::Buy,
                limit_price: Price::parse_at_scale("65000", 6)?,
                quantity: Quantity::parse_at_scale("1", 8)?,
            }),
            EventPayload::OrderRested(OrderRested {
                order_id: order_id.clone(),
                market_id: market.clone(),
                remaining_quantity: Quantity::parse_at_scale("1", 8)?,
                limit_price: Price::parse_at_scale("65000", 6)?,
            }),
            EventPayload::OrderModified(OrderModified {
                order_id: order_id.clone(),
                previous_price: Price::parse_at_scale("65000", 6)?,
                new_price: Price::parse_at_scale("65010", 6)?,
                previous_quantity: Quantity::parse_at_scale("1", 8)?,
                new_quantity: Quantity::parse_at_scale("1.25", 8)?,
            }),
            EventPayload::OrderPartiallyFilled(OrderPartiallyFilled {
                order_id: order_id.clone(),
                trade_id: TradeId::new(format!("state-replay-partial-{height}"))?,
                fill_price: Price::parse_at_scale("65005", 6)?,
                fill_quantity: Quantity::parse_at_scale("0.25", 8)?,
                remaining_quantity: Quantity::parse_at_scale("1", 8)?,
            }),
            EventPayload::OrderFilled(OrderFilled {
                order_id,
                trade_id: TradeId::new(format!("state-replay-terminal-{height}"))?,
                fill_price: Price::parse_at_scale("65010", 6)?,
                fill_quantity: Quantity::parse_at_scale("1", 8)?,
            }),
        ]
    } else {
        vec![
            EventPayload::OrderAccepted(OrderAccepted {
                order_id: order_id.clone(),
                account_id: account,
                market_id: market.clone(),
                side: OrderSide::Sell,
                limit_price: Price::parse_at_scale("3500", 6)?,
                quantity: Quantity::parse_at_scale("2", 8)?,
            }),
            EventPayload::OrderCancelled(OrderCancelled {
                order_id,
                reason: "operator_requested".to_owned(),
                remaining_quantity: Quantity::parse_at_scale("2", 8)?,
            }),
            EventPayload::OrderRejected(OrderRejected {
                client_order_id: ClientOrderId::new(format!("state-replay-rejected-{height}"))?,
                account_id: account,
                reason_code: "invalid_tick".to_owned(),
                reason: "limit price is not aligned to the active tick".to_owned(),
            }),
        ]
    };
    events_block(height, chain, schema_version, account, market, payloads)
}

fn malformed_order_block(height: u64, chain: &ChainId) -> Result<BlockEnvelope, FixtureRunError> {
    let account = Address::from_bytes([0x11; 20]);
    let market = MarketId::new("perp:BTC")?;
    let order_id = OrderId::new(format!("state-replay-malformed-{height}"))?;
    events_block(
        height,
        chain,
        "1.0.0",
        account,
        market.clone(),
        vec![
            EventPayload::OrderAccepted(OrderAccepted {
                order_id: order_id.clone(),
                account_id: account,
                market_id: market,
                side: OrderSide::Buy,
                limit_price: Price::parse_at_scale("65000", 6)?,
                quantity: Quantity::parse_at_scale("1", 8)?,
            }),
            EventPayload::OrderPartiallyFilled(OrderPartiallyFilled {
                order_id,
                trade_id: TradeId::new(format!("state-replay-malformed-trade-{height}"))?,
                fill_price: Price::parse_at_scale("65000", 6)?,
                fill_quantity: Quantity::parse_at_scale("2", 8)?,
                remaining_quantity: Quantity::parse_at_scale("1", 8)?,
            }),
        ],
    )
}

fn events_block(
    height: u64,
    chain: &ChainId,
    schema_version: &str,
    account: Address,
    market: MarketId,
    payloads: Vec<EventPayload>,
) -> Result<BlockEnvelope, FixtureRunError> {
    let time = fixture_time(height)?;
    let mut events = Vec::with_capacity(payloads.len());
    for (index, payload) in payloads.into_iter().enumerate() {
        let event_index = u32::try_from(index).map_err(|_| FixtureRunError::InvalidConfig)?;
        let is_rejection = matches!(payload, EventPayload::OrderRejected(_));
        let payload_hash = *blake3::hash(&payload.encode_to_vec()?).as_bytes();
        events.push(CanonicalEventEnvelope::from_input(CanonicalEventInput {
            schema_version: schema_version.to_owned(),
            chain_id: chain.clone(),
            block_height: BlockHeight::new(height),
            block_time: time,
            transaction_id: TransactionId::new(format!("state-replay-order-tx-{height}"))?,
            transaction_index: 0,
            canonical_event_index: event_index,
            market_ids: if is_rejection {
                Vec::new()
            } else {
                vec![market.clone()]
            },
            account_ids: vec![account],
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
            parser_version: "state-replay-order-fixture-v1".to_owned(),
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

fn summarize_order_state(
    ledger: &CanonicalLedger<CanonicalOrderReducerV1>,
    end_height: u64,
    expected_block_count: u64,
) -> Result<OrderStateSummary, FixtureRunError> {
    let mut order_fact_count = 0_u64;
    let mut order_current_count = 0_u64;
    let mut order_transition_count = 0_u64;
    let mut filled_order_count = 0_u64;
    let mut cancelled_order_count = 0_u64;
    let mut rejection_fact_count = 0_u64;
    for (key, bytes) in ledger.state_image().entries() {
        match key.namespace() {
            "order-fact.v1" => {
                let fact = OrderFactRecordV1::decode_at(key, bytes)?;
                order_fact_count = checked_count(order_fact_count)?;
                if fact.event_kind() == canonical_events::EventKind::OrderRejected {
                    rejection_fact_count = checked_count(rejection_fact_count)?;
                }
            }
            "order-current.v1" => {
                let current = OrderCurrentRecordV1::decode_at(key, bytes)?;
                order_current_count = checked_count(order_current_count)?;
                match current.lifecycle() {
                    OrderLifecycleV1::Filled => {
                        filled_order_count = checked_count(filled_order_count)?;
                    }
                    OrderLifecycleV1::Cancelled => {
                        cancelled_order_count = checked_count(cancelled_order_count)?;
                    }
                    _ => {
                        return Err(FixtureRunError::Invariant(
                            "synthetic order did not reach a terminal state",
                        ));
                    }
                }
            }
            "order-transition.v1" => {
                OrderTransitionRecordV1::decode_at(key, bytes)?;
                order_transition_count = checked_count(order_transition_count)?;
            }
            _ => {
                return Err(FixtureRunError::Invariant(
                    "order state contains an unexpected namespace",
                ));
            }
        }
    }
    let filled_expected = expected_block_count
        .checked_add(1)
        .ok_or(FixtureRunError::InvalidConfig)?
        / 2;
    let cancelled_expected = expected_block_count / 2;
    let fact_expected = filled_expected
        .checked_mul(5)
        .and_then(|count| {
            cancelled_expected
                .checked_mul(3)
                .and_then(|cancelled| count.checked_add(cancelled))
        })
        .ok_or(FixtureRunError::InvalidConfig)?;
    if order_fact_count != fact_expected
        || order_current_count != expected_block_count
        || order_transition_count != fact_expected
        || filled_order_count != filled_expected
        || cancelled_order_count != cancelled_expected
        || rejection_fact_count != cancelled_expected
    {
        return Err(FixtureRunError::Invariant(
            "order state record cardinality is inconsistent",
        ));
    }

    let sample_order_id = OrderId::new(format!("state-replay-order-{end_height}"))?;
    let sample_market = MarketId::new("perp:BTC")?;
    let sample_key = OrderCurrentRecordV1::state_key(&sample_market, &sample_order_id)?;
    let sample = OrderCurrentRecordV1::decode_at(
        &sample_key,
        ledger
            .state_image()
            .entries()
            .get(&sample_key)
            .ok_or(FixtureRunError::Invariant(
                "final order current sample is absent",
            ))?,
    )?;
    Ok(OrderStateSummary {
        order_fact_count,
        order_current_count,
        order_transition_count,
        filled_order_count,
        cancelled_order_count,
        rejection_fact_count,
        sample_order: OrderSample {
            order_id: sample.order_id().as_str().to_owned(),
            account_id: sample.account_id().to_api_string(),
            market_id: sample.market_id().as_str().to_owned(),
            side: sample.side().as_wire_name(),
            lifecycle: lifecycle_name(sample.lifecycle()),
            accepted_quantity: sample.accepted_quantity().to_string(),
            filled_quantity: sample.filled_quantity().to_string(),
            remaining_quantity: sample.remaining_quantity().to_string(),
        },
    })
}

fn checked_count(value: u64) -> Result<u64, FixtureRunError> {
    value
        .checked_add(1)
        .ok_or(FixtureRunError::Invariant("order record count overflow"))
}

const fn lifecycle_name(lifecycle: OrderLifecycleV1) -> &'static str {
    match lifecycle {
        OrderLifecycleV1::Accepted => "accepted",
        OrderLifecycleV1::Rested => "rested",
        OrderLifecycleV1::Modified => "modified",
        OrderLifecycleV1::PartiallyFilled => "partially_filled",
        OrderLifecycleV1::Filled => "filled",
        OrderLifecycleV1::Cancelled => "cancelled",
    }
}

#[derive(Debug)]
struct OrderStateSummary {
    order_fact_count: u64,
    order_current_count: u64,
    order_transition_count: u64,
    filled_order_count: u64,
    cancelled_order_count: u64,
    rejection_fact_count: u64,
    sample_order: OrderSample,
}

#[derive(Debug, Serialize)]
struct OrderSample {
    order_id: String,
    account_id: String,
    market_id: String,
    side: &'static str,
    lifecycle: &'static str,
    accepted_quantity: String,
    filled_quantity: String,
    remaining_quantity: String,
}

#[derive(Debug, Serialize)]
struct OrderReport<'a> {
    schema_version: &'static str,
    evidence_class: &'static str,
    state_semantics: &'static str,
    source_qualification: &'static str,
    reducer_set_version: &'static str,
    synthetic_order_contract_proven: bool,
    stage_1_qualified: bool,
    stage_2_qualified: bool,
    live_source_qualified: bool,
    deployed_source_qualified: bool,
    position_state_qualified: bool,
    margin_state_qualified: bool,
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
    order_fact_count: u64,
    order_current_count: u64,
    order_transition_count: u64,
    filled_order_count: u64,
    cancelled_order_count: u64,
    rejection_fact_count: u64,
    sample_order: OrderSample,
    malformed_order: RejectionReport,
    unsupported_schema: RejectionReport,
}
