use std::{
    collections::{BTreeMap, BTreeSet},
    num::NonZeroU64,
    path::{Path, PathBuf},
};

use canonical_events::BlockEnvelope;
use domain_types::{BlockHeight, BlockRange, ChainId, KnownTime, ManifestId, SourceId};
use hl_protocol::SourceObservation;

pub const ARCHIVE_MANIFEST_SCHEMA_V1: &str = "hyperliquid-alpha-desk/archive-manifest/v1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveReceipt {
    receipt_id: String,
    manifest_id: ManifestId,
    block_height: BlockHeight,
    canonical_block_hash: [u8; 32],
    object_sha256: [u8; 32],
    manifest_sha256: [u8; 32],
    schema_fingerprint: [u8; 32],
    durable_at: KnownTime,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactionReceipt {
    manifest_id: ManifestId,
    block_range: BlockRange,
    input_object_count: u64,
    output_object_sha256: [u8; 32],
    row_count: u64,
    rolling_content_sha256: [u8; 32],
    completed_at: KnownTime,
}

impl CompactionReceipt {
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        manifest_id: ManifestId,
        block_range: BlockRange,
        input_object_count: u64,
        output_object_sha256: [u8; 32],
        row_count: u64,
        rolling_content_sha256: [u8; 32],
        completed_at: KnownTime,
    ) -> Result<Self, ArchiveError> {
        if input_object_count < 2 {
            return Err(ArchiveError::InvalidInput(
                "compaction requires at least two input objects",
            ));
        }
        Ok(Self {
            manifest_id,
            block_range,
            input_object_count,
            output_object_sha256,
            row_count,
            rolling_content_sha256,
            completed_at,
        })
    }

    #[must_use]
    pub const fn manifest_id(&self) -> &ManifestId {
        &self.manifest_id
    }

    #[must_use]
    pub const fn block_range(&self) -> BlockRange {
        self.block_range
    }

    #[must_use]
    pub const fn input_object_count(&self) -> u64 {
        self.input_object_count
    }

    #[must_use]
    pub const fn output_object_sha256(&self) -> [u8; 32] {
        self.output_object_sha256
    }

    #[must_use]
    pub const fn row_count(&self) -> u64 {
        self.row_count
    }

    #[must_use]
    pub const fn rolling_content_sha256(&self) -> [u8; 32] {
        self.rolling_content_sha256
    }

    #[must_use]
    pub const fn completed_at(&self) -> KnownTime {
        self.completed_at
    }
}

pub const RAW_ARCHIVE_MAXIMUM_DATA_PACK_BYTES: u64 = 512 * 1024 * 1024;
pub const RAW_ARCHIVE_MAXIMUM_PACK_LOGICAL_INPUTS: usize = 65_536;
pub const RAW_ARCHIVE_MAXIMUM_SEQUENCE_PAGE_BYTES: u64 = 4 * 1024 * 1024;
pub const RAW_ARCHIVE_MAXIMUM_SEQUENCE_TREE_DEPTH: u8 = 8;
const PRODUCTION_MINIMUM_RAW_PACK_BYTES: u64 = 128 * 1024 * 1024;
const PRODUCTION_TARGET_RAW_PACK_BYTES: u64 = 256 * 1024 * 1024;
const PRODUCTION_MAXIMUM_RAW_PACK_BYTES: u64 = RAW_ARCHIVE_MAXIMUM_DATA_PACK_BYTES;
const PRODUCTION_MAXIMUM_RAW_PACK_INPUT_OBJECTS: u64 =
    RAW_ARCHIVE_MAXIMUM_PACK_LOGICAL_INPUTS as u64;
const ACTIVE_RAW_INDEX_INODE_RESERVE: u64 = 16;
pub const RAW_ARCHIVE_MAXIMUM_LOGICAL_MANIFEST_BYTES: u64 = 64 * 1024;
pub const RAW_ARCHIVE_MAXIMUM_EMBEDDED_PACK_MANIFEST_BYTES: u64 = 64 * 1024 * 1024;
pub const RAW_ARCHIVE_MAXIMUM_INDEX_PACK_BYTES: u64 = 64 * 1024 * 1024;
pub const RAW_ARCHIVE_MAXIMUM_RELATIVE_PATH_BYTES: usize = 4 * 1024;
const PRODUCTION_RECEIPT_HINT_BYTES_PER_COMMIT: u64 = 512;
const PRODUCTION_CHECKPOINT_BYTES_PER_COMMIT: u64 = 512;
const PRODUCTION_JOURNAL_BYTES_PER_COMMIT: u64 =
    (RAW_ARCHIVE_MAXIMUM_SEQUENCE_TREE_DEPTH as u64 + 1) * RAW_ARCHIVE_MAXIMUM_SEQUENCE_PAGE_BYTES;
const PRODUCTION_INDEX_PACK_TARGET_BYTES: u64 = 4 * 1024 * 1024;
const PRODUCTION_FIXED_METADATA_RESERVE_BYTES: u64 = 3 * 64 * 1024 * 1024;
const PRODUCTION_FIXED_ACTIVE_INODE_RESERVE: u64 = 64;
const PRODUCTION_DELETION_JOURNAL_BYTES_PER_ELIGIBLE_INODE: u64 =
    2 * RAW_ARCHIVE_MAXIMUM_RELATIVE_PATH_BYTES as u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawArchivePackingPolicy {
    minimum_pack_bytes: u64,
    target_pack_bytes: u64,
    maximum_pack_bytes: u64,
    maximum_input_objects: u64,
}

impl RawArchivePackingPolicy {
    #[must_use]
    pub const fn production() -> Self {
        Self {
            minimum_pack_bytes: PRODUCTION_MINIMUM_RAW_PACK_BYTES,
            target_pack_bytes: PRODUCTION_TARGET_RAW_PACK_BYTES,
            maximum_pack_bytes: PRODUCTION_MAXIMUM_RAW_PACK_BYTES,
            maximum_input_objects: PRODUCTION_MAXIMUM_RAW_PACK_INPUT_OBJECTS,
        }
    }

    pub fn try_new(
        minimum_pack_bytes: u64,
        target_pack_bytes: u64,
        maximum_pack_bytes: u64,
        maximum_input_objects: u64,
    ) -> Result<Self, ArchiveError> {
        if minimum_pack_bytes == 0
            || minimum_pack_bytes > target_pack_bytes
            || target_pack_bytes > maximum_pack_bytes
            || maximum_pack_bytes > RAW_ARCHIVE_MAXIMUM_DATA_PACK_BYTES
            || maximum_input_objects < 2
        {
            return Err(ArchiveError::InvalidInput("raw archive packing policy"));
        }
        Ok(Self {
            minimum_pack_bytes,
            target_pack_bytes,
            maximum_pack_bytes,
            maximum_input_objects,
        })
    }

    #[must_use]
    pub const fn minimum_pack_bytes(self) -> u64 {
        self.minimum_pack_bytes
    }

    #[must_use]
    pub const fn target_pack_bytes(self) -> u64 {
        self.target_pack_bytes
    }

    #[must_use]
    pub const fn maximum_pack_bytes(self) -> u64 {
        self.maximum_pack_bytes
    }

    #[must_use]
    pub const fn maximum_input_objects(self) -> u64 {
        self.maximum_input_objects
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RawArchiveCapacityRejection {
    #[error("raw archive capacity policy is invalid")]
    InvalidPolicy,
    #[error("raw archive capacity arithmetic overflowed")]
    ArithmeticOverflow,
    #[error(
        "raw archive production retention requires a configured digest-confirmed purge workflow"
    )]
    PurgeWorkflowMissing,
    #[error("raw archive data budget is insufficient")]
    RawDataBudget,
    #[error("raw archive metadata budget is insufficient")]
    MetadataBudget,
    #[error("raw archive total storage budget is insufficient")]
    TotalStorageBudget,
    #[error("raw archive inode budget is insufficient")]
    InodeBudget,
    #[error("raw archive runtime exceeded an admitted workload limit")]
    RuntimeLimitExceeded,
}

