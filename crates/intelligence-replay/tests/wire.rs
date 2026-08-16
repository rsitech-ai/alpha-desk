use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;

use canonical_events::{
    AssetContextUpdated, BlockEnvelope, CanonicalEventEnvelope, CanonicalEventInput,
    CommittedNodeV1MappingContext, ConfirmationClass, DepositCredited, DexCreated, EventPayload,
    MarketCreated, OrderAccepted, OrderFilled, SourceEvidence, SpotTransfer, SubaccountTransfer,
    TradeMatched, TradeParticipantRoleV1, TradeParticipantV1, VaultDeposit,
};
use canonical_ledger::{PositionEpisodeRecordV1, derive_position_episode_id};
use domain_types::{
    AccountId, Address, AssetId, BlockHeight, ChainId, ClosedInterval, DexId, Direction, EntityId,
    EventId, FeatureSetVersion, Horizon, KnownTime, MarketId, OrderId, OrderSide, PositionQuantity,
    Price, ProbabilityPpm, ProtocolTime, Quantity, QuoteAmount, ScenarioId, SignalId, SourceId,
    TradeId, TransactionId, UsdAmount, VaultId,
};
use feature_core::{FeatureValue, HealthAssessment, HealthState, MissingReason};
use hl_protocol::node::v1::{NodeStreamKind, parse_node_record};
use intelligence_replay::{
    IntelligenceReplayError, IntelligenceReplayReport, MaterializeRequest, QualificationClaim,
    admit_committed_confirmation, fold_withhold_reason, holding_time_from_closed_episodes,
    holding_time_from_replay_blocks, materialize_committed_node, materialize_synthetic_replay,
    qualification_what_for_withhold, refuse_leaked_withheld_emission, slippage_from_replay_blocks,
};
use market_intelligence::{
    CrowdingPosition, FragilityScenario, MarketError, MarketFeatureSnapshot,
    crowding_components_from_snapshot, market_feature_key, simulate_fragility_from_snapshot,
};
use signal_core::{
    ProofWithholdReason, Signal, SignalConfirmationClass, SignalError, SignalLifecycleState,
    SignalType, proof_withhold_reason, suppress_proof_withhold,
};
use wallet_intelligence::{
    HoldingTimeDistribution, IntelligenceError, ObservedHoldInterval, holding_time_distribution,
    slippage_from_order_events,
};

const BUYER: Address = Address::from_bytes([0x11; 20]);
const SELLER: Address = Address::from_bytes([0x22; 20]);
const OPERATOR: Address = Address::from_bytes([0x55; 20]);
const START_HEIGHT: u64 = 1;

fn request() -> MaterializeRequest {
    MaterializeRequest::synthetic_unassessed(FeatureSetVersion::new("synthetic-replay-v1").unwrap())
        .unwrap()
}

fn time(height: u64) -> ProtocolTime {
    ProtocolTime::from_unix_micros(
        i64::try_from(height)
            .unwrap()
            .checked_mul(1_000_000)
            .unwrap(),
    )
    .unwrap()
}

fn known(height: u64) -> KnownTime {
    KnownTime::from_unix_micros(time(height).unix_micros()).unwrap()
}

fn chain() -> ChainId {
    ChainId::new("mainnet").unwrap()
}

fn market() -> MarketId {
    MarketId::new("perp:BTC").unwrap()
}

fn account_id(address: Address) -> AccountId {
    AccountId::new(address.to_api_string()).unwrap()
}

fn event(
    height: u64,
    index: u32,
    payload: EventPayload,
    market_ids: Vec<MarketId>,
    account_ids: Vec<Address>,
) -> CanonicalEventEnvelope {
    confirmed_event(
        height,
        index,
        payload,
        market_ids,
        account_ids,
        ConfirmationClass::CommittedPrimary,
    )
}

fn confirmed_event(
    height: u64,
    index: u32,
    payload: EventPayload,
    market_ids: Vec<MarketId>,
    account_ids: Vec<Address>,
    confirmation: ConfirmationClass,
) -> CanonicalEventEnvelope {
    let payload_hash = *blake3::hash(&payload.encode_to_vec().unwrap()).as_bytes();
    let block_time = time(height);
    CanonicalEventEnvelope::from_input(CanonicalEventInput {
        schema_version: "1.0.0".to_owned(),
        chain_id: chain(),
        block_height: BlockHeight::new(height),
        block_time,
        transaction_id: TransactionId::new(format!("intelligence-replay-{height}-{index}"))
            .unwrap(),
        transaction_index: index,
        canonical_event_index: 0,
        market_ids,
        account_ids,
        source_evidence: vec![
            SourceEvidence::try_new_indexed(
                SourceId::new("intelligence-replay-synthetic").unwrap(),
                "v1",
                height.to_string(),
                payload_hash,
                index,
            )
            .unwrap(),
        ],
        confirmation_class: confirmation,
        observed_at: known(height),
        ingested_at: known(height),
        canonicalized_at: known(height),
        parser_version: "intelligence-replay-synthetic-v1".to_owned(),
        payload,
    })
    .unwrap()
}

fn block(height: u64, events: Vec<CanonicalEventEnvelope>) -> BlockEnvelope {
    BlockEnvelope::try_new(
        chain(),
        BlockHeight::new(height),
        time(height),
        ConfirmationClass::CommittedPrimary,
        events,
        BTreeMap::from([(
            SourceId::new("intelligence-replay-synthetic").unwrap(),
            [height as u8; 32],
        )]),
    )
    .unwrap()
}

fn market_prerequisite_block() -> BlockEnvelope {
    let btc = AssetId::new("BTC").unwrap();
    let usdc = AssetId::new("USDC").unwrap();
    let market = market();
    block(
        START_HEIGHT,
        vec![
            event(
                START_HEIGHT,
                0,
                EventPayload::DexCreated(DexCreated {
                    dex_id: DexId::new("intelligence-replay").unwrap(),
                    name: "Intelligence replay".to_owned(),
                    operator_account_id: OPERATOR,
                }),
                vec![],
                vec![OPERATOR],
            ),
            event(
                START_HEIGHT,
                1,
                EventPayload::AssetContextUpdated(AssetContextUpdated {
                    asset_id: btc.clone(),
                    context_version: "btc-v1".to_owned(),
                    context_hash: [1; 32],
                }),
                vec![],
                vec![],
            ),
            event(
                START_HEIGHT,
                2,
                EventPayload::AssetContextUpdated(AssetContextUpdated {
                    asset_id: usdc.clone(),
                    context_version: "usdc-v1".to_owned(),
                    context_hash: [2; 32],
                }),
                vec![],
                vec![],
            ),
            event(
                START_HEIGHT,
                3,
                EventPayload::MarketCreated(MarketCreated {
                    market_id: market.clone(),
                    dex_id: DexId::new("intelligence-replay").unwrap(),
                    base_asset_id: btc,
                    quote_asset_id: usdc,
                    tick_size: Price::parse_at_scale("0.1", 6).unwrap(),
                    lot_size: Quantity::parse_at_scale("0.001", 8).unwrap(),
                }),
                vec![market],
                vec![],
            ),
        ],
    )
}

