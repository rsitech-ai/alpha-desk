use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

use domain_types::SourceId;
use storage_ports::CursorPolicy;

use super::{MAX_HEADER_BYTES, MAX_IDENTITY_BYTES, SpoolError, io_error};

const MAGIC_V1: [u8; 8] = *b"HLSPV001";
const MAGIC_V2: [u8; 8] = *b"HLSPV002";
const FIXED_HEADER_V1_BYTES: usize = 8 + 4 + 2 + 2 + 2 + 8 + 8 + 32;
const FIXED_HEADER_V2_BYTES: usize = FIXED_HEADER_V1_BYTES + 1;
const BYTE_OFFSET_POLICY: u8 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
/// Decoded header for both legacy `HLSPV001` segments and policy-tagged
/// `HLSPV002` segments.
pub struct SegmentHeader {
    source_id: SourceId,
    source_version: String,
    schema_version: String,
    segment_sequence: u64,
    created_at_micros: i64,
    producer_build_hash: [u8; 32],
    cursor_policy: CursorPolicy,
}

/// Compatibility name for callers that construct the byte-stable V1 format.
/// `SegmentHeaderV1::new` always emits `HLSPV001`.
pub type SegmentHeaderV1 = SegmentHeader;

impl SegmentHeader {
    pub fn new(
        source_id: SourceId,
        source_version: impl Into<String>,
        schema_version: impl Into<String>,
        segment_sequence: u64,
        created_at_micros: i64,
        producer_build_hash: [u8; 32],
    ) -> Result<Self, SpoolError> {
        Self::new_with_cursor_policy(
            source_id,
            source_version,
            schema_version,
            segment_sequence,
            created_at_micros,
            producer_build_hash,
            CursorPolicy::ContiguousNativeOffset,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new_with_cursor_policy(
        source_id: SourceId,
        source_version: impl Into<String>,
        schema_version: impl Into<String>,
        segment_sequence: u64,
        created_at_micros: i64,
        producer_build_hash: [u8; 32],
        cursor_policy: CursorPolicy,
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
            cursor_policy,
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

    #[must_use]
    pub const fn cursor_policy(&self) -> CursorPolicy {
        self.cursor_policy
    }

    pub(crate) fn encode(&self) -> Result<Vec<u8>, SpoolError> {
        let fixed_header_bytes = match self.cursor_policy {
            CursorPolicy::ContiguousNativeOffset => FIXED_HEADER_V1_BYTES,
            CursorPolicy::MonotonicByteOffset => FIXED_HEADER_V2_BYTES,
        };
        let header_len = fixed_header_bytes
            .checked_add(self.source_id.as_str().len())
            .and_then(|length| length.checked_add(self.source_version.len()))
            .and_then(|length| length.checked_add(self.schema_version.len()))
            .ok_or(SpoolError::SizeOverflow)?;
        if header_len > MAX_HEADER_BYTES {
            return Err(SpoolError::InvalidHeader);
        }
        let mut encoded = Vec::with_capacity(header_len);
        encoded.extend_from_slice(match self.cursor_policy {
            CursorPolicy::ContiguousNativeOffset => &MAGIC_V1,
            CursorPolicy::MonotonicByteOffset => &MAGIC_V2,
        });
        encoded.extend_from_slice(
            &u32::try_from(header_len)
                .map_err(|_| SpoolError::SizeOverflow)?
                .to_le_bytes(),
        );
        push_text(&mut encoded, self.source_id.as_str())?;
        push_text(&mut encoded, &self.source_version)?;
        push_text(&mut encoded, &self.schema_version)?;
        match self.cursor_policy {
            CursorPolicy::MonotonicByteOffset => encoded.push(BYTE_OFFSET_POLICY),
            CursorPolicy::ContiguousNativeOffset => {
                // Legacy HLSPV001 headers omit the policy byte.
            }
        }
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
        let fixed_header_bytes = fixed_header_bytes(&prefix[..8])?;
        let header_len = usize::try_from(u32::from_le_bytes(
            prefix[8..12]
                .try_into()
                .map_err(|_| SpoolError::InvalidHeader)?,
        ))
        .map_err(|_| SpoolError::SizeOverflow)?;
        if !(fixed_header_bytes..=MAX_HEADER_BYTES).contains(&header_len) {
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
        let fixed_header_bytes = fixed_header_bytes(&prefix[..8])?;
        let header_len = usize::try_from(u32::from_le_bytes(
            prefix[8..12]
                .try_into()
                .map_err(|_| SpoolError::InvalidHeader)?,
        ))
        .map_err(|_| SpoolError::SizeOverflow)?;
        if !(fixed_header_bytes..=MAX_HEADER_BYTES).contains(&header_len) {
            return Err(SpoolError::InvalidHeader);
        }
        let encoded = input
            .get(..header_len)
            .ok_or(SpoolError::IncompleteHeader)?;
        Ok((Self::decode(encoded)?, header_len))
    }

    fn decode(encoded: &[u8]) -> Result<Self, SpoolError> {
        let cursor_policy = match encoded.get(..8) {
            Some(magic) if magic == MAGIC_V1 => CursorPolicy::ContiguousNativeOffset,
            Some(magic) if magic == MAGIC_V2 => CursorPolicy::MonotonicByteOffset,
            _ => return Err(SpoolError::InvalidHeader),
        };
        let mut cursor = 12;
        let source_id = read_text(encoded, &mut cursor)?;
        let source_version = read_text(encoded, &mut cursor)?;
        let schema_version = read_text(encoded, &mut cursor)?;
        match cursor_policy {
            CursorPolicy::MonotonicByteOffset => {
                if read_u8(encoded, &mut cursor)? != BYTE_OFFSET_POLICY {
                    return Err(SpoolError::InvalidHeader);
                }
            }
            CursorPolicy::ContiguousNativeOffset => {
                // Legacy HLSPV001 headers omit the policy byte.
            }
        }
        let segment_sequence = read_u64(encoded, &mut cursor)?;
        let created_at_micros = read_i64(encoded, &mut cursor)?;
        let producer_build_hash = take(encoded, &mut cursor, 32)?
            .try_into()
            .map_err(|_| SpoolError::InvalidHeader)?;
        if cursor != encoded.len() {
            return Err(SpoolError::InvalidHeader);
        }
        let source_id = SourceId::new(source_id).map_err(|_| SpoolError::InvalidHeader)?;
        Self::new_with_cursor_policy(
            source_id,
            source_version,
            schema_version,
            segment_sequence,
            created_at_micros,
            producer_build_hash,
            cursor_policy,
        )
    }
}

fn fixed_header_bytes(magic: &[u8]) -> Result<usize, SpoolError> {
    if magic == MAGIC_V1 {
        Ok(FIXED_HEADER_V1_BYTES)
    } else if magic == MAGIC_V2 {
        Ok(FIXED_HEADER_V2_BYTES)
    } else {
        Err(SpoolError::InvalidHeader)
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

fn read_u8(input: &[u8], cursor: &mut usize) -> Result<u8, SpoolError> {
    take(input, cursor, 1)?
        .first()
        .copied()
        .ok_or(SpoolError::InvalidHeader)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn example_header(cursor_policy: CursorPolicy) -> SegmentHeader {
        SegmentHeader::new_with_cursor_policy(
            SourceId::new("primary-node").unwrap(),
            "hyperliquid-node-v1",
            "spool-v1",
            1,
            100,
            [0x31; 32],
            cursor_policy,
        )
        .unwrap()
    }

    fn after_identities(encoded: &[u8]) -> usize {
        let mut cursor = 12;
        for _ in 0..3 {
            let length = usize::from(u16::from_le_bytes(
                encoded[cursor..cursor + 2]
                    .try_into()
                    .expect("identity length prefix"),
            ));
            cursor += 2 + length;
        }
        cursor
    }

    #[test]
    fn encode_decode_covers_every_constructible_cursor_policy() {
        for cursor_policy in [
            CursorPolicy::ContiguousNativeOffset,
            CursorPolicy::MonotonicByteOffset,
        ] {
            let header = example_header(cursor_policy);
            let encoded = header
                .encode()
                .expect("constructible policies still encode");
            let after = after_identities(&encoded);
            let identity_bytes = header.source_id().as_str().len()
                + header.source_version().len()
                + header.schema_version().len();
            match cursor_policy {
                CursorPolicy::ContiguousNativeOffset => {
                    assert_eq!(&encoded[..8], MAGIC_V1);
                    assert_eq!(encoded.len(), FIXED_HEADER_V1_BYTES + identity_bytes);
                    assert_eq!(&encoded[after..after + 8], 1_u64.to_le_bytes().as_slice());
                }
                CursorPolicy::MonotonicByteOffset => {
                    assert_eq!(&encoded[..8], MAGIC_V2);
                    assert_eq!(encoded.len(), FIXED_HEADER_V2_BYTES + identity_bytes);
                    assert_eq!(encoded[after], BYTE_OFFSET_POLICY);
                    assert_eq!(
                        &encoded[after + 1..after + 9],
                        1_u64.to_le_bytes().as_slice()
                    );
                }
            }

            let (decoded, header_len) = SegmentHeader::read_from_slice(&encoded)
                .expect("constructible policies still decode today's wire bytes");
            assert_eq!(header_len, encoded.len());
            assert_eq!(decoded, header);
            assert_eq!(decoded.cursor_policy(), cursor_policy);
        }
    }

    #[test]
    fn v2_unknown_policy_byte_still_fails_as_invalid_header() {
        let header = example_header(CursorPolicy::MonotonicByteOffset);
        let mut encoded = header.encode().unwrap();
        let after = after_identities(&encoded);
        encoded[after] = 0;
        let error =
            SegmentHeader::decode(&encoded).expect_err("unknown V2 policy bytes fail closed");
        assert!(matches!(error, SpoolError::InvalidHeader));
        assert_eq!(error.reason_code(), "spool.invalid_header");
    }
}
