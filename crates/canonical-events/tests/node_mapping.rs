use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use canonical_events::{
    BlockEnvelope, CanonicalEventEnvelope, CommittedNodeV1MappingContext, ConfirmationClass,
    EventPayload, EvidenceOnlyReason, MappingDisposition, MappingError, MarketCatalogV1,
    NodeV1MappingContext, TradeParticipantRoleV1, map_committed_node_v1_block, map_node_v1_record,
};
use domain_types::{BlockHeight, ChainId, KnownTime, MarketId, SourceId};
use hl_protocol::SourceError;
use hl_protocol::node::v1::{NodeRecordKind, NodeStreamKind, parse_node_record};

fn fixture(name: &str) -> Vec<u8> {
    fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("fixtures/source/node-v1")
            .join(name),
    )
    .expect("fixture")
}

fn catalog() -> MarketCatalogV1 {
    MarketCatalogV1::try_new(
        "normalized-public-docs-v1",
        [("COMP", MarketId::new("perp:COMP").unwrap())],
    )
    .unwrap()
}

fn context() -> NodeV1MappingContext {
    NodeV1MappingContext {
        chain_id: domain_types::ChainId::new("hyperliquid-mainnet").unwrap(),
        source_id: SourceId::new("fixture-node-trades").unwrap(),
        source_version: "node-public-docs-2026-07-29".to_owned(),
        source_offset: "fixtures/source/node-v1/trade-batch.json".to_owned(),
        observed_at: KnownTime::from_unix_micros(1_721_982_386_000_000).unwrap(),
        ingested_at: KnownTime::from_unix_micros(1_721_982_386_100_000).unwrap(),
        canonicalized_at: KnownTime::from_unix_micros(1_721_982_386_200_000).unwrap(),
        mapper_version: "node-v1-mapper-1".to_owned(),
    }
}

fn committed_context() -> CommittedNodeV1MappingContext {
    CommittedNodeV1MappingContext {
        chain_id: ChainId::new("hyperliquid-mainnet").unwrap(),
        source_id: SourceId::new("primary-node").unwrap(),
        source_version: "node-public-docs-2026-07-29".to_owned(),
        source_offset: "992814678".to_owned(),
        expected_height: BlockHeight::new(992_814_678),
        confirmation_class: ConfirmationClass::CommittedPrimary,
    }
}

#[test]
fn empty_transaction_block_maps_to_a_committed_source_bound_block() {
    let record = parse_node_record(
        NodeStreamKind::TransactionBlocks,
        fixture("transaction-block.json").into(),
    )
    .unwrap();

    let first = map_committed_node_v1_block(&record, &committed_context()).unwrap();
    let repeated = map_committed_node_v1_block(&record, &committed_context()).unwrap();

    assert_eq!(first, repeated);
    assert_eq!(first.chain_id().as_str(), "hyperliquid-mainnet");
    assert_eq!(first.block_height(), BlockHeight::new(992_814_678));
    assert_eq!(first.block_time().unix_micros(), 1_785_240_000_000_000);
    assert_eq!(
        first.confirmation_class(),
        ConfirmationClass::CommittedPrimary
    );
    assert!(first.events().is_empty());
    assert_eq!(
        first.source_block_hashes()[&SourceId::new("primary-node").unwrap()],
        *record.content_hash().as_bytes()
    );
}

#[test]
fn committed_mapper_confirmation_covers_every_class() {
    let record = parse_node_record(
        NodeStreamKind::TransactionBlocks,
        fixture("transaction-block.json").into(),
    )
    .unwrap();

    for class in [
        ConfirmationClass::ProvisionalSource,
        ConfirmationClass::CommittedPrimary,
        ConfirmationClass::CommittedIndependent,
        ConfirmationClass::ReconciledSnapshot,
        ConfirmationClass::Corrected,
        ConfirmationClass::Expired,
    ] {
        let mut context = committed_context();
        context.confirmation_class = class;
        match class {
            ConfirmationClass::CommittedPrimary | ConfirmationClass::CommittedIndependent => {
                let block = map_committed_node_v1_block(&record, &context)
                    .expect("empty committed blocks still map");
                assert_eq!(block.confirmation_class(), class);
                assert!(block.events().is_empty());
            }
            ConfirmationClass::ProvisionalSource
            | ConfirmationClass::ReconciledSnapshot
            | ConfirmationClass::Corrected
            | ConfirmationClass::Expired => {
                let error = map_committed_node_v1_block(&record, &context)
                    .expect_err("non-committed lanes fail closed");
                assert!(
                    matches!(error, MappingError::InvalidCommittedConfirmation),
                    "{class:?} must not blur into committed mapping"
                );
                assert_eq!(
                    error.reason_code(),
                    "canonical_mapping.invalid_committed_confirmation",
                    "{class:?} must reuse the existing committed-mapping reason"
                );
            }
        }
    }
}

