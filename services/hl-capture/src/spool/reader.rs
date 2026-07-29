use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use super::header::SegmentHeaderV1;
use super::record::{SpoolRecord, decode_record};
use super::{MAX_RECORD_BYTES, SpoolError, io_error};

pub struct SpoolReader {
    path: PathBuf,
    header: SegmentHeaderV1,
    records_offset: u64,
}

impl SpoolReader {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, SpoolError> {
        let path = path.as_ref().to_owned();
        let mut file = OpenOptions::new()
            .read(true)
            .open(&path)
            .map_err(|source| io_error("opening a spool segment", source))?;
        let (header, records_offset) = SegmentHeaderV1::read_from(&mut file)?;
        Ok(Self {
            path,
            header,
            records_offset,
        })
    }

    #[must_use]
    pub const fn header(&self) -> &SegmentHeaderV1 {
        &self.header
    }

    pub fn read_all(&self) -> Result<Vec<SpoolRecord>, SpoolError> {
        let mut file = File::open(&self.path)
            .map_err(|source| io_error("reopening a spool segment", source))?;
        let (header, records_offset) = SegmentHeaderV1::read_from(&mut file)?;
        if header != self.header || records_offset != self.records_offset {
            return Err(SpoolError::InvalidHeader);
        }
        let scan = scan_records(&mut file, records_offset)?;
        if let Some(record_offset) = scan.incomplete_tail {
            return Err(SpoolError::IncompleteTail { record_offset });
        }
        Ok(scan.records)
    }
}

pub fn validate_segment_bytes(input: &[u8]) -> Result<u64, SpoolError> {
    let (_, mut cursor) = SegmentHeaderV1::read_from_slice(input)?;
    let mut records = 0_u64;
    while cursor < input.len() {
        let record_offset = u64::try_from(cursor).map_err(|_| SpoolError::SizeOverflow)?;
        let length_end = cursor.checked_add(4).ok_or(SpoolError::SizeOverflow)?;
        let Some(length_bytes) = input.get(cursor..length_end) else {
            return Err(SpoolError::IncompleteTail { record_offset });
        };
        let record_len = usize::try_from(u32::from_le_bytes(
            length_bytes
                .try_into()
                .map_err(|_| SpoolError::CorruptRecord { record_offset })?,
        ))
        .map_err(|_| SpoolError::CorruptRecord { record_offset })?;
        if !(4..=MAX_RECORD_BYTES).contains(&record_len) {
            return Err(SpoolError::CorruptRecord { record_offset });
        }
        let record_end = length_end
            .checked_add(record_len)
            .ok_or(SpoolError::CorruptRecord { record_offset })?;
        let Some(framed) = input.get(length_end..record_end) else {
            return Err(SpoolError::IncompleteTail { record_offset });
        };
        decode_record(framed, record_offset)?;
        records = records.checked_add(1).ok_or(SpoolError::SizeOverflow)?;
        cursor = record_end;
    }
    Ok(records)
}

pub(crate) struct ScanResult {
    pub records: Vec<SpoolRecord>,
    pub last_valid_offset: u64,
    pub last_record_offset: Option<u64>,
    pub incomplete_tail: Option<u64>,
}

pub(crate) fn scan_records(file: &mut File, records_offset: u64) -> Result<ScanResult, SpoolError> {
    file.seek(SeekFrom::Start(records_offset))
        .map_err(|source| io_error("seeking to spool records", source))?;
    let mut records = Vec::new();
    let mut last_valid_offset = records_offset;
    let mut last_record_offset = None;
    loop {
        let record_offset = file
            .stream_position()
            .map_err(|source| io_error("reading the spool position", source))?;
        let mut length_bytes = [0_u8; 4];
        let prefix_read = read_up_to(file, &mut length_bytes)?;
        if prefix_read == 0 {
            return Ok(ScanResult {
                records,
                last_valid_offset,
                last_record_offset,
                incomplete_tail: None,
            });
        }
        if prefix_read < length_bytes.len() {
            return Ok(ScanResult {
                records,
                last_valid_offset,
                last_record_offset,
                incomplete_tail: Some(record_offset),
            });
        }
        let record_len = usize::try_from(u32::from_le_bytes(length_bytes))
            .map_err(|_| SpoolError::CorruptRecord { record_offset })?;
        if !(4..=MAX_RECORD_BYTES).contains(&record_len) {
            return Err(SpoolError::CorruptRecord { record_offset });
        }
        let mut framed = vec![0_u8; record_len];
        let body_read = read_up_to(file, &mut framed)?;
        if body_read < record_len {
            return Ok(ScanResult {
                records,
                last_valid_offset,
                last_record_offset,
                incomplete_tail: Some(record_offset),
            });
        }
        records.push(decode_record(&framed, record_offset)?);
        last_record_offset = Some(record_offset);
        last_valid_offset = file
            .stream_position()
            .map_err(|source| io_error("reading the spool position", source))?;
    }
}

fn read_up_to(file: &mut File, output: &mut [u8]) -> Result<usize, SpoolError> {
    let mut read = 0;
    while read < output.len() {
        match file.read(&mut output[read..]) {
            Ok(0) => break,
            Ok(count) => read += count,
            Err(source) if source.kind() == std::io::ErrorKind::Interrupted => {}
            Err(source) => return Err(io_error("reading a spool record", source)),
        }
    }
    Ok(read)
}
