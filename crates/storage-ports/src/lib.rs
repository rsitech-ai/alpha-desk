#![forbid(unsafe_code)]

mod archive;

pub use archive::{
    ARCHIVE_MANIFEST_SCHEMA_V1, ArchiveError, ArchiveObject, ArchiveReceipt, BlockIterator,
    CanonicalArchive, CanonicalArchiveMaintenance, CompactionReceipt, RawArchiveObject,
    RawObservationArchive, RawObservationBatch, RawObservationIterator, RawObservationRange,
    RawObservationReceipt, SourceWatermark, VerifiedManifest, VerifiedRawManifest,
};

pub const CRATE_BOOTSTRAPPED: bool = true;
