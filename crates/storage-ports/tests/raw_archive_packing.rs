use domain_types::{ChainId, KnownTime, ManifestId, SourceId};
use storage_ports::{
    ArchiveError, LocalRecordSequence, LocalRecordSequenceRange, RawArchiveCapacityBudgets,
    RawArchiveCapacityRejection, RawArchiveCheckpointEntriesV2, RawArchiveCheckpointEntryV2,
    RawArchiveDurableFormatEnvelope, RawArchiveIndexCapacityEstimate,
    RawArchiveMaintenanceStatistics, RawArchivePackingPolicy,
    RawArchiveProductionCapacityAdmission, RawArchiveRootLeaseIdentity, RawArchiveWorkloadEnvelope,
    RawObservationReceipt, RawPackedRangeReceipt, SequenceBoundRawObservationReceipt,
};

#[test]
fn packing_policy_is_ordered_bounded_and_has_a_production_profile() {
    let production = RawArchivePackingPolicy::production();
    assert_eq!(production.minimum_pack_bytes(), 128 * 1024 * 1024);
    assert_eq!(production.target_pack_bytes(), 256 * 1024 * 1024);
    assert_eq!(production.maximum_pack_bytes(), 512 * 1024 * 1024);
    assert_eq!(production.maximum_input_objects(), 65_536);

    assert!(matches!(
        RawArchivePackingPolicy::try_new(0, 2, 3, 2),
        Err(ArchiveError::InvalidInput(_))
    ));
    assert!(matches!(
        RawArchivePackingPolicy::try_new(3, 2, 4, 2),
        Err(ArchiveError::InvalidInput(_))
    ));
    assert!(matches!(
        RawArchivePackingPolicy::try_new(1, 2, 512 * 1024 * 1024 + 1, 2),
        Err(ArchiveError::InvalidInput(_))
    ));
    assert!(matches!(
        RawArchivePackingPolicy::try_new(1, 2, 3, 1),
        Err(ArchiveError::InvalidInput(_))
    ));
}

#[test]
fn index_capacity_estimate_bounds_its_explicit_partial_inputs() {
    let commits_per_second = 10;
    let horizon_seconds = 86_400;
    let receipt_bytes_per_commit = 192;
    let index_pack_target_bytes = 4 * 1024 * 1024;
    let required_index_bytes = 165_888_000;
    let required_index_pack_files = 40;
    let required_inodes = required_index_pack_files + 16;

    let estimate = RawArchiveIndexCapacityEstimate::try_new(
        commits_per_second,
        horizon_seconds,
        receipt_bytes_per_commit,
        index_pack_target_bytes,
        required_index_bytes,
        required_inodes,
    )
    .expect("bounded partial index estimate");
    assert_eq!(estimate.maximum_logical_commits(), 864_000);
    assert_eq!(estimate.required_index_bytes(), required_index_bytes);
    assert_eq!(
        estimate.required_index_pack_files(),
        required_index_pack_files
    );
    assert_eq!(estimate.required_inodes(), required_inodes);

    assert!(matches!(
        RawArchiveIndexCapacityEstimate::try_new(
            commits_per_second,
            horizon_seconds,
            receipt_bytes_per_commit,
            index_pack_target_bytes,
            required_index_bytes - 1,
            required_inodes,
        ),
        Err(ArchiveError::InvalidInput(_))
    ));
    assert!(matches!(
        RawArchiveIndexCapacityEstimate::try_new(
            commits_per_second,
            horizon_seconds,
            receipt_bytes_per_commit,
            index_pack_target_bytes,
            required_index_bytes,
            required_inodes - 1,
        ),
        Err(ArchiveError::InvalidInput(_))
    ));
    assert!(matches!(
        RawArchiveIndexCapacityEstimate::try_new(u64::MAX, 2, 1, 1, u64::MAX, u64::MAX),
        Err(ArchiveError::InvalidInput(_))
    ));
}

fn production_capacity_inputs() -> (RawArchiveWorkloadEnvelope, RawArchiveDurableFormatEnvelope) {
    let workload = RawArchiveWorkloadEnvelope::try_new(
        100,
        100,
        1_000,
        3_600,
        1_024,
        100,
        64 * 1024 * 1024,
        64,
    )
    .unwrap();
    let format = RawArchiveDurableFormatEnvelope::production();
    (workload, format)
}

