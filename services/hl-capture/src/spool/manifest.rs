use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use hl_protocol::SourceCursor;
use serde::{Deserialize, Serialize};

use super::{SpoolError, io_error};

pub const MANIFEST_SCHEMA_V1: &str = "hl-spool-manifest-v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClosedSegmentManifestV1 {
    schema_version: String,
    segment_sequence: u64,
    segment_file: String,
    source_id: String,
    source_version: String,
    spool_schema_version: String,
    #[serde(with = "hex_32")]
    producer_build_hash: [u8; 32],
    file_size_bytes: u64,
    record_count: u64,
    min_cursor: SourceCursor,
    max_cursor: SourceCursor,
    #[serde(with = "hex_32")]
    segment_blake3: [u8; 32],
    #[serde(with = "option_hex_32")]
    previous_manifest_blake3: Option<[u8; 32]>,
    closed_at_micros: i64,
}

impl ClosedSegmentManifestV1 {
    pub(crate) fn new(fields: ManifestFields) -> Result<Self, SpoolError> {
        if fields.record_count == 0
            || fields.file_size_bytes == 0
            || fields.closed_at_micros < 0
            || fields.segment_file.is_empty()
            || fields.min_cursor.epoch() != fields.max_cursor.epoch()
            || fields.min_cursor.offset() > fields.max_cursor.offset()
        {
            return Err(SpoolError::InvalidManifest);
        }
        Ok(Self {
            schema_version: MANIFEST_SCHEMA_V1.to_owned(),
            segment_sequence: fields.segment_sequence,
            segment_file: fields.segment_file,
            source_id: fields.source_id,
            source_version: fields.source_version,
            spool_schema_version: fields.spool_schema_version,
            producer_build_hash: fields.producer_build_hash,
            file_size_bytes: fields.file_size_bytes,
            record_count: fields.record_count,
            min_cursor: fields.min_cursor,
            max_cursor: fields.max_cursor,
            segment_blake3: fields.segment_blake3,
            previous_manifest_blake3: fields.previous_manifest_blake3,
            closed_at_micros: fields.closed_at_micros,
        })
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
    pub fn segment_file(&self) -> &str {
        &self.segment_file
    }

    #[must_use]
    pub fn source_id(&self) -> &str {
        &self.source_id
    }

    #[must_use]
    pub fn source_version(&self) -> &str {
        &self.source_version
    }

    #[must_use]
    pub fn spool_schema_version(&self) -> &str {
        &self.spool_schema_version
    }

    #[must_use]
    pub const fn producer_build_hash(&self) -> [u8; 32] {
        self.producer_build_hash
    }

    #[must_use]
    pub const fn file_size_bytes(&self) -> u64 {
        self.file_size_bytes
    }

    #[must_use]
    pub const fn record_count(&self) -> u64 {
        self.record_count
    }

    #[must_use]
    pub const fn min_cursor(&self) -> &SourceCursor {
        &self.min_cursor
    }

    #[must_use]
    pub const fn max_cursor(&self) -> &SourceCursor {
        &self.max_cursor
    }

    #[must_use]
    pub const fn segment_blake3(&self) -> [u8; 32] {
        self.segment_blake3
    }

    #[must_use]
    pub const fn previous_manifest_blake3(&self) -> Option<[u8; 32]> {
        self.previous_manifest_blake3
    }

    #[must_use]
    pub const fn closed_at_micros(&self) -> i64 {
        self.closed_at_micros
    }

    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, SpoolError> {
        let encoded = fs::read(path.as_ref())
            .map_err(|source| io_error("reading a closed-segment manifest", source))?;
        let manifest: Self =
            serde_json::from_slice(&encoded).map_err(|_| SpoolError::InvalidManifest)?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub(crate) fn encode(&self) -> Result<Vec<u8>, SpoolError> {
        self.validate()?;
        let mut encoded =
            serde_json::to_vec_pretty(self).map_err(|_| SpoolError::InvalidManifest)?;
        encoded.push(b'\n');
        Ok(encoded)
    }

    pub(crate) fn validate(&self) -> Result<(), SpoolError> {
        if self.schema_version != MANIFEST_SCHEMA_V1
            || self.record_count == 0
            || self.file_size_bytes == 0
            || self.segment_file.is_empty()
            || self.source_id.is_empty()
            || self.source_version.is_empty()
            || self.spool_schema_version.is_empty()
            || self.closed_at_micros < 0
            || self.min_cursor.epoch() != self.max_cursor.epoch()
            || self.min_cursor.offset() > self.max_cursor.offset()
        {
            return Err(SpoolError::InvalidManifest);
        }
        Ok(())
    }
}

#[derive(Debug)]
pub(crate) struct ManifestFields {
    pub segment_sequence: u64,
    pub segment_file: String,
    pub source_id: String,
    pub source_version: String,
    pub spool_schema_version: String,
    pub producer_build_hash: [u8; 32],
    pub file_size_bytes: u64,
    pub record_count: u64,
    pub min_cursor: SourceCursor,
    pub max_cursor: SourceCursor,
    pub segment_blake3: [u8; 32],
    pub previous_manifest_blake3: Option<[u8; 32]>,
    pub closed_at_micros: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloseReceipt {
    manifest: ClosedSegmentManifestV1,
    segment_path: PathBuf,
    manifest_path: PathBuf,
    manifest_hash: [u8; 32],
}

impl CloseReceipt {
    #[must_use]
    pub const fn manifest(&self) -> &ClosedSegmentManifestV1 {
        &self.manifest
    }

    #[must_use]
    pub fn segment_path(&self) -> &Path {
        &self.segment_path
    }

    #[must_use]
    pub fn manifest_path(&self) -> &Path {
        &self.manifest_path
    }

    #[must_use]
    pub const fn manifest_hash(&self) -> [u8; 32] {
        self.manifest_hash
    }

    pub fn verify_current(&self) -> Result<(), SpoolError> {
        let current = load_close_receipt(&self.segment_path)?;
        if current != *self {
            return Err(SpoolError::ManifestContentMismatch);
        }
        Ok(())
    }
}

pub(crate) fn publish_manifest(
    segment_path: &Path,
    manifest: ClosedSegmentManifestV1,
) -> Result<CloseReceipt, SpoolError> {
    let manifest_path = manifest_path_for(segment_path);
    let mut temporary_path = manifest_path.as_os_str().to_owned();
    temporary_path.push(".tmp");
    let temporary_path = PathBuf::from(temporary_path);
    if manifest_path.exists() || temporary_path.exists() {
        return Err(SpoolError::ManifestAlreadyExists);
    }

    let encoded = manifest.encode()?;
    let mut temporary = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary_path)
        .map_err(|source| {
            if source.kind() == std::io::ErrorKind::AlreadyExists {
                SpoolError::ManifestAlreadyExists
            } else {
                io_error("creating a temporary segment manifest", source)
            }
        })?;
    temporary
        .write_all(&encoded)
        .map_err(|source| io_error("writing a temporary segment manifest", source))?;
    temporary
        .sync_all()
        .map_err(|source| io_error("syncing a temporary segment manifest", source))?;
    drop(temporary);

    rustix::fs::renameat_with(
        rustix::fs::CWD,
        &temporary_path,
        rustix::fs::CWD,
        &manifest_path,
        rustix::fs::RenameFlags::NOREPLACE,
    )
    .map_err(|source| {
        if source == rustix::io::Errno::EXIST {
            SpoolError::ManifestAlreadyExists
        } else {
            io_error("publishing a segment manifest", source.into())
        }
    })?;
    sync_directory(manifest_path.parent().ok_or(SpoolError::InvalidManifest)?)?;
    Ok(CloseReceipt {
        manifest,
        segment_path: segment_path.to_owned(),
        manifest_path,
        manifest_hash: *blake3::hash(&encoded).as_bytes(),
    })
}

pub(crate) fn load_close_receipt(
    segment_path: impl AsRef<Path>,
) -> Result<CloseReceipt, SpoolError> {
    let segment_path = segment_path.as_ref();
    let manifest_path = manifest_path_for(segment_path);
    let encoded = fs::read(&manifest_path)
        .map_err(|source| io_error("reading a closed-segment manifest", source))?;
    let manifest: ClosedSegmentManifestV1 =
        serde_json::from_slice(&encoded).map_err(|_| SpoolError::InvalidManifest)?;
    manifest.validate()?;
    if segment_path.file_name().and_then(std::ffi::OsStr::to_str) != Some(manifest.segment_file()) {
        return Err(SpoolError::ManifestContentMismatch);
    }
    super::inspection::verify_manifest_bytes(&manifest, segment_path)?;
    let reader = super::SpoolReader::open(segment_path)?;
    let records = reader.read_all()?;
    super::inspection::verify_manifest_content(&manifest, &reader, &records)?;
    Ok(CloseReceipt {
        manifest,
        segment_path: segment_path.to_owned(),
        manifest_path,
        manifest_hash: *blake3::hash(&encoded).as_bytes(),
    })
}

pub(crate) fn manifest_path_for(segment_path: &Path) -> PathBuf {
    let mut manifest_path = segment_path.as_os_str().to_owned();
    manifest_path.push(".manifest");
    PathBuf::from(manifest_path)
}

fn sync_directory(path: &Path) -> Result<(), SpoolError> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|source| io_error("syncing the spool directory", source))
}

mod hex_32 {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(value: &[u8; 32], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&hex::encode(value))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<[u8; 32], D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        if !is_lower_hex_32(&value) {
            return Err(serde::de::Error::custom(
                "expected 64 lowercase hexadecimal characters",
            ));
        }
        let bytes = hex::decode(&value).map_err(serde::de::Error::custom)?;
        bytes
            .try_into()
            .map_err(|_| serde::de::Error::custom("expected a 32-byte hash"))
    }

    fn is_lower_hex_32(value: &str) -> bool {
        value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    }
}

mod option_hex_32 {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S>(value: &Option<[u8; 32]>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        value.map(hex::encode).serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<[u8; 32]>, D::Error>
    where
        D: Deserializer<'de>,
    {
        Option::<String>::deserialize(deserializer)?
            .map(|value| {
                if value.len() != 64
                    || !value
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                {
                    return Err(serde::de::Error::custom(
                        "expected 64 lowercase hexadecimal characters",
                    ));
                }
                let bytes = hex::decode(value).map_err(serde::de::Error::custom)?;
                bytes
                    .try_into()
                    .map_err(|_| serde::de::Error::custom("expected a 32-byte hash"))
            })
            .transpose()
    }
}
