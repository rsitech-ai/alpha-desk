use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

use domain_types::SourceId;

use super::{MAX_HEADER_BYTES, MAX_IDENTITY_BYTES, SpoolError, io_error};

const MAGIC: [u8; 8] = *b"HLSPV001";
const FIXED_HEADER_BYTES: usize = 8 + 4 + 2 + 2 + 2 + 8 + 8 + 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentHeaderV1 {
    source_id: SourceId,
    source_version: String,
    schema_version: String,
    segment_sequence: u64,
    created_at_micros: i64,
    producer_build_hash: [u8; 32],
}

impl SegmentHeaderV1 {
    pub fn new(
        source_id: SourceId,
        source_version: impl Into<String>,
        schema_version: impl Into<String>,
        segment_sequence: u64,
        created_at_micros: i64,
        producer_build_hash: [u8; 32],
    ) -> Result<Self, SpoolError> {
        let source_version = source_version.into();
        let schema_version = schema_version.into();
        if !valid_identity(source_id.as_str())
            || !valid_identity(&source_version)
            || !valid_identity(&schema_version)
            || created_at_micros < 0
        {
            return Err(SpoolError::InvalidHeader);
        }
        Ok(Self {
            source_id,
            source_version,
            schema_version,
            segment_sequence,
            created_at_micros,
            producer_build_hash,
        })
    }

    #[must_use]
    pub const fn source_id(&self) -> &SourceId {
        &self.source_id
    }

    #[must_use]
    pub fn source_version(&self) -> &str {
        &self.source_version
    }

    #[must_use]
    pub fn schema_version(&self) -> &str {
        &self.schema_version
    }

    #[must_use]
    pub const fn segment_sequence(&self) -> u64 {
        self.segment_sequence
    }

    #[must_use]
    pub const fn created_at_micros(&self) -> i64 {
        self.created_at_micros
    }

    #[must_use]
    pub const fn producer_build_hash(&self) -> [u8; 32] {
        self.producer_build_hash
    }

    pub(crate) fn encode(&self) -> Result<Vec<u8>, SpoolError> {
        let header_len = FIXED_HEADER_BYTES
            .checked_add(self.source_id.as_str().len())
            .and_then(|length| length.checked_add(self.source_version.len()))
            .and_then(|length| length.checked_add(self.schema_version.len()))
            .ok_or(SpoolError::SizeOverflow)?;
        if header_len > MAX_HEADER_BYTES {
            return Err(SpoolError::InvalidHeader);
        }
        let mut encoded = Vec::with_capacity(header_len);
        encoded.extend_from_slice(&MAGIC);
        encoded.extend_from_slice(
            &u32::try_from(header_len)
                .map_err(|_| SpoolError::SizeOverflow)?
                .to_le_bytes(),
        );
        push_text(&mut encoded, self.source_id.as_str())?;
        push_text(&mut encoded, &self.source_version)?;
        push_text(&mut encoded, &self.schema_version)?;
        encoded.extend_from_slice(&self.segment_sequence.to_le_bytes());
        encoded.extend_from_slice(&self.created_at_micros.to_le_bytes());
        encoded.extend_from_slice(&self.producer_build_hash);
        Ok(encoded)
    }

    pub(crate) fn read_from(file: &mut File) -> Result<(Self, u64), SpoolError> {
        file.seek(SeekFrom::Start(0))
            .map_err(|source| io_error("seeking to the segment header", source))?;
        let mut prefix = [0_u8; 12];
        read_exact_header(file, &mut prefix)?;
        if prefix[..8] != MAGIC {
            return Err(SpoolError::InvalidHeader);
        }
        let header_len = usize::try_from(u32::from_le_bytes(
            prefix[8..12]
                .try_into()
                .map_err(|_| SpoolError::InvalidHeader)?,
        ))
        .map_err(|_| SpoolError::SizeOverflow)?;
        if !(FIXED_HEADER_BYTES..=MAX_HEADER_BYTES).contains(&header_len) {
            return Err(SpoolError::InvalidHeader);
        }
        let mut encoded = vec![0_u8; header_len];
        encoded[..12].copy_from_slice(&prefix);
        read_exact_header(file, &mut encoded[12..])?;
        let header = Self::decode(&encoded)?;
        Ok((
            header,
            u64::try_from(header_len).map_err(|_| SpoolError::SizeOverflow)?,
        ))
    }

