mod header;
mod inspection;
mod manifest;
mod reader;
mod record;
mod recovery;
mod source_spool;
mod writer;

use std::io;

pub use header::{SegmentHeader, SegmentHeaderV1};
pub use inspection::{SpoolInspection, inspect_spool, recover_spool_tail};
pub use manifest::{
    CloseReceipt, ClosedSegmentManifest, ClosedSegmentManifestV1, ClosedSegmentManifestV2,
    MANIFEST_SCHEMA_V1, MANIFEST_SCHEMA_V2,
};
pub use reader::{
    SpoolRead, SpoolReader, SpoolRecordStream, SpoolRecordSummary, validate_segment_bytes,
};
pub use record::SpoolRecord;
pub use recovery::{RecoveryReport, recover_open_segment};
pub use source_spool::{
    SourceSpool, SourceSpoolAppend, SourceSpoolAppendDisposition, SourceSpoolConfig,
    SpoolRotationPolicy,
};
pub use writer::{AppendReceipt, DurabilityPolicy, SpoolWriter};

pub(crate) const MAX_IDENTITY_BYTES: usize = 256;
pub(crate) const MAX_HEADER_BYTES: usize = 4096;
pub(crate) const MAX_PAYLOAD_BYTES: usize = 256 * 1024 * 1024;
pub(crate) const MAX_RECORD_BYTES: usize = MAX_PAYLOAD_BYTES + 4096;

#[derive(Debug, thiserror::Error)]
pub enum SpoolError {
    #[error("spool I/O failed while {operation}")]
    Io {
        operation: &'static str,
        #[source]
        source: io::Error,
    },
    #[error("spool segment header is invalid")]
    InvalidHeader,
    #[error("spool segment header is incomplete")]
    IncompleteHeader,
    #[error("spool record at byte offset {record_offset} is corrupt")]
    CorruptRecord { record_offset: u64 },
    #[error("spool segment has an incomplete record at byte offset {record_offset}")]
    IncompleteTail { record_offset: u64 },
    #[error("spool observation does not match the segment source")]
    SourceMismatch,
    #[error("spool v1 cannot preserve parse warnings")]
    UnsupportedWarnings,
    #[error("spool observation cursor regressed")]
    CursorRegression,
    #[error("spool observation conflicts with the retained cursor content")]
    CursorConflict,
    #[error("spool observation class is incompatible with the configured cursor policy")]
    CursorPolicyMismatch,
    #[error("spool durability policy is invalid")]
    InvalidDurabilityPolicy,
    #[error("spool durability timestamp is invalid")]
    InvalidTimestamp,
    #[error("spool segment cannot be closed without records")]
    EmptySegment,
    #[error("spool closed-segment manifest is invalid")]
    InvalidManifest,
    #[error("spool closed-segment manifest already exists")]
    ManifestAlreadyExists,
    #[error("spool directory contains an unsafe entry")]
    UnsafeSpoolEntry,
    #[error("spool directory contains an incomplete manifest publication")]
    IncompleteManifestPublication,
    #[error("spool directory contains a duplicate segment sequence")]
    DuplicateSegmentSequence,
    #[error("spool manifest references a missing segment")]
    ManifestSegmentMissing,
    #[error("spool manifest chain is broken")]
    ManifestChainBroken,
    #[error("spool manifest does not match its segment")]
    ManifestContentMismatch,
    #[error("spool segment size does not match its manifest")]
    SegmentSizeMismatch,
    #[error("spool segment hash does not match its manifest")]
    SegmentHashMismatch,
    #[error("spool directory contains an unexpected open segment")]
    UnexpectedOpenSegment,
    #[error("spool segment is already closed")]
    SegmentClosed,
    #[error("spool recovery is forbidden for a closed segment")]
    ClosedSegment,
    #[error("spool segment sequence already exists")]
    SegmentAlreadyExists,
    #[error("spool size exceeds the supported format")]
    SizeOverflow,
}

impl SpoolError {
    #[must_use]
    pub const fn reason_code(&self) -> &'static str {
        match self {
            Self::Io { .. } => "spool.io",
            Self::InvalidHeader => "spool.invalid_header",
            Self::IncompleteHeader => "spool.incomplete_header",
            Self::CorruptRecord { .. } => "spool.corrupt_record",
            Self::IncompleteTail { .. } => "spool.incomplete_tail",
            Self::SourceMismatch => "spool.source_mismatch",
            Self::UnsupportedWarnings => "spool.unsupported_warnings",
            Self::CursorRegression => "spool.cursor_regression",
            Self::CursorConflict => "spool.cursor_conflict",
            Self::CursorPolicyMismatch => "spool.cursor_policy_mismatch",
            Self::InvalidDurabilityPolicy => "spool.invalid_durability_policy",
            Self::InvalidTimestamp => "spool.invalid_timestamp",
            Self::EmptySegment => "spool.empty_segment",
            Self::InvalidManifest => "spool.invalid_manifest",
            Self::ManifestAlreadyExists => "spool.manifest_already_exists",
            Self::UnsafeSpoolEntry => "spool.unsafe_entry",
            Self::IncompleteManifestPublication => "spool.incomplete_manifest_publication",
            Self::DuplicateSegmentSequence => "spool.duplicate_segment_sequence",
            Self::ManifestSegmentMissing => "spool.manifest_segment_missing",
            Self::ManifestChainBroken => "spool.manifest_chain_broken",
            Self::ManifestContentMismatch => "spool.manifest_content_mismatch",
            Self::SegmentSizeMismatch => "spool.segment_size_mismatch",
            Self::SegmentHashMismatch => "spool.segment_hash_mismatch",
            Self::UnexpectedOpenSegment => "spool.unexpected_open_segment",
            Self::SegmentClosed => "spool.segment_closed",
            Self::ClosedSegment => "spool.closed_segment",
            Self::SegmentAlreadyExists => "spool.segment_already_exists",
            Self::SizeOverflow => "spool.size_overflow",
        }
    }
}

pub(crate) fn io_error(operation: &'static str, source: io::Error) -> SpoolError {
    SpoolError::Io { operation, source }
}