#[test]
fn production_capacity_admission_accounts_for_data_metadata_working_space_and_inodes() {
    let (workload, format) = production_capacity_inputs();
    let generous =
        RawArchiveCapacityBudgets::try_new(1_000_000_000, u64::MAX, u64::MAX, u64::MAX, true)
            .unwrap();
    let admission = RawArchiveProductionCapacityAdmission::evaluate(workload, format, generous)
        .expect("complete capacity envelope should admit");

    assert_eq!(admission.maximum_records(), 360_000);
    assert_eq!(admission.maximum_logical_commits(), 7_201);
    assert_eq!(admission.retained_raw_data_bytes(), 368_640_000);
    assert!(admission.required_raw_data_budget_bytes() > admission.retained_raw_data_bytes());
    assert!(admission.required_metadata_budget_bytes() > 0);
    assert!(admission.required_working_space_bytes() >= 512 * 1024 * 1024);
    assert!(admission.required_total_storage_bytes() > admission.required_raw_data_budget_bytes());
    assert!(admission.required_inodes() > 0);

    for (budgets, expected) in [
        (
            RawArchiveCapacityBudgets::try_new(
                admission.required_raw_data_budget_bytes() - 1,
                u64::MAX,
                u64::MAX,
                u64::MAX,
                true,
            )
            .unwrap(),
            RawArchiveCapacityRejection::RawDataBudget,
        ),
        (
            RawArchiveCapacityBudgets::try_new(
                u64::MAX,
                admission.required_metadata_budget_bytes() - 1,
                u64::MAX,
                u64::MAX,
                true,
            )
            .unwrap(),
            RawArchiveCapacityRejection::MetadataBudget,
        ),
        (
            RawArchiveCapacityBudgets::try_new(
                u64::MAX,
                u64::MAX,
                admission.required_total_storage_bytes() - 1,
                u64::MAX,
                true,
            )
            .unwrap(),
            RawArchiveCapacityRejection::TotalStorageBudget,
        ),
        (
            RawArchiveCapacityBudgets::try_new(
                u64::MAX,
                u64::MAX,
                u64::MAX,
                admission.required_inodes() - 1,
                true,
            )
            .unwrap(),
            RawArchiveCapacityRejection::InodeBudget,
        ),
    ] {
        assert_eq!(
            RawArchiveProductionCapacityAdmission::evaluate(workload, format, budgets),
            Err(expected)
        );
    }
}

#[test]
fn production_capacity_admission_fails_closed_without_purge_or_on_overflow() {
    let (workload, format) = production_capacity_inputs();
    let no_purge =
        RawArchiveCapacityBudgets::try_new(u64::MAX, u64::MAX, u64::MAX, u64::MAX, false).unwrap();
    assert_eq!(
        RawArchiveProductionCapacityAdmission::evaluate(workload, format, no_purge),
        Err(RawArchiveCapacityRejection::PurgeWorkflowMissing)
    );
    assert_eq!(
        RawArchiveWorkloadEnvelope::try_new(u64::MAX, 1, 1, 2, 1, 1, 1, 1),
        Err(RawArchiveCapacityRejection::ArithmeticOverflow)
    );
    assert_eq!(
        RawArchiveCapacityRejection::PurgeWorkflowMissing.reason_code(),
        "raw_archive.capacity.purge_workflow_missing"
    );

    let partition_bounded =
        RawArchiveWorkloadEnvelope::try_new(1, 100_000, 10_000_000, 7_200, 1, 1, 1, 1).unwrap();
    let generous =
        RawArchiveCapacityBudgets::try_new(u64::MAX, u64::MAX, u64::MAX, u64::MAX, true).unwrap();
    let admission = RawArchiveProductionCapacityAdmission::evaluate(
        partition_bounded,
        RawArchiveDurableFormatEnvelope::production(),
        generous,
    )
    .unwrap();
    assert_eq!(admission.maximum_logical_commits(), 4);
}

