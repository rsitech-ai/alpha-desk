use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;

use canonical_events::{
    AssetContextUpdated, BlockEnvelope, CanonicalEventEnvelope, CanonicalEventInput,
    CommittedNodeV1MappingContext, ConfirmationClass, DepositCredited, DexCreated, EventPayload,
    MarketCreated, SourceEvidence, SpotTransfer, SubaccountTransfer, VaultDeposit,
};
use domain_types::{
    AccountId, Address, AssetId, BlockHeight, ChainId, ClosedInterval, DexId, Direction, EntityId,
    FeatureSetVersion, Horizon, KnownTime, MarketId, Price, ProbabilityPpm, ProtocolTime, Quantity,
    QuoteAmount, ScenarioId, SignalId, SourceId, TransactionId, UsdAmount, VaultId,
};
use feature_core::{FeatureValue, HealthState, MissingReason};
use hl_protocol::node::v1::{NodeStreamKind, parse_node_record};
use intelligence_replay::{
    IntelligenceReplayError, IntelligenceReplayReport, MaterializeRequest, QualificationClaim,
    materialize_committed_node, materialize_synthetic_replay,
};
use market_intelligence::{
    CrowdingPosition, FragilityScenario, MarketError, crowding_components_from_snapshot,
    simulate_fragility_from_snapshot,
};
use signal_core::{
    Signal, SignalConfirmationClass, SignalError, SignalLifecycleState, SignalType,
    suppress_missing_book_or_fills,
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
        confirmation_class: ConfirmationClass::CommittedPrimary,
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
        feature_core::HealthAssessment::try_new(
            "signal",
            HealthState::Amber,
            "synthetic_unassessed",
        )
        .unwrap(),
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
        match suppress_missing_book_or_fills(snapshot) {
            Some(signal_core::SignalEvaluation::Suppressed { reasons, .. }) => {
                assert!(
                    reasons
                        .iter()
                        .any(|reason| reason == "missing_book_or_fills")
                );
            }
            other => panic!("expected suppression, got {other:?}"),
        }
    }
}