#[test]
fn committed_mapper_rejects_height_discontinuity_and_unmapped_actions() {
    let record = parse_node_record(
        NodeStreamKind::TransactionBlocks,
        fixture("transaction-block.json").into(),
    )
    .unwrap();
    let mut mismatched = committed_context();
    mismatched.expected_height = BlockHeight::new(992_814_679);
    let error = map_committed_node_v1_block(&record, &mismatched).unwrap_err();
    assert!(matches!(error, MappingError::BlockHeightMismatch { .. }));
    assert_eq!(
        error.reason_code(),
        "canonical_mapping.block_height_mismatch"
    );

    let payload = serde_json::json!({
        "abci_block": {
            "time": "2026-07-28T12:00:00.000000000",
            "round": 992814678,
            "parent_round": 992814677,
            "proposer": "0x5ac99df645f3414876c816caa18b2d234024b487"
        },
        "signed_action_bundles": [["0xbundle", {"signed_actions": []}]]
    });
    let record = parse_node_record(
        NodeStreamKind::TransactionBlocks,
        serde_json::to_vec(&payload).unwrap().into(),
    )
    .unwrap();
    let error = map_committed_node_v1_block(&record, &committed_context()).unwrap_err();
    assert!(matches!(
        error,
        MappingError::UnsupportedCommittedActions { action_bundles: 1 }
    ));
    assert_eq!(
        error.reason_code(),
        "canonical_mapping.unsupported_committed_actions"
    );
}

#[test]
fn committed_mapper_accepts_the_current_nested_empty_bundle_shape_only_when_unambiguous() {
    let nested = serde_json::json!({
        "abci_block": {
            "time": "2026-07-28T12:00:00.000000000",
            "round": 992814678,
            "parent_round": 992814677,
            "proposer": "0x5ac99df645f3414876c816caa18b2d234024b487",
            "signed_action_bundles": []
        }
    });
    let record = parse_node_record(
        NodeStreamKind::TransactionBlocks,
        serde_json::to_vec(&nested).unwrap().into(),
    )
    .unwrap();
    map_committed_node_v1_block(&record, &committed_context()).unwrap();

    let ambiguous = serde_json::json!({
        "abci_block": {
            "time": "2026-07-28T12:00:00.000000000",
            "round": 992814678,
            "parent_round": 992814677,
            "proposer": "0x5ac99df645f3414876c816caa18b2d234024b487",
            "signed_action_bundles": []
        },
        "signed_action_bundles": []
    });
    let record = parse_node_record(
        NodeStreamKind::TransactionBlocks,
        serde_json::to_vec(&ambiguous).unwrap().into(),
    )
    .unwrap();
    let error = map_committed_node_v1_block(&record, &committed_context()).unwrap_err();
    assert_eq!(error.reason_code(), "canonical_mapping.malformed_record");
}

#[test]
fn complete_block_batched_trade_maps_with_explicit_maker_taker() {
    let record =
        parse_node_record(NodeStreamKind::Trades, fixture("trade-batch.json").into()).unwrap();

    let first = map_node_v1_record(&record, &catalog(), &context()).unwrap();
    let repeated = map_node_v1_record(&record, &catalog(), &context()).unwrap();

    assert_eq!(first, repeated);
    let MappingDisposition::Mapped(events) = first else {
        panic!("block-bearing complete trade must map");
    };
    assert_eq!(events.len(), 1);

    let event = &events[0];
    assert_eq!(event.block_height().get(), 42);
    assert_eq!(event.block_time().unix_micros(), 1_721_982_385_899_000);
    assert_eq!(
        event.transaction_id().as_str(),
        "0xad8e0566e813bdf98176040e6d51bd011100efa789e89430cdf17964235f55d8"
    );
    assert_eq!(event.transaction_index(), 0);
    assert_eq!(event.canonical_event_index(), 0);
    assert_eq!(
        event.confirmation_class(),
        ConfirmationClass::ProvisionalSource
    );
    assert_eq!(
        event.account_addresses()[0].to_api_string(),
        "0xc64cc00b46101bd40aa1c3121195e85c0b0918d8"
    );
    assert_eq!(
        event.account_addresses()[1].to_api_string(),
        "0x768484f7e2ebb675c57838366c02ae99ba2a9b08"
    );
    assert_eq!(event.source_evidence()[0].source_event_index(), Some(0));
    assert_eq!(
        event.parser_version(),
        "node-v1-mapper-1/catalog:normalized-public-docs-v1"
    );

    let EventPayload::TradeMatched(trade) = event.payload() else {
        panic!("trade source must map to TradeMatched");
    };
    assert_eq!(trade.market_id.as_ref().unwrap().as_str(), "perp:COMP");
    assert_eq!(trade.price.to_string(), "51.367");
    assert_eq!(trade.quantity.to_string(), "0.31");
    assert_eq!(
        trade.trade_id.as_ref().unwrap().as_str(),
        "trd_9d76b6581c97fe76b0d8e8e1bec50b7fc85ead4f7235abff2a03f9991c0e70ff"
    );
    assert_eq!(
        trade.maker_order_id.as_ref().unwrap().as_str(),
        "12212198275"
    );
    assert_eq!(
        trade.taker_order_id.as_ref().unwrap().as_str(),
        "12212201265"
    );
    assert_eq!(
        canonical_events::node_trade_match_key(trade.trade_id.as_ref().unwrap()),
        format!("node-trade:{}", trade.trade_id.as_ref().unwrap().as_str())
    );
    assert_eq!(trade.deterministic_seed, 0);
    let [buyer, seller] = trade
        .participants
        .as_deref()
        .expect("documented side_info must be retained");
    assert_eq!(buyer.role, TradeParticipantRoleV1::Buyer);
    assert_eq!(buyer.account_id, event.account_addresses()[0]);
    assert_eq!(buyer.start_position.to_string(), "996.67");
    assert_eq!(buyer.order_id.as_str(), "12212201265");
    assert_eq!(buyer.twap_id, None);
    assert_eq!(buyer.client_order_id, None);
    assert_eq!(seller.role, TradeParticipantRoleV1::Seller);
    assert_eq!(seller.account_id, event.account_addresses()[1]);
    assert_eq!(seller.start_position.to_string(), "-996.7");
    assert_eq!(seller.order_id.as_str(), "12212198275");
    assert_eq!(seller.twap_id, None);
    assert_eq!(seller.client_order_id, None);
    assert_eq!(
        event.source_evidence()[0].content_hash(),
        *record.content_hash().as_bytes()
    );
}

