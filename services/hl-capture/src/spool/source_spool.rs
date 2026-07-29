use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use domain_types::SourceId;
use hl_protocol::{CursorTransition, SourceCursor, SourceObservation};

use super::manifest::load_close_receipt;
use super::{
    AppendReceipt, CloseReceipt, DurabilityPolicy, SegmentHeaderV1, SpoolError, SpoolReader,
    SpoolWriter, inspect_spool, io_error, recover_spool_tail,
};

#[derive(Debug, Clone)]
pub struct SourceSpoolConfig {
    directory: PathBuf,
    source_id: SourceId,
    source_version: String,
    schema_version: String,
    producer_build_hash: [u8; 32],
    durability: DurabilityPolicy,
    segment_target_bytes: u64,
    rotation_interval: Duration,
}

#[derive(Debug, Clone, Copy)]
pub struct SpoolRotationPolicy {
    segment_target_bytes: u64,
    rotation_interval: Duration,
}

impl SpoolRotationPolicy {
    pub fn try_new(
        segment_target_bytes: u64,
        rotation_interval: Duration,
    ) -> Result<Self, SpoolError> {
        if segment_target_bytes == 0
            || rotation_interval.is_zero()
            || i64::try_from(rotation_interval.as_micros()).is_err()
        {
            return Err(SpoolError::SizeOverflow);
        }
        Ok(Self {
            segment_target_bytes,
            rotation_interval,
        })
    }
}

impl SourceSpoolConfig {
    pub fn try_new(
        directory: PathBuf,
        source_id: SourceId,
        source_version: impl Into<String>,
        schema_version: impl Into<String>,
        producer_build_hash: [u8; 32],
        durability: DurabilityPolicy,
        rotation: SpoolRotationPolicy,
    ) -> Result<Self, SpoolError> {
        let source_version = source_version.into();
        let schema_version = schema_version.into();
        SegmentHeaderV1::new(
            source_id.clone(),
            source_version.clone(),
            schema_version.clone(),
            1,
            0,
            producer_build_hash,
        )?;
        Ok(Self {
            directory,
            source_id,
            source_version,
            schema_version,
            producer_build_hash,
            durability,
            segment_target_bytes: rotation.segment_target_bytes,
            rotation_interval: rotation.rotation_interval,
        })
    }
}

#[derive(Debug)]
pub struct SourceSpool {
    config: SourceSpoolConfig,
    writer: Option<SpoolWriter>,
    segment_paths: Vec<PathBuf>,
    closed_segments: Vec<CloseReceipt>,
    last_durable_cursor: Option<SourceCursor>,
    chain_tip: Option<[u8; 32]>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceSpoolAppend {
    durability_receipt: Option<AppendReceipt>,
    closed_segment: Option<CloseReceipt>,
}

impl SourceSpoolAppend {
    #[must_use]
    pub const fn durability_receipt(&self) -> Option<&AppendReceipt> {
        self.durability_receipt.as_ref()
    }

    #[must_use]
    pub const fn closed_segment(&self) -> Option<&CloseReceipt> {
        self.closed_segment.as_ref()
    }

    #[must_use]
    pub fn into_parts(self) -> (Option<AppendReceipt>, Option<CloseReceipt>) {
        (self.durability_receipt, self.closed_segment)
    }
}

impl SourceSpool {
    pub fn open(config: SourceSpoolConfig, created_at_micros: i64) -> Result<Self, SpoolError> {
        fs::create_dir_all(&config.directory)
            .map_err(|source| io_error("creating a source spool directory", source))?;
        let metadata = fs::symlink_metadata(&config.directory)
            .map_err(|source| io_error("reading a source spool directory", source))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(SpoolError::UnsafeSpoolEntry);
        }
        recover_spool_tail(&config.directory)?;
        let inspection = inspect_spool(&config.directory)?;
        let mut last_durable_cursor = None;
        for path in inspection.segment_paths() {
            let reader = SpoolReader::open(path)?;
            validate_header(reader.header(), &config)?;
            for record in reader.read_all()? {
                if let Some(previous) = &last_durable_cursor {
                    validate_successor(record.cursor(), previous)?;
                }
                last_durable_cursor = Some(record.cursor().clone());
            }
        }

        let (writer, active_path) = if let Some(path) = inspection.open_segment_path() {
            let (writer, _) = SpoolWriter::open_recovered(path, config.durability)?;
            validate_header(writer.header(), &config)?;
            let active = writer.segment_path().to_owned();
            (writer, active)
        } else {
            let sequence = inspection
                .last_sequence()
                .map_or(Some(1), |sequence| sequence.checked_add(1))
                .ok_or(SpoolError::SizeOverflow)?;
            let header = SegmentHeaderV1::new(
                config.source_id.clone(),
                config.source_version.clone(),
                config.schema_version.clone(),
                sequence,
                created_at_micros,
                config.producer_build_hash,
            )?;
            let writer = SpoolWriter::create(&config.directory, header, config.durability)?;
            let active = writer.segment_path().to_owned();
            (writer, active)
        };
        let mut segment_paths = inspection.segment_paths().to_vec();
        let closed_segments = segment_paths
            .iter()
            .filter(|path| Some(path.as_path()) != inspection.open_segment_path())
            .map(load_close_receipt)
            .collect::<Result<Vec<_>, _>>()?;
        if segment_paths.last() != Some(&active_path) {
            segment_paths.push(active_path);
        }
        Ok(Self {
            config,
            writer: Some(writer),
            segment_paths,
            closed_segments,
            last_durable_cursor,
            chain_tip: inspection.chain_tip(),
        })
    }

