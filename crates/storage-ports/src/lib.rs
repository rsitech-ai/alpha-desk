#![forbid(unsafe_code)]

mod archive;
mod capture_progress;

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

pub const CRATE_BOOTSTRAPPED: bool = true;