impl RawArchiveCapacityRejection {
    #[must_use]
    pub const fn reason_code(self) -> &'static str {
        match self {
            Self::InvalidPolicy => "raw_archive.capacity.invalid_policy",
            Self::ArithmeticOverflow => "raw_archive.capacity.arithmetic_overflow",
            Self::PurgeWorkflowMissing => "raw_archive.capacity.purge_workflow_missing",
            Self::RawDataBudget => "raw_archive.capacity.raw_data_budget",
            Self::MetadataBudget => "raw_archive.capacity.metadata_budget",
            Self::TotalStorageBudget => "raw_archive.capacity.total_storage_budget",
            Self::InodeBudget => "raw_archive.capacity.inode_budget",
            Self::RuntimeLimitExceeded => "raw_archive.capacity.runtime_limit_exceeded",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawArchiveWorkloadEnvelope {
    maximum_records_per_second: u64,
    minimum_group_records: u64,
    maximum_group_delay_millis: u64,
    retention_horizon_seconds: u64,
    maximum_encoded_record_bytes: u64,
    maximum_uncompacted_commits: u64,
    maximum_eligible_bytes: u64,
    maximum_eligible_inodes: u64,
}

impl RawArchiveWorkloadEnvelope {
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        maximum_records_per_second: u64,
        minimum_group_records: u64,
        maximum_group_delay_millis: u64,
        retention_horizon_seconds: u64,
        maximum_encoded_record_bytes: u64,
        maximum_uncompacted_commits: u64,
        maximum_eligible_bytes: u64,
        maximum_eligible_inodes: u64,
    ) -> Result<Self, RawArchiveCapacityRejection> {
        if maximum_records_per_second == 0
            || minimum_group_records == 0
            || maximum_group_delay_millis == 0
            || retention_horizon_seconds == 0
            || maximum_encoded_record_bytes == 0
            || maximum_uncompacted_commits == 0
            || maximum_eligible_bytes == 0
            || maximum_eligible_inodes == 0
        {
            return Err(RawArchiveCapacityRejection::InvalidPolicy);
        }
        maximum_records_per_second
            .checked_mul(retention_horizon_seconds)
            .ok_or(RawArchiveCapacityRejection::ArithmeticOverflow)?;
        retention_horizon_seconds
            .checked_mul(1_000)
            .ok_or(RawArchiveCapacityRejection::ArithmeticOverflow)?;
        Ok(Self {
            maximum_records_per_second,
            minimum_group_records,
            maximum_group_delay_millis,
            retention_horizon_seconds,
            maximum_encoded_record_bytes,
            maximum_uncompacted_commits,
            maximum_eligible_bytes,
            maximum_eligible_inodes,
        })
    }

    /// Runtime admission must call this before accepting a source record.
    pub fn validate_record_bytes(
        self,
        encoded_record_bytes: u64,
    ) -> Result<(), RawArchiveCapacityRejection> {
        if encoded_record_bytes == 0 || encoded_record_bytes > self.maximum_encoded_record_bytes {
            return Err(RawArchiveCapacityRejection::RuntimeLimitExceeded);
        }
        Ok(())
    }

    /// Maintenance must call this before publishing capacity health as green.
    pub fn validate_backlog(
        self,
        uncompacted_commits: u64,
        eligible_bytes: u64,
        eligible_inodes: u64,
    ) -> Result<(), RawArchiveCapacityRejection> {
        if uncompacted_commits > self.maximum_uncompacted_commits
            || eligible_bytes > self.maximum_eligible_bytes
            || eligible_inodes > self.maximum_eligible_inodes
        {
            return Err(RawArchiveCapacityRejection::RuntimeLimitExceeded);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawArchiveDurableFormatEnvelope {
    maximum_logical_manifest_bytes: u64,
    receipt_hint_bytes_per_commit: u64,
    checkpoint_bytes_per_commit: u64,
    journal_bytes_per_commit: u64,
    index_pack_target_bytes: u64,
    minimum_data_pack_bytes: u64,
    maximum_data_pack_bytes: u64,
    fixed_metadata_reserve_bytes: u64,
    fixed_active_inode_reserve: u64,
}

impl RawArchiveDurableFormatEnvelope {
    /// Frozen V3 production limits shared by admission and durable builders.
    /// Callers cannot reduce these costs to obtain a false capacity result.
    #[must_use]
    pub const fn production() -> Self {
        Self {
            maximum_logical_manifest_bytes: RAW_ARCHIVE_MAXIMUM_LOGICAL_MANIFEST_BYTES,
            receipt_hint_bytes_per_commit: PRODUCTION_RECEIPT_HINT_BYTES_PER_COMMIT,
            checkpoint_bytes_per_commit: PRODUCTION_CHECKPOINT_BYTES_PER_COMMIT,
            journal_bytes_per_commit: PRODUCTION_JOURNAL_BYTES_PER_COMMIT,
            index_pack_target_bytes: PRODUCTION_INDEX_PACK_TARGET_BYTES,
            minimum_data_pack_bytes: PRODUCTION_MINIMUM_RAW_PACK_BYTES,
            maximum_data_pack_bytes: PRODUCTION_MAXIMUM_RAW_PACK_BYTES,
            fixed_metadata_reserve_bytes: PRODUCTION_FIXED_METADATA_RESERVE_BYTES,
            fixed_active_inode_reserve: PRODUCTION_FIXED_ACTIVE_INODE_RESERVE,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Operator budgets used for startup admission.
///
/// Configuring the digest-confirmed purge workflow only proves that bounded
/// retention has an executable fail-closed path. It never authorizes a purge;
/// each deletion still requires its separately confirmed plan digest.
pub struct RawArchiveCapacityBudgets {
    raw_data_bytes: u64,
    metadata_bytes: u64,
    total_storage_bytes: u64,
    inodes: u64,
    digest_confirmed_purge_workflow_configured: bool,
}

impl RawArchiveCapacityBudgets {
    pub fn try_new(
        raw_data_bytes: u64,
        metadata_bytes: u64,
        total_storage_bytes: u64,
        inodes: u64,
        digest_confirmed_purge_workflow_configured: bool,
    ) -> Result<Self, RawArchiveCapacityRejection> {
        if raw_data_bytes == 0 || metadata_bytes == 0 || total_storage_bytes == 0 || inodes == 0 {
            return Err(RawArchiveCapacityRejection::InvalidPolicy);
        }
        Ok(Self {
            raw_data_bytes,
            metadata_bytes,
            total_storage_bytes,
            inodes,
            digest_confirmed_purge_workflow_configured,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawArchiveProductionCapacityAdmission {
    maximum_records: u64,
    maximum_logical_commits: u64,
    retained_raw_data_bytes: u64,
    required_raw_data_budget_bytes: u64,
    required_metadata_budget_bytes: u64,
    required_working_space_bytes: u64,
    required_total_storage_bytes: u64,
    required_inodes: u64,
}

impl RawArchiveProductionCapacityAdmission {
    pub fn evaluate(
        workload: RawArchiveWorkloadEnvelope,
        format: RawArchiveDurableFormatEnvelope,
        budgets: RawArchiveCapacityBudgets,
    ) -> Result<Self, RawArchiveCapacityRejection> {
        if !budgets.digest_confirmed_purge_workflow_configured {
            return Err(RawArchiveCapacityRejection::PurgeWorkflowMissing);
        }
        let maximum_records = checked_product(
            workload.maximum_records_per_second,
            workload.retention_horizon_seconds,
        )?;
        let record_triggered_commits =
            ceiling_div(maximum_records, workload.minimum_group_records)?;
        let horizon_millis = checked_product(workload.retention_horizon_seconds, 1_000)?;
        let delay_triggered_commits =
            ceiling_div(horizon_millis, workload.maximum_group_delay_millis)?;
        let time_partitions = ceiling_div(workload.retention_horizon_seconds, 3_600)?;
        let maximum_logical_commits = maximum_records.min(checked_sum_capacity([
            record_triggered_commits,
            delay_triggered_commits,
            time_partitions,
        ])?);
        let retained_raw_data_bytes =
            checked_product(maximum_records, workload.maximum_encoded_record_bytes)?;
        let required_raw_data_budget_bytes =
            checked_sum_capacity([retained_raw_data_bytes, workload.maximum_eligible_bytes])?;
        let metadata_bytes_per_commit = checked_sum_capacity([
            format.maximum_logical_manifest_bytes,
            format.receipt_hint_bytes_per_commit,
            format.checkpoint_bytes_per_commit,
            format.journal_bytes_per_commit,
        ])?;
        let required_metadata_budget_bytes = checked_sum_capacity([
            checked_product(maximum_logical_commits, metadata_bytes_per_commit)?,
            checked_product(
                workload.maximum_eligible_inodes,
                PRODUCTION_DELETION_JOURNAL_BYTES_PER_ELIGIBLE_INODE,
            )?,
            format.fixed_metadata_reserve_bytes,
        ])?;
        let required_working_space_bytes = checked_sum_capacity([
            format.maximum_data_pack_bytes,
            RAW_ARCHIVE_MAXIMUM_EMBEDDED_PACK_MANIFEST_BYTES,
            RAW_ARCHIVE_MAXIMUM_INDEX_PACK_BYTES,
            format.fixed_metadata_reserve_bytes,
        ])?;
        let required_total_storage_bytes = checked_sum_capacity([
            required_raw_data_budget_bytes,
            required_metadata_budget_bytes,
            required_working_space_bytes,
        ])?;

        let full_data_packs = ceiling_div(retained_raw_data_bytes, format.minimum_data_pack_bytes)?;
        let embedded_manifest_inputs_per_pack = RAW_ARCHIVE_MAXIMUM_EMBEDDED_PACK_MANIFEST_BYTES
            / RAW_ARCHIVE_MAXIMUM_LOGICAL_MANIFEST_BYTES;
        let format_inputs_per_pack = embedded_manifest_inputs_per_pack.min(
            u64::try_from(RAW_ARCHIVE_MAXIMUM_PACK_LOGICAL_INPUTS)
                .map_err(|_| RawArchiveCapacityRejection::ArithmeticOverflow)?,
        );
        let input_forced_packs = ceiling_div(maximum_logical_commits, format_inputs_per_pack)?;
        let data_pack_files = maximum_logical_commits.min(
            full_data_packs
                .max(input_forced_packs)
                .checked_add(time_partitions)
                .ok_or(RawArchiveCapacityRejection::ArithmeticOverflow)?,
        );
        let index_bytes_per_commit = checked_sum_capacity([
            format.receipt_hint_bytes_per_commit,
            format.journal_bytes_per_commit,
        ])?;
        let index_pack_files = ceiling_div(
            checked_product(maximum_logical_commits, index_bytes_per_commit)?,
            format.index_pack_target_bytes,
        )?;
        let required_inodes = checked_sum_capacity([
            checked_product(data_pack_files, 2)?,
            checked_product(index_pack_files, 2)?,
            checked_product(workload.maximum_uncompacted_commits, 2)?,
            workload.maximum_eligible_inodes,
            format.fixed_active_inode_reserve,
        ])?;

        if required_raw_data_budget_bytes > budgets.raw_data_bytes {
            return Err(RawArchiveCapacityRejection::RawDataBudget);
        }
        if required_metadata_budget_bytes > budgets.metadata_bytes {
            return Err(RawArchiveCapacityRejection::MetadataBudget);
        }
        if required_total_storage_bytes > budgets.total_storage_bytes {
            return Err(RawArchiveCapacityRejection::TotalStorageBudget);
        }
        if required_inodes > budgets.inodes {
            return Err(RawArchiveCapacityRejection::InodeBudget);
        }
        Ok(Self {
            maximum_records,
            maximum_logical_commits,
            retained_raw_data_bytes,
            required_raw_data_budget_bytes,
            required_metadata_budget_bytes,
            required_working_space_bytes,
            required_total_storage_bytes,
            required_inodes,
        })
    }

    #[must_use]
    pub const fn maximum_records(self) -> u64 {
        self.maximum_records
    }

    #[must_use]
    pub const fn maximum_logical_commits(self) -> u64 {
        self.maximum_logical_commits
    }

    #[must_use]
    pub const fn retained_raw_data_bytes(self) -> u64 {
        self.retained_raw_data_bytes
    }

    #[must_use]
    pub const fn required_raw_data_budget_bytes(self) -> u64 {
        self.required_raw_data_budget_bytes
    }

    #[must_use]
    pub const fn required_metadata_budget_bytes(self) -> u64 {
        self.required_metadata_budget_bytes
    }

    #[must_use]
    pub const fn required_working_space_bytes(self) -> u64 {
        self.required_working_space_bytes
    }

    #[must_use]
    pub const fn required_total_storage_bytes(self) -> u64 {
        self.required_total_storage_bytes
    }

    #[must_use]
    pub const fn required_inodes(self) -> u64 {
        self.required_inodes
    }
}

fn checked_product(left: u64, right: u64) -> Result<u64, RawArchiveCapacityRejection> {
    left.checked_mul(right)
        .ok_or(RawArchiveCapacityRejection::ArithmeticOverflow)
}

fn checked_sum_capacity(
    values: impl IntoIterator<Item = u64>,
) -> Result<u64, RawArchiveCapacityRejection> {
    values.into_iter().try_fold(0_u64, |total, value| {
        total
            .checked_add(value)
            .ok_or(RawArchiveCapacityRejection::ArithmeticOverflow)
    })
}

fn ceiling_div(value: u64, divisor: u64) -> Result<u64, RawArchiveCapacityRejection> {
    if divisor == 0 {
        return Err(RawArchiveCapacityRejection::InvalidPolicy);
    }
    let quotient = value / divisor;
    quotient
        .checked_add(u64::from(!value.is_multiple_of(divisor)))
        .ok_or(RawArchiveCapacityRejection::ArithmeticOverflow)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Partial estimate for receipt-index packing only.
///
/// This deliberately excludes data objects, manifests, journals, roots,
/// checkpoints, retention, GC, and temporary double-space. It must not be
/// used as production capacity admission; the complete V3 envelope owns that
/// decision once all durable format bounds are frozen.
pub struct RawArchiveIndexCapacityEstimate {
    maximum_logical_commits: u64,
    required_index_bytes: u64,
    required_index_pack_files: u64,
    required_inodes: u64,
    index_budget_bytes: u64,
    inode_budget: u64,
}

impl RawArchiveIndexCapacityEstimate {
    pub fn try_new(
        maximum_commits_per_second: u64,
        retention_horizon_seconds: u64,
        receipt_bytes_per_commit: u64,
        index_pack_target_bytes: u64,
        index_budget_bytes: u64,
        inode_budget: u64,
    ) -> Result<Self, ArchiveError> {
        if maximum_commits_per_second == 0
            || retention_horizon_seconds == 0
            || receipt_bytes_per_commit == 0
            || index_pack_target_bytes == 0
            || index_budget_bytes == 0
            || inode_budget == 0
        {
            return Err(ArchiveError::InvalidInput(
                "raw archive index capacity estimate",
            ));
        }
        let maximum_logical_commits = maximum_commits_per_second
            .checked_mul(retention_horizon_seconds)
            .ok_or(ArchiveError::InvalidInput(
                "raw archive logical commit capacity overflows",
            ))?;
        let required_index_bytes = maximum_logical_commits
            .checked_mul(receipt_bytes_per_commit)
            .ok_or(ArchiveError::InvalidInput(
                "raw archive receipt index capacity overflows",
            ))?;
        let complete_packs = required_index_bytes / index_pack_target_bytes;
        let required_index_pack_files = complete_packs
            .checked_add(u64::from(
                required_index_bytes % index_pack_target_bytes != 0,
            ))
            .ok_or(ArchiveError::InvalidInput(
                "raw archive index pack count overflows",
            ))?;
        let required_inodes = required_index_pack_files
            .checked_add(ACTIVE_RAW_INDEX_INODE_RESERVE)
            .ok_or(ArchiveError::InvalidInput(
                "raw archive inode capacity overflows",
            ))?;
        if required_index_bytes > index_budget_bytes || required_inodes > inode_budget {
            return Err(ArchiveError::InvalidInput(
                "raw archive capacity budget is insufficient",
            ));
        }
        Ok(Self {
            maximum_logical_commits,
            required_index_bytes,
            required_index_pack_files,
            required_inodes,
            index_budget_bytes,
            inode_budget,
        })
    }

    #[must_use]
    pub const fn maximum_logical_commits(self) -> u64 {
        self.maximum_logical_commits
    }

    #[must_use]
    pub const fn required_index_bytes(self) -> u64 {
        self.required_index_bytes
    }

    #[must_use]
    pub const fn required_index_pack_files(self) -> u64 {
        self.required_index_pack_files
    }

    #[must_use]
    pub const fn required_inodes(self) -> u64 {
        self.required_inodes
    }

    #[must_use]
    pub const fn index_budget_bytes(self) -> u64 {
        self.index_budget_bytes
    }

    #[must_use]
    pub const fn inode_budget(self) -> u64 {
        self.inode_budget
    }
}

impl ArchiveReceipt {
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        receipt_id: impl Into<String>,
        manifest_id: ManifestId,
        block_height: BlockHeight,
        canonical_block_hash: [u8; 32],
        object_sha256: [u8; 32],
        manifest_sha256: [u8; 32],
        schema_fingerprint: [u8; 32],
        durable_at: KnownTime,
    ) -> Result<Self, ArchiveError> {
        let receipt_id = receipt_id.into();
        validate_identity(&receipt_id, "receipt ID")?;
        Ok(Self {
            receipt_id,
            manifest_id,
            block_height,
            canonical_block_hash,
            object_sha256,
            manifest_sha256,
            schema_fingerprint,
            durable_at,
        })
    }

    #[must_use]
    pub fn receipt_id(&self) -> &str {
        &self.receipt_id
    }

    #[must_use]
    pub const fn manifest_id(&self) -> &ManifestId {
        &self.manifest_id
    }

    #[must_use]
    pub const fn block_height(&self) -> BlockHeight {
        self.block_height
    }

    #[must_use]
    pub const fn canonical_block_hash(&self) -> [u8; 32] {
        self.canonical_block_hash
    }

    #[must_use]
    pub const fn object_sha256(&self) -> [u8; 32] {
        self.object_sha256
    }

    #[must_use]
    pub const fn manifest_sha256(&self) -> [u8; 32] {
        self.manifest_sha256
    }

    #[must_use]
    pub const fn schema_fingerprint(&self) -> [u8; 32] {
        self.schema_fingerprint
    }

    #[must_use]
    pub const fn durable_at(&self) -> KnownTime {
        self.durable_at
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawObservationReceipt {
    receipt_id: String,
    manifest_id: ManifestId,
    chain_id: ChainId,
    source_id: SourceId,
    cursor_epoch: String,
    start_offset: u64,
    end_offset: u64,
    cursor_policy: CursorPolicy,
    local_sequence_range: Option<LocalRecordSequenceRange>,
    spool_manifest_blake3: [u8; 32],
    spool_segment_blake3: [u8; 32],
    rolling_content_sha256: [u8; 32],
    object_sha256: [u8; 32],
    manifest_sha256: [u8; 32],
    schema_fingerprint: [u8; 32],
    durable_at: KnownTime,
}

impl RawObservationReceipt {
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        receipt_id: impl Into<String>,
        manifest_id: ManifestId,
        chain_id: ChainId,
        source_id: SourceId,
        cursor_epoch: impl Into<String>,
        start_offset: u64,
        end_offset: u64,
        spool_manifest_blake3: [u8; 32],
        spool_segment_blake3: [u8; 32],
        rolling_content_sha256: [u8; 32],
        object_sha256: [u8; 32],
        manifest_sha256: [u8; 32],
        schema_fingerprint: [u8; 32],
        durable_at: KnownTime,
    ) -> Result<Self, ArchiveError> {
        let receipt_id = receipt_id.into();
        validate_identity(&receipt_id, "receipt ID")?;
        let cursor_epoch = cursor_epoch.into();
        validate_identity(&cursor_epoch, "cursor epoch")?;
        if start_offset > end_offset {
            return Err(ArchiveError::InvalidInput(
                "raw observation receipt cursor range",
            ));
        }
        Ok(Self {
            receipt_id,
            manifest_id,
            chain_id,
            source_id,
            cursor_epoch,
            start_offset,
            end_offset,
            cursor_policy: CursorPolicy::ContiguousNativeOffset,
            local_sequence_range: None,
            spool_manifest_blake3,
            spool_segment_blake3,
            rolling_content_sha256,
            object_sha256,
            manifest_sha256,
            schema_fingerprint,
            durable_at,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn try_new_byte_offsets(
        receipt_id: impl Into<String>,
        manifest_id: ManifestId,
        chain_id: ChainId,
        source_id: SourceId,
        cursor_epoch: impl Into<String>,
        start_offset: u64,
        end_offset: u64,
        local_sequence_range: LocalRecordSequenceRange,
        spool_manifest_blake3: [u8; 32],
        spool_segment_blake3: [u8; 32],
        rolling_content_sha256: [u8; 32],
        object_sha256: [u8; 32],
        manifest_sha256: [u8; 32],
        schema_fingerprint: [u8; 32],
        durable_at: KnownTime,
    ) -> Result<Self, ArchiveError> {
        let mut receipt = Self::try_new(
            receipt_id,
            manifest_id,
            chain_id,
            source_id,
            cursor_epoch,
            start_offset,
            end_offset,
            spool_manifest_blake3,
            spool_segment_blake3,
            rolling_content_sha256,
            object_sha256,
            manifest_sha256,
            schema_fingerprint,
            durable_at,
        )?;
        receipt.cursor_policy = CursorPolicy::MonotonicByteOffset;
        receipt.local_sequence_range = Some(local_sequence_range);
        Ok(receipt)
    }

    #[must_use]
    pub fn receipt_id(&self) -> &str {
        &self.receipt_id
    }

    #[must_use]
    pub const fn manifest_id(&self) -> &ManifestId {
        &self.manifest_id
    }

    #[must_use]
    pub const fn source_id(&self) -> &SourceId {
        &self.source_id
    }

    #[must_use]
    pub const fn chain_id(&self) -> &ChainId {
        &self.chain_id
    }

    #[must_use]
    pub fn cursor_epoch(&self) -> &str {
        &self.cursor_epoch
    }

    #[must_use]
    pub const fn start_offset(&self) -> u64 {
        self.start_offset
    }

    #[must_use]
    pub const fn end_offset(&self) -> u64 {
        self.end_offset
    }

    #[must_use]
    pub const fn cursor_policy(&self) -> CursorPolicy {
        self.cursor_policy
    }

    #[must_use]
    pub const fn local_sequence_range(&self) -> Option<LocalRecordSequenceRange> {
        self.local_sequence_range
    }

    #[must_use]
    pub const fn spool_manifest_blake3(&self) -> [u8; 32] {
        self.spool_manifest_blake3
    }

    #[must_use]
    pub const fn spool_segment_blake3(&self) -> [u8; 32] {
        self.spool_segment_blake3
    }

    #[must_use]
    pub const fn rolling_content_sha256(&self) -> [u8; 32] {
        self.rolling_content_sha256
    }

    #[must_use]
    pub const fn object_sha256(&self) -> [u8; 32] {
        self.object_sha256
    }

    #[must_use]
    pub const fn manifest_sha256(&self) -> [u8; 32] {
        self.manifest_sha256
    }

    #[must_use]
    pub const fn schema_fingerprint(&self) -> [u8; 32] {
        self.schema_fingerprint
    }

    #[must_use]
    pub const fn durable_at(&self) -> KnownTime {
        self.durable_at
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SequenceBoundRawObservationReceipt {
    receipt: RawObservationReceipt,
    local_sequence_range: LocalRecordSequenceRange,
}

impl TryFrom<RawObservationReceipt> for SequenceBoundRawObservationReceipt {
    type Error = ArchiveError;

    fn try_from(receipt: RawObservationReceipt) -> Result<Self, Self::Error> {
        if receipt.cursor_policy() != CursorPolicy::MonotonicByteOffset {
            return Err(ArchiveError::InvalidInput(
                "sequence-bound receipt requires monotonic byte offsets",
            ));
        }
        let local_sequence_range =
            receipt
                .local_sequence_range()
                .ok_or(ArchiveError::InvalidInput(
                    "sequence-bound receipt is missing sequence evidence",
                ))?;
        Ok(Self {
            receipt,
            local_sequence_range,
        })
    }
}

impl SequenceBoundRawObservationReceipt {
    #[must_use]
    pub const fn receipt(&self) -> &RawObservationReceipt {
        &self.receipt
    }

    #[must_use]
    pub const fn local_sequence_range(&self) -> LocalRecordSequenceRange {
        self.local_sequence_range
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawPackedRangeReceipt {
    chain_id: ChainId,
    source_id: SourceId,
    local_sequence_range: LocalRecordSequenceRange,
    input_logical_manifest_count: u64,
    pack_manifest_sha256: [u8; 32],
    output_object_size_bytes: u64,
    previous_root_sha256: [u8; 32],
    current_root_sha256: [u8; 32],
    completed_at: KnownTime,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawArchiveCheckpointEntryV2 {
    manifest_id: ManifestId,
    manifest_sha256: [u8; 32],
    local_sequence_range: LocalRecordSequenceRange,
}

impl RawArchiveCheckpointEntryV2 {
    #[must_use]
    pub const fn new(
        manifest_id: ManifestId,
        manifest_sha256: [u8; 32],
        local_sequence_range: LocalRecordSequenceRange,
    ) -> Self {
        Self {
            manifest_id,
            manifest_sha256,
            local_sequence_range,
        }
    }

    #[must_use]
    pub const fn manifest_id(&self) -> &ManifestId {
        &self.manifest_id
    }

    #[must_use]
    pub const fn manifest_sha256(&self) -> [u8; 32] {
        self.manifest_sha256
    }

    #[must_use]
    pub const fn local_sequence_range(&self) -> LocalRecordSequenceRange {
        self.local_sequence_range
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawArchiveCheckpointEntriesV2 {
    entries: Vec<RawArchiveCheckpointEntryV2>,
    first_local_sequence: LocalRecordSequence,
    last_local_sequence: LocalRecordSequence,
}

impl RawArchiveCheckpointEntriesV2 {
    pub fn try_new(entries: Vec<RawArchiveCheckpointEntryV2>) -> Result<Self, ArchiveError> {
        if entries.is_empty() || entries.len() > 4_096 {
            return Err(ArchiveError::InvalidInput(
                "raw archive checkpoint V2 entry count",
            ));
        }
        let mut manifest_ids = BTreeSet::new();
        let mut manifest_hashes = BTreeSet::new();
        for pair in entries.windows(2) {
            if pair[0].local_sequence_range.end().checked_next()?
                != pair[1].local_sequence_range.start()
            {
                return Err(ArchiveError::InvalidInput(
                    "raw archive checkpoint V2 ranges are not contiguous",
                ));
            }
        }
        for entry in &entries {
            if !manifest_ids.insert(entry.manifest_id.as_str())
                || !manifest_hashes.insert(entry.manifest_sha256)
            {
                return Err(ArchiveError::InvalidInput(
                    "raw archive checkpoint V2 receipt keys are duplicated",
                ));
            }
        }
        let first_local_sequence = entries[0].local_sequence_range.start();
        let last_local_sequence = entries
            .last()
            .ok_or(ArchiveError::InvalidInput(
                "raw archive checkpoint V2 entries are empty",
            ))?
            .local_sequence_range
            .end();
        Ok(Self {
            entries,
            first_local_sequence,
            last_local_sequence,
        })
    }

    #[must_use]
    pub fn entries(&self) -> &[RawArchiveCheckpointEntryV2] {
        &self.entries
    }

    #[must_use]
    pub const fn first_local_sequence(&self) -> LocalRecordSequence {
        self.first_local_sequence
    }

    #[must_use]
    pub const fn last_local_sequence(&self) -> LocalRecordSequence {
        self.last_local_sequence
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawArchiveRootLeaseIdentity {
    root_sha256: [u8; 32],
    relative_path: PathBuf,
}

impl RawArchiveRootLeaseIdentity {
    #[must_use]
    pub fn new(root_sha256: [u8; 32]) -> Self {
        let encoded = root_sha256
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        Self {
            root_sha256,
            relative_path: PathBuf::from(format!("leases/root-{encoded}.lease")),
        }
    }

    #[must_use]
    pub const fn root_sha256(&self) -> [u8; 32] {
        self.root_sha256
    }

    #[must_use]
    pub fn relative_path(&self) -> &Path {
        &self.relative_path
    }
}

impl RawPackedRangeReceipt {
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        chain_id: ChainId,
        source_id: SourceId,
        local_sequence_range: LocalRecordSequenceRange,
        input_logical_manifest_count: u64,
        pack_manifest_sha256: [u8; 32],
        output_object_size_bytes: u64,
        previous_root_sha256: [u8; 32],
        current_root_sha256: [u8; 32],
        completed_at: KnownTime,
    ) -> Result<Self, ArchiveError> {
        if input_logical_manifest_count < 2
            || input_logical_manifest_count > local_sequence_range.len()
            || output_object_size_bytes == 0
            || previous_root_sha256 == current_root_sha256
        {
            return Err(ArchiveError::InvalidInput(
                "packed range receipt count, size, or root transition",
            ));
        }
        Ok(Self {
            chain_id,
            source_id,
            local_sequence_range,
            input_logical_manifest_count,
            pack_manifest_sha256,
            output_object_size_bytes,
            previous_root_sha256,
            current_root_sha256,
            completed_at,
        })
    }

    #[must_use]
    pub const fn chain_id(&self) -> &ChainId {
        &self.chain_id
    }

    #[must_use]
    pub const fn source_id(&self) -> &SourceId {
        &self.source_id
    }

    #[must_use]
    pub const fn local_sequence_range(&self) -> LocalRecordSequenceRange {
        self.local_sequence_range
    }

    #[must_use]
    pub const fn input_logical_manifest_count(&self) -> u64 {
        self.input_logical_manifest_count
    }

    #[must_use]
    pub const fn pack_manifest_sha256(&self) -> [u8; 32] {
        self.pack_manifest_sha256
    }

    #[must_use]
    pub const fn output_object_size_bytes(&self) -> u64 {
        self.output_object_size_bytes
    }

    #[must_use]
    pub const fn previous_root_sha256(&self) -> [u8; 32] {
        self.previous_root_sha256
    }

    #[must_use]
    pub const fn current_root_sha256(&self) -> [u8; 32] {
        self.current_root_sha256
    }

    #[must_use]
    pub const fn completed_at(&self) -> KnownTime {
        self.completed_at
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawArchiveMaintenanceStatistics {
    logical_manifest_count: u64,
    logical_row_count: u64,
    physical_data_object_count: u64,
    packed_range_count: u64,
    logical_data_bytes: u64,
    physical_data_bytes: u64,
    index_bytes: u64,
    index_inode_count: u64,
    pending_pack_manifest_count: u64,
}

impl RawArchiveMaintenanceStatistics {
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        logical_manifest_count: u64,
        logical_row_count: u64,
        physical_data_object_count: u64,
        packed_range_count: u64,
        logical_data_bytes: u64,
        physical_data_bytes: u64,
        index_bytes: u64,
        index_inode_count: u64,
        pending_pack_manifest_count: u64,
    ) -> Result<Self, ArchiveError> {
        let empty = logical_manifest_count == 0;
        let empty_fields_are_zero = logical_row_count == 0
            && physical_data_object_count == 0
            && packed_range_count == 0
            && logical_data_bytes == 0
            && physical_data_bytes == 0
            && index_bytes == 0
            && index_inode_count == 0
            && pending_pack_manifest_count == 0;
        let populated_fields_are_valid = logical_row_count > 0
            && physical_data_object_count > 0
            && physical_data_object_count <= logical_manifest_count
            && packed_range_count <= physical_data_object_count
            && logical_data_bytes > 0
            && physical_data_bytes > 0
            && index_bytes > 0
            && index_inode_count > 0
            && pending_pack_manifest_count <= logical_manifest_count;
        if (empty && !empty_fields_are_zero) || (!empty && !populated_fields_are_valid) {
            return Err(ArchiveError::InvalidInput(
                "raw archive maintenance statistics are inconsistent",
            ));
        }
        Ok(Self {
            logical_manifest_count,
            logical_row_count,
            physical_data_object_count,
            packed_range_count,
            logical_data_bytes,
            physical_data_bytes,
            index_bytes,
            index_inode_count,
            pending_pack_manifest_count,
        })
    }

    #[must_use]
    pub const fn logical_manifest_count(self) -> u64 {
        self.logical_manifest_count
    }

    #[must_use]
    pub const fn logical_row_count(self) -> u64 {
        self.logical_row_count
    }

    #[must_use]
    pub const fn physical_data_object_count(self) -> u64 {
        self.physical_data_object_count
    }

    #[must_use]
    pub const fn packed_range_count(self) -> u64 {
        self.packed_range_count
    }

    #[must_use]
    pub const fn logical_data_bytes(self) -> u64 {
        self.logical_data_bytes
    }

    #[must_use]
    pub const fn physical_data_bytes(self) -> u64 {
        self.physical_data_bytes
    }

    #[must_use]
    pub const fn index_bytes(self) -> u64 {
        self.index_bytes
    }

    #[must_use]
    pub const fn index_inode_count(self) -> u64 {
        self.index_inode_count
    }

    #[must_use]
    pub const fn pending_pack_manifest_count(self) -> u64 {
        self.pending_pack_manifest_count
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorPolicy {
    ContiguousNativeOffset,
    MonotonicByteOffset,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LocalRecordSequence(NonZeroU64);

impl LocalRecordSequence {
    pub fn try_new(value: u64) -> Result<Self, ArchiveError> {
        NonZeroU64::new(value)
            .map(Self)
            .ok_or(ArchiveError::InvalidInput("local record sequence is zero"))
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0.get()
    }

    pub fn checked_next(self) -> Result<Self, ArchiveError> {
        self.checked_advance_by(1)
    }

    pub fn checked_advance_by(self, count: u64) -> Result<Self, ArchiveError> {
        self.get()
            .checked_add(count)
            .and_then(NonZeroU64::new)
            .map(Self)
            .ok_or(ArchiveError::InvalidInput(
                "local record sequence overflows",
            ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LocalRecordSequenceRange {
    start: LocalRecordSequence,
    end: LocalRecordSequence,
}

impl LocalRecordSequenceRange {
    pub fn try_new(
        start: LocalRecordSequence,
        end: LocalRecordSequence,
    ) -> Result<Self, ArchiveError> {
        if start > end {
            return Err(ArchiveError::InvalidInput("local record sequence range"));
        }
        Ok(Self { start, end })
    }

    #[must_use]
    pub const fn start(self) -> LocalRecordSequence {
        self.start
    }

    #[must_use]
    pub const fn end(self) -> LocalRecordSequence {
        self.end
    }

    #[must_use]
    pub const fn len(self) -> u64 {
        self.end.get() - self.start.get() + 1
    }

    #[must_use]
    pub const fn contains(self, sequence: LocalRecordSequence) -> bool {
        sequence.get() >= self.start.get() && sequence.get() <= self.end.get()
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        false
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SequencedSourceObservation<'a> {
    observation: &'a SourceObservation,
    local_sequence: LocalRecordSequence,
}

impl<'a> SequencedSourceObservation<'a> {
    #[must_use]
    pub const fn observation(&self) -> &'a SourceObservation {
        self.observation
    }

    #[must_use]
    pub const fn local_sequence(&self) -> LocalRecordSequence {
        self.local_sequence
    }
}

#[derive(Debug, Clone)]
pub struct OwnedSequencedSourceObservation {
    observation: SourceObservation,
    local_sequence: LocalRecordSequence,
}

impl OwnedSequencedSourceObservation {
    #[must_use]
    pub const fn new(observation: SourceObservation, local_sequence: LocalRecordSequence) -> Self {
        Self {
            observation,
            local_sequence,
        }
    }

    #[must_use]
    pub const fn observation(&self) -> &SourceObservation {
        &self.observation
    }

    #[must_use]
    pub const fn local_sequence(&self) -> LocalRecordSequence {
        self.local_sequence
    }

    #[must_use]
    pub fn into_observation(self) -> SourceObservation {
        self.observation
    }

    #[must_use]
    pub fn into_parts(self) -> (LocalRecordSequence, SourceObservation) {
        (self.local_sequence, self.observation)
    }
}

#[derive(Debug, Clone)]
pub struct RawObservationBatch {
    chain_id: ChainId,
    observations: Vec<SourceObservation>,
    spool_manifest_blake3: [u8; 32],
    spool_segment_blake3: [u8; 32],
    cursor_policy: CursorPolicy,
    first_local_sequence: Option<LocalRecordSequence>,
    last_local_sequence: Option<LocalRecordSequence>,
}

impl RawObservationBatch {
    pub fn try_new(
        chain_id: ChainId,
        observations: Vec<SourceObservation>,
        spool_manifest_blake3: [u8; 32],
        spool_segment_blake3: [u8; 32],
    ) -> Result<Self, ArchiveError> {
        let first = observations
            .first()
            .ok_or(ArchiveError::InvalidInput("raw observation batch is empty"))?;
        let mut previous = first.cursor().offset();
        for (index, observation) in observations.iter().enumerate() {
            if observation.source_id() != first.source_id()
                || observation.source_version() != first.source_version()
                || observation.observation_class() != first.observation_class()
                || observation.cursor().epoch() != first.cursor().epoch()
                || observation.parser_schema_version() != first.parser_schema_version()
            {
                return Err(ArchiveError::InvalidInput(
                    "raw observation batch metadata is inconsistent",
                ));
            }
            if index != 0 {
                let expected = previous.checked_add(1).ok_or(ArchiveError::InvalidInput(
                    "raw observation cursor overflows",
                ))?;
                if observation.cursor().offset() != expected {
                    return Err(ArchiveError::InvalidInput(
                        "raw observation cursors are not contiguous",
                    ));
                }
                previous = observation.cursor().offset();
            }
        }
        Ok(Self {
            chain_id,
            observations,
            spool_manifest_blake3,
            spool_segment_blake3,
            cursor_policy: CursorPolicy::ContiguousNativeOffset,
            first_local_sequence: None,
            last_local_sequence: None,
        })
    }

    /// Validates byte-cursor shape and local ordering without claiming that the
    /// source is qualified for runtime admission.
    pub fn try_new_byte_offsets(
        chain_id: ChainId,
        observations: Vec<SourceObservation>,
        spool_manifest_blake3: [u8; 32],
        spool_segment_blake3: [u8; 32],
        first_local_sequence: LocalRecordSequence,
    ) -> Result<Self, ArchiveError> {
        let first = observations
            .first()
            .ok_or(ArchiveError::InvalidInput("raw observation batch is empty"))?;
        if matches!(
            first.observation_class(),
            hl_protocol::ObservationClass::CommittedBlock
                | hl_protocol::ObservationClass::HistoricalBlock
        ) {
            return Err(ArchiveError::InvalidInput(
                "byte-offset cursor policy is incompatible with block-height observation class",
            ));
        }
        let mut previous_offset = first.cursor().offset();
        for observation in observations.iter().skip(1) {
            if observation.source_id() != first.source_id()
                || observation.source_version() != first.source_version()
                || observation.observation_class() != first.observation_class()
                || observation.parser_schema_version() != first.parser_schema_version()
            {
                return Err(ArchiveError::InvalidInput(
                    "raw observation batch metadata is inconsistent",
                ));
            }
            if observation.cursor().epoch() != first.cursor().epoch() {
                return Err(ArchiveError::InvalidInput(
                    "raw observation batch cursor epochs are inconsistent",
                ));
            }
            match observation.cursor().offset().cmp(&previous_offset) {
                std::cmp::Ordering::Less => {
                    return Err(ArchiveError::InvalidInput(
                        "raw observation byte offsets regress",
                    ));
                }
                std::cmp::Ordering::Equal => {
                    return Err(ArchiveError::InvalidInput(
                        "raw observation byte offsets are duplicated",
                    ));
                }
                std::cmp::Ordering::Greater => {
                    previous_offset = observation.cursor().offset();
                }
            }
        }
        let last_local_sequence =
            validate_local_sequence_span(first_local_sequence, observations.len())?;
        Ok(Self {
            chain_id,
            observations,
            spool_manifest_blake3,
            spool_segment_blake3,
            cursor_policy: CursorPolicy::MonotonicByteOffset,
            first_local_sequence: Some(first_local_sequence),
            last_local_sequence: Some(last_local_sequence),
        })
    }

    #[must_use]
    pub fn observations(&self) -> &[SourceObservation] {
        &self.observations
    }

    #[must_use]
    pub const fn cursor_policy(&self) -> CursorPolicy {
        self.cursor_policy
    }

    #[must_use]
    pub const fn first_local_sequence(&self) -> Option<LocalRecordSequence> {
        self.first_local_sequence
    }

    #[must_use]
    pub const fn last_local_sequence(&self) -> Option<LocalRecordSequence> {
        self.last_local_sequence
    }

    #[must_use]
    pub fn local_sequence_range(&self) -> Option<LocalRecordSequenceRange> {
        self.first_local_sequence
            .zip(self.last_local_sequence)
            .map(|(start, end)| LocalRecordSequenceRange { start, end })
    }

    pub fn sequenced_observations(
        &self,
    ) -> Option<impl ExactSizeIterator<Item = Result<SequencedSourceObservation<'_>, ArchiveError>>>
    {
        self.first_local_sequence.map(|first_local_sequence| {
            self.observations
                .iter()
                .enumerate()
                .map(move |(index, observation)| {
                    let advance_by = u64::try_from(index).map_err(|_| {
                        ArchiveError::InvalidInput("local record sequence overflows")
                    })?;
                    let local_sequence = first_local_sequence.checked_advance_by(advance_by)?;
                    Ok(SequencedSourceObservation {
                        observation,
                        local_sequence,
                    })
                })
        })
    }

    #[must_use]
    pub const fn chain_id(&self) -> &ChainId {
        &self.chain_id
    }

    #[must_use]
    pub const fn spool_manifest_blake3(&self) -> [u8; 32] {
        self.spool_manifest_blake3
    }

    #[must_use]
    pub const fn spool_segment_blake3(&self) -> [u8; 32] {
        self.spool_segment_blake3
    }
}

fn validate_local_sequence_span(
    first: LocalRecordSequence,
    count: usize,
) -> Result<LocalRecordSequence, ArchiveError> {
    let last_index = count
        .checked_sub(1)
        .ok_or(ArchiveError::InvalidInput("raw observation batch is empty"))?;
    let advance_by = u64::try_from(last_index)
        .map_err(|_| ArchiveError::InvalidInput("local record sequence overflows"))?;
    first.checked_advance_by(advance_by)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawObservationRange {
    epoch: String,
    start_offset: u64,
    end_offset: u64,
}

impl RawObservationRange {
    pub fn try_new(
        epoch: impl Into<String>,
        start_offset: u64,
        end_offset: u64,
    ) -> Result<Self, ArchiveError> {
        let epoch = epoch.into();
        validate_identity(&epoch, "raw observation cursor epoch")?;
        if start_offset > end_offset {
            return Err(ArchiveError::InvalidInput("raw observation cursor range"));
        }
        Ok(Self {
            epoch,
            start_offset,
            end_offset,
        })
    }

    #[must_use]
    pub fn epoch(&self) -> &str {
        &self.epoch
    }

    #[must_use]
    pub const fn start_offset(&self) -> u64 {
        self.start_offset
    }

    #[must_use]
    pub const fn end_offset(&self) -> u64 {
        self.end_offset
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawArchiveObject {
    relative_path: PathBuf,
    sha256: [u8; 32],
    size_bytes: u64,
    row_count: u64,
    chain_id: ChainId,
    source_id: SourceId,
    cursor_range: RawObservationRange,
    cursor_policy: CursorPolicy,
    local_sequence_range: Option<LocalRecordSequenceRange>,
}

impl RawArchiveObject {
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        relative_path: PathBuf,
        sha256: [u8; 32],
        size_bytes: u64,
        row_count: u64,
        chain_id: ChainId,
        source_id: SourceId,
        cursor_range: RawObservationRange,
    ) -> Result<Self, ArchiveError> {
        validate_relative_path(&relative_path)?;
        if size_bytes == 0 || row_count == 0 {
            return Err(ArchiveError::InvalidInput(
                "raw archive object size or row count",
            ));
        }
        Ok(Self {
            relative_path,
            sha256,
            size_bytes,
            row_count,
            chain_id,
            source_id,
            cursor_range,
            cursor_policy: CursorPolicy::ContiguousNativeOffset,
            local_sequence_range: None,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn try_new_byte_offsets(
        relative_path: PathBuf,
        sha256: [u8; 32],
        size_bytes: u64,
        row_count: u64,
        chain_id: ChainId,
        source_id: SourceId,
        cursor_range: RawObservationRange,
        local_sequence_range: LocalRecordSequenceRange,
    ) -> Result<Self, ArchiveError> {
        if row_count != local_sequence_range.len() {
            return Err(ArchiveError::InvalidInput(
                "raw archive object local sequence span",
            ));
        }
        let mut object = Self::try_new(
            relative_path,
            sha256,
            size_bytes,
            row_count,
            chain_id,
            source_id,
            cursor_range,
        )?;
        object.cursor_policy = CursorPolicy::MonotonicByteOffset;
        object.local_sequence_range = Some(local_sequence_range);
        Ok(object)
    }

    #[must_use]
    pub fn relative_path(&self) -> &std::path::Path {
        &self.relative_path
    }

    #[must_use]
    pub const fn sha256(&self) -> [u8; 32] {
        self.sha256
    }

    #[must_use]
    pub const fn size_bytes(&self) -> u64 {
        self.size_bytes
    }

    #[must_use]
    pub const fn row_count(&self) -> u64 {
        self.row_count
    }

    #[must_use]
    pub const fn source_id(&self) -> &SourceId {
        &self.source_id
    }

    #[must_use]
    pub const fn chain_id(&self) -> &ChainId {
        &self.chain_id
    }

    #[must_use]
    pub const fn cursor_range(&self) -> &RawObservationRange {
        &self.cursor_range
    }

    #[must_use]
    pub const fn cursor_policy(&self) -> CursorPolicy {
        self.cursor_policy
    }

    #[must_use]
    pub const fn local_sequence_range(&self) -> Option<LocalRecordSequenceRange> {
        self.local_sequence_range
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedRawManifest {
    manifest_id: ManifestId,
    manifest_sha256: [u8; 32],
    schema_fingerprint: [u8; 32],
    rolling_content_sha256: [u8; 32],
    spool_manifest_blake3: [u8; 32],
    spool_segment_blake3: [u8; 32],
    object: RawArchiveObject,
}

impl VerifiedRawManifest {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        manifest_id: ManifestId,
        manifest_sha256: [u8; 32],
        schema_fingerprint: [u8; 32],
        rolling_content_sha256: [u8; 32],
        spool_manifest_blake3: [u8; 32],
        spool_segment_blake3: [u8; 32],
        object: RawArchiveObject,
    ) -> Self {
        Self {
            manifest_id,
            manifest_sha256,
            schema_fingerprint,
            rolling_content_sha256,
            spool_manifest_blake3,
            spool_segment_blake3,
            object,
        }
    }

    #[must_use]
    pub const fn manifest_id(&self) -> &ManifestId {
        &self.manifest_id
    }

    #[must_use]
    pub const fn manifest_sha256(&self) -> [u8; 32] {
        self.manifest_sha256
    }

    #[must_use]
    pub const fn schema_fingerprint(&self) -> [u8; 32] {
        self.schema_fingerprint
    }

    #[must_use]
    pub const fn rolling_content_sha256(&self) -> [u8; 32] {
        self.rolling_content_sha256
    }

    #[must_use]
    pub const fn spool_manifest_blake3(&self) -> [u8; 32] {
        self.spool_manifest_blake3
    }

    #[must_use]
    pub const fn spool_segment_blake3(&self) -> [u8; 32] {
        self.spool_segment_blake3
    }

    #[must_use]
    pub const fn object(&self) -> &RawArchiveObject {
        &self.object
    }

    #[must_use]
    pub const fn cursor_policy(&self) -> CursorPolicy {
        self.object.cursor_policy()
    }

    #[must_use]
    pub const fn local_sequence_range(&self) -> Option<LocalRecordSequenceRange> {
        self.object.local_sequence_range()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceWatermark {
    source_id: SourceId,
    epoch: String,
    offset: u64,
}

impl SourceWatermark {
    pub fn try_new(
        source_id: SourceId,
        epoch: impl Into<String>,
        offset: u64,
    ) -> Result<Self, ArchiveError> {
        let epoch = epoch.into();
        validate_identity(&epoch, "source watermark epoch")?;
        Ok(Self {
            source_id,
            epoch,
            offset,
        })
    }

    #[must_use]
    pub const fn source_id(&self) -> &SourceId {
        &self.source_id
    }

    #[must_use]
    pub fn epoch(&self) -> &str {
        &self.epoch
    }

    #[must_use]
    pub const fn offset(&self) -> u64 {
        self.offset
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveObject {
    relative_path: PathBuf,
    sha256: [u8; 32],
    size_bytes: u64,
    row_count: u64,
    block_range: BlockRange,
}

impl ArchiveObject {
    pub fn try_new(
        relative_path: PathBuf,
        sha256: [u8; 32],
        size_bytes: u64,
        row_count: u64,
        block_range: BlockRange,
    ) -> Result<Self, ArchiveError> {
        validate_relative_path(&relative_path)?;
        if size_bytes == 0 {
            return Err(ArchiveError::InvalidInput(
                "archive object size must be nonzero",
            ));
        }
        Ok(Self {
            relative_path,
            sha256,
            size_bytes,
            row_count,
            block_range,
        })
    }

    #[must_use]
    pub fn relative_path(&self) -> &std::path::Path {
        &self.relative_path
    }

    #[must_use]
    pub const fn sha256(&self) -> [u8; 32] {
        self.sha256
    }

    #[must_use]
    pub const fn size_bytes(&self) -> u64 {
        self.size_bytes
    }

    #[must_use]
    pub const fn row_count(&self) -> u64 {
        self.row_count
    }

    #[must_use]
    pub const fn block_range(&self) -> BlockRange {
        self.block_range
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedManifest {
    manifest_id: ManifestId,
    chain_id: ChainId,
    object_count: u64,
    row_count: u64,
    block_range: BlockRange,
    manifest_sha256: [u8; 32],
    previous_manifest_sha256: Option<[u8; 32]>,
    schema_fingerprints: BTreeMap<String, [u8; 32]>,
    source_watermarks: Vec<SourceWatermark>,
    objects: Vec<ArchiveObject>,
}

impl VerifiedManifest {
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        manifest_id: ManifestId,
        chain_id: ChainId,
        row_count: u64,
        block_range: BlockRange,
        manifest_sha256: [u8; 32],
        previous_manifest_sha256: Option<[u8; 32]>,
        schema_fingerprints: BTreeMap<String, [u8; 32]>,
        source_watermarks: Vec<SourceWatermark>,
        objects: Vec<ArchiveObject>,
    ) -> Result<Self, ArchiveError> {
        if schema_fingerprints.is_empty() {
            return Err(ArchiveError::InvalidInput(
                "verified manifest requires a schema fingerprint",
            ));
        }
        let object_count = u64::try_from(objects.len())
            .map_err(|_| ArchiveError::InvalidInput("archive object count exceeds u64"))?;
        Ok(Self {
            manifest_id,
            chain_id,
            object_count,
            row_count,
            block_range,
            manifest_sha256,
            previous_manifest_sha256,
            schema_fingerprints,
            source_watermarks,
            objects,
        })
    }

    #[must_use]
    pub const fn manifest_id(&self) -> &ManifestId {
        &self.manifest_id
    }

    #[must_use]
    pub const fn chain_id(&self) -> &ChainId {
        &self.chain_id
    }

    #[must_use]
    pub const fn object_count(&self) -> u64 {
        self.object_count
    }

    #[must_use]
    pub const fn row_count(&self) -> u64 {
        self.row_count
    }

    #[must_use]
    pub const fn block_range(&self) -> BlockRange {
        self.block_range
    }

    #[must_use]
    pub const fn manifest_sha256(&self) -> [u8; 32] {
        self.manifest_sha256
    }

    #[must_use]
    pub const fn previous_manifest_sha256(&self) -> Option<[u8; 32]> {
        self.previous_manifest_sha256
    }

    #[must_use]
    pub const fn schema_fingerprints(&self) -> &BTreeMap<String, [u8; 32]> {
        &self.schema_fingerprints
    }

    #[must_use]
    pub fn source_watermarks(&self) -> &[SourceWatermark] {
        &self.source_watermarks
    }

    #[must_use]
    pub fn objects(&self) -> &[ArchiveObject] {
        &self.objects
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ArchiveError {
    #[error("archive I/O failed while {0}")]
    Io(&'static str),
    #[error("manifest verification failed: {0}")]
    ManifestVerification(&'static str),
    #[error("archive range is unavailable")]
    RangeUnavailable,
    #[error("archive schema fingerprint mismatch")]
    SchemaMismatch,
    #[error("archive object is corrupt: {0}")]
    CorruptObject(String),
    #[error("archive contains conflicting canonical content at block {0:?}")]
    ConflictingBlock(BlockHeight),
    #[error(
        "archive contains a conflicting raw range for source {source_id} epoch {epoch} offsets {start}..={end}"
    )]
    ConflictingRawRange {
        source_id: SourceId,
        epoch: String,
        start: u64,
        end: u64,
    },
    #[error("archive input is invalid: {0}")]
    InvalidInput(&'static str),
    #[error("archive path is unsafe")]
    UnsafePath,
    #[error("archive writer is already active")]
    WriterBusy,
    #[error("canonical archive codec failed: {0}")]
    Codec(String),
    #[error(transparent)]
    Capacity(#[from] RawArchiveCapacityRejection),
    #[error("raw archive receipt index rebuild is required")]
    ReceiptIndexRebuildRequired,
}

impl ArchiveError {
    #[must_use]
    pub const fn reason_code(&self) -> &'static str {
        match self {
            Self::Io(_) => "archive.io",
            Self::ManifestVerification(_) => "archive.manifest_verification",
            Self::RangeUnavailable => "archive.range_unavailable",
            Self::SchemaMismatch => "archive.schema_mismatch",
            Self::CorruptObject(_) => "archive.corrupt_object",
            Self::ConflictingBlock(_) => "archive.conflicting_block",
            Self::ConflictingRawRange { .. } => "archive.conflicting_raw_range",
            Self::InvalidInput(_) => "archive.invalid_input",
            Self::UnsafePath => "archive.unsafe_path",
            Self::WriterBusy => "archive.writer_busy",
            Self::Codec(_) => "archive.codec",
            Self::Capacity(rejection) => rejection.reason_code(),
            Self::ReceiptIndexRebuildRequired => "archive.receipt_index_rebuild_required",
        }
    }
}

pub type BlockIterator = Box<dyn Iterator<Item = Result<BlockEnvelope, ArchiveError>> + Send>;
pub type RawObservationIterator =
    Box<dyn Iterator<Item = Result<SourceObservation, ArchiveError>> + Send>;
pub type SequencedRawObservationIterator =
    Box<dyn Iterator<Item = Result<OwnedSequencedSourceObservation, ArchiveError>> + Send>;

pub trait CanonicalArchive: Send + Sync {
    fn append_block(&self, block: &BlockEnvelope) -> Result<ArchiveReceipt, ArchiveError>;

    fn read_range(&self, chain: &ChainId, range: BlockRange)
    -> Result<BlockIterator, ArchiveError>;

    fn plan_range(
        &self,
        chain: &ChainId,
        range: BlockRange,
    ) -> Result<Vec<VerifiedManifest>, ArchiveError>;

    fn verify_manifest(&self, manifest: &ManifestId) -> Result<VerifiedManifest, ArchiveError>;

    fn read_manifest_blocks(&self, manifest: &ManifestId) -> Result<BlockIterator, ArchiveError>;
}

pub trait CanonicalArchiveMaintenance: Send + Sync {
    fn compact_range(
        &self,
        chain: &ChainId,
        range: BlockRange,
    ) -> Result<CompactionReceipt, ArchiveError>;
}

pub trait RawObservationArchive: Send + Sync {
    fn append_batch(
        &self,
        batch: &RawObservationBatch,
    ) -> Result<RawObservationReceipt, ArchiveError>;

    fn read_observations(
        &self,
        chain: &ChainId,
        source: &SourceId,
        range: RawObservationRange,
    ) -> Result<RawObservationIterator, ArchiveError>;

    fn read_observations_by_sequence(
        &self,
        chain: &ChainId,
        source: &SourceId,
        range: LocalRecordSequenceRange,
    ) -> Result<SequencedRawObservationIterator, ArchiveError>;

    fn verify_raw_manifest(
        &self,
        manifest: &ManifestId,
    ) -> Result<VerifiedRawManifest, ArchiveError>;

    fn verify_raw_manifest_at_sequence(
        &self,
        manifest: &ManifestId,
        expected_range: LocalRecordSequenceRange,
    ) -> Result<VerifiedRawManifest, ArchiveError> {
        let verified = self.verify_raw_manifest(manifest)?;
        if verified.local_sequence_range() != Some(expected_range) {
            return Err(ArchiveError::ManifestVerification(
                "raw manifest sequence evidence does not match the expected range",
            ));
        }
        Ok(verified)
    }

    fn contains_raw_cursor_epoch(
        &self,
        chain: &ChainId,
        source: &SourceId,
        cursor_epoch: &str,
    ) -> Result<bool, ArchiveError>;
}

fn validate_identity(value: &str, label: &'static str) -> Result<(), ArchiveError> {
    if value.is_empty()
        || value.trim() != value
        || value.len() > 256
        || value.bytes().any(|byte| byte.is_ascii_control())
    {
        return Err(ArchiveError::InvalidInput(label));
    }
    Ok(())
}

fn validate_relative_path(path: &std::path::Path) -> Result<(), ArchiveError> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err(ArchiveError::UnsafePath);
    }
    for component in path.components() {
        match component {
            std::path::Component::Normal(_) => {}
            _ => return Err(ArchiveError::UnsafePath),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use domain_types::{BlockHeight, BlockRange};

    use super::{ArchiveError, ArchiveObject};

    #[test]
    fn archive_object_rejects_unsafe_paths_and_empty_files() {
        let range = BlockRange::new(BlockHeight::new(1), BlockHeight::new(1)).expect("range");

        assert!(matches!(
            ArchiveObject::try_new(PathBuf::from("../escape.parquet"), [1; 32], 1, 1, range),
            Err(ArchiveError::UnsafePath)
        ));
        assert!(matches!(
            ArchiveObject::try_new(PathBuf::from("safe.parquet"), [1; 32], 0, 1, range),
            Err(ArchiveError::InvalidInput(_))
        ));
    }
}
