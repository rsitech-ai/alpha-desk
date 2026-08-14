use std::{
    fs::{self, File},
    io::{Read as _, Write as _},
    os::unix::fs::{DirBuilderExt as _, PermissionsExt as _},
    path::{Component, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use rustix::fs::{Mode, OFlags, open};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::CanonicalDelivery;

pub const DEAD_LETTER_SCHEMA_V1: &str = "hl.core.deadletter.v1";
const MAX_IDENTITY_BYTES: usize = 512;
const MAX_RECORD_BYTES: usize = 4_096;
const MAX_DEAD_LETTER_FILE_BYTES: usize = MAX_RECORD_BYTES.saturating_mul(256);
const MAX_DEAD_LETTER_PATH_BYTES: usize = 4_096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadLetterRecord {
    reason_code: String,
    subject: String,
    message_id: String,
    stream_sequence: Option<u64>,
    consumer_sequence: Option<u64>,
    payload_sha256: [u8; 32],
    block_hash: [u8; 32],
    consumer: String,
    retry_count: u64,
    failed_at_unix_micros: i64,
}

impl DeadLetterRecord {
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        reason_code: impl Into<String>,
        subject: impl Into<String>,
        message_id: impl Into<String>,
        stream_sequence: Option<u64>,
        consumer_sequence: Option<u64>,
        payload_sha256: [u8; 32],
        block_hash: [u8; 32],
        consumer: impl Into<String>,
        retry_count: u64,
        failed_at_unix_micros: i64,
    ) -> Result<Self, DeadLetterError> {
        let reason_code = reason_code.into();
        let subject = subject.into();
        let message_id = message_id.into();
        let consumer = consumer.into();
        validate_reason_code(&reason_code)?;
        validate_identity(&subject)?;
        validate_identity(&message_id)?;
        validate_identity(&consumer)?;
        if failed_at_unix_micros < 0 {
            return Err(DeadLetterError::InvalidRecord);
        }
        Ok(Self {
            reason_code,
            subject,
            message_id,
            stream_sequence,
            consumer_sequence,
            payload_sha256,
            block_hash,
            consumer,
            retry_count,
            failed_at_unix_micros,
        })
    }

    pub fn from_delivery(
        delivery: &CanonicalDelivery,
        reason_code: &str,
        consumer: &str,
        failed_at_unix_micros: i64,
    ) -> Result<Self, DeadLetterError> {
        Self::try_new(
            reason_code,
            delivery.subject.as_str(),
            delivery.message_id.as_str(),
            delivery.stream_sequence,
            delivery.consumer_sequence,
            Sha256::digest(&delivery.payload).into(),
            delivery.block_hash,
            consumer,
            delivery.delivery_count,
            failed_at_unix_micros,
        )
    }

    #[must_use]
    pub fn reason_code(&self) -> &str {
        &self.reason_code
    }

    #[must_use]
    pub fn subject(&self) -> &str {
        &self.subject
    }

    #[must_use]
    pub fn message_id(&self) -> &str {
        &self.message_id
    }

    #[must_use]
    pub const fn stream_sequence(&self) -> Option<u64> {
        self.stream_sequence
    }

    #[must_use]
    pub const fn consumer_sequence(&self) -> Option<u64> {
        self.consumer_sequence
    }

    #[must_use]
    pub const fn payload_sha256(&self) -> [u8; 32] {
        self.payload_sha256
    }

    #[must_use]
    pub const fn block_hash(&self) -> [u8; 32] {
        self.block_hash
    }

    #[must_use]
    pub fn consumer(&self) -> &str {
        &self.consumer
    }

    #[must_use]
    pub const fn retry_count(&self) -> u64 {
        self.retry_count
    }

    #[must_use]
    pub const fn failed_at_unix_micros(&self) -> i64 {
        self.failed_at_unix_micros
    }
}

pub trait DeadLetterSink: Send {
    fn persist(&mut self, record: &DeadLetterRecord) -> Result<(), DeadLetterError>;
}

#[derive(Debug, Default)]
pub struct InMemoryDeadLetterSink {
    records: Vec<DeadLetterRecord>,
}

impl InMemoryDeadLetterSink {
    #[must_use]
    pub fn records(&self) -> &[DeadLetterRecord] {
        &self.records
    }
}

impl DeadLetterSink for InMemoryDeadLetterSink {
    fn persist(&mut self, record: &DeadLetterRecord) -> Result<(), DeadLetterError> {
        self.records.push(record.clone());
        Ok(())
    }
}

#[derive(Debug)]
pub struct FileDeadLetterSink {
    file: Option<File>,
    path: PathBuf,
}

impl FileDeadLetterSink {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, DeadLetterError> {
        let path = path.as_ref();
        validate_dead_letter_path(path)?;
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            ensure_private_directory(parent)?;
        }
        let file = match fs::symlink_metadata(path) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                return Err(DeadLetterError::UnsafePath);
            }
            Ok(metadata) if metadata.len() == 0 => match fs::remove_file(path) {
                Ok(()) => None,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(_) => return Err(DeadLetterError::Io),
            },
            Ok(_) => {
                validate_existing_jsonl(path)?;
                Some(Self::open_file(path, false)?)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => return Err(DeadLetterError::Io),
        };
        Ok(Self {
            file,
            path: path.to_path_buf(),
        })
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn open_file(path: &Path, create: bool) -> Result<File, DeadLetterError> {
        let mut flags = OFlags::WRONLY | OFlags::APPEND | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        if create {
            flags |= OFlags::CREATE;
        }
        let file = File::from(
            open(path, flags, Mode::RUSR | Mode::WUSR).map_err(|_| DeadLetterError::Io)?,
        );
        let opened = match file.metadata() {
            Ok(opened) => opened,
            Err(_) => return abandon_created(path, create, DeadLetterError::Io),
        };
        if !opened.is_file() {
            return abandon_created(path, create, DeadLetterError::UnsafePath);
        }
        if file
            .set_permissions(fs::Permissions::from_mode(0o600))
            .is_err()
        {
            return abandon_created(path, create, DeadLetterError::Io);
        }
        Ok(file)
    }
}

