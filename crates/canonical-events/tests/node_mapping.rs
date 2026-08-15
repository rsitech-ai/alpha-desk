use std::fs;
use std::path::Path;

use canonical_events::{
    CommittedNodeV1MappingContext, ConfirmationClass, EventPayload, EvidenceOnlyReason,
    MappingDisposition, MappingError, MarketCatalogV1, NodeV1MappingContext,
    TradeParticipantRoleV1, map_committed_node_v1_block, map_node_v1_record,
};
use domain_types::{BlockHeight, ChainId, KnownTime, MarketId, SourceId};
use hl_protocol::node::v1::{NodeStreamKind, parse_node_record};

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
fn complete_block_batched_trade_maps_without_inventing_maker_taker_semantics() {
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
    assert_eq!(trade.maker_order_id, None);
    assert_eq!(trade.taker_order_id, None);
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
fn documented_trade_optionals_are_retained_without_inventing_maker_taker() {
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
    assert_eq!(trade.maker_order_id, None);
    assert_eq!(trade.taker_order_id, None);
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
fn known_but_unmapped_node_record_has_an_explicit_disposition() {
    let record = parse_node_record(NodeStreamKind::Fills, fixture("fill.json").into()).unwrap();

    assert_eq!(
        map_node_v1_record(&record, &catalog(), &context()).unwrap(),
        MappingDisposition::EvidenceOnly(EvidenceOnlyReason::UnsupportedCanonicalSemantics)
    );
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