fn account_relation_block() -> BlockEnvelope {
    let height = START_HEIGHT + 1;
    let usdc = AssetId::new("USDC").unwrap();
    block(
        height,
        vec![
            event(
                height,
                0,
                EventPayload::DepositCredited(DepositCredited {
                    account_id: BUYER,
                    asset_id: usdc.clone(),
                    amount: Quantity::from_str("10").unwrap(),
                    deposit_reference: "deposit".to_owned(),
                }),
                vec![],
                vec![BUYER],
            ),
            event(
                height,
                1,
                EventPayload::SubaccountTransfer(SubaccountTransfer {
                    master_account_id: BUYER,
                    from_account_id: BUYER,
                    to_account_id: SELLER,
                    asset_id: usdc,
                    amount: Quantity::from_str("1.5").unwrap(),
                }),
                vec![],
                vec![BUYER, BUYER, SELLER],
            ),
            event(
                height,
                2,
                EventPayload::VaultDeposit(VaultDeposit {
                    vault_id: VaultId::new("intelligence-replay-vault").unwrap(),
                    account_id: BUYER,
                    amount: QuoteAmount::from_str("4").unwrap(),
                    shares_issued: Quantity::from_str("4").unwrap(),
                }),
                vec![],
                vec![BUYER],
            ),
        ],
    )
}

fn synthetic_blocks() -> Vec<BlockEnvelope> {
    vec![market_prerequisite_block(), account_relation_block()]
}

fn order_id() -> OrderId {
    OrderId::new("intelligence-replay-order-1").unwrap()
}

fn accepted_payload() -> EventPayload {
    EventPayload::OrderAccepted(OrderAccepted {
        order_id: order_id(),
        account_id: BUYER,
        market_id: market(),
        side: OrderSide::Buy,
        limit_price: Price::parse_at_scale("100", 6).unwrap(),
        quantity: Quantity::parse_at_scale("1", 8).unwrap(),
    })
}

fn filled_payload(trade: &str, fill_price: &str) -> EventPayload {
    EventPayload::OrderFilled(OrderFilled {
        order_id: order_id(),
        trade_id: TradeId::new(trade).unwrap(),
        fill_price: Price::parse_at_scale(fill_price, 6).unwrap(),
        fill_quantity: Quantity::parse_at_scale("1", 8).unwrap(),
    })
}

fn accept_and_fill_blocks() -> Vec<BlockEnvelope> {
    let height = START_HEIGHT + 2;
    let mut blocks = synthetic_blocks();
    blocks.push(block(
        height,
        vec![
            event(height, 0, accepted_payload(), vec![market()], vec![BUYER]),
            event(
                height,
                1,
                filled_payload("intelligence-replay-trade-1", "101"),
                vec![market()],
                vec![BUYER],
            ),
        ],
    ));
    blocks
}

fn fill_without_limit_blocks() -> Vec<BlockEnvelope> {
    let height = START_HEIGHT + 2;
    let mut blocks = synthetic_blocks();
    blocks.push(block(
        height,
        vec![event(
            height,
            0,
            filled_payload("intelligence-replay-trade-orphan", "101"),
            vec![market()],
            vec![BUYER],
        )],
    ));
    blocks
}

fn matched_trade(
    height: u64,
    index: u32,
    trade_id: &str,
    buyer: Address,
    buyer_start: &str,
    seller: Address,
    seller_start: &str,
) -> CanonicalEventEnvelope {
    event(
        height,
        index,
        EventPayload::TradeMatched(TradeMatched {
            trade_id: Some(TradeId::new(trade_id).unwrap()),
            market_id: Some(market()),
            maker_order_id: None,
            taker_order_id: None,
            price: Price::from_str("100").unwrap(),
            quantity: Quantity::from_str("1").unwrap(),
            deterministic_seed: height,
            participants: Some(Box::new([
                TradeParticipantV1 {
                    role: TradeParticipantRoleV1::Buyer,
                    account_id: buyer,
                    start_position: PositionQuantity::from_str(buyer_start).unwrap(),
                    order_id: OrderId::new(format!("buyer-order-{trade_id}")).unwrap(),
                    twap_id: None,
                    client_order_id: None,
                },
                TradeParticipantV1 {
                    role: TradeParticipantRoleV1::Seller,
                    account_id: seller,
                    start_position: PositionQuantity::from_str(seller_start).unwrap(),
                    order_id: OrderId::new(format!("seller-order-{trade_id}")).unwrap(),
                    twap_id: None,
                    client_order_id: None,
                },
            ])),
        }),
        vec![market()],
        vec![buyer, seller],
    )
}

fn open_and_close_blocks() -> Vec<BlockEnvelope> {
    let mut blocks = synthetic_blocks();
    let open_height = START_HEIGHT + 2;
    let close_height = START_HEIGHT + 3;
    blocks.push(block(
        open_height,
        vec![matched_trade(
            open_height,
            0,
            "trd-open",
            BUYER,
            "0",
            SELLER,
            "0",
        )],
    ));
    blocks.push(block(
        close_height,
        vec![matched_trade(
            close_height,
            0,
            "trd-close",
            SELLER,
            "-1",
            BUYER,
            "1",
        )],
    ));
    blocks
}

fn open_only_blocks() -> Vec<BlockEnvelope> {
    let mut blocks = synthetic_blocks();
    let open_height = START_HEIGHT + 2;
    blocks.push(block(
        open_height,
        vec![matched_trade(
            open_height,
            0,
            "trd-open-only",
            BUYER,
            "0",
            SELLER,
            "0",
        )],
    ));
    blocks
}

fn participant_free_trade_blocks() -> Vec<BlockEnvelope> {
    let mut blocks = synthetic_blocks();
    let height = START_HEIGHT + 2;
    blocks.push(block(
        height,
        vec![event(
            height,
            0,
            EventPayload::TradeMatched(TradeMatched {
                trade_id: Some(TradeId::new("trd-legacy").unwrap()),
                market_id: Some(market()),
                maker_order_id: None,
                taker_order_id: None,
                price: Price::from_str("100").unwrap(),
                quantity: Quantity::from_str("1").unwrap(),
                deterministic_seed: height,
                participants: None,
            }),
            vec![market()],
            vec![BUYER, SELLER],
        )],
    ));
    blocks
}

fn trade_named<'a>(
    events: &'a [CanonicalEventEnvelope],
    trade_id: &str,
) -> &'a CanonicalEventEnvelope {
    events
        .iter()
        .find(|event| {
            matches!(
                event.payload(),
                EventPayload::TradeMatched(trade)
                    if trade.trade_id.as_ref().is_some_and(|id| id.as_str() == trade_id)
            )
        })
        .unwrap_or_else(|| panic!("missing TradeMatched {trade_id}"))
}

fn expected_closed_holding_time(blocks: &[BlockEnvelope]) -> HoldingTimeDistribution {
    let events = replay_events(blocks);
    let open = trade_named(&events, "trd-open");
    let close = trade_named(&events, "trd-close");
    let interval = ObservedHoldInterval::try_new(open.block_time(), close.block_time()).unwrap();
    holding_time_distribution(&[interval.clone(), interval]).unwrap()
}

