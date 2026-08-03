use std::path::PathBuf;

use domain_types::SourceId;
use hl_protocol::SourceObservation;
use storage_ports::{CursorPolicy, LocalRecordSequence};

use crate::spool::{
    CloseReceipt, SpoolError, SpoolRead, SpoolReader, SpoolRecord, SpoolRecordStream, inspect_spool,
};

const MAX_IDENTITY_BYTES: usize = 256;

#[derive(Debug)]
pub struct SpoolBacklog {
    source_id: SourceId,
    source_version: String,
    max_payload_bytes: usize,
    expected_offset: u64,
    pending_offset: Option<u64>,
    retry_record: Option<SpoolRecord>,
    segments: Vec<SegmentEvidence>,
    next_segment: usize,
    current: Option<CurrentSegment>,
}

#[derive(Debug, Clone)]
struct SegmentEvidence {
    path: PathBuf,
    close_receipt: Option<CloseReceipt>,
}

#[derive(Debug)]
struct CurrentSegment {
    records: SpoolRecordStream,
    close_receipt: Option<CloseReceipt>,
}

#[derive(Debug, Clone)]
pub enum BacklogRead {
    Observation(Box<SourceObservation>),
    CaughtUp { next_expected_offset: u64 },
}

#[derive(Debug)]
pub struct SequencedBacklogObservation {
    observation: Box<SourceObservation>,
    local_sequence: LocalRecordSequence,
}

impl SequencedBacklogObservation {
    #[must_use]
    pub const fn observation(&self) -> &SourceObservation {
        &self.observation
    }

    #[must_use]
    pub const fn local_sequence(&self) -> LocalRecordSequence {
        self.local_sequence
    }

    #[must_use]
    pub fn into_observation(self) -> Box<SourceObservation> {
        self.observation
    }
}

#[derive(Debug)]
pub enum SequencedBacklogRead {
    Observation(SequencedBacklogObservation),
    CaughtUp {
        next_expected_sequence: LocalRecordSequence,
    },
}

#[derive(Debug)]
pub struct ByteOffsetSpoolBacklog {
    source_id: SourceId,
    source_version: String,
    max_payload_bytes: usize,
    next_expected_sequence: LocalRecordSequence,
    next_record_sequence: LocalRecordSequence,
    pending_sequence: Option<LocalRecordSequence>,
    retry_record: Option<SpoolRecord>,
    segments: Vec<SegmentEvidence>,
    next_segment: usize,
    current: Option<CurrentSegment>,
}

impl SpoolBacklog {
    pub fn open(
        directory: PathBuf,
        source_id: SourceId,
        source_version: impl Into<String>,
        expected_offset: u64,
        max_payload_bytes: usize,
    ) -> Result<Self, BacklogError> {
        let source_version = source_version.into();
        validate_config(&source_version, max_payload_bytes)?;
        let inspection = inspect_spool(&directory).map_err(BacklogError::Spool)?;
        let segments = segment_evidence(&inspection)?;
        Ok(Self {
            source_id,
            source_version,
            max_payload_bytes,
            expected_offset,
            pending_offset: None,
            retry_record: None,
            segments,
            next_segment: 0,
            current: None,
        })
    }

    #[must_use]
    pub const fn next_expected_offset(&self) -> u64 {
        self.expected_offset
    }

    pub fn next_observation(&mut self) -> Result<BacklogRead, BacklogError> {
        if self.pending_offset.is_some() {
            return Err(BacklogError::PendingAcknowledgement);
        }
        loop {
            if let Some(record) = self.retry_record.take() {
                return self.reconstruct_record(record);
            }
            if self.current.is_none() && !self.open_next_segment()? {
                return Ok(BacklogRead::CaughtUp {
                    next_expected_offset: self.expected_offset,
                });
            }
            let current = self.current.as_mut().ok_or(BacklogError::InvalidState)?;
            match current.records.next_record().map_err(BacklogError::Spool)? {
                SpoolRead::Record(record) => {
                    let offset = record.cursor().offset();
                    if offset < self.expected_offset {
                        continue;
                    }
                    if offset > self.expected_offset {
                        return Err(BacklogError::Gap {
                            expected: self.expected_offset,
                            observed: offset,
                        });
                    }
                    return self.reconstruct_record(record);
                }
                SpoolRead::EndOfFile => {
                    if current.close_receipt.is_none() {
                        return Ok(BacklogRead::CaughtUp {
                            next_expected_offset: self.expected_offset,
                        });
                    }
                    self.finish_closed_segment()?;
                }
                SpoolRead::IncompleteTail { record_offset } => {
                    if current.close_receipt.is_some() {
                        return Err(BacklogError::Spool(SpoolError::IncompleteTail {
                            record_offset,
                        }));
                    }
                    return Ok(BacklogRead::CaughtUp {
                        next_expected_offset: self.expected_offset,
                    });
                }
            }
        }
    }

