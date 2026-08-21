use std::collections::{BTreeMap, BTreeSet};

use canonical_events::{
    AssetContextUpdated, BlockEnvelope, CanonicalEventEnvelope, CanonicalEventInput,
    ConfirmationClass, DexCreated, EventKind, EventPayload, FundingRateUpdated, MarginTableChanged,
    MarketCreated, MarketHalted, MarketMetadataChanged, MarketResumed, OpenInterestCapChanged,
    OracleUpdated, OutcomeCreated, OutcomeResolved, SourceEvidence,
};
use canonical_ledger::{
    ApplyOutcome, AssetContextCurrentRecordV1, CanonicalLedger, CanonicalMarketReducerV1,
    DexCurrentRecordV1, EventReducer, LedgerLimits, MarketCurrentRecordV1, MarketFactRecordV1,
    MarketMetadataResolutionV1, MarketMetadataVersionRecordV1, MarketStateError, MarketStatusV1,
    OutcomeCurrentRecordV1, StateKey,
};
use domain_types::{
    Address, AssetId, BlockHeight, ChainId, DexId, EventId, FundingRate, KnownTime, MarketId,
    OutcomeId, Price, ProtocolTime, Quantity, QuoteAmount, SourceId, TransactionId,
};

const OPERATOR: Address = Address::from_bytes([0x11; 20]);

#[test]
fn exact_market_creation_requires_dex_and_assets_and_is_key_bound() {
    let dex_id = DexId::new("validator").unwrap();
    let base = AssetId::new("BTC").unwrap();
    let quote = AssetId::new("USDC").unwrap();
    let market = MarketId::new("perp:BTC").unwrap();
    let events = vec![
        event(
            100,
            0,
            EventPayload::DexCreated(DexCreated {
                dex_id: dex_id.clone(),
                name: "Validator Perpetuals".to_owned(),
                operator_account_id: OPERATOR,
            }),
            Vec::new(),
            vec![OPERATOR],
            "1.0.0",
        ),
        event(
            100,
            1,
            EventPayload::AssetContextUpdated(AssetContextUpdated {
                asset_id: base.clone(),
                context_version: "btc-v1".to_owned(),
                context_hash: [0x21; 32],
            }),
            Vec::new(),
            Vec::new(),
            "1.0.0",
        ),
        event(
            100,
            2,
            EventPayload::AssetContextUpdated(AssetContextUpdated {
                asset_id: quote.clone(),
                context_version: "usdc-v1".to_owned(),
                context_hash: [0x22; 32],
            }),
            Vec::new(),
            Vec::new(),
            "1.0.0",
        ),
        event(
            100,
            3,
            EventPayload::MarketCreated(MarketCreated {
                market_id: market.clone(),
                dex_id: dex_id.clone(),
                base_asset_id: base.clone(),
                quote_asset_id: quote.clone(),
                tick_size: Price::parse_at_scale("0.1", 6).unwrap(),
                lot_size: Quantity::parse_at_scale("0.00001", 8).unwrap(),
            }),
            vec![market.clone()],
            Vec::new(),
            "1.0.0",
        ),
    ];
    let mut ledger = ledger(100);

    let ApplyOutcome::Applied(delta) = ledger.apply_block(&block(100, events)).unwrap() else {
        panic!("new block must apply");
    };
    assert_eq!(delta.mutations().len(), 9);

    let dex_key = DexCurrentRecordV1::state_key(&dex_id).unwrap();
    let dex = DexCurrentRecordV1::decode_at(
        &dex_key,
        ledger.state_image().entries().get(&dex_key).unwrap(),
    )
    .unwrap();
    assert_eq!(dex.dex_id(), &dex_id);
    assert_eq!(dex.operator_account_id(), OPERATOR);

    for asset in [&base, &quote] {
        let key = AssetContextCurrentRecordV1::state_key(asset).unwrap();
        assert!(
            AssetContextCurrentRecordV1::decode_at(
                &key,
                ledger.state_image().entries().get(&key).unwrap(),
            )
            .is_ok()
        );
    }

    let market_key = MarketCurrentRecordV1::state_key(&market).unwrap();
    let current = MarketCurrentRecordV1::decode_at(
        &market_key,
        ledger.state_image().entries().get(&market_key).unwrap(),
    )
    .unwrap();
    assert_eq!(current.market_id(), &market);
    assert_eq!(current.status(), MarketStatusV1::Active);
    assert_eq!(
        current.metadata_resolution(),
        MarketMetadataResolutionV1::Exact
    );
    assert_eq!(current.price_scale(), Some(6));
    assert_eq!(current.quantity_scale(), Some(8));
    let policy = current.l2_reconcile_policy_v1().unwrap();
    assert_eq!(policy.version(), orderbook::L2_RECONCILE_POLICY_V1);
    assert_eq!(policy.tick_size(), current.tick_size());
    assert_eq!(policy.lot_size(), current.lot_size());

    let version_key =
        MarketMetadataVersionRecordV1::state_key(&market, current.metadata_version()).unwrap();
    let version = MarketMetadataVersionRecordV1::decode_at(
        &version_key,
        ledger.state_image().entries().get(&version_key).unwrap(),
    )
    .unwrap();
    assert_eq!(version.effective_from_block(), BlockHeight::new(100));
    assert_eq!(version.effective_until_block(), None);
    assert_eq!(version.resolution(), MarketMetadataResolutionV1::Exact);
}

#[test]
fn hash_only_metadata_change_closes_exact_interval_and_suppresses_values() {
    let market = MarketId::new("perp:BTC").unwrap();
    let mut ledger = seeded_market(200, &market);
    let before = ledger.state_hash();

    ledger
        .apply_block(&block(
            201,
            vec![event(
                201,
                0,
                EventPayload::MarketMetadataChanged(MarketMetadataChanged {
                    market_id: market.clone(),
                    metadata_version: "metadata-v2".to_owned(),
                    metadata_hash: [0x33; 32],
                }),
                vec![market.clone()],
                Vec::new(),
                "1.0.0",
            )],
        ))
        .unwrap();
    assert_ne!(ledger.state_hash(), before);

    let market_key = MarketCurrentRecordV1::state_key(&market).unwrap();
    let current = MarketCurrentRecordV1::decode_at(
        &market_key,
        ledger.state_image().entries().get(&market_key).unwrap(),
    )
    .unwrap();
    assert_eq!(
        current.metadata_resolution(),
        MarketMetadataResolutionV1::Unresolved
    );
    assert_eq!(current.metadata_version(), "metadata-v2");
    assert_eq!(current.tick_size(), None);
    assert_eq!(current.lot_size(), None);
    assert_eq!(current.price_scale(), None);
    assert_eq!(current.quantity_scale(), None);

    let prior_key = MarketMetadataVersionRecordV1::state_key(&market, "creation@1.0.0").unwrap();
    let prior = MarketMetadataVersionRecordV1::decode_at(
        &prior_key,
        ledger.state_image().entries().get(&prior_key).unwrap(),
    )
    .unwrap();
    assert_eq!(prior.effective_until_block(), Some(BlockHeight::new(200)));

    let state_before_rejection = ledger.state_image().canonical_bytes();
    let error = ledger
        .apply_block(&block(
            202,
            vec![event(
                202,
                0,
                EventPayload::OracleUpdated(OracleUpdated {
                    market_id: market.clone(),
                    oracle_price: Price::parse_at_scale("65000", 6).unwrap(),
                    source: "validator".to_owned(),
                    effective_at: ProtocolTime::from_unix_micros(202).unwrap(),
                }),
                vec![market],
                Vec::new(),
                "1.0.0",
            )],
        ))
        .unwrap_err();
    assert_eq!(error.reason_code(), "ledger.reducer_failed");
    assert_eq!(
        error.reducer_reason_code(),
        Some("market_state.metadata_unresolved")
    );
    assert_eq!(
        ledger.state_image().canonical_bytes(),
        state_before_rejection
    );
}

