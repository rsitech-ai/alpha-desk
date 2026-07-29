use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use hl_protocol::{CursorTransition, SourceCursor, SourceObservation};

use super::header::SegmentHeaderV1;
use super::manifest::{CloseReceipt, ClosedSegmentManifestV1, ManifestFields, publish_manifest};
use super::reader::scan_records;
use super::record::encode_record;
use super::recovery::{RecoveryReport, recover_open_segment};
use super::{SpoolError, io_error};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DurabilityPolicy {
    FsyncEveryRecord,
    FsyncEvery {
        max_records: u32,
        max_delay: Duration,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppendReceipt {
    pub segment_sequence: u64,
    pub record_offset: u64,
    pub durable_cursor: SourceCursor,
    pub durable_at_micros: i64,
}

#[derive(Debug)]
pub struct SpoolWriter {
    file: File,
    segment_path: PathBuf,
    header: SegmentHeaderV1,
    durability: DurabilityPolicy,
    pending_records: u32,
    pending_since: Option<Instant>,
    last_cursor: Option<SourceCursor>,
    min_cursor: Option<SourceCursor>,
    last_record_offset: Option<u64>,
    record_count: u64,
    closed: bool,
}

impl SpoolWriter {
    pub fn create(
        directory: impl AsRef<Path>,
        header: SegmentHeaderV1,
        durability: DurabilityPolicy,
    ) -> Result<Self, SpoolError> {
        validate_durability(durability)?;
        fs::create_dir_all(directory.as_ref())
            .map_err(|source| io_error("creating the spool directory", source))?;
        let segment_path = directory
            .as_ref()
            .join(segment_name(header.segment_sequence()));
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&segment_path)
            .map_err(|source| {
                if source.kind() == std::io::ErrorKind::AlreadyExists {
                    SpoolError::SegmentAlreadyExists
                } else {
                    io_error("creating a spool segment", source)
                }
            })?;
        let encoded = header.encode()?;
        file.write_all(&encoded)
            .map_err(|source| io_error("writing a spool segment header", source))?;
        file.sync_all()
            .map_err(|source| io_error("syncing a spool segment header", source))?;
        sync_directory(directory.as_ref())?;
        Ok(Self {
            file,
            segment_path,
            header,
            durability,
            pending_records: 0,
            pending_since: None,
            last_cursor: None,
            min_cursor: None,
            last_record_offset: None,
            record_count: 0,
            closed: false,
        })
    }

    pub fn open_recovered(
        segment_path: impl AsRef<Path>,
        durability: DurabilityPolicy,
    ) -> Result<(Self, RecoveryReport), SpoolError> {
        validate_durability(durability)?;
        let segment_path = segment_path.as_ref().to_owned();
        let report = recover_open_segment(&segment_path)?;
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&segment_path)
            .map_err(|source| io_error("opening a recovered spool segment", source))?;
        let (header, records_offset) = SegmentHeaderV1::read_from(&mut file)?;
        if segment_path.file_name().and_then(std::ffi::OsStr::to_str)
            != Some(segment_name(header.segment_sequence()).as_str())
        {
            return Err(SpoolError::InvalidHeader);
        }
        let scan = scan_records(&mut file, records_offset)?;
        if let Some(record_offset) = scan.incomplete_tail {
            return Err(SpoolError::IncompleteTail { record_offset });
        }
        let mut previous = None;
        for record in &scan.records {
            if let Some(previous) = &previous {
                validate_cursor_successor(record.cursor(), previous)?;
            }
            previous = Some(record.cursor().clone());
        }
        let record_count =
            u64::try_from(scan.records.len()).map_err(|_| SpoolError::SizeOverflow)?;
        if record_count != report.valid_records {
            return Err(SpoolError::SizeOverflow);
        }
        let min_cursor = scan.records.first().map(|record| record.cursor().clone());
        let last_cursor = scan.records.last().map(|record| record.cursor().clone());
        file.seek(SeekFrom::End(0))
            .map_err(|source| io_error("seeking to the recovered spool tail", source))?;
        Ok((
            Self {
                file,
                segment_path,
                header,
                durability,
                pending_records: 0,
                pending_since: None,
                last_cursor,
                min_cursor,
                last_record_offset: scan.last_record_offset,
                record_count,
                closed: false,
            },
            report,
        ))
    }

    #[must_use]
    pub fn segment_path(&self) -> &Path {
        &self.segment_path
    }

    pub fn append(
        &mut self,
        observation: &SourceObservation,
        durable_at_micros: i64,
    ) -> Result<Option<AppendReceipt>, SpoolError> {
        if self.closed {
            return Err(SpoolError::SegmentClosed);
        }
        validate_timestamp(durable_at_micros)?;
        if observation.source_id() != self.header.source_id()
            || observation.source_version() != self.header.source_version()
        {
            return Err(SpoolError::SourceMismatch);
        }
        if let Some(previous) = &self.last_cursor {
            validate_cursor_successor(observation.cursor(), previous)?;
        }
        let encoded = encode_record(observation)?;
        let record_offset = self
            .file
            .seek(SeekFrom::End(0))
            .map_err(|source| io_error("seeking to the spool tail", source))?;
        self.file
            .write_all(&encoded)
            .map_err(|source| io_error("appending a spool record", source))?;
        self.pending_records = self
            .pending_records
            .checked_add(1)
            .ok_or(SpoolError::SizeOverflow)?;
        let now = Instant::now();
        if self.pending_since.is_none() {
            self.pending_since = Some(now);
        }
        self.last_cursor = Some(observation.cursor().clone());
        if self.min_cursor.is_none() {
            self.min_cursor = Some(observation.cursor().clone());
        }
        self.last_record_offset = Some(record_offset);
        self.record_count = self
            .record_count
            .checked_add(1)
            .ok_or(SpoolError::SizeOverflow)?;

        let should_sync = match self.durability {
            DurabilityPolicy::FsyncEveryRecord => true,
            DurabilityPolicy::FsyncEvery {
                max_records,
                max_delay,
            } => {
                self.pending_records >= max_records
                    || self
                        .pending_since
                        .and_then(|started| started.checked_add(max_delay))
                        .is_some_and(|deadline| now >= deadline)
            }
        };
        if should_sync {
            self.sync_pending(durable_at_micros)
        } else {
            Ok(None)
        }
    }

    pub fn flush(&mut self, durable_at_micros: i64) -> Result<Option<AppendReceipt>, SpoolError> {
        if self.closed {
            return Err(SpoolError::SegmentClosed);
        }
        self.sync_pending(durable_at_micros)
    }

    #[must_use]
    pub fn next_sync_deadline(&self) -> Option<Instant> {
        let DurabilityPolicy::FsyncEvery { max_delay, .. } = self.durability else {
            return None;
        };
        self.pending_since
            .and_then(|started| started.checked_add(max_delay))
    }

    pub fn flush_due(
        &mut self,
        now: Instant,
        durable_at_micros: i64,
    ) -> Result<Option<AppendReceipt>, SpoolError> {
        if self.closed {
            return Err(SpoolError::SegmentClosed);
        }
        let Some(deadline) = self.next_sync_deadline() else {
            return Ok(None);
        };
        if now < deadline {
            return Ok(None);
        }
        self.sync_pending(durable_at_micros)
    }

    pub fn close(
        mut self,
        closed_at_micros: i64,
        previous_manifest_blake3: Option<[u8; 32]>,
    ) -> Result<CloseReceipt, SpoolError> {
        validate_timestamp(closed_at_micros)?;
        if self.record_count == 0 {
            return Err(SpoolError::EmptySegment);
        }
        self.sync_pending(closed_at_micros)?;
        self.file
            .sync_all()
            .map_err(|source| io_error("syncing a closed spool segment", source))?;
        let file_size_bytes = self
            .file
            .metadata()
            .map_err(|source| io_error("reading closed segment metadata", source))?
            .len();
        let segment_blake3 = hash_file(&mut self.file)?;
        let segment_file = self
            .segment_path
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .ok_or(SpoolError::InvalidManifest)?
            .to_owned();
        let manifest = ClosedSegmentManifestV1::new(ManifestFields {
            segment_sequence: self.header.segment_sequence(),
            segment_file,
            source_id: self.header.source_id().as_str().to_owned(),
            source_version: self.header.source_version().to_owned(),
            spool_schema_version: self.header.schema_version().to_owned(),
            producer_build_hash: self.header.producer_build_hash(),
            file_size_bytes,
            record_count: self.record_count,
            min_cursor: self.min_cursor.clone().ok_or(SpoolError::EmptySegment)?,
            max_cursor: self.last_cursor.clone().ok_or(SpoolError::EmptySegment)?,
            segment_blake3,
            previous_manifest_blake3,
            closed_at_micros,
        })?;
        let receipt = publish_manifest(&self.segment_path, manifest)?;
        self.closed = true;
        Ok(receipt)
    }

    fn sync_pending(
        &mut self,
        durable_at_micros: i64,
    ) -> Result<Option<AppendReceipt>, SpoolError> {
        if self.pending_records == 0 {
            return Ok(None);
        }
        validate_timestamp(durable_at_micros)?;
        self.file
            .sync_data()
            .map_err(|source| io_error("syncing appended spool records", source))?;
        self.pending_records = 0;
        self.pending_since = None;
        Ok(Some(AppendReceipt {
            segment_sequence: self.header.segment_sequence(),
            record_offset: self.last_record_offset.ok_or(SpoolError::SizeOverflow)?,
            durable_cursor: self.last_cursor.clone().ok_or(SpoolError::SizeOverflow)?,
            durable_at_micros,
        }))
    }
}

