//! Official historical S3 adapter. Archive exact object bytes before cursor advance.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use canonical_archive::{ArchiveConfig, LocalParquetArchive};
use domain_types::{BlockHeight, ChainId, KnownTime, SourceId};
use hl_protocol::{
    ObservationClass, ObservationError, ReceiveTimestamps, SourceCursor, SourceObservation,
};
use serde::{Deserialize, Serialize};
use storage_ports::{
    ArchiveError, HistoricalBackfillCursor, HistoricalBackfillProgress, HistoricalGapRecord,
    HistoricalGapStatus, HistoricalObjectManifest, HistoricalObjectPlan, ProgressError,
    ProgressRecordDisposition, RawObservationArchive, RawObservationBatch, RawObservationRange,
    RequesterPaysCost,
};

use crate::historical_manifest::{
    DATASET_VERSION, DatasetFormat, DatasetKind, PARSER_BUILD, coverage_block, coverage_event_time,
};

pub use crate::historical_manifest::{HistoricalError, HistoricalFaultPoint};

const SOURCE_VERSION: &str = "official-historical-s3-v1";
const PARSER_SCHEMA: &str = PARSER_BUILD;
const ARCHIVE_BUILD_ID: &str = "hl-capture-historical-s3";
const CHECKPOINT_SCHEMA: &str = "hl.historical-backfill-checkpoint.v1";