#[test]
fn documented_trade_optionals_are_retained_with_maker_taker() {
    let mut batch: serde_json::Value =
        serde_json::from_slice(&fixture("trade-batch.json")).expect("batch JSON");
    batch["events"][0]["side_info"][0]["twap_id"] = serde_json::json!(91);
    batch["events"][0]["side_info"][0]["cloid"] =
        serde_json::json!("0x11111111111111111111111111111111");
    let record = parse_node_record(
        NodeStreamKind::Trades,
        serde_json::to_vec(&batch).unwrap().into(),
    )
    .unwrap();

    let MappingDisposition::Mapped(events) =
        map_node_v1_record(&record, &catalog(), &context()).unwrap()
    else {
        panic!("complete trade batch must map");
    };
    let EventPayload::TradeMatched(trade) = events[0].payload() else {
        panic!("trade source must map to TradeMatched");
    };
    let buyer = &trade.participants.as_ref().unwrap()[0];
    assert_eq!(buyer.twap_id.unwrap().get(), 91);
    assert_eq!(
        buyer.client_order_id.as_ref().unwrap().as_str(),
        "0x11111111111111111111111111111111"
    );
    assert_eq!(
        trade.maker_order_id.as_ref().unwrap().as_str(),
        "12212198275"
    );
    assert_eq!(
        trade.taker_order_id.as_ref().unwrap().as_str(),
        "12212201265"
    );
}

#[test]
fn malformed_or_incomplete_trade_participants_fail_closed() {
    let base: serde_json::Value =
        serde_json::from_slice(&fixture("trade-batch.json")).expect("batch JSON");
    let mut cases = Vec::new();

    let mut wrong_length = base.clone();
    wrong_length["events"][0]["side_info"]
        .as_array_mut()
        .unwrap()
        .pop();
    cases.push(wrong_length);

    for field in ["user", "start_pos", "oid", "twap_id", "cloid"] {
        let mut missing = base.clone();
        missing["events"][0]["side_info"][0]
            .as_object_mut()
            .unwrap()
            .remove(field);
        cases.push(missing);
    }

    for (field, invalid) in [
        ("start_pos", serde_json::json!("not-a-position")),
        ("oid", serde_json::json!(0)),
        ("twap_id", serde_json::json!("91")),
        ("cloid", serde_json::json!(91)),
        ("cloid", serde_json::json!("")),
        (
            "cloid",
            serde_json::json!("11111111111111111111111111111111"),
        ),
        (
            "cloid",
            serde_json::json!("0x1111111111111111111111111111111"),
        ),
        (
            "cloid",
            serde_json::json!("0x111111111111111111111111111111111"),
        ),
        (
            "cloid",
            serde_json::json!("0xA1111111111111111111111111111111"),
        ),
        (
            "cloid",
            serde_json::json!("0xg1111111111111111111111111111111"),
        ),
    ] {
        let mut malformed = base.clone();
        malformed["events"][0]["side_info"][0][field] = invalid;
        cases.push(malformed);
    }
    let mut duplicate_accounts = base.clone();
    duplicate_accounts["events"][0]["side_info"][1]["user"] =
        duplicate_accounts["events"][0]["side_info"][0]["user"].clone();
    cases.push(duplicate_accounts);

    for case in cases {
        let Ok(record) = parse_node_record(
            NodeStreamKind::Trades,
            serde_json::to_vec(&case).unwrap().into(),
        ) else {
            continue;
        };
        assert!(
            map_node_v1_record(&record, &catalog(), &context()).is_err(),
            "malformed side_info must not be partially mapped: {case}"
        );
    }
}

#[test]
fn unmapped_trade_market_fails_closed_with_stable_reason_code() {
    let record =
        parse_node_record(NodeStreamKind::Trades, fixture("trade-batch.json").into()).unwrap();
    let empty_catalog =
        MarketCatalogV1::try_new("empty-v1", std::iter::empty::<(&str, MarketId)>()).unwrap();

    let error = map_node_v1_record(&record, &empty_catalog, &context())
        .expect_err("unknown market must not be guessed");

    assert!(matches!(
        error,
        MappingError::UnmappedMarket { ref symbol } if symbol == "COMP"
    ));
    assert_eq!(error.reason_code(), "canonical_mapping.unmapped_market");
}