#[test]
fn all_twelve_market_events_reduce_in_valid_order() {
    let market = MarketId::new("perp:BTC").unwrap();
    let outcome = OutcomeId::new("up").unwrap();
    let mut ledger = seeded_market(300, &market);
    let transitions = vec![
        EventPayload::MarketHalted(MarketHalted {
            market_id: market.clone(),
            reason: "maintenance".to_owned(),
        }),
        EventPayload::MarketResumed(MarketResumed {
            market_id: market.clone(),
            reason: "maintenance complete".to_owned(),
        }),
        EventPayload::OpenInterestCapChanged(OpenInterestCapChanged {
            market_id: market.clone(),
            previous_cap: QuoteAmount::parse_at_scale("1000000", 6).unwrap(),
            new_cap: QuoteAmount::parse_at_scale("1250000", 6).unwrap(),
        }),
        EventPayload::MarginTableChanged(MarginTableChanged {
            market_id: market.clone(),
            previous_table_hash: "margin-v1".to_owned(),
            new_table_hash: "margin-v2".to_owned(),
        }),
        EventPayload::OracleUpdated(OracleUpdated {
            market_id: market.clone(),
            oracle_price: Price::parse_at_scale("65000", 6).unwrap(),
            source: "validator".to_owned(),
            effective_at: ProtocolTime::from_unix_micros(305).unwrap(),
        }),
        EventPayload::FundingRateUpdated(FundingRateUpdated {
            market_id: market.clone(),
            funding_rate: FundingRate::parse_at_scale("0.0001", 8).unwrap(),
            effective_at: ProtocolTime::from_unix_micros(306).unwrap(),
        }),
        EventPayload::OutcomeCreated(OutcomeCreated {
            market_id: market.clone(),
            outcome_id: outcome.clone(),
            description: "BTC closes higher".to_owned(),
        }),
        EventPayload::OutcomeResolved(OutcomeResolved {
            market_id: market.clone(),
            outcome_id: outcome.clone(),
            settlement_value: Price::parse_at_scale("1", 6).unwrap(),
            resolved_at: ProtocolTime::from_unix_micros(308).unwrap(),
        }),
        EventPayload::MarketMetadataChanged(MarketMetadataChanged {
            market_id: market.clone(),
            metadata_version: "metadata-v2".to_owned(),
            metadata_hash: [0x41; 32],
        }),
    ];
    for (offset, payload) in transitions.into_iter().enumerate() {
        let height = 301 + u64::try_from(offset).unwrap();
        ledger
            .apply_block(&block(
                height,
                vec![event(
                    height,
                    0,
                    payload,
                    vec![market.clone()],
                    Vec::new(),
                    "1.0.0",
                )],
            ))
            .unwrap();
    }

    assert_eq!(namespace_count(&ledger, "market-fact.v1"), 13);
    let actual_kinds = ledger
        .state_image()
        .entries()
        .iter()
        .filter(|(key, _)| key.namespace() == "market-fact.v1")
        .map(|(key, bytes)| {
            MarketFactRecordV1::decode_at(key, bytes)
                .unwrap()
                .event_kind()
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        actual_kinds,
        BTreeSet::from([
            EventKind::DexCreated,
            EventKind::AssetContextUpdated,
            EventKind::MarketCreated,
            EventKind::MarketMetadataChanged,
            EventKind::MarketHalted,
            EventKind::MarketResumed,
            EventKind::OpenInterestCapChanged,
            EventKind::MarginTableChanged,
            EventKind::OracleUpdated,
            EventKind::FundingRateUpdated,
            EventKind::OutcomeCreated,
            EventKind::OutcomeResolved,
        ])
    );
    let current = current_market(&ledger, &market);
    assert_eq!(current.status(), MarketStatusV1::Active);
    assert_eq!(
        current.metadata_resolution(),
        MarketMetadataResolutionV1::Unresolved
    );
    assert_eq!(current.open_interest_cap(), None);
    assert_eq!(current.margin_table_hash(), None);
    assert_eq!(current.oracle_price(), None);
    assert_eq!(current.funding_rate(), None);

    let outcome_key = OutcomeCurrentRecordV1::state_key(&market, &outcome).unwrap();
    let outcome_record = OutcomeCurrentRecordV1::decode_at(
        &outcome_key,
        ledger.state_image().entries().get(&outcome_key).unwrap(),
    )
    .unwrap();
    assert_eq!(
        outcome_record.settlement_value(),
        Some(Price::parse_at_scale("1", 6).unwrap())
    );
    assert_eq!(
        outcome_record.resolved_at(),
        Some(ProtocolTime::from_unix_micros(308).unwrap())
    );
}

#[test]
fn unresolved_current_getters_hide_all_prior_value_dependent_state() {
    let market = MarketId::new("perp:BTC").unwrap();
    let mut ledger = seeded_market(350, &market);
    let payloads = vec![
        EventPayload::OpenInterestCapChanged(OpenInterestCapChanged {
            market_id: market.clone(),
            previous_cap: QuoteAmount::parse_at_scale("10", 6).unwrap(),
            new_cap: QuoteAmount::parse_at_scale("20", 6).unwrap(),
        }),
        EventPayload::MarginTableChanged(MarginTableChanged {
            market_id: market.clone(),
            previous_table_hash: "margin-v1".to_owned(),
            new_table_hash: "margin-v2".to_owned(),
        }),
        EventPayload::OracleUpdated(OracleUpdated {
            market_id: market.clone(),
            oracle_price: Price::parse_at_scale("65000", 6).unwrap(),
            source: "validator".to_owned(),
            effective_at: ProtocolTime::from_unix_micros(353).unwrap(),
        }),
        EventPayload::FundingRateUpdated(FundingRateUpdated {
            market_id: market.clone(),
            funding_rate: FundingRate::parse_at_scale("0.0001", 8).unwrap(),
            effective_at: ProtocolTime::from_unix_micros(354).unwrap(),
        }),
        EventPayload::MarketMetadataChanged(MarketMetadataChanged {
            market_id: market.clone(),
            metadata_version: "metadata-v2".to_owned(),
            metadata_hash: [0x42; 32],
        }),
    ];
    for (offset, payload) in payloads.into_iter().enumerate() {
        let height = 351 + u64::try_from(offset).unwrap();
        ledger
            .apply_block(&single_market_block(height, &market, payload))
            .unwrap();
    }

    let current = current_market(&ledger, &market);
    assert_eq!(
        current.metadata_resolution(),
        MarketMetadataResolutionV1::Unresolved
    );
    assert_eq!(current.tick_size(), None);
    assert_eq!(current.lot_size(), None);
    assert_eq!(current.price_scale(), None);
    assert_eq!(current.quantity_scale(), None);
    assert_eq!(current.open_interest_cap(), None);
    assert_eq!(current.margin_table_hash(), None);
    assert_eq!(current.oracle_price(), None);
    assert_eq!(current.oracle_source(), None);
    assert_eq!(current.oracle_effective_at(), None);
    assert_eq!(current.funding_rate(), None);
    assert_eq!(current.funding_effective_at(), None);
}

#[test]
fn identities_prerequisites_and_collisions_default_deny() {
    let market = MarketId::new("perp:BTC").unwrap();
    let mut missing = ledger(400);
    let missing_error = missing
        .apply_block(&block(
            400,
            vec![event(
                400,
                0,
                EventPayload::MarketCreated(MarketCreated {
                    market_id: market.clone(),
                    dex_id: DexId::new("missing").unwrap(),
                    base_asset_id: AssetId::new("BTC").unwrap(),
                    quote_asset_id: AssetId::new("USDC").unwrap(),
                    tick_size: Price::parse_at_scale("0.1", 6).unwrap(),
                    lot_size: Quantity::parse_at_scale("0.001", 8).unwrap(),
                }),
                vec![market.clone()],
                Vec::new(),
                "1.0.0",
            )],
        ))
        .unwrap_err();
    assert_eq!(
        missing_error.reducer_reason_code(),
        Some("market_state.missing_dex")
    );

    let mut seeded = seeded_market(400, &market);
    let wrong_market = MarketId::new("perp:ETH").unwrap();
    let identity_error = seeded
        .apply_block(&block(
            401,
            vec![event(
                401,
                0,
                EventPayload::MarketHalted(MarketHalted {
                    market_id: market.clone(),
                    reason: "maintenance".to_owned(),
                }),
                vec![wrong_market],
                Vec::new(),
                "1.0.0",
            )],
        ))
        .unwrap_err();
    assert_eq!(
        identity_error.reducer_reason_code(),
        Some("market_state.invalid_market_identity")
    );

    let collision = seeded
        .apply_block(&block(
            401,
            vec![event(
                401,
                0,
                EventPayload::OutcomeCreated(OutcomeCreated {
                    market_id: market.clone(),
                    outcome_id: OutcomeId::new("up").unwrap(),
                    description: "first".to_owned(),
                }),
                vec![market.clone()],
                Vec::new(),
                "1.0.0",
            )],
        ))
        .unwrap();
    assert!(matches!(collision, ApplyOutcome::Applied(_)));
    let collision_error = seeded
        .apply_block(&block(
            402,
            vec![event(
                402,
                0,
                EventPayload::OutcomeCreated(OutcomeCreated {
                    market_id: market.clone(),
                    outcome_id: OutcomeId::new("up").unwrap(),
                    description: "second".to_owned(),
                }),
                vec![market],
                Vec::new(),
                "1.0.0",
            )],
        ))
        .unwrap_err();
    assert_eq!(
        collision_error.reducer_reason_code(),
        Some("market_state.outcome_identity_collision")
    );
}

#[test]
fn dex_asset_market_collisions_and_envelope_accounts_fail_closed() {
    let other_operator = Address::from_bytes([0x12; 20]);
    let dex = DexId::new("validator").unwrap();
    let dex_payload = EventPayload::DexCreated(DexCreated {
        dex_id: dex.clone(),
        name: "Validator Perpetuals".to_owned(),
        operator_account_id: OPERATOR,
    });

    let mut wrong_operator = ledger(450);
    let before = wrong_operator.state_image().canonical_bytes();
    let error = wrong_operator
        .apply_block(&block(
            450,
            vec![event(
                450,
                0,
                dex_payload.clone(),
                Vec::new(),
                vec![other_operator],
                "1.0.0",
            )],
        ))
        .unwrap_err();
    assert_eq!(
        error.reducer_reason_code(),
        Some("market_state.invalid_account_identity")
    );
    assert_eq!(wrong_operator.state_image().canonical_bytes(), before);

    let mut duplicate_dex = ledger(450);
    duplicate_dex
        .apply_block(&block(
            450,
            vec![event(
                450,
                0,
                dex_payload.clone(),
                Vec::new(),
                vec![OPERATOR],
                "1.0.0",
            )],
        ))
        .unwrap();
    let error = duplicate_dex
        .apply_block(&block(
            451,
            vec![event(
                451,
                0,
                dex_payload,
                Vec::new(),
                vec![OPERATOR],
                "1.0.0",
            )],
        ))
        .unwrap_err();
    assert_eq!(
        error.reducer_reason_code(),
        Some("market_state.dex_identity_collision")
    );

    let asset = AssetId::new("BTC").unwrap();
    let asset_payload = EventPayload::AssetContextUpdated(AssetContextUpdated {
        asset_id: asset,
        context_version: "btc-v1".to_owned(),
        context_hash: [0x51; 32],
    });
    let mut duplicate_asset = ledger(450);
    duplicate_asset
        .apply_block(&block(
            450,
            vec![event(
                450,
                0,
                asset_payload.clone(),
                Vec::new(),
                Vec::new(),
                "1.0.0",
            )],
        ))
        .unwrap();
    let error = duplicate_asset
        .apply_block(&block(
            451,
            vec![event(
                451,
                0,
                asset_payload,
                Vec::new(),
                Vec::new(),
                "1.0.0",
            )],
        ))
        .unwrap_err();
    assert_eq!(
        error.reducer_reason_code(),
        Some("market_state.asset_identity_collision")
    );

    let market = MarketId::new("perp:BTC").unwrap();
    let mut duplicate_market = seeded_market(450, &market);
    let error = duplicate_market
        .apply_block(&single_market_block(
            451,
            &market,
            EventPayload::MarketCreated(MarketCreated {
                market_id: market.clone(),
                dex_id: dex,
                base_asset_id: AssetId::new("BTC").unwrap(),
                quote_asset_id: AssetId::new("USDC").unwrap(),
                tick_size: Price::parse_at_scale("0.1", 6).unwrap(),
                lot_size: Quantity::parse_at_scale("0.00001", 8).unwrap(),
            }),
        ))
        .unwrap_err();
    assert_eq!(
        error.reducer_reason_code(),
        Some("market_state.market_identity_collision")
    );

    let error = duplicate_market
        .apply_block(&block(
            451,
            vec![event(
                451,
                0,
                EventPayload::MarketHalted(MarketHalted {
                    market_id: market.clone(),
                    reason: "maintenance".to_owned(),
                }),
                vec![market],
                vec![OPERATOR],
                "1.0.0",
            )],
        ))
        .unwrap_err();
    assert_eq!(
        error.reducer_reason_code(),
        Some("market_state.invalid_account_identity")
    );
}

#[test]
fn invalid_halt_resume_and_outcome_transitions_are_rejected() {
    let market = MarketId::new("perp:BTC").unwrap();
    let mut active = seeded_market(500, &market);
    let resume_error = active
        .apply_block(&single_market_block(
            501,
            &market,
            EventPayload::MarketResumed(MarketResumed {
                market_id: market.clone(),
                reason: "not halted".to_owned(),
            }),
        ))
        .unwrap_err();
    assert_eq!(
        resume_error.reducer_reason_code(),
        Some("market_state.invalid_status_transition")
    );

    let mut halted = seeded_market(500, &market);
    halted
        .apply_block(&single_market_block(
            501,
            &market,
            EventPayload::MarketHalted(MarketHalted {
                market_id: market.clone(),
                reason: "maintenance".to_owned(),
            }),
        ))
        .unwrap();
    let halt_error = halted
        .apply_block(&single_market_block(
            502,
            &market,
            EventPayload::MarketHalted(MarketHalted {
                market_id: market.clone(),
                reason: "again".to_owned(),
            }),
        ))
        .unwrap_err();
    assert_eq!(
        halt_error.reducer_reason_code(),
        Some("market_state.invalid_status_transition")
    );

    let missing_outcome_error = active
        .apply_block(&single_market_block(
            501,
            &market,
            EventPayload::OutcomeResolved(OutcomeResolved {
                market_id: market.clone(),
                outcome_id: OutcomeId::new("missing").unwrap(),
                settlement_value: Price::parse_at_scale("1", 6).unwrap(),
                resolved_at: ProtocolTime::from_unix_micros(501).unwrap(),
            }),
        ))
        .unwrap_err();
    assert_eq!(
        missing_outcome_error.reducer_reason_code(),
        Some("market_state.missing_outcome")
    );
}

#[test]
fn previous_cap_and_margin_table_must_match_current_values() {
    let market = MarketId::new("perp:BTC").unwrap();
    let mut ledger = seeded_market(600, &market);
    ledger
        .apply_block(&single_market_block(
            601,
            &market,
            EventPayload::OpenInterestCapChanged(OpenInterestCapChanged {
                market_id: market.clone(),
                previous_cap: QuoteAmount::parse_at_scale("10", 6).unwrap(),
                new_cap: QuoteAmount::parse_at_scale("20", 6).unwrap(),
            }),
        ))
        .unwrap();
    let cap_error = ledger
        .apply_block(&single_market_block(
            602,
            &market,
            EventPayload::OpenInterestCapChanged(OpenInterestCapChanged {
                market_id: market.clone(),
                previous_cap: QuoteAmount::parse_at_scale("19", 6).unwrap(),
                new_cap: QuoteAmount::parse_at_scale("30", 6).unwrap(),
            }),
        ))
        .unwrap_err();
    assert_eq!(
        cap_error.reducer_reason_code(),
        Some("market_state.previous_cap_mismatch")
    );

    let mut margin = seeded_market(600, &market);
    margin
        .apply_block(&single_market_block(
            601,
            &market,
            EventPayload::MarginTableChanged(MarginTableChanged {
                market_id: market.clone(),
                previous_table_hash: "v1".to_owned(),
                new_table_hash: "v2".to_owned(),
            }),
        ))
        .unwrap();
    let table_error = margin
        .apply_block(&single_market_block(
            602,
            &market,
            EventPayload::MarginTableChanged(MarginTableChanged {
                market_id: market.clone(),
                previous_table_hash: "not-v2".to_owned(),
                new_table_hash: "v3".to_owned(),
            }),
        ))
        .unwrap_err();
    assert_eq!(
        table_error.reducer_reason_code(),
        Some("market_state.previous_margin_table_mismatch")
    );
}

#[test]
fn oracle_and_funding_effective_times_never_regress() {
    let market = MarketId::new("perp:BTC").unwrap();
    let mut oracle = seeded_market(700, &market);
    oracle
        .apply_block(&single_market_block(
            701,
            &market,
            EventPayload::OracleUpdated(OracleUpdated {
                market_id: market.clone(),
                oracle_price: Price::parse_at_scale("65000", 6).unwrap(),
                source: "validator".to_owned(),
                effective_at: ProtocolTime::from_unix_micros(1000).unwrap(),
            }),
        ))
        .unwrap();
    let oracle_error = oracle
        .apply_block(&single_market_block(
            702,
            &market,
            EventPayload::OracleUpdated(OracleUpdated {
                market_id: market.clone(),
                oracle_price: Price::parse_at_scale("65001", 6).unwrap(),
                source: "validator".to_owned(),
                effective_at: ProtocolTime::from_unix_micros(999).unwrap(),
            }),
        ))
        .unwrap_err();
    assert_eq!(
        oracle_error.reducer_reason_code(),
        Some("market_state.stale_oracle_time")
    );

    let mut funding = seeded_market(700, &market);
    funding
        .apply_block(&single_market_block(
            701,
            &market,
            EventPayload::FundingRateUpdated(FundingRateUpdated {
                market_id: market.clone(),
                funding_rate: FundingRate::parse_at_scale("0.0001", 8).unwrap(),
                effective_at: ProtocolTime::from_unix_micros(1000).unwrap(),
            }),
        ))
        .unwrap();
    let funding_error = funding
        .apply_block(&single_market_block(
            702,
            &market,
            EventPayload::FundingRateUpdated(FundingRateUpdated {
                market_id: market.clone(),
                funding_rate: FundingRate::parse_at_scale("0.0002", 8).unwrap(),
                effective_at: ProtocolTime::from_unix_micros(999).unwrap(),
            }),
        ))
        .unwrap_err();
    assert_eq!(
        funding_error.reducer_reason_code(),
        Some("market_state.stale_funding_time")
    );
}

#[test]
fn late_invalid_event_rolls_back_the_whole_block() {
    let market = MarketId::new("perp:BTC").unwrap();
    let mut ledger = seeded_market(800, &market);
    let before = ledger.state_image().canonical_bytes();
    let error = ledger
        .apply_block(&block(
            801,
            vec![
                event(
                    801,
                    0,
                    EventPayload::MarketHalted(MarketHalted {
                        market_id: market.clone(),
                        reason: "maintenance".to_owned(),
                    }),
                    vec![market.clone()],
                    Vec::new(),
                    "1.0.0",
                ),
                event(
                    801,
                    1,
                    EventPayload::MarketHalted(MarketHalted {
                        market_id: market.clone(),
                        reason: "duplicate".to_owned(),
                    }),
                    vec![market],
                    Vec::new(),
                    "1.0.0",
                ),
            ],
        ))
        .unwrap_err();
    assert_eq!(
        error.reducer_reason_code(),
        Some("market_state.invalid_status_transition")
    );
    assert_eq!(ledger.state_image().canonical_bytes(), before);
}

#[test]
fn unsupported_schema_is_default_denied() {
    let market = MarketId::new("perp:BTC").unwrap();
    let mut ledger = seeded_market(900, &market);
    let unsupported = event(
        901,
        0,
        EventPayload::MarketHalted(MarketHalted {
            market_id: market.clone(),
            reason: "maintenance".to_owned(),
        }),
        vec![market],
        Vec::new(),
        "1.1.0",
    );
    assert!(!CanonicalMarketReducerV1.supports(&unsupported));
    let error = ledger
        .apply_block(&block(901, vec![unsupported]))
        .unwrap_err();
    assert_eq!(error.reason_code(), "ledger.unsupported_event");
}

#[test]
fn every_record_codec_is_canonical_bounded_unknown_field_denying_and_key_bound() {
    let market = MarketId::new("perp:BTC").unwrap();
    let outcome = OutcomeId::new("up").unwrap();
    let mut ledger = seeded_market(1000, &market);
    ledger
        .apply_block(&single_market_block(
            1001,
            &market,
            EventPayload::OutcomeCreated(OutcomeCreated {
                market_id: market.clone(),
                outcome_id: outcome.clone(),
                description: "BTC closes higher".to_owned(),
            }),
        ))
        .unwrap();

    let (fact_key, fact_bytes) = first_namespace_record(&ledger, "market-fact.v1");
    let dex = DexId::new("validator").unwrap();
    let dex_key = DexCurrentRecordV1::state_key(&dex).unwrap();
    let asset = AssetId::new("BTC").unwrap();
    let asset_key = AssetContextCurrentRecordV1::state_key(&asset).unwrap();
    let market_key = MarketCurrentRecordV1::state_key(&market).unwrap();
    let metadata_key = MarketMetadataVersionRecordV1::state_key(&market, "creation@1.0.0").unwrap();
    let outcome_key = OutcomeCurrentRecordV1::state_key(&market, &outcome).unwrap();

    let cases = vec![
        CodecCase {
            name: "fact",
            key: fact_key,
            wrong_key: MarketFactRecordV1::state_key(&EventId::new("wrong-event").unwrap())
                .unwrap(),
            bytes: fact_bytes,
            decode: decode_fact,
            decode_at: decode_fact_at,
        },
        CodecCase {
            name: "dex",
            key: dex_key.clone(),
            wrong_key: DexCurrentRecordV1::state_key(&DexId::new("other-dex").unwrap()).unwrap(),
            bytes: state_bytes(&ledger, &dex_key),
            decode: decode_dex,
            decode_at: decode_dex_at,
        },
        CodecCase {
            name: "asset",
            key: asset_key.clone(),
            wrong_key: AssetContextCurrentRecordV1::state_key(&AssetId::new("ETH").unwrap())
                .unwrap(),
            bytes: state_bytes(&ledger, &asset_key),
            decode: decode_asset,
            decode_at: decode_asset_at,
        },
        CodecCase {
            name: "market",
            key: market_key.clone(),
            wrong_key: MarketCurrentRecordV1::state_key(&MarketId::new("perp:ETH").unwrap())
                .unwrap(),
            bytes: state_bytes(&ledger, &market_key),
            decode: decode_market,
            decode_at: decode_market_at,
        },
        CodecCase {
            name: "metadata",
            key: metadata_key.clone(),
            wrong_key: MarketMetadataVersionRecordV1::state_key(&market, "other-version").unwrap(),
            bytes: state_bytes(&ledger, &metadata_key),
            decode: decode_metadata,
            decode_at: decode_metadata_at,
        },
        CodecCase {
            name: "outcome",
            key: outcome_key.clone(),
            wrong_key: OutcomeCurrentRecordV1::state_key(&market, &OutcomeId::new("down").unwrap())
                .unwrap(),
            bytes: state_bytes(&ledger, &outcome_key),
            decode: decode_outcome,
            decode_at: decode_outcome_at,
        },
    ];

    for case in cases {
        assert!(case.bytes.len() < 16 * 1024, "{} fixture", case.name);
        assert_eq!(
            (case.decode)(&case.bytes),
            Ok(()),
            "{} canonical",
            case.name
        );
        assert_eq!(
            (case.decode_at)(&case.key, &case.bytes),
            Ok(()),
            "{} key-bound",
            case.name
        );

        let mut whitespace = vec![b' '];
        whitespace.extend_from_slice(&case.bytes);
        assert_eq!(
            (case.decode)(&whitespace),
            Err(MarketStateError::NonCanonical),
            "{} noncanonical",
            case.name
        );
        assert_eq!(
            (case.decode)(&with_unknown_field(&case.bytes)),
            Err(MarketStateError::Codec),
            "{} unknown field",
            case.name
        );
        assert_eq!(
            (case.decode)(&vec![b'x'; 16 * 1024 + 1]),
            Err(MarketStateError::LimitExceeded),
            "{} bound",
            case.name
        );
        assert_eq!(
            (case.decode_at)(&case.wrong_key, &case.bytes),
            Err(MarketStateError::KeyMismatch),
            "{} wrong key",
            case.name
        );
    }
}

#[test]
fn single_and_compound_keys_enforce_exact_framed_boundaries() {
    const MAX_KEY_BYTES: usize = 64 * 1024;
    const FRAME_BYTES: usize = 8;

    let exact_single = MarketId::new("m".repeat(MAX_KEY_BYTES - FRAME_BYTES)).unwrap();
    let exact_single_key = MarketCurrentRecordV1::state_key(&exact_single).unwrap();
    assert_eq!(exact_single_key.key().len(), MAX_KEY_BYTES);
    let oversized_single = MarketId::new("m".repeat(MAX_KEY_BYTES - FRAME_BYTES + 1)).unwrap();
    assert_eq!(
        MarketCurrentRecordV1::state_key(&oversized_single),
        Err(MarketStateError::InvalidKey)
    );

    let market = MarketId::new("m").unwrap();
    let exact_second_length = MAX_KEY_BYTES - (2 * FRAME_BYTES) - market.as_str().len();
    let exact_version = "v".repeat(exact_second_length);
    let exact_metadata = MarketMetadataVersionRecordV1::state_key(&market, &exact_version).unwrap();
    assert_eq!(exact_metadata.key().len(), MAX_KEY_BYTES);
    assert_eq!(
        MarketMetadataVersionRecordV1::state_key(&market, &format!("{exact_version}v"),),
        Err(MarketStateError::InvalidKey)
    );

    let exact_outcome = OutcomeId::new("o".repeat(exact_second_length)).unwrap();
    let exact_outcome_key = OutcomeCurrentRecordV1::state_key(&market, &exact_outcome).unwrap();
    assert_eq!(exact_outcome_key.key().len(), MAX_KEY_BYTES);
    let oversized_outcome = OutcomeId::new("o".repeat(exact_second_length + 1)).unwrap();
    assert_eq!(
        OutcomeCurrentRecordV1::state_key(&market, &oversized_outcome),
        Err(MarketStateError::InvalidKey)
    );
}

#[test]
fn metadata_versions_require_distinct_heights_and_monotonic_identity() {
    let market = MarketId::new("perp:BTC").unwrap();
    let dex = DexId::new("validator").unwrap();
    let base = AssetId::new("BTC").unwrap();
    let quote = AssetId::new("USDC").unwrap();
    let mut same_height = ledger(1100);
    let same_height_before = same_height.state_image().canonical_bytes();
    let error = same_height
        .apply_block(&block(
            1100,
            vec![
                event(
                    1100,
                    0,
                    EventPayload::DexCreated(DexCreated {
                        dex_id: dex.clone(),
                        name: "Validator Perpetuals".to_owned(),
                        operator_account_id: OPERATOR,
                    }),
                    Vec::new(),
                    vec![OPERATOR],
                    "1.0.0",
                ),
                event(
                    1100,
                    1,
                    EventPayload::AssetContextUpdated(AssetContextUpdated {
                        asset_id: base.clone(),
                        context_version: "btc-v1".to_owned(),
                        context_hash: [0x21; 32],
                    }),
                    Vec::new(),
                    Vec::new(),
                    "1.0.0",
                ),
                event(
                    1100,
                    2,
                    EventPayload::AssetContextUpdated(AssetContextUpdated {
                        asset_id: quote.clone(),
                        context_version: "usdc-v1".to_owned(),
                        context_hash: [0x22; 32],
                    }),
                    Vec::new(),
                    Vec::new(),
                    "1.0.0",
                ),
                event(
                    1100,
                    3,
                    EventPayload::MarketCreated(MarketCreated {
                        market_id: market.clone(),
                        dex_id: dex,
                        base_asset_id: base,
                        quote_asset_id: quote,
                        tick_size: Price::parse_at_scale("0.1", 6).unwrap(),
                        lot_size: Quantity::parse_at_scale("0.001", 8).unwrap(),
                    }),
                    vec![market.clone()],
                    Vec::new(),
                    "1.0.0",
                ),
                event(
                    1100,
                    4,
                    EventPayload::MarketMetadataChanged(MarketMetadataChanged {
                        market_id: market.clone(),
                        metadata_version: "metadata-v2".to_owned(),
                        metadata_hash: [0x44; 32],
                    }),
                    vec![market.clone()],
                    Vec::new(),
                    "1.0.0",
                ),
            ],
        ))
        .unwrap_err();
    assert_eq!(
        error.reducer_reason_code(),
        Some("market_state.non_monotonic_metadata")
    );
    assert_eq!(
        same_height.state_image().canonical_bytes(),
        same_height_before
    );

    let mut non_monotonic = seeded_market(1100, &market);
    let error = non_monotonic
        .apply_block(&single_market_block(
            1101,
            &market,
            EventPayload::MarketMetadataChanged(MarketMetadataChanged {
                market_id: market.clone(),
                metadata_version: "a".to_owned(),
                metadata_hash: [0x55; 32],
            }),
        ))
        .unwrap_err();
    assert_eq!(
        error.reducer_reason_code(),
        Some("market_state.non_monotonic_metadata")
    );
}

#[test]
fn strictly_increasing_unresolved_metadata_versions_close_each_prior_interval() {
    let market = MarketId::new("perp:BTC").unwrap();
    let mut ledger = seeded_market(1150, &market);

    for (height, version, hash) in [
        (1151, "metadata-v2", [0x71; 32]),
        (1152, "metadata-v3", [0x72; 32]),
    ] {
        ledger
            .apply_block(&single_market_block(
                height,
                &market,
                EventPayload::MarketMetadataChanged(MarketMetadataChanged {
                    market_id: market.clone(),
                    metadata_version: version.to_owned(),
                    metadata_hash: hash,
                }),
            ))
            .unwrap();
    }

    let creation = metadata_version(&ledger, &market, "creation@1.0.0");
    assert_eq!(
        creation.effective_until_block(),
        Some(BlockHeight::new(1150))
    );
    assert_eq!(creation.resolution(), MarketMetadataResolutionV1::Exact);

    let v2 = metadata_version(&ledger, &market, "metadata-v2");
    assert_eq!(v2.effective_from_block(), BlockHeight::new(1151));
    assert_eq!(v2.effective_until_block(), Some(BlockHeight::new(1151)));
    assert_eq!(v2.resolution(), MarketMetadataResolutionV1::Unresolved);

    let v3 = metadata_version(&ledger, &market, "metadata-v3");
    assert_eq!(v3.effective_from_block(), BlockHeight::new(1152));
    assert_eq!(v3.effective_until_block(), None);
    assert_eq!(v3.resolution(), MarketMetadataResolutionV1::Unresolved);
    assert_eq!(v3.tick_size(), None);
    assert_eq!(v3.lot_size(), None);

    let current = current_market(&ledger, &market);
    assert_eq!(current.metadata_version(), "metadata-v3");
    assert_eq!(
        current.metadata_resolution(),
        MarketMetadataResolutionV1::Unresolved
    );
    assert_eq!(current.tick_size(), None);
    assert_eq!(current.lot_size(), None);
    assert_eq!(current.price_scale(), None);
    assert_eq!(current.quantity_scale(), None);

    let before_invalid = ledger.state_image().canonical_bytes();
    let error = ledger
        .apply_block(&single_market_block(
            1153,
            &market,
            EventPayload::MarketMetadataChanged(MarketMetadataChanged {
                market_id: market.clone(),
                metadata_version: "metadata-v2".to_owned(),
                metadata_hash: [0x73; 32],
            }),
        ))
        .unwrap_err();
    assert_eq!(
        error.reducer_reason_code(),
        Some("market_state.non_monotonic_metadata")
    );
    assert_eq!(ledger.state_image().canonical_bytes(), before_invalid);
}

#[test]
fn outcome_resolution_is_single_assignment() {
    let market = MarketId::new("perp:BTC").unwrap();
    let outcome = OutcomeId::new("up").unwrap();
    let mut ledger = seeded_market(1200, &market);
    ledger
        .apply_block(&single_market_block(
            1201,
            &market,
            EventPayload::OutcomeCreated(OutcomeCreated {
                market_id: market.clone(),
                outcome_id: outcome.clone(),
                description: "BTC closes higher".to_owned(),
            }),
        ))
        .unwrap();
    ledger
        .apply_block(&single_market_block(
            1202,
            &market,
            EventPayload::OutcomeResolved(OutcomeResolved {
                market_id: market.clone(),
                outcome_id: outcome.clone(),
                settlement_value: Price::parse_at_scale("1", 6).unwrap(),
                resolved_at: ProtocolTime::from_unix_micros(1202).unwrap(),
            }),
        ))
        .unwrap();
    let before = ledger.state_image().canonical_bytes();
    let error = ledger
        .apply_block(&single_market_block(
            1203,
            &market,
            EventPayload::OutcomeResolved(OutcomeResolved {
                market_id: market.clone(),
                outcome_id: outcome,
                settlement_value: Price::parse_at_scale("0", 6).unwrap(),
                resolved_at: ProtocolTime::from_unix_micros(1203).unwrap(),
            }),
        ))
        .unwrap_err();
    assert_eq!(
        error.reducer_reason_code(),
        Some("market_state.outcome_already_resolved")
    );
    assert_eq!(ledger.state_image().canonical_bytes(), before);
}

#[test]
fn unresolved_metadata_suppresses_every_value_dependent_transition() {
    let market = MarketId::new("perp:BTC").unwrap();
    let outcome = OutcomeId::new("up").unwrap();
    let payloads = vec![
        EventPayload::OpenInterestCapChanged(OpenInterestCapChanged {
            market_id: market.clone(),
            previous_cap: QuoteAmount::parse_at_scale("10", 6).unwrap(),
            new_cap: QuoteAmount::parse_at_scale("20", 6).unwrap(),
        }),
        EventPayload::MarginTableChanged(MarginTableChanged {
            market_id: market.clone(),
            previous_table_hash: "v1".to_owned(),
            new_table_hash: "v2".to_owned(),
        }),
        EventPayload::OracleUpdated(OracleUpdated {
            market_id: market.clone(),
            oracle_price: Price::parse_at_scale("65000", 6).unwrap(),
            source: "validator".to_owned(),
            effective_at: ProtocolTime::from_unix_micros(1302).unwrap(),
        }),
        EventPayload::FundingRateUpdated(FundingRateUpdated {
            market_id: market.clone(),
            funding_rate: FundingRate::parse_at_scale("0.0001", 8).unwrap(),
            effective_at: ProtocolTime::from_unix_micros(1302).unwrap(),
        }),
        EventPayload::OutcomeResolved(OutcomeResolved {
            market_id: market.clone(),
            outcome_id: outcome.clone(),
            settlement_value: Price::parse_at_scale("1", 6).unwrap(),
            resolved_at: ProtocolTime::from_unix_micros(1302).unwrap(),
        }),
    ];
    for payload in payloads {
        let mut ledger = seeded_market(1300, &market);
        if matches!(&payload, EventPayload::OutcomeResolved(_)) {
            ledger
                .apply_block(&single_market_block(
                    1301,
                    &market,
                    EventPayload::OutcomeCreated(OutcomeCreated {
                        market_id: market.clone(),
                        outcome_id: outcome.clone(),
                        description: "BTC closes higher".to_owned(),
                    }),
                ))
                .unwrap();
            ledger
                .apply_block(&single_market_block(
                    1302,
                    &market,
                    EventPayload::MarketMetadataChanged(MarketMetadataChanged {
                        market_id: market.clone(),
                        metadata_version: "metadata-v2".to_owned(),
                        metadata_hash: [0x66; 32],
                    }),
                ))
                .unwrap();
            let error = ledger
                .apply_block(&single_market_block(1303, &market, payload))
                .unwrap_err();
            assert_eq!(
                error.reducer_reason_code(),
                Some("market_state.metadata_unresolved")
            );
        } else {
            ledger
                .apply_block(&single_market_block(
                    1301,
                    &market,
                    EventPayload::MarketMetadataChanged(MarketMetadataChanged {
                        market_id: market.clone(),
                        metadata_version: "metadata-v2".to_owned(),
                        metadata_hash: [0x66; 32],
                    }),
                ))
                .unwrap();
            let error = ledger
                .apply_block(&single_market_block(1302, &market, payload))
                .unwrap_err();
            assert_eq!(
                error.reducer_reason_code(),
                Some("market_state.metadata_unresolved")
            );
        }
    }
}

fn seeded_market(height: u64, market: &MarketId) -> CanonicalLedger<CanonicalMarketReducerV1> {
    let dex = DexId::new("validator").unwrap();
    let base = AssetId::new("BTC").unwrap();
    let quote = AssetId::new("USDC").unwrap();
    let mut ledger = ledger(height);
    ledger
        .apply_block(&block(
            height,
            vec![
                event(
                    height,
                    0,
                    EventPayload::DexCreated(DexCreated {
                        dex_id: dex.clone(),
                        name: "Validator Perpetuals".to_owned(),
                        operator_account_id: OPERATOR,
                    }),
                    Vec::new(),
                    vec![OPERATOR],
                    "1.0.0",
                ),
                event(
                    height,
                    1,
                    EventPayload::AssetContextUpdated(AssetContextUpdated {
                        asset_id: base.clone(),
                        context_version: "btc-v1".to_owned(),
                        context_hash: [0x21; 32],
                    }),
                    Vec::new(),
                    Vec::new(),
                    "1.0.0",
                ),
                event(
                    height,
                    2,
                    EventPayload::AssetContextUpdated(AssetContextUpdated {
                        asset_id: quote.clone(),
                        context_version: "usdc-v1".to_owned(),
                        context_hash: [0x22; 32],
                    }),
                    Vec::new(),
                    Vec::new(),
                    "1.0.0",
                ),
                event(
                    height,
                    3,
                    EventPayload::MarketCreated(MarketCreated {
                        market_id: market.clone(),
                        dex_id: dex,
                        base_asset_id: base,
                        quote_asset_id: quote,
                        tick_size: Price::parse_at_scale("0.1", 6).unwrap(),
                        lot_size: Quantity::parse_at_scale("0.00001", 8).unwrap(),
                    }),
                    vec![market.clone()],
                    Vec::new(),
                    "1.0.0",
                ),
            ],
        ))
        .unwrap();
    ledger
}

fn single_market_block(height: u64, market: &MarketId, payload: EventPayload) -> BlockEnvelope {
    block(
        height,
        vec![event(
            height,
            0,
            payload,
            vec![market.clone()],
            Vec::new(),
            "1.0.0",
        )],
    )
}

fn current_market(
    ledger: &CanonicalLedger<CanonicalMarketReducerV1>,
    market: &MarketId,
) -> MarketCurrentRecordV1 {
    let key = MarketCurrentRecordV1::state_key(market).unwrap();
    MarketCurrentRecordV1::decode_at(&key, ledger.state_image().entries().get(&key).unwrap())
        .unwrap()
}

fn metadata_version(
    ledger: &CanonicalLedger<CanonicalMarketReducerV1>,
    market: &MarketId,
    version: &str,
) -> MarketMetadataVersionRecordV1 {
    let key = MarketMetadataVersionRecordV1::state_key(market, version).unwrap();
    MarketMetadataVersionRecordV1::decode_at(
        &key,
        ledger.state_image().entries().get(&key).unwrap(),
    )
    .unwrap()
}

fn namespace_count(ledger: &CanonicalLedger<CanonicalMarketReducerV1>, namespace: &str) -> usize {
    ledger
        .state_image()
        .entries()
        .keys()
        .filter(|key| key.namespace() == namespace)
        .count()
}

struct CodecCase {
    name: &'static str,
    key: StateKey,
    wrong_key: StateKey,
    bytes: Vec<u8>,
    decode: fn(&[u8]) -> Result<(), MarketStateError>,
    decode_at: fn(&StateKey, &[u8]) -> Result<(), MarketStateError>,
}

fn first_namespace_record(
    ledger: &CanonicalLedger<CanonicalMarketReducerV1>,
    namespace: &str,
) -> (StateKey, Vec<u8>) {
    ledger
        .state_image()
        .entries()
        .iter()
        .find(|(key, _)| key.namespace() == namespace)
        .map(|(key, bytes)| (key.clone(), bytes.clone()))
        .unwrap()
}

fn state_bytes(ledger: &CanonicalLedger<CanonicalMarketReducerV1>, key: &StateKey) -> Vec<u8> {
    ledger.state_image().entries().get(key).unwrap().clone()
}

fn with_unknown_field(bytes: &[u8]) -> Vec<u8> {
    let mut value: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .insert("unknown_field".to_owned(), serde_json::Value::Bool(true));
    serde_json::to_vec(&value).unwrap()
}

fn decode_fact(bytes: &[u8]) -> Result<(), MarketStateError> {
    MarketFactRecordV1::decode(bytes).map(|_| ())
}

fn decode_fact_at(key: &StateKey, bytes: &[u8]) -> Result<(), MarketStateError> {
    MarketFactRecordV1::decode_at(key, bytes).map(|_| ())
}

fn decode_dex(bytes: &[u8]) -> Result<(), MarketStateError> {
    DexCurrentRecordV1::decode(bytes).map(|_| ())
}

fn decode_dex_at(key: &StateKey, bytes: &[u8]) -> Result<(), MarketStateError> {
    DexCurrentRecordV1::decode_at(key, bytes).map(|_| ())
}

fn decode_asset(bytes: &[u8]) -> Result<(), MarketStateError> {
    AssetContextCurrentRecordV1::decode(bytes).map(|_| ())
}

fn decode_asset_at(key: &StateKey, bytes: &[u8]) -> Result<(), MarketStateError> {
    AssetContextCurrentRecordV1::decode_at(key, bytes).map(|_| ())
}

fn decode_market(bytes: &[u8]) -> Result<(), MarketStateError> {
    MarketCurrentRecordV1::decode(bytes).map(|_| ())
}

fn decode_market_at(key: &StateKey, bytes: &[u8]) -> Result<(), MarketStateError> {
    MarketCurrentRecordV1::decode_at(key, bytes).map(|_| ())
}

fn decode_metadata(bytes: &[u8]) -> Result<(), MarketStateError> {
    MarketMetadataVersionRecordV1::decode(bytes).map(|_| ())
}

fn decode_metadata_at(key: &StateKey, bytes: &[u8]) -> Result<(), MarketStateError> {
    MarketMetadataVersionRecordV1::decode_at(key, bytes).map(|_| ())
}

fn decode_outcome(bytes: &[u8]) -> Result<(), MarketStateError> {
    OutcomeCurrentRecordV1::decode(bytes).map(|_| ())
}

fn decode_outcome_at(key: &StateKey, bytes: &[u8]) -> Result<(), MarketStateError> {
    OutcomeCurrentRecordV1::decode_at(key, bytes).map(|_| ())
}

fn ledger(first_height: u64) -> CanonicalLedger<CanonicalMarketReducerV1> {
    CanonicalLedger::try_new(
        ChainId::new("mainnet").unwrap(),
        BlockHeight::new(first_height),
        CanonicalMarketReducerV1,
        LedgerLimits::production(),
    )
    .unwrap()
}

fn block(height: u64, events: Vec<CanonicalEventEnvelope>) -> BlockEnvelope {
    BlockEnvelope::try_new(
        ChainId::new("mainnet").unwrap(),
        BlockHeight::new(height),
        ProtocolTime::from_unix_micros(height as i64).unwrap(),
        ConfirmationClass::CommittedPrimary,
        events,
        BTreeMap::from([(SourceId::new("test-primary").unwrap(), [height as u8; 32])]),
    )
    .unwrap()
}

fn event(
    height: u64,
    event_index: u32,
    payload: EventPayload,
    market_ids: Vec<MarketId>,
    account_ids: Vec<Address>,
    schema: &str,
) -> CanonicalEventEnvelope {
    let payload_hash = *blake3::hash(&payload.encode_to_vec().unwrap()).as_bytes();
    CanonicalEventEnvelope::from_input(CanonicalEventInput {
        schema_version: schema.to_owned(),
        chain_id: ChainId::new("mainnet").unwrap(),
        block_height: BlockHeight::new(height),
        block_time: ProtocolTime::from_unix_micros(height as i64).unwrap(),
        transaction_id: TransactionId::new(format!("tx-{height}")).unwrap(),
        transaction_index: 0,
        canonical_event_index: event_index,
        market_ids,
        account_ids,
        source_evidence: vec![
            SourceEvidence::try_new_indexed(
                SourceId::new("test-primary").unwrap(),
                "v1",
                height.to_string(),
                payload_hash,
                event_index,
            )
            .unwrap(),
        ],
        confirmation_class: ConfirmationClass::CommittedPrimary,
        observed_at: KnownTime::from_unix_micros(height as i64).unwrap(),
        ingested_at: KnownTime::from_unix_micros(height as i64).unwrap(),
        canonicalized_at: KnownTime::from_unix_micros(height as i64).unwrap(),
        parser_version: "test-parser-v1".to_owned(),
        payload,
    })
    .unwrap()
}
