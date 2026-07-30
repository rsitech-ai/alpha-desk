use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;
use std::str::FromStr;

use canonical_events::{
    BlockEnvelope, CanonicalEventEnvelope, CanonicalEventInput, ConfirmationClass, EventKind,
    EventPayload, SourceEvidence, TradeMatched,
};
use canonical_ledger::{
    ApplyContext, BlockDeltaView, CanonicalLedger, CanonicalPositionEpisodeReducerV1, EventReducer,
    LedgerLimits, PositionEpisodeCurrentRecordV1, PositionEpisodeRecordV1,
    PositionQuantityCurrentRecordV1, ReducerError, StateKey, StateMutation, StateView,
    derive_position_episode_id,
};
use domain_types::{
    Address, BlockHeight, ChainId, EventId, KnownTime, MarketId, PositionEpisodeId, Price,
    ProtocolTime, Quantity, SourceId, TransactionId,
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct ObservedDeltaEntry {
    namespace: String,
    key: Vec<u8>,
    block_start_value: Option<Vec<u8>>,
    block_final_value: Option<Vec<u8>>,
    write_count: u32,
}

#[derive(Debug, Clone)]
struct ResolvedPairFixture {
    account: Address,
    market: MarketId,
    quantity_key: StateKey,
    current_key: StateKey,
    episode_key: StateKey,
    quantity: Vec<u8>,
    current: Vec<u8>,
    episode: Vec<u8>,
}

impl ResolvedPairFixture {
    fn mutations(&self) -> Vec<StateMutation> {
        vec![
            StateMutation::put(self.current_key.clone(), self.current.clone()),
            StateMutation::put(self.episode_key.clone(), self.episode.clone()),
            StateMutation::put(self.quantity_key.clone(), self.quantity.clone()),
        ]
    }
}

#[derive(Debug, Clone)]
struct DeltaRecordingReducer {
    observed: Rc<RefCell<Vec<Vec<ObservedDeltaEntry>>>>,
}

#[derive(Debug, Clone)]
struct EpisodeDeltaReducer {
    scripts: Rc<BTreeMap<u64, Vec<StateMutation>>>,
    episode: CanonicalPositionEpisodeReducerV1,
    validate: Rc<Cell<bool>>,
}

#[derive(Debug, Clone)]
struct ScriptedReducer {
    scripts: Rc<BTreeMap<u64, Vec<StateMutation>>>,
}

impl EventReducer for ScriptedReducer {
    fn reducer_set_version(&self) -> &str {
        "scripted-block-delta-test@1.0.0"
    }

    fn supports(&self, event: &CanonicalEventEnvelope) -> bool {
        event.event_kind() == EventKind::TradeMatched && event.schema_version() == "1.0.0"
    }

    fn reduce(
        &self,
        _state: &StateView<'_>,
        event: &CanonicalEventEnvelope,
        _context: &ApplyContext<'_>,
    ) -> Result<Vec<StateMutation>, ReducerError> {
        let EventPayload::TradeMatched(trade) = event.payload() else {
            unreachable!("only trade events are supported");
        };
        Ok(self
            .scripts
            .get(&trade.deterministic_seed)
            .cloned()
            .unwrap_or_default())
    }
}

impl EventReducer for EpisodeDeltaReducer {
    fn reducer_set_version(&self) -> &str {
        "episode-block-delta-test@1.0.0"
    }

    fn supports(&self, event: &CanonicalEventEnvelope) -> bool {
        event.event_kind() == EventKind::TradeMatched && event.schema_version() == "1.0.0"
    }

    fn reduce(
        &self,
        _state: &StateView<'_>,
        event: &CanonicalEventEnvelope,
        _context: &ApplyContext<'_>,
    ) -> Result<Vec<StateMutation>, ReducerError> {
        let EventPayload::TradeMatched(trade) = event.payload() else {
            unreachable!("only trade events are supported");
        };
        Ok(self
            .scripts
            .get(&trade.deterministic_seed)
            .cloned()
            .unwrap_or_default())
    }

    fn validate_block_delta(
        &self,
        final_state: &StateView<'_>,
        delta: &BlockDeltaView<'_>,
        context: &ApplyContext<'_>,
    ) -> Result<(), ReducerError> {
        if !self.validate.get() {
            return Ok(());
        }
        self.episode
            .validate_block_delta(final_state, delta, context)
    }
}

impl EventReducer for DeltaRecordingReducer {
    fn reducer_set_version(&self) -> &str {
        "block-delta-recording@1.0.0"
    }

    fn supports(&self, event: &CanonicalEventEnvelope) -> bool {
        event.event_kind() == EventKind::TradeMatched && event.schema_version() == "1.0.0"
    }

    fn reduce(
        &self,
        _state: &StateView<'_>,
        event: &CanonicalEventEnvelope,
        _context: &ApplyContext<'_>,
    ) -> Result<Vec<StateMutation>, ReducerError> {
        let EventPayload::TradeMatched(trade) = event.payload() else {
            unreachable!("only trade events are supported");
        };
        let namespace = "test.delta";
        let mutations = match trade.deterministic_seed {
            1 => vec![
                StateMutation::put(
                    StateKey::try_new(namespace, b"z".to_vec()).unwrap(),
                    b"one".to_vec(),
                ),
                StateMutation::put(
                    StateKey::try_new(namespace, b"a".to_vec()).unwrap(),
                    b"created".to_vec(),
                ),
            ],
            2 => vec![
                StateMutation::put(
                    StateKey::try_new(namespace, b"z".to_vec()).unwrap(),
                    b"two".to_vec(),
                ),
                StateMutation::delete(StateKey::try_new(namespace, b"a".to_vec()).unwrap()),
            ],
            3 => vec![StateMutation::put(
                StateKey::try_new(namespace, b"z".to_vec()).unwrap(),
                b"two".to_vec(),
            )],
            _ => unreachable!("unexpected deterministic seed"),
        };
        Ok(mutations)
    }

    fn validate_block(
        &self,
        _state: &StateView<'_>,
        _context: &ApplyContext<'_>,
    ) -> Result<(), ReducerError> {
        panic!("the ledger must bridge block validation only through validate_block_delta")
    }

    fn validate_block_delta(
        &self,
        _final_state: &StateView<'_>,
        delta: &BlockDeltaView<'_>,
        _context: &ApplyContext<'_>,
    ) -> Result<(), ReducerError> {
        assert_eq!(delta.len(), delta.entries().len());
        assert_eq!(delta.is_empty(), delta.entries().is_empty());
        let observed = delta
            .into_iter()
            .map(|entry| ObservedDeltaEntry {
                namespace: entry.key().namespace().to_owned(),
                key: entry.key().key().to_vec(),
                block_start_value: entry.block_start_value().map(<[u8]>::to_vec),
                block_final_value: entry.block_final_value().map(<[u8]>::to_vec),
                write_count: entry.write_count(),
            })
            .collect();
        self.observed.borrow_mut().push(observed);
        Ok(())
    }
}

#[test]
fn normalized_delta_is_sorted_and_retains_create_delete_and_repeated_write_history() {
    let observed = Rc::new(RefCell::new(Vec::new()));
    let reducer = DeltaRecordingReducer {
        observed: Rc::clone(&observed),
    };
    let mut ledger = CanonicalLedger::try_new(
        ChainId::new("mainnet").unwrap(),
        BlockHeight::new(10),
        reducer,
        LedgerLimits::production(),
    )
    .unwrap();

    ledger.apply_block(&trade_block(10, &[1, 2])).unwrap();
    ledger.apply_block(&trade_block(11, &[3])).unwrap();
    ledger.apply_block(&empty_block(12)).unwrap();

    assert_eq!(
        observed.borrow().as_slice(),
        [
            vec![
                ObservedDeltaEntry {
                    namespace: "test.delta".to_owned(),
                    key: b"a".to_vec(),
                    block_start_value: None,
                    block_final_value: None,
                    write_count: 2,
                },
                ObservedDeltaEntry {
                    namespace: "test.delta".to_owned(),
                    key: b"z".to_vec(),
                    block_start_value: None,
                    block_final_value: Some(b"two".to_vec()),
                    write_count: 2,
                },
            ],
            vec![ObservedDeltaEntry {
                namespace: "test.delta".to_owned(),
                key: b"z".to_vec(),
                block_start_value: Some(b"two".to_vec()),
                block_final_value: Some(b"two".to_vec()),
                write_count: 1,
            },],
            Vec::new(),
        ]
    );
}

#[test]
fn touched_quantity_without_episode_current_is_rejected_atomically() {
    let account = Address::from_bytes([0x11; 20]);
    let market = MarketId::new("perp:BTC").unwrap();
    let quantity_key = PositionQuantityCurrentRecordV1::state_key(&account, &market).unwrap();
    let quantity = quantity_current_bytes(account, &market, "1.00000000");
    PositionQuantityCurrentRecordV1::decode_at(&quantity_key, &quantity).unwrap();
    let reducer = EpisodeDeltaReducer {
        scripts: Rc::new(BTreeMap::from([(
            1,
            vec![StateMutation::put(quantity_key, quantity)],
        )])),
        episode: CanonicalPositionEpisodeReducerV1,
        validate: Rc::new(Cell::new(true)),
    };
    let mut ledger = CanonicalLedger::try_new(
        ChainId::new("mainnet").unwrap(),
        BlockHeight::new(20),
        reducer,
        LedgerLimits::production(),
    )
    .unwrap();
    let before = ledger.state_image().canonical_bytes();

    let error = ledger
        .apply_block(&trade_block(20, &[1]))
        .expect_err("orphan quantity current must fail block validation");

    assert_eq!(error.reason_code(), "ledger.reducer_failed");
    assert_eq!(
        error.reducer_reason_code(),
        Some("position_episode.current_pair_mismatch")
    );
    assert_eq!(ledger.state_image().canonical_bytes(), before);
}

#[test]
fn corrupt_quantity_trigger_precedes_corrupt_episode_current_trigger() {
    let account = Address::from_bytes([0x12; 20]);
    let market = MarketId::new("perp:ETH").unwrap();
    let quantity_key = PositionQuantityCurrentRecordV1::state_key(&account, &market).unwrap();
    let current_key = PositionEpisodeCurrentRecordV1::state_key(&account, &market).unwrap();
    let reducer = EpisodeDeltaReducer {
        scripts: Rc::new(BTreeMap::from([(
            1,
            vec![
                StateMutation::put(current_key, b"{}".to_vec()),
                StateMutation::put(quantity_key, b"{}".to_vec()),
            ],
        )])),
        episode: CanonicalPositionEpisodeReducerV1,
        validate: Rc::new(Cell::new(true)),
    };
    let mut ledger = CanonicalLedger::try_new(
        ChainId::new("mainnet").unwrap(),
        BlockHeight::new(25),
        reducer,
        LedgerLimits::production(),
    )
    .unwrap();
    let before = ledger.state_image().canonical_bytes();

    let error = ledger
        .apply_block(&trade_block(25, &[1]))
        .expect_err("quantity trigger corruption has frozen precedence");

    assert_eq!(
        error.reducer_reason_code(),
        Some("position_episode.quantity_current_invalid")
    );
    assert_eq!(ledger.state_image().canonical_bytes(), before);
}

#[test]
fn all_three_episode_namespaces_trigger_and_complete_current_deletion_is_valid() {
    let pair = resolved_pair_fixture(0x21, "perp:SOL", "sol-open");
    let scripts = BTreeMap::from([
        (1, pair.mutations()),
        (
            2,
            vec![
                StateMutation::delete(pair.current_key.clone()),
                StateMutation::delete(pair.quantity_key.clone()),
            ],
        ),
    ]);
    let mut ledger = episode_delta_ledger(70, scripts, Rc::new(Cell::new(true)));

    ledger.apply_block(&trade_block(70, &[1])).unwrap();
    ledger.apply_block(&trade_block(71, &[2])).unwrap();

    assert!(
        !ledger
            .state_image()
            .entries()
            .contains_key(&pair.quantity_key)
    );
    assert!(
        !ledger
            .state_image()
            .entries()
            .contains_key(&pair.current_key)
    );
    assert_eq!(
        ledger.state_image().entries()[&pair.episode_key],
        pair.episode
    );
}

#[test]
fn zero_unknown_and_create_delete_current_pair_matrix_cases_are_valid() {
    let zero_account = Address::from_bytes([0x31; 20]);
    let zero_market = MarketId::new("perp:ZERO").unwrap();
    let zero_quantity_key =
        PositionQuantityCurrentRecordV1::state_key(&zero_account, &zero_market).unwrap();
    let zero_current_key =
        PositionEpisodeCurrentRecordV1::state_key(&zero_account, &zero_market).unwrap();
    let unknown_account = Address::from_bytes([0x32; 20]);
    let unknown_market = MarketId::new("perp:UNKNOWN").unwrap();
    let unknown_quantity_key =
        PositionQuantityCurrentRecordV1::state_key(&unknown_account, &unknown_market).unwrap();
    let unknown_current_key =
        PositionEpisodeCurrentRecordV1::state_key(&unknown_account, &unknown_market).unwrap();
    let scripts = BTreeMap::from([(
        1,
        vec![
            StateMutation::put(
                zero_quantity_key.clone(),
                quantity_current_bytes(zero_account, &zero_market, "0.00000000"),
            ),
            StateMutation::put(
                zero_current_key.clone(),
                unresolved_current_bytes(zero_account, &zero_market, "no_open_episode"),
            ),
            StateMutation::put(
                unknown_quantity_key.clone(),
                unknown_quantity_bytes(unknown_account, &unknown_market),
            ),
            StateMutation::put(
                unknown_current_key.clone(),
                unresolved_current_bytes(unknown_account, &unknown_market, "interrupted"),
            ),
        ],
    )]);
    let mut ledger = episode_delta_ledger(75, scripts, Rc::new(Cell::new(true)));
    ledger.apply_block(&trade_block(75, &[1])).unwrap();
    assert!(
        ledger
            .state_image()
            .entries()
            .contains_key(&zero_quantity_key)
    );
    assert!(
        ledger
            .state_image()
            .entries()
            .contains_key(&zero_current_key)
    );
    assert!(
        ledger
            .state_image()
            .entries()
            .contains_key(&unknown_quantity_key)
    );
    assert!(
        ledger
            .state_image()
            .entries()
            .contains_key(&unknown_current_key)
    );

    let transient_account = Address::from_bytes([0x33; 20]);
    let transient_market = MarketId::new("perp:TRANSIENT").unwrap();
    let transient_quantity_key =
        PositionQuantityCurrentRecordV1::state_key(&transient_account, &transient_market).unwrap();
    let transient_current_key =
        PositionEpisodeCurrentRecordV1::state_key(&transient_account, &transient_market).unwrap();
    let scripts = BTreeMap::from([
        (
            1,
            vec![
                StateMutation::put(
                    transient_quantity_key.clone(),
                    quantity_current_bytes(transient_account, &transient_market, "0.00000000"),
                ),
                StateMutation::put(
                    transient_current_key.clone(),
                    unresolved_current_bytes(
                        transient_account,
                        &transient_market,
                        "no_open_episode",
                    ),
                ),
            ],
        ),
        (
            2,
            vec![
                StateMutation::delete(transient_current_key),
                StateMutation::delete(transient_quantity_key),
            ],
        ),
    ]);
    let mut transient = episode_delta_ledger(76, scripts, Rc::new(Cell::new(true)));
    transient.apply_block(&trade_block(76, &[1, 2])).unwrap();
    assert!(transient.state_image().entries().is_empty());
}

#[test]
fn every_inconsistent_present_current_pair_matrix_combination_is_rejected() {
    for case in 0_u8..6 {
        let account_byte = 0x40 + case;
        let market_name = format!("perp:MATRIX{case}");
        let anchor = format!("matrix-open-{case}");
        let pair = resolved_pair_fixture(account_byte, &market_name, &anchor);
        let (quantity, current, episode) = match case {
            0 => (
                quantity_current_bytes(pair.account, &pair.market, "0.00000000"),
                unresolved_current_bytes(pair.account, &pair.market, "interrupted"),
                None,
            ),
            1 => (
                unknown_quantity_bytes(pair.account, &pair.market),
                unresolved_current_bytes(pair.account, &pair.market, "no_open_episode"),
                None,
            ),
            2 => (
                quantity_current_bytes(pair.account, &pair.market, "1.00000000"),
                unresolved_current_bytes(pair.account, &pair.market, "no_open_episode"),
                None,
            ),
            3 => (
                quantity_current_bytes(pair.account, &pair.market, "1.00000000"),
                unresolved_current_bytes(pair.account, &pair.market, "interrupted"),
                None,
            ),
            4 => (
                quantity_current_bytes(pair.account, &pair.market, "0.00000000"),
                pair.current.clone(),
                Some(pair.episode.clone()),
            ),
            5 => (
                unknown_quantity_bytes(pair.account, &pair.market),
                pair.current.clone(),
                Some(pair.episode.clone()),
            ),
            _ => unreachable!(),
        };
        let mut mutations = vec![
            StateMutation::put(pair.current_key.clone(), current),
            StateMutation::put(pair.quantity_key.clone(), quantity),
        ];
        if let Some(episode) = episode {
            mutations.push(StateMutation::put(pair.episode_key.clone(), episode));
        }
        let height = 77 + u64::from(case);
        let mut ledger = episode_delta_ledger(
            height,
            BTreeMap::from([(1, mutations)]),
            Rc::new(Cell::new(true)),
        );
        let before = ledger.state_image().canonical_bytes();

        let error = ledger
            .apply_block(&trade_block(height, &[1]))
            .expect_err("inconsistent current pair matrix must fail");

        assert_eq!(
            error.reducer_reason_code(),
            Some("position_episode.current_pair_mismatch")
        );
        assert_eq!(ledger.state_image().canonical_bytes(), before);
    }
}

#[test]
fn deleting_a_referenced_episode_fails_reference_validation_before_pair_matrix() {
    let pair = resolved_pair_fixture(0x22, "perp:XRP", "xrp-open");
    let zero_quantity = quantity_current_bytes(pair.account, &pair.market, "0.00000000");
    let scripts = BTreeMap::from([
        (1, pair.mutations()),
        (
            2,
            vec![
                StateMutation::delete(pair.episode_key.clone()),
                StateMutation::put(pair.quantity_key.clone(), zero_quantity),
            ],
        ),
    ]);
    let mut ledger = episode_delta_ledger(80, scripts, Rc::new(Cell::new(true)));
    ledger.apply_block(&trade_block(80, &[1])).unwrap();
    let before = ledger.state_image().canonical_bytes();

    let error = ledger
        .apply_block(&trade_block(81, &[2]))
        .expect_err("missing referenced episode must precede the zero/resolved matrix mismatch");

    assert_eq!(
        error.reducer_reason_code(),
        Some("position_episode.episode_reference_invalid")
    );
    assert_eq!(ledger.state_image().canonical_bytes(), before);

    let replacement = closed_episode_bytes(
        pair.account,
        &pair.market,
        &derive_position_episode_id(
            &pair.account,
            &pair.market,
            &EventId::new("xrp-open").unwrap(),
            0,
        )
        .unwrap(),
        "xrp-open",
    );
    PositionEpisodeRecordV1::decode_at(&pair.episode_key, &replacement).unwrap();
    let scripts = BTreeMap::from([
        (1, pair.mutations()),
        (
            2,
            vec![StateMutation::put(pair.episode_key.clone(), replacement)],
        ),
    ]);
    let mut replaced = episode_delta_ledger(82, scripts, Rc::new(Cell::new(true)));
    replaced.apply_block(&trade_block(82, &[1])).unwrap();
    let before = replaced.state_image().canonical_bytes();
    let error = replaced
        .apply_block(&trade_block(83, &[2]))
        .expect_err("replacing a referenced open episode with a terminal record must fail");
    assert_eq!(
        error.reducer_reason_code(),
        Some("position_episode.episode_reference_invalid")
    );
    assert_eq!(replaced.state_image().canonical_bytes(), before);
}

#[test]
fn corrupt_episode_current_trigger_precedes_reference_validation() {
    let pair = resolved_pair_fixture(0x23, "perp:ADA", "ada-open");
    let scripts = BTreeMap::from([
        (1, pair.mutations()),
        (
            2,
            vec![
                StateMutation::delete(pair.episode_key.clone()),
                StateMutation::put(pair.current_key.clone(), b"{}".to_vec()),
            ],
        ),
    ]);
    let mut ledger = episode_delta_ledger(90, scripts, Rc::new(Cell::new(true)));
    ledger.apply_block(&trade_block(90, &[1])).unwrap();
    let before = ledger.state_image().canonical_bytes();

    let error = ledger
        .apply_block(&trade_block(91, &[2]))
        .expect_err("corrupt current trigger must fail before reference validation");

    assert_eq!(
        error.reducer_reason_code(),
        Some("position_episode.episode_current_invalid")
    );
    assert_eq!(ledger.state_image().canonical_bytes(), before);
}

#[test]
fn touched_episode_decodes_block_start_before_block_final() {
    let pair = resolved_pair_fixture(0x24, "perp:DOGE", "doge-open");
    let validate = Rc::new(Cell::new(false));
    let scripts = BTreeMap::from([
        (
            1,
            vec![
                StateMutation::put(pair.quantity_key.clone(), pair.quantity.clone()),
                StateMutation::put(pair.current_key.clone(), pair.current.clone()),
                StateMutation::put(pair.episode_key.clone(), b"{}".to_vec()),
            ],
        ),
        (
            2,
            vec![StateMutation::put(
                pair.episode_key.clone(),
                pair.episode.clone(),
            )],
        ),
    ]);
    let mut ledger = episode_delta_ledger(100, scripts, Rc::clone(&validate));
    ledger.apply_block(&trade_block(100, &[1])).unwrap();
    validate.set(true);
    let before = ledger.state_image().canonical_bytes();

    let error = ledger
        .apply_block(&trade_block(101, &[2]))
        .expect_err("corrupt block-start episode must win over a valid block-final replacement");

    assert_eq!(
        error.reducer_reason_code(),
        Some("position_episode.episode_prior_invalid")
    );
    assert_eq!(ledger.state_image().canonical_bytes(), before);
}

#[test]
fn key_mismatched_trigger_records_map_to_their_frozen_namespace_reasons() {
    let left = resolved_pair_fixture(0x26, "perp:LEFT", "left-open");
    let right = resolved_pair_fixture(0x27, "perp:RIGHT", "right-open");
    let cases = [
        (
            left.quantity_key.clone(),
            right.quantity.clone(),
            "position_episode.quantity_current_invalid",
        ),
        (
            left.current_key.clone(),
            right.current.clone(),
            "position_episode.episode_current_invalid",
        ),
        (
            left.episode_key.clone(),
            right.episode.clone(),
            "position_episode.episode_prior_invalid",
        ),
    ];

    for (offset, (key, value, reason)) in cases.into_iter().enumerate() {
        let height = 105 + u64::try_from(offset).unwrap();
        let mut ledger = episode_delta_ledger(
            height,
            BTreeMap::from([(1, vec![StateMutation::put(key, value)])]),
            Rc::new(Cell::new(true)),
        );
        let before = ledger.state_image().canonical_bytes();

        let error = ledger
            .apply_block(&trade_block(height, &[1]))
            .expect_err("key-mismatched trigger record must fail closed");

        assert_eq!(error.reducer_reason_code(), Some(reason));
        assert_eq!(ledger.state_image().canonical_bytes(), before);
    }
}

#[test]
fn malformed_current_key_and_unidentifiable_episode_create_delete_fail_closed() {
    let pair = resolved_pair_fixture(0x25, "perp:AVAX", "avax-open");
    let account = Address::from_bytes([0x25; 20]);
    let valid_market = b"perp:AVAX";
    let malformed_keys = [
        vec![0; 7],
        vec![0; 8],
        [frame(account.as_bytes()), 0_u64.to_be_bytes().to_vec()].concat(),
        [
            frame(account.as_bytes()),
            frame(valid_market),
            b"trailing".to_vec(),
        ]
        .concat(),
        [frame(&[0x25; 19]), frame(valid_market)].concat(),
        [frame(account.as_bytes()), frame(&[0xff])].concat(),
        [frame(account.as_bytes()), frame(b" bad-market")].concat(),
    ];
    for (offset, encoded) in malformed_keys.into_iter().enumerate() {
        let malformed_quantity_key =
            StateKey::try_new("position-quantity-current.v1", encoded).unwrap();
        let scripts = BTreeMap::from([
            (
                1,
                vec![StateMutation::put(
                    malformed_quantity_key.clone(),
                    b"temporary".to_vec(),
                )],
            ),
            (
                2,
                vec![StateMutation::delete(malformed_quantity_key.clone())],
            ),
        ]);
        let height = 110 + u64::try_from(offset).unwrap();
        let mut malformed = episode_delta_ledger(height, scripts, Rc::new(Cell::new(true)));
        let before = malformed.state_image().canonical_bytes();
        let error = malformed
            .apply_block(&trade_block(height, &[1, 2]))
            .expect_err("malformed create-delete current key remains a delta trigger");
        assert_eq!(
            error.reducer_reason_code(),
            Some("position_episode.quantity_current_invalid")
        );
        assert_eq!(malformed.state_image().canonical_bytes(), before);
    }

    let malformed_current_key =
        StateKey::try_new("position-episode-current.v1", vec![0; 8]).unwrap();
    let scripts = BTreeMap::from([
        (
            1,
            vec![StateMutation::put(
                malformed_current_key.clone(),
                b"temporary".to_vec(),
            )],
        ),
        (
            2,
            vec![StateMutation::delete(malformed_current_key.clone())],
        ),
    ]);
    let mut malformed_current = episode_delta_ledger(118, scripts, Rc::new(Cell::new(true)));
    let error = malformed_current
        .apply_block(&trade_block(118, &[1, 2]))
        .expect_err("malformed episode-current key must use its namespace reason");
    assert_eq!(
        error.reducer_reason_code(),
        Some("position_episode.episode_current_invalid")
    );

    let scripts = BTreeMap::from([
        (
            1,
            vec![StateMutation::put(
                pair.episode_key.clone(),
                pair.episode.clone(),
            )],
        ),
        (2, vec![StateMutation::delete(pair.episode_key.clone())]),
    ]);
    let mut unidentifiable = episode_delta_ledger(120, scripts, Rc::new(Cell::new(true)));
    let before = unidentifiable.state_image().canonical_bytes();
    let error = unidentifiable
        .apply_block(&trade_block(120, &[1, 2]))
        .expect_err("episode create-delete cannot establish an account-market pair");
    assert_eq!(
        error.reducer_reason_code(),
        Some("position_episode.episode_prior_invalid")
    );
    assert_eq!(unidentifiable.state_image().canonical_bytes(), before);
}

fn frame(value: &[u8]) -> Vec<u8> {
    [
        u64::try_from(value.len()).unwrap().to_be_bytes().as_slice(),
        value,
    ]
    .concat()
}

#[test]
fn final_pair_validation_is_canonically_ordered_and_ignores_untouched_pairs() {
    let low = resolved_pair_fixture(0x01, "perp:LOW", "low-open");
    let high = resolved_pair_fixture(0xf0, "perp:HIGH", "high-open");
    let missing_episode_id = derive_position_episode_id(
        &high.account,
        &high.market,
        &EventId::new("missing-high-open").unwrap(),
        0,
    )
    .unwrap();
    let high_missing_reference =
        current_bytes(high.account, &high.market, &missing_episode_id, "high-open");
    let scripts = BTreeMap::from([(
        1,
        vec![
            StateMutation::put(high.quantity_key.clone(), high.quantity.clone()),
            StateMutation::put(high.current_key.clone(), high_missing_reference),
            StateMutation::put(low.quantity_key.clone(), low.quantity.clone()),
        ],
    )]);
    let mut ordered = episode_delta_ledger(130, scripts, Rc::new(Cell::new(true)));
    let error = ordered
        .apply_block(&trade_block(130, &[1]))
        .expect_err("lower canonical quantity key must fail first");
    assert_eq!(
        error.reducer_reason_code(),
        Some("position_episode.current_pair_mismatch")
    );

    let validate = Rc::new(Cell::new(false));
    let scripts = BTreeMap::from([
        (
            1,
            [
                low.mutations(),
                vec![StateMutation::put(
                    high.quantity_key.clone(),
                    high.quantity.clone(),
                )],
            ]
            .concat(),
        ),
        (
            2,
            vec![
                StateMutation::delete(low.quantity_key.clone()),
                StateMutation::delete(low.current_key.clone()),
            ],
        ),
    ]);
    let mut bounded = episode_delta_ledger(140, scripts, Rc::clone(&validate));
    bounded.apply_block(&trade_block(140, &[1])).unwrap();
    validate.set(true);
    bounded.apply_block(&trade_block(141, &[2])).unwrap();
    assert!(
        bounded
            .state_image()
            .entries()
            .contains_key(&high.quantity_key)
    );
    assert!(
        !bounded
            .state_image()
            .entries()
            .contains_key(&low.quantity_key)
    );
    assert!(
        !bounded
            .state_image()
            .entries()
            .contains_key(&low.current_key)
    );
}

#[test]
fn raw_entry_and_referenced_delta_limits_fail_independently_and_transiently() {
    let key = StateKey::try_new("test.limit", b"k".to_vec()).unwrap();

    let entry_scripts = BTreeMap::from([(
        1,
        vec![
            StateMutation::put(key.clone(), b"x".to_vec()),
            StateMutation::put(
                StateKey::try_new("test.limit", b"z".to_vec()).unwrap(),
                b"y".to_vec(),
            ),
        ],
    )]);
    let mut entry_limited = scripted_ledger(
        30,
        entry_scripts,
        LedgerLimits::try_new(1, 2, 64, 64, 1_024, 1, 1_024).unwrap(),
    );
    let before = entry_limited.state_image().canonical_bytes();
    let error = entry_limited
        .apply_block(&trade_block(30, &[1]))
        .expect_err("two unique entries must exceed only the normalized entry limit");
    assert_eq!(error.reason_code(), "ledger.mutation_limit_exceeded");
    assert_eq!(entry_limited.state_image().canonical_bytes(), before);

    let raw_scripts = BTreeMap::from([
        (1, vec![StateMutation::put(key.clone(), b"x".to_vec())]),
        (2, vec![StateMutation::put(key.clone(), b"y".to_vec())]),
    ]);
    let mut raw_limited = scripted_ledger(
        40,
        raw_scripts,
        LedgerLimits::try_new(2, 1, 16, 16, 20, 2, 20).unwrap(),
    );
    let before = raw_limited.state_image().canonical_bytes();
    let error = raw_limited
        .apply_block(&trade_block(40, &[1, 2]))
        .expect_err("repeated writes must still count against the raw mutation-byte limit");
    assert_eq!(error.reason_code(), "ledger.mutation_limit_exceeded");
    assert_eq!(raw_limited.state_image().canonical_bytes(), before);

    let referenced_scripts = BTreeMap::from([
        (1, vec![StateMutation::put(key.clone(), b"x".to_vec())]),
        (2, vec![StateMutation::put(key.clone(), b"12345".to_vec())]),
        (3, vec![StateMutation::put(key.clone(), b"y".to_vec())]),
    ]);
    let mut referenced_limited = scripted_ledger(
        50,
        referenced_scripts,
        LedgerLimits::try_new(2, 1, 64, 64, 100, 2, 16).unwrap(),
    );
    referenced_limited
        .apply_block(&trade_block(50, &[1]))
        .unwrap();
    let before = referenced_limited.state_image().canonical_bytes();
    let error = referenced_limited
        .apply_block(&trade_block(51, &[2, 3]))
        .expect_err("transient start plus large final value must breach before later shrink");
    assert_eq!(error.reason_code(), "ledger.mutation_limit_exceeded");
    assert_eq!(referenced_limited.state_image().canonical_bytes(), before);
}

#[test]
fn deleting_a_missing_key_fails_before_delta_validation_or_commit() {
    let missing = StateKey::try_new("test.limit", b"missing".to_vec()).unwrap();
    let mut ledger = scripted_ledger(
        60,
        BTreeMap::from([(1, vec![StateMutation::delete(missing)])]),
        LedgerLimits::production(),
    );
    let before = ledger.state_image().canonical_bytes();

    let error = ledger
        .apply_block(&trade_block(60, &[1]))
        .expect_err("missing deletion must fail");

    assert_eq!(error.reason_code(), "ledger.invalid_mutation");
    assert_eq!(ledger.state_image().canonical_bytes(), before);
}

fn scripted_ledger(
    first_height: u64,
    scripts: BTreeMap<u64, Vec<StateMutation>>,
    limits: LedgerLimits,
) -> CanonicalLedger<ScriptedReducer> {
    CanonicalLedger::try_new(
        ChainId::new("mainnet").unwrap(),
        BlockHeight::new(first_height),
        ScriptedReducer {
            scripts: Rc::new(scripts),
        },
        limits,
    )
    .unwrap()
}

fn episode_delta_ledger(
    first_height: u64,
    scripts: BTreeMap<u64, Vec<StateMutation>>,
    validate: Rc<Cell<bool>>,
) -> CanonicalLedger<EpisodeDeltaReducer> {
    CanonicalLedger::try_new(
        ChainId::new("mainnet").unwrap(),
        BlockHeight::new(first_height),
        EpisodeDeltaReducer {
            scripts: Rc::new(scripts),
            episode: CanonicalPositionEpisodeReducerV1,
            validate,
        },
        LedgerLimits::production(),
    )
    .unwrap()
}

fn resolved_pair_fixture(account_byte: u8, market: &str, anchor: &str) -> ResolvedPairFixture {
    let account = Address::from_bytes([account_byte; 20]);
    let market = MarketId::new(market).unwrap();
    let anchor_id = EventId::new(anchor).unwrap();
    let episode_id = derive_position_episode_id(&account, &market, &anchor_id, 0).unwrap();
    let quantity_key = PositionQuantityCurrentRecordV1::state_key(&account, &market).unwrap();
    let current_key = PositionEpisodeCurrentRecordV1::state_key(&account, &market).unwrap();
    let episode_key = PositionEpisodeRecordV1::state_key(&episode_id).unwrap();
    let quantity = quantity_current_bytes(account, &market, "1.00000000");
    let current = current_bytes(account, &market, &episode_id, anchor);
    let episode = episode_bytes(account, &market, &episode_id, anchor);
    PositionQuantityCurrentRecordV1::decode_at(&quantity_key, &quantity).unwrap();
    PositionEpisodeCurrentRecordV1::decode_at(&current_key, &current).unwrap();
    PositionEpisodeRecordV1::decode_at(&episode_key, &episode).unwrap();
    ResolvedPairFixture {
        account,
        market,
        quantity_key,
        current_key,
        episode_key,
        quantity,
        current,
        episode,
    }
}

fn quantity_current_bytes(account: Address, market: &MarketId, quantity: &str) -> Vec<u8> {
    format!(
        "{{\"schema\":\"hyperliquid-alpha-desk/position-quantity-current/v1\",\"account_id\":\"{}\",\"market_id\":\"{}\",\"known_quantity\":\"{quantity}\",\"first_anchor_event_id\":\"seed-open\",\"last_event_id\":\"seed-open\",\"last_block_height\":19}}",
        account.to_api_string(),
        market.as_str()
    )
    .into_bytes()
}

fn current_bytes(
    account: Address,
    market: &MarketId,
    episode_id: &PositionEpisodeId,
    last_event_id: &str,
) -> Vec<u8> {
    format!(
        "{{\"schema\":\"hyperliquid-alpha-desk/position-episode-current/v1\",\"account_id\":\"{}\",\"market_id\":\"{}\",\"episode_id\":\"{}\",\"attribution_resolution\":\"resolved\",\"last_event_id\":\"{last_event_id}\",\"last_block_height\":19}}",
        account.to_api_string(),
        market.as_str(),
        episode_id.as_str()
    )
    .into_bytes()
}

fn unknown_quantity_bytes(account: Address, market: &MarketId) -> Vec<u8> {
    format!(
        "{{\"schema\":\"hyperliquid-alpha-desk/position-quantity-current/v1\",\"account_id\":\"{}\",\"market_id\":\"{}\",\"known_quantity\":null,\"first_anchor_event_id\":\"seed-open\",\"last_event_id\":\"seed-open\",\"last_block_height\":19}}",
        account.to_api_string(),
        market.as_str()
    )
    .into_bytes()
}

fn unresolved_current_bytes(account: Address, market: &MarketId, resolution: &str) -> Vec<u8> {
    format!(
        "{{\"schema\":\"hyperliquid-alpha-desk/position-episode-current/v1\",\"account_id\":\"{}\",\"market_id\":\"{}\",\"episode_id\":null,\"attribution_resolution\":\"{resolution}\",\"last_event_id\":\"seed-open\",\"last_block_height\":19}}",
        account.to_api_string(),
        market.as_str()
    )
    .into_bytes()
}

fn episode_bytes(
    account: Address,
    market: &MarketId,
    episode_id: &PositionEpisodeId,
    anchor: &str,
) -> Vec<u8> {
    format!(
        "{{\"schema\":\"hyperliquid-alpha-desk/position-episode/v1\",\"episode_id\":\"{}\",\"account_id\":\"{}\",\"market_id\":\"{}\",\"opening_anchor_event_id\":\"{anchor}\",\"opening_leg_ordinal\":0,\"opening_position\":\"0.00000000\",\"close_event_id\":null,\"close_cause\":null,\"completeness\":\"complete_from_flat\",\"buy_quantity\":\"1.00000000\",\"buy_notional\":\"100\",\"sell_quantity\":\"0.00000000\",\"sell_notional\":\"0\",\"funding_paid\":\"0\",\"funding_received\":\"0\",\"status\":\"open\",\"last_event_id\":\"{anchor}\",\"last_block_height\":19}}",
        episode_id.as_str(),
        account.to_api_string(),
        market.as_str()
    )
    .into_bytes()
}

fn closed_episode_bytes(
    account: Address,
    market: &MarketId,
    episode_id: &PositionEpisodeId,
    anchor: &str,
) -> Vec<u8> {
    format!(
        "{{\"schema\":\"hyperliquid-alpha-desk/position-episode/v1\",\"episode_id\":\"{}\",\"account_id\":\"{}\",\"market_id\":\"{}\",\"opening_anchor_event_id\":\"{anchor}\",\"opening_leg_ordinal\":0,\"opening_position\":\"0.00000000\",\"close_event_id\":\"closed-trade\",\"close_cause\":\"trade_flat\",\"completeness\":\"complete_from_flat\",\"buy_quantity\":\"1.00000000\",\"buy_notional\":\"100\",\"sell_quantity\":\"1.00000000\",\"sell_notional\":\"100\",\"funding_paid\":\"0\",\"funding_received\":\"0\",\"status\":\"closed\",\"last_event_id\":\"closed-trade\",\"last_block_height\":20}}",
        episode_id.as_str(),
        account.to_api_string(),
        market.as_str()
    )
    .into_bytes()
}

fn trade_block(height: u64, seeds: &[u64]) -> BlockEnvelope {
    let block_time = ProtocolTime::from_unix_micros(height as i64).unwrap();
    let events = seeds
        .iter()
        .enumerate()
        .map(|(index, seed)| {
            let payload = EventPayload::TradeMatched(TradeMatched::without_identities(
                Price::from_str("100").unwrap(),
                Quantity::from_str("1").unwrap(),
                *seed,
            ));
            let payload_hash = *blake3::hash(&payload.encode_to_vec().unwrap()).as_bytes();
            CanonicalEventEnvelope::from_input(CanonicalEventInput {
                schema_version: "1.0.0".to_owned(),
                chain_id: ChainId::new("mainnet").unwrap(),
                block_height: BlockHeight::new(height),
                block_time,
                transaction_id: TransactionId::new(format!("tx-{height}-{index}")).unwrap(),
                transaction_index: u32::try_from(index).unwrap(),
                canonical_event_index: 0,
                market_ids: vec![MarketId::new("perp:BTC").unwrap()],
                account_ids: vec![
                    Address::from_bytes([0x11; 20]),
                    Address::from_bytes([0x22; 20]),
                ],
                source_evidence: vec![
                    SourceEvidence::try_new_indexed(
                        SourceId::new("test-primary").unwrap(),
                        "v1",
                        format!("{height}:{index}"),
                        payload_hash,
                        0,
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
        })
        .collect();
    BlockEnvelope::try_new(
        ChainId::new("mainnet").unwrap(),
        BlockHeight::new(height),
        block_time,
        ConfirmationClass::CommittedPrimary,
        events,
        BTreeMap::from([(
            SourceId::new("test-primary").unwrap(),
            *blake3::hash(&height.to_be_bytes()).as_bytes(),
        )]),
    )
    .unwrap()
}

fn empty_block(height: u64) -> BlockEnvelope {
    BlockEnvelope::try_new(
        ChainId::new("mainnet").unwrap(),
        BlockHeight::new(height),
        ProtocolTime::from_unix_micros(height as i64).unwrap(),
        ConfirmationClass::CommittedPrimary,
        Vec::new(),
        BTreeMap::from([(
            SourceId::new("test-primary").unwrap(),
            *blake3::hash(&height.to_be_bytes()).as_bytes(),
        )]),
    )
    .unwrap()
}