impl DeadLetterSink for FileDeadLetterSink {
    fn persist(&mut self, record: &DeadLetterRecord) -> Result<(), DeadLetterError> {
        let encoded = encode_record(record)?;
        let created = self.file.is_none();
        let file = match self.file.as_mut() {
            Some(file) => file,
            None => self.file.insert(Self::open_file(&self.path, true)?),
        };
        match file.write_all(&encoded).and_then(|_| file.sync_all()) {
            Ok(()) => Ok(()),
            Err(_) => {
                if created {
                    self.file = None;
                    let _ = fs::remove_file(&self.path);
                }
                Err(DeadLetterError::Io)
            }
        }
    }
}

#[derive(Debug, thiserror::Error, Clone, Copy, PartialEq, Eq)]
pub enum DeadLetterError {
    #[error("dead-letter path is unsafe")]
    UnsafePath,
    #[error("dead-letter record could not be persisted")]
    Io,
    #[error("dead-letter record is invalid")]
    InvalidRecord,
    #[error("dead-letter record could not be serialized")]
    Serialization,
    #[error("dead-letter file is truncated or corrupt")]
    Corrupt,
}

impl DeadLetterError {
    #[must_use]
    pub const fn reason_code(self) -> &'static str {
        match self {
            Self::UnsafePath => "core.deadletter_unsafe_path",
            Self::Io => "core.deadletter_io",
            Self::InvalidRecord => "core.deadletter_invalid_record",
            Self::Serialization => "core.deadletter_serialization",
            Self::Corrupt => "core.deadletter_corrupt",
        }
    }
}

pub(crate) fn failed_at_unix_micros() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| i64::try_from(elapsed.as_micros()).ok())
        .unwrap_or(0)
}

fn validate_existing_jsonl(path: &Path) -> Result<(), DeadLetterError> {
    let mut file = File::from(
        open(
            path,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|_| DeadLetterError::Io)?,
    );
    let mut bytes = Vec::new();
    let limit = u64::try_from(MAX_DEAD_LETTER_FILE_BYTES.saturating_add(1)).unwrap_or(u64::MAX);
    std::io::Read::take(&mut file, limit)
        .read_to_end(&mut bytes)
        .map_err(|_| DeadLetterError::Io)?;
    if bytes.is_empty() {
        return Err(DeadLetterError::Io);
    }
    if bytes.len() > MAX_DEAD_LETTER_FILE_BYTES {
        return Err(DeadLetterError::Corrupt);
    }
    decode_existing_jsonl(&bytes)
}

fn decode_existing_jsonl(bytes: &[u8]) -> Result<(), DeadLetterError> {
    let Some(payload) = bytes.strip_suffix(b"\n") else {
        return Err(DeadLetterError::Corrupt);
    };
    if payload.is_empty() {
        return Err(DeadLetterError::Corrupt);
    }
    for line in payload.split(|&byte| byte == b'\n') {
        decode_existing_line(line)?;
    }
    Ok(())
}

