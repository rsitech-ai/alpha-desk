use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use domain_types::SourceId;
use hl_protocol::{CursorTransition, ObservationClass, SourceCursor, SourceObservation};
use storage_ports::{CursorPolicy, LocalRecordSequence};

use super::manifest::load_close_receipt;
use super::record::validate_record;
use super::{
    AppendReceipt, CloseReceipt, DurabilityPolicy, SegmentHeader, SegmentHeaderV1, SpoolError,
    SpoolRead, SpoolReader, SpoolWriter, inspect_spool, io_error, recover_spool_tail,
};

const POLICY_SCHEMA_PREFIX: &str = "hl-spool-policy-v1:";

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
    cursor_policy: CursorPolicy,
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
        Self::try_new_with_cursor_policy(
            directory,
            source_id,
            source_version,
            schema_version,
            producer_build_hash,
            durability,
            rotation,
            CursorPolicy::ContiguousNativeOffset,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn try_new_with_cursor_policy(
        directory: PathBuf,
        source_id: SourceId,
        source_version: impl Into<String>,
        schema_version: impl Into<String>,
        producer_build_hash: [u8; 32],
        durability: DurabilityPolicy,
        rotation: SpoolRotationPolicy,
        cursor_policy: CursorPolicy,
    ) -> Result<Self, SpoolError> {
        let source_version = source_version.into();
        let base_schema_version = schema_version.into();
        if cursor_policy == CursorPolicy::MonotonicByteOffset
            && durability != DurabilityPolicy::FsyncEveryRecord
        {
            return Err(SpoolError::InvalidDurabilityPolicy);
        }
        SegmentHeaderV1::new(
            source_id.clone(),
            source_version.clone(),
            base_schema_version.clone(),
            1,
            0,
            producer_build_hash,
        )?;
        let schema_version = persisted_schema_identity(base_schema_version, cursor_policy)?;
        SegmentHeader::new_with_cursor_policy(
            source_id.clone(),
            source_version.clone(),
            schema_version.clone(),
            1,
            0,
            producer_build_hash,
            cursor_policy,
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
            cursor_policy,
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
    last_record_identity: Option<LastRecordIdentity>,
    retained_segments: Vec<RetainedSegmentIndex>,
    last_local_sequence: Option<LocalRecordSequence>,
    chain_tip: Option<[u8; 32]>,
}

#[derive(Debug, Clone)]
struct LastRecordIdentity {
    cursor: SourceCursor,
}

#[derive(Debug, Clone)]
struct RetainedRecordIdentity {
    observation_class: ObservationClass,
    parser_schema_version: String,
    content_hash: blake3::Hash,
    local_sequence: LocalRecordSequence,
}

#[derive(Debug, Clone)]
struct RetainedSegmentIndex {
    path: PathBuf,
    epoch: String,
    min_offset: u64,
    max_offset: u64,
    first_local_sequence: LocalRecordSequence,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceSpoolAppend {
    durability_receipt: Option<AppendReceipt>,
    closed_segment: Option<CloseReceipt>,
    local_sequence: LocalRecordSequence,
    disposition: SourceSpoolAppendDisposition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceSpoolAppendDisposition {
    Appended,
    Duplicate,
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

    #[must_use]
    pub const fn local_sequence(&self) -> LocalRecordSequence {
        self.local_sequence
    }

    #[must_use]
    pub const fn disposition(&self) -> SourceSpoolAppendDisposition {
        self.disposition
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
        let mut last_durable_cursor: Option<SourceCursor> = None;
        let mut last_record_identity = None;
        let mut last_local_sequence = None;
        let mut retained_segments = Vec::new();
        for path in inspection.segment_paths() {
            let reader = SpoolReader::open(path)?;
            validate_header(reader.header(), &config)?;
            let mut records = reader.stream()?;
            let mut first_in_segment = true;
            let mut retained_segment: Option<RetainedSegmentIndex> = None;
            loop {
                match records.next_record()? {
                    SpoolRead::Record(record) => {
                        validate_observation_policy(
                            config.cursor_policy,
                            record.observation_class(),
                        )?;
                        if let Some(previous) = &last_durable_cursor {
                            if first_in_segment
                                && previous.epoch() != record.cursor().epoch()
                                && retained_segments
                                    .iter()
                                    .any(|index: &RetainedSegmentIndex| {
                                        index.epoch == record.cursor().epoch()
                                    })
                            {
                                return Err(SpoolError::CursorRegression);
                            }
                            validate_successor(
                                config.cursor_policy,
                                record.cursor(),
                                previous,
                                first_in_segment,
                            )?;
                        }
                        let local_sequence = next_local_sequence(last_local_sequence)?;
                        if config.cursor_policy == CursorPolicy::MonotonicByteOffset {
                            match &mut retained_segment {
                                Some(index) => {
                                    if index.epoch != record.cursor().epoch()
                                        || record.cursor().offset() <= index.max_offset
                                    {
                                        return Err(SpoolError::CursorRegression);
                                    }
                                    index.max_offset = record.cursor().offset();
                                }
                                None => {
                                    retained_segment = Some(RetainedSegmentIndex {
                                        path: path.clone(),
                                        epoch: record.cursor().epoch().to_owned(),
                                        min_offset: record.cursor().offset(),
                                        max_offset: record.cursor().offset(),
                                        first_local_sequence: local_sequence,
                                    });
                                }
                            }
                        }
                        last_local_sequence = Some(local_sequence);
                        last_record_identity = Some(LastRecordIdentity {
                            cursor: record.cursor().clone(),
                        });
                        last_durable_cursor = Some(record.cursor().clone());
                        first_in_segment = false;
                    }
                    SpoolRead::EndOfFile => break,
                    SpoolRead::IncompleteTail { record_offset } => {
                        return Err(SpoolError::IncompleteTail { record_offset });
                    }
                }
            }
            if let Some(index) = retained_segment {
                retained_segments.push(index);
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
            let header = SegmentHeader::new_with_cursor_policy(
                config.source_id.clone(),
                config.source_version.clone(),
                config.schema_version.clone(),
                sequence,
                created_at_micros,
                config.producer_build_hash,
                config.cursor_policy,
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
            last_record_identity,
            retained_segments,
            last_local_sequence,
            chain_tip: inspection.chain_tip(),
        })
    }

    pub fn append(
        &mut self,
        observation: &SourceObservation,
        durable_at_micros: i64,
    ) -> Result<SourceSpoolAppend, SpoolError> {
        if self.config.cursor_policy == CursorPolicy::MonotonicByteOffset {
            self.append_byte_offset(observation, durable_at_micros)
        } else {
            self.append_legacy(observation, durable_at_micros)
        }
    }

    fn append_legacy(
        &mut self,
        observation: &SourceObservation,
        durable_at_micros: i64,
    ) -> Result<SourceSpoolAppend, SpoolError> {
        if let Some(previous) = &self.last_durable_cursor {
            validate_legacy_successor(observation.cursor(), previous)?;
        }
        let closed_segment = if self.rotation_due(durable_at_micros)? {
            Some(self.rotate(durable_at_micros)?)
        } else {
            None
        };
        let local_sequence = next_local_sequence(self.last_local_sequence)?;
        let durability_receipt = self
            .writer
            .as_mut()
            .ok_or(SpoolError::SegmentClosed)?
            .append(observation, durable_at_micros)?;
        self.last_record_identity = Some(LastRecordIdentity {
            cursor: observation.cursor().clone(),
        });
        self.last_local_sequence = Some(local_sequence);
        if let Some(receipt) = &durability_receipt {
            self.last_durable_cursor = Some(receipt.durable_cursor.clone());
        }
        Ok(SourceSpoolAppend {
            durability_receipt,
            closed_segment,
            local_sequence,
            disposition: SourceSpoolAppendDisposition::Appended,
        })
    }

    fn append_byte_offset(
        &mut self,
        observation: &SourceObservation,
        durable_at_micros: i64,
    ) -> Result<SourceSpoolAppend, SpoolError> {
        if observation.source_id() != &self.config.source_id
            || observation.source_version() != self.config.source_version
        {
            return Err(SpoolError::SourceMismatch);
        }
        validate_observation_policy(self.config.cursor_policy, observation.observation_class())?;
        validate_record(observation)?;
        validate_timestamp(durable_at_micros)?;

        if let Some(retained) = self.find_retained(observation.cursor())? {
            return duplicate_append(&retained, observation);
        }

        let epoch_changed = if let Some(previous) = self
            .last_record_identity
            .as_ref()
            .map(|identity| &identity.cursor)
        {
            match observation
                .cursor()
                .validate_successor_of(previous)
                .map_err(|_| SpoolError::CursorRegression)?
            {
                CursorTransition::Advanced { .. } => false,
                CursorTransition::EpochChanged => {
                    if self
                        .retained_segments
                        .iter()
                        .any(|index| index.epoch == observation.cursor().epoch())
                    {
                        return Err(SpoolError::CursorRegression);
                    }
                    true
                }
                CursorTransition::Duplicate => return Err(SpoolError::CursorRegression),
            }
        } else {
            false
        };
        let local_sequence = next_local_sequence(self.last_local_sequence)?;
        let rotate_for_epoch = epoch_changed
            && self
                .writer
                .as_ref()
                .ok_or(SpoolError::SegmentClosed)?
                .record_count()
                > 0;
        let closed_segment = if rotate_for_epoch || self.rotation_due(durable_at_micros)? {
            Some(self.rotate(durable_at_micros)?)
        } else {
            None
        };
        let active_path = self
            .writer
            .as_ref()
            .ok_or(SpoolError::SegmentClosed)?
            .segment_path()
            .to_owned();
        let durability_receipt = self
            .writer
            .as_mut()
            .ok_or(SpoolError::SegmentClosed)?
            .append(observation, durable_at_micros)?;
        let identity = LastRecordIdentity {
            cursor: observation.cursor().clone(),
        };
        self.extend_retained_segment(&active_path, observation, local_sequence);
        self.last_record_identity = Some(identity);
        self.last_local_sequence = Some(local_sequence);
        if let Some(receipt) = &durability_receipt {
            self.last_durable_cursor = Some(receipt.durable_cursor.clone());
        }
        Ok(SourceSpoolAppend {
            durability_receipt,
            closed_segment,
            local_sequence,
            disposition: SourceSpoolAppendDisposition::Appended,
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
    pub const fn last_local_sequence(&self) -> Option<LocalRecordSequence> {
        self.last_local_sequence
    }

    #[must_use]
    pub const fn cursor_policy(&self) -> CursorPolicy {
        self.config.cursor_policy
    }

    /// Number of segment-range entries used for retained duplicate lookup.
    ///
    /// Byte-offset mode keeps one compact entry per non-empty segment and
    /// scans only a matching segment on overlap. It does not retain one heap
    /// entry per observation.
    #[must_use]
    pub fn retained_segment_count(&self) -> usize {
        self.retained_segments.len()
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
        let header = SegmentHeader::new_with_cursor_policy(
            self.config.source_id.clone(),
            self.config.source_version.clone(),
            self.config.schema_version.clone(),
            next_sequence,
            closed_at_micros,
            self.config.producer_build_hash,
            self.config.cursor_policy,
        )?;
        let writer = SpoolWriter::create(&self.config.directory, header, self.config.durability)?;
        self.segment_paths.push(writer.segment_path().to_owned());
        self.writer = Some(writer);
        self.closed_segments.push(closed.clone());
        Ok(closed)
    }

    fn find_retained(
        &self,
        cursor: &SourceCursor,
    ) -> Result<Option<RetainedRecordIdentity>, SpoolError> {
        let Some(index) = self.retained_segments.iter().find(|index| {
            index.epoch == cursor.epoch()
                && (index.min_offset..=index.max_offset).contains(&cursor.offset())
        }) else {
            return Ok(None);
        };
        let reader = SpoolReader::open(&index.path)?;
        let mut records = reader.stream()?;
        let mut local_sequence = index.first_local_sequence;
        loop {
            match records.next_record()? {
                SpoolRead::Record(record) => {
                    if record.cursor() == cursor {
                        return Ok(Some(RetainedRecordIdentity {
                            observation_class: record.observation_class(),
                            parser_schema_version: record.parser_schema_version().to_owned(),
                            content_hash: record.content_hash(),
                            local_sequence,
                        }));
                    }
                    if record.cursor().epoch() == cursor.epoch()
                        && record.cursor().offset() > cursor.offset()
                    {
                        return Ok(None);
                    }
                    local_sequence = local_sequence
                        .checked_next()
                        .map_err(|_| SpoolError::SizeOverflow)?;
                }
                SpoolRead::EndOfFile => return Ok(None),
                SpoolRead::IncompleteTail { record_offset } => {
                    return Err(SpoolError::IncompleteTail { record_offset });
                }
            }
        }
    }

    fn extend_retained_segment(
        &mut self,
        active_path: &Path,
        observation: &SourceObservation,
        local_sequence: LocalRecordSequence,
    ) {
        if let Some(index) = self
            .retained_segments
            .last_mut()
            .filter(|index| index.path == active_path)
        {
            debug_assert_eq!(index.epoch, observation.cursor().epoch());
            debug_assert!(observation.cursor().offset() > index.max_offset);
            index.max_offset = observation.cursor().offset();
        } else {
            self.retained_segments.push(RetainedSegmentIndex {
                path: active_path.to_owned(),
                epoch: observation.cursor().epoch().to_owned(),
                min_offset: observation.cursor().offset(),
                max_offset: observation.cursor().offset(),
                first_local_sequence: local_sequence,
            });
        }
    }
}

fn validate_header(header: &SegmentHeader, config: &SourceSpoolConfig) -> Result<(), SpoolError> {
    if header.source_id() != &config.source_id
        || header.source_version() != config.source_version
        || header.schema_version() != config.schema_version
    {
        return Err(SpoolError::SourceMismatch);
    }
    if header.cursor_policy() != config.cursor_policy {
        return Err(SpoolError::CursorPolicyMismatch);
    }
    Ok(())
}

fn validate_successor(
    policy: CursorPolicy,
    cursor: &SourceCursor,
    previous: &SourceCursor,
    first_in_segment: bool,
) -> Result<(), SpoolError> {
    match cursor
        .validate_successor_of(previous)
        .map_err(|_| SpoolError::CursorRegression)?
    {
        CursorTransition::Advanced { .. } => Ok(()),
        CursorTransition::EpochChanged
            if policy == CursorPolicy::MonotonicByteOffset && first_in_segment =>
        {
            Ok(())
        }
        CursorTransition::Duplicate | CursorTransition::EpochChanged => {
            Err(SpoolError::CursorRegression)
        }
    }
}

fn next_local_sequence(
    previous: Option<LocalRecordSequence>,
) -> Result<LocalRecordSequence, SpoolError> {
    match previous {
        Some(previous) => previous
            .checked_next()
            .map_err(|_| SpoolError::SizeOverflow),
        None => LocalRecordSequence::try_new(1).map_err(|_| SpoolError::SizeOverflow),
    }
}

fn persisted_schema_identity(
    schema_version: String,
    policy: CursorPolicy,
) -> Result<String, SpoolError> {
    if policy == CursorPolicy::ContiguousNativeOffset {
        return Ok(schema_version);
    }
    Ok(format!(
        "{POLICY_SCHEMA_PREFIX}monotonic-byte-offset:{}",
        blake3::hash(schema_version.as_bytes()).to_hex()
    ))
}

fn duplicate_append(
    retained: &RetainedRecordIdentity,
    observation: &SourceObservation,
) -> Result<SourceSpoolAppend, SpoolError> {
    if retained.observation_class != observation.observation_class()
        || retained.parser_schema_version != observation.parser_schema_version()
        || retained.content_hash != observation.content_hash()
    {
        return Err(SpoolError::CursorConflict);
    }
    Ok(SourceSpoolAppend {
        durability_receipt: None,
        closed_segment: None,
        local_sequence: retained.local_sequence,
        disposition: SourceSpoolAppendDisposition::Duplicate,
    })
}

fn validate_timestamp(timestamp_micros: i64) -> Result<(), SpoolError> {
    if timestamp_micros < 0 {
        Err(SpoolError::InvalidTimestamp)
    } else {
        Ok(())
    }
}

fn validate_legacy_successor(
    cursor: &SourceCursor,
    previous: &SourceCursor,
) -> Result<(), SpoolError> {
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

fn validate_observation_policy(
    policy: CursorPolicy,
    observation_class: ObservationClass,
) -> Result<(), SpoolError> {
    if policy == CursorPolicy::MonotonicByteOffset
        && matches!(
            observation_class,
            ObservationClass::CommittedBlock | ObservationClass::HistoricalBlock
        )
    {
        Err(SpoolError::CursorPolicyMismatch)
    } else {
        Ok(())
    }
}