    pub fn append(
        &mut self,
        observation: &SourceObservation,
        durable_at_micros: i64,
    ) -> Result<SourceSpoolAppend, SpoolError> {
        if let Some(previous) = &self.last_durable_cursor {
            validate_successor(observation.cursor(), previous)?;
        }
        let closed_segment = if self.rotation_due(durable_at_micros)? {
            Some(self.rotate(durable_at_micros)?)
        } else {
            None
        };
        let durability_receipt = self
            .writer
            .as_mut()
            .ok_or(SpoolError::SegmentClosed)?
            .append(observation, durable_at_micros)?;
        if let Some(receipt) = &durability_receipt {
            self.last_durable_cursor = Some(receipt.durable_cursor.clone());
        }
        Ok(SourceSpoolAppend {
            durability_receipt,
            closed_segment,
        })
    }

    #[must_use]
    pub fn active_segment_path(&self) -> &Path {
        self.writer
            .as_ref()
            .expect("SourceSpool always owns an active writer")
            .segment_path()
    }

    #[must_use]
    pub fn verified_segment_paths(&self) -> &[PathBuf] {
        &self.segment_paths
    }

    #[must_use]
    pub fn closed_segments(&self) -> &[CloseReceipt] {
        &self.closed_segments
    }

    #[must_use]
    pub fn last_durable_cursor(&self) -> Option<&SourceCursor> {
        self.last_durable_cursor.as_ref()
    }

    #[must_use]
    pub const fn source_id(&self) -> &SourceId {
        &self.config.source_id
    }

    pub fn seal_active(
        &mut self,
        closed_at_micros: i64,
    ) -> Result<Option<CloseReceipt>, SpoolError> {
        if self
            .writer
            .as_ref()
            .ok_or(SpoolError::SegmentClosed)?
            .record_count()
            == 0
        {
            return Ok(None);
        }
        self.rotate(closed_at_micros).map(Some)
    }

    pub fn shutdown(mut self, closed_at_micros: i64) -> Result<Option<CloseReceipt>, SpoolError> {
        let writer = self.writer.take().ok_or(SpoolError::SegmentClosed)?;
        if writer.record_count() == 0 {
            return Ok(None);
        }
        writer.close(closed_at_micros, self.chain_tip).map(Some)
    }

    fn rotation_due(&self, durable_at_micros: i64) -> Result<bool, SpoolError> {
        let writer = self.writer.as_ref().ok_or(SpoolError::SegmentClosed)?;
        if writer.record_count() == 0 {
            return Ok(false);
        }
        let size_due = fs::metadata(writer.segment_path())
            .map_err(|source| io_error("reading the active spool size", source))?
            .len()
            >= self.config.segment_target_bytes;
        let rotation_micros = i64::try_from(self.config.rotation_interval.as_micros())
            .map_err(|_| SpoolError::SizeOverflow)?;
        let age_due = durable_at_micros
            .checked_sub(writer.header().created_at_micros())
            .is_some_and(|age| age >= rotation_micros);
        Ok(size_due || age_due)
    }

    fn rotate(&mut self, closed_at_micros: i64) -> Result<CloseReceipt, SpoolError> {
        let writer = self.writer.take().ok_or(SpoolError::SegmentClosed)?;
        let next_sequence = writer
            .header()
            .segment_sequence()
            .checked_add(1)
            .ok_or(SpoolError::SizeOverflow)?;
        let closed = writer.close(closed_at_micros, self.chain_tip)?;
        self.chain_tip = Some(closed.manifest_hash());
        let header = SegmentHeaderV1::new(
            self.config.source_id.clone(),
            self.config.source_version.clone(),
            self.config.schema_version.clone(),
            next_sequence,
            closed_at_micros,
            self.config.producer_build_hash,
        )?;
        let writer = SpoolWriter::create(&self.config.directory, header, self.config.durability)?;
        self.segment_paths.push(writer.segment_path().to_owned());
        self.writer = Some(writer);
        self.closed_segments.push(closed.clone());
        Ok(closed)
    }
}

fn validate_header(header: &SegmentHeaderV1, config: &SourceSpoolConfig) -> Result<(), SpoolError> {
    if header.source_id() != &config.source_id
        || header.source_version() != config.source_version
        || header.schema_version() != config.schema_version
    {
        return Err(SpoolError::SourceMismatch);
    }
    Ok(())
}

fn validate_successor(cursor: &SourceCursor, previous: &SourceCursor) -> Result<(), SpoolError> {
    match cursor
        .validate_successor_of(previous)
        .map_err(|_| SpoolError::CursorRegression)?
    {
        CursorTransition::Advanced { .. } => Ok(()),
        CursorTransition::Duplicate | CursorTransition::EpochChanged => {
            Err(SpoolError::CursorRegression)
        }
    }
}
