use bytes::Bytes;
use domain_types::SourceId;
use hl_protocol::{
    ObservationClass, ObservationError, ParseWarning, ReceiveTimestamps, SourceCursor,
    SourceObservation,
};

use super::{MAX_IDENTITY_BYTES, MAX_PAYLOAD_BYTES, MAX_RECORD_BYTES, SpoolError};

const CRC_BYTES: usize = 4;
const MIN_BODY_BYTES: usize = 2 + 1 + 8 + 1 + 8 + 8 + 2 + 1 + 4 + 1 + 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpoolRecord {
    cursor: SourceCursor,
    observation_class: ObservationClass,
    received: ReceiveTimestamps,
    parser_schema_version: String,
    payload: Bytes,
    content_hash: blake3::Hash,
}

impl SpoolRecord {
    #[must_use]
    pub const fn cursor(&self) -> &SourceCursor {
        &self.cursor
    }

    #[must_use]
    pub const fn observation_class(&self) -> ObservationClass {
        self.observation_class
    }

    #[must_use]
    pub const fn received(&self) -> ReceiveTimestamps {
        self.received
    }

    #[must_use]
    pub fn parser_schema_version(&self) -> &str {
        &self.parser_schema_version
    }

    #[must_use]
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    #[must_use]
    pub const fn content_hash(&self) -> blake3::Hash {
        self.content_hash
    }

    pub fn into_observation(
        self,
        source_id: SourceId,
        source_version: impl Into<String>,
        max_payload_bytes: usize,
    ) -> Result<SourceObservation, ObservationError> {
        SourceObservation::new(
            source_id,
            source_version,
            self.observation_class,
            self.cursor,
            self.received,
            self.parser_schema_version,
            self.payload,
            Vec::<ParseWarning>::new(),
            max_payload_bytes,
        )
    }
}

pub(crate) fn encode_record(observation: &SourceObservation) -> Result<Vec<u8>, SpoolError> {
    let body_capacity = validate_record(observation)?;
    let mut body = Vec::with_capacity(body_capacity);
    push_text(&mut body, observation.cursor().epoch())?;
    body.extend_from_slice(&observation.cursor().offset().to_le_bytes());
    body.push(class_to_u8(observation.observation_class()));
    body.extend_from_slice(&observation.received().wall_micros().to_le_bytes());
    body.extend_from_slice(&observation.received().monotonic_nanos().to_le_bytes());
    push_text(&mut body, observation.parser_schema_version())?;
    body.extend_from_slice(
        &u32::try_from(observation.payload().len())
            .map_err(|_| SpoolError::SizeOverflow)?
            .to_le_bytes(),
    );
    body.extend_from_slice(observation.payload());
    body.extend_from_slice(observation.content_hash().as_bytes());

    let record_len = CRC_BYTES
        .checked_add(body.len())
        .ok_or(SpoolError::SizeOverflow)?;
    let mut encoded = Vec::with_capacity(record_len + 4);
    encoded.extend_from_slice(
        &u32::try_from(record_len)
            .map_err(|_| SpoolError::SizeOverflow)?
            .to_le_bytes(),
    );
    encoded.extend_from_slice(&crc32c::crc32c(&body).to_le_bytes());
    encoded.extend_from_slice(&body);
    Ok(encoded)
}

pub(crate) fn validate_record(observation: &SourceObservation) -> Result<usize, SpoolError> {
    if !observation.warnings().is_empty() {
        return Err(SpoolError::UnsupportedWarnings);
    }
    if !valid_text(observation.cursor().epoch())
        || !valid_text(observation.parser_schema_version())
        || observation.payload().len() > MAX_PAYLOAD_BYTES
    {
        return Err(SpoolError::SizeOverflow);
    }
    let body_len = MIN_BODY_BYTES
        .checked_add(observation.cursor().epoch().len())
        .and_then(|length| length.checked_add(observation.parser_schema_version().len()))
        .and_then(|length| length.checked_add(observation.payload().len()))
        .ok_or(SpoolError::SizeOverflow)?;
    let record_len = CRC_BYTES
        .checked_add(body_len)
        .ok_or(SpoolError::SizeOverflow)?;
    if record_len > MAX_RECORD_BYTES {
        return Err(SpoolError::SizeOverflow);
    }
    Ok(body_len)
}

pub(crate) fn decode_record(framed: &[u8], record_offset: u64) -> Result<SpoolRecord, SpoolError> {
    if framed.len() < CRC_BYTES + MIN_BODY_BYTES || framed.len() > MAX_RECORD_BYTES {
        return Err(SpoolError::CorruptRecord { record_offset });
    }
    let expected_crc = u32::from_le_bytes(
        framed[..4]
            .try_into()
            .map_err(|_| SpoolError::CorruptRecord { record_offset })?,
    );
    let body = &framed[4..];
    if crc32c::crc32c(body) != expected_crc {
        return Err(SpoolError::CorruptRecord { record_offset });
    }

    let mut cursor = 0;
    let epoch = read_text(body, &mut cursor, record_offset)?;
    let offset = read_u64(body, &mut cursor, record_offset)?;
    let class = class_from_u8(read_u8(body, &mut cursor, record_offset)?)
        .ok_or(SpoolError::CorruptRecord { record_offset })?;
    let wall_micros = read_i64(body, &mut cursor, record_offset)?;
    let monotonic_nanos = read_u64(body, &mut cursor, record_offset)?;
    let parser_schema_version = read_text(body, &mut cursor, record_offset)?;
    let payload_len = usize::try_from(read_u32(body, &mut cursor, record_offset)?)
        .map_err(|_| SpoolError::CorruptRecord { record_offset })?;
    if payload_len == 0 || payload_len > MAX_PAYLOAD_BYTES {
        return Err(SpoolError::CorruptRecord { record_offset });
    }
    let payload = Bytes::copy_from_slice(take(body, &mut cursor, payload_len, record_offset)?);
    let content_hash = blake3::Hash::from_bytes(
        take(body, &mut cursor, 32, record_offset)?
            .try_into()
            .map_err(|_| SpoolError::CorruptRecord { record_offset })?,
    );
    if cursor != body.len() || blake3::hash(&payload) != content_hash {
        return Err(SpoolError::CorruptRecord { record_offset });
    }
    let cursor_value = SourceCursor::new(epoch, offset)
        .map_err(|_| SpoolError::CorruptRecord { record_offset })?;
    let received = ReceiveTimestamps::new(wall_micros, monotonic_nanos)
        .map_err(|_| SpoolError::CorruptRecord { record_offset })?;
    Ok(SpoolRecord {
        cursor: cursor_value,
        observation_class: class,
        received,
        parser_schema_version,
        payload,
        content_hash,
    })
}