    pub(crate) fn read_from_slice(input: &[u8]) -> Result<(Self, usize), SpoolError> {
        let prefix = input.get(..12).ok_or(SpoolError::IncompleteHeader)?;
        if prefix[..8] != MAGIC {
            return Err(SpoolError::InvalidHeader);
        }
        let header_len = usize::try_from(u32::from_le_bytes(
            prefix[8..12]
                .try_into()
                .map_err(|_| SpoolError::InvalidHeader)?,
        ))
        .map_err(|_| SpoolError::SizeOverflow)?;
        if !(FIXED_HEADER_BYTES..=MAX_HEADER_BYTES).contains(&header_len) {
            return Err(SpoolError::InvalidHeader);
        }
        let encoded = input
            .get(..header_len)
            .ok_or(SpoolError::IncompleteHeader)?;
        Ok((Self::decode(encoded)?, header_len))
    }

    fn decode(encoded: &[u8]) -> Result<Self, SpoolError> {
        let mut cursor = 12;
        let source_id = read_text(encoded, &mut cursor)?;
        let source_version = read_text(encoded, &mut cursor)?;
        let schema_version = read_text(encoded, &mut cursor)?;
        let segment_sequence = read_u64(encoded, &mut cursor)?;
        let created_at_micros = read_i64(encoded, &mut cursor)?;
        let producer_build_hash = take(encoded, &mut cursor, 32)?
            .try_into()
            .map_err(|_| SpoolError::InvalidHeader)?;
        if cursor != encoded.len() {
            return Err(SpoolError::InvalidHeader);
        }
        let source_id = SourceId::new(source_id).map_err(|_| SpoolError::InvalidHeader)?;
        Self::new(
            source_id,
            source_version,
            schema_version,
            segment_sequence,
            created_at_micros,
            producer_build_hash,
        )
    }
}

fn push_text(output: &mut Vec<u8>, value: &str) -> Result<(), SpoolError> {
    let length = u16::try_from(value.len()).map_err(|_| SpoolError::InvalidHeader)?;
    output.extend_from_slice(&length.to_le_bytes());
    output.extend_from_slice(value.as_bytes());
    Ok(())
}

fn read_text(input: &[u8], cursor: &mut usize) -> Result<String, SpoolError> {
    let length = usize::from(read_u16(input, cursor)?);
    if !(1..=MAX_IDENTITY_BYTES).contains(&length) {
        return Err(SpoolError::InvalidHeader);
    }
    let bytes = take(input, cursor, length)?;
    let value = std::str::from_utf8(bytes).map_err(|_| SpoolError::InvalidHeader)?;
    if !valid_identity(value) {
        return Err(SpoolError::InvalidHeader);
    }
    Ok(value.to_owned())
}

fn read_u16(input: &[u8], cursor: &mut usize) -> Result<u16, SpoolError> {
    Ok(u16::from_le_bytes(
        take(input, cursor, 2)?
            .try_into()
            .map_err(|_| SpoolError::InvalidHeader)?,
    ))
}

fn read_u64(input: &[u8], cursor: &mut usize) -> Result<u64, SpoolError> {
    Ok(u64::from_le_bytes(
        take(input, cursor, 8)?
            .try_into()
            .map_err(|_| SpoolError::InvalidHeader)?,
    ))
}

fn read_i64(input: &[u8], cursor: &mut usize) -> Result<i64, SpoolError> {
    Ok(i64::from_le_bytes(
        take(input, cursor, 8)?
            .try_into()
            .map_err(|_| SpoolError::InvalidHeader)?,
    ))
}

fn take<'a>(input: &'a [u8], cursor: &mut usize, length: usize) -> Result<&'a [u8], SpoolError> {
    let end = cursor
        .checked_add(length)
        .ok_or(SpoolError::InvalidHeader)?;
    let bytes = input
        .get(*cursor..end)
        .ok_or(SpoolError::IncompleteHeader)?;
    *cursor = end;
    Ok(bytes)
}

fn valid_identity(value: &str) -> bool {
    !value.is_empty()
        && value.trim() == value
        && value.len() <= MAX_IDENTITY_BYTES
        && !value.chars().any(char::is_control)
}

fn read_exact_header(file: &mut File, output: &mut [u8]) -> Result<(), SpoolError> {
    file.read_exact(output).map_err(|source| {
        if source.kind() == std::io::ErrorKind::UnexpectedEof {
            SpoolError::IncompleteHeader
        } else {
            io_error("reading the segment header", source)
        }
    })
}
