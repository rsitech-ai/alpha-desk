use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use domain_types::{ManifestId, SourceId};
use hl_protocol::SourceCursor;
use serde::{Deserialize, Serialize};
use storage_ports::LocalRecordSequence;

use crate::spool::{CloseReceipt, SourceSpoolBaseline, SpoolError};

const CHECKPOINT_SCHEMA: &str = "hl.auxiliary-archive-checkpoint.v1";
const CHECKPOINT_FILE: &str = "auxiliary-archive-checkpoint-v1.json";
const CHECKPOINT_TEMP_FILE: &str = "auxiliary-archive-checkpoint-v1.json.tmp";
const MAX_CHECKPOINT_BYTES: u64 = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AuxiliaryArchiveCheckpoint {
    schema_version: String,
    source_id: String,
    source_version: String,
    archive_identity: String,
    segment_sequence: u64,
    segment_file: String,
    segment_blake3: String,
    manifest_file: String,
    manifest_blake3: String,
    raw_manifest_ids: Vec<String>,
    first_local_sequence: u64,
    cursor_epoch: String,
    cursor_offset: u64,
    local_sequence: u64,
    record_count: u64,
    last_received_wall_micros: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    quarantine_reason: Option<String>,
}

impl AuxiliaryArchiveCheckpoint {
    pub(crate) fn load(
        directory: &Path,
        source_id: &SourceId,
        source_version: &str,
        archive_identity: &str,
    ) -> Result<Option<Self>, SpoolError> {
        let path = directory.join(CHECKPOINT_FILE);
        let temporary = directory.join(CHECKPOINT_TEMP_FILE);
        if temporary.exists() {
            if path.exists() {
                fs::remove_file(&temporary).map_err(|source| {
                    crate::spool::io_error("removing stale checkpoint", source)
                })?;
            } else {
                fs::remove_file(&temporary).map_err(|source| {
                    crate::spool::io_error("discarding untrusted checkpoint temp", source)
                })?;
                return Ok(None);
            }
        }
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => {
                return Err(crate::spool::io_error(
                    "reading auxiliary checkpoint metadata",
                    source,
                ));
            }
        };
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || metadata.len() > MAX_CHECKPOINT_BYTES
        {
            return Err(SpoolError::InvalidManifest);
        }
        let encoded = fs::read(&path)
            .map_err(|source| crate::spool::io_error("reading auxiliary checkpoint", source))?;
        let checkpoint: Self =
            serde_json::from_slice(&encoded).map_err(|_| SpoolError::InvalidManifest)?;
        checkpoint.validate(directory, source_id, source_version, archive_identity)?;
        Ok(Some(checkpoint))
    }

    pub(crate) fn publish(
        directory: &Path,
        archive_identity: &str,
        segment: &CloseReceipt,
        raw_manifest_ids: &[ManifestId],
        last_received_wall_micros: i64,
        quarantine_reason: Option<String>,
    ) -> Result<Self, SpoolError> {
        segment.verify_current()?;
        let local_sequence = segment
            .manifest()
            .last_local_sequence()
            .ok_or(SpoolError::InvalidManifest)?;
        let first_local_sequence = segment
            .manifest()
            .first_local_sequence()
            .ok_or(SpoolError::InvalidManifest)?;
        let manifest_file = segment
            .manifest_path()
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .ok_or(SpoolError::InvalidManifest)?
            .to_owned();
        let checkpoint = Self {
            schema_version: CHECKPOINT_SCHEMA.to_owned(),
            source_id: segment.manifest().source_id().to_owned(),
            source_version: segment.manifest().source_version().to_owned(),
            archive_identity: archive_identity.to_owned(),
            segment_sequence: segment.manifest().segment_sequence(),
            segment_file: segment.manifest().segment_file().to_owned(),
            segment_blake3: hex::encode(segment.manifest().segment_blake3()),
            manifest_file,
            manifest_blake3: hex::encode(segment.manifest_hash()),
            raw_manifest_ids: raw_manifest_ids
                .iter()
                .map(|manifest| manifest.as_str().to_owned())
                .collect(),
            first_local_sequence: first_local_sequence.get(),
            cursor_epoch: segment.manifest().max_cursor().epoch().to_owned(),
            cursor_offset: segment.manifest().max_cursor().offset(),
            local_sequence: local_sequence.get(),
            record_count: segment.manifest().record_count(),
            last_received_wall_micros,
            quarantine_reason,
        };
        let source_id = SourceId::new(segment.manifest().source_id().to_owned())
            .map_err(|_| SpoolError::InvalidManifest)?;
        checkpoint.validate(
            directory,
            &source_id,
            segment.manifest().source_version(),
            archive_identity,
        )?;
        let mut encoded =
            serde_json::to_vec_pretty(&checkpoint).map_err(|_| SpoolError::InvalidManifest)?;
        encoded.push(b'\n');
        if u64::try_from(encoded.len()).map_err(|_| SpoolError::SizeOverflow)?
            > MAX_CHECKPOINT_BYTES
        {
            return Err(SpoolError::InvalidManifest);
        }
        let temporary = directory.join(CHECKPOINT_TEMP_FILE);
        let path = directory.join(CHECKPOINT_FILE);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|source| crate::spool::io_error("creating auxiliary checkpoint", source))?;
        file.write_all(&encoded)
            .map_err(|source| crate::spool::io_error("writing auxiliary checkpoint", source))?;
        file.sync_all()
            .map_err(|source| crate::spool::io_error("syncing auxiliary checkpoint", source))?;
        drop(file);
        fs::rename(&temporary, &path)
            .map_err(|source| crate::spool::io_error("publishing auxiliary checkpoint", source))?;
        sync_directory(directory)?;
        Ok(checkpoint)
    }

    pub(crate) fn cleanup_archived_segment(&self, directory: &Path) -> Result<(), SpoolError> {
        self.verify_and_remove(
            &directory.join(&self.manifest_file),
            decode_hash(&self.manifest_blake3)?,
            "removing archived spool manifest",
        )?;
        self.verify_and_remove(
            &directory.join(&self.segment_file),
            decode_hash(&self.segment_blake3)?,
            "removing archived spool segment",
        )?;
        sync_directory(directory)
    }

    fn verify_and_remove(
        &self,
        path: &Path,
        expected_hash: [u8; 32],
        operation: &'static str,
    ) -> Result<(), SpoolError> {
        match fs::symlink_metadata(path) {
            Ok(metadata) => {
                if !metadata.is_file() || metadata.file_type().is_symlink() {
                    return Err(SpoolError::UnsafeSpoolEntry);
                }
                let bytes = fs::read(path).map_err(|source| {
                    crate::spool::io_error("reading archived spool file", source)
                })?;
                if *blake3::hash(&bytes).as_bytes() != expected_hash {
                    return Err(SpoolError::ManifestContentMismatch);
                }
                fs::remove_file(path)
                    .map_err(|source| crate::spool::io_error(operation, source))?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(crate::spool::io_error(
                    "checking archived spool file",
                    source,
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn baseline(&self) -> Result<SourceSpoolBaseline, SpoolError> {
        SourceSpoolBaseline::try_new(
            self.segment_sequence,
            decode_hash(&self.manifest_blake3)?,
            SourceCursor::new(self.cursor_epoch.clone(), self.cursor_offset)
                .map_err(|_| SpoolError::InvalidManifest)?,
            Some(
                LocalRecordSequence::try_new(self.local_sequence)
                    .map_err(|_| SpoolError::InvalidManifest)?,
            ),
        )
    }

    pub(crate) fn raw_manifest_ids(&self) -> Result<Vec<ManifestId>, SpoolError> {
        self.raw_manifest_ids
            .iter()
            .map(|value| ManifestId::new(value.clone()).map_err(|_| SpoolError::InvalidManifest))
            .collect()
    }

    pub(crate) fn spool_manifest_blake3(&self) -> Result<[u8; 32], SpoolError> {
        decode_hash(&self.manifest_blake3)
    }

    pub(crate) fn spool_segment_blake3(&self) -> Result<[u8; 32], SpoolError> {
        decode_hash(&self.segment_blake3)
    }

    pub(crate) fn last_cursor(&self) -> Result<SourceCursor, SpoolError> {
        SourceCursor::new(self.cursor_epoch.clone(), self.cursor_offset)
            .map_err(|_| SpoolError::InvalidManifest)
    }

    pub(crate) fn last_local_sequence(&self) -> Result<LocalRecordSequence, SpoolError> {
        LocalRecordSequence::try_new(self.local_sequence).map_err(|_| SpoolError::InvalidManifest)
    }

    pub(crate) fn first_local_sequence(&self) -> Result<LocalRecordSequence, SpoolError> {
        LocalRecordSequence::try_new(self.first_local_sequence)
            .map_err(|_| SpoolError::InvalidManifest)
    }

    pub(crate) const fn record_count(&self) -> u64 {
        self.record_count
    }

    pub(crate) fn last_received_wall_micros(&self) -> i64 {
        self.last_received_wall_micros
    }

    pub(crate) fn quarantine_reason(&self) -> Option<&str> {
        self.quarantine_reason.as_deref()
    }

    fn validate(
        &self,
        directory: &Path,
        source_id: &SourceId,
        source_version: &str,
        archive_identity: &str,
    ) -> Result<(), SpoolError> {
        if self.schema_version != CHECKPOINT_SCHEMA
            || self.source_id != source_id.as_str()
            || self.source_version != source_version
            || self.archive_identity != archive_identity
            || self.archive_identity.len() != 64
            || hex::decode(&self.archive_identity).map_or(true, |bytes| bytes.len() != 32)
            || self.segment_sequence == 0
            || self.local_sequence == 0
            || self.first_local_sequence == 0
            || self.record_count == 0
            || self.first_local_sequence.checked_add(self.record_count - 1)
                != Some(self.local_sequence)
            || self.raw_manifest_ids.is_empty()
            || self.raw_manifest_ids.len() > 4_096
            || self.last_received_wall_micros < 0
            || self.segment_file != format!("segment-{:010}.hlsp", self.segment_sequence)
            || self.manifest_file != format!("{}.manifest", self.segment_file)
            || Path::new(&self.segment_file).parent() != Some(Path::new(""))
            || Path::new(&self.manifest_file).parent() != Some(Path::new(""))
        {
            return Err(SpoolError::InvalidManifest);
        }
        let _ = directory;
        decode_hash(&self.segment_blake3)?;
        decode_hash(&self.manifest_blake3)?;
        for manifest in &self.raw_manifest_ids {
            ManifestId::new(manifest.clone()).map_err(|_| SpoolError::InvalidManifest)?;
        }
        SourceCursor::new(self.cursor_epoch.clone(), self.cursor_offset)
            .map_err(|_| SpoolError::InvalidManifest)?;
        if let Some(reason) = &self.quarantine_reason {
            crate::status::validate_reason_code(reason).map_err(|_| SpoolError::InvalidManifest)?;
        }
        Ok(())
    }
}

fn decode_hash(value: &str) -> Result<[u8; 32], SpoolError> {
    let bytes = hex::decode(value).map_err(|_| SpoolError::InvalidManifest)?;
    bytes.try_into().map_err(|_| SpoolError::InvalidManifest)
}

fn sync_directory(path: &Path) -> Result<(), SpoolError> {
    let directory = OpenOptions::new()
        .read(true)
        .open(path)
        .map_err(|source| crate::spool::io_error("opening checkpoint directory", source))?;
    directory
        .sync_all()
        .map_err(|source| crate::spool::io_error("syncing checkpoint directory", source))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use domain_types::SourceId;
    use tempfile::TempDir;

    use super::{AuxiliaryArchiveCheckpoint, CHECKPOINT_FILE, CHECKPOINT_TEMP_FILE};
    use crate::spool::SpoolError;

    #[test]
    fn uncommitted_checkpoint_temp_is_discarded_without_advancing_the_baseline() {
        let root = TempDir::new().unwrap();
        let temporary = root.path().join(CHECKPOINT_TEMP_FILE);
        fs::write(&temporary, b"partial checkpoint").unwrap();

        let loaded = AuxiliaryArchiveCheckpoint::load(
            root.path(),
            &SourceId::new("node-fills").unwrap(),
            "hyperliquid-node-v1",
            &"00".repeat(32),
        )
        .unwrap();

        assert!(loaded.is_none());
        assert!(!temporary.exists());
        assert!(!root.path().join(CHECKPOINT_FILE).exists());
    }

    #[test]
    fn corrupt_published_checkpoint_fails_closed() {
        let root = TempDir::new().unwrap();
        fs::write(root.path().join(CHECKPOINT_FILE), b"not-json").unwrap();

        let error = AuxiliaryArchiveCheckpoint::load(
            root.path(),
            &SourceId::new("node-fills").unwrap(),
            "hyperliquid-node-v1",
            &"00".repeat(32),
        )
        .unwrap_err();

        assert!(matches!(error, SpoolError::InvalidManifest));
    }
}
