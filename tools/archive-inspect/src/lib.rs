#![forbid(unsafe_code)]

use std::path::Path;

use canonical_archive::{ArchiveConfig, ArchiveDataset, ArchiveInspection, LocalParquetArchive};
use datafusion::prelude::{ParquetReadOptions, SessionContext};
use storage_ports::ArchiveError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationSummary {
    inspection: ArchiveInspection,
}

impl VerificationSummary {
    #[must_use]
    pub const fn inspection(&self) -> &ArchiveInspection {
        &self.inspection
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CountSummary {
    canonical_events: u64,
    canonical_objects: u64,
}

impl CountSummary {
    #[must_use]
    pub const fn canonical_events(&self) -> u64 {
        self.canonical_events
    }

    #[must_use]
    pub const fn canonical_objects(&self) -> u64 {
        self.canonical_objects
    }
}

pub fn verify(root: impl AsRef<Path>) -> Result<VerificationSummary, InspectError> {
    let archive = open(root.as_ref())?;
    let inspection = archive.inspect()?;
    require_objects(&inspection)?;
    Ok(VerificationSummary { inspection })
}

pub async fn count(root: impl AsRef<Path>) -> Result<CountSummary, InspectError> {
    let root = root.as_ref();
    let archive = open(root)?;
    let inspection = archive.inspect()?;
    require_objects(&inspection)?;
    let context = SessionContext::new();
    let mut actual_total = 0_u64;
    let mut object_count = 0_u64;
    for object in inspection
        .objects()
        .iter()
        .filter(|object| object.dataset() == ArchiveDataset::CanonicalEvents)
    {
        let path = root.join(object.relative_path());
        let path = path.to_str().ok_or(InspectError::NonUtf8Path)?;
        let frame = context
            .read_parquet(path, ParquetReadOptions::default())
            .await
            .map_err(|_| InspectError::Query)?;
        let rows = u64::try_from(frame.count().await.map_err(|_| InspectError::Query)?)
            .map_err(|_| InspectError::CountOverflow)?;
        if rows != object.row_count() {
            return Err(InspectError::RowCountMismatch);
        }
        actual_total = actual_total
            .checked_add(rows)
            .ok_or(InspectError::CountOverflow)?;
        object_count = object_count
            .checked_add(1)
            .ok_or(InspectError::CountOverflow)?;
    }
    if actual_total != inspection.canonical_events() {
        return Err(InspectError::RowCountMismatch);
    }
    Ok(CountSummary {
        canonical_events: actual_total,
        canonical_objects: object_count,
    })
}

fn open(root: &Path) -> Result<LocalParquetArchive, InspectError> {
    let config = ArchiveConfig::production("archive-inspect-v1")?;
    LocalParquetArchive::open(root, config).map_err(InspectError::from)
}

fn require_objects(inspection: &ArchiveInspection) -> Result<(), InspectError> {
    if inspection.objects().is_empty() {
        return Err(InspectError::EmptyArchive);
    }
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum InspectError {
    #[error(transparent)]
    Archive(#[from] ArchiveError),
    #[error("archive has no committed objects")]
    EmptyArchive,
    #[error("archive path is not valid UTF-8")]
    NonUtf8Path,
    #[error("independent Parquet query failed")]
    Query,
    #[error("archive count exceeds u64")]
    CountOverflow,
    #[error("independent Parquet row count disagrees with verified manifest")]
    RowCountMismatch,
}

impl InspectError {
    #[must_use]
    pub const fn reason_code(&self) -> &'static str {
        match self {
            Self::Archive(error) => error.reason_code(),
            Self::EmptyArchive => "archive_inspect.empty_archive",
            Self::NonUtf8Path => "archive_inspect.non_utf8_path",
            Self::Query => "archive_inspect.query",
            Self::CountOverflow => "archive_inspect.count_overflow",
            Self::RowCountMismatch => "archive_inspect.row_count_mismatch",
        }
    }
}
