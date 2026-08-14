use std::{cell::Cell, collections::BTreeMap, rc::Rc};

use canonical_events::{
    BlockEnvelope, CanonicalEventEnvelope, CanonicalEventInput, ConfirmationClass, EventKind,
    EventPayload, SourceEvidence, TradeMatched,
};
use canonical_ledger::{
    ApplyContext, ApplyOutcome, CanonicalLedger, EventReducer, LedgerLimits,
    MAX_BLOCK_DELTA_ENTRIES, MAX_BLOCK_DELTA_REFERENCED_BYTES, PrepareOutcome, ReducerError,
    StateKey, StateMutation, StateView,
};
use domain_types::{
    Address, BlockHeight, ChainId, KnownTime, MarketId, Price, ProtocolTime, Quantity, SourceId,
    TransactionId,
};

#[derive(Debug, Clone, Copy)]
struct TradeReducer {
    fail_on_seed: Option<u64>,
    fail_validation: bool,
}

impl TradeReducer {
    const fn accepting() -> Self {
        Self {
            fail_on_seed: None,
            fail_validation: false,
        }
    }
}

impl EventReducer for TradeReducer {
    fn reducer_set_version(&self) -> &str {
        "test-trades@1.0.0"
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
            return Err(reducer_error("test.unexpected_payload"));
        };
        if self.fail_on_seed == Some(trade.deterministic_seed) {
            return Err(reducer_error("test.injected_failure"));
        }
        Ok(vec![StateMutation::put(
            StateKey::try_new("test.trade", event.event_id().as_str().as_bytes().to_vec())
                .expect("valid state key"),
            event.payload_hash().to_vec(),
        )])
    }

    fn validate_block(
        &self,
        _state: &StateView<'_>,
        _context: &ApplyContext<'_>,
    ) -> Result<(), ReducerError> {
        if self.fail_validation {
            Err(reducer_error("test.invariant_failed"))
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct RejectingReducer;

impl EventReducer for RejectingReducer {
    fn reducer_set_version(&self) -> &str {
        "reject-all@1.0.0"
    }

    fn supports(&self, _event: &CanonicalEventEnvelope) -> bool {
        false
    }

    fn reduce(
        &self,
        _state: &StateView<'_>,
        _event: &CanonicalEventEnvelope,
        _context: &ApplyContext<'_>,
    ) -> Result<Vec<StateMutation>, ReducerError> {
        unreachable!("unsupported events must never reach the reducer")
    }
}

#[derive(Debug, Clone)]
struct LegacyValidationCounter {
    calls: Rc<Cell<u32>>,
}

impl EventReducer for LegacyValidationCounter {
    fn reducer_set_version(&self) -> &str {
        "legacy-validation-counter@1.0.0"
    }

    fn supports(&self, _event: &CanonicalEventEnvelope) -> bool {
        false
    }

    fn reduce(
        &self,
        _state: &StateView<'_>,
        _event: &CanonicalEventEnvelope,
        _context: &ApplyContext<'_>,
    ) -> Result<Vec<StateMutation>, ReducerError> {
        unreachable!("empty block has no events")
    }

    fn validate_block(
        &self,
        _state: &StateView<'_>,
        _context: &ApplyContext<'_>,
    ) -> Result<(), ReducerError> {
        self.calls.set(self.calls.get() + 1);
        Ok(())
    }
}

#[test]
fn empty_committed_blocks_advance_contiguously_with_identical_state_hashes() {
    let mut left = ledger(10, RejectingReducer);
    let mut right = ledger(10, RejectingReducer);

    for height in [10, 11] {
        let block = empty_block("mainnet", height, height as i64);
        let left_delta = applied(left.apply_block(&block).expect("left apply"));
        let right_delta = applied(right.apply_block(&block).expect("right apply"));
        assert_eq!(left_delta, right_delta);
        assert_eq!(left.state_hash(), right.state_hash());
        assert_eq!(
            left.state_image().canonical_bytes(),
            right.state_image().canonical_bytes()
        );
    }

    let checkpoint = left.checkpoint().expect("checkpoint");
    assert_eq!(checkpoint.block_height(), BlockHeight::new(11));
    assert_eq!(checkpoint.state_hash(), left.state_hash());
    assert!(left.state_image().entries().is_empty());
    assert_eq!(
        left.state_hash(),
        [
            122, 133, 255, 92, 12, 65, 243, 154, 104, 239, 9, 24, 92, 102, 1, 126, 102, 64, 58,
            139, 12, 12, 231, 124, 63, 15, 110, 45, 205, 191, 95, 209,
        ]
    );
}

#[test]
fn empty_block_calls_legacy_validation_exactly_once_through_the_delta_default() {
    let calls = Rc::new(Cell::new(0));
    let mut ledger = ledger(
        15,
        LegacyValidationCounter {
            calls: Rc::clone(&calls),
        },
    );

    ledger.apply_block(&empty_block("mainnet", 15, 15)).unwrap();

    assert_eq!(calls.get(), 1);
}

#[test]
fn a_late_reducer_failure_rolls_back_the_entire_block() {
    let mut ledger = ledger(
        20,
        TradeReducer {
            fail_on_seed: Some(2),
            fail_validation: false,
        },
    );
    let before_hash = ledger.state_hash();
    let before_bytes = ledger.state_image().canonical_bytes();
    let block = trade_block(20, &[1, 2], ConfirmationClass::CommittedPrimary);

    let error = ledger.apply_block(&block).expect_err("second event fails");

    assert_eq!(error.reason_code(), "ledger.reducer_failed");
    assert_eq!(error.reducer_reason_code(), Some("test.injected_failure"));
    assert_eq!(ledger.state_hash(), before_hash);
    assert_eq!(ledger.state_image().canonical_bytes(), before_bytes);
    assert!(ledger.checkpoint().is_none());
    assert!(ledger.state_image().entries().is_empty());
}

#[test]
fn block_invariant_failure_rolls_back_all_prepared_mutations() {
    let mut ledger = ledger(
        25,
        TradeReducer {
            fail_on_seed: None,
            fail_validation: true,
        },
    );
    let before = ledger.state_image().canonical_bytes();

    let error = ledger
        .apply_block(&trade_block(25, &[1], ConfirmationClass::CommittedPrimary))
        .expect_err("block invariant");

    assert_eq!(error.reason_code(), "ledger.reducer_failed");
    assert_eq!(error.reducer_reason_code(), Some("test.invariant_failed"));
    assert_eq!(ledger.state_image().canonical_bytes(), before);
    assert!(ledger.checkpoint().is_none());
}

#[test]
fn duplicate_delivery_is_idempotent_and_conflicting_same_height_is_rejected() {
    let mut ledger = ledger(30, RejectingReducer);
    let block = empty_block("mainnet", 30, 30);
    let applied = applied(ledger.apply_block(&block).expect("first delivery"));
    let after_first = ledger.state_hash();

    let duplicate = ledger.apply_block(&block).expect("duplicate delivery");
    let ApplyOutcome::AlreadyApplied(checkpoint) = duplicate else {
        panic!("expected idempotent duplicate disposition");
    };
    assert_eq!(checkpoint.state_hash(), applied.after_state_hash());
    assert_eq!(ledger.state_hash(), after_first);

    let conflicting = empty_block("mainnet", 30, 31);
    let error = ledger
        .apply_block(&conflicting)
        .expect_err("same height with different canonical hash");
    assert_eq!(error.reason_code(), "ledger.canonical_divergence");
    assert_eq!(ledger.state_hash(), after_first);
}

#[test]
fn wrong_chain_gap_and_provisional_blocks_fail_before_state_changes() {
    let cases = [
        (empty_block("other-chain", 40, 40), "ledger.chain_mismatch"),
        (
            empty_block("mainnet", 41, 41),
            "ledger.height_discontinuity",
        ),
        (
            empty_confirmed_block("mainnet", 40, 40, ConfirmationClass::ProvisionalSource),
            "ledger.non_committed_block",
        ),
    ];

    for (block, reason) in cases {
        let mut ledger = ledger(40, RejectingReducer);
        let before = ledger.state_image().canonical_bytes();
        let error = ledger.apply_block(&block).expect_err(reason);
        assert_eq!(error.reason_code(), reason);
        assert_eq!(ledger.state_image().canonical_bytes(), before);
    }
}

#[test]
fn confirmation_boundary_covers_every_class() {
    for class in [
        ConfirmationClass::ProvisionalSource,
        ConfirmationClass::CommittedPrimary,
        ConfirmationClass::CommittedIndependent,
        ConfirmationClass::ReconciledSnapshot,
        ConfirmationClass::Corrected,
        ConfirmationClass::Expired,
    ] {
        let mut ledger = ledger(40, RejectingReducer);
        let before = ledger.state_image().canonical_bytes();
        let block = empty_confirmed_block("mainnet", 40, 40, class);
        match class {
            ConfirmationClass::CommittedPrimary | ConfirmationClass::CommittedIndependent => {
                let outcome = ledger
                    .apply_block(&block)
                    .expect("empty committed blocks still admit");
                assert!(
                    matches!(outcome, ApplyOutcome::Applied(_)),
                    "{class:?} empty committed block must apply, not already-applied"
                );
                assert_eq!(
                    ledger.checkpoint().expect("checkpoint").block_height(),
                    BlockHeight::new(40)
                );
            }
            ConfirmationClass::ProvisionalSource
            | ConfirmationClass::ReconciledSnapshot
            | ConfirmationClass::Corrected
            | ConfirmationClass::Expired => {
                let error = ledger
                    .apply_block(&block)
                    .expect_err("non-committed lanes fail closed");
                assert_eq!(
                    error.reason_code(),
                    "ledger.non_committed_block",
                    "{class:?} must not blur into the committed ledger lane"
                );
                assert_eq!(ledger.state_image().canonical_bytes(), before);
            }
        }
    }
}

#[test]
fn unsupported_kind_or_schema_is_quarantined_without_invoking_the_reducer() {
    let mut ledger = ledger(50, RejectingReducer);
    let block = trade_block(50, &[7], ConfirmationClass::CommittedIndependent);
    let before = ledger.state_hash();

    let error = ledger.apply_block(&block).expect_err("unsupported event");

    assert_eq!(error.reason_code(), "ledger.unsupported_event");
    assert_eq!(error.event_kind(), Some(EventKind::TradeMatched));
    assert_eq!(error.schema_version(), Some("1.0.0"));
    assert_eq!(ledger.state_hash(), before);
    assert!(ledger.checkpoint().is_none());
}

#[test]
fn canonical_state_order_is_independent_of_reducer_mutation_emission_order() {
    let mut forward = ledger(55, OrderingReducer { reverse: false });
    let mut reverse = ledger(55, OrderingReducer { reverse: true });
    let block = trade_block(55, &[1], ConfirmationClass::CommittedPrimary);

    forward.apply_block(&block).expect("forward");
    reverse.apply_block(&block).expect("reverse");

    assert_eq!(forward.state_hash(), reverse.state_hash());
    assert_eq!(
        forward.state_image().canonical_bytes(),
        reverse.state_image().canonical_bytes()
    );
}

#[derive(Debug, Clone, Copy)]
struct OrderingReducer {
    reverse: bool,
}

impl EventReducer for OrderingReducer {
    fn reducer_set_version(&self) -> &str {
        "ordering-test@1.0.0"
    }

    fn supports(&self, event: &CanonicalEventEnvelope) -> bool {
        event.event_kind() == EventKind::TradeMatched && event.schema_version() == "1.0.0"
    }

    fn reduce(
        &self,
        _state: &StateView<'_>,
        _event: &CanonicalEventEnvelope,
        _context: &ApplyContext<'_>,
    ) -> Result<Vec<StateMutation>, ReducerError> {
        let mut mutations = vec![
            StateMutation::put(
                StateKey::try_new("test.order", b"a".to_vec()).expect("key"),
                b"first".to_vec(),
            ),
            StateMutation::put(
                StateKey::try_new("test.order", b"b".to_vec()).expect("key"),
                b"second".to_vec(),
            ),
        ];
        if self.reverse {
            mutations.reverse();
        }
        Ok(mutations)
    }
}

#[test]
fn mutation_limits_fail_closed_before_the_candidate_state_is_committed() {
    let valid = [8, 2, 64, 64, 1_024, 16, 512];
    for index in 0..valid.len() {
        let mut invalid = valid;
        invalid[index] = 0;
        let error = LedgerLimits::try_new(
            invalid[0], invalid[1], invalid[2], invalid[3], invalid[4], invalid[5], invalid[6],
        )
        .expect_err("all seven limits must be nonzero");
        assert_eq!(error.reason_code(), "ledger.invalid_limits");
    }
    let too_many_events = usize::try_from(u64::from(u32::MAX) + 1).unwrap();
    assert!(matches!(
        LedgerLimits::try_new(too_many_events, 1, 64, 64, 1_024, 1, 512),
        Err(canonical_ledger::LedgerError::InvalidLimits)
    ));
    assert!(matches!(
        LedgerLimits::try_new(
            usize::try_from(u32::MAX).unwrap(),
            usize::MAX,
            64,
            64,
            usize::MAX,
            1,
            512
        ),
        Err(canonical_ledger::LedgerError::InvalidLimits)
    ));
    assert!(LedgerLimits::try_new(2, 3, 64, 64, 1_024, 7, 512).is_err());
    assert!(LedgerLimits::try_new(8, 2, 64, 64, 1_024, 16, 1_025).is_err());
    assert!(LedgerLimits::try_new(8, 2, 1_025, 64, 1_024, 16, 512).is_err());
    assert!(LedgerLimits::try_new(8, 2, 64, 1_025, 1_024, 16, 512).is_err());

    let production = LedgerLimits::production();
    assert_eq!(
        production.max_block_delta_entries(),
        MAX_BLOCK_DELTA_ENTRIES
    );
    assert_eq!(
        production.max_block_delta_referenced_bytes(),
        MAX_BLOCK_DELTA_REFERENCED_BYTES
    );
    let custom_above_defaults = LedgerLimits::try_new(
        MAX_BLOCK_DELTA_ENTRIES + 1,
        2,
        64,
        64,
        MAX_BLOCK_DELTA_REFERENCED_BYTES + 1,
        MAX_BLOCK_DELTA_ENTRIES + 1,
        MAX_BLOCK_DELTA_REFERENCED_BYTES + 1,
    )
    .expect("production defaults are not global hard caps");
    assert_eq!(
        custom_above_defaults.max_block_delta_entries(),
        MAX_BLOCK_DELTA_ENTRIES + 1
    );

    let mut ledger = CanonicalLedger::try_new(
        ChainId::new("mainnet").expect("chain"),
        BlockHeight::new(60),
        TradeReducer::accepting(),
        LedgerLimits::try_new(8, 1, 8, 8, 16, 8, 16).expect("limits"),
    )
    .expect("ledger");
    let before = ledger.state_hash();

    let error = ledger
        .apply_block(&trade_block(60, &[9], ConfirmationClass::CommittedPrimary))
        .expect_err("payload hash exceeds configured value bound");

    assert_eq!(error.reason_code(), "ledger.mutation_limit_exceeded");
    assert_eq!(ledger.state_hash(), before);
    assert!(ledger.state_image().entries().is_empty());
}

#[test]
fn reducer_version_and_state_keys_reject_ambiguous_inputs() {
    let bad_version = CanonicalLedger::try_new(
        ChainId::new("mainnet").expect("chain"),
        BlockHeight::new(1),
        InvalidVersionReducer,
        LedgerLimits::production(),
    )
    .expect_err("invalid reducer version");
    assert_eq!(bad_version.reason_code(), "ledger.invalid_reducer_version");

    for namespace in ["", "Test", "test space", ".test", "test."] {
        assert!(StateKey::try_new(namespace, b"id".to_vec()).is_err());
    }
    assert!(StateKey::try_new("test.valid-v1", Vec::new()).is_err());
}

#[test]
fn reducer_version_drift_is_rejected_before_even_an_empty_block_advances() {
    let drift = Rc::new(Cell::new(false));
    let mut ledger = ledger(
        70,
        DriftingReducer {
            drift: Rc::clone(&drift),
        },
    );
    let before = ledger.state_image().canonical_bytes();
    drift.set(true);

    let error = ledger
        .apply_block(&empty_block("mainnet", 70, 70))
        .expect_err("version drift");

    assert_eq!(error.reason_code(), "ledger.reducer_version_drift");
    assert_eq!(ledger.state_image().canonical_bytes(), before);
}

#[test]
fn prepared_block_is_invisible_until_explicit_commit() {
    let mut ledger = ledger(80, RejectingReducer);
    let before_hash = ledger.state_hash();
    let before_bytes = ledger.state_image().canonical_bytes();
    let block = empty_block("mainnet", 80, 80);

    let PrepareOutcome::Ready(prepared) = ledger.prepare_block(&block).expect("prepare") else {
        panic!("new block must prepare");
    };

    assert_eq!(prepared.delta().before_state_hash(), before_hash);
    assert_eq!(
        prepared.state_image().state_hash(),
        prepared.delta().after_state_hash()
    );
    assert_eq!(ledger.state_hash(), before_hash);
    assert_eq!(ledger.state_image().canonical_bytes(), before_bytes);
    assert!(ledger.checkpoint().is_none());

    let delta = ledger.commit_prepared(prepared).expect("commit");
    assert_eq!(ledger.state_hash(), delta.after_state_hash());
    assert_eq!(ledger.state_image().state_hash(), delta.after_state_hash());
    assert_eq!(
        ledger.checkpoint().expect("checkpoint").block_height(),
        BlockHeight::new(80)
    );
}

#[test]
fn stale_prepared_block_cannot_overwrite_newer_visible_state() {
    let mut ledger = ledger(90, RejectingReducer);
    let block = empty_block("mainnet", 90, 90);
    let PrepareOutcome::Ready(first) = ledger.prepare_block(&block).expect("first prepare") else {
        panic!("new block must prepare");
    };
    let PrepareOutcome::Ready(stale) = ledger.prepare_block(&block).expect("second prepare") else {
        panic!("new block must prepare");
    };
    ledger.commit_prepared(first).expect("first commit");
    let committed_hash = ledger.state_hash();

    let error = ledger
        .commit_prepared(stale)
        .expect_err("stale candidate must fail");

    assert_eq!(error.reason_code(), "ledger.prepared_state_drift");
    assert_eq!(ledger.state_hash(), committed_hash);
}

#[derive(Debug, Clone)]
struct DriftingReducer {
    drift: Rc<Cell<bool>>,
}

impl EventReducer for DriftingReducer {
    fn reducer_set_version(&self) -> &str {
        if self.drift.get() {
            "drifting@2.0.0"
        } else {
            "drifting@1.0.0"
        }
    }

    fn supports(&self, _event: &CanonicalEventEnvelope) -> bool {
        false
    }

    fn reduce(
        &self,
        _state: &StateView<'_>,
        _event: &CanonicalEventEnvelope,
        _context: &ApplyContext<'_>,
    ) -> Result<Vec<StateMutation>, ReducerError> {
        unreachable!("empty block has no events")
    }
}

#[derive(Debug, Clone, Copy)]
struct InvalidVersionReducer;

impl EventReducer for InvalidVersionReducer {
    fn reducer_set_version(&self) -> &str {
        " contains spaces "
    }

    fn supports(&self, _event: &CanonicalEventEnvelope) -> bool {
        false
    }

    fn reduce(
        &self,
        _state: &StateView<'_>,
        _event: &CanonicalEventEnvelope,
        _context: &ApplyContext<'_>,
    ) -> Result<Vec<StateMutation>, ReducerError> {
        Ok(Vec::new())
    }
}

fn ledger<R: EventReducer>(first_height: u64, reducer: R) -> CanonicalLedger<R> {
    CanonicalLedger::try_new(
        ChainId::new("mainnet").expect("chain"),
        BlockHeight::new(first_height),
        reducer,
        LedgerLimits::production(),
    )
    .expect("ledger")
}

fn applied(outcome: ApplyOutcome) -> canonical_ledger::StateDelta {
    match outcome {
        ApplyOutcome::Applied(delta) => delta,
        ApplyOutcome::AlreadyApplied(_) => panic!("expected applied block"),
    }
}

fn empty_block(chain: &str, height: u64, time: i64) -> BlockEnvelope {
    empty_confirmed_block(chain, height, time, ConfirmationClass::CommittedPrimary)
}

fn empty_confirmed_block(
    chain: &str,
    height: u64,
    time: i64,
    confirmation: ConfirmationClass,
) -> BlockEnvelope {
    BlockEnvelope::try_new(
        ChainId::new(chain).expect("chain"),
        BlockHeight::new(height),
        ProtocolTime::from_unix_micros(time).expect("time"),
        confirmation,
        Vec::new(),
        source_hashes(height),
    )
    .expect("empty block")
}

fn trade_block(height: u64, seeds: &[u64], confirmation: ConfirmationClass) -> BlockEnvelope {
    let block_time = ProtocolTime::from_unix_micros(height as i64).expect("time");
    let events = seeds
        .iter()
        .enumerate()
        .map(|(index, seed)| trade_event(height, block_time, index as u32, *seed, confirmation))
        .collect();
    BlockEnvelope::try_new(
        ChainId::new("mainnet").expect("chain"),
        BlockHeight::new(height),
        block_time,
        confirmation,
        events,
        source_hashes(height),
    )
    .expect("trade block")
}

fn trade_event(
    height: u64,
    block_time: ProtocolTime,
    transaction_index: u32,
    seed: u64,
    confirmation: ConfirmationClass,
) -> CanonicalEventEnvelope {
    let payload = EventPayload::TradeMatched(TradeMatched::without_identities(
        Price::parse_at_scale("65000", 6).expect("price"),
        Quantity::parse_at_scale("0.01", 8).expect("quantity"),
        seed,
    ));
    let payload_hash = *blake3::hash(&payload.encode_to_vec().expect("payload bytes")).as_bytes();
    CanonicalEventEnvelope::from_input(CanonicalEventInput {
        schema_version: "1.0.0".to_owned(),
        chain_id: ChainId::new("mainnet").expect("chain"),
        block_height: BlockHeight::new(height),
        block_time,
        transaction_id: TransactionId::new(format!("tx-{height}-{transaction_index}"))
            .expect("transaction"),
        transaction_index,
        canonical_event_index: 0,
        market_ids: vec![MarketId::new("perp:BTC").expect("market")],
        account_ids: vec![
            Address::from_bytes([0x11; 20]),
            Address::from_bytes([0x22; 20]),
        ],
        source_evidence: vec![
            SourceEvidence::try_new_indexed(
                SourceId::new("test-primary").expect("source"),
                "v1",
                format!("{height}:{transaction_index}"),
                payload_hash,
                0,
            )
            .expect("evidence"),
        ],
        confirmation_class: confirmation,
        observed_at: KnownTime::from_unix_micros(height as i64).expect("known time"),
        ingested_at: KnownTime::from_unix_micros(height as i64).expect("known time"),
        canonicalized_at: KnownTime::from_unix_micros(height as i64).expect("known time"),
        parser_version: "test-parser-v1".to_owned(),
        payload,
    })
    .expect("event")
}

fn source_hashes(height: u64) -> BTreeMap<SourceId, [u8; 32]> {
    BTreeMap::from([(
        SourceId::new("test-primary").expect("source"),
        *blake3::hash(&height.to_be_bytes()).as_bytes(),
    )])
}

fn reducer_error(reason_code: &'static str) -> ReducerError {
    ReducerError::try_new(reason_code, "deterministic test failure").expect("reducer error")
}
