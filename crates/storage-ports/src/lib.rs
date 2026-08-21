#![forbid(unsafe_code)]

mod archive;
mod capture_progress;
mod checkpoint;
mod source_catalog;
mod state_store;

pub use archive::{
    ARCHIVE_MANIFEST_SCHEMA_V1, ArchiveError, ArchiveObject, ArchiveReceipt, BlockIterator,
    CanonicalArchive, CanonicalArchiveMaintenance, CompactionReceipt, CursorPolicy,
    HISTORICAL_OBJECT_MANIFEST_SCHEMA_V1, HistoricalGapStatus, HistoricalObjectManifest,
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
    RawObservationReceipt, RawPackedRangeReceipt, RequesterPaysCost,
    SequenceBoundRawObservationReceipt, SequencedRawObservationIterator,
    SequencedSourceObservation, SourceWatermark, VerifiedManifest, VerifiedRawManifest,
};
pub use capture_progress::{
    ArchivedBlockPlan, CaptureCursor, CaptureProgressStore, HistoricalBackfillCursor,
    HistoricalBackfillProgress, HistoricalGapRecord, HistoricalObjectPlan, PlannedPublication,
    ProgressError, ProgressRecordDisposition, PublicationAcknowledgement,
};
pub use checkpoint::{
    CheckpointPublishDisposition, CheckpointReceipt, CheckpointStoreError, StateCheckpointStore,
};
pub use source_catalog::{SourceCatalogStore, SourceCatalogStoreError};
pub use state_store::{
    AtomicStateCommit, AtomicStateStore, STATE_STORE_CFS, STATE_STORE_SCHEMA,
    StateCommitDisposition, StateCommitReceipt, StateStoreError, admit_column_family_schema,
};

pub const CRATE_BOOTSTRAPPED: bool = true;