#[test]
fn standalone_trade_is_explicit_evidence_only_without_block_context() {
    let batch: serde_json::Value =
        serde_json::from_slice(&fixture("trade-batch.json")).expect("batch JSON");
    let standalone = serde_json::to_vec(&batch["events"][0]).expect("standalone trade");
    let record = parse_node_record(NodeStreamKind::Trades, standalone.into()).unwrap();

    let disposition = map_node_v1_record(&record, &catalog(), &context()).unwrap();

    assert_eq!(
        disposition,
        MappingDisposition::EvidenceOnly(EvidenceOnlyReason::MissingBlockContext)
    );
}

#[test]
fn repeated_transaction_hash_uses_contiguous_event_indices_and_source_sub_indices() {
    let mut batch: serde_json::Value =
        serde_json::from_slice(&fixture("trade-batch.json")).expect("batch JSON");
    let second = batch["events"][0].clone();
    batch["events"].as_array_mut().unwrap().push(second);
    let record = parse_node_record(
        NodeStreamKind::Trades,
        serde_json::to_vec(&batch).unwrap().into(),
    )
    .unwrap();

    let MappingDisposition::Mapped(events) =
        map_node_v1_record(&record, &catalog(), &context()).unwrap()
    else {
        panic!("complete trade batch must map");
    };

    assert_eq!(events.len(), 2);
    assert_eq!(events[0].transaction_index(), 0);
    assert_eq!(events[0].canonical_event_index(), 0);
    assert_eq!(events[0].source_evidence()[0].source_event_index(), Some(0));
    assert_eq!(events[1].transaction_index(), 0);
    assert_eq!(events[1].canonical_event_index(), 1);
    assert_eq!(events[1].source_evidence()[0].source_event_index(), Some(1));
    assert_ne!(events[0].event_id(), events[1].event_id());
}

#[test]
fn known_one_sided_fill_is_evidence_only_and_never_a_complete_trade() {
    let record = parse_node_record(NodeStreamKind::Fills, fixture("fill.json").into()).unwrap();

    assert_eq!(
        map_node_v1_record(&record, &catalog(), &context()).unwrap(),
        MappingDisposition::EvidenceOnly(EvidenceOnlyReason::OneSidedFill)
    );
    assert_eq!(
        EvidenceOnlyReason::OneSidedFill.reason_code(),
        "canonical_mapping.one_sided_fill"
    );
}

#[test]
fn public_node_v1_corpus_has_an_explicit_disposition_for_every_fixture() {
    #[derive(Debug)]
    enum Expected {
        MappedTrade,
        EmptyCommittedBlock,
        EvidenceOnly(EvidenceOnlyReason),
        SourceSchemaDrift,
    }

    let cases = [
        (
            "trade-batch.json",
            NodeStreamKind::Trades,
            Expected::MappedTrade,
        ),
        (
            "transaction-block.json",
            NodeStreamKind::TransactionBlocks,
            Expected::EmptyCommittedBlock,
        ),
        (
            "fill.json",
            NodeStreamKind::Fills,
            Expected::EvidenceOnly(EvidenceOnlyReason::OneSidedFill),
        ),
        (
            "order-status.json",
            NodeStreamKind::OrderStatuses,
            Expected::EvidenceOnly(EvidenceOnlyReason::AuxiliaryOrderStatus),
        ),
        (
            "raw-book-diff.json",
            NodeStreamKind::RawBookDiffs,
            Expected::EvidenceOnly(EvidenceOnlyReason::AuxiliaryBookDiff),
        ),
        (
            "transfer.json",
            NodeStreamKind::MiscEvents,
            Expected::EvidenceOnly(EvidenceOnlyReason::IncompleteLedgerTransfer),
        ),
        (
            "liquidation.json",
            NodeStreamKind::MiscEvents,
            Expected::EvidenceOnly(EvidenceOnlyReason::IncompleteLiquidation),
        ),
        (
            "market-metadata.json",
            NodeStreamKind::MarketMetadata,
            Expected::EvidenceOnly(EvidenceOnlyReason::AuxiliaryMarketMetadata),
        ),
        (
            "unknown-variant.json",
            NodeStreamKind::MiscEvents,
            Expected::SourceSchemaDrift,
        ),
    ];
    let corpus_json = cases.iter().map(|(file, _, _)| *file).collect::<Vec<_>>();
    let mut on_disk = std::fs::read_dir(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("fixtures/source/node-v1"),
    )
    .unwrap()
    .filter_map(|entry| {
        let name = entry.ok()?.file_name().into_string().ok()?;
        name.ends_with(".json").then_some(name)
    })
    .collect::<Vec<_>>();
    on_disk.sort_unstable();
    let mut expected_files = corpus_json.clone();
    expected_files.sort_unstable();
    assert_eq!(
        on_disk, expected_files,
        "every hashed public JSON fixture must have an explicit mapping disposition"
    );

    for (file, stream, expected) in cases {
        let payload = fixture(file);
        match expected {
            Expected::SourceSchemaDrift => {
                let error = parse_node_record(stream, payload.into()).expect_err(file);
                assert!(
                    matches!(error, SourceError::SchemaDrift(_)),
                    "{file} must fail closed as schema drift, not map"
                );
                assert_eq!(error.reason_code(), "source.schema_drift");
            }
            Expected::EmptyCommittedBlock => {
                let record = parse_node_record(stream, payload.into()).unwrap();
                assert_eq!(record.kind(), NodeRecordKind::TransactionBlock);
                let mapped = map_committed_node_v1_block(&record, &committed_context()).unwrap();
                assert!(
                    mapped.events().is_empty(),
                    "{file} must stay an empty committed block"
                );
                assert_eq!(
                    mapped.confirmation_class(),
                    ConfirmationClass::CommittedPrimary
                );
                let auxiliary = map_node_v1_record(&record, &catalog(), &context()).unwrap();
                assert_eq!(
                    auxiliary,
                    MappingDisposition::EvidenceOnly(
                        EvidenceOnlyReason::UnsupportedCanonicalSemantics
                    ),
                    "committed blocks are not mapped through the auxiliary record mapper"
                );
            }
            Expected::MappedTrade => {
                let record = parse_node_record(stream, payload.into()).unwrap();
                let MappingDisposition::Mapped(events) =
                    map_node_v1_record(&record, &catalog(), &context()).unwrap()
                else {
                    panic!("{file} must map as a provisional trade");
                };
                assert_eq!(events.len(), 1);
                assert_eq!(
                    events[0].confirmation_class(),
                    ConfirmationClass::ProvisionalSource
                );
            }
            Expected::EvidenceOnly(reason) => {
                let record = parse_node_record(stream, payload.into()).unwrap();
                let disposition = map_node_v1_record(&record, &catalog(), &context()).unwrap();
                assert_eq!(
                    disposition,
                    MappingDisposition::EvidenceOnly(reason),
                    "{file} must keep an explicit evidence-only disposition"
                );
            }
        }
    }
}