    pub fn acknowledge(&mut self, offset: u64) -> Result<(), BacklogError> {
        if self.pending_offset != Some(offset) || offset != self.expected_offset {
            return Err(BacklogError::AcknowledgementMismatch);
        }
        self.expected_offset = self
            .expected_offset
            .checked_add(1)
            .ok_or(BacklogError::OffsetOverflow)?;
        self.pending_offset = None;
        Ok(())
    }

    fn open_next_segment(&mut self) -> Result<bool, BacklogError> {
        while let Some(segment) = self.segments.get(self.next_segment).cloned() {
            let next_segment = self
                .next_segment
                .checked_add(1)
                .ok_or(BacklogError::OffsetOverflow)?;
            let reader = SpoolReader::open(&segment.path).map_err(BacklogError::Spool)?;
            validate_reader(
                &reader,
                &self.source_id,
                &self.source_version,
                CursorPolicy::ContiguousNativeOffset,
            )?;
            if segment.close_receipt.as_ref().is_some_and(|receipt| {
                receipt.manifest().max_cursor().offset() < self.expected_offset
            }) {
                self.next_segment = next_segment;
                continue;
            }
            if let Some(receipt) = &segment.close_receipt {
                receipt.verify_current().map_err(BacklogError::Spool)?;
            }
            let records = reader.stream().map_err(BacklogError::Spool)?;
            self.current = Some(CurrentSegment {
                records,
                close_receipt: segment.close_receipt,
            });
            self.next_segment = next_segment;
            return Ok(true);
        }
        Ok(false)
    }

    fn finish_closed_segment(&mut self) -> Result<(), BacklogError> {
        self.current
            .as_ref()
            .ok_or(BacklogError::InvalidState)?
            .close_receipt
            .as_ref()
            .ok_or(BacklogError::InvalidState)?
            .verify_current()
            .map_err(BacklogError::Spool)?;
        self.current = None;
        Ok(())
    }

    fn reconstruct_record(&mut self, record: SpoolRecord) -> Result<BacklogRead, BacklogError> {
        let offset = record.cursor().offset();
        match record.clone().into_observation(
            self.source_id.clone(),
            self.source_version.clone(),
            self.max_payload_bytes,
        ) {
            Ok(observation) => {
                self.pending_offset = Some(offset);
                Ok(BacklogRead::Observation(Box::new(observation)))
            }
            Err(_) => {
                self.retry_record = Some(record);
                Err(BacklogError::Observation)
            }
        }
    }
}

impl ByteOffsetSpoolBacklog {
    pub fn open(
        directory: PathBuf,
        source_id: SourceId,
        source_version: impl Into<String>,
        next_expected_sequence: LocalRecordSequence,
        max_payload_bytes: usize,
    ) -> Result<Self, BacklogError> {
        let source_version = source_version.into();
        validate_config(&source_version, max_payload_bytes)?;
        let inspection = inspect_spool(&directory).map_err(BacklogError::Spool)?;
        for path in inspection.segment_paths() {
            let reader = SpoolReader::open(path).map_err(BacklogError::Spool)?;
            validate_reader(
                &reader,
                &source_id,
                &source_version,
                CursorPolicy::MonotonicByteOffset,
            )?;
        }
        let segments = segment_evidence(&inspection)?;
        Ok(Self {
            source_id,
            source_version,
            max_payload_bytes,
            next_expected_sequence,
            next_record_sequence: LocalRecordSequence::try_new(1)
                .map_err(|_| BacklogError::SequenceOverflow)?,
            pending_sequence: None,
            retry_record: None,
            segments,
            next_segment: 0,
            current: None,
        })
    }

    #[must_use]
    pub const fn next_expected_sequence(&self) -> LocalRecordSequence {
        self.next_expected_sequence
    }