type ObjectCoverage = (
    Option<BlockHeight>,
    Option<BlockHeight>,
    Option<KnownTime>,
    Option<KnownTime>,
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestPayer {
    Requester,
}

impl RequestPayer {
    pub fn parse(value: &str) -> Result<Self, HistoricalError> {
        match value {
            "requester" => Ok(Self::Requester),
            _ => Err(HistoricalError::RequesterPaysRequired),
        }
    }
}

pub trait HistoricalFaultInjector: Send + Sync {
    fn check(&self, point: HistoricalFaultPoint) -> Result<(), HistoricalError>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NoHistoricalFaults;

impl HistoricalFaultInjector for NoHistoricalFaults {
    fn check(&self, _point: HistoricalFaultPoint) -> Result<(), HistoricalError> {
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedObject {
    pub key: String,
    pub etag: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectBody {
    pub key: String,
    pub etag: String,
    pub bytes: Bytes,
}

pub trait ObjectStore {
    fn list(
        &self,
        bucket: &str,
        prefix: &str,
        start_key: &str,
        end_key: &str,
        payer: RequestPayer,
    ) -> Result<Vec<ListedObject>, HistoricalError>;

    fn get(
        &self,
        bucket: &str,
        key: &str,
        payer: RequestPayer,
    ) -> Result<Option<ObjectBody>, HistoricalError>;
}

#[derive(Debug, Clone)]
pub struct FsObjectStore {
    root: PathBuf,
}

impl FsObjectStore {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn object_path(&self, bucket: &str, key: &str) -> Result<PathBuf, HistoricalError> {
        if bucket.is_empty()
            || key.is_empty()
            || bucket.contains('/')
            || key.contains("..")
            || Path::new(key).is_absolute()
        {
            return Err(HistoricalError::InvalidRange);
        }
        let mut path = self.root.join(bucket);
        for component in key.split('/') {
            if component.is_empty() || component == "." || component == ".." {
                return Err(HistoricalError::InvalidRange);
            }
            path.push(component);
        }
        Ok(path)
    }
}

impl ObjectStore for FsObjectStore {
    fn list(
        &self,
        bucket: &str,
        prefix: &str,
        start_key: &str,
        end_key: &str,
        payer: RequestPayer,
    ) -> Result<Vec<ListedObject>, HistoricalError> {
        let RequestPayer::Requester = payer;
        if start_key > end_key {
            return Err(HistoricalError::InvalidRange);
        }
        let bucket_root = self.root.join(bucket);
        let mut keys = Vec::new();
        collect_keys(&bucket_root, &bucket_root, &mut keys)?;
        keys.sort();
        keys.into_iter()
            .filter(|key| {
                key.starts_with(prefix) && key.as_str() >= start_key && key.as_str() <= end_key
            })
            .map(|key| {
                let path = self.object_path(bucket, &key)?;
                let bytes = fs::read(&path).map_err(|_| HistoricalError::Store)?;
                Ok(ListedObject {
                    etag: object_etag(&bytes),
                    size: u64::try_from(bytes.len()).map_err(|_| HistoricalError::Store)?,
                    key,
                })
            })
            .collect()
    }

    fn get(
        &self,
        bucket: &str,
        key: &str,
        payer: RequestPayer,
    ) -> Result<Option<ObjectBody>, HistoricalError> {
        let RequestPayer::Requester = payer;
        let path = self.object_path(bucket, key)?;
        match fs::read(&path) {
            Ok(bytes) => Ok(Some(ObjectBody {
                key: key.to_owned(),
                etag: object_etag(&bytes),
                bytes: Bytes::from(bytes),
            })),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(_) => Err(HistoricalError::Store),
        }
    }
}

fn collect_keys(
    bucket_root: &Path,
    directory: &Path,
    keys: &mut Vec<String>,
) -> Result<(), HistoricalError> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err(HistoricalError::Store),
    };
    for entry in entries {
        let entry = entry.map_err(|_| HistoricalError::Store)?;
        let file_type = entry.file_type().map_err(|_| HistoricalError::Store)?;
        let path = entry.path();
        if file_type.is_dir() {
            collect_keys(bucket_root, &path, keys)?;
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        let relative = path
            .strip_prefix(bucket_root)
            .map_err(|_| HistoricalError::Store)?;
        let key = relative
            .components()
            .map(|component| component.as_os_str().to_str().ok_or(HistoricalError::Store))
            .collect::<Result<Vec<_>, _>>()?
            .join("/");
        keys.push(key);
    }
    Ok(())
}

fn object_etag(bytes: &[u8]) -> String {
    hex::encode(blake3::hash(bytes).as_bytes())
}

fn content_hash(bytes: &[u8]) -> [u8; 32] {
    *blake3::hash(bytes).as_bytes()
}

fn archive_epoch(bucket: &str, key: &str) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(bucket.as_bytes());
    hasher.update(&[0]);
    hasher.update(key.as_bytes());
    hex::encode(hasher.finalize().as_bytes())
}

pub trait HistoricalArchive {
    fn put(
        &mut self,
        bucket: &str,
        key: &str,
        body: &[u8],
        received_at: KnownTime,
    ) -> Result<String, HistoricalError>;

    fn get(&self, archive_ref: &str) -> Result<Option<Bytes>, HistoricalError>;
}

pub struct RawPortHistoricalArchive {
    archive: Arc<dyn RawObservationArchive>,
    chain_id: ChainId,
    source_id: SourceId,
    max_payload_bytes: usize,
}

impl std::fmt::Debug for RawPortHistoricalArchive {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RawPortHistoricalArchive")
            .field("chain_id", &self.chain_id)
            .field("source_id", &self.source_id)
            .finish_non_exhaustive()
    }
}

impl RawPortHistoricalArchive {
    #[must_use]
    pub fn new(
        archive: Arc<dyn RawObservationArchive>,
        chain_id: ChainId,
        source_id: SourceId,
        max_payload_bytes: usize,
    ) -> Self {
        Self {
            archive,
            chain_id,
            source_id,
            max_payload_bytes,
        }
    }

    pub fn open(
        root: impl AsRef<Path>,
        chain_id: ChainId,
        source_id: SourceId,
        max_payload_bytes: usize,
    ) -> Result<Self, HistoricalError> {
        let archive = LocalParquetArchive::open(
            root,
            ArchiveConfig::production(ARCHIVE_BUILD_ID).map_err(|_| HistoricalError::Archive)?,
        )
        .map_err(|_| HistoricalError::Archive)?;
        Ok(Self::new(
            Arc::new(archive),
            chain_id,
            source_id,
            max_payload_bytes,
        ))
    }

    fn read_body(&self, archive_ref: &str) -> Result<Option<Bytes>, HistoricalError> {
        let range = RawObservationRange::try_new(archive_ref, 0, 0)
            .map_err(|_| HistoricalError::Archive)?;
        match self
            .archive
            .read_observations(&self.chain_id, &self.source_id, range)
        {
            Ok(mut iterator) => iterator
                .next()
                .transpose()
                .map_err(|_| HistoricalError::Archive)
                .map(|observation| observation.map(|item| item.payload().clone())),
            Err(ArchiveError::RangeUnavailable) => Ok(None),
            Err(_) => Err(HistoricalError::Archive),
        }
    }
}

impl HistoricalArchive for RawPortHistoricalArchive {
    fn put(
        &mut self,
        bucket: &str,
        key: &str,
        body: &[u8],
        received_at: KnownTime,
    ) -> Result<String, HistoricalError> {
        if body.is_empty() {
            return Err(HistoricalError::InvalidRange);
        }
        let archive_ref = archive_epoch(bucket, key);
        if let Some(existing) = self.read_body(&archive_ref)? {
            if existing.as_ref() == body {
                return Ok(archive_ref);
            }
            return Err(HistoricalError::Conflict);
        }
        let observation = SourceObservation::new(
            self.source_id.clone(),
            SOURCE_VERSION,
            ObservationClass::HistoricalBlock,
            SourceCursor::new(archive_ref.clone(), 0).map_err(|_| HistoricalError::Archive)?,
            ReceiveTimestamps::new(received_at.unix_micros(), 0)
                .map_err(|_| HistoricalError::Archive)?,
            PARSER_SCHEMA,
            Bytes::copy_from_slice(body),
            Vec::new(),
            self.max_payload_bytes,
        )
        .map_err(|error| match error {
            ObservationError::EmptyPayload => HistoricalError::InvalidRange,
            _ => HistoricalError::Archive,
        })?;
        let spool_hash = content_hash(body);
        let batch = RawObservationBatch::try_new(
            self.chain_id.clone(),
            vec![observation],
            spool_hash,
            spool_hash,
        )
        .map_err(|_| HistoricalError::Archive)?;
        self.archive
            .append_batch(&batch)
            .map_err(|_| HistoricalError::Archive)?;
        Ok(archive_ref)
    }

    fn get(&self, archive_ref: &str) -> Result<Option<Bytes>, HistoricalError> {
        self.read_body(archive_ref)
    }
}

#[derive(Debug, Default)]
struct DatasetProgress {
    cursor: Option<HistoricalBackfillCursor>,
    objects: BTreeMap<String, HistoricalObjectPlan>,
    gaps: BTreeMap<String, HistoricalGapRecord>,
}

#[derive(Debug)]
pub struct HistoricalProgressStore {
    path: Option<PathBuf>,
    state: Mutex<BTreeMap<String, DatasetProgress>>,
}

impl HistoricalProgressStore {
    pub fn memory() -> Self {
        Self {
            path: None,
            state: Mutex::new(BTreeMap::new()),
        }
    }

    pub fn open(path: impl Into<PathBuf>) -> Result<Self, HistoricalError> {
        let path = path.into();
        let state = match fs::read(&path) {
            Ok(bytes) => decode_checkpoint(&bytes)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(_) => return Err(HistoricalError::Checkpoint),
        };
        Ok(Self {
            path: Some(path),
            state: Mutex::new(state),
        })
    }

    fn lock(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, BTreeMap<String, DatasetProgress>>, HistoricalError> {
        self.state.lock().map_err(|_| HistoricalError::Progress)
    }

    fn persist_locked(
        &self,
        state: &BTreeMap<String, DatasetProgress>,
    ) -> Result<(), HistoricalError> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|_| HistoricalError::Checkpoint)?;
        }
        let encoded = encode_checkpoint(state)?;
        let temporary = path.with_extension("tmp");
        fs::write(&temporary, encoded).map_err(|_| HistoricalError::Checkpoint)?;
        fs::rename(&temporary, path).map_err(|_| HistoricalError::Checkpoint)
    }
}