#[test]
fn block_wrapped_one_sided_fill_still_does_not_become_a_trade() {
    let fill: serde_json::Value = serde_json::from_slice(&fixture("fill.json")).unwrap();
    let batched = serde_json::json!({
        "local_time": "2026-07-28T12:00:00",
        "block_time": "2026-07-28T12:00:00",
        "block_number": 42,
        "events": [fill]
    });
    let record = parse_node_record(
        NodeStreamKind::Fills,
        serde_json::to_vec(&batched).unwrap().into(),
    )
    .unwrap();
    assert_eq!(record.block_number(), Some(42));
    assert_eq!(
        map_node_v1_record(&record, &catalog(), &context()).unwrap(),
        MappingDisposition::EvidenceOnly(EvidenceOnlyReason::OneSidedFill)
    );
}

#[test]
fn independent_empty_committed_block_is_not_reconciled_truth() {
    let record = parse_node_record(
        NodeStreamKind::TransactionBlocks,
        fixture("transaction-block.json").into(),
    )
    .unwrap();
    let mut independent = committed_context();
    independent.source_id = SourceId::new("independent-node").unwrap();
    independent.confirmation_class = ConfirmationClass::CommittedIndependent;

    let mapped = map_committed_node_v1_block(&record, &independent).unwrap();
    assert!(mapped.events().is_empty());
    assert_eq!(
        mapped.confirmation_class(),
        ConfirmationClass::CommittedIndependent
    );
    assert_ne!(
        mapped.confirmation_class(),
        ConfirmationClass::ReconciledSnapshot
    );
    assert_eq!(mapped.source_block_hashes().len(), 1);
    assert!(
        mapped
            .source_block_hashes()
            .contains_key(&independent.source_id)
    );
}

#[test]
fn committed_mapper_rejects_non_committed_confirmation_classes() {
    let record = parse_node_record(
        NodeStreamKind::TransactionBlocks,
        fixture("transaction-block.json").into(),
    )
    .unwrap();
    for confirmation_class in [
        ConfirmationClass::ProvisionalSource,
        ConfirmationClass::ReconciledSnapshot,
        ConfirmationClass::Corrected,
        ConfirmationClass::Expired,
    ] {
        let mut context = committed_context();
        context.confirmation_class = confirmation_class;
        let error = map_committed_node_v1_block(&record, &context).unwrap_err();
        assert!(matches!(error, MappingError::InvalidCommittedConfirmation));
        assert_eq!(
            error.reason_code(),
            "canonical_mapping.invalid_committed_confirmation"
        );
    }
}