fn zero_holding_time_distribution() -> HoldingTimeDistribution {
    HoldingTimeDistribution {
        sample_count: 0,
        min_micros: 0,
        max_micros: 0,
        median_micros: 0,
        total_micros: 0,
    }
}

fn decoded_closed_episode() -> PositionEpisodeRecordV1 {
    let opening = EventId::new("evt-open").unwrap();
    let episode_id = derive_position_episode_id(&BUYER, &market(), &opening, 0).unwrap();
    let bytes = format!(
        "{{\"schema\":\"hyperliquid-alpha-desk/position-episode/v1\",\"episode_id\":\"{}\",\"account_id\":\"{}\",\"market_id\":\"{}\",\"opening_anchor_event_id\":\"evt-open\",\"opening_leg_ordinal\":0,\"opening_position\":\"0\",\"close_event_id\":\"evt-close\",\"close_cause\":\"trade_flat\",\"completeness\":\"complete_from_flat\",\"buy_quantity\":\"1\",\"buy_notional\":\"100\",\"sell_quantity\":\"1\",\"sell_notional\":\"100\",\"funding_paid\":\"0\",\"funding_received\":\"0\",\"status\":\"closed\",\"last_event_id\":\"evt-close\",\"last_block_height\":4}}",
        episode_id.as_str(),
        BUYER.to_api_string(),
        market().as_str(),
    );
    PositionEpisodeRecordV1::decode(bytes.as_bytes()).unwrap()
}

fn decoded_open_episode() -> PositionEpisodeRecordV1 {
    let opening = EventId::new("evt-open").unwrap();
    let episode_id = derive_position_episode_id(&BUYER, &market(), &opening, 0).unwrap();
    let bytes = format!(
        "{{\"schema\":\"hyperliquid-alpha-desk/position-episode/v1\",\"episode_id\":\"{}\",\"account_id\":\"{}\",\"market_id\":\"{}\",\"opening_anchor_event_id\":\"evt-open\",\"opening_leg_ordinal\":0,\"opening_position\":\"0\",\"close_event_id\":null,\"close_cause\":null,\"completeness\":\"complete_from_flat\",\"buy_quantity\":\"1\",\"buy_notional\":\"100\",\"sell_quantity\":\"0\",\"sell_notional\":\"0\",\"funding_paid\":\"0\",\"funding_received\":\"0\",\"status\":\"open\",\"last_event_id\":\"evt-open\",\"last_block_height\":4}}",
        episode_id.as_str(),
        BUYER.to_api_string(),
        market().as_str(),
    );
    PositionEpisodeRecordV1::decode(bytes.as_bytes()).unwrap()
}

fn replay_events(blocks: &[BlockEnvelope]) -> Vec<CanonicalEventEnvelope> {
    blocks
        .iter()
        .flat_map(BlockEnvelope::events)
        .cloned()
        .collect()
}

fn empty_confirmed_block(confirmation: ConfirmationClass) -> BlockEnvelope {
    BlockEnvelope::try_new(
        chain(),
        BlockHeight::new(START_HEIGHT),
        time(START_HEIGHT),
        confirmation,
        Vec::new(),
        BTreeMap::from([(
            SourceId::new("intelligence-replay-synthetic").unwrap(),
            [START_HEIGHT as u8; 32],
        )]),
    )
    .unwrap()
}

fn deposit_confirmed_block(confirmation: ConfirmationClass) -> BlockEnvelope {
    BlockEnvelope::try_new(
        chain(),
        BlockHeight::new(START_HEIGHT),
        time(START_HEIGHT),
        confirmation,
        vec![confirmed_event(
            START_HEIGHT,
            0,
            EventPayload::DepositCredited(DepositCredited {
                account_id: BUYER,
                asset_id: usdc(),
                amount: Quantity::from_str("10").unwrap(),
                deposit_reference: "deposit".to_owned(),
            }),
            vec![],
            vec![BUYER],
            confirmation,
        )],
        BTreeMap::from([(
            SourceId::new("intelligence-replay-synthetic").unwrap(),
            [START_HEIGHT as u8; 32],
        )]),
    )
    .unwrap()
}

fn usdc() -> AssetId {
    AssetId::new("USDC").unwrap()
}

fn deposit_event(height: u64, index: u32, account: Address) -> CanonicalEventEnvelope {
    event(
        height,
        index,
        EventPayload::DepositCredited(DepositCredited {
            account_id: account,
            asset_id: usdc(),
            amount: Quantity::from_str("10").unwrap(),
            deposit_reference: format!("deposit-{index}"),
        }),
        vec![],
        vec![account],
    )
}

fn independent_deposit_blocks() -> Vec<BlockEnvelope> {
    let height = START_HEIGHT + 1;
    vec![
        market_prerequisite_block(),
        block(
            height,
            vec![
                deposit_event(height, 0, BUYER),
                deposit_event(height, 1, SELLER),
            ],
        ),
    ]
}

fn spot_transfer_blocks() -> Vec<BlockEnvelope> {
    let height = START_HEIGHT + 1;
    vec![
        market_prerequisite_block(),
        block(
            height,
            vec![
                deposit_event(height, 0, BUYER),
                deposit_event(height, 1, SELLER),
                event(
                    height,
                    2,
                    EventPayload::SpotTransfer(SpotTransfer {
                        from_account_id: BUYER,
                        to_account_id: SELLER,
                        asset_id: usdc(),
                        amount: Quantity::from_str("1.5").unwrap(),
                    }),
                    vec![],
                    vec![BUYER, SELLER],
                ),
            ],
        ),
    ]
}

fn shared_vault_depositor_blocks() -> Vec<BlockEnvelope> {
    let height = START_HEIGHT + 1;
    let vault = VaultId::new("intelligence-replay-vault").unwrap();
    vec![
        market_prerequisite_block(),
        block(
            height,
            vec![
                deposit_event(height, 0, BUYER),
                deposit_event(height, 1, SELLER),
                event(
                    height,
                    2,
                    EventPayload::VaultDeposit(VaultDeposit {
                        vault_id: vault.clone(),
                        account_id: BUYER,
                        amount: QuoteAmount::from_str("4").unwrap(),
                        shares_issued: Quantity::from_str("4").unwrap(),
                    }),
                    vec![],
                    vec![BUYER],
                ),
                event(
                    height,
                    3,
                    EventPayload::VaultDeposit(VaultDeposit {
                        vault_id: vault,
                        account_id: SELLER,
                        amount: QuoteAmount::from_str("3").unwrap(),
                        shares_issued: Quantity::from_str("3").unwrap(),
                    }),
                    vec![],
                    vec![SELLER],
                ),
            ],
        ),
    ]
}

fn assert_synthetic_unassessed(report: &IntelligenceReplayReport) {
    assert_eq!(report.source_qualification, "synthetic_unassessed");
    assert!(!report.stage_3_pass);
    assert!(!report.live_qualified);
    assert!(!report.alpha_qualified);
    assert!(!report.fills_invented);
    assert!(!report.replica_cmds_used);
    assert_eq!(report.live_signal_count, 0);
    assert_eq!(report.crowding_emitted, 0);
    assert_eq!(report.fragility_emitted, 0);
    assert!(!report.marks_invented);
    assert_eq!(
        report.signal_confirmation,
        SignalConfirmationClass::SyntheticUnqualified
    );
}