impl HistoricalBackfillProgress for HistoricalProgressStore {
    fn record_object(
        &self,
        plan: &HistoricalObjectPlan,
    ) -> Result<ProgressRecordDisposition, ProgressError> {
        let mut state = self
            .lock()
            .map_err(|_| ProgressError::Storage("historical progress lock"))?;
        let dataset = state.entry(plan.dataset_id().to_owned()).or_default();
        if let Some(existing) = dataset.objects.get(plan.key()) {
            if existing.content_hash() == plan.content_hash() && existing.etag() == plan.etag() {
                return Ok(ProgressRecordDisposition::IdenticalDuplicate);
            }
            return Err(ProgressError::ConflictingObject);
        }
        dataset.objects.insert(plan.key().to_owned(), plan.clone());
        self.persist_locked(&state)
            .map_err(|_| ProgressError::Storage("historical progress persist"))?;
        Ok(ProgressRecordDisposition::New)
    }

    fn record_gap(
        &self,
        record: &HistoricalGapRecord,
    ) -> Result<ProgressRecordDisposition, ProgressError> {
        let mut state = self
            .lock()
            .map_err(|_| ProgressError::Storage("historical progress lock"))?;
        let dataset = state.entry(record.dataset_id().to_owned()).or_default();
        if dataset.gaps.contains_key(record.key()) {
            return Ok(ProgressRecordDisposition::IdenticalDuplicate);
        }
        dataset.gaps.insert(record.key().to_owned(), record.clone());
        self.persist_locked(&state)
            .map_err(|_| ProgressError::Storage("historical progress persist"))?;
        Ok(ProgressRecordDisposition::New)
    }