#[test]
fn trade_transaction_hash_and_positive_fixed_point_values_are_validated() {
    let mut invalid_hash: serde_json::Value =
        serde_json::from_slice(&fixture("trade-batch.json")).expect("batch JSON");
    invalid_hash["events"][0]["hash"] = serde_json::json!("not-a-transaction-hash");
    let record = parse_node_record(
        NodeStreamKind::Trades,
        serde_json::to_vec(&invalid_hash).unwrap().into(),
    )
    .unwrap();
    let error = map_node_v1_record(&record, &catalog(), &context()).unwrap_err();
    assert!(matches!(error, MappingError::InvalidTransactionHash));
    assert_eq!(
        error.reason_code(),
        "canonical_mapping.invalid_transaction_hash"
    );

    for (source_field, canonical_field) in [("px", "price"), ("sz", "quantity")] {
        let mut non_positive: serde_json::Value =
            serde_json::from_slice(&fixture("trade-batch.json")).expect("batch JSON");
        non_positive["events"][0][source_field] = serde_json::json!("0");
        let record = parse_node_record(
            NodeStreamKind::Trades,
            serde_json::to_vec(&non_positive).unwrap().into(),
        )
        .unwrap();
        let error = map_node_v1_record(&record, &catalog(), &context()).unwrap_err();
        assert!(
            matches!(error, MappingError::InvalidDecimal { field: actual, .. } if actual == canonical_field)
        );
        assert_eq!(error.reason_code(), "canonical_mapping.invalid_decimal");
    }
}

#[test]
fn ask_side_swaps_maker_and_taker() {
    let mut batch: serde_json::Value =
        serde_json::from_slice(&fixture("trade-batch.json")).expect("batch JSON");
    batch["events"][0]["side"] = serde_json::json!("A");
    let record = parse_node_record(
        NodeStreamKind::Trades,
        serde_json::to_vec(&batch).unwrap().into(),
    )
    .unwrap();
    let MappingDisposition::Mapped(events) =
        map_node_v1_record(&record, &catalog(), &context()).unwrap()
    else {
        panic!("ask-side trade must map");
    };
    let EventPayload::TradeMatched(trade) = events[0].payload() else {
        panic!("trade");
    };
    assert_eq!(
        trade.maker_order_id.as_ref().unwrap().as_str(),
        "12212201265"
    );
    assert_eq!(
        trade.taker_order_id.as_ref().unwrap().as_str(),
        "12212198275"
    );
}

fn venue_catalog() -> MarketCatalogV1 {
    MarketCatalogV1::try_new(
        "normalized-public-docs-v1",
        [
            ("COMP", MarketId::new("perp:COMP").unwrap()),
            ("INJ", MarketId::new("perp:INJ").unwrap()),
            ("CHILLGUY", MarketId::new("spot:CHILLGUY").unwrap()),
        ],
    )
    .unwrap()
}

fn wrap_events(events: Vec<serde_json::Value>) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "local_time": "2026-07-28T12:00:00",
        "block_time": "2024-07-26T08:31:48.717",
        "block_number": 42,
        "events": events
    }))
    .unwrap()
}

fn wrap_event(event: serde_json::Value) -> Vec<u8> {
    wrap_events(vec![event])
}

fn assemble_mapped_block(
    events: &[CanonicalEventEnvelope],
    record: &hl_protocol::node::v1::NodeRecordV1,
) {
    let first = events.first().expect("mapped batch is non-empty");
    BlockEnvelope::try_new(
        first.chain_id().clone(),
        first.block_height(),
        first.block_time(),
        first.confirmation_class(),
        events.to_vec(),
        BTreeMap::from([(
            context().source_id.clone(),
            *record.content_hash().as_bytes(),
        )]),
    )
    .expect("mapped events must assemble");
}

#[test]
fn batched_canceled_order_maps_the_resting_user() {
    let event: serde_json::Value = serde_json::from_slice(&fixture("order-status.json")).unwrap();
    let record =
        parse_node_record(NodeStreamKind::OrderStatuses, wrap_event(event).into()).unwrap();
    let MappingDisposition::Mapped(events) =
        map_node_v1_record(&record, &venue_catalog(), &context()).unwrap()
    else {
        panic!("batched order status must map");
    };
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].account_addresses()[0].to_api_string(),
        "0xc64cc00b46101bd40aa1c3121195e85c0b0918d8"
    );
    let EventPayload::OrderCancelled(cancelled) = events[0].payload() else {
        panic!("canceled status must map to OrderCancelled");
    };
    assert_eq!(cancelled.order_id.as_str(), "12212359592");
    assert_eq!(cancelled.reason, "canceled");
}

#[test]
fn batched_l4_new_maps_the_resting_user() {
    let event: serde_json::Value = serde_json::from_slice(&fixture("raw-book-diff.json")).unwrap();
    let record = parse_node_record(NodeStreamKind::RawBookDiffs, wrap_event(event).into()).unwrap();
    let MappingDisposition::Mapped(events) =
        map_node_v1_record(&record, &venue_catalog(), &context()).unwrap()
    else {
        panic!("batched l4 diff must map");
    };
    assert_eq!(
        events[0].account_addresses()[0].to_api_string(),
        "0x768484f7e2ebb675c57838366c02ae99ba2a9b08"
    );
    let EventPayload::OrderRested(rested) = events[0].payload() else {
        panic!("new diff must rest");
    };
    assert_eq!(rested.order_id.as_str(), "35061046831");
    assert_eq!(rested.remaining_quantity.to_string(), "186910.0");
}