fn class_to_u8(class: ObservationClass) -> u8 {
    match class {
        ObservationClass::CommittedBlock => 1,
        ObservationClass::AuxiliaryOrderStatus => 2,
        ObservationClass::AuxiliaryBookDiff => 3,
        ObservationClass::AuxiliaryLedger => 4,
        ObservationClass::Snapshot => 5,
        ObservationClass::HistoricalBlock => 6,
        ObservationClass::PublicMarketData => 7,
        ObservationClass::ProvisionalFeed => 8,
        ObservationClass::ProvisionalMempool => 9,
    }
}

fn class_from_u8(value: u8) -> Option<ObservationClass> {
    match value {
        1 => Some(ObservationClass::CommittedBlock),
        2 => Some(ObservationClass::AuxiliaryOrderStatus),
        3 => Some(ObservationClass::AuxiliaryBookDiff),
        4 => Some(ObservationClass::AuxiliaryLedger),
        5 => Some(ObservationClass::Snapshot),
        6 => Some(ObservationClass::HistoricalBlock),
        7 => Some(ObservationClass::PublicMarketData),
        8 => Some(ObservationClass::ProvisionalFeed),
        9 => Some(ObservationClass::ProvisionalMempool),
        _ => None,
    }
}

fn push_text(output: &mut Vec<u8>, value: &str) -> Result<(), SpoolError> {
    if !valid_text(value) {
        return Err(SpoolError::SizeOverflow);
    }
    output.extend_from_slice(
        &u16::try_from(value.len())
            .map_err(|_| SpoolError::SizeOverflow)?
            .to_le_bytes(),
    );
    output.extend_from_slice(value.as_bytes());
    Ok(())
}

fn valid_text(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_IDENTITY_BYTES
        && value.trim() == value
        && !value.chars().any(char::is_control)
}

fn read_text(input: &[u8], cursor: &mut usize, record_offset: u64) -> Result<String, SpoolError> {
    let length = usize::from(read_u16(input, cursor, record_offset)?);
    if !(1..=MAX_IDENTITY_BYTES).contains(&length) {
        return Err(SpoolError::CorruptRecord { record_offset });
    }
    let bytes = take(input, cursor, length, record_offset)?;
    let value =
        std::str::from_utf8(bytes).map_err(|_| SpoolError::CorruptRecord { record_offset })?;
    if value.trim() != value || value.chars().any(char::is_control) {
        return Err(SpoolError::CorruptRecord { record_offset });
    }
    Ok(value.to_owned())
}

fn read_u8(input: &[u8], cursor: &mut usize, record_offset: u64) -> Result<u8, SpoolError> {
    Ok(*take(input, cursor, 1, record_offset)?
        .first()
        .ok_or(SpoolError::CorruptRecord { record_offset })?)
}

fn read_u16(input: &[u8], cursor: &mut usize, record_offset: u64) -> Result<u16, SpoolError> {
    Ok(u16::from_le_bytes(
        take(input, cursor, 2, record_offset)?
            .try_into()
            .map_err(|_| SpoolError::CorruptRecord { record_offset })?,
    ))
}

fn read_u32(input: &[u8], cursor: &mut usize, record_offset: u64) -> Result<u32, SpoolError> {
    Ok(u32::from_le_bytes(
        take(input, cursor, 4, record_offset)?
            .try_into()
            .map_err(|_| SpoolError::CorruptRecord { record_offset })?,
    ))
}

fn read_u64(input: &[u8], cursor: &mut usize, record_offset: u64) -> Result<u64, SpoolError> {
    Ok(u64::from_le_bytes(
        take(input, cursor, 8, record_offset)?
            .try_into()
            .map_err(|_| SpoolError::CorruptRecord { record_offset })?,
    ))
}

fn read_i64(input: &[u8], cursor: &mut usize, record_offset: u64) -> Result<i64, SpoolError> {
    Ok(i64::from_le_bytes(
        take(input, cursor, 8, record_offset)?
            .try_into()
            .map_err(|_| SpoolError::CorruptRecord { record_offset })?,
    ))
}

fn take<'a>(
    input: &'a [u8],
    cursor: &mut usize,
    length: usize,
    record_offset: u64,
) -> Result<&'a [u8], SpoolError> {
    let end = cursor
        .checked_add(length)
        .ok_or(SpoolError::CorruptRecord { record_offset })?;
    let bytes = input
        .get(*cursor..end)
        .ok_or(SpoolError::CorruptRecord { record_offset })?;
    *cursor = end;
    Ok(bytes)
}
