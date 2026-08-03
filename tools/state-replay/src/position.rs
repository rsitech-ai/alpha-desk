use std::{collections::BTreeMap, path::PathBuf, str::FromStr, time::Instant};

use canonical_archive::{ArchiveConfig, LocalParquetArchive};
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
    CHAIN, FIXTURE_EPOCH_MICROS, FixtureRunError, NeverCancel, REPORT_FILE, RejectionReport,
    START_HEIGHT, account, create_private_output_root, fixture_time, harden_private_tree,
    publish_report, rejection_report, replay_request, validate_atomic_rejection,
    validate_replay_counts,
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
            .checked_add(7)
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
    let archive = open_archive(&output.join("archive"), POSITION_ARCHIVE_ID)?;
    let manifests = append_blocks(&archive, &scenario.blocks)?;
    let schema_fingerprint = schema_fingerprint(&archive, &manifests)?;

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
        if height == checkpoint_end + 5 {
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
        duplicate_trade_identity,
        start_position_mismatch,
        unsupported_schema,
    };
    let report_path = output.join(REPORT_FILE);
    publish_report(&report_path, &report)?;
    harden_private_tree(&output)?;
    Ok(PositionEvidence { report_path })
}

#[derive(Debug)]
struct Scenario {
    blocks: Vec<BlockEnvelope>,
    expected: FrozenExpectation,
    checkpoint_height: u64,
    end_height: u64,
}

#[derive(Debug)]
struct FrozenExpectation {
    market: MarketId,
    liquidation: LiquidationId,
    opening_event: EventId,
    reversal_event: EventId,
    first_funding_event: EventId,
    liquidation_start_event: EventId,
    liquidation_fill_event: EventId,
    backstop_event: EventId,
    interrupted_funding_event: EventId,
    settlement_event: EventId,
    recovery_event: EventId,
    recovered_funding_event: EventId,
    opening_buyer_episode: PositionEpisodeId,
    opening_seller_episode: PositionEpisodeId,
    reversal_buyer_episode: PositionEpisodeId,
    reversal_seller_episode: PositionEpisodeId,
    liquidation_remainder_episode: PositionEpisodeId,
    recovery_buyer_episode: PositionEpisodeId,
    recovery_seller_episode: PositionEpisodeId,
}

