#![forbid(unsafe_code)]

mod archive;
mod capture_progress;
mod checkpoint;
mod state_store;

pub use archive::{
    ARCHIVE_MANIFEST_SCHEMA_V1, ArchiveError, ArchiveObject, ArchiveReceipt, BlockIterator,
    CanonicalArchive, CanonicalArchiveMaintenance, CompactionReceipt, CursorPolicy,
    LocalRecordSequence, RawArchiveObject, RawObservationArchive, RawObservationBatch,
    RawObservationIterator, RawObservationRange, RawObservationReceipt, SequencedSourceObservation,
    SourceWatermark, VerifiedManifest, VerifiedRawManifest,
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
