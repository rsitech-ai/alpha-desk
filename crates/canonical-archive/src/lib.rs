#![forbid(unsafe_code)]

mod compactor;
mod fs;
mod inspection;
mod manifest;
mod raw;
mod raw_policy;
mod raw_v2;
pub mod raw_v3;
mod raw_v3_store;
mod reader;
mod schema;
mod writer;

use std::{
    path::{Path, PathBuf},
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

use canonical_events::BlockEnvelope;
use domain_types::{BlockRange, ChainId, KnownTime, ManifestId};
use storage_ports::{
    ArchiveError, ArchiveReceipt, BlockIterator, CanonicalArchive, CanonicalArchiveMaintenance,
    CompactionReceipt, VerifiedManifest,
};

pub use inspection::{ArchiveDataset, ArchiveInspection, InspectedObject};
pub use raw_v3_store::{
    RawArchiveCheckpoint, RawArchiveCheckpointV1, RawArchiveCheckpointV2, RawArchiveGcPlan,
    RawArchiveGcReceipt, RawArchiveRestoreReceipt, RawArchiveRetentionReport,
    RawArchiveRetentionRequest, RawArchiveScrubReport, RawV3Archive,
};
use storage_ports::{
    LocalRecordSequenceRange, RawObservationArchive, RawObservationBatch, RawObservationIterator,
    RawObservationRange, RawObservationReceipt, SequencedRawObservationIterator,
    VerifiedRawManifest,
};

const DEFAULT_MAX_READ_BLOCKS: u64 = 100_000;
const DEFAULT_MAX_READ_BYTES: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveConfig {
    producer_build_id: String,
    fixed_time: Option<KnownTime>,
    max_read_blocks: u64,
    max_read_bytes: u64,
}

impl ArchiveConfig {
    pub fn production(producer_build_id: impl Into<String>) -> Result<Self, ArchiveError> {
        Self::try_new(producer_build_id, None)
    }

    pub fn deterministic_fixture(
        producer_build_id: impl Into<String>,
        fixed_time: KnownTime,
    ) -> Result<Self, ArchiveError> {
        Self::try_new(producer_build_id, Some(fixed_time))
    }

    fn try_new(
        producer_build_id: impl Into<String>,
        fixed_time: Option<KnownTime>,
    ) -> Result<Self, ArchiveError> {
        let producer_build_id = producer_build_id.into();
        if producer_build_id.is_empty()
            || producer_build_id.trim() != producer_build_id
            || producer_build_id.len() > 256
            || producer_build_id
                .bytes()
                .any(|byte| byte.is_ascii_control())
        {
            return Err(ArchiveError::InvalidInput("producer build ID"));
        }
        Ok(Self {
            producer_build_id,
            fixed_time,
            max_read_blocks: DEFAULT_MAX_READ_BLOCKS,
            max_read_bytes: DEFAULT_MAX_READ_BYTES,
        })
    }

    #[must_use]
    pub fn producer_build_id(&self) -> &str {
        &self.producer_build_id
    }

    #[must_use]
    pub const fn max_read_blocks(&self) -> u64 {
        self.max_read_blocks
    }

    #[must_use]
    pub const fn max_read_bytes(&self) -> u64 {
        self.max_read_bytes
    }

    pub fn with_read_limits(
        mut self,
        max_records: u64,
        max_bytes: u64,
    ) -> Result<Self, ArchiveError> {
        if max_records == 0 || max_bytes == 0 {
            return Err(ArchiveError::InvalidInput(
                "archive read limits must be nonzero",
            ));
        }
        self.max_read_blocks = max_records;
        self.max_read_bytes = max_bytes;
        Ok(self)
    }

    pub(crate) fn now(&self) -> Result<KnownTime, ArchiveError> {
        if let Some(fixed) = self.fixed_time {
            return Ok(fixed);
        }
        let duration = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| ArchiveError::Io("reading system time"))?;
        let micros = duration.as_micros();
        let micros = i64::try_from(micros)
            .map_err(|_| ArchiveError::InvalidInput("system time exceeds i64 microseconds"))?;
        KnownTime::from_unix_micros(micros)
            .map_err(|_| ArchiveError::InvalidInput("system time is negative"))
    }
}

