#![forbid(unsafe_code)]

use std::path::Path;

use canonical_archive::{
    ArchiveConfig, ArchiveDataset, ArchiveInspection, LocalParquetArchive, RawV3Archive,
    RawV3SourceInspection,
};
use datafusion::prelude::{ParquetReadOptions, SessionContext};
use storage_ports::{
    ArchiveError, LocalRecordSequence, LocalRecordSequenceRange, RawArchiveCapacityBudgets,
    RawArchiveWorkloadEnvelope, RawObservationArchive,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationSummary {
    inspection: ArchiveInspection,
    v3: Option<V3InspectSummary>,
}

impl VerificationSummary {
    #[must_use]
    pub const fn inspection(&self) -> &ArchiveInspection {
        &self.inspection
    }

    #[must_use]
    pub const fn v3(&self) -> Option<&V3InspectSummary> {
        self.v3.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CountSummary {
    canonical_events: u64,
    canonical_objects: u64,
    v3_sources: u64,
    v3_logical_rows: u64,
    v3_logical_manifests: u64,
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

    #[must_use]
    pub const fn v3_sources(&self) -> u64 {
        self.v3_sources
    }

    #[must_use]
    pub const fn v3_logical_rows(&self) -> u64 {
        self.v3_logical_rows
    }

    #[must_use]
    pub const fn v3_logical_manifests(&self) -> u64 {
        self.v3_logical_manifests
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct V3InspectSummary {
    sources: Vec<RawV3SourceInspection>,
}

impl V3InspectSummary {
    #[must_use]
    pub fn sources(&self) -> &[RawV3SourceInspection] {
        &self.sources
    }

    #[must_use]
    pub fn logical_row_count(&self) -> u64 {
        self.sources.iter().fold(0, |total, source| {
            total.saturating_add(source.statistics().logical_row_count())
        })
    }

    #[must_use]
    pub fn logical_manifest_count(&self) -> u64 {
        self.sources.iter().fold(0, |total, source| {
            total.saturating_add(source.scrub().logical_manifest_count())
        })
    }
}

pub fn verify(root: impl AsRef<Path>) -> Result<VerificationSummary, InspectError> {
    let root = root.as_ref();
    let inspection = open(root)?.inspect()?;
    let v3 = optional_v3(root)?;
    require_verified_dataset(&inspection, v3.as_ref())?;
    Ok(VerificationSummary { inspection, v3 })
}

pub async fn count(root: impl AsRef<Path>) -> Result<CountSummary, InspectError> {
    let root = root.as_ref();
    let inspection = open(root)?.inspect()?;
    let canonical = count_canonical(root, &inspection).await?;
    let v3 = count_v3(root)?;
    if inspection.objects().is_empty() && v3.v3_sources == 0 {
        return Err(InspectError::EmptyArchive);
    }
    Ok(CountSummary {
        canonical_events: canonical.canonical_events,
        canonical_objects: canonical.canonical_objects,
        v3_sources: v3.v3_sources,
        v3_logical_rows: v3.v3_logical_rows,
        v3_logical_manifests: v3.v3_logical_manifests,
    })
}

pub fn scrub_v3(root: impl AsRef<Path>) -> Result<V3InspectSummary, InspectError> {
    inspect_v3(root)
}

pub fn stats_v3(root: impl AsRef<Path>) -> Result<V3InspectSummary, InspectError> {
    inspect_v3(root)
}

pub fn health_v3(root: impl AsRef<Path>) -> Result<V3InspectSummary, InspectError> {
    inspect_v3(root)
}

fn inspect_v3(root: impl AsRef<Path>) -> Result<V3InspectSummary, InspectError> {
    optional_v3(root.as_ref())?.ok_or(InspectError::EmptyArchive)
}

fn optional_v3(root: &Path) -> Result<Option<V3InspectSummary>, InspectError> {
    let archive = open_v3(root)?;
    let sources = archive.inspect_sources()?;
    if sources.is_empty() {
        Ok(None)
    } else {
        Ok(Some(V3InspectSummary { sources }))
    }
}

async fn count_canonical(
    root: &Path,
    inspection: &ArchiveInspection,
) -> Result<CountSummary, InspectError> {
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
        v3_sources: 0,
        v3_logical_rows: 0,
        v3_logical_manifests: 0,
    })
}

fn count_v3(root: &Path) -> Result<CountSummary, InspectError> {
    let archive = open_v3(root)?;
    let sources = archive.inspect_sources()?;
    if sources.is_empty() {
        return Ok(CountSummary {
            canonical_events: 0,
            canonical_objects: 0,
            v3_sources: 0,
            v3_logical_rows: 0,
            v3_logical_manifests: 0,
        });
    }
    let mut logical_rows = 0_u64;
    let mut logical_manifests = 0_u64;
    for source in &sources {
        logical_rows = logical_rows
            .checked_add(count_v3_source(&archive, source)?)
            .ok_or(InspectError::CountOverflow)?;
        logical_manifests = logical_manifests
            .checked_add(source.scrub().logical_manifest_count())
            .ok_or(InspectError::CountOverflow)?;
    }
    let source_count = u64::try_from(sources.len()).map_err(|_| InspectError::CountOverflow)?;
    Ok(CountSummary {
        canonical_events: 0,
        canonical_objects: 0,
        v3_sources: source_count,
        v3_logical_rows: logical_rows,
        v3_logical_manifests: logical_manifests,
    })
}

fn count_v3_source(
    archive: &RawV3Archive,
    source: &RawV3SourceInspection,
) -> Result<u64, InspectError> {
    let expected_rows = source.statistics().logical_row_count();
    if expected_rows == 0
        || source.scrub().logical_manifest_count() != source.statistics().logical_manifest_count()
    {
        return Err(InspectError::RowCountMismatch);
    }
    let range = LocalRecordSequenceRange::try_new(
        LocalRecordSequence::try_new(1)?,
        LocalRecordSequence::try_new(expected_rows)?,
    )?;
    let replayed = archive
        .read_observations_by_sequence(source.chain_id(), source.source_id(), range)?
        .try_fold(0_u64, |total, item| {
            item?;
            total.checked_add(1).ok_or(InspectError::CountOverflow)
        })?;
    if replayed != expected_rows {
        return Err(InspectError::RowCountMismatch);
    }
    Ok(expected_rows)
}

fn open(root: &Path) -> Result<LocalParquetArchive, InspectError> {
    let config = ArchiveConfig::production("archive-inspect-v1")?;
    LocalParquetArchive::open(root, config).map_err(InspectError::from)
}

fn open_v3(root: &Path) -> Result<RawV3Archive, InspectError> {
    let config = ArchiveConfig::production("archive-inspect-v3")?;
    let workload = RawArchiveWorkloadEnvelope::try_new(
        100,
        1,
        1_000,
        3_600,
        1_024,
        1_000,
        64 * 1024 * 1024,
        64,
    )
    .map_err(ArchiveError::from)?;
    let budgets = RawArchiveCapacityBudgets::try_new(u64::MAX, u64::MAX, u64::MAX, u64::MAX, true)
        .map_err(ArchiveError::from)?;
    RawV3Archive::open(root, config, workload, budgets).map_err(InspectError::from)
}

fn require_verified_dataset(
    inspection: &ArchiveInspection,
    v3: Option<&V3InspectSummary>,
) -> Result<(), InspectError> {
    if inspection.objects().is_empty() && v3.is_none() {
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