#[test]
fn every_documented_order_status_maps_when_block_batched() {
    let base: serde_json::Value = serde_json::from_slice(&fixture("order-status.json")).unwrap();
    for name in hl_protocol::node::order_status::ORDER_STATUS_NAMES {
        let mut event = base.clone();
        event["status"] = serde_json::json!(name);
        if *name == "triggered" {
            event["order"]["isTrigger"] = serde_json::json!(true);
            event["order"]["triggerPx"] = serde_json::json!("24.5");
        }
        let record = parse_node_record(NodeStreamKind::OrderStatuses, wrap_event(event).into())
            .unwrap_or_else(|_| panic!("{name} must parse"));
        let MappingDisposition::Mapped(events) =
            map_node_v1_record(&record, &venue_catalog(), &context())
                .unwrap_or_else(|_| panic!("{name} must map"))
        else {
            panic!("{name} must not stay evidence-only when batched");
        };
        assert_eq!(events[0].schema_version(), "1.0.0");
        assert_eq!(
            events[0].confirmation_class(),
            ConfirmationClass::ProvisionalSource
        );
        match *name {
            "open" => assert!(matches!(
                events[0].payload(),
                EventPayload::OrderAccepted(_)
            )),
            "filled" => assert!(matches!(events[0].payload(), EventPayload::OrderFilled(_))),
            "triggered" => {
                assert!(matches!(
                    events[0].payload(),
                    EventPayload::TriggerOrderActivated(_)
                ));
            }
            _ => assert!(matches!(
                events[0].payload(),
                EventPayload::OrderCancelled(_)
            )),
        }
    }
}

#[test]
fn rejected_status_with_canonical_cloid_maps_to_order_rejected() {
    let mut event: serde_json::Value =
        serde_json::from_slice(&fixture("order-status.json")).unwrap();
    event["status"] = serde_json::json!("rejected");
    event["order"]["cloid"] = serde_json::json!("0x11111111111111111111111111111111");
    let record =
        parse_node_record(NodeStreamKind::OrderStatuses, wrap_event(event).into()).unwrap();
    let MappingDisposition::Mapped(events) =
        map_node_v1_record(&record, &venue_catalog(), &context()).unwrap()
    else {
        panic!("rejected with cloid must map");
    };
    let EventPayload::OrderRejected(rejected) = events[0].payload() else {
        panic!("canonical cloid rejection must keep OrderRejected");
    };
    assert_eq!(
        rejected.client_order_id.as_str(),
        "0x11111111111111111111111111111111"
    );
    assert_eq!(rejected.reason_code, "rejected");
}

#[test]
fn batched_l4_update_and_remove_keep_the_resting_user() {
    let event: serde_json::Value = serde_json::from_slice(&fixture("raw-book-diff.json")).unwrap();
    let mut update = event.clone();
    update["raw_book_diff"] = serde_json::json!({ "update": { "sz": "100.0" } });
    let record =
        parse_node_record(NodeStreamKind::RawBookDiffs, wrap_event(update).into()).unwrap();
    let MappingDisposition::Mapped(events) =
        map_node_v1_record(&record, &venue_catalog(), &context()).unwrap()
    else {
        panic!("update must map");
    };
    let EventPayload::OrderRested(rested) = events[0].payload() else {
        panic!("update rests");
    };
    assert_eq!(rested.remaining_quantity.to_string(), "100.0");

    let mut remove = event;
    remove["raw_book_diff"] = serde_json::json!("remove");
    let record =
        parse_node_record(NodeStreamKind::RawBookDiffs, wrap_event(remove).into()).unwrap();
    let MappingDisposition::Mapped(events) =
        map_node_v1_record(&record, &venue_catalog(), &context()).unwrap()
    else {
        panic!("remove must map");
    };
    let EventPayload::OrderCancelled(cancelled) = events[0].payload() else {
        panic!("remove cancels");
    };
    assert_eq!(cancelled.reason, "raw_book_diff_remove");
    assert_eq!(cancelled.remaining_quantity.to_string(), "0");
}

#[test]
fn batched_l4_new_then_update_same_oid_get_distinct_event_ids() {
    let event: serde_json::Value = serde_json::from_slice(&fixture("raw-book-diff.json")).unwrap();
    let mut update = event.clone();
    update["raw_book_diff"] = serde_json::json!({ "update": { "sz": "100.0" } });
    let record = parse_node_record(
        NodeStreamKind::RawBookDiffs,
        wrap_events(vec![event, update]).into(),
    )
    .unwrap();
    let MappingDisposition::Mapped(events) =
        map_node_v1_record(&record, &venue_catalog(), &context()).unwrap()
    else {
        panic!("batched l4 new+update must map");
    };
    assert_eq!(events.len(), 2);
    assert_eq!(
        events[0].transaction_id().as_str(),
        events[1].transaction_id().as_str()
    );
    assert_eq!(events[0].transaction_index(), 0);
    assert_eq!(events[1].transaction_index(), 0);
    assert_eq!(events[0].canonical_event_index(), 0);
    assert_eq!(events[1].canonical_event_index(), 1);
    assert!(matches!(events[0].payload(), EventPayload::OrderRested(_)));
    assert!(matches!(events[1].payload(), EventPayload::OrderRested(_)));
    assert_ne!(events[0].event_id(), events[1].event_id());
    assemble_mapped_block(&events, &record);
}

