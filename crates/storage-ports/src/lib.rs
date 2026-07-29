#![forbid(unsafe_code)]

mod archive;
mod capture_progress;
mod checkpoint;

pub use archive::{
    ARCHIVE_MANIFEST_SCHEMA_V1, ArchiveError, ArchiveObject, ArchiveReceipt, BlockIterator,
    CanonicalArchive, CanonicalArchiveMaintenance, CompactionReceipt, RawArchiveObject,
    RawObservationArchive, RawObservationBatch, RawObservationIterator, RawObservationRange,
    RawObservationReceipt, SourceWatermark, VerifiedManifest, VerifiedRawManifest,
};
pub use capture_progress::{
    ArchivedBlockPlan, CaptureCursor, CaptureProgressStore, PlannedPublication, ProgressError,
    ProgressRecordDisposition, PublicationAcknowledgement,
};
pub use checkpoint::{
    CheckpointPublishDisposition, CheckpointReceipt, CheckpointStoreError, StateCheckpointStore,
};

pub const CRATE_BOOTSTRAPPED: bool = true;