fn committed_context(height: u64) -> CommittedNodeV1MappingContext {
    CommittedNodeV1MappingContext {
        chain_id: ChainId::new("hyperliquid-mainnet").unwrap(),
        source_id: SourceId::new("primary-node").unwrap(),
        source_version: "intelligence-replay-test".to_owned(),
        source_offset: height.to_string(),
        expected_height: BlockHeight::new(height),
        confirmation_class: ConfirmationClass::CommittedPrimary,
    }
}

fn parse_committed(payload: serde_json::Value) -> hl_protocol::node::v1::NodeRecordV1 {
    parse_node_record(
        NodeStreamKind::TransactionBlocks,
        bytes::Bytes::from(serde_json::to_vec(&payload).unwrap()),
    )
    .unwrap()
}

#[test]
fn synthetic_replay_wires_reconstructed_state_to_pit_features() {
    let blocks = synthetic_blocks();
    let first = materialize_synthetic_replay(&blocks, &request()).unwrap();
    let second = materialize_synthetic_replay(&blocks, &request()).unwrap();

    assert_eq!(first.state_hash, second.state_hash);
    assert_synthetic_unassessed(&first);
    assert!(first.wallet_performance_withheld);
    assert!(first.slippage.is_none());
    assert!(first.holding_time.is_none());
    assert!(matches!(
        first.require_wallet_performance(),
        Err(IntelligenceReplayError::MissingState)
    ));
    assert!(matches!(
        first.require_live_signal(),
        Err(IntelligenceReplayError::QualificationClaim {
            what: "live_signal"
        })
    ));

    let buyer = account_id(BUYER);
    let early = first.require_asof_account(&buyer, time(1), known(1));
    assert!(matches!(early, Err(IntelligenceReplayError::MissingState)));
    let later = first
        .require_asof_account(&buyer, time(2), known(2))
        .unwrap();
    assert_eq!(later.data_health, HealthState::Amber);
    assert_eq!(
        later
            .values
            .get(&feature_core::FeatureKey::try_new("wallet", "fills", 1).unwrap()),
        Some(&FeatureValue::Missing(MissingReason::NotObserved))
    );
    assert_eq!(
        later
            .values
            .get(&feature_core::FeatureKey::try_new("wallet", "equity_usd", 1).unwrap()),
        Some(&FeatureValue::Missing(MissingReason::NotObserved))
    );
    assert_eq!(
        later
            .values
            .get(&feature_core::FeatureKey::try_new("wallet", "reconstructed", 1).unwrap()),
        Some(&FeatureValue::Boolean(true))
    );

    assert!(first.entity_graph.links_as_of(time(1), known(1)).is_empty());
    let links = first.entity_graph.links_as_of(time(2), known(2));
    assert_eq!(links.len(), 2);
    assert!(
        links
            .iter()
            .any(|link| link.kind == entity_graph::LinkKind::ProtocolSubaccount)
    );
    assert!(
        links
            .iter()
            .any(|link| link.kind == entity_graph::LinkKind::ProtocolVaultMembership)
    );
    let groups = first
        .entity_graph
        .known_administrative_groups(time(2), known(2))
        .unwrap();
    assert_eq!(groups.len(), 1);
    let members: BTreeSet<_> = groups.into_iter().next().unwrap().into_iter().collect();
    assert_eq!(
        members,
        BTreeSet::from([
            entity_graph::GraphNodeId::Account(account_id(BUYER)),
            entity_graph::GraphNodeId::Account(account_id(SELLER)),
        ])
    );

    assert_eq!(first.market_snapshots.len(), 1);
    assert_eq!(
        first.market_snapshots[0].health.reason_code,
        "synthetic_unassessed"
    );
    assert_eq!(
        first.market_snapshots[0]
            .values
            .get(&market_intelligence::market_feature_key("fills").unwrap()),
        Some(&FeatureValue::Missing(MissingReason::NotObserved))
    );
    assert_eq!(
        first.market_snapshots[0]
            .values
            .get(&market_intelligence::market_feature_key("book").unwrap()),
        Some(&FeatureValue::Missing(MissingReason::NotObserved))
    );
    assert_eq!(
        first.market_snapshots[0]
            .values
            .get(&market_intelligence::market_feature_key("inventory").unwrap()),
        Some(&FeatureValue::Missing(MissingReason::NotObserved))
    );
}

#[test]
fn observed_accept_and_fill_emits_slippage_from_in_force_limit_join() {
    let blocks = accept_and_fill_blocks();
    let report = materialize_synthetic_replay(&blocks, &request()).unwrap();
    assert_synthetic_unassessed(&report);
    assert!(report.wallet_performance_withheld);
    assert!(!report.fills_invented);
    assert!(!report.marks_invented);
    assert!(
        report.holding_time.is_none(),
        "OrderFilled timestamps are not holding intervals"
    );
    assert_eq!(
        report
            .require_asof_account(&account_id(BUYER), time(3), known(3))
            .unwrap()
            .values
            .get(&feature_core::FeatureKey::try_new("wallet", "equity_usd", 1).unwrap()),
        Some(&FeatureValue::Missing(MissingReason::NotObserved))
    );
    assert_eq!(
        report
            .require_asof_account(&account_id(BUYER), time(3), known(3))
            .unwrap()
            .values
            .get(&feature_core::FeatureKey::try_new("wallet", "fills", 1).unwrap()),
        Some(&FeatureValue::Missing(MissingReason::NotObserved))
    );

    let expected = slippage_from_order_events(&replay_events(&blocks))
        .unwrap()
        .unwrap();
    assert_eq!(
        report.slippage.as_ref(),
        Some(&expected),
        "replay must attach the join result, not a mid or invented fill"
    );
    assert_eq!(
        slippage_from_replay_blocks(&blocks).unwrap().as_ref(),
        Some(&expected)
    );
    assert_eq!(expected.observed_fill_count, 1);
    assert_eq!(expected.withheld_missing_reference_count, 0);
    assert_eq!(expected.notional_weighted_slippage_bps.raw(), 10_000);
    assert_ne!(expected.notional_weighted_slippage_bps.raw(), 0);
}

#[test]
fn missing_in_force_limit_withholds_slippage() {
    let fill_only = fill_without_limit_blocks();
    assert!(
        slippage_from_order_events(&replay_events(&fill_only))
            .unwrap()
            .is_none(),
        "fill without an in-force limit must withhold, not invent a mid"
    );
    assert!(slippage_from_replay_blocks(&fill_only).unwrap().is_none());

    let error = materialize_synthetic_replay(&fill_only, &request()).unwrap_err();
    assert_eq!(error.reason_code(), "ledger.reducer_failed");
}

#[test]
fn inverted_replay_block_times_fail_closed_with_existing_order_event_error() {
    let blocks = accept_and_fill_blocks();
    let reversed: Vec<_> = blocks.iter().rev().cloned().collect();
    let error = slippage_from_replay_blocks(&reversed).unwrap_err();
    assert!(matches!(
        error,
        IntelligenceReplayError::Wallet(IntelligenceError::Malformed {
            what: "order_event",
            reason: "inverted times"
        })
    ));
}

