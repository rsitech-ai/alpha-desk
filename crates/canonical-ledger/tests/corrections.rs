use std::collections::BTreeMap;

use canonical_events::{BlockEnvelope, ConfirmationClass};
use canonical_ledger::{
    CanonicalLedger, ConfirmationAdmission, LedgerLimits, WatermarkOnlyReducerV1,
    admit_confirmation,
};
use domain_types::{BlockHeight, ChainId, ProtocolTime, SourceId};

#[test]
fn confirmation_admission_is_exhaustive_and_corrections_are_unimplemented() {
    assert_eq!(
        admit_confirmation(ConfirmationClass::CommittedPrimary),
        ConfirmationAdmission::Committed
    );
    assert_eq!(
        admit_confirmation(ConfirmationClass::CommittedIndependent),
        ConfirmationAdmission::Committed
    );
    assert_eq!(
        admit_confirmation(ConfirmationClass::Corrected),
        ConfirmationAdmission::CorrectionUnimplemented
    );
    for class in [
        ConfirmationClass::ProvisionalSource,
        ConfirmationClass::ReconciledSnapshot,
        ConfirmationClass::Expired,
    ] {
        assert_eq!(
            admit_confirmation(class),
            ConfirmationAdmission::NonCommitted
        );
    }
}

#[test]
fn corrected_blocks_are_refused_without_mutating_state_and_redelivery_is_idempotent() {
    let mut ledger = ledger(10);
    let before = ledger.state_image().canonical_bytes();
    let corrected = classified_block(10, ConfirmationClass::Corrected);

    let first = ledger
        .apply_block(&corrected)
        .expect_err("correction must be denied");
    assert_eq!(first.reason_code(), "ledger.correction_unimplemented");
    assert_eq!(ledger.state_image().canonical_bytes(), before);
    assert!(ledger.checkpoint().is_none());

    let second = ledger
        .apply_block(&corrected)
        .expect_err("redelivered correction must stay denied");
    assert_eq!(second.reason_code(), "ledger.correction_unimplemented");
    assert_eq!(ledger.state_image().canonical_bytes(), before);
    assert!(ledger.checkpoint().is_none());
}

#[test]
fn non_committed_non_correction_classes_stay_fail_closed() {
    for class in [
        ConfirmationClass::ProvisionalSource,
        ConfirmationClass::ReconciledSnapshot,
        ConfirmationClass::Expired,
    ] {
        let mut ledger = ledger(20);
        let before = ledger.state_image().canonical_bytes();
        let error = ledger
            .apply_block(&classified_block(20, class))
            .expect_err("non-committed block");
        assert_eq!(error.reason_code(), "ledger.non_committed_block");
        assert_eq!(ledger.state_image().canonical_bytes(), before);
    }
}

fn ledger(height: u64) -> CanonicalLedger<WatermarkOnlyReducerV1> {
    CanonicalLedger::try_new(
        ChainId::new("mainnet").expect("chain"),
        BlockHeight::new(height),
        WatermarkOnlyReducerV1,
        LedgerLimits::production(),
    )
    .expect("ledger")
}

fn classified_block(height: u64, confirmation: ConfirmationClass) -> BlockEnvelope {
    BlockEnvelope::try_new(
        ChainId::new("mainnet").expect("chain"),
        BlockHeight::new(height),
        ProtocolTime::from_unix_micros(height as i64).expect("time"),
        confirmation,
        Vec::new(),
        BTreeMap::from([(
            SourceId::new("correction-test").expect("source"),
            [height as u8; 32],
        )]),
    )
    .expect("block")
}
