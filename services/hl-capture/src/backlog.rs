use std::path::PathBuf;

use domain_types::SourceId;
use hl_protocol::SourceObservation;

use crate::spool::{
    CloseReceipt, SpoolError, SpoolRead, SpoolReader, SpoolRecordStream, inspect_spool,
};

const MAX_IDENTITY_BYTES: usize = 256;

#[derive(Debug)]
pub struct SpoolBacklog {
    source_id: SourceId,
    source_version: String,
    max_payload_bytes: usize,
    expected_offset: u64,
    pending_offset: Option<u64>,
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

impl SpoolBacklog {
    pub fn open(
        directory: PathBuf,
        source_id: SourceId,
        source_version: impl Into<String>,
        expected_offset: u64,
        max_payload_bytes: usize,
    ) -> Result<Self, BacklogError> {
        let source_version = source_version.into();
        if source_version.is_empty()
            || source_version.trim() != source_version
            || source_version.len() > MAX_IDENTITY_BYTES
            || source_version.chars().any(char::is_control)
            || max_payload_bytes == 0
        {
            return Err(BacklogError::InvalidConfig);
        }
        let inspection = inspect_spool(&directory).map_err(BacklogError::Spool)?;
        let segments = inspection
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
            .collect::<Result<Vec<_>, BacklogError>>()?;
        Ok(Self {
            source_id,
            source_version,
            max_payload_bytes,
            expected_offset,
            pending_offset: None,
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
                    let observation = record
                        .into_observation(
                            self.source_id.clone(),
                            self.source_version.clone(),
                            self.max_payload_bytes,
                        )
                        .map_err(|_| BacklogError::Observation)?;
                    self.pending_offset = Some(offset);
                    return Ok(BacklogRead::Observation(Box::new(observation)));
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
            self.next_segment = self
                .next_segment
                .checked_add(1)
                .ok_or(BacklogError::OffsetOverflow)?;
            if segment.close_receipt.as_ref().is_some_and(|receipt| {
                receipt.manifest().max_cursor().offset() < self.expected_offset
            }) {
                continue;
            }
            if let Some(receipt) = &segment.close_receipt {
                receipt.verify_current().map_err(BacklogError::Spool)?;
            }
            let reader = SpoolReader::open(&segment.path).map_err(BacklogError::Spool)?;
            if reader.header().source_id() != &self.source_id
                || reader.header().source_version() != self.source_version
            {
                return Err(BacklogError::SourceMismatch);
            }
            let records = reader.stream().map_err(BacklogError::Spool)?;
            self.current = Some(CurrentSegment {
                records,
                close_receipt: segment.close_receipt,
            });
            return Ok(true);
        }
        Ok(false)
    }

    fn finish_closed_segment(&mut self) -> Result<(), BacklogError> {
        let current = self.current.take().ok_or(BacklogError::InvalidState)?;
        current
            .close_receipt
            .ok_or(BacklogError::InvalidState)?
            .verify_current()
            .map_err(BacklogError::Spool)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BacklogError {
    #[error("spool backlog configuration is invalid")]
    InvalidConfig,
    #[error("spool backlog evidence failed verification: {0}")]
    Spool(#[source] SpoolError),
    #[error("spool backlog segment does not match the configured source")]
    SourceMismatch,
    #[error("spool backlog record cannot be reconstructed")]
    Observation,
    #[error("spool backlog requires acknowledgement before reading another observation")]
    PendingAcknowledgement,
    #[error("spool backlog acknowledgement does not match the pending observation")]
    AcknowledgementMismatch,
    #[error("spool backlog cursor overflowed")]
    OffsetOverflow,
    #[error("spool backlog encountered a committed source gap")]
    Gap { expected: u64, observed: u64 },
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
            Self::Observation => "capture_backlog.observation",
            Self::PendingAcknowledgement => "capture_backlog.pending_acknowledgement",
            Self::AcknowledgementMismatch => "capture_backlog.acknowledgement_mismatch",
            Self::OffsetOverflow => "capture_backlog.offset_overflow",
            Self::Gap { .. } => "capture_backlog.gap",
            Self::InvalidState => "capture_backlog.invalid_state",
        }
    }
}