#[test]
fn closed_episodes_emit_holding_time_from_observed_stream_event_times() {
    let blocks = open_and_close_blocks();
    let report = materialize_synthetic_replay(&blocks, &request()).unwrap();
    assert_synthetic_unassessed(&report);
    assert!(report.wallet_performance_withheld);
    assert_eq!(
        report
            .require_asof_account(&account_id(BUYER), time(4), known(4))
            .unwrap()
            .values
            .get(&feature_core::FeatureKey::try_new("wallet", "equity_usd", 1).unwrap()),
        Some(&FeatureValue::Missing(MissingReason::NotObserved))
    );
    assert_eq!(
        report
            .require_asof_account(&account_id(BUYER), time(4), known(4))
            .unwrap()
            .values
            .get(&feature_core::FeatureKey::try_new("wallet", "fills", 1).unwrap()),
        Some(&FeatureValue::Missing(MissingReason::NotObserved))
    );
    assert!(report.slippage.is_none());

    let expected = expected_closed_holding_time(&blocks);
    assert_eq!(report.holding_time.as_ref(), Some(&expected));
    assert_eq!(
        holding_time_from_replay_blocks(&blocks, &[]).unwrap(),
        None,
        "empty episode sample must withhold, not invent intervals from TradeMatched times"
    );
    assert_eq!(expected.sample_count, 2);
    assert_eq!(expected.min_micros, 1_000_000);
    assert_eq!(expected.max_micros, 1_000_000);
    assert_eq!(expected.median_micros, 1_000_000);
    assert_eq!(expected.total_micros, 2_000_000);
    assert_ne!(expected.min_micros, 0);
}

#[test]
fn open_episodes_withhold_holding_time_instead_of_inventing_a_close() {
    let blocks = open_only_blocks();
    let report = materialize_synthetic_replay(&blocks, &request()).unwrap();
    assert_synthetic_unassessed(&report);
    assert!(report.wallet_performance_withheld);
    assert!(report.holding_time.is_none());
    assert_ne!(report.holding_time, Some(zero_holding_time_distribution()));

    let open = decoded_open_episode();
    let mut times = BTreeMap::new();
    times.insert(open.opening_anchor_event_id().clone(), time(3));
    times.insert(EventId::new("watermark-is-not-a-close").unwrap(), time(4));
    assert!(
        holding_time_from_closed_episodes(std::slice::from_ref(&open), &times)
            .unwrap()
            .is_none()
    );
    assert!(
        holding_time_from_replay_blocks(&blocks, &[open])
            .unwrap()
            .is_none()
    );
}

#[test]
fn closed_episode_missing_open_or_close_event_on_stream_withholds() {
    let closed = decoded_closed_episode();
    let open_id = closed.opening_anchor_event_id().clone();
    let close_id = closed.close_event_id().unwrap().clone();
    let mut times = BTreeMap::new();
    times.insert(open_id.clone(), time(3));
    assert!(
        holding_time_from_closed_episodes(std::slice::from_ref(&closed), &times)
            .unwrap()
            .is_none(),
        "missing close event must withhold, not use last_block_height"
    );
    times.remove(&open_id);
    times.insert(close_id, time(4));
    assert!(
        holding_time_from_closed_episodes(std::slice::from_ref(&closed), &times)
            .unwrap()
            .is_none(),
        "missing open event must withhold"
    );
    times.insert(open_id, time(3));
    let expected =
        holding_time_distribution(&[ObservedHoldInterval::try_new(time(3), time(4)).unwrap()])
            .unwrap();
    assert_eq!(
        holding_time_from_closed_episodes(&[closed], &times)
            .unwrap()
            .as_ref(),
        Some(&expected)
    );
}

#[test]
fn empty_holding_time_sample_withholds_instead_of_emitting_a_zero_distribution() {
    assert!(matches!(
        holding_time_distribution(&[]).unwrap_err(),
        IntelligenceError::InsufficientHistory {
            what: "holding_time"
        }
    ));
    let empty = holding_time_from_closed_episodes(&[], &BTreeMap::new()).unwrap();
    assert!(empty.is_none());
    assert_ne!(empty, Some(zero_holding_time_distribution()));

    let report = materialize_synthetic_replay(&synthetic_blocks(), &request()).unwrap();
    assert!(report.holding_time.is_none());
    assert_ne!(report.holding_time, Some(zero_holding_time_distribution()));
}

#[test]
fn participant_free_trade_matched_does_not_invent_a_holding_interval() {
    let blocks = participant_free_trade_blocks();
    let report = materialize_synthetic_replay(&blocks, &request()).unwrap();
    assert_synthetic_unassessed(&report);
    assert!(report.wallet_performance_withheld);
    assert!(report.holding_time.is_none());
    assert_ne!(report.holding_time, Some(zero_holding_time_distribution()));
    assert!(
        holding_time_from_replay_blocks(&blocks, &[])
            .unwrap()
            .is_none(),
        "participant-free TradeMatched must not mint an episode or interval"
    );
}

fn assert_accounts_unmerged(report: &IntelligenceReplayReport) {
    assert_synthetic_unassessed(report);
    assert!(
        report
            .entity_graph
            .links_as_of(time(1), known(1))
            .is_empty()
    );
    let groups = report
        .entity_graph
        .known_administrative_groups(time(2), known(2))
        .unwrap();
    assert!(
        groups.is_empty(),
        "distinct wallets must not collapse without an explicit protocol identity link: {groups:?}"
    );
}

#[test]
fn distinct_deposit_addresses_do_not_merge() {
    let report = materialize_synthetic_replay(&independent_deposit_blocks(), &request()).unwrap();
    assert!(
        report
            .entity_graph
            .links_as_of(time(2), known(2))
            .is_empty()
    );
    assert_accounts_unmerged(&report);
}

#[test]
fn spot_transfer_without_protocol_subaccount_does_not_merge() {
    let report = materialize_synthetic_replay(&spot_transfer_blocks(), &request()).unwrap();
    assert!(
        report
            .entity_graph
            .links_as_of(time(2), known(2))
            .is_empty()
    );
    assert_accounts_unmerged(&report);
}

#[test]
fn shared_vault_depositors_do_not_merge() {
    let report =
        materialize_synthetic_replay(&shared_vault_depositor_blocks(), &request()).unwrap();
    let links = report.entity_graph.links_as_of(time(2), known(2));
    assert_eq!(links.len(), 2);
    assert!(
        links
            .iter()
            .all(|link| link.kind == entity_graph::LinkKind::ProtocolVaultMembership)
    );
    assert_accounts_unmerged(&report);
}

#[test]
fn missing_reconstructed_state_fails_closed() {
    let error = materialize_synthetic_replay(&[], &request()).unwrap_err();
    assert!(matches!(error, IntelligenceReplayError::MissingState));

    let empty = block(START_HEIGHT, Vec::new());
    let error = materialize_synthetic_replay(&[empty], &request()).unwrap_err();
    assert!(matches!(error, IntelligenceReplayError::MissingState));
}