fn validate_cursor_successor(
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

fn hash_file(file: &mut File) -> Result<[u8; 32], SpoolError> {
    file.seek(SeekFrom::Start(0))
        .map_err(|source| io_error("seeking to hash a closed segment", source))?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|source| io_error("hashing a closed segment", source))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(*hasher.finalize().as_bytes())
}

fn sync_directory(path: &Path) -> Result<(), SpoolError> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|source| io_error("syncing the spool directory", source))
}

pub(crate) fn segment_name(sequence: u64) -> String {
    format!("segment-{sequence:010}.hlsp")
}

fn validate_durability(policy: DurabilityPolicy) -> Result<(), SpoolError> {
    match policy {
        DurabilityPolicy::FsyncEveryRecord => Ok(()),
        DurabilityPolicy::FsyncEvery {
            max_records,
            max_delay,
        } if max_records > 0
            && !max_delay.is_zero()
            && Instant::now().checked_add(max_delay).is_some() =>
        {
            Ok(())
        }
        DurabilityPolicy::FsyncEvery { .. } => Err(SpoolError::InvalidDurabilityPolicy),
    }
}

fn validate_timestamp(timestamp_micros: i64) -> Result<(), SpoolError> {
    if timestamp_micros < 0 {
        Err(SpoolError::InvalidTimestamp)
    } else {
        Ok(())
    }
}
