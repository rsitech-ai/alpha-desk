#![forbid(unsafe_code)]

mod archive;
mod capture_progress;
mod checkpoint;
mod state_store;

pub use archive::{
    ARCHIVE_MANIFEST_SCHEMA_V1, ArchiveError, ArchiveObject, ArchiveReceipt, BlockIterator,
    CanonicalArchive, CanonicalArchiveMaintenance, CompactionReceipt, CursorPolicy,
    LocalRecordSequence, LocalRecordSequenceRange, OwnedSequencedSourceObservation,
    RAW_ARCHIVE_MAXIMUM_DATA_PACK_BYTES, RAW_ARCHIVE_MAXIMUM_EMBEDDED_PACK_MANIFEST_BYTES,
    RAW_ARCHIVE_MAXIMUM_INDEX_PACK_BYTES, RAW_ARCHIVE_MAXIMUM_LOGICAL_MANIFEST_BYTES,
    RAW_ARCHIVE_MAXIMUM_PACK_LOGICAL_INPUTS, RAW_ARCHIVE_MAXIMUM_RELATIVE_PATH_BYTES,
    RAW_ARCHIVE_MAXIMUM_SEQUENCE_PAGE_BYTES, RAW_ARCHIVE_MAXIMUM_SEQUENCE_TREE_DEPTH,
    RawArchiveCapacityBudgets, RawArchiveCapacityRejection, RawArchiveCheckpointEntriesV2,
    RawArchiveCheckpointEntryV2, RawArchiveDurableFormatEnvelope, RawArchiveIndexCapacityEstimate,
    RawArchiveMaintenanceStatistics, RawArchiveObject, RawArchivePackingPolicy,
    RawArchiveProductionCapacityAdmission, RawArchiveRootLeaseIdentity, RawArchiveWorkloadEnvelope,
    RawObservationArchive, RawObservationBatch, RawObservationIterator, RawObservationRange,
    RawObservationReceipt, RawPackedRangeReceipt, SequenceBoundRawObservationReceipt,
    SequencedRawObservationIterator, SequencedSourceObservation, SourceWatermark, VerifiedManifest,
    VerifiedRawManifest,
};
pub use capture_progress::{
    ArchivedBlockPlan, CaptureCursor, CaptureProgressStore, PlannedPublication, ProgressError,
    ProgressRecordDisposition, PublicationAcknowledgement,
};
pub use checkpoint::{
    CheckpointPublishDisposition, CheckpointReceipt, CheckpointStoreError, StateCheckpointStore,
};
pub use state_store::{
    AtomicStateCommit, AtomicStateStore, StateCommitDisposition, StateCommitReceipt,
    StateStoreError,
};

pub const CRATE_BOOTSTRAPPED: bool = true;