fn decode_existing_line(line: &[u8]) -> Result<(), DeadLetterError> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct ExistingRecord {
        schema_version: String,
        reason_code: String,
        subject: String,
        message_id: String,
        #[serde(default)]
        stream_sequence: Option<u64>,
        #[serde(default)]
        consumer_sequence: Option<u64>,
        payload_sha256: String,
        block_hash: String,
        consumer: String,
        retry_count: u64,
        failed_at_unix_micros: i64,
    }

    if line.is_empty() || line.len() > MAX_RECORD_BYTES {
        return Err(DeadLetterError::Corrupt);
    }
    let decoded: ExistingRecord =
        serde_json::from_slice(line).map_err(|_| DeadLetterError::Corrupt)?;
    if decoded.schema_version != DEAD_LETTER_SCHEMA_V1 {
        return Err(DeadLetterError::Corrupt);
    }
    let payload_sha256 = decode_record_hash(&decoded.payload_sha256)?;
    let block_hash = decode_record_hash(&decoded.block_hash)?;
    DeadLetterRecord::try_new(
        decoded.reason_code,
        decoded.subject,
        decoded.message_id,
        decoded.stream_sequence,
        decoded.consumer_sequence,
        payload_sha256,
        block_hash,
        decoded.consumer,
        decoded.retry_count,
        decoded.failed_at_unix_micros,
    )
    .map(|_| ())
    .map_err(|_| DeadLetterError::Corrupt)
}

fn decode_record_hash(value: &str) -> Result<[u8; 32], DeadLetterError> {
    let bytes = hex::decode(value).map_err(|_| DeadLetterError::Corrupt)?;
    <[u8; 32]>::try_from(bytes).map_err(|_| DeadLetterError::Corrupt)
}

fn encode_record(record: &DeadLetterRecord) -> Result<Vec<u8>, DeadLetterError> {
    #[derive(Serialize)]
    struct FileRecord<'a> {
        schema_version: &'static str,
        reason_code: &'a str,
        subject: &'a str,
        message_id: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        stream_sequence: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        consumer_sequence: Option<u64>,
        payload_sha256: String,
        block_hash: String,
        consumer: &'a str,
        retry_count: u64,
        failed_at_unix_micros: i64,
    }

    let mut encoded = serde_json::to_vec(&FileRecord {
        schema_version: DEAD_LETTER_SCHEMA_V1,
        reason_code: &record.reason_code,
        subject: &record.subject,
        message_id: &record.message_id,
        stream_sequence: record.stream_sequence,
        consumer_sequence: record.consumer_sequence,
        payload_sha256: hex::encode(record.payload_sha256),
        block_hash: hex::encode(record.block_hash),
        consumer: &record.consumer,
        retry_count: record.retry_count,
        failed_at_unix_micros: record.failed_at_unix_micros,
    })
    .map_err(|_| DeadLetterError::Serialization)?;
    if encoded.len() > MAX_RECORD_BYTES {
        return Err(DeadLetterError::Serialization);
    }
    encoded.push(b'\n');
    Ok(encoded)
}

fn validate_reason_code(value: &str) -> Result<(), DeadLetterError> {
    if value.is_empty()
        || value.trim() != value
        || value.len() > MAX_IDENTITY_BYTES
        || !value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_')
        })
    {
        Err(DeadLetterError::InvalidRecord)
    } else {
        Ok(())
    }
}

fn validate_identity(value: &str) -> Result<(), DeadLetterError> {
    if value.is_empty()
        || value.trim() != value
        || value.len() > MAX_IDENTITY_BYTES
        || value.chars().any(char::is_control)
    {
        Err(DeadLetterError::InvalidRecord)
    } else {
        Ok(())
    }
}

pub(crate) fn validate_dead_letter_path(path: &Path) -> Result<(), DeadLetterError> {
    if path.as_os_str().is_empty()
        || path == Path::new("/")
        || path.as_os_str().len() > MAX_DEAD_LETTER_PATH_BYTES
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
    {
        Err(DeadLetterError::UnsafePath)
    } else {
        Ok(())
    }
}

fn ensure_private_directory(path: &Path) -> Result<(), DeadLetterError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            Err(DeadLetterError::UnsafePath)
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(path)
                .map_err(|_| DeadLetterError::Io)?;
            let created = fs::symlink_metadata(path).map_err(|_| DeadLetterError::Io)?;
            if created.file_type().is_symlink() || !created.is_dir() {
                return Err(DeadLetterError::UnsafePath);
            }
            Ok(())
        }
        Err(_) => Err(DeadLetterError::Io),
    }
}

fn abandon_created<T>(
    path: &Path,
    create: bool,
    error: DeadLetterError,
) -> Result<T, DeadLetterError> {
    if create {
        let _ = fs::remove_file(path);
    }
    Err(error)
}