    fn persist_cursor(
        &self,
        cursor: &HistoricalBackfillCursor,
    ) -> Result<ProgressRecordDisposition, ProgressError> {
        let mut state = self
            .lock()
            .map_err(|_| ProgressError::Storage("historical progress lock"))?;
        let dataset = state.entry(cursor.dataset_id().to_owned()).or_default();
        let duplicate = dataset.cursor.as_ref() == Some(cursor);
        dataset.cursor = Some(cursor.clone());
        self.persist_locked(&state)
            .map_err(|_| ProgressError::Storage("historical progress persist"))?;
        if duplicate {
            Ok(ProgressRecordDisposition::IdenticalDuplicate)
        } else {
            Ok(ProgressRecordDisposition::New)
        }
    }

    fn load_cursor(
        &self,
        dataset_id: &str,
    ) -> Result<Option<HistoricalBackfillCursor>, ProgressError> {
        let state = self
            .lock()
            .map_err(|_| ProgressError::Storage("historical progress lock"))?;
        Ok(state
            .get(dataset_id)
            .and_then(|dataset| dataset.cursor.clone()))
    }

    fn load_object(
        &self,
        dataset_id: &str,
        key: &str,
    ) -> Result<Option<HistoricalObjectPlan>, ProgressError> {
        let state = self
            .lock()
            .map_err(|_| ProgressError::Storage("historical progress lock"))?;
        Ok(state
            .get(dataset_id)
            .and_then(|dataset| dataset.objects.get(key).cloned()))
    }