#[test]
fn batched_order_status_same_oid_same_kind_get_distinct_event_ids() {
    let event: serde_json::Value = serde_json::from_slice(&fixture("order-status.json")).unwrap();
    let mut second = event.clone();
    second["order"]["sz"] = serde_json::json!("100.0");
    let record = parse_node_record(
        NodeStreamKind::OrderStatuses,
        wrap_events(vec![event, second]).into(),
    )
    .unwrap();
    let MappingDisposition::Mapped(events) =
        map_node_v1_record(&record, &venue_catalog(), &context()).unwrap()
    else {
        panic!("batched order statuses must map");
    };
    assert_eq!(events.len(), 2);
    assert_eq!(
        events[0].transaction_id().as_str(),
        events[1].transaction_id().as_str()
    );
    assert_eq!(events[0].transaction_index(), 0);
    assert_eq!(events[1].transaction_index(), 0);
    assert_eq!(events[0].canonical_event_index(), 0);
    assert_eq!(events[1].canonical_event_index(), 1);
    assert!(matches!(
        events[0].payload(),
        EventPayload::OrderCancelled(_)
    ));
    assert!(matches!(
        events[1].payload(),
        EventPayload::OrderCancelled(_)
    ));
    assert_ne!(events[0].event_id(), events[1].event_id());
    assemble_mapped_block(&events, &record);
}

#[test]
fn batched_order_status_distinct_oids_assemble_a_block() {
    let event: serde_json::Value = serde_json::from_slice(&fixture("order-status.json")).unwrap();
    let mut other = event.clone();
    other["order"]["oid"] = serde_json::json!(12_212_359_593_u64);
    let record = parse_node_record(
        NodeStreamKind::OrderStatuses,
        wrap_events(vec![event, other]).into(),
    )
    .unwrap();
    let MappingDisposition::Mapped(events) =
        map_node_v1_record(&record, &venue_catalog(), &context()).unwrap()
    else {
        panic!("distinct-oid order statuses must map");
    };
    assert_eq!(events.len(), 2);
    assert_ne!(
        events[0].transaction_id().as_str(),
        events[1].transaction_id().as_str()
    );
    assert_eq!(events[0].transaction_index(), 0);
    assert_eq!(events[0].canonical_event_index(), 0);
    assert_eq!(events[1].transaction_index(), 1);
    assert_eq!(events[1].canonical_event_index(), 0);
    assert_ne!(events[0].event_id(), events[1].event_id());
    assemble_mapped_block(&events, &record);
}

#[test]
fn batched_l4_distinct_oids_assemble_a_block() {
    let event: serde_json::Value = serde_json::from_slice(&fixture("raw-book-diff.json")).unwrap();
    let mut other = event.clone();
    other["oid"] = serde_json::json!(35_061_046_832_u64);
    let record = parse_node_record(
        NodeStreamKind::RawBookDiffs,
        wrap_events(vec![event, other]).into(),
    )
    .unwrap();
    let MappingDisposition::Mapped(events) =
        map_node_v1_record(&record, &venue_catalog(), &context()).unwrap()
    else {
        panic!("distinct-oid l4 diffs must map");
    };
    assert_eq!(events.len(), 2);
    assert_ne!(
        events[0].transaction_id().as_str(),
        events[1].transaction_id().as_str()
    );
    assert_eq!(events[0].transaction_index(), 0);
    assert_eq!(events[0].canonical_event_index(), 0);
    assert_eq!(events[1].transaction_index(), 1);
    assert_eq!(events[1].canonical_event_index(), 0);
    assert_ne!(events[0].event_id(), events[1].event_id());
    assemble_mapped_block(&events, &record);
}

#[test]
fn batched_order_status_interleaved_oids_fail_closed_as_non_contiguous() {
    let event: serde_json::Value = serde_json::from_slice(&fixture("order-status.json")).unwrap();
    let mut other = event.clone();
    other["order"]["oid"] = serde_json::json!(12_212_359_593_u64);
    let record = parse_node_record(
        NodeStreamKind::OrderStatuses,
        wrap_events(vec![event.clone(), other, event]).into(),
    )
    .unwrap();
    let error = map_node_v1_record(&record, &venue_catalog(), &context()).unwrap_err();
    assert!(matches!(error, MappingError::NonContiguousTransaction));
    assert_eq!(
        error.reason_code(),
        "canonical_mapping.non_contiguous_transaction"
    );
}

#[test]
fn batched_l4_interleaved_oids_fail_closed_as_non_contiguous() {
    let event: serde_json::Value = serde_json::from_slice(&fixture("raw-book-diff.json")).unwrap();
    let mut other = event.clone();
    other["oid"] = serde_json::json!(35_061_046_832_u64);
    let record = parse_node_record(
        NodeStreamKind::RawBookDiffs,
        wrap_events(vec![event.clone(), other, event]).into(),
    )
    .unwrap();
    let error = map_node_v1_record(&record, &venue_catalog(), &context()).unwrap_err();
    assert!(matches!(error, MappingError::NonContiguousTransaction));
    assert_eq!(
        error.reason_code(),
        "canonical_mapping.non_contiguous_transaction"
    );
}