fn build_scenario(
    config: &PositionRunConfig,
    chain: &ChainId,
    settlement_pnl: &str,
) -> Result<Scenario, FixtureRunError> {
    let checkpoint_height = START_HEIGHT + config.checkpoint_after - 1;
    let end_height = START_HEIGHT + config.blocks - 1;
    let market = MarketId::new(MARKET)?;
    let liquidation = LiquidationId::new(LIQUIDATION)?;
    let mut blocks = Vec::with_capacity(
        usize::try_from(config.blocks).map_err(|_| FixtureRunError::InvalidConfig)?,
    );
    let mut opening_event = None;
    let mut reversal_event = None;
    let mut first_funding_event = None;
    let mut liquidation_start_event = None;
    let mut liquidation_fill_event = None;
    let mut backstop_event = None;
    let mut interrupted_funding_event = None;
    let mut settlement_event = None;
    let mut recovery_event = None;
    let mut recovered_funding_event = None;

    for height in START_HEIGHT..=end_height {
        let mut events = if height == START_HEIGHT {
            account::market_prerequisite_events(height)?
        } else {
            Vec::new()
        };
        if height == checkpoint_height {
            events.extend(order_prerequisites(height, events.len() as u32, &market)?);
            let event = trade_event(
                height,
                events.len() as u32,
                OPEN_TRADE,
                "position-open",
                BUYER,
                SELLER,
                "0.00000000",
                "0.00000000",
                "position-open-buyer-order",
                "position-open-seller-order",
                "100.000000",
                "2.00000000",
                "1.0.0",
            )?;
            opening_event = Some(event.event_id().clone());
            events.push(event);
        } else if height == checkpoint_height + 1 {
            let event = trade_event(
                height,
                0,
                REVERSAL_TRADE,
                "position-reversal",
                SELLER,
                BUYER,
                "-2.00000000",
                "2.00000000",
                "position-reversal-buyer-order",
                "position-reversal-seller-order",
                "110.000000",
                "3.00000000",
                "1.0.0",
            )?;
            reversal_event = Some(event.event_id().clone());
            events.push(event);
        } else if height == checkpoint_height + 2 {
            let event = position_event(
                height,
                0,
                "position-first-funding",
                EventPayload::FundingPaid(FundingPaid {
                    account_id: BUYER,
                    market_id: market.clone(),
                    amount: QuoteAmount::from_str("1.25")?,
                    funding_rate: FundingRate::from_str("0.0001")?,
                }),
                vec![market.clone()],
                vec![BUYER],
                "1.0.0",
            )?;
            first_funding_event = Some(event.event_id().clone());
            events.push(event);
        } else if height == checkpoint_height + 3 {
            let event = position_event(
                height,
                0,
                "position-liquidation-start",
                EventPayload::LiquidationStarted(LiquidationStarted {
                    account_id: BUYER,
                    liquidation_id: liquidation.clone(),
                    margin_value: UsdAmount::from_str("9")?,
                    maintenance_requirement: UsdAmount::from_str("10")?,
                }),
                vec![],
                vec![BUYER],
                "1.0.0",
            )?;
            liquidation_start_event = Some(event.event_id().clone());
            events.push(event);
        } else if height == checkpoint_height + 4 {
            let event = position_event(
                height,
                0,
                "position-liquidation-fill",
                EventPayload::LiquidationFill(LiquidationFill {
                    liquidation_id: liquidation.clone(),
                    account_id: BUYER,
                    market_id: market.clone(),
                    price: Price::parse_at_scale("90", 6)?,
                    quantity: Quantity::parse_at_scale("0.25", 8)?,
                }),
                vec![market.clone()],
                vec![BUYER],
                "1.0.0",
            )?;
            liquidation_fill_event = Some(event.event_id().clone());
            events.push(event);
        } else if height == checkpoint_height + 5 {
            let backstop = position_event(
                height,
                0,
                "position-backstop",
                EventPayload::BackstopLiquidation(BackstopLiquidation {
                    liquidation_id: liquidation.clone(),
                    account_id: BUYER,
                    backstop_account_id: SELLER,
                    market_id: market.clone(),
                    quantity: Quantity::parse_at_scale("0.5", 8)?,
                }),
                vec![market.clone()],
                vec![BUYER, SELLER],
                "1.0.0",
            )?;
            backstop_event = Some(backstop.event_id().clone());
            events.push(backstop);
            let funding = position_event(
                height,
                1,
                "position-interrupted-funding",
                EventPayload::FundingPaid(FundingPaid {
                    account_id: BUYER,
                    market_id: market.clone(),
                    amount: QuoteAmount::from_str("0.5")?,
                    funding_rate: FundingRate::from_str("0.0001")?,
                }),
                vec![market.clone()],
                vec![BUYER],
                "1.0.0",
            )?;
            interrupted_funding_event = Some(funding.event_id().clone());
            events.push(funding);
        } else if height == checkpoint_height + 6 {
            let event = position_event(
                height,
                0,
                "position-settlement",
                EventPayload::PositionSettled(PositionSettled {
                    account_id: BUYER,
                    market_id: market.clone(),
                    settlement_price: Price::parse_at_scale("0", 6)?,
                    settled_quantity: Quantity::parse_at_scale("0.25", 8)?,
                    realized_pnl: QuoteAmount::from_str(settlement_pnl)?,
                }),
                vec![market.clone()],
                vec![BUYER],
                "1.0.0",
            )?;
            settlement_event = Some(event.event_id().clone());
            events.push(event);
        } else if height == checkpoint_height + 7 {
            let recovery = trade_event(
                height,
                0,
                RECOVERY_TRADE,
                "position-recovery",
                BUYER,
                SELLER,
                "4.00000000",
                "0.00000000",
                "position-recovery-buyer-order",
                "position-recovery-seller-order",
                "95.000000",
                "0.25000000",
                "1.0.0",
            )?;
            recovery_event = Some(recovery.event_id().clone());
            events.push(recovery);
            let funding = position_event(
                height,
                1,
                "position-recovered-funding",
                EventPayload::FundingPaid(FundingPaid {
                    account_id: BUYER,
                    market_id: market.clone(),
                    amount: QuoteAmount::from_str("0.75")?,
                    funding_rate: FundingRate::from_str("0.0001")?,
                }),
                vec![market.clone()],
                vec![BUYER],
                "1.0.0",
            )?;
            recovered_funding_event = Some(funding.event_id().clone());
            events.push(funding);
        }
        blocks.push(account::block(height, chain, events)?);
    }

    let opening_event = opening_event.ok_or(FixtureRunError::InvalidConfig)?;
    let reversal_event = reversal_event.ok_or(FixtureRunError::InvalidConfig)?;
    let recovery_event = recovery_event.ok_or(FixtureRunError::InvalidConfig)?;
    let expected = FrozenExpectation {
        market: market.clone(),
        liquidation,
        opening_buyer_episode: derive_position_episode_id(&BUYER, &market, &opening_event, 0)
            .map_err(|_| FixtureRunError::PositionSemanticMismatch)?,
        opening_seller_episode: derive_position_episode_id(&SELLER, &market, &opening_event, 0)
            .map_err(|_| FixtureRunError::PositionSemanticMismatch)?,
        reversal_buyer_episode: derive_position_episode_id(&SELLER, &market, &reversal_event, 1)
            .map_err(|_| FixtureRunError::PositionSemanticMismatch)?,
        reversal_seller_episode: derive_position_episode_id(&BUYER, &market, &reversal_event, 1)
            .map_err(|_| FixtureRunError::PositionSemanticMismatch)?,
        liquidation_remainder_episode: derive_position_episode_id(
            &BUYER,
            &market,
            liquidation_fill_event
                .as_ref()
                .ok_or(FixtureRunError::InvalidConfig)?,
            1,
        )
        .map_err(|_| FixtureRunError::PositionSemanticMismatch)?,
        recovery_buyer_episode: derive_position_episode_id(&BUYER, &market, &recovery_event, 0)
            .map_err(|_| FixtureRunError::PositionSemanticMismatch)?,
        recovery_seller_episode: derive_position_episode_id(&SELLER, &market, &recovery_event, 0)
            .map_err(|_| FixtureRunError::PositionSemanticMismatch)?,
        opening_event,
        reversal_event,
        first_funding_event: first_funding_event.ok_or(FixtureRunError::InvalidConfig)?,
        liquidation_start_event: liquidation_start_event.ok_or(FixtureRunError::InvalidConfig)?,
        liquidation_fill_event: liquidation_fill_event.ok_or(FixtureRunError::InvalidConfig)?,
        backstop_event: backstop_event.ok_or(FixtureRunError::InvalidConfig)?,
        interrupted_funding_event: interrupted_funding_event
            .ok_or(FixtureRunError::InvalidConfig)?,
        settlement_event: settlement_event.ok_or(FixtureRunError::InvalidConfig)?,
        recovery_event,
        recovered_funding_event: recovered_funding_event.ok_or(FixtureRunError::InvalidConfig)?,
    };
    Ok(Scenario {
        blocks,
        expected,
        checkpoint_height,
        end_height,
    })
}