    pub fn next_observation(&mut self) -> Result<SequencedBacklogRead, BacklogError> {
        if self.pending_sequence.is_some() {
            return Err(BacklogError::PendingAcknowledgement);
        }
        loop {
            if let Some(record) = self.retry_record.take() {
                return self.reconstruct_record(record);
            }
            if self.current.is_none() && !self.open_next_segment()? {
                if self.next_record_sequence != self.next_expected_sequence {
                    return Err(BacklogError::SequenceGap {
                        expected: self.next_expected_sequence.get(),
                        observed: self.next_record_sequence.get(),
                    });
                }
                return Ok(SequencedBacklogRead::CaughtUp {
                    next_expected_sequence: self.next_expected_sequence,
                });
            }
            let current = self.current.as_mut().ok_or(BacklogError::InvalidState)?;
            match current.records.next_record().map_err(BacklogError::Spool)? {
                SpoolRead::Record(record) => {
                    let local_sequence = self.next_record_sequence;
                    if local_sequence < self.next_expected_sequence {
                        self.next_record_sequence = next_sequence(local_sequence)?;
                        continue;
                    }
                    if local_sequence > self.next_expected_sequence {
                        return Err(BacklogError::SequenceGap {
                            expected: self.next_expected_sequence.get(),
                            observed: local_sequence.get(),
                        });
                    }
                    return self.reconstruct_record(record);
                }
                SpoolRead::EndOfFile => {
                    if current.close_receipt.is_none() {
                        if self.next_record_sequence != self.next_expected_sequence {
                            return Err(BacklogError::SequenceGap {
                                expected: self.next_expected_sequence.get(),
                                observed: self.next_record_sequence.get(),
                            });
                        }
                        return Ok(SequencedBacklogRead::CaughtUp {
                            next_expected_sequence: self.next_expected_sequence,
                        });
                    }
                    self.finish_closed_segment()?;
                }
                SpoolRead::IncompleteTail { record_offset } => {
                    if current.close_receipt.is_some() {
                        return Err(BacklogError::Spool(SpoolError::IncompleteTail {
                            record_offset,
                        }));
                    }
                    if self.next_record_sequence != self.next_expected_sequence {
                        return Err(BacklogError::SequenceGap {
                            expected: self.next_expected_sequence.get(),
                            observed: self.next_record_sequence.get(),
                        });
                    }
                    return Ok(SequencedBacklogRead::CaughtUp {
                        next_expected_sequence: self.next_expected_sequence,
                    });
                }
            }
        }
    }

    pub fn acknowledge(&mut self, sequence: LocalRecordSequence) -> Result<(), BacklogError> {
        if self.pending_sequence != Some(sequence) || sequence != self.next_expected_sequence {
            return Err(BacklogError::AcknowledgementMismatch);
        }
        let next = next_sequence(sequence)?;
        self.next_expected_sequence = next;
        self.next_record_sequence = next;
        self.pending_sequence = None;
        Ok(())
    }

    fn open_next_segment(&mut self) -> Result<bool, BacklogError> {
        while let Some(segment) = self.segments.get(self.next_segment).cloned() {
            let next_segment = self
                .next_segment
                .checked_add(1)
                .ok_or(BacklogError::SequenceOverflow)?;
            if let Some(receipt) = &segment.close_receipt {
                receipt.verify_current().map_err(BacklogError::Spool)?;
                let record_count = receipt.manifest().record_count();
                let last = sequence_after_count(self.next_record_sequence, record_count)?;
                if last < self.next_expected_sequence {
                    self.next_record_sequence = next_sequence(last)?;
                    self.next_segment = next_segment;
                    continue;
                }
            }
            let reader = SpoolReader::open(&segment.path).map_err(BacklogError::Spool)?;
            validate_reader(
                &reader,
                &self.source_id,
                &self.source_version,
                CursorPolicy::MonotonicByteOffset,
            )?;
            let records = reader.stream().map_err(BacklogError::Spool)?;
            self.current = Some(CurrentSegment {
                records,
                close_receipt: segment.close_receipt,
            });
            self.next_segment = next_segment;
            return Ok(true);
        }
        Ok(false)
    }

    fn finish_closed_segment(&mut self) -> Result<(), BacklogError> {
        self.current
            .as_ref()
            .ok_or(BacklogError::InvalidState)?
            .close_receipt
            .as_ref()
            .ok_or(BacklogError::InvalidState)?
            .verify_current()
            .map_err(BacklogError::Spool)?;
        self.current = None;
        Ok(())
    }