#[test]
fn admit_committed_confirmation_covers_every_class() {
    for class in [
        ConfirmationClass::ProvisionalSource,
        ConfirmationClass::CommittedPrimary,
        ConfirmationClass::CommittedIndependent,
        ConfirmationClass::ReconciledSnapshot,
        ConfirmationClass::Corrected,
        ConfirmationClass::Expired,
    ] {
        let admitted = admit_committed_confirmation(class);
        match class {
            ConfirmationClass::CommittedPrimary | ConfirmationClass::CommittedIndependent => {
                admitted.expect("committed lanes are admitted");
            }
            ConfirmationClass::ProvisionalSource
            | ConfirmationClass::ReconciledSnapshot
            | ConfirmationClass::Corrected
            | ConfirmationClass::Expired => {
                let error = admitted.expect_err("non-committed lanes fail closed");
                assert_eq!(error.reason_code(), "ledger.non_committed_block");
            }
        }

        let error =
            materialize_synthetic_replay(&[empty_confirmed_block(class)], &request()).unwrap_err();
        match class {
            ConfirmationClass::CommittedPrimary | ConfirmationClass::CommittedIndependent => {
                assert!(
                    matches!(error, IntelligenceReplayError::MissingState),
                    "{class:?} empty committed block must stay missing_state, not non_committed"
                );
            }
            ConfirmationClass::ProvisionalSource
            | ConfirmationClass::ReconciledSnapshot
            | ConfirmationClass::Corrected
            | ConfirmationClass::Expired => {
                assert_eq!(
                    error.reason_code(),
                    "ledger.non_committed_block",
                    "{class:?} must fail closed before missing intelligence"
                );
            }
        }
    }
}

#[test]
fn provisional_account_events_cannot_materialize_intelligence() {
    let error = materialize_synthetic_replay(
        &[deposit_confirmed_block(
            ConfirmationClass::ProvisionalSource,
        )],
        &request(),
    )
    .unwrap_err();
    assert_eq!(error.reason_code(), "ledger.non_committed_block");
}

#[test]
fn reconciled_corrected_and_expired_account_events_fail_closed() {
    for class in [
        ConfirmationClass::ReconciledSnapshot,
        ConfirmationClass::Corrected,
        ConfirmationClass::Expired,
    ] {
        let error = materialize_synthetic_replay(&[deposit_confirmed_block(class)], &request())
            .unwrap_err();
        assert_eq!(
            error.reason_code(),
            "ledger.non_committed_block",
            "{class:?} account events must not materialize intelligence"
        );
    }
}

#[test]
fn qualification_claims_fail_closed() {
    for claim in [
        QualificationClaim::LiveQualified,
        QualificationClaim::Stage3Pass,
        QualificationClaim::Alpha,
    ] {
        let request = MaterializeRequest {
            qualification: claim,
            feature_set_version: FeatureSetVersion::new("synthetic-replay-v1").unwrap(),
        };
        let error = materialize_synthetic_replay(&synthetic_blocks(), &request).unwrap_err();
        assert!(matches!(
            error,
            IntelligenceReplayError::QualificationClaim { .. }
        ));
        assert_eq!(
            error.reason_code(),
            "intelligence_replay.qualification_claim"
        );
    }
}

#[test]
fn action_bearing_committed_blocks_fail_closed() {
    let payload = serde_json::json!({
        "abci_block": {
            "time": "2026-07-28T12:00:00.000000000",
            "round": 992814678,
            "parent_round": 992814677,
            "proposer": "0x5ac99df645f3414876c816caa18b2d234024b487"
        },
        "signed_action_bundles": [["0xbundle", {"signed_actions": []}]]
    });
    let error = materialize_committed_node(
        &parse_committed(payload),
        &committed_context(992_814_678),
        &request(),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        IntelligenceReplayError::ActionBearingRejected { action_bundles: 1 }
    ));
}

#[test]
fn empty_committed_block_is_watermark_only_and_missing_intelligence() {
    let payload = serde_json::json!({
        "abci_block": {
            "time": "2026-07-28T12:00:00.000000000",
            "round": 992814678,
            "parent_round": 992814677,
            "proposer": "0x5ac99df645f3414876c816caa18b2d234024b487",
            "signed_action_bundles": []
        }
    });
    let error = materialize_committed_node(
        &parse_committed(payload),
        &committed_context(992_814_678),
        &request(),
    )
    .unwrap_err();
    assert!(matches!(error, IntelligenceReplayError::MissingState));
}

#[test]
fn synthetic_confirmation_cannot_enter_live() {
    let error = Signal::try_new(
        SignalId::new("sig-live").unwrap(),
        SignalType::IndependentSmartFlowAcceleration,
        MarketId::new("BTC").unwrap(),
        Direction::Long,
        known(1),
        time(1),
        BlockHeight::new(1),
        SignalConfirmationClass::SyntheticUnqualified,
        Horizon::MINUTES_5,
        domain_types::BasisPoints::from_raw(20, 0).unwrap(),
        domain_types::BasisPoints::from_raw(5, 0).unwrap(),
        ProbabilityPpm::ONE,
        ClosedInterval::new(
            domain_types::BasisPoints::from_raw(1, 0).unwrap(),
            domain_types::BasisPoints::from_raw(30, 0).unwrap(),
        )
        .unwrap(),
        UsdAmount::from_raw(1, 8).unwrap(),
        Horizon::MINUTES_5,
        ProbabilityPpm::from_ppm(100_000).unwrap(),
        domain_types::BasisPoints::from_raw(10, 0).unwrap(),
        HealthAssessment::try_new("signal", HealthState::Amber, "synthetic_unassessed").unwrap(),
        domain_types::ModelVersion::new("signal-v1").unwrap(),
        FeatureSetVersion::new("synthetic-replay-v1").unwrap(),
        [1_u8; 32],
        [2_u8; 32],
        SignalLifecycleState::Live,
    )
    .unwrap_err();
    assert!(matches!(error, SignalError::ContractViolation(_)));
}

#[test]
fn missing_book_or_fills_cannot_emit_crowding_fragility_or_live_signals() {
    let report = materialize_synthetic_replay(&synthetic_blocks(), &request()).unwrap();
    assert_synthetic_unassessed(&report);
    assert!(!report.stage_3_pass);
    assert!(!report.live_qualified);
    assert!(!report.alpha_qualified);
    assert_eq!(report.source_qualification, "synthetic_unassessed");

    let remaining = UsdAmount::from_raw(0, 8).unwrap();
    let invented_marks = vec![CrowdingPosition {
        entity_id: EntityId::new("invented-mark").unwrap(),
        independence_weight: ProbabilityPpm::ONE,
        is_follower: false,
        post_originator: false,
        exposure: UsdAmount::from_raw(100_000_000, 8).unwrap(),
        entry_bps_from_mark: 12,
        funding_percentile: ProbabilityPpm::from_ppm(500_000).unwrap(),
        leverage_milli: 200_000,
    }];
    let scenario = FragilityScenario::default_grid(ScenarioId::new("replay-deny").unwrap());
    assert!(!report.market_snapshots.is_empty());
    for snapshot in &report.market_snapshots {
        assert!(matches!(
            snapshot.require_observed_book_and_fills(),
            Err(MarketError::MissingInput { name: "book" })
        ));
        assert!(matches!(
            crowding_components_from_snapshot(snapshot, &invented_marks, remaining),
            Err(MarketError::MissingInput { name: "book" })
        ));
        assert!(matches!(
            simulate_fragility_from_snapshot(snapshot, &scenario, &[], -100),
            Err(MarketError::MissingInput { name: "book" })
        ));
        match suppress_proof_withhold(snapshot) {
            Some(signal_core::SignalEvaluation::Suppressed { reasons, .. }) => {
                assert!(
                    reasons
                        .iter()
                        .any(|reason| reason == "missing_book_or_fills")
                );
            }
            other => panic!("expected suppression, got {other:?}"),
        }
        assert_book_or_fills_qualification_is_not_inventory(
            ProofWithholdReason::MissingBookOrFills,
        );
    }
}

