use domain_types::{BlockHeight, ChainId, KnownTime, ManifestId};
use hl_capture::progress::InMemoryProgressStore;
use storage_ports::{
    ArchivedBlockPlan, CaptureProgressStore, PlannedPublication, ProgressError,
    ProgressRecordDisposition, PublicationAcknowledgement,
};

fn known(micros: i64) -> KnownTime {
    KnownTime::from_unix_micros(micros).expect("known time")
}

fn plan(height: u64, salt: u8) -> ArchivedBlockPlan {
    ArchivedBlockPlan::try_new(
        ChainId::new("mainnet").expect("chain ID"),
        BlockHeight::new(height),
        [salt; 32],
        format!("receipt-{height}-{salt}"),
        ManifestId::new(format!("manifest-{height}-{salt}")).expect("manifest ID"),
        [salt.wrapping_add(1); 32],
        [salt.wrapping_add(2); 32],
        [salt.wrapping_add(3); 32],
        vec![
            PlannedPublication::try_new(
                0,
                format!("blk-{height}"),
                "hl.v1.block.committed",
                [salt.wrapping_add(4); 32],
            )
            .expect("block publication"),
            PlannedPublication::try_new(
                1,
                format!("evt-{height}"),
                "hl.v1.event.fill",
                [salt.wrapping_add(5); 32],
            )
            .expect("event publication"),
        ],
        known(1_721_779_300_000_000),
    )
    .expect("archived block plan")
}

fn ack(plan: &ArchivedBlockPlan, ordinal: u32, sequence: u64) -> PublicationAcknowledgement {
    let publication = &plan.publications()[usize::try_from(ordinal).expect("ordinal fits usize")];
    PublicationAcknowledgement::try_new(
        ordinal,
        publication.message_id(),
        publication.subject(),
        publication.publication_sha256(),
        "HL_CANONICAL",
        sequence,
        false,
        known(1_721_779_300_000_100 + i64::from(ordinal)),
    )
    .expect("publication acknowledgement")
}

#[tokio::test]
async fn archived_plans_and_acknowledgements_are_idempotent_but_hash_bound() {
    let store = InMemoryProgressStore::new(128).expect("progress store");
    let chain = ChainId::new("mainnet").expect("chain ID");
    store
        .initialize_chain(&chain, BlockHeight::new(42))
        .await
        .expect("initialize chain");
    let original = plan(42, 7);

    assert_eq!(
        store
            .record_archived(&original)
            .await
            .expect("first archived plan"),
        ProgressRecordDisposition::New
    );
    assert_eq!(
        store
            .record_archived(&original)
            .await
            .expect("identical archived plan"),
        ProgressRecordDisposition::IdenticalDuplicate
    );
    let error = store
        .record_archived(&plan(42, 8))
        .await
        .expect_err("same height with divergent archive binding");
    assert_eq!(error, ProgressError::ConflictingBlock);

    let first_ack = ack(&original, 0, 100);
    assert_eq!(
        store
            .record_acknowledgement(&chain, BlockHeight::new(42), &first_ack)
            .await
            .expect("first acknowledgement"),
        ProgressRecordDisposition::New
    );
    assert_eq!(
        store
            .record_acknowledgement(&chain, BlockHeight::new(42), &first_ack)
            .await
            .expect("identical acknowledgement"),
        ProgressRecordDisposition::IdenticalDuplicate
    );
    let conflicting_ack = PublicationAcknowledgement::try_new(
        0,
        first_ack.message_id(),
        first_ack.subject(),
        first_ack.publication_sha256(),
        first_ack.stream(),
        101,
        false,
        first_ack.acknowledged_at(),
    )
    .expect("conflicting acknowledgement");
    let error = store
        .record_acknowledgement(&chain, BlockHeight::new(42), &conflicting_ack)
        .await
        .expect_err("ack sequence changed");
    assert_eq!(error, ProgressError::ConflictingAcknowledgement);
}

#[tokio::test]
async fn cursor_advances_only_when_the_next_block_is_fully_acknowledged() {
    let store = InMemoryProgressStore::new(128).expect("progress store");
    let chain = ChainId::new("mainnet").expect("chain ID");
    store
        .initialize_chain(&chain, BlockHeight::new(42))
        .await
        .expect("initialize chain");
    let first = plan(42, 7);
    let second = plan(43, 8);
    store.record_archived(&first).await.expect("first plan");
    store.record_archived(&second).await.expect("second plan");

    for ordinal in 0..2 {
        store
            .record_acknowledgement(
                &chain,
                BlockHeight::new(43),
                &ack(&second, ordinal, 200 + u64::from(ordinal)),
            )
            .await
            .expect("second block ack");
    }
    let error = store
        .advance_cursor(&chain, BlockHeight::new(43))
        .await
        .expect_err("cursor may not skip the first configured height");
    assert_eq!(
        error,
        ProgressError::NonContiguousAdvance {
            expected: BlockHeight::new(42),
            actual: BlockHeight::new(43),
        }
    );

    store
        .record_acknowledgement(&chain, BlockHeight::new(42), &ack(&first, 0, 100))
        .await
        .expect("first block marker ack");
    let error = store
        .advance_cursor(&chain, BlockHeight::new(42))
        .await
        .expect_err("one publication remains");
    assert_eq!(error, ProgressError::PublicationIncomplete);

    store
        .record_acknowledgement(&chain, BlockHeight::new(42), &ack(&first, 1, 101))
        .await
        .expect("first event ack");
    let first_cursor = store
        .advance_cursor(&chain, BlockHeight::new(42))
        .await
        .expect("first cursor");
    assert_eq!(first_cursor.committed_block_height(), BlockHeight::new(42));
    assert_eq!(first_cursor.cursor_version(), 1);

    let second_cursor = store
        .advance_cursor(&chain, BlockHeight::new(43))
        .await
        .expect("second cursor");
    assert_eq!(second_cursor.committed_block_height(), BlockHeight::new(43));
    assert_eq!(second_cursor.cursor_version(), 2);
    assert_eq!(
        store.load_cursor(&chain).await.expect("load cursor"),
        Some(second_cursor)
    );
}

#[tokio::test]
async fn pending_recovery_scan_is_ordered_and_bounded() {
    let store = InMemoryProgressStore::new(3).expect("progress store");
    let chain = ChainId::new("mainnet").expect("chain ID");
    store
        .initialize_chain(&chain, BlockHeight::new(42))
        .await
        .expect("initialize chain");
    for height in [44, 42, 43] {
        store
            .record_archived(&plan(height, u8::try_from(height).expect("small height")))
            .await
            .expect("archived plan");
    }

    let pending = store
        .pending_blocks(&chain, 2)
        .await
        .expect("bounded pending scan");
    assert_eq!(
        pending
            .iter()
            .map(ArchivedBlockPlan::block_height)
            .collect::<Vec<_>>(),
        vec![BlockHeight::new(42), BlockHeight::new(43)]
    );
    let error = store
        .pending_blocks(&chain, 0)
        .await
        .expect_err("zero limit is invalid");
    assert_eq!(error, ProgressError::InvalidLimit);

    let error = store
        .record_archived(&plan(45, 45))
        .await
        .expect_err("capacity is fail closed");
    assert_eq!(error, ProgressError::CapacityExceeded { limit: 3 });
}