    fn reconstruct_record(
        &mut self,
        record: SpoolRecord,
    ) -> Result<SequencedBacklogRead, BacklogError> {
        let local_sequence = self.next_record_sequence;
        match record.clone().into_observation(
            self.source_id.clone(),
            self.source_version.clone(),
            self.max_payload_bytes,
        ) {
            Ok(observation) => {
                self.pending_sequence = Some(local_sequence);
                Ok(SequencedBacklogRead::Observation(
                    SequencedBacklogObservation {
                        observation: Box::new(observation),
                        local_sequence,
                    },
                ))
            }
            Err(_) => {
                self.retry_record = Some(record);
                Err(BacklogError::Observation)
            }
        }
    }
}

fn validate_config(source_version: &str, max_payload_bytes: usize) -> Result<(), BacklogError> {
    if source_version.is_empty()
        || source_version.trim() != source_version
        || source_version.len() > MAX_IDENTITY_BYTES
        || source_version.chars().any(char::is_control)
        || max_payload_bytes == 0
    {
        Err(BacklogError::InvalidConfig)
    } else {
        Ok(())
    }
}

fn segment_evidence(
    inspection: &crate::spool::SpoolInspection,
) -> Result<Vec<SegmentEvidence>, BacklogError> {
    inspection
        .segment_paths()
        .iter()
        .map(|path| {
            let close_receipt = if Some(path.as_path()) == inspection.open_segment_path() {
                None
            } else {
                Some(CloseReceipt::load(path).map_err(BacklogError::Spool)?)
            };
            Ok(SegmentEvidence {
                path: path.clone(),
                close_receipt,
            })
        })
        .collect()
}

fn validate_reader(
    reader: &SpoolReader,
    source_id: &SourceId,
    source_version: &str,
    cursor_policy: CursorPolicy,
) -> Result<(), BacklogError> {
    if reader.header().source_id() != source_id
        || reader.header().source_version() != source_version
    {
        return Err(BacklogError::SourceMismatch);
    }
    if reader.header().cursor_policy() != cursor_policy {
        return Err(BacklogError::CursorPolicyMismatch);
    }
    Ok(())
}

fn sequence_after_count(
    first: LocalRecordSequence,
    count: u64,
) -> Result<LocalRecordSequence, BacklogError> {
    let advance = count.checked_sub(1).ok_or(BacklogError::InvalidState)?;
    first
        .checked_advance_by(advance)
        .map_err(|_| BacklogError::SequenceOverflow)
}

fn next_sequence(sequence: LocalRecordSequence) -> Result<LocalRecordSequence, BacklogError> {
    sequence
        .checked_next()
        .map_err(|_| BacklogError::SequenceOverflow)
}

#[derive(Debug, thiserror::Error)]
pub enum BacklogError {
    #[error("spool backlog configuration is invalid")]
    InvalidConfig,
    #[error("spool backlog evidence failed verification: {0}")]
    Spool(#[source] SpoolError),
    #[error("spool backlog segment does not match the configured source")]
    SourceMismatch,
    #[error("spool backlog cursor policy does not match the segment format")]
    CursorPolicyMismatch,
    #[error("spool backlog record cannot be reconstructed")]
    Observation,
    #[error("spool backlog requires acknowledgement before reading another observation")]
    PendingAcknowledgement,
    #[error("spool backlog acknowledgement does not match the pending observation")]
    AcknowledgementMismatch,
    #[error("spool backlog cursor overflowed")]
    OffsetOverflow,
    #[error("spool backlog local record sequence overflowed")]
    SequenceOverflow,
    #[error("spool backlog encountered a committed source gap")]
    Gap { expected: u64, observed: u64 },
    #[error("spool backlog encountered a physical record sequence gap")]
    SequenceGap { expected: u64, observed: u64 },
    #[error("spool backlog internal state is invalid")]
    InvalidState,
}

impl BacklogError {
    #[must_use]
    pub const fn reason_code(&self) -> &'static str {
        match self {
            Self::InvalidConfig => "capture_backlog.invalid_config",
            Self::Spool(error) => error.reason_code(),
            Self::SourceMismatch => "capture_backlog.source_mismatch",
            Self::CursorPolicyMismatch => "capture_backlog.cursor_policy_mismatch",
            Self::Observation => "capture_backlog.observation",
            Self::PendingAcknowledgement => "capture_backlog.pending_acknowledgement",
            Self::AcknowledgementMismatch => "capture_backlog.acknowledgement_mismatch",
            Self::OffsetOverflow => "capture_backlog.offset_overflow",
            Self::SequenceOverflow => "capture_backlog.sequence_overflow",
            Self::Gap { .. } => "capture_backlog.gap",
            Self::SequenceGap { .. } => "capture_backlog.sequence_gap",
            Self::InvalidState => "capture_backlog.invalid_state",
        }
    }
}