#[test]
fn missing_inventory_cannot_emit_live_signals_with_decimal_book() {
    let remaining = UsdAmount::from_raw(0, 8).unwrap();
    let invented_marks = vec![CrowdingPosition {
        entity_id: EntityId::new("invented-mark").unwrap(),
        independence_weight: ProbabilityPpm::ONE,
        is_follower: false,
        post_originator: false,
        exposure: UsdAmount::from_raw(100_000_000, 8).unwrap(),
        entry_bps_from_mark: 12,
        funding_percentile: ProbabilityPpm::from_ppm(500_000).unwrap(),
        leverage_milli: 200_000,
    }];
    let scenario = FragilityScenario::default_grid(ScenarioId::new("replay-deny").unwrap());
    let snapshot = constructed_market_snapshot(
        FeatureValue::Decimal {
            raw: 20_000 * 100_000_000,
            scale: 8,
        },
        FeatureValue::Boolean(true),
        FeatureValue::Missing(MissingReason::NotObserved),
    );
    assert!(matches!(
        snapshot.require_observed_book_and_fills(),
        Err(MarketError::MissingInput { name: "inventory" })
    ));
    assert!(matches!(
        crowding_components_from_snapshot(&snapshot, &invented_marks, remaining),
        Err(MarketError::MissingInput { name: "inventory" })
    ));
    assert!(matches!(
        simulate_fragility_from_snapshot(&snapshot, &scenario, &[], -100),
        Err(MarketError::MissingInput { name: "inventory" })
    ));
    assert_eq!(
        proof_withhold_reason(&snapshot),
        Some(ProofWithholdReason::MissingInventory)
    );
    match suppress_proof_withhold(&snapshot) {
        Some(signal_core::SignalEvaluation::Suppressed { reasons, .. }) => {
            assert_eq!(
                reasons.as_slice(),
                [ProofWithholdReason::MissingInventory.as_wire_name()]
            );
            assert!(
                !reasons
                    .iter()
                    .any(|reason| reason == "missing_book_or_fills")
            );
        }
        other => panic!("expected missing inventory suppression, got {other:?}"),
    }
    assert_inventory_qualification_is_not_invented_fills(ProofWithholdReason::MissingInventory);
}

#[test]
fn boolean_inventory_cannot_emit_live_signals_with_decimal_book() {
    let remaining = UsdAmount::from_raw(0, 8).unwrap();
    let invented_marks = vec![CrowdingPosition {
        entity_id: EntityId::new("invented-mark").unwrap(),
        independence_weight: ProbabilityPpm::ONE,
        is_follower: false,
        post_originator: false,
        exposure: UsdAmount::from_raw(100_000_000, 8).unwrap(),
        entry_bps_from_mark: 12,
        funding_percentile: ProbabilityPpm::from_ppm(500_000).unwrap(),
        leverage_milli: 200_000,
    }];
    let scenario = FragilityScenario::default_grid(ScenarioId::new("replay-deny").unwrap());
    let snapshot = constructed_market_snapshot(
        FeatureValue::Decimal {
            raw: 20_000 * 100_000_000,
            scale: 8,
        },
        FeatureValue::Boolean(true),
        FeatureValue::Boolean(true),
    );
    assert!(matches!(
        snapshot.require_observed_book_and_fills(),
        Err(MarketError::Malformed {
            what: "inventory",
            reason: "boolean cannot mint decimal depth",
        })
    ));
    assert!(matches!(
        crowding_components_from_snapshot(&snapshot, &invented_marks, remaining),
        Err(MarketError::Malformed {
            what: "inventory",
            reason: "boolean cannot mint decimal depth",
        })
    ));
    assert!(matches!(
        simulate_fragility_from_snapshot(&snapshot, &scenario, &[], -100),
        Err(MarketError::Malformed {
            what: "inventory",
            reason: "boolean cannot mint decimal depth",
        })
    ));
    assert_eq!(
        proof_withhold_reason(&snapshot),
        Some(ProofWithholdReason::MalformedInventory)
    );
    match suppress_proof_withhold(&snapshot) {
        Some(signal_core::SignalEvaluation::Suppressed { reasons, .. }) => {
            assert_eq!(
                reasons.as_slice(),
                [ProofWithholdReason::MalformedInventory.as_wire_name()]
            );
            assert!(
                !reasons
                    .iter()
                    .any(|reason| reason == "missing_book_or_fills")
            );
            assert!(
                !reasons
                    .iter()
                    .any(|reason| reason == ProofWithholdReason::MissingInventory.as_wire_name())
            );
        }
        other => panic!("expected malformed inventory suppression, got {other:?}"),
    }
    assert_inventory_qualification_is_not_invented_fills(ProofWithholdReason::MalformedInventory);
}

#[test]
fn replay_qualification_pins_book_fills_and_inventory_families_separately() {
    for reason in [
        ProofWithholdReason::MissingBookOrFills,
        ProofWithholdReason::MissingInventory,
        ProofWithholdReason::MalformedInventory,
    ] {
        match reason {
            ProofWithholdReason::MissingBookOrFills => {
                assert_book_or_fills_qualification_is_not_inventory(reason);
            }
            ProofWithholdReason::MissingInventory => {
                assert_inventory_qualification_is_not_invented_fills(reason);
            }
            ProofWithholdReason::MalformedInventory => {
                assert_inventory_qualification_is_not_invented_fills(reason);
            }
        }
    }
}

