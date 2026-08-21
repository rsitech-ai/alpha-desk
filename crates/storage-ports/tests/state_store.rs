use std::collections::BTreeMap;

use canonical_events::{BlockEnvelope, ConfirmationClass};
use canonical_ledger::{
    ApplyContext, CanonicalLedger, EventReducer, LedgerLimits, PrepareOutcome, ReducerError,
    StateMutation, StateView,
};
use domain_types::{BlockHeight, ChainId, ProtocolTime, SourceId};
use storage_ports::AtomicStateCommit;

#[derive(Debug, Clone, Copy)]
struct EmptyReducer;

impl EventReducer for EmptyReducer {
    fn reducer_set_version(&self) -> &str {
        "state-store-test@1.0.0"
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
fn atomic_commit_binds_one_exact_prepared_state_transition() {
    let ledger = ledger();
    let PrepareOutcome::Ready(prepared) = ledger.prepare_block(&block(100)).expect("prepare")
    else {
        panic!("new block must prepare");
    };

    let commit =
        AtomicStateCommit::try_new(prepared.delta(), prepared.state_image()).expect("contract");

    assert_eq!(commit.before_state_hash(), ledger.state_hash());
    assert_eq!(
        commit.after_state_hash(),
        prepared.state_image().state_hash()
    );
    assert_eq!(commit.block_height(), BlockHeight::new(100));
    assert_eq!(
        commit.canonical_block_hash(),
        block(100).canonical_block_hash()
    );
    assert_eq!(commit.reducer_set_version(), "state-store-test@1.0.0");
}

#[test]
fn column_family_schema_is_exact_and_rebuilds_on_drift() {
    storage_ports::admit_column_family_schema(storage_ports::STATE_STORE_CFS).unwrap();
    let error = storage_ports::admit_column_family_schema(&["meta"]).unwrap_err();
    assert_eq!(error.reason_code(), "state_store.rebuild_required");
}

#[test]
fn atomic_commit_rejects_a_delta_and_state_image_from_different_transitions() {
    let ledger = ledger();
    let PrepareOutcome::Ready(first) = ledger.prepare_block(&block(100)).expect("first") else {
        panic!("new block must prepare");
    };
    let PrepareOutcome::Ready(other) = ledger
        .prepare_block(&block_with_time(100, 101))
        .expect("other")
    else {
        panic!("new block must prepare");
    };

    let error = AtomicStateCommit::try_new(first.delta(), other.state_image())
        .expect_err("mismatched state transition");

    assert_eq!(error.reason_code(), "state_store.invalid_commit");
}

fn ledger() -> CanonicalLedger<EmptyReducer> {
    CanonicalLedger::try_new(
        ChainId::new("mainnet").expect("chain"),
        BlockHeight::new(100),
        EmptyReducer,
        LedgerLimits::production(),
    )
    .expect("ledger")
}

fn block(height: u64) -> BlockEnvelope {
    block_with_time(height, height as i64)
}

fn block_with_time(height: u64, time: i64) -> BlockEnvelope {
    BlockEnvelope::try_new(
        ChainId::new("mainnet").expect("chain"),
        BlockHeight::new(height),
        ProtocolTime::from_unix_micros(time).expect("time"),
        ConfirmationClass::CommittedPrimary,
        Vec::new(),
        BTreeMap::from([(
            SourceId::new("state-store-test").expect("source"),
            *blake3::hash(&time.to_be_bytes()).as_bytes(),
        )]),
    )
    .expect("block")
}
