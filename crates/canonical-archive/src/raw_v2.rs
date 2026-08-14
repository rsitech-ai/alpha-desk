use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use arrow::{
    array::{BinaryArray, FixedSizeBinaryArray, Int64Array, StringArray, UInt64Array},
    record_batch::RecordBatch,
};
use domain_types::{ChainId, KnownTime, ManifestId, SourceId};
use hl_protocol::{ParseWarning, ReceiveTimestamps, SourceCursor, SourceObservation};
use parquet::{
    arrow::{ArrowWriter, arrow_reader::ParquetRecordBatchReaderBuilder},
    basic::{Compression, ZstdLevel},
    file::{
        metadata::KeyValue,
        properties::{EnabledStatistics, WriterProperties},
    },
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use storage_ports::{
    ArchiveError, CursorPolicy, LocalRecordSequence, LocalRecordSequenceRange,
    RAW_ARCHIVE_MAXIMUM_LOGICAL_MANIFEST_BYTES, RawArchiveObject, RawObservationBatch,
    RawObservationRange, RawObservationReceipt, SequencedRawObservationIterator,
    VerifiedRawManifest,
};

use super::{
    LocalParquetArchive, fs,
    inspection::{ArchiveDataset, ArchiveInspection, InspectedObject},
    manifest::{
        self, CURRENT_POINTER_SCHEMA_V1, CurrentPointerV1, ObjectDescriptorV1, canonical_json,
    },
    raw, schema,
};

const RAW_V2_DATASET: &str = "raw_source_observations_byte_v2";
const RAW_V2_CURSOR_POLICY: &str = "monotonic-byte-offset";
const RAW_BATCH_MANIFEST_SCHEMA_V2: &str = "hyperliquid-alpha-desk/archive-raw-batch-manifest/v2";
const RAW_PARTITION_MANIFEST_SCHEMA_V2: &str =
    "hyperliquid-alpha-desk/archive-raw-partition-manifest/v2";
const RAW_CATALOG_SCHEMA_V2: &str = "hyperliquid-alpha-desk/archive-raw-catalog/v2";
const RAW_V2_ROLLING_HASH_DOMAIN: &[u8] = b"hyperliquid-alpha-desk/raw-rolling-content/v2";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBatchDescriptorV2 {
    chain_id: String,
    source_id: String,
    source_version: String,
    observation_class: String,
    cursor_policy: String,
    cursor_epoch: String,
    start_offset: u64,
    end_offset: u64,
    first_local_sequence: u64,
    last_local_sequence: u64,
    first_received_wall_micros: i64,
    last_received_wall_micros: i64,
    parser_schema_version: String,
    spool_manifest_blake3: String,
    spool_segment_blake3: String,
    rolling_content_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBatchManifestV2 {
    schema: String,
    producer_build_id: String,
    created_at_micros: i64,
    batch: RawBatchDescriptorV2,
    object: ObjectDescriptorV1,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct EmbeddedRawManifestEvidenceV2 {
    pub(super) manifest_id: ManifestId,
    pub(super) canonical_manifest_json: String,
    pub(super) manifest_sha256: [u8; 32],
    pub(super) chain_id: ChainId,
    pub(super) source_id: SourceId,
    pub(super) partition: String,
    pub(super) cursor_epoch: String,
    pub(super) start_offset: u64,
    pub(super) end_offset: u64,
    pub(super) first_local_sequence: u64,
    pub(super) last_local_sequence: u64,
    pub(super) object_sha256: [u8; 32],
    pub(super) row_count: u64,
    pub(super) rolling_content_sha256: [u8; 32],
}

pub(super) fn validate_embedded_manifest_v2(
    canonical_manifest_bytes: Vec<u8>,
    expected_manifest_sha256: [u8; 32],
) -> Result<EmbeddedRawManifestEvidenceV2, ArchiveError> {
    let embedded_manifest_bytes = u64::try_from(canonical_manifest_bytes.len())
        .map_err(|_| ArchiveError::ManifestVerification("embedded raw V2 manifest is too large"))?;
    if canonical_manifest_bytes.is_empty()
        || embedded_manifest_bytes > RAW_ARCHIVE_MAXIMUM_LOGICAL_MANIFEST_BYTES
        || manifest::sha256(&canonical_manifest_bytes) != expected_manifest_sha256
    {
        return Err(ArchiveError::ManifestVerification(
            "embedded raw V2 manifest bytes or hash are invalid",
        ));
    }
    let value: RawBatchManifestV2 = serde_json::from_slice(&canonical_manifest_bytes)
        .map_err(|_| ArchiveError::ManifestVerification("invalid embedded raw V2 manifest"))?;
    validate_batch_manifest(&value)?;
    let first_partition = manifest::partition_for(value.batch.first_received_wall_micros)?;
    let last_partition = manifest::partition_for(value.batch.last_received_wall_micros)?;
    SourceCursor::new(value.batch.cursor_epoch.clone(), value.batch.start_offset)
        .map_err(|_| ArchiveError::ManifestVerification("invalid raw V2 cursor epoch"))?;
    if !valid_producer_build_id(&value.producer_build_id)
        || !valid_manifest_identity(&value.batch.source_version)
        || !valid_manifest_identity(&value.batch.parser_schema_version)
        || value.batch.first_received_wall_micros < 0
        || value.batch.last_received_wall_micros < 0
        || value.batch.first_received_wall_micros > value.batch.last_received_wall_micros
        || first_partition != last_partition
        || Path::new(&value.object.relative_path) != expected_object_relative(&value)?
        || manifest::parse_hash(&value.object.schema_fingerprint_sha256)?
            != schema::raw_schema_fingerprint()?
    {
        return Err(ArchiveError::ManifestVerification(
            "embedded raw V2 manifest semantics are invalid",
        ));
    }
    if canonical_json(&value)? != canonical_manifest_bytes {
        return Err(ArchiveError::ManifestVerification(
            "embedded raw V2 manifest is not canonical",
        ));
    }
    let canonical_manifest_json = String::from_utf8(canonical_manifest_bytes)
        .map_err(|_| ArchiveError::ManifestVerification("raw V2 manifest is not UTF-8"))?;
    Ok(EmbeddedRawManifestEvidenceV2 {
        manifest_id: manifest::manifest_id(expected_manifest_sha256)?,
        canonical_manifest_json,
        manifest_sha256: expected_manifest_sha256,
        chain_id: chain_id(&value.batch.chain_id)?,
        source_id: source_id(&value.batch.source_id)?,
        partition: first_partition,
        cursor_epoch: value.batch.cursor_epoch,
        start_offset: value.batch.start_offset,
        end_offset: value.batch.end_offset,
        first_local_sequence: value.batch.first_local_sequence,
        last_local_sequence: value.batch.last_local_sequence,
        object_sha256: manifest::parse_hash(&value.object.sha256)?,
        row_count: value.object.row_count,
        rolling_content_sha256: manifest::parse_hash(&value.batch.rolling_content_sha256)?,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBatchRefV2 {
    cursor_policy: String,
    cursor_epoch: String,
    start_offset: u64,
    end_offset: u64,
    first_local_sequence: u64,
    last_local_sequence: u64,
    manifest_relative_path: String,
    manifest_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPartitionManifestV2 {
    schema: String,
    chain_id: String,
    source_id: String,
    dataset: String,
    cursor_policy: String,
    partition: String,
    generation: u64,
    producer_build_id: String,
    created_at_micros: i64,
    previous_manifest_sha256: Option<String>,
    batches: Vec<RawBatchRefV2>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPartitionManifestRefV2 {
    partition: String,
    manifest_relative_path: String,
    manifest_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCatalogV2 {
    schema: String,
    chain_id: String,
    source_id: String,
    dataset: String,
    cursor_policy: String,
    generation: u64,
    producer_build_id: String,
    created_at_micros: i64,
    previous_manifest_sha256: Option<String>,
    partitions: BTreeMap<String, RawPartitionManifestRefV2>,
    batches: Vec<RawBatchRefV2>,
}

#[derive(Debug, Clone)]
struct Loaded<T> {
    value: T,
    hash: [u8; 32],
    relative: PathBuf,
}

pub(super) fn append_batch(
    archive: &LocalParquetArchive,
    batch: &RawObservationBatch,
    durable_at: KnownTime,
) -> Result<RawObservationReceipt, ArchiveError> {
    match batch.cursor_policy() {
        CursorPolicy::MonotonicByteOffset => {}
        CursorPolicy::ContiguousNativeOffset => {
            return Err(ArchiveError::InvalidInput(
                "raw V2 archive requires monotonic byte offsets",
            ));
        }
    }
    let first = batch
        .observations()
        .first()
        .ok_or(ArchiveError::InvalidInput("raw observation batch is empty"))?;
    let descriptor = batch_descriptor(batch)?;
    let chain = batch.chain_id();
    let source = first.source_id();
    let _process_lock = fs::open_writer_lock(
        archive.root(),
        &super::raw_policy::writer_lock_relative(chain, source),
    )?;
    super::raw_policy::ensure_append_policy(
        archive.root(),
        chain,
        source,
        super::raw_policy::RawPolicy::MonotonicByteV2,
    )?;
    let current = load_current_catalog(archive, chain, source)?;

    if let Some(existing) = current
        .as_ref()
        .and_then(|catalog| find_exact_ref(&catalog.value, &descriptor))
    {
        let loaded = load_batch_ref(archive.root(), existing)?;
        if loaded.value.batch != descriptor {
            return Err(conflicting_range(source, &descriptor));
        }
        let verified = verify_loaded_batch(archive, &loaded)?;
        return receipt(&loaded.value, loaded.hash, &verified);
    }
    if current.as_ref().is_some_and(|catalog| {
        catalog.value.batches.iter().any(|reference| {
            ranges_overlap(
                reference.first_local_sequence,
                reference.last_local_sequence,
                descriptor.first_local_sequence,
                descriptor.last_local_sequence,
            ) || (reference.cursor_epoch == descriptor.cursor_epoch
                && ranges_overlap(
                    reference.start_offset,
                    reference.end_offset,
                    descriptor.start_offset,
                    descriptor.end_offset,
                ))
        })
    }) {
        return Err(conflicting_range(source, &descriptor));
    }
    if let Some(previous) = current
        .as_ref()
        .and_then(|catalog| catalog.value.batches.last())
    {
        let expected =
            previous
                .last_local_sequence
                .checked_add(1)
                .ok_or(ArchiveError::InvalidInput(
                    "raw V2 local sequence overflows",
                ))?;
        if descriptor.first_local_sequence != expected {
            return Err(ArchiveError::InvalidInput(
                "raw V2 local sequence does not extend the catalog head",
            ));
        }
    }

    let schema_fingerprint = schema::raw_schema_fingerprint()?;
    let object = write_object(archive, batch, &descriptor, schema_fingerprint)?;
    let manifest_value = RawBatchManifestV2 {
        schema: RAW_BATCH_MANIFEST_SCHEMA_V2.to_owned(),
        producer_build_id: archive.config().producer_build_id().to_owned(),
        created_at_micros: durable_at.unix_micros(),
        batch: descriptor,
        object,
    };
    let bytes = canonical_json(&manifest_value)?;
    let hash = manifest::sha256(&bytes);
    let relative = global_manifest_relative(hash);
    fs::publish_immutable(archive.root(), &relative, &bytes)?;
    let partition = publish_partition(
        archive,
        current.as_ref(),
        &manifest_value,
        &relative,
        hash,
        durable_at,
    )?;
    publish_catalog(
        archive,
        current.as_ref(),
        &manifest_value,
        &relative,
        hash,
        partition,
        durable_at,
    )?;
    let loaded = Loaded {
        value: manifest_value,
        hash,
        relative,
    };
    let verified = verify_loaded_batch(archive, &loaded)?;
    receipt(&loaded.value, hash, &verified)
}

pub(super) fn verify_raw_manifest(
    archive: &LocalParquetArchive,
    manifest_id: &ManifestId,
) -> Result<VerifiedRawManifest, ArchiveError> {
    let hash = manifest::hash_from_manifest_id(manifest_id)?;
    let relative = global_manifest_relative(hash);
    let loaded = load_batch_at(archive.root(), &relative)?;
    if loaded.hash != hash || loaded.relative != relative {
        return Err(ArchiveError::ManifestVerification(
            "raw V2 manifest ID does not bind exact bytes and path",
        ));
    }
    verify_loaded_batch(archive, &loaded)
}

pub(super) fn read_observations_by_sequence(
    archive: &LocalParquetArchive,
    chain: &ChainId,
    source: &SourceId,
    range: LocalRecordSequenceRange,
) -> Result<SequencedRawObservationIterator, ArchiveError> {
    if !super::raw_policy::ensure_read_policy(
        archive.root(),
        chain,
        source,
        super::raw_policy::RawPolicy::MonotonicByteV2,
    )? {
        return Err(ArchiveError::RangeUnavailable);
    }
    if range.len() > archive.config().max_read_blocks() {
        return Err(ArchiveError::InvalidInput(
            "raw observation sequence range exceeds configured record limit",
        ));
    }
    let catalog =
        load_current_catalog(archive, chain, source)?.ok_or(ArchiveError::RangeUnavailable)?;
    let mut references = catalog
        .value
        .batches
        .iter()
        .filter(|reference| {
            ranges_overlap(
                reference.first_local_sequence,
                reference.last_local_sequence,
                range.start().get(),
                range.end().get(),
            )
        })
        .cloned()
        .collect::<Vec<_>>();
    references.sort_by_key(|reference| reference.first_local_sequence);

    let mut total_bytes = 0_u64;
    let mut replayed = Vec::new();
    for reference in references {
        let loaded = load_batch_ref(archive.root(), &reference)?;
        total_bytes = total_bytes
            .checked_add(loaded.value.object.size_bytes)
            .ok_or(ArchiveError::InvalidInput(
                "raw observation byte count overflows",
            ))?;
        if total_bytes > archive.config().max_read_bytes() {
            return Err(ArchiveError::InvalidInput(
                "raw observation sequence range exceeds configured byte limit",
            ));
        }
        let (observations, _) = verify_and_decode(archive, &loaded.value)?;
        for (index, observation) in observations.into_iter().enumerate() {
            let advance_by = u64::try_from(index)
                .map_err(|_| ArchiveError::InvalidInput("local record sequence overflows"))?;
            let sequence = LocalRecordSequence::try_new(reference.first_local_sequence)?
                .checked_advance_by(advance_by)?;
            if range.contains(sequence) {
                replayed.push(storage_ports::OwnedSequencedSourceObservation::new(
                    observation,
                    sequence,
                ));
            }
        }
    }
    if u64::try_from(replayed.len()).ok() != Some(range.len()) {
        return Err(ArchiveError::RangeUnavailable);
    }
    for (index, item) in replayed.iter().enumerate() {
        let advance_by = u64::try_from(index)
            .map_err(|_| ArchiveError::InvalidInput("local record sequence overflows"))?;
        if item.local_sequence() != range.start().checked_advance_by(advance_by)? {
            return Err(ArchiveError::RangeUnavailable);
        }
    }
    Ok(Box::new(replayed.into_iter().map(Ok)))
}

pub(super) fn contains_cursor_epoch(
    archive: &LocalParquetArchive,
    chain: &ChainId,
    source: &SourceId,
    cursor_epoch: &str,
) -> Result<bool, ArchiveError> {
    SourceCursor::new(cursor_epoch.to_owned(), 0)
        .map_err(|_| ArchiveError::InvalidInput("raw cursor epoch"))?;
    if !super::raw_policy::ensure_read_policy(
        archive.root(),
        chain,
        source,
        super::raw_policy::RawPolicy::MonotonicByteV2,
    )? {
        return Ok(false);
    }
    let Some(catalog) = load_current_catalog(archive, chain, source)? else {
        return Ok(false);
    };
    Ok(catalog
        .value
        .batches
        .iter()
        .any(|reference| reference.cursor_epoch == cursor_epoch))
}

pub(crate) fn inspect_source(
    archive: &LocalParquetArchive,
    chain: &ChainId,
    source: &SourceId,
) -> Result<Option<ArchiveInspection>, ArchiveError> {
    if !super::raw_policy::ensure_read_policy(
        archive.root(),
        chain,
        source,
        super::raw_policy::RawPolicy::MonotonicByteV2,
    )? {
        return Ok(None);
    }
    let Some(catalog) = load_current_catalog(archive, chain, source)? else {
        return Ok(None);
    };
    let mut observations = 0_u64;
    let mut seen = BTreeSet::new();
    let mut objects = Vec::new();
    for reference in &catalog.value.batches {
        let loaded = load_batch_ref(archive.root(), reference)?;
        if !seen.insert(loaded.hash) {
            return Err(ArchiveError::ManifestVerification(
                "raw V2 catalog references a batch manifest more than once",
            ));
        }
        let verified = verify_loaded_batch(archive, &loaded)?;
        observations = observations
            .checked_add(verified.object().row_count())
            .ok_or(ArchiveError::InvalidInput(
                "raw V2 inspection observation count overflows",
            ))?;
        objects.push(InspectedObject::new(
            ArchiveDataset::RawSourceObservations,
            verified.object().relative_path().to_path_buf(),
            verified.object().sha256(),
            verified.object().size_bytes(),
            verified.object().row_count(),
        ));
    }
    Ok(Some(ArchiveInspection::raw(observations, objects)))
}

fn batch_descriptor(batch: &RawObservationBatch) -> Result<RawBatchDescriptorV2, ArchiveError> {
    let first = batch
        .observations()
        .first()
        .ok_or(ArchiveError::InvalidInput("raw observation batch is empty"))?;
    let last = batch
        .observations()
        .last()
        .ok_or(ArchiveError::InvalidInput("raw observation batch is empty"))?;
    let sequence_range = batch
        .local_sequence_range()
        .ok_or(ArchiveError::InvalidInput(
            "raw V2 batch is missing local sequence evidence",
        ))?;
    let partition = manifest::partition_for(first.received().wall_micros())?;
    for observation in batch.observations().iter().skip(1) {
        if manifest::partition_for(observation.received().wall_micros())? != partition {
            return Err(ArchiveError::InvalidInput(
                "raw observation batch crosses an hour partition",
            ));
        }
    }
    Ok(RawBatchDescriptorV2 {
        chain_id: batch.chain_id().as_str().to_owned(),
        source_id: first.source_id().as_str().to_owned(),
        source_version: first.source_version().to_owned(),
        observation_class: raw::observation_class_name(first.observation_class())?,
        cursor_policy: RAW_V2_CURSOR_POLICY.to_owned(),
        cursor_epoch: first.cursor().epoch().to_owned(),
        start_offset: first.cursor().offset(),
        end_offset: last.cursor().offset(),
        first_local_sequence: sequence_range.start().get(),
        last_local_sequence: sequence_range.end().get(),
        first_received_wall_micros: first.received().wall_micros(),
        last_received_wall_micros: last.received().wall_micros(),
        parser_schema_version: first.parser_schema_version().to_owned(),
        spool_manifest_blake3: hex::encode(batch.spool_manifest_blake3()),
        spool_segment_blake3: hex::encode(batch.spool_segment_blake3()),
        rolling_content_sha256: hex::encode(rolling_content_hash(batch)?),
    })
}

fn rolling_content_hash(batch: &RawObservationBatch) -> Result<[u8; 32], ArchiveError> {
    let mut hasher = Sha256::new();
    hash_frame(&mut hasher, RAW_V2_ROLLING_HASH_DOMAIN)?;
    hash_frame(&mut hasher, RAW_V2_CURSOR_POLICY.as_bytes())?;
    let sequenced = batch
        .sequenced_observations()
        .ok_or(ArchiveError::InvalidInput(
            "raw V2 batch is missing local sequence evidence",
        ))?;
    for item in sequenced {
        let item = item?;
        let observation = item.observation();
        hash_frame(&mut hasher, &item.local_sequence().get().to_be_bytes())?;
        hash_frame(&mut hasher, batch.chain_id().as_str().as_bytes())?;
        hash_frame(&mut hasher, observation.source_id().as_str().as_bytes())?;
        hash_frame(&mut hasher, observation.source_version().as_bytes())?;
        hash_frame(
            &mut hasher,
            raw::observation_class_name(observation.observation_class())?.as_bytes(),
        )?;
        hash_frame(&mut hasher, observation.cursor().epoch().as_bytes())?;
        hash_frame(&mut hasher, &observation.cursor().offset().to_be_bytes())?;
        hash_frame(
            &mut hasher,
            &observation.received().wall_micros().to_be_bytes(),
        )?;
        hash_frame(
            &mut hasher,
            &observation.received().monotonic_nanos().to_be_bytes(),
        )?;
        hash_frame(&mut hasher, observation.parser_schema_version().as_bytes())?;
        hash_frame(&mut hasher, observation.content_hash().as_bytes())?;
        hash_frame(
            &mut hasher,
            &serde_json::to_vec(observation.warnings())
                .map_err(|_| ArchiveError::Codec("serializing parse warnings".into()))?,
        )?;
        hash_frame(&mut hasher, observation.payload())?;
    }
    Ok(hasher.finalize().into())
}

fn hash_frame(hasher: &mut Sha256, bytes: &[u8]) -> Result<(), ArchiveError> {
    let length = u64::try_from(bytes.len())
        .map_err(|_| ArchiveError::InvalidInput("raw observation field exceeds u64"))?;
    hasher.update(length.to_be_bytes());
    hasher.update(bytes);
    Ok(())
}

fn write_object(
    archive: &LocalParquetArchive,
    batch: &RawObservationBatch,
    descriptor: &RawBatchDescriptorV2,
    schema_fingerprint: [u8; 32],
) -> Result<ObjectDescriptorV1, ArchiveError> {
    let source = source_id(&descriptor.source_id)?;
    let chain = chain_id(&descriptor.chain_id)?;
    let partition = manifest::partition_for(descriptor.first_received_wall_micros)?;
    let parent = dataset_relative(&chain, &source)
        .join(partition)
        .join("objects")
        .join(format!(
            "epoch={}",
            manifest::encoded_component(&descriptor.cursor_epoch)
        ))
        .join(format!(
            "sequences={}-{}",
            descriptor.first_local_sequence, descriptor.last_local_sequence
        ))
        .join(format!(
            "offsets={}-{}",
            descriptor.start_offset, descriptor.end_offset
        ));
    let mut staged = fs::create_parquet_staging_file(archive.root(), &parent)?;
    let record_batch = raw::raw_record_batch(batch)?;
    let compression = ZstdLevel::try_new(3)
        .map_err(|_| ArchiveError::InvalidInput("invalid Parquet compression level"))?;
    let properties = WriterProperties::builder()
        .set_created_by("hyperliquid-alpha-desk/raw-archive-writer-v2".to_owned())
        .set_compression(Compression::ZSTD(compression))
        .set_statistics_enabled(EnabledStatistics::Page)
        .set_key_value_metadata(Some(vec![
            KeyValue::new("alpha_desk.dataset".to_owned(), RAW_V2_DATASET.to_owned()),
            KeyValue::new(
                "alpha_desk.cursor_policy".to_owned(),
                RAW_V2_CURSOR_POLICY.to_owned(),
            ),
            KeyValue::new(
                "alpha_desk.schema_fingerprint_sha256".to_owned(),
                hex::encode(schema_fingerprint),
            ),
        ]))
        .build();
    {
        let mut writer =
            ArrowWriter::try_new(staged.as_file_mut(), schema::raw_schema(), Some(properties))
                .map_err(|error| ArchiveError::Codec(error.to_string()))?;
        writer
            .write(&record_batch)
            .map_err(|error| ArchiveError::Codec(error.to_string()))?;
        writer
            .close()
            .map_err(|error| ArchiveError::Codec(error.to_string()))?;
    }
    staged
        .as_file_mut()
        .sync_all()
        .map_err(|_| ArchiveError::Io("syncing raw V2 Parquet object"))?;
    let (hash, size_bytes) = raw::hash_file(staged.as_file_mut())?;
    let relative = parent.join(format!("part-{}.parquet", hex::encode(hash)));
    fs::publish_staged_immutable(archive.root(), &relative, staged)?;
    let published = fs::read_regular(archive.root(), &relative, size_bytes)?;
    if u64::try_from(published.len()).ok() != Some(size_bytes)
        || <[u8; 32]>::from(Sha256::digest(&published)) != hash
    {
        return Err(ArchiveError::CorruptObject(raw::path_string(&relative)?));
    }
    Ok(ObjectDescriptorV1 {
        relative_path: raw::path_string(&relative)?,
        sha256: hex::encode(hash),
        size_bytes,
        row_count: u64::try_from(batch.observations().len())
            .map_err(|_| ArchiveError::InvalidInput("raw row count exceeds u64"))?,
        schema_fingerprint_sha256: hex::encode(schema_fingerprint),
    })
}

fn publish_partition(
    archive: &LocalParquetArchive,
    catalog: Option<&Loaded<RawCatalogV2>>,
    batch: &RawBatchManifestV2,
    batch_relative: &Path,
    batch_hash: [u8; 32],
    durable_at: KnownTime,
) -> Result<RawPartitionManifestRefV2, ArchiveError> {
    let source = source_id(&batch.batch.source_id)?;
    let chain = chain_id(&batch.batch.chain_id)?;
    let dataset = dataset_relative(&chain, &source);
    let partition = manifest::partition_for(batch.batch.first_received_wall_micros)?;
    let previous = catalog
        .and_then(|loaded| loaded.value.partitions.get(&partition))
        .map(|reference| load_partition_ref(archive.root(), &dataset, &chain, &source, reference))
        .transpose()?;
    let mut batches = previous
        .as_ref()
        .map(|loaded| loaded.value.batches.clone())
        .unwrap_or_default();
    batches.push(raw_batch_ref(batch, batch_relative, batch_hash)?);
    batches.sort_by_key(|reference| reference.first_local_sequence);
    validate_partition_refs(&batches)?;
    let generation = next_generation(
        previous.as_ref().map(|loaded| loaded.value.generation),
        "raw V2 partition generation overflows",
    )?;
    let value = RawPartitionManifestV2 {
        schema: RAW_PARTITION_MANIFEST_SCHEMA_V2.to_owned(),
        chain_id: chain.as_str().to_owned(),
        source_id: source.as_str().to_owned(),
        dataset: RAW_V2_DATASET.to_owned(),
        cursor_policy: RAW_V2_CURSOR_POLICY.to_owned(),
        partition: partition.clone(),
        generation,
        producer_build_id: archive.config().producer_build_id().to_owned(),
        created_at_micros: durable_at.unix_micros(),
        previous_manifest_sha256: previous.as_ref().map(|loaded| hex::encode(loaded.hash)),
        batches,
    };
    validate_partition(&value, &chain, &source, &partition)?;
    let bytes = canonical_json(&value)?;
    let hash = manifest::sha256(&bytes);
    let relative = partition_manifest_relative(&dataset, &partition, hash);
    fs::publish_immutable(archive.root(), &relative, &bytes)?;
    Ok(RawPartitionManifestRefV2 {
        partition,
        manifest_relative_path: raw::path_string(&relative)?,
        manifest_sha256: hex::encode(hash),
    })
}

fn publish_catalog(
    archive: &LocalParquetArchive,
    previous: Option<&Loaded<RawCatalogV2>>,
    batch: &RawBatchManifestV2,
    batch_relative: &Path,
    batch_hash: [u8; 32],
    partition: RawPartitionManifestRefV2,
    durable_at: KnownTime,
) -> Result<(), ArchiveError> {
    let source = source_id(&batch.batch.source_id)?;
    let chain = chain_id(&batch.batch.chain_id)?;
    let dataset = dataset_relative(&chain, &source);
    let mut batches = previous
        .map(|loaded| loaded.value.batches.clone())
        .unwrap_or_default();
    batches.push(raw_batch_ref(batch, batch_relative, batch_hash)?);
    batches.sort_by_key(|reference| reference.first_local_sequence);
    validate_catalog_refs(&batches)?;
    let generation = next_generation(
        previous.map(|loaded| loaded.value.generation),
        "raw V2 catalog generation overflows",
    )?;
    let mut partitions = previous
        .map(|loaded| loaded.value.partitions.clone())
        .unwrap_or_default();
    partitions.insert(partition.partition.clone(), partition);
    let value = RawCatalogV2 {
        schema: RAW_CATALOG_SCHEMA_V2.to_owned(),
        chain_id: chain.as_str().to_owned(),
        source_id: source.as_str().to_owned(),
        dataset: RAW_V2_DATASET.to_owned(),
        cursor_policy: RAW_V2_CURSOR_POLICY.to_owned(),
        generation,
        producer_build_id: archive.config().producer_build_id().to_owned(),
        created_at_micros: durable_at.unix_micros(),
        previous_manifest_sha256: previous.map(|loaded| hex::encode(loaded.hash)),
        partitions,
        batches,
    };
    validate_catalog(&value, &chain, &source)?;
    let bytes = canonical_json(&value)?;
    let hash = manifest::sha256(&bytes);
    let relative = catalog_manifest_relative(&dataset, hash);
    fs::publish_immutable(archive.root(), &relative, &bytes)?;
    let pointer = CurrentPointerV1 {
        schema: CURRENT_POINTER_SCHEMA_V1.to_owned(),
        manifest_relative_path: raw::path_string(&relative)?,
        manifest_sha256: hex::encode(hash),
    };
    fs::publish_current(
        archive.root(),
        &dataset.join("CURRENT"),
        &canonical_json(&pointer)?,
    )
}

fn load_current_catalog(
    archive: &LocalParquetArchive,
    chain: &ChainId,
    source: &SourceId,
) -> Result<Option<Loaded<RawCatalogV2>>, ArchiveError> {
    let dataset = dataset_relative(chain, source);
    let current = dataset.join("CURRENT");
    let bytes = match fs::read_manifest(archive.root(), &current) {
        Ok(bytes) => bytes,
        Err(ArchiveError::Io(_)) if !archive.root().join(&current).exists() => return Ok(None),
        Err(error) => return Err(error),
    };
    let pointer: CurrentPointerV1 = serde_json::from_slice(&bytes)
        .map_err(|_| ArchiveError::ManifestVerification("invalid raw V2 current pointer JSON"))?;
    if pointer.schema != CURRENT_POINTER_SCHEMA_V1 {
        return Err(ArchiveError::ManifestVerification(
            "unsupported raw V2 current pointer schema",
        ));
    }
    let hash = manifest::parse_hash(&pointer.manifest_sha256)?;
    let expected = catalog_manifest_relative(&dataset, hash);
    if Path::new(&pointer.manifest_relative_path) != expected {
        return Err(ArchiveError::ManifestVerification(
            "raw V2 current pointer does not bind exact catalog path",
        ));
    }
    let loaded = load_catalog_at(archive.root(), &expected)?;
    if loaded.hash != hash {
        return Err(ArchiveError::ManifestVerification(
            "raw V2 current pointer hash mismatch",
        ));
    }
    validate_catalog(&loaded.value, chain, source)?;
    verify_catalog_chain(archive.root(), &dataset, &loaded)?;
    verify_catalog_partitions(archive.root(), &dataset, &loaded)?;
    Ok(Some(loaded))
}

fn verify_catalog_chain(
    root: &Path,
    dataset: &Path,
    head: &Loaded<RawCatalogV2>,
) -> Result<(), ArchiveError> {
    let mut seen = BTreeSet::new();
    let mut current = head.clone();
    loop {
        if !seen.insert(current.hash) {
            return Err(ArchiveError::ManifestVerification(
                "raw V2 catalog chain contains a cycle",
            ));
        }
        let Some(previous_hash) = current.value.previous_manifest_sha256.as_deref() else {
            return if current.value.generation == 1 {
                Ok(())
            } else {
                Err(ArchiveError::ManifestVerification(
                    "raw V2 catalog root generation is invalid",
                ))
            };
        };
        let previous_hash = manifest::parse_hash(previous_hash)?;
        let previous = load_catalog_at(root, &catalog_manifest_relative(dataset, previous_hash))?;
        let chain = chain_id(&current.value.chain_id)?;
        let source = source_id(&current.value.source_id)?;
        validate_catalog(&previous.value, &chain, &source)?;
        if previous.hash != previous_hash
            || previous.value.generation.checked_add(1) != Some(current.value.generation)
            || !extends_at_tail(&previous.value.batches, &current.value.batches)
        {
            return Err(ArchiveError::ManifestVerification(
                "raw V2 catalog chain is not append-only",
            ));
        }
        verify_partition_transition(root, dataset, &previous.value, &current.value)?;
        current = previous;
    }
}

fn verify_partition_transition(
    root: &Path,
    dataset: &Path,
    previous: &RawCatalogV2,
    current: &RawCatalogV2,
) -> Result<(), ArchiveError> {
    let chain = chain_id(&current.chain_id)?;
    let source = source_id(&current.source_id)?;
    let mut changes = 0_u64;
    for (partition, prior_ref) in &previous.partitions {
        let next_ref =
            current
                .partitions
                .get(partition)
                .ok_or(ArchiveError::ManifestVerification(
                    "raw V2 catalog removed a partition",
                ))?;
        if prior_ref == next_ref {
            continue;
        }
        changes = changes.checked_add(1).ok_or(ArchiveError::InvalidInput(
            "raw V2 catalog partition change count overflows",
        ))?;
        let prior_hash = manifest::parse_hash(&prior_ref.manifest_sha256)?;
        let next = load_partition_ref(root, dataset, &chain, &source, next_ref)?;
        if next.value.previous_manifest_sha256.as_deref() != Some(hex::encode(prior_hash).as_str())
        {
            return Err(ArchiveError::ManifestVerification(
                "raw V2 partition update does not extend prior generation",
            ));
        }
    }
    for (partition, reference) in &current.partitions {
        if previous.partitions.contains_key(partition) {
            continue;
        }
        changes = changes.checked_add(1).ok_or(ArchiveError::InvalidInput(
            "raw V2 catalog partition change count overflows",
        ))?;
        let added = load_partition_ref(root, dataset, &chain, &source, reference)?;
        if added.value.generation != 1 || added.value.previous_manifest_sha256.is_some() {
            return Err(ArchiveError::ManifestVerification(
                "new raw V2 partition does not begin at generation one",
            ));
        }
    }
    if changes != 1 {
        return Err(ArchiveError::ManifestVerification(
            "raw V2 catalog transition must change exactly one partition",
        ));
    }
    Ok(())
}

fn verify_catalog_partitions(
    root: &Path,
    dataset: &Path,
    catalog: &Loaded<RawCatalogV2>,
) -> Result<(), ArchiveError> {
    let chain = chain_id(&catalog.value.chain_id)?;
    let source = source_id(&catalog.value.source_id)?;
    let mut batches = Vec::new();
    for (partition, reference) in &catalog.value.partitions {
        if reference.partition != *partition {
            return Err(ArchiveError::ManifestVerification(
                "raw V2 catalog partition key and reference disagree",
            ));
        }
        let loaded = load_partition_ref(root, dataset, &chain, &source, reference)?;
        verify_partition_chain(root, dataset, &chain, &source, &loaded)?;
        for batch_ref in &loaded.value.batches {
            let batch = load_batch_ref(root, batch_ref)?;
            if batch.value.batch.chain_id != chain.as_str()
                || batch.value.batch.source_id != source.as_str()
                || manifest::partition_for(batch.value.batch.first_received_wall_micros)?
                    != *partition
            {
                return Err(ArchiveError::ManifestVerification(
                    "raw V2 partition references a batch outside its identity or hour",
                ));
            }
        }
        batches.extend(loaded.value.batches);
    }
    batches.sort_by_key(|reference| reference.first_local_sequence);
    if batches != catalog.value.batches {
        return Err(ArchiveError::ManifestVerification(
            "raw V2 catalog batches disagree with partition manifests",
        ));
    }
    Ok(())
}

fn verify_partition_chain(
    root: &Path,
    dataset: &Path,
    chain: &ChainId,
    source: &SourceId,
    head: &Loaded<RawPartitionManifestV2>,
) -> Result<(), ArchiveError> {
    let mut seen = BTreeSet::new();
    let mut current = head.clone();
    loop {
        if !seen.insert(current.hash) {
            return Err(ArchiveError::ManifestVerification(
                "raw V2 partition chain contains a cycle",
            ));
        }
        let Some(previous_hash) = current.value.previous_manifest_sha256.as_deref() else {
            return if current.value.generation == 1 {
                Ok(())
            } else {
                Err(ArchiveError::ManifestVerification(
                    "raw V2 partition root generation is invalid",
                ))
            };
        };
        let previous_hash = manifest::parse_hash(previous_hash)?;
        let previous = load_partition_at(
            root,
            &partition_manifest_relative(dataset, &current.value.partition, previous_hash),
        )?;
        validate_partition(&previous.value, chain, source, &current.value.partition)?;
        if previous.hash != previous_hash
            || previous.value.generation.checked_add(1) != Some(current.value.generation)
            || !extends_at_tail(&previous.value.batches, &current.value.batches)
        {
            return Err(ArchiveError::ManifestVerification(
                "raw V2 partition chain is not append-only",
            ));
        }
        current = previous;
    }
}

fn validate_catalog(
    value: &RawCatalogV2,
    chain: &ChainId,
    source: &SourceId,
) -> Result<(), ArchiveError> {
    if value.schema != RAW_CATALOG_SCHEMA_V2
        || value.chain_id != chain.as_str()
        || value.source_id != source.as_str()
        || value.dataset != RAW_V2_DATASET
        || value.cursor_policy != RAW_V2_CURSOR_POLICY
        || value.generation == 0
        || value.producer_build_id.is_empty()
        || value.created_at_micros < 0
        || value.partitions.is_empty()
        || value.batches.is_empty()
    {
        return Err(ArchiveError::ManifestVerification(
            "raw V2 catalog metadata is invalid",
        ));
    }
    validate_generation(value.generation, value.previous_manifest_sha256.as_deref())?;
    let dataset = dataset_relative(chain, source);
    for (partition, reference) in &value.partitions {
        validate_partition_name(partition)?;
        let hash = manifest::parse_hash(&reference.manifest_sha256)?;
        if reference.partition != *partition
            || Path::new(&reference.manifest_relative_path)
                != partition_manifest_relative(&dataset, partition, hash)
        {
            return Err(ArchiveError::ManifestVerification(
                "raw V2 catalog partition reference is invalid",
            ));
        }
    }
    validate_catalog_refs(&value.batches)
}

fn validate_partition(
    value: &RawPartitionManifestV2,
    chain: &ChainId,
    source: &SourceId,
    partition: &str,
) -> Result<(), ArchiveError> {
    if value.schema != RAW_PARTITION_MANIFEST_SCHEMA_V2
        || value.chain_id != chain.as_str()
        || value.source_id != source.as_str()
        || value.dataset != RAW_V2_DATASET
        || value.cursor_policy != RAW_V2_CURSOR_POLICY
        || value.partition != partition
        || value.generation == 0
        || value.producer_build_id.is_empty()
        || value.created_at_micros < 0
        || value.batches.is_empty()
    {
        return Err(ArchiveError::ManifestVerification(
            "raw V2 partition manifest metadata is invalid",
        ));
    }
    validate_partition_name(partition)?;
    validate_generation(value.generation, value.previous_manifest_sha256.as_deref())?;
    validate_partition_refs(&value.batches)
}

fn validate_generation(generation: u64, previous: Option<&str>) -> Result<(), ArchiveError> {
    if let Some(previous) = previous {
        manifest::parse_hash(previous)?;
    }
    if (generation == 1) != previous.is_none() {
        return Err(ArchiveError::ManifestVerification(
            "raw V2 generation and previous hash disagree",
        ));
    }
    Ok(())
}

fn validate_partition_name(partition: &str) -> Result<(), ArchiveError> {
    let timestamp = format!("{partition}:00:00");
    if chrono::NaiveDateTime::parse_from_str(&timestamp, "date=%Y-%m-%d/hour=%H:%M:%S").is_err()
        || fs::validate_relative(Path::new(partition)).is_err()
    {
        return Err(ArchiveError::ManifestVerification(
            "raw V2 partition name is invalid",
        ));
    }
    Ok(())
}

fn validate_catalog_refs(references: &[RawBatchRefV2]) -> Result<(), ArchiveError> {
    validate_refs(references, true)
}

fn validate_partition_refs(references: &[RawBatchRefV2]) -> Result<(), ArchiveError> {
    validate_refs(references, false)
}

fn validate_refs(
    references: &[RawBatchRefV2],
    require_contiguous: bool,
) -> Result<(), ArchiveError> {
    let mut previous: Option<&RawBatchRefV2> = None;
    for reference in references {
        if reference.cursor_policy != RAW_V2_CURSOR_POLICY
            || reference.cursor_epoch.is_empty()
            || reference.start_offset > reference.end_offset
            || reference.first_local_sequence == 0
            || reference.first_local_sequence > reference.last_local_sequence
        {
            return Err(ArchiveError::ManifestVerification(
                "raw V2 batch reference is invalid",
            ));
        }
        manifest::parse_hash(&reference.manifest_sha256)?;
        fs::validate_relative(Path::new(&reference.manifest_relative_path))?;
        if let Some(prior) = previous {
            let expected = prior.last_local_sequence.checked_add(1).ok_or(
                ArchiveError::ManifestVerification("raw V2 local sequence overflows"),
            )?;
            if prior.last_local_sequence >= reference.first_local_sequence
                || (require_contiguous && expected != reference.first_local_sequence)
            {
                return Err(ArchiveError::ManifestVerification(
                    "raw V2 sequence references overlap, contain a gap, or are not ordered",
                ));
            }
        }
        previous = Some(reference);
    }
    for (index, reference) in references.iter().enumerate() {
        if references[..index].iter().any(|prior| {
            prior.cursor_epoch == reference.cursor_epoch
                && ranges_overlap(
                    prior.start_offset,
                    prior.end_offset,
                    reference.start_offset,
                    reference.end_offset,
                )
        }) {
            return Err(ArchiveError::ManifestVerification(
                "raw V2 native cursor references overlap",
            ));
        }
    }
    Ok(())
}

fn load_catalog_at(root: &Path, relative: &Path) -> Result<Loaded<RawCatalogV2>, ArchiveError> {
    let bytes = fs::read_manifest(root, relative)?;
    let hash = manifest::sha256(&bytes);
    let value = serde_json::from_slice(&bytes)
        .map_err(|_| ArchiveError::ManifestVerification("invalid raw V2 catalog JSON"))?;
    Ok(Loaded {
        value,
        hash,
        relative: relative.to_path_buf(),
    })
}

fn load_partition_ref(
    root: &Path,
    dataset: &Path,
    chain: &ChainId,
    source: &SourceId,
    reference: &RawPartitionManifestRefV2,
) -> Result<Loaded<RawPartitionManifestV2>, ArchiveError> {
    let hash = manifest::parse_hash(&reference.manifest_sha256)?;
    let expected = partition_manifest_relative(dataset, &reference.partition, hash);
    if Path::new(&reference.manifest_relative_path) != expected {
        return Err(ArchiveError::ManifestVerification(
            "raw V2 partition reference does not bind exact path",
        ));
    }
    let loaded = load_partition_at(root, &expected)?;
    if loaded.hash != hash {
        return Err(ArchiveError::ManifestVerification(
            "raw V2 partition reference hash mismatch",
        ));
    }
    validate_partition(&loaded.value, chain, source, &reference.partition)?;
    Ok(loaded)
}

fn load_partition_at(
    root: &Path,
    relative: &Path,
) -> Result<Loaded<RawPartitionManifestV2>, ArchiveError> {
    let bytes = fs::read_manifest(root, relative)?;
    let hash = manifest::sha256(&bytes);
    let value = serde_json::from_slice(&bytes).map_err(|_| {
        ArchiveError::ManifestVerification("invalid raw V2 partition manifest JSON")
    })?;
    Ok(Loaded {
        value,
        hash,
        relative: relative.to_path_buf(),
    })
}

fn load_batch_ref(
    root: &Path,
    reference: &RawBatchRefV2,
) -> Result<Loaded<RawBatchManifestV2>, ArchiveError> {
    let hash = manifest::parse_hash(&reference.manifest_sha256)?;
    let expected = global_manifest_relative(hash);
    if Path::new(&reference.manifest_relative_path) != expected {
        return Err(ArchiveError::ManifestVerification(
            "raw V2 batch reference does not bind exact path",
        ));
    }
    let loaded = load_batch_at(root, &expected)?;
    if loaded.hash != hash
        || loaded.value.batch.cursor_policy != reference.cursor_policy
        || loaded.value.batch.cursor_epoch != reference.cursor_epoch
        || loaded.value.batch.start_offset != reference.start_offset
        || loaded.value.batch.end_offset != reference.end_offset
        || loaded.value.batch.first_local_sequence != reference.first_local_sequence
        || loaded.value.batch.last_local_sequence != reference.last_local_sequence
    {
        return Err(ArchiveError::ManifestVerification(
            "raw V2 batch reference content mismatch",
        ));
    }
    Ok(loaded)
}

fn load_batch_at(root: &Path, relative: &Path) -> Result<Loaded<RawBatchManifestV2>, ArchiveError> {
    let bytes = fs::read_manifest(root, relative)?;
    let hash = manifest::sha256(&bytes);
    let value = serde_json::from_slice(&bytes)
        .map_err(|_| ArchiveError::ManifestVerification("invalid raw V2 batch manifest JSON"))?;
    validate_batch_manifest(&value)?;
    Ok(Loaded {
        value,
        hash,
        relative: relative.to_path_buf(),
    })
}

fn validate_batch_manifest(value: &RawBatchManifestV2) -> Result<(), ArchiveError> {
    let sequence_count = value
        .batch
        .last_local_sequence
        .checked_sub(value.batch.first_local_sequence)
        .and_then(|span| span.checked_add(1))
        .ok_or(ArchiveError::ManifestVerification(
            "raw V2 local sequence span overflows",
        ))?;
    if value.schema != RAW_BATCH_MANIFEST_SCHEMA_V2
        || !valid_producer_build_id(&value.producer_build_id)
        || value.created_at_micros < 0
        || value.batch.cursor_policy != RAW_V2_CURSOR_POLICY
        || value.batch.start_offset > value.batch.end_offset
        || value.batch.first_local_sequence == 0
        || value.batch.first_received_wall_micros < 0
        || value.batch.last_received_wall_micros < 0
        || value.batch.first_received_wall_micros > value.batch.last_received_wall_micros
        || !valid_manifest_identity(&value.batch.source_version)
        || !valid_manifest_identity(&value.batch.parser_schema_version)
        || value.object.row_count != sequence_count
        || value.object.size_bytes == 0
    {
        return Err(ArchiveError::ManifestVerification(
            "raw V2 batch manifest metadata is invalid",
        ));
    }
    source_id(&value.batch.source_id)?;
    chain_id(&value.batch.chain_id)?;
    raw::parse_observation_class(&value.batch.observation_class)?;
    for hash in [
        &value.batch.spool_manifest_blake3,
        &value.batch.spool_segment_blake3,
        &value.batch.rolling_content_sha256,
        &value.object.sha256,
        &value.object.schema_fingerprint_sha256,
    ] {
        manifest::parse_hash(hash)?;
    }
    fs::validate_relative(Path::new(&value.object.relative_path))
}

fn valid_manifest_identity(value: &str) -> bool {
    !value.is_empty()
        && value.trim() == value
        && value.len() <= 256
        && !value.chars().any(char::is_control)
}

fn valid_producer_build_id(value: &str) -> bool {
    !value.is_empty()
        && value.trim() == value
        && value.len() <= 256
        && !value.bytes().any(|byte| byte.is_ascii_control())
}

fn expected_object_relative(value: &RawBatchManifestV2) -> Result<PathBuf, ArchiveError> {
    let chain = chain_id(&value.batch.chain_id)?;
    let source = source_id(&value.batch.source_id)?;
    let object_hash = manifest::parse_hash(&value.object.sha256)?;
    let partition = manifest::partition_for(value.batch.first_received_wall_micros)?;
    Ok(dataset_relative(&chain, &source)
        .join(partition)
        .join("objects")
        .join(format!(
            "epoch={}",
            manifest::encoded_component(&value.batch.cursor_epoch)
        ))
        .join(format!(
            "sequences={}-{}",
            value.batch.first_local_sequence, value.batch.last_local_sequence
        ))
        .join(format!(
            "offsets={}-{}",
            value.batch.start_offset, value.batch.end_offset
        ))
        .join(format!("part-{}.parquet", hex::encode(object_hash))))
}

fn verify_loaded_batch(
    archive: &LocalParquetArchive,
    loaded: &Loaded<RawBatchManifestV2>,
) -> Result<VerifiedRawManifest, ArchiveError> {
    let (_, object) = verify_and_decode(archive, &loaded.value)?;
    Ok(VerifiedRawManifest::new(
        manifest::manifest_id(loaded.hash)?,
        loaded.hash,
        schema::raw_schema_fingerprint()?,
        manifest::parse_hash(&loaded.value.batch.rolling_content_sha256)?,
        manifest::parse_hash(&loaded.value.batch.spool_manifest_blake3)?,
        manifest::parse_hash(&loaded.value.batch.spool_segment_blake3)?,
        object,
    ))
}

fn verify_and_decode(
    archive: &LocalParquetArchive,
    value: &RawBatchManifestV2,
) -> Result<(Vec<SourceObservation>, RawArchiveObject), ArchiveError> {
    let source = source_id(&value.batch.source_id)?;
    let chain = chain_id(&value.batch.chain_id)?;
    let object_hash = manifest::parse_hash(&value.object.sha256)?;
    let expected = expected_object_relative(value)?;
    if Path::new(&value.object.relative_path) != expected {
        return Err(ArchiveError::ManifestVerification(
            "raw V2 manifest does not bind exact object path",
        ));
    }
    let bytes = fs::read_regular(archive.root(), &expected, archive.config().max_read_bytes())
        .map_err(|error| match error {
            ArchiveError::Io(_) => ArchiveError::CorruptObject(value.object.relative_path.clone()),
            other => other,
        })?;
    if u64::try_from(bytes.len()).ok() != Some(value.object.size_bytes)
        || <[u8; 32]>::from(Sha256::digest(&bytes)) != object_hash
    {
        return Err(ArchiveError::CorruptObject(
            value.object.relative_path.clone(),
        ));
    }
    let expected_schema = schema::raw_schema_fingerprint()?;
    if manifest::parse_hash(&value.object.schema_fingerprint_sha256)? != expected_schema {
        return Err(ArchiveError::SchemaMismatch);
    }
    let builder = ParquetRecordBatchReaderBuilder::try_new(bytes::Bytes::from(bytes))
        .map_err(|_| ArchiveError::CorruptObject(value.object.relative_path.clone()))?;
    if builder.schema().fields() != schema::raw_schema().fields() {
        return Err(ArchiveError::SchemaMismatch);
    }
    let reader = builder
        .build()
        .map_err(|_| ArchiveError::CorruptObject(value.object.relative_path.clone()))?;
    let mut observations = Vec::new();
    for batch in reader {
        decode_raw_batch(
            &batch.map_err(|_| ArchiveError::CorruptObject(value.object.relative_path.clone()))?,
            value,
            archive.config().max_read_bytes(),
            &mut observations,
        )?;
    }
    if u64::try_from(observations.len()).ok() != Some(value.object.row_count) {
        return Err(ArchiveError::ManifestVerification(
            "raw V2 Parquet row count does not match manifest",
        ));
    }
    let first_observation = observations
        .first()
        .ok_or(ArchiveError::ManifestVerification(
            "raw V2 Parquet object is empty",
        ))?;
    let last_observation = observations
        .last()
        .ok_or(ArchiveError::ManifestVerification(
            "raw V2 Parquet object is empty",
        ))?;
    if first_observation.cursor().offset() != value.batch.start_offset
        || last_observation.cursor().offset() != value.batch.end_offset
        || first_observation.received().wall_micros() != value.batch.first_received_wall_micros
        || last_observation.received().wall_micros() != value.batch.last_received_wall_micros
    {
        return Err(ArchiveError::ManifestVerification(
            "raw V2 descriptor boundaries disagree with Parquet rows",
        ));
    }
    let first_sequence = LocalRecordSequence::try_new(value.batch.first_local_sequence)?;
    let reconstructed = RawObservationBatch::try_new_byte_offsets(
        chain.clone(),
        observations.clone(),
        manifest::parse_hash(&value.batch.spool_manifest_blake3)?,
        manifest::parse_hash(&value.batch.spool_segment_blake3)?,
        first_sequence,
    )?;
    if rolling_content_hash(&reconstructed)?
        != manifest::parse_hash(&value.batch.rolling_content_sha256)?
    {
        return Err(ArchiveError::ManifestVerification(
            "raw V2 rolling content hash mismatch",
        ));
    }
    let cursor_range = RawObservationRange::try_new(
        value.batch.cursor_epoch.clone(),
        value.batch.start_offset,
        value.batch.end_offset,
    )?;
    let sequence_range = LocalRecordSequenceRange::try_new(
        first_sequence,
        LocalRecordSequence::try_new(value.batch.last_local_sequence)?,
    )?;
    let object = RawArchiveObject::try_new_byte_offsets(
        expected,
        object_hash,
        value.object.size_bytes,
        value.object.row_count,
        chain,
        source,
        cursor_range,
        sequence_range,
    )?;
    Ok((observations, object))
}

fn decode_raw_batch(
    batch: &RecordBatch,
    manifest_value: &RawBatchManifestV2,
    max_payload_bytes: u64,
    output: &mut Vec<SourceObservation>,
) -> Result<(), ArchiveError> {
    let chains = raw::column::<StringArray>(batch, 0)?;
    let sources = raw::column::<StringArray>(batch, 1)?;
    let versions = raw::column::<StringArray>(batch, 2)?;
    let classes = raw::column::<StringArray>(batch, 3)?;
    let epochs = raw::column::<StringArray>(batch, 4)?;
    let offsets = raw::column::<UInt64Array>(batch, 5)?;
    let walls = raw::column::<Int64Array>(batch, 6)?;
    let monotonic = raw::column::<UInt64Array>(batch, 7)?;
    let parsers = raw::column::<StringArray>(batch, 8)?;
    let hashes = raw::column::<FixedSizeBinaryArray>(batch, 9)?;
    let warnings = raw::column::<StringArray>(batch, 10)?;
    let payloads = raw::column::<BinaryArray>(batch, 11)?;
    let payload_limit = usize::try_from(max_payload_bytes)
        .map_err(|_| ArchiveError::InvalidInput("raw payload limit exceeds address space"))?;
    let expected_partition =
        manifest::partition_for(manifest_value.batch.first_received_wall_micros)?;
    for row in 0..batch.num_rows() {
        let warning_values: Vec<ParseWarning> = serde_json::from_str(warnings.value(row))
            .map_err(|_| ArchiveError::ManifestVerification("invalid raw warning JSON"))?;
        let source = SourceId::new(sources.value(row).to_owned())
            .map_err(|_| ArchiveError::ManifestVerification("raw source ID is invalid"))?;
        let observation = SourceObservation::new(
            source,
            versions.value(row),
            raw::parse_observation_class(classes.value(row))?,
            SourceCursor::new(epochs.value(row), offsets.value(row))
                .map_err(|error| ArchiveError::Codec(error.to_string()))?,
            ReceiveTimestamps::new(walls.value(row), monotonic.value(row))
                .map_err(|error| ArchiveError::Codec(error.to_string()))?,
            parsers.value(row),
            bytes::Bytes::copy_from_slice(payloads.value(row)),
            warning_values,
            payload_limit,
        )
        .map_err(|error| ArchiveError::Codec(error.to_string()))?;
        if chains.value(row) != manifest_value.batch.chain_id
            || observation.source_id().as_str() != manifest_value.batch.source_id
            || observation.source_version() != manifest_value.batch.source_version
            || raw::observation_class_name(observation.observation_class())?
                != manifest_value.batch.observation_class
            || observation.cursor().epoch() != manifest_value.batch.cursor_epoch
            || observation.parser_schema_version() != manifest_value.batch.parser_schema_version
            || hashes.value(row) != observation.content_hash().as_bytes()
            || manifest::partition_for(observation.received().wall_micros())? != expected_partition
        {
            return Err(ArchiveError::ManifestVerification(
                "raw V2 Parquet query columns disagree with authoritative payload",
            ));
        }
        if let Some(previous) = output.last()
            && observation.cursor().offset() <= previous.cursor().offset()
        {
            return Err(ArchiveError::ManifestVerification(
                "raw V2 Parquet native offsets are not strictly increasing",
            ));
        }
        output.push(observation);
    }
    Ok(())
}

fn receipt(
    value: &RawBatchManifestV2,
    manifest_hash: [u8; 32],
    verified: &VerifiedRawManifest,
) -> Result<RawObservationReceipt, ArchiveError> {
    let durable_at = KnownTime::from_unix_micros(value.created_at_micros)
        .map_err(|_| ArchiveError::ManifestVerification("invalid raw V2 durable time"))?;
    let sequence_range =
        verified
            .local_sequence_range()
            .ok_or(ArchiveError::ManifestVerification(
                "verified raw V2 manifest lost sequence evidence",
            ))?;
    RawObservationReceipt::try_new_byte_offsets(
        format!("raw-archive-receipt-v2-{}", hex::encode(manifest_hash)),
        manifest::manifest_id(manifest_hash)?,
        verified.object().chain_id().clone(),
        verified.object().source_id().clone(),
        value.batch.cursor_epoch.clone(),
        value.batch.start_offset,
        value.batch.end_offset,
        sequence_range,
        verified.spool_manifest_blake3(),
        verified.spool_segment_blake3(),
        verified.rolling_content_sha256(),
        verified.object().sha256(),
        manifest_hash,
        verified.schema_fingerprint(),
        durable_at,
    )
}

fn raw_batch_ref(
    batch: &RawBatchManifestV2,
    relative: &Path,
    hash: [u8; 32],
) -> Result<RawBatchRefV2, ArchiveError> {
    Ok(RawBatchRefV2 {
        cursor_policy: batch.batch.cursor_policy.clone(),
        cursor_epoch: batch.batch.cursor_epoch.clone(),
        start_offset: batch.batch.start_offset,
        end_offset: batch.batch.end_offset,
        first_local_sequence: batch.batch.first_local_sequence,
        last_local_sequence: batch.batch.last_local_sequence,
        manifest_relative_path: raw::path_string(relative)?,
        manifest_sha256: hex::encode(hash),
    })
}

fn find_exact_ref<'a>(
    catalog: &'a RawCatalogV2,
    descriptor: &RawBatchDescriptorV2,
) -> Option<&'a RawBatchRefV2> {
    catalog.value_refs().find(|reference| {
        reference.cursor_policy == descriptor.cursor_policy
            && reference.cursor_epoch == descriptor.cursor_epoch
            && reference.start_offset == descriptor.start_offset
            && reference.end_offset == descriptor.end_offset
            && reference.first_local_sequence == descriptor.first_local_sequence
            && reference.last_local_sequence == descriptor.last_local_sequence
    })
}

impl RawCatalogV2 {
    fn value_refs(&self) -> impl Iterator<Item = &RawBatchRefV2> {
        self.batches.iter()
    }
}

fn next_generation(previous: Option<u64>, overflow: &'static str) -> Result<u64, ArchiveError> {
    previous
        .map(|value| {
            value
                .checked_add(1)
                .ok_or(ArchiveError::InvalidInput(overflow))
        })
        .transpose()
        .map(|value| value.unwrap_or(1))
}

fn source_id(value: &str) -> Result<SourceId, ArchiveError> {
    SourceId::new(value.to_owned())
        .map_err(|_| ArchiveError::ManifestVerification("raw V2 source ID is invalid"))
}

fn chain_id(value: &str) -> Result<ChainId, ArchiveError> {
    ChainId::new(value.to_owned())
        .map_err(|_| ArchiveError::ManifestVerification("raw V2 chain ID is invalid"))
}

fn dataset_relative(chain: &ChainId, source: &SourceId) -> PathBuf {
    super::raw_policy::dataset_relative(
        chain,
        source,
        super::raw_policy::RawPolicy::MonotonicByteV2,
    )
}

fn global_manifest_relative(hash: [u8; 32]) -> PathBuf {
    PathBuf::from("_manifests")
        .join("raw-byte-v2")
        .join(format!("manifest-{}.json", hex::encode(hash)))
}

fn catalog_manifest_relative(dataset: &Path, hash: [u8; 32]) -> PathBuf {
    dataset
        .join("manifests")
        .join(format!("catalog-{}.json", hex::encode(hash)))
}

fn partition_manifest_relative(dataset: &Path, partition: &str, hash: [u8; 32]) -> PathBuf {
    dataset
        .join(partition)
        .join("manifests")
        .join(format!("partition-{}.json", hex::encode(hash)))
}

fn conflicting_range(source: &SourceId, descriptor: &RawBatchDescriptorV2) -> ArchiveError {
    ArchiveError::ConflictingRawRange {
        source_id: source.clone(),
        epoch: descriptor.cursor_epoch.clone(),
        start: descriptor.start_offset,
        end: descriptor.end_offset,
    }
}

const fn ranges_overlap(left_start: u64, left_end: u64, right_start: u64, right_end: u64) -> bool {
    left_start <= right_end && right_start <= left_end
}

fn extends_at_tail<T: PartialEq>(previous: &[T], current: &[T]) -> bool {
    current.len() == previous.len().saturating_add(1) && current.starts_with(previous)
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use hl_protocol::{ObservationClass, ReceiveTimestamps, SourceCursor};

    use super::*;

    #[test]
    fn append_batch_covers_every_constructible_cursor_policy() {
        for cursor_policy in [
            CursorPolicy::ContiguousNativeOffset,
            CursorPolicy::MonotonicByteOffset,
        ] {
            let temporary = tempfile::tempdir().expect("archive root");
            let durable_at = KnownTime::from_unix_micros(1_721_779_300_000_000).expect("time");
            let archive = crate::LocalParquetArchive::open(
                temporary.path(),
                crate::ArchiveConfig::deterministic_fixture("raw-v2-policy-test", durable_at)
                    .expect("config"),
            )
            .expect("archive");
            let batch = match cursor_policy {
                CursorPolicy::ContiguousNativeOffset => contiguous_batch(),
                CursorPolicy::MonotonicByteOffset => byte_offset_batch(),
            };
            let result = append_batch(&archive, &batch, durable_at);
            match cursor_policy {
                CursorPolicy::MonotonicByteOffset => {
                    let receipt = result.expect("raw V2 append still admits MonotonicByteOffset");
                    assert_eq!(receipt.cursor_policy(), CursorPolicy::MonotonicByteOffset);
                    assert!(
                        receipt.receipt_id().starts_with("raw-archive-receipt-v2-"),
                        "MonotonicByteOffset still takes the V2 byte-offset append path"
                    );
                    assert!(receipt.local_sequence_range().is_some());
                }
                CursorPolicy::ContiguousNativeOffset => {
                    let error = result.expect_err(
                        "raw V2 append still rejects ContiguousNativeOffset as InvalidInput",
                    );
                    assert!(matches!(
                        error,
                        ArchiveError::InvalidInput(
                            "raw V2 archive requires monotonic byte offsets"
                        )
                    ));
                    assert_eq!(error.reason_code(), "archive.invalid_input");
                }
            }
        }
    }

    fn contiguous_batch() -> RawObservationBatch {
        RawObservationBatch::try_new(
            ChainId::new("mainnet").expect("chain"),
            vec![policy_observation(
                "primary-node",
                ObservationClass::CommittedBlock,
                10,
                b"legacy",
            )],
            [0xa1; 32],
            [0xb2; 32],
        )
        .expect("contiguous batch")
    }

    fn byte_offset_batch() -> RawObservationBatch {
        RawObservationBatch::try_new_byte_offsets(
            ChainId::new("mainnet").expect("chain"),
            vec![policy_observation(
                "node-trades",
                ObservationClass::AuxiliaryLedger,
                19,
                b"byte",
            )],
            [0xa1; 32],
            [0xb2; 32],
            LocalRecordSequence::try_new(41).expect("local sequence"),
        )
        .expect("byte-offset batch")
    }

    fn policy_observation(
        source: &str,
        observation_class: ObservationClass,
        offset: u64,
        payload: &'static [u8],
    ) -> SourceObservation {
        SourceObservation::new(
            SourceId::new(source).expect("source"),
            "capture-v1",
            observation_class,
            SourceCursor::new("epoch-a", offset).expect("cursor"),
            ReceiveTimestamps::new(1_721_779_200_000_000, 9_000_000).expect("timestamps"),
            "raw-parser-v1",
            Bytes::from_static(payload),
            Vec::new(),
            1024,
        )
        .expect("observation")
    }

    #[test]
    fn batch_manifest_v2_bytes_are_frozen() {
        let value = RawBatchManifestV2 {
            schema: RAW_BATCH_MANIFEST_SCHEMA_V2.to_owned(),
            producer_build_id: "build-v2".to_owned(),
            created_at_micros: 42,
            batch: RawBatchDescriptorV2 {
                chain_id: "mainnet".to_owned(),
                source_id: "node-trades".to_owned(),
                source_version: "capture-v1".to_owned(),
                observation_class: "auxiliary_ledger".to_owned(),
                cursor_policy: RAW_V2_CURSOR_POLICY.to_owned(),
                cursor_epoch: "rotation-7".to_owned(),
                start_offset: 19,
                end_offset: 47,
                first_local_sequence: 41,
                last_local_sequence: 43,
                first_received_wall_micros: 100,
                last_received_wall_micros: 200,
                parser_schema_version: "raw-parser-v1".to_owned(),
                spool_manifest_blake3: "11".repeat(32),
                spool_segment_blake3: "22".repeat(32),
                rolling_content_sha256: "33".repeat(32),
            },
            object: ObjectDescriptorV1 {
                relative_path: "object.parquet".to_owned(),
                sha256: "44".repeat(32),
                size_bytes: 512,
                row_count: 3,
                schema_fingerprint_sha256: "55".repeat(32),
            },
        };
        let bytes = canonical_json(&value).expect("serialize frozen V2 fixture");
        let expected = concat!(
            r#"{"schema":"hyperliquid-alpha-desk/archive-raw-batch-manifest/v2","producer_build_id":"build-v2","created_at_micros":42,"batch":{"chain_id":"mainnet","source_id":"node-trades","source_version":"capture-v1","observation_class":"auxiliary_ledger","cursor_policy":"monotonic-byte-offset","cursor_epoch":"rotation-7","start_offset":19,"end_offset":47,"first_local_sequence":41,"last_local_sequence":43,"first_received_wall_micros":100,"last_received_wall_micros":200,"parser_schema_version":"raw-parser-v1","spool_manifest_blake3":"#,
            "\"",
            "1111111111111111111111111111111111111111111111111111111111111111",
            r#"","spool_segment_blake3":"#,
            "\"",
            "2222222222222222222222222222222222222222222222222222222222222222",
            r#"","rolling_content_sha256":"#,
            "\"",
            "3333333333333333333333333333333333333333333333333333333333333333",
            r#""},"object":{"relative_path":"object.parquet","sha256":"#,
            "\"",
            "4444444444444444444444444444444444444444444444444444444444444444",
            r#"","size_bytes":512,"row_count":3,"schema_fingerprint_sha256":"#,
            "\"",
            "5555555555555555555555555555555555555555555555555555555555555555",
            r#""}}"#,
        );
        assert_eq!(bytes, expected.as_bytes());
        assert_eq!(
            hex::encode(manifest::sha256(&bytes)),
            "5e676f347f14b3dd803c6f9a7a8f5fae7938626fd8d4dd0ffc5d349e3542802a"
        );
    }

    #[test]
    fn manifest_generation_must_add_exactly_one_tail_reference() {
        assert!(extends_at_tail(&[10_u64, 20], &[10, 20, 30]));
        assert!(!extends_at_tail(&[10_u64, 20], &[5, 10, 20]));
        assert!(!extends_at_tail(&[10_u64, 20], &[10, 15, 20]));
        assert!(!extends_at_tail(&[10_u64, 20], &[10, 20]));
        assert!(!extends_at_tail(&[10_u64, 20], &[10, 20, 30, 40]));
    }

    #[test]
    fn decoder_rejects_a_middle_row_from_another_hour() {
        let base = 1_721_779_200_000_000_i64;
        let observations = [base, base + 3_600_000_000, base + 2]
            .into_iter()
            .enumerate()
            .map(|(index, wall_micros)| {
                SourceObservation::new(
                    SourceId::new("node-trades").unwrap(),
                    "capture-v1",
                    ObservationClass::AuxiliaryLedger,
                    SourceCursor::new("rotation-7", 19 + u64::try_from(index).unwrap()).unwrap(),
                    ReceiveTimestamps::new(wall_micros, 9_000_000 + u64::try_from(index).unwrap())
                        .unwrap(),
                    "raw-parser-v1",
                    Bytes::from_static(b"payload"),
                    Vec::new(),
                    1024,
                )
                .unwrap()
            })
            .collect::<Vec<_>>();
        let batch = RawObservationBatch::try_new_byte_offsets(
            ChainId::new("mainnet").unwrap(),
            observations,
            [0x11; 32],
            [0x22; 32],
            LocalRecordSequence::try_new(41).unwrap(),
        )
        .unwrap();
        let manifest_value = RawBatchManifestV2 {
            schema: RAW_BATCH_MANIFEST_SCHEMA_V2.to_owned(),
            producer_build_id: "build-v2".to_owned(),
            created_at_micros: base,
            batch: RawBatchDescriptorV2 {
                chain_id: "mainnet".to_owned(),
                source_id: "node-trades".to_owned(),
                source_version: "capture-v1".to_owned(),
                observation_class: raw::observation_class_name(ObservationClass::AuxiliaryLedger)
                    .unwrap(),
                cursor_policy: RAW_V2_CURSOR_POLICY.to_owned(),
                cursor_epoch: "rotation-7".to_owned(),
                start_offset: 19,
                end_offset: 21,
                first_local_sequence: 41,
                last_local_sequence: 43,
                first_received_wall_micros: base,
                last_received_wall_micros: base + 2,
                parser_schema_version: "raw-parser-v1".to_owned(),
                spool_manifest_blake3: "11".repeat(32),
                spool_segment_blake3: "22".repeat(32),
                rolling_content_sha256: "33".repeat(32),
            },
            object: ObjectDescriptorV1 {
                relative_path: "object.parquet".to_owned(),
                sha256: "44".repeat(32),
                size_bytes: 512,
                row_count: 3,
                schema_fingerprint_sha256: "55".repeat(32),
            },
        };
        let record_batch = raw::raw_record_batch(&batch).unwrap();
        let error = decode_raw_batch(&record_batch, &manifest_value, 1024, &mut Vec::new())
            .expect_err("middle-row hour mismatch must fail");
        assert!(matches!(error, ArchiveError::ManifestVerification(_)));
    }
}