fn sequence_range(start: u64, end: u64) -> LocalRecordSequenceRange {
    LocalRecordSequenceRange::try_new(
        LocalRecordSequence::try_new(start).unwrap(),
        LocalRecordSequence::try_new(end).unwrap(),
    )
    .unwrap()
}

#[test]
fn sequence_bound_receipt_rejects_legacy_receipts_without_sequence_evidence() {
    let manifest_id = ManifestId::new("manifest-1").unwrap();
    let chain = ChainId::new("mainnet").unwrap();
    let source = SourceId::new("node-fills").unwrap();
    let legacy = RawObservationReceipt::try_new(
        "receipt-1",
        manifest_id.clone(),
        chain.clone(),
        source.clone(),
        "epoch-1",
        0,
        99,
        [1; 32],
        [2; 32],
        [3; 32],
        [4; 32],
        [5; 32],
        [6; 32],
        KnownTime::from_unix_micros(1_000).unwrap(),
    )
    .unwrap();
    assert!(SequenceBoundRawObservationReceipt::try_from(legacy).is_err());

    let range = sequence_range(10, 12);
    let byte = RawObservationReceipt::try_new_byte_offsets(
        "receipt-2",
        manifest_id,
        chain,
        source,
        "epoch-1",
        0,
        99,
        range,
        [1; 32],
        [2; 32],
        [3; 32],
        [4; 32],
        [5; 32],
        [6; 32],
        KnownTime::from_unix_micros(1_000).unwrap(),
    )
    .unwrap();
    let bound = SequenceBoundRawObservationReceipt::try_from(byte).unwrap();
    assert_eq!(bound.local_sequence_range(), range);
    assert_eq!(bound.receipt().manifest_id().as_str(), "manifest-1");
}

#[test]
fn packed_range_receipt_and_maintenance_statistics_are_internally_consistent() {
    let range = sequence_range(1, 100);
    let receipt = RawPackedRangeReceipt::try_new(
        ChainId::new("mainnet").unwrap(),
        SourceId::new("node-fills").unwrap(),
        range,
        4,
        [1; 32],
        1024,
        [2; 32],
        [3; 32],
        KnownTime::from_unix_micros(2_000).unwrap(),
    )
    .unwrap();
    assert_eq!(receipt.input_logical_manifest_count(), 4);
    assert_eq!(receipt.local_sequence_range(), range);
    assert!(
        RawPackedRangeReceipt::try_new(
            ChainId::new("mainnet").unwrap(),
            SourceId::new("node-fills").unwrap(),
            range,
            1,
            [1; 32],
            1024,
            [2; 32],
            [3; 32],
            KnownTime::from_unix_micros(2_000).unwrap(),
        )
        .is_err()
    );

    let statistics =
        RawArchiveMaintenanceStatistics::try_new(10, 100, 7, 3, 10_000, 7_000, 2_000, 4, 3)
            .unwrap();
    assert_eq!(statistics.logical_manifest_count(), 10);
    assert_eq!(statistics.physical_data_object_count(), 7);
    assert!(
        RawArchiveMaintenanceStatistics::try_new(10, 100, 11, 3, 10_000, 7_000, 2_000, 4, 3)
            .is_err()
    );
}

#[test]
fn checkpoint_v2_entries_are_sequence_exact_and_root_leases_are_content_addressed() {
    let first = RawArchiveCheckpointEntryV2::new(
        ManifestId::new("manifest-1").unwrap(),
        [1; 32],
        sequence_range(1, 2),
    );
    let second = RawArchiveCheckpointEntryV2::new(
        ManifestId::new("manifest-2").unwrap(),
        [2; 32],
        sequence_range(3, 4),
    );
    let entries = RawArchiveCheckpointEntriesV2::try_new(vec![first.clone(), second]).unwrap();
    assert_eq!(entries.first_local_sequence().get(), 1);
    assert_eq!(entries.last_local_sequence().get(), 4);
    assert!(RawArchiveCheckpointEntriesV2::try_new(vec![first.clone(), first]).is_err());

    let lease = RawArchiveRootLeaseIdentity::new([0xAB; 32]);
    assert_eq!(
        lease.relative_path().to_string_lossy(),
        format!("leases/root-{}.lease", "ab".repeat(32))
    );
}