fn order_prerequisites(
    height: u64,
    start_index: u32,
    market: &MarketId,
) -> Result<Vec<CanonicalEventEnvelope>, FixtureRunError> {
    let specs = [
        ("position-open-buyer-order", BUYER, OrderSide::Buy, "2"),
        ("position-open-seller-order", SELLER, OrderSide::Sell, "2"),
        ("position-reversal-buyer-order", SELLER, OrderSide::Buy, "3"),
        (
            "position-reversal-seller-order",
            BUYER,
            OrderSide::Sell,
            "3",
        ),
        (
            "position-recovery-buyer-order",
            BUYER,
            OrderSide::Buy,
            "0.25",
        ),
        (
            "position-recovery-seller-order",
            SELLER,
            OrderSide::Sell,
            "0.25",
        ),
    ];
    specs
        .into_iter()
        .enumerate()
        .map(|(offset, (order_id, account_id, side, quantity))| {
            position_event(
                height,
                start_index + u32::try_from(offset).map_err(|_| FixtureRunError::InvalidConfig)?,
                order_id,
                EventPayload::OrderAccepted(OrderAccepted {
                    order_id: OrderId::new(order_id)?,
                    account_id,
                    market_id: market.clone(),
                    side,
                    limit_price: Price::parse_at_scale("100", 6)?,
                    quantity: Quantity::parse_at_scale(quantity, 8)?,
                }),
                vec![market.clone()],
                vec![account_id],
                "1.0.0",
            )
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn trade_event(
    height: u64,
    index: u32,
    trade_id: &str,
    transaction: &str,
    buyer: Address,
    seller: Address,
    buyer_start: &str,
    seller_start: &str,
    buyer_order: &str,
    seller_order: &str,
    price: &str,
    quantity: &str,
    schema: &str,
) -> Result<CanonicalEventEnvelope, FixtureRunError> {
    let market = MarketId::new(MARKET)?;
    position_event(
        height,
        index,
        transaction,
        EventPayload::TradeMatched(TradeMatched {
            trade_id: Some(TradeId::new(trade_id)?),
            market_id: Some(market.clone()),
            maker_order_id: Some(OrderId::new(seller_order)?),
            taker_order_id: Some(OrderId::new(buyer_order)?),
            price: Price::parse_at_scale(price, 6)?,
            quantity: Quantity::parse_at_scale(quantity, 8)?,
            deterministic_seed: height,
            participants: Some(Box::new([
                TradeParticipantV1 {
                    role: TradeParticipantRoleV1::Buyer,
                    account_id: buyer,
                    start_position: PositionQuantity::from_str(buyer_start)?,
                    order_id: OrderId::new(buyer_order)?,
                    twap_id: None,
                    client_order_id: None,
                },
                TradeParticipantV1 {
                    role: TradeParticipantRoleV1::Seller,
                    account_id: seller,
                    start_position: PositionQuantity::from_str(seller_start)?,
                    order_id: OrderId::new(seller_order)?,
                    twap_id: None,
                    client_order_id: None,
                },
            ])),
        }),
        vec![market],
        vec![buyer, seller],
        schema,
    )
}

fn position_event(
    height: u64,
    index: u32,
    transaction: &str,
    payload: EventPayload,
    market_ids: Vec<MarketId>,
    account_ids: Vec<Address>,
    schema: &str,
) -> Result<CanonicalEventEnvelope, FixtureRunError> {
    let time = fixture_time(height)?;
    let payload_hash = *blake3::hash(&payload.encode_to_vec()?).as_bytes();
    Ok(CanonicalEventEnvelope::from_input(CanonicalEventInput {
        schema_version: schema.to_owned(),
        chain_id: ChainId::new(CHAIN)?,
        block_height: BlockHeight::new(height),
        block_time: time,
        transaction_id: TransactionId::new(format!("state-replay-{transaction}"))?,
        transaction_index: index,
        canonical_event_index: 0,
        market_ids,
        account_ids,
        source_evidence: vec![SourceEvidence::try_new_indexed(
            SourceId::new("state-replay-position")?,
            "v1",
            height.to_string(),
            payload_hash,
            index,
        )?],
        confirmation_class: ConfirmationClass::CommittedPrimary,
        observed_at: KnownTime::from_unix_micros(time.unix_micros())?,
        ingested_at: KnownTime::from_unix_micros(time.unix_micros())?,
        canonicalized_at: KnownTime::from_unix_micros(time.unix_micros())?,
        parser_version: "state-replay-position-fixture-v1".to_owned(),
        payload,
    })?)
}

fn open_archive(path: &std::path::Path, id: &str) -> Result<LocalParquetArchive, FixtureRunError> {
    Ok(LocalParquetArchive::open(
        path,
        ArchiveConfig::deterministic_fixture(
            id,
            KnownTime::from_unix_micros(FIXTURE_EPOCH_MICROS)?,
        )?,
    )?)
}

fn append_blocks(
    archive: &LocalParquetArchive,
    blocks: &[BlockEnvelope],
) -> Result<Vec<domain_types::ManifestId>, FixtureRunError> {
    blocks
        .iter()
        .map(|block| Ok(archive.append_block(block)?.manifest_id().clone()))
        .collect()
}

fn schema_fingerprint(
    archive: &LocalParquetArchive,
    manifests: &[domain_types::ManifestId],
) -> Result<[u8; 32], FixtureRunError> {
    Ok(*archive
        .verify_manifest(
            manifests
                .first()
                .ok_or(FixtureRunError::Invariant("missing position manifest"))?,
        )?
        .schema_fingerprints()
        .get("canonical_events")
        .ok_or(FixtureRunError::Invariant(
            "missing canonical schema fingerprint",
        ))?)
}

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
    let archive = open_archive(
        &output.join("semantic-variant"),
        "state-replay-position-semantic-v1",
    )?;
    let manifests = append_blocks(&archive, &scenario.blocks)?;
    let fingerprint = schema_fingerprint(&archive, &manifests)?;
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
    let archive = open_archive(
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

fn validate_checkpoint_entries(
    entries: &BTreeMap<StateKey, Vec<u8>>,
    expected: &FrozenExpectation,
) -> Result<(), FixtureRunError> {
    assert_quantity(
        entries,
        BUYER,
        &expected.market,
        Some("2.00000000"),
        &expected.opening_event,
    )?;
    assert_quantity(
        entries,
        SELLER,
        &expected.market,
        Some("-2.00000000"),
        &expected.opening_event,
    )?;
    assert_position_effect(
        entries,
        OPEN_TRADE,
        TradeParticipantRoleV1::Buyer,
        BUYER,
        "0.00000000",
        "2.00000000",
        PositionAnchorTransitionV1::FirstObservation,
    )?;
    assert_position_effect(
        entries,
        OPEN_TRADE,
        TradeParticipantRoleV1::Seller,
        SELLER,
        "0.00000000",
        "-2.00000000",
        PositionAnchorTransitionV1::FirstObservation,
    )?;
    assert_open_episode(
        entries,
        &expected.opening_buyer_episode,
        BUYER,
        &expected.opening_event,
        EpisodeCompletenessV1::CompleteFromFlat,
        &expected.opening_event,
        "2.00000000",
        "200",
        "0.00000000",
        "0",
        "0",
    )?;
    assert_open_episode(
        entries,
        &expected.opening_seller_episode,
        SELLER,
        &expected.opening_event,
        EpisodeCompletenessV1::CompleteFromFlat,
        &expected.opening_event,
        "0.00000000",
        "0",
        "2.00000000",
        "200",
        "0",
    )?;
    if namespace_count(entries, "position-quantity-current.v1") != 2
        || namespace_count(entries, "position-effect-fact.v1") != 2
        || namespace_count(entries, "position-episode-current.v1") != 2
        || namespace_count(entries, "position-episode.v1") != 2
        || namespace_count(entries, "position-episode-effect-fact.v1") != 2
    {
        return Err(FixtureRunError::PositionSemanticMismatch);
    }
    Ok(())
}

fn validate_interrupted_entries(
    entries: &BTreeMap<StateKey, Vec<u8>>,
    expected: &FrozenExpectation,
) -> Result<(), FixtureRunError> {
    assert_quantity(
        entries,
        BUYER,
        &expected.market,
        None,
        &expected.backstop_event,
    )?;
    assert_quantity(
        entries,
        SELLER,
        &expected.market,
        None,
        &expected.backstop_event,
    )?;
    for account in [BUYER, SELLER] {
        let current_key = PositionEpisodeCurrentRecordV1::state_key(&account, &expected.market)
            .map_err(|_| FixtureRunError::PositionSemanticMismatch)?;
        let current = decode_at(
            entries,
            &current_key,
            PositionEpisodeCurrentRecordV1::decode_at,
        )?;
        if current.account_id() != account
            || current.episode_id().is_some()
            || current.attribution_resolution() != EpisodeAttributionResolutionV1::Interrupted
            || current.last_event_id() != &expected.backstop_event
        {
            return Err(FixtureRunError::PositionSemanticMismatch);
        }
        let cause_key = PositionUnresolvedCauseFactRecordV1::state_key(
            &account,
            &expected.market,
            &expected.backstop_event,
            &expected.liquidation,
        )
        .map_err(|_| FixtureRunError::PositionSemanticMismatch)?;
        let cause = decode_at(
            entries,
            &cause_key,
            PositionUnresolvedCauseFactRecordV1::decode_at,
        )?;
        if cause.cause() != PositionUnresolvedCauseV1::BackstopLiquidation {
            return Err(FixtureRunError::PositionSemanticMismatch);
        }
    }
    assert_interrupted_episode(
        entries,
        &expected.liquidation_remainder_episode,
        BUYER,
        &expected.backstop_event,
        EpisodeCloseCauseV1::BackstopInterrupted,
    )?;
    assert_interrupted_episode(
        entries,
        &expected.reversal_buyer_episode,
        SELLER,
        &expected.backstop_event,
        EpisodeCloseCauseV1::BackstopInterrupted,
    )?;
    if entries.iter().any(|(key, bytes)| {
        (key.namespace() == "position-episode.v1"
            || key.namespace() == "position-episode-effect-fact.v1")
            && bytes
                .windows(expected.interrupted_funding_event.as_str().len())
                .any(|window| window == expected.interrupted_funding_event.as_str().as_bytes())
    }) {
        return Err(FixtureRunError::PositionSemanticMismatch);
    }
    Ok(())
}

fn validate_final_entries(
    entries: &BTreeMap<StateKey, Vec<u8>>,
    expected: &FrozenExpectation,
) -> Result<(), FixtureRunError> {
    assert_quantity(
        entries,
        BUYER,
        &expected.market,
        Some("4.25000000"),
        &expected.recovery_event,
    )?;
    assert_quantity(
        entries,
        SELLER,
        &expected.market,
        Some("-0.25000000"),
        &expected.recovery_event,
    )?;
    assert_position_effect(
        entries,
        RECOVERY_TRADE,
        TradeParticipantRoleV1::Buyer,
        BUYER,
        "4.00000000",
        "4.25000000",
        PositionAnchorTransitionV1::ReanchoredFromUnresolved,
    )?;
    assert_position_effect(
        entries,
        RECOVERY_TRADE,
        TradeParticipantRoleV1::Seller,
        SELLER,
        "0.00000000",
        "-0.25000000",
        PositionAnchorTransitionV1::ReanchoredFromUnresolved,
    )?;
    assert_open_episode(
        entries,
        &expected.recovery_buyer_episode,
        BUYER,
        &expected.recovery_event,
        EpisodeCompletenessV1::PartialFromFirstObservation,
        &expected.recovered_funding_event,
        "0.25000000",
        "23.75",
        "0.00000000",
        "0",
        "0.75",
    )?;
    assert_open_episode(
        entries,
        &expected.recovery_seller_episode,
        SELLER,
        &expected.recovery_event,
        EpisodeCompletenessV1::CompleteFromFlat,
        &expected.recovery_event,
        "0.00000000",
        "0",
        "0.25000000",
        "23.75",
        "0",
    )?;
    assert_episode_snapshot(
        entries,
        &expected.opening_buyer_episode,
        BUYER,
        &expected.opening_event,
        0,
        "0.00000000",
        EpisodeCompletenessV1::CompleteFromFlat,
        "2.00000000",
        "200",
        "2.00000000",
        "220",
        "0",
        EpisodeStatusV1::Closed,
        &expected.reversal_event,
        EpisodeCloseCauseV1::TradeReversal,
    )?;
    assert_episode_snapshot(
        entries,
        &expected.opening_seller_episode,
        SELLER,
        &expected.opening_event,
        0,
        "0.00000000",
        EpisodeCompletenessV1::CompleteFromFlat,
        "2.00000000",
        "220",
        "2.00000000",
        "200",
        "0",
        EpisodeStatusV1::Closed,
        &expected.reversal_event,
        EpisodeCloseCauseV1::TradeReversal,
    )?;
    assert_episode_snapshot(
        entries,
        &expected.reversal_seller_episode,
        BUYER,
        &expected.reversal_event,
        1,
        "0.00000000",
        EpisodeCompletenessV1::CompleteFromFlat,
        "0.00000000",
        "0",
        "1.00000000",
        "110",
        "1.25",
        EpisodeStatusV1::Interrupted,
        &expected.liquidation_fill_event,
        EpisodeCloseCauseV1::LiquidationFill,
    )?;
    assert_episode_snapshot(
        entries,
        &expected.reversal_buyer_episode,
        SELLER,
        &expected.reversal_event,
        1,
        "0.00000000",
        EpisodeCompletenessV1::CompleteFromFlat,
        "1.00000000",
        "110",
        "0.00000000",
        "0",
        "0",
        EpisodeStatusV1::Interrupted,
        &expected.backstop_event,
        EpisodeCloseCauseV1::BackstopInterrupted,
    )?;
    assert_episode_snapshot(
        entries,
        &expected.liquidation_remainder_episode,
        BUYER,
        &expected.liquidation_fill_event,
        1,
        "-0.75000000",
        EpisodeCompletenessV1::PartialFromFirstObservation,
        "0.00000000",
        "0",
        "0.00000000",
        "0",
        "0",
        EpisodeStatusV1::Interrupted,
        &expected.backstop_event,
        EpisodeCloseCauseV1::BackstopInterrupted,
    )?;
    assert_episode_effect(
        entries,
        &expected.reversal_event,
        BUYER,
        0,
        &expected.opening_buyer_episode,
        EpisodeEffectKindV1::Closed,
        "0.00000000",
        "0",
        "2.00000000",
        "220",
        "0",
        Some(EpisodeCloseCauseV1::TradeReversal),
    )?;
    assert_episode_effect(
        entries,
        &expected.reversal_event,
        BUYER,
        1,
        &expected.reversal_seller_episode,
        EpisodeEffectKindV1::Opened,
        "0.00000000",
        "0",
        "1.00000000",
        "110",
        "0",
        None,
    )?;
    assert_episode_effect(
        entries,
        &expected.reversal_event,
        SELLER,
        0,
        &expected.opening_seller_episode,
        EpisodeEffectKindV1::Closed,
        "2.00000000",
        "220",
        "0.00000000",
        "0",
        "0",
        Some(EpisodeCloseCauseV1::TradeReversal),
    )?;
    assert_episode_effect(
        entries,
        &expected.reversal_event,
        SELLER,
        1,
        &expected.reversal_buyer_episode,
        EpisodeEffectKindV1::Opened,
        "1.00000000",
        "110",
        "0.00000000",
        "0",
        "0",
        None,
    )?;
    assert_episode_effect(
        entries,
        &expected.first_funding_event,
        BUYER,
        0,
        &expected.reversal_seller_episode,
        EpisodeEffectKindV1::Updated,
        "0.00000000",
        "0",
        "0.00000000",
        "0",
        "1.25",
        None,
    )?;
    assert_episode_effect(
        entries,
        &expected.recovered_funding_event,
        BUYER,
        0,
        &expected.recovery_buyer_episode,
        EpisodeEffectKindV1::Updated,
        "0.00000000",
        "0",
        "0.00000000",
        "0",
        "0.75",
        None,
    )?;
    assert_current_episode(
        entries,
        BUYER,
        &expected.market,
        &expected.recovery_buyer_episode,
    )?;
    assert_current_episode(
        entries,
        SELLER,
        &expected.market,
        &expected.recovery_seller_episode,
    )?;

    for account in [BUYER, SELLER] {
        let key = PositionUnresolvedCauseFactRecordV1::state_key(
            &account,
            &expected.market,
            &expected.backstop_event,
            &expected.liquidation,
        )
        .map_err(|_| FixtureRunError::PositionSemanticMismatch)?;
        let record = decode_at(
            entries,
            &key,
            PositionUnresolvedCauseFactRecordV1::decode_at,
        )?;
        if record.account_id() != account
            || record.market_id() != &expected.market
            || record.event_id() != &expected.backstop_event
            || record.liquidation_id() != &expected.liquidation
            || record.cause() != PositionUnresolvedCauseV1::BackstopLiquidation
        {
            return Err(FixtureRunError::PositionSemanticMismatch);
        }
    }

    let current_key = LiquidationCurrentRecordV1::state_key(&expected.liquidation)
        .map_err(|_| FixtureRunError::PositionSemanticMismatch)?;
    let current = decode_at(entries, &current_key, LiquidationCurrentRecordV1::decode_at)?;
    if current.account_id() != BUYER
        || current.observed_status() != LiquidationObservedStatusV1::BackstopObserved
        || current.start_margin_value() != UsdAmount::from_str("9")?
        || current.start_maintenance_requirement() != UsdAmount::from_str("10")?
        || current.start_event_id() != &expected.liquidation_start_event
        || current.first_backstop_event_id() != Some(&expected.backstop_event)
    {
        return Err(FixtureRunError::PositionSemanticMismatch);
    }
    let start_key = LiquidationStartFactRecordV1::state_key(
        &expected.liquidation,
        &expected.liquidation_start_event,
    )
    .map_err(|_| FixtureRunError::PositionSemanticMismatch)?;
    let start = decode_at(entries, &start_key, LiquidationStartFactRecordV1::decode_at)?;
    if start.account_id() != BUYER
        || start.margin_value() != UsdAmount::from_str("9")?
        || start.maintenance_requirement() != UsdAmount::from_str("10")?
    {
        return Err(FixtureRunError::PositionSemanticMismatch);
    }
    let fill_key = LiquidationFillFactRecordV1::state_key(
        &expected.liquidation,
        &expected.liquidation_fill_event,
    )
    .map_err(|_| FixtureRunError::PositionSemanticMismatch)?;
    let fill = decode_at(entries, &fill_key, LiquidationFillFactRecordV1::decode_at)?;
    if fill.account_id() != BUYER
        || fill.market_id() != &expected.market
        || fill.price() != Price::parse_at_scale("90", 6)?
        || fill.quantity() != Quantity::parse_at_scale("0.25", 8)?
    {
        return Err(FixtureRunError::PositionSemanticMismatch);
    }
    let flow_key = LiquidationMarketFlowCurrentRecordV1::state_key(
        &expected.liquidation,
        &BUYER,
        &expected.market,
    )
    .map_err(|_| FixtureRunError::PositionSemanticMismatch)?;
    let flow = decode_at(
        entries,
        &flow_key,
        LiquidationMarketFlowCurrentRecordV1::decode_at,
    )?;
    if flow.observed_filled_quantity() != Quantity::parse_at_scale("0.25", 8)?
        || flow.first_fill_event_id() != &expected.liquidation_fill_event
        || flow.last_fill_event_id() != &expected.liquidation_fill_event
    {
        return Err(FixtureRunError::PositionSemanticMismatch);
    }
    let backstop_key =
        BackstopLiquidationFactRecordV1::state_key(&expected.liquidation, &expected.backstop_event)
            .map_err(|_| FixtureRunError::PositionSemanticMismatch)?;
    let backstop = decode_at(
        entries,
        &backstop_key,
        BackstopLiquidationFactRecordV1::decode_at,
    )?;
    if backstop.account_id() != BUYER
        || backstop.backstop_account_id() != SELLER
        || backstop.market_id() != &expected.market
        || backstop.quantity() != Quantity::parse_at_scale("0.5", 8)?
        || backstop.transfer_price_resolution()
            != LiquidationSourceValueResolutionV1::UnavailableFromSource
        || backstop.entry_price_resolution()
            != LiquidationSourceValueResolutionV1::UnavailableFromSource
    {
        return Err(FixtureRunError::PositionSemanticMismatch);
    }
    let settlement_key = PositionSettlementFactRecordV1::state_key(
        &expected.settlement_event,
        &BUYER,
        &expected.market,
    )
    .map_err(|_| FixtureRunError::PositionSemanticMismatch)?;
    let settlement = decode_at(
        entries,
        &settlement_key,
        PositionSettlementFactRecordV1::decode_at,
    )?;
    if settlement.settlement_price() != Price::parse_at_scale("0", 6)?
        || settlement.settled_quantity() != Quantity::parse_at_scale("0.25", 8)?
        || settlement.realized_pnl() != QuoteAmount::from_str("-2.5")?
    {
        return Err(FixtureRunError::PositionSemanticMismatch);
    }
    if entries.iter().any(|(key, bytes)| {
        (key.namespace() == "position-episode.v1"
            || key.namespace() == "position-episode-effect-fact.v1")
            && bytes
                .windows(expected.interrupted_funding_event.as_str().len())
                .any(|window| window == expected.interrupted_funding_event.as_str().as_bytes())
    }) {
        return Err(FixtureRunError::PositionSemanticMismatch);
    }
    if namespace_counts(entries) != expected_namespace_counts() {
        return Err(FixtureRunError::PositionSemanticMismatch);
    }
    Ok(())
}

fn assert_interrupted_episode(
    entries: &BTreeMap<StateKey, Vec<u8>>,
    episode_id: &PositionEpisodeId,
    account: Address,
    close_event: &EventId,
    close_cause: EpisodeCloseCauseV1,
) -> Result<(), FixtureRunError> {
    let key = PositionEpisodeRecordV1::state_key(episode_id)
        .map_err(|_| FixtureRunError::PositionSemanticMismatch)?;
    let record = decode_at(entries, &key, PositionEpisodeRecordV1::decode_at)?;
    if record.account_id() != account
        || record.status() != EpisodeStatusV1::Interrupted
        || record.close_event_id() != Some(close_event)
        || record.close_cause() != Some(close_cause)
        || record.last_event_id() != close_event
    {
        return Err(FixtureRunError::PositionSemanticMismatch);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn assert_episode_snapshot(
    entries: &BTreeMap<StateKey, Vec<u8>>,
    episode_id: &PositionEpisodeId,
    account: Address,
    opening_event: &EventId,
    opening_ordinal: u8,
    opening_position: &str,
    completeness: EpisodeCompletenessV1,
    buy_quantity: &str,
    buy_notional: &str,
    sell_quantity: &str,
    sell_notional: &str,
    funding_paid: &str,
    status: EpisodeStatusV1,
    close_event: &EventId,
    close_cause: EpisodeCloseCauseV1,
) -> Result<(), FixtureRunError> {
    let key = PositionEpisodeRecordV1::state_key(episode_id)
        .map_err(|_| FixtureRunError::PositionSemanticMismatch)?;
    let record = decode_at(entries, &key, PositionEpisodeRecordV1::decode_at)?;
    if record.episode_id() != episode_id
        || record.account_id() != account
        || record.market_id().as_str() != MARKET
        || record.opening_anchor_event_id() != opening_event
        || record.opening_leg_ordinal() != opening_ordinal
        || record.opening_position() != PositionQuantity::from_str(opening_position)?
        || record.completeness() != completeness
        || record.buy_quantity() != Quantity::from_str(buy_quantity)?
        || record.buy_notional().to_string() != buy_notional
        || record.sell_quantity() != Quantity::from_str(sell_quantity)?
        || record.sell_notional().to_string() != sell_notional
        || record.funding_paid() != QuoteAmount::from_str(funding_paid)?
        || record.status() != status
        || record.close_event_id() != Some(close_event)
        || record.close_cause() != Some(close_cause)
        || record.last_event_id() != close_event
    {
        return Err(FixtureRunError::PositionSemanticMismatch);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn assert_episode_effect(
    entries: &BTreeMap<StateKey, Vec<u8>>,
    event_id: &EventId,
    account: Address,
    ordinal: u8,
    episode_id: &PositionEpisodeId,
    effect_kind: EpisodeEffectKindV1,
    buy_quantity: &str,
    buy_notional: &str,
    sell_quantity: &str,
    sell_notional: &str,
    funding_paid: &str,
    close_cause: Option<EpisodeCloseCauseV1>,
) -> Result<(), FixtureRunError> {
    let market = MarketId::new(MARKET)?;
    let key = PositionEpisodeEffectFactRecordV1::state_key(event_id, &account, &market, ordinal)
        .map_err(|_| FixtureRunError::PositionSemanticMismatch)?;
    let record = decode_at(entries, &key, PositionEpisodeEffectFactRecordV1::decode_at)?;
    if record.event_id() != event_id
        || record.account_id() != account
        || record.market_id() != &market
        || record.leg_ordinal() != ordinal
        || record.episode_id() != episode_id
        || record.effect_kind() != effect_kind
        || record.buy_quantity_delta() != Quantity::from_str(buy_quantity)?
        || record.buy_notional_delta().to_string() != buy_notional
        || record.sell_quantity_delta() != Quantity::from_str(sell_quantity)?
        || record.sell_notional_delta().to_string() != sell_notional
        || record.funding_paid_delta() != QuoteAmount::from_str(funding_paid)?
        || record.close_cause() != close_cause
    {
        return Err(FixtureRunError::PositionSemanticMismatch);
    }
    Ok(())
}

fn assert_quantity(
    entries: &BTreeMap<StateKey, Vec<u8>>,
    account: Address,
    market: &MarketId,
    known: Option<&str>,
    last_event: &EventId,
) -> Result<(), FixtureRunError> {
    let key = PositionQuantityCurrentRecordV1::state_key(&account, market)
        .map_err(|_| FixtureRunError::PositionSemanticMismatch)?;
    let record = decode_at(entries, &key, PositionQuantityCurrentRecordV1::decode_at)?;
    let expected_quantity = known
        .map(PositionQuantity::from_str)
        .transpose()
        .map_err(FixtureRunError::from)?;
    if record.account_id() != account
        || record.market_id() != market
        || record.known_quantity() != expected_quantity
        || record.last_event_id() != last_event
    {
        return Err(FixtureRunError::PositionSemanticMismatch);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn assert_position_effect(
    entries: &BTreeMap<StateKey, Vec<u8>>,
    trade_id: &str,
    role: TradeParticipantRoleV1,
    account: Address,
    start: &str,
    result: &str,
    transition: PositionAnchorTransitionV1,
) -> Result<(), FixtureRunError> {
    let trade_id = TradeId::new(trade_id)?;
    let key = PositionEffectFactRecordV1::state_key(&trade_id, role)
        .map_err(|_| FixtureRunError::PositionSemanticMismatch)?;
    let record = decode_at(entries, &key, PositionEffectFactRecordV1::decode_at)?;
    if record.trade_id() != &trade_id
        || record.account_id() != account
        || record.market_id().as_str() != MARKET
        || record.role() != role
        || record.anchor_transition() != transition
        || record.start_position() != PositionQuantity::from_str(start)?
        || record.result_position() != PositionQuantity::from_str(result)?
    {
        return Err(FixtureRunError::PositionSemanticMismatch);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn assert_open_episode(
    entries: &BTreeMap<StateKey, Vec<u8>>,
    episode_id: &PositionEpisodeId,
    account: Address,
    opening_event: &EventId,
    completeness: EpisodeCompletenessV1,
    last_event: &EventId,
    buy_quantity: &str,
    buy_notional: &str,
    sell_quantity: &str,
    sell_notional: &str,
    funding_paid: &str,
) -> Result<(), FixtureRunError> {
    let key = PositionEpisodeRecordV1::state_key(episode_id)
        .map_err(|_| FixtureRunError::PositionSemanticMismatch)?;
    let record = decode_at(entries, &key, PositionEpisodeRecordV1::decode_at)?;
    if record.episode_id() != episode_id
        || record.account_id() != account
        || record.market_id().as_str() != MARKET
        || record.opening_anchor_event_id() != opening_event
        || record.completeness() != completeness
        || record.last_event_id() != last_event
        || record.buy_quantity() != Quantity::from_str(buy_quantity)?
        || record.buy_notional().to_string() != buy_notional
        || record.sell_quantity() != Quantity::from_str(sell_quantity)?
        || record.sell_notional().to_string() != sell_notional
        || record.funding_paid() != QuoteAmount::from_str(funding_paid)?
        || record.status() != EpisodeStatusV1::Open
    {
        return Err(FixtureRunError::PositionSemanticMismatch);
    }
    Ok(())
}

fn assert_current_episode(
    entries: &BTreeMap<StateKey, Vec<u8>>,
    account: Address,
    market: &MarketId,
    episode_id: &PositionEpisodeId,
) -> Result<(), FixtureRunError> {
    let key = PositionEpisodeCurrentRecordV1::state_key(&account, market)
        .map_err(|_| FixtureRunError::PositionSemanticMismatch)?;
    let record = decode_at(entries, &key, PositionEpisodeCurrentRecordV1::decode_at)?;
    if record.account_id() != account
        || record.market_id() != market
        || record.episode_id() != Some(episode_id)
        || record.attribution_resolution() != EpisodeAttributionResolutionV1::Resolved
    {
        return Err(FixtureRunError::PositionSemanticMismatch);
    }
    Ok(())
}

fn decode_at<T>(
    entries: &BTreeMap<StateKey, Vec<u8>>,
    key: &StateKey,
    decode: impl FnOnce(&StateKey, &[u8]) -> Result<T, canonical_ledger::PositionStateError>,
) -> Result<T, FixtureRunError> {
    entries
        .get(key)
        .ok_or(FixtureRunError::PositionSemanticMismatch)
        .and_then(|bytes| decode(key, bytes).map_err(|_| FixtureRunError::PositionSemanticMismatch))
}

fn namespace_count(entries: &BTreeMap<StateKey, Vec<u8>>, namespace: &str) -> usize {
    entries
        .keys()
        .filter(|key| key.namespace() == namespace)
        .count()
}

fn namespace_counts(entries: &BTreeMap<StateKey, Vec<u8>>) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for key in entries.keys() {
        *counts.entry(key.namespace().to_owned()).or_insert(0) += 1;
    }
    counts
}

fn expected_namespace_counts() -> BTreeMap<String, usize> {
    BTreeMap::from([
        ("account-fact.v1".to_owned(), 3),
        ("account-quote-flow-current.v1".to_owned(), 1),
        ("asset-context-current.v1".to_owned(), 2),
        ("backstop-liquidation-fact.v1".to_owned(), 1),
        ("dex-current.v1".to_owned(), 1),
        ("liquidation-current.v1".to_owned(), 1),
        ("liquidation-fill-fact.v1".to_owned(), 1),
        ("liquidation-market-flow-current.v1".to_owned(), 1),
        ("liquidation-start-fact.v1".to_owned(), 1),
        ("market-current.v1".to_owned(), 1),
        ("market-fact.v1".to_owned(), 4),
        ("market-metadata-version.v1".to_owned(), 1),
        ("order-current.v1".to_owned(), 6),
        ("order-fact.v1".to_owned(), 6),
        ("order-transition.v1".to_owned(), 6),
        ("position-effect-fact.v1".to_owned(), 6),
        ("position-episode-current.v1".to_owned(), 2),
        ("position-episode-effect-fact.v1".to_owned(), 14),
        ("position-episode.v1".to_owned(), 7),
        ("position-quantity-current.v1".to_owned(), 2),
        ("position-settlement-fact.v1".to_owned(), 1),
        ("position-unresolved-cause-fact.v1".to_owned(), 2),
        ("reconciliation.v1".to_owned(), 3),
        ("trade-participant.v1".to_owned(), 6),
        ("trade-participant.v2".to_owned(), 6),
        ("trade-reconciliation.v2".to_owned(), 3),
        ("trade.v1".to_owned(), 3),
        ("trade.v2".to_owned(), 3),
    ])
}

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
    duplicate_trade_identity: RejectionReport,
    start_position_mismatch: RejectionReport,
    unsupported_schema: RejectionReport,
}