#[test]
fn mixed_snapshot_fold_prefers_book_fills_then_missing_inventory() {
    let missing_book_and_inventory = constructed_market_snapshot(
        FeatureValue::Missing(MissingReason::NotObserved),
        FeatureValue::Boolean(true),
        FeatureValue::Missing(MissingReason::NotObserved),
    );
    let missing_inventory = constructed_market_snapshot(
        FeatureValue::Decimal {
            raw: 20_000 * 100_000_000,
            scale: 8,
        },
        FeatureValue::Boolean(true),
        FeatureValue::Missing(MissingReason::NotObserved),
    );
    let boolean_inventory = constructed_market_snapshot(
        FeatureValue::Decimal {
            raw: 20_000 * 100_000_000,
            scale: 8,
        },
        FeatureValue::Boolean(true),
        FeatureValue::Boolean(true),
    );

    assert_eq!(
        proof_withhold_reason(&missing_book_and_inventory),
        Some(ProofWithholdReason::MissingBookOrFills)
    );
    assert_eq!(
        proof_withhold_reason(&missing_inventory),
        Some(ProofWithholdReason::MissingInventory)
    );
    assert_eq!(
        proof_withhold_reason(&boolean_inventory),
        Some(ProofWithholdReason::MalformedInventory)
    );

    let book_over_missing = fold_withhold_reason(
        proof_withhold_reason(&missing_book_and_inventory),
        proof_withhold_reason(&missing_inventory),
    );
    assert_eq!(
        book_over_missing,
        Some(ProofWithholdReason::MissingBookOrFills)
    );
    assert_book_or_fills_qualification_is_not_inventory(
        book_over_missing.expect("mixed book/inventory fold"),
    );

    let missing_over_book = fold_withhold_reason(
        proof_withhold_reason(&missing_inventory),
        proof_withhold_reason(&missing_book_and_inventory),
    );
    assert_eq!(
        missing_over_book,
        Some(ProofWithholdReason::MissingBookOrFills)
    );
    assert_book_or_fills_qualification_is_not_inventory(
        missing_over_book.expect("mixed inventory/book fold"),
    );

    let missing_over_malformed = fold_withhold_reason(
        proof_withhold_reason(&missing_inventory),
        proof_withhold_reason(&boolean_inventory),
    );
    assert_eq!(
        missing_over_malformed,
        Some(ProofWithholdReason::MissingInventory)
    );
    assert_inventory_qualification_is_not_invented_fills(
        missing_over_malformed.expect("missing vs malformed fold"),
    );

    let malformed_over_missing = fold_withhold_reason(
        proof_withhold_reason(&boolean_inventory),
        proof_withhold_reason(&missing_inventory),
    );
    assert_eq!(
        malformed_over_missing,
        Some(ProofWithholdReason::MissingInventory)
    );
    assert_inventory_qualification_is_not_invented_fills(
        malformed_over_missing.expect("malformed vs missing fold"),
    );

    assert_eq!(
        fold_withhold_reason(None, proof_withhold_reason(&boolean_inventory)),
        Some(ProofWithholdReason::MalformedInventory)
    );
    assert_inventory_qualification_is_not_invented_fills(ProofWithholdReason::MalformedInventory);
}

#[test]
fn fold_withhold_reason_covers_every_proof_family_pair() {
    let reasons = [
        ProofWithholdReason::MissingBookOrFills,
        ProofWithholdReason::MissingInventory,
        ProofWithholdReason::MalformedInventory,
    ];
    for current in reasons {
        for next in reasons {
            let folded = fold_withhold_reason(Some(current), Some(next))
                .expect("present reasons fold to a reason");
            match (current, next) {
                (
                    ProofWithholdReason::MissingBookOrFills,
                    ProofWithholdReason::MissingBookOrFills,
                )
                | (
                    ProofWithholdReason::MissingBookOrFills,
                    ProofWithholdReason::MissingInventory,
                )
                | (
                    ProofWithholdReason::MissingBookOrFills,
                    ProofWithholdReason::MalformedInventory,
                )
                | (
                    ProofWithholdReason::MissingInventory,
                    ProofWithholdReason::MissingBookOrFills,
                )
                | (
                    ProofWithholdReason::MalformedInventory,
                    ProofWithholdReason::MissingBookOrFills,
                ) => {
                    assert_eq!(folded, ProofWithholdReason::MissingBookOrFills);
                    assert_book_or_fills_qualification_is_not_inventory(folded);
                }
                (ProofWithholdReason::MissingInventory, ProofWithholdReason::MissingInventory)
                | (
                    ProofWithholdReason::MissingInventory,
                    ProofWithholdReason::MalformedInventory,
                )
                | (
                    ProofWithholdReason::MalformedInventory,
                    ProofWithholdReason::MissingInventory,
                ) => {
                    assert_eq!(folded, ProofWithholdReason::MissingInventory);
                    assert_inventory_qualification_is_not_invented_fills(folded);
                }
                (
                    ProofWithholdReason::MalformedInventory,
                    ProofWithholdReason::MalformedInventory,
                ) => {
                    assert_eq!(folded, ProofWithholdReason::MalformedInventory);
                    assert_inventory_qualification_is_not_invented_fills(folded);
                }
            }
        }
    }
    assert_eq!(fold_withhold_reason(None, None), None);
}

fn assert_book_or_fills_qualification_is_not_inventory(reason: ProofWithholdReason) {
    assert_eq!(reason, ProofWithholdReason::MissingBookOrFills);
    let what = qualification_what_for_withhold(reason);
    assert_eq!(what, "invented_marks_or_fills");
    assert_ne!(what, ProofWithholdReason::MissingInventory.as_wire_name());
    assert_ne!(what, ProofWithholdReason::MalformedInventory.as_wire_name());
    assert!(matches!(
        refuse_leaked_withheld_emission(Some(reason), 1, 0, 0),
        Err(IntelligenceReplayError::QualificationClaim {
            what: "invented_marks_or_fills"
        })
    ));
    assert!(refuse_leaked_withheld_emission(Some(reason), 0, 0, 0).is_ok());
}

fn assert_inventory_qualification_is_not_invented_fills(reason: ProofWithholdReason) {
    let expected = match reason {
        ProofWithholdReason::MissingInventory => {
            ProofWithholdReason::MissingInventory.as_wire_name()
        }
        ProofWithholdReason::MalformedInventory => {
            ProofWithholdReason::MalformedInventory.as_wire_name()
        }
        ProofWithholdReason::MissingBookOrFills => {
            panic!("book/fills must not reuse the inventory qualification helper")
        }
    };
    let what = qualification_what_for_withhold(reason);
    assert_eq!(what, expected);
    assert_ne!(what, "invented_marks_or_fills");
    match refuse_leaked_withheld_emission(Some(reason), 1, 0, 0) {
        Err(IntelligenceReplayError::QualificationClaim { what }) => {
            assert_eq!(what, expected);
            assert_ne!(what, "invented_marks_or_fills");
        }
        other => panic!("expected typed inventory qualification claim, got {other:?}"),
    }
    assert!(refuse_leaked_withheld_emission(Some(reason), 0, 0, 0).is_ok());
}

fn constructed_market_snapshot(
    book: FeatureValue,
    fills: FeatureValue,
    inventory: FeatureValue,
) -> MarketFeatureSnapshot {
    let mut values = BTreeMap::new();
    values.insert(market_feature_key("book").unwrap(), book);
    values.insert(market_feature_key("fills").unwrap(), fills);
    values.insert(market_feature_key("inventory").unwrap(), inventory);
    MarketFeatureSnapshot::try_new(
        MarketId::new("BTC").unwrap(),
        Horizon::MINUTES_5,
        FeatureSetVersion::new("market-v1").unwrap(),
        time(1),
        known(1),
        BlockHeight::new(1),
        values,
        HealthAssessment::try_new("market", HealthState::Green, "synthetic").unwrap(),
    )
    .unwrap()
}