#[derive(Debug)]
pub struct LocalParquetArchive {
    root: PathBuf,
    config: ArchiveConfig,
    writer: Mutex<()>,
}

impl LocalParquetArchive {
    pub fn open(root: impl AsRef<Path>, config: ArchiveConfig) -> Result<Self, ArchiveError> {
        let root = root.as_ref();
        if root.as_os_str().is_empty() {
            return Err(ArchiveError::UnsafePath);
        }
        if root.exists() {
            let metadata = std::fs::symlink_metadata(root)
                .map_err(|_| ArchiveError::Io("inspecting archive root"))?;
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Err(ArchiveError::UnsafePath);
            }
        } else {
            std::fs::create_dir_all(root).map_err(|_| ArchiveError::Io("creating archive root"))?;
        }
        let root = root
            .canonicalize()
            .map_err(|_| ArchiveError::Io("canonicalizing archive root"))?;
        Ok(Self {
            root,
            config,
            writer: Mutex::new(()),
        })
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) const fn config(&self) -> &ArchiveConfig {
        &self.config
    }

    pub fn inspect(&self) -> Result<ArchiveInspection, ArchiveError> {
        inspection::inspect(self)
    }
}

impl CanonicalArchive for LocalParquetArchive {
    fn append_block(&self, block: &BlockEnvelope) -> Result<ArchiveReceipt, ArchiveError> {
        let _in_process = self.writer.lock().map_err(|_| ArchiveError::WriterBusy)?;
        writer::append_block(self, block, self.config.now()?)
    }

    fn read_range(
        &self,
        chain: &ChainId,
        range: BlockRange,
    ) -> Result<BlockIterator, ArchiveError> {
        reader::read_range(self, chain, range)
    }

    fn plan_range(
        &self,
        chain: &ChainId,
        range: BlockRange,
    ) -> Result<Vec<VerifiedManifest>, ArchiveError> {
        reader::plan_range(self, chain, range)
    }

    fn verify_manifest(&self, manifest: &ManifestId) -> Result<VerifiedManifest, ArchiveError> {
        reader::verify_block_manifest(self, manifest)
    }

    fn read_manifest_blocks(&self, manifest: &ManifestId) -> Result<BlockIterator, ArchiveError> {
        reader::read_manifest_blocks(self, manifest)
    }
}

impl RawObservationArchive for LocalParquetArchive {
    fn append_batch(
        &self,
        batch: &RawObservationBatch,
    ) -> Result<RawObservationReceipt, ArchiveError> {
        let _in_process = self.writer.lock().map_err(|_| ArchiveError::WriterBusy)?;
        raw::append_batch(self, batch, self.config.now()?)
    }

    fn read_observations(
        &self,
        chain: &domain_types::ChainId,
        source: &domain_types::SourceId,
        range: RawObservationRange,
    ) -> Result<RawObservationIterator, ArchiveError> {
        raw::read_observations(self, chain, source, range)
    }

    fn read_observations_by_sequence(
        &self,
        chain: &domain_types::ChainId,
        source: &domain_types::SourceId,
        range: LocalRecordSequenceRange,
    ) -> Result<SequencedRawObservationIterator, ArchiveError> {
        raw_v2::read_observations_by_sequence(self, chain, source, range)
    }

    fn verify_raw_manifest(
        &self,
        manifest: &ManifestId,
    ) -> Result<VerifiedRawManifest, ArchiveError> {
        raw::verify_raw_manifest(self, manifest)
    }

    fn contains_raw_cursor_epoch(
        &self,
        chain: &domain_types::ChainId,
        source: &domain_types::SourceId,
        cursor_epoch: &str,
    ) -> Result<bool, ArchiveError> {
        raw_v2::contains_cursor_epoch(self, chain, source, cursor_epoch)
    }
}

impl CanonicalArchiveMaintenance for LocalParquetArchive {
    fn compact_range(
        &self,
        chain: &ChainId,
        range: BlockRange,
    ) -> Result<CompactionReceipt, ArchiveError> {
        let _in_process = self.writer.lock().map_err(|_| ArchiveError::WriterBusy)?;
        compactor::compact_range(self, chain, range, self.config.now()?)
    }
}
