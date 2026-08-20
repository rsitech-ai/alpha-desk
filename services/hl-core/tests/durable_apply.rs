use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
};

use canonical_events::{BlockEnvelope, ConfirmationClass};
use canonical_ledger::{
    ApplyContext, CanonicalLedger, CorrectionRecord, EventReducer, LedgerLimits, ReducerError,
    StateMutation, StateView,
};
use domain_types::{BlockHeight, ChainId, ProtocolTime, SourceId};
use hl_core::{
    DurableApplyError, DurableApplyOutcome, apply_block_durably, ingest_correction_record,
};
use storage_ports::{
    AtomicStateCommit, AtomicStateStore, StateCommitDisposition, StateCommitReceipt,
    StateStoreError,
};

#[derive(Debug, Clone, Copy)]
struct EmptyReducer;

impl EventReducer for EmptyReducer {
    fn reducer_set_version(&self) -> &str {
        "durable-apply-test@1.0.0"
    }

    fn supports(&self, _event: &canonical_events::CanonicalEventEnvelope) -> bool {
        false
    }

    fn reduce(
        &self,
        _state: &StateView<'_>,
        _event: &canonical_events::CanonicalEventEnvelope,
        _context: &ApplyContext<'_>,
    ) -> Result<Vec<StateMutation>, ReducerError> {
        unreachable!("empty block")
    }
}

#[test]
fn storage_failure_leaves_the_visible_ledger_unchanged() {
    let mut ledger = ledger();
    let before = ledger.state_image().canonical_bytes();
    let store = RecordingStore::failing();

    let error =
        apply_block_durably(&mut ledger, &store, &block()).expect_err("injected storage failure");

    assert!(matches!(error, DurableApplyError::Store(_)));
    assert_eq!(error.reason_code(), "core.state_store");
    assert_eq!(ledger.state_image().canonical_bytes(), before);
    assert!(ledger.checkpoint().is_none());
    assert_eq!(store.calls.get(), 1);
}

#[test]
fn mismatched_storage_receipt_leaves_the_visible_ledger_unchanged() {
    let mut ledger = ledger();
    let before = ledger.state_image().canonical_bytes();
    let store = RecordingStore::mismatching();

    let error = apply_block_durably(&mut ledger, &store, &block()).expect_err("mismatched receipt");

    assert!(matches!(error, DurableApplyError::ReceiptMismatch));
    assert_eq!(error.reason_code(), "core.state_receipt_mismatch");
    assert_eq!(ledger.state_image().canonical_bytes(), before);
    assert!(ledger.checkpoint().is_none());
}

#[test]
fn successful_atomic_store_commit_precedes_visible_ledger_advance() {
    let mut ledger = ledger();
    let store = RecordingStore::successful();

    let outcome = apply_block_durably(&mut ledger, &store, &block()).expect("durable apply");

    let DurableApplyOutcome::Applied { delta, disposition } = outcome else {
        panic!("new block must apply");
    };
    assert!(matches!(disposition, StateCommitDisposition::Committed(_)));
    assert_eq!(ledger.state_hash(), delta.after_state_hash());
    assert_eq!(store.calls.get(), 1);
    assert_eq!(
        store.observed_before.borrow().as_ref(),
        Some(&delta.before_state_hash())
    );

    let duplicate = apply_block_durably(&mut ledger, &store, &block()).expect("duplicate delivery");
    assert!(matches!(duplicate, DurableApplyOutcome::AlreadyApplied(_)));
    assert_eq!(store.calls.get(), 1);
}

#[test]
fn correction_ingest_and_apply_fail_closed_without_touching_the_store() {
    let mut ledger = ledger();
    let before = ledger.state_image().canonical_bytes();
    let store = RecordingStore::successful();
    let corrected = classified_block(ConfirmationClass::Corrected);
    let record = CorrectionRecord::try_from_block(&corrected).expect("typed correction");

    let ingest = ingest_correction_record(&ledger, &store, &record)
        .expect_err("typed correction ingest denied");
    assert_eq!(ingest.reason_code(), "ledger.correction_unimplemented");
    assert_eq!(store.calls.get(), 0);
    assert_eq!(ledger.state_image().canonical_bytes(), before);
    assert!(ledger.checkpoint().is_none());

    let apply = apply_block_durably(&mut ledger, &store, &corrected)
        .expect_err("corrected block durable apply denied");
    assert_eq!(apply.reason_code(), "ledger.correction_unimplemented");
    assert_eq!(store.calls.get(), 0);
    assert_eq!(ledger.state_image().canonical_bytes(), before);
    assert!(ledger.checkpoint().is_none());
}

struct RecordingStore {
    behavior: StoreBehavior,
    calls: Cell<u64>,
    observed_before: RefCell<Option<[u8; 32]>>,
}

#[derive(Clone, Copy)]
enum StoreBehavior {
    Fail,
    Mismatch,
    Succeed,
}

impl RecordingStore {
    fn failing() -> Self {
        Self::new(StoreBehavior::Fail)
    }

    fn mismatching() -> Self {
        Self::new(StoreBehavior::Mismatch)
    }

    fn successful() -> Self {
        Self::new(StoreBehavior::Succeed)
    }

    fn new(behavior: StoreBehavior) -> Self {
        Self {
            behavior,
            calls: Cell::new(0),
            observed_before: RefCell::new(None),
        }
    }
}

impl AtomicStateStore for RecordingStore {
    fn commit(
        &self,
        commit: &AtomicStateCommit<'_>,
    ) -> Result<StateCommitDisposition, StateStoreError> {
        self.calls.set(self.calls.get() + 1);
        self.observed_before
            .replace(Some(commit.before_state_hash()));
        if matches!(self.behavior, StoreBehavior::Fail) {
            return Err(StateStoreError::Io("injected test failure"));
        }
        let state_hash = if matches!(self.behavior, StoreBehavior::Mismatch) {
            [0x55; 32]
        } else {
            commit.after_state_hash()
        };
        Ok(StateCommitDisposition::Committed(StateCommitReceipt::new(
            commit.block_height(),
            commit.canonical_block_hash(),
            state_hash,
        )))
    }

    fn load_latest(
        &self,
        _limits: canonical_ledger::StateImageLimits,
    ) -> Result<Option<canonical_ledger::StateImage>, StateStoreError> {
        Ok(None)
    }
}

fn ledger() -> CanonicalLedger<EmptyReducer> {
    CanonicalLedger::try_new(
        ChainId::new("mainnet").expect("chain"),
        BlockHeight::new(200),
        EmptyReducer,
        LedgerLimits::production(),
    )
    .expect("ledger")
}

fn block() -> BlockEnvelope {
    classified_block(ConfirmationClass::CommittedPrimary)
}

fn classified_block(confirmation: ConfirmationClass) -> BlockEnvelope {
    BlockEnvelope::try_new(
        ChainId::new("mainnet").expect("chain"),
        BlockHeight::new(200),
        ProtocolTime::from_unix_micros(200).expect("time"),
        confirmation,
        Vec::new(),
        BTreeMap::from([(
            SourceId::new("durable-apply-test").expect("source"),
            *blake3::hash(b"block-200").as_bytes(),
        )]),
    )
    .expect("block")
}