    fn load_gaps(&self, dataset_id: &str) -> Result<Vec<HistoricalGapRecord>, ProgressError> {
        let state = self
            .lock()
            .map_err(|_| ProgressError::Storage("historical progress lock"))?;
        Ok(state
            .get(dataset_id)
            .map(|dataset| dataset.gaps.values().cloned().collect())
            .unwrap_or_default())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckpointFile {
    schema_version: String,
    datasets: BTreeMap<String, CheckpointDataset>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckpointDataset {
    cursor: Option<CheckpointCursor>,
    objects: Vec<CheckpointObject>,
    gaps: Vec<CheckpointGap>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckpointCursor {
    last_key: String,
    parser_build: String,
    coverage_start_key: String,
    coverage_end_key: String,
    cursor_version: u64,
    updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckpointObject {
    bucket: String,
    key: String,
    etag: String,
    content_hash: String,
    archive_ref: String,
    byte_count: u64,
    parser_build: String,
    archived_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckpointGap {
    bucket: String,
    key: String,
    recorded_at: i64,
}

fn encode_checkpoint(
    state: &BTreeMap<String, DatasetProgress>,
) -> Result<Vec<u8>, HistoricalError> {
    let mut datasets = BTreeMap::new();
    for (dataset_id, progress) in state {
        datasets.insert(
            dataset_id.clone(),
            CheckpointDataset {
                cursor: progress.cursor.as_ref().map(|cursor| CheckpointCursor {
                    last_key: cursor.last_key().to_owned(),
                    parser_build: cursor.parser_build().to_owned(),
                    coverage_start_key: cursor.coverage_start_key().to_owned(),
                    coverage_end_key: cursor.coverage_end_key().to_owned(),
                    cursor_version: cursor.cursor_version(),
                    updated_at: cursor.updated_at().unix_micros(),
                }),
                objects: progress
                    .objects
                    .values()
                    .map(|plan| CheckpointObject {
                        bucket: plan.bucket().to_owned(),
                        key: plan.key().to_owned(),
                        etag: plan.etag().to_owned(),
                        content_hash: hex::encode(plan.content_hash()),
                        archive_ref: plan.archive_ref().to_owned(),
                        byte_count: plan.byte_count(),
                        parser_build: plan.parser_build().to_owned(),
                        archived_at: plan.archived_at().unix_micros(),
                    })
                    .collect(),
                gaps: progress
                    .gaps
                    .values()
                    .map(|gap| CheckpointGap {
                        bucket: gap.bucket().to_owned(),
                        key: gap.key().to_owned(),
                        recorded_at: gap.recorded_at().unix_micros(),
                    })
                    .collect(),
            },
        );
    }
    serde_json::to_vec_pretty(&CheckpointFile {
        schema_version: CHECKPOINT_SCHEMA.to_owned(),
        datasets,
    })
    .map_err(|_| HistoricalError::Checkpoint)
}

fn decode_checkpoint(bytes: &[u8]) -> Result<BTreeMap<String, DatasetProgress>, HistoricalError> {
    let file: CheckpointFile =
        serde_json::from_slice(bytes).map_err(|_| HistoricalError::Checkpoint)?;
    if file.schema_version != CHECKPOINT_SCHEMA {
        return Err(HistoricalError::Checkpoint);
    }
    let mut state = BTreeMap::new();
    for (dataset_id, dataset) in file.datasets {
        let mut progress = DatasetProgress::default();
        if let Some(cursor) = dataset.cursor {
            progress.cursor = Some(
                HistoricalBackfillCursor::try_new(
                    dataset_id.clone(),
                    cursor.last_key,
                    cursor.parser_build,
                    cursor.coverage_start_key,
                    cursor.coverage_end_key,
                    cursor.cursor_version,
                    KnownTime::from_unix_micros(cursor.updated_at)
                        .map_err(|_| HistoricalError::Checkpoint)?,
                )
                .map_err(|_| HistoricalError::Checkpoint)?,
            );
        }
        for object in dataset.objects {
            let mut hash = [0_u8; 32];
            hex::decode_to_slice(&object.content_hash, &mut hash)
                .map_err(|_| HistoricalError::Checkpoint)?;
            let plan = HistoricalObjectPlan::try_new(
                dataset_id.clone(),
                object.bucket,
                object.key.clone(),
                object.etag,
                hash,
                object.archive_ref,
                object.byte_count,
                object.parser_build,
                KnownTime::from_unix_micros(object.archived_at)
                    .map_err(|_| HistoricalError::Checkpoint)?,
            )
            .map_err(|_| HistoricalError::Checkpoint)?;
            progress.objects.insert(object.key, plan);
        }
        for gap in dataset.gaps {
            let record = HistoricalGapRecord::try_new(
                dataset_id.clone(),
                gap.bucket,
                gap.key.clone(),
                KnownTime::from_unix_micros(gap.recorded_at)
                    .map_err(|_| HistoricalError::Checkpoint)?,
            )
            .map_err(|_| HistoricalError::Checkpoint)?;
            progress.gaps.insert(gap.key, record);
        }
        state.insert(dataset_id, progress);
    }
    Ok(state)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackfillRequest {
    pub dataset: DatasetKind,
    pub format: DatasetFormat,
    pub bucket: String,
    pub keys: Vec<String>,
    pub request_payer: RequestPayer,
    pub imported_at: KnownTime,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackfillReport {
    pub imported: usize,
    pub duplicates: usize,
    pub gaps: usize,
    pub last_key: Option<String>,
    pub parser_build: String,
    pub coverage_start_key: Option<String>,
    pub coverage_end_key: Option<String>,
    pub manifests: Vec<HistoricalObjectManifest>,
}

pub fn import_objects<S, A, P, F>(
    store: &S,
    archive: &mut A,
    progress: &P,
    faults: &F,
    request: &BackfillRequest,
) -> Result<BackfillReport, HistoricalError>
where
    S: ObjectStore,
    A: HistoricalArchive,
    P: HistoricalBackfillProgress,
    F: HistoricalFaultInjector,
{
    request
        .dataset
        .validate_format(request.format)
        .map_err(|_| HistoricalError::FormatMismatch)?;
    request.dataset.accept_bucket(&request.bucket)?;
    let RequestPayer::Requester = request.request_payer;
    if request.keys.is_empty() {
        return Err(HistoricalError::InvalidRange);
    }
    let mut ordered = request.keys.clone();
    ordered.sort();
    ordered.dedup();
    let start_key = ordered
        .first()
        .cloned()
        .ok_or(HistoricalError::InvalidRange)?;
    let end_key = ordered
        .last()
        .cloned()
        .ok_or(HistoricalError::InvalidRange)?;
    let dataset_id = request.dataset.identifier();
    let mut cursor_version = progress
        .load_cursor(dataset_id)
        .map_err(|_| HistoricalError::Progress)?
        .map(|cursor| cursor.cursor_version())
        .unwrap_or(0_u64);

    let mut imported = 0_usize;
    let mut duplicates = 0_usize;
    let mut gaps = 0_usize;
    let mut last_key = None;
    let mut manifests = Vec::new();

    for key in &ordered {
        let listed = store.get(&request.bucket, key, request.request_payer)?;
        let Some(object) = listed else {
            let gap = HistoricalGapRecord::try_new(
                dataset_id,
                request.bucket.clone(),
                key.clone(),
                request.imported_at,
            )
            .map_err(|_| HistoricalError::Progress)?;
            match progress.record_gap(&gap).map_err(map_progress)? {
                ProgressRecordDisposition::New => gaps = gaps.saturating_add(1),
                ProgressRecordDisposition::IdenticalDuplicate => {}
            }
            manifests.push(gap_manifest(request, key)?);
            cursor_version = cursor_version.saturating_add(1);
            last_key = Some(key.clone());
            persist_cursor(
                progress,
                dataset_id,
                key,
                &start_key,
                &end_key,
                cursor_version,
                request.imported_at,
            )?;
            continue;
        };
        let hash = content_hash(&object.bytes);
        let expected_etag = object_etag(&object.bytes);
        if object.etag != expected_etag {
            return Err(HistoricalError::HashMismatch);
        }
        if let Some(existing) = progress
            .load_object(dataset_id, key)
            .map_err(|_| HistoricalError::Progress)?
            && (existing.content_hash() != hash || existing.etag() != object.etag)
        {
            return Err(HistoricalError::Conflict);
        }
        let archive_ref = archive.put(&request.bucket, key, &object.bytes, request.imported_at)?;
        faults.check(HistoricalFaultPoint::AfterArchive)?;
        let byte_count = u64::try_from(object.bytes.len()).map_err(|_| HistoricalError::Store)?;
        let plan = HistoricalObjectPlan::try_new(
            dataset_id,
            request.bucket.clone(),
            key.clone(),
            object.etag.clone(),
            hash,
            archive_ref,
            byte_count,
            PARSER_BUILD,
            request.imported_at,
        )
        .map_err(|_| HistoricalError::Progress)?;
        match progress.record_object(&plan).map_err(map_progress)? {
            ProgressRecordDisposition::New => imported = imported.saturating_add(1),
            ProgressRecordDisposition::IdenticalDuplicate => {
                duplicates = duplicates.saturating_add(1);
            }
        }
        manifests.push(present_manifest(request, key, &object, hash, byte_count)?);
        cursor_version = cursor_version.saturating_add(1);
        last_key = Some(key.clone());
        persist_cursor(
            progress,
            dataset_id,
            key,
            &start_key,
            &end_key,
            cursor_version,
            request.imported_at,
        )?;
    }

    Ok(BackfillReport {
        imported,
        duplicates,
        gaps,
        last_key,
        parser_build: PARSER_BUILD.to_owned(),
        coverage_start_key: Some(start_key),
        coverage_end_key: Some(end_key),
        manifests,
    })
}

fn persist_cursor<P: HistoricalBackfillProgress>(
    progress: &P,
    dataset_id: &str,
    last_key: &str,
    start_key: &str,
    end_key: &str,
    cursor_version: u64,
    imported_at: KnownTime,
) -> Result<(), HistoricalError> {
    let cursor = HistoricalBackfillCursor::try_new(
        dataset_id,
        last_key,
        PARSER_BUILD,
        start_key,
        end_key,
        cursor_version.max(1),
        imported_at,
    )
    .map_err(|_| HistoricalError::Progress)?;
    progress
        .persist_cursor(&cursor)
        .map_err(|_| HistoricalError::Progress)?;
    Ok(())
}

fn map_progress(error: ProgressError) -> HistoricalError {
    match error {
        ProgressError::ConflictingObject => HistoricalError::Conflict,
        _ => HistoricalError::Progress,
    }
}

fn gap_manifest(
    request: &BackfillRequest,
    key: &str,
) -> Result<HistoricalObjectManifest, HistoricalError> {
    let (first_block, last_block, first_event, last_event) = coverage_fields(request.dataset, key)?;
    HistoricalObjectManifest::try_new(
        request.bucket.clone(),
        key,
        None,
        None,
        request.dataset.identifier(),
        DATASET_VERSION,
        0,
        first_block,
        last_block,
        first_event,
        last_event,
        PARSER_BUILD,
        request.imported_at,
        HistoricalGapStatus::MissingObject,
        None,
    )
    .map_err(|_| HistoricalError::Archive)
}

fn present_manifest(
    request: &BackfillRequest,
    key: &str,
    object: &ObjectBody,
    hash: [u8; 32],
    byte_count: u64,
) -> Result<HistoricalObjectManifest, HistoricalError> {
    let (first_block, last_block, first_event, last_event) = coverage_fields(request.dataset, key)?;
    HistoricalObjectManifest::try_new(
        request.bucket.clone(),
        key,
        Some(object.etag.clone()),
        Some(hash),
        request.dataset.identifier(),
        DATASET_VERSION,
        byte_count,
        first_block,
        last_block,
        first_event,
        last_event,
        PARSER_BUILD,
        request.imported_at,
        HistoricalGapStatus::Present,
        Some(RequesterPaysCost::try_new(byte_count).map_err(|_| HistoricalError::Archive)?),
    )
    .map_err(|_| HistoricalError::Archive)
}

fn coverage_fields(dataset: DatasetKind, key: &str) -> Result<ObjectCoverage, HistoricalError> {
    let parts: Vec<&str> = key.split('/').collect();
    match dataset {
        DatasetKind::L2Snapshots => {
            let date = parts.get(1).copied().ok_or(HistoricalError::InvalidRange)?;
            let hour: u8 = parts
                .get(2)
                .copied()
                .ok_or(HistoricalError::InvalidRange)?
                .parse()
                .map_err(|_| HistoricalError::InvalidRange)?;
            let time = coverage_event_time(date, Some(hour))?;
            Ok((None, None, Some(time), Some(time)))
        }
        DatasetKind::AssetContexts => {
            let file = parts.last().copied().ok_or(HistoricalError::InvalidRange)?;
            let date = file
                .strip_suffix(".csv.lz4")
                .ok_or(HistoricalError::InvalidRange)?;
            let time = coverage_event_time(date, None)?;
            Ok((None, None, Some(time), Some(time)))
        }
        DatasetKind::NodeFillsByBlock
        | DatasetKind::NodeFillsLegacy
        | DatasetKind::NodeTradesLegacy
        | DatasetKind::ExplorerBlocks
        | DatasetKind::ReplicaCmds => {
            let date = parts.get(1).copied().ok_or(HistoricalError::InvalidRange)?;
            let name = parts.get(2).copied().ok_or(HistoricalError::InvalidRange)?;
            let time = coverage_event_time(date, None)?;
            let block = coverage_block(name);
            Ok((block, block, Some(time), Some(time)))
        }
    }
}
