use std::{
    collections::BTreeSet,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::Arc,
};

use arrow::{
    array::{
        Array, ArrayRef, BinaryArray, BinaryBuilder, FixedSizeBinaryArray, FixedSizeBinaryBuilder,
        Int64Array, StringArray, UInt64Array,
    },
    record_batch::RecordBatch,
};
use chrono::NaiveDateTime;
use domain_types::{ChainId, KnownTime, ManifestId, SourceId};
use hl_protocol::{
    ObservationClass, ParseWarning, ReceiveTimestamps, SourceCursor, SourceObservation,
};
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
    ArchiveError, RawArchiveObject, RawObservationBatch, RawObservationIterator,
    RawObservationRange, RawObservationReceipt, VerifiedRawManifest,
};

use super::{
    LocalParquetArchive, fs,
    inspection::{ArchiveDataset, ArchiveInspection, InspectedObject},
    manifest::{
        self, CURRENT_POINTER_SCHEMA_V1, CurrentPointerV1, ObjectDescriptorV1, canonical_json,
    },
    schema,
};

const RAW_DATASET: &str = "raw_source_observations";
const RAW_BATCH_MANIFEST_SCHEMA_V1: &str = "hyperliquid-alpha-desk/archive-raw-batch-manifest/v1";
const RAW_PARTITION_MANIFEST_SCHEMA_V1: &str =
    "hyperliquid-alpha-desk/archive-raw-partition-manifest/v1";
const RAW_CATALOG_SCHEMA_V1: &str = "hyperliquid-alpha-desk/archive-raw-catalog/v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBatchDescriptorV1 {
    chain_id: String,
    source_id: String,
    source_version: String,
    observation_class: String,
    cursor_epoch: String,
    start_offset: u64,
    end_offset: u64,
    first_received_wall_micros: i64,
    last_received_wall_micros: i64,
    parser_schema_version: String,
    spool_manifest_blake3: String,
    spool_segment_blake3: String,
    rolling_content_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBatchManifestV1 {
    schema: String,
    producer_build_id: String,
    created_at_micros: i64,
    batch: RawBatchDescriptorV1,
    object: ObjectDescriptorV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBatchRefV1 {
    cursor_epoch: String,
    start_offset: u64,
    end_offset: u64,
    manifest_relative_path: String,
    manifest_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPartitionManifestV1 {
    schema: String,
    chain_id: String,
    source_id: String,
    dataset: String,
    partition: String,
    generation: u64,
    producer_build_id: String,
    created_at_micros: i64,
    previous_manifest_sha256: Option<String>,
    batches: Vec<RawBatchRefV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPartitionManifestRefV1 {
    partition: String,
    manifest_relative_path: String,
    manifest_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCatalogV1 {
    schema: String,
    chain_id: String,
    source_id: String,
    dataset: String,
    generation: u64,
    producer_build_id: String,
    created_at_micros: i64,
    previous_manifest_sha256: Option<String>,
    partitions: std::collections::BTreeMap<String, RawPartitionManifestRefV1>,
    batches: Vec<RawBatchRefV1>,
}

#[derive(Debug, Clone)]
struct Loaded<T> {
    value: T,
    hash: [u8; 32],
    relative: PathBuf,
}

pub fn append_batch(
    archive: &LocalParquetArchive,
    batch: &RawObservationBatch,
    durable_at: KnownTime,
) -> Result<RawObservationReceipt, ArchiveError> {
    let first = batch
        .observations()
        .first()
        .ok_or(ArchiveError::InvalidInput("raw observation batch is empty"))?;
    let descriptor = batch_descriptor(batch)?;
    let source = first.source_id();
    let chain = batch.chain_id();
    let dataset = dataset_relative(chain, source);
    let _process_lock = fs::open_writer_lock(archive.root(), &dataset.join(".writer.lock"))?;
    let current = load_current_catalog(archive, chain, source)?;

    if let Some(existing) = current
        .as_ref()
        .and_then(|catalog| find_exact_ref(&catalog.value, &descriptor))
    {
        let loaded = load_batch_ref(archive, existing)?;
        if loaded.value.batch != descriptor {
            return Err(conflicting_range(source, &descriptor));
        }
        let verified = verify_loaded_batch(archive, &loaded)?;
        return receipt(&loaded.value, loaded.hash, &verified);
    }
    if current.as_ref().is_some_and(|catalog| {
        catalog.value.batches.iter().any(|reference| {
            reference.cursor_epoch == descriptor.cursor_epoch
                && ranges_overlap(
                    reference.start_offset,
                    reference.end_offset,
                    descriptor.start_offset,
                    descriptor.end_offset,
                )
        })
    }) {
        return Err(conflicting_range(source, &descriptor));
    }

    let schema_fingerprint = schema::raw_schema_fingerprint()?;
    let object = write_object(archive, batch, &descriptor, schema_fingerprint)?;
    let raw_manifest = RawBatchManifestV1 {
        schema: RAW_BATCH_MANIFEST_SCHEMA_V1.to_owned(),
        producer_build_id: archive.config().producer_build_id().to_owned(),
        created_at_micros: durable_at.unix_micros(),
        batch: descriptor,
        object,
    };
    let bytes = canonical_json(&raw_manifest)?;
    let hash = manifest::sha256(&bytes);
    let relative = global_manifest_relative(hash);
    fs::publish_immutable(archive.root(), &relative, &bytes)?;
    let partition = publish_partition(
        archive,
        current.as_ref(),
        &raw_manifest,
        &relative,
        hash,
        durable_at,
    )?;
    publish_catalog(
        archive,
        current.as_ref(),
        &raw_manifest,
        &relative,
        hash,
        partition,
        durable_at,
    )?;
    let loaded = Loaded {
        value: raw_manifest,
        hash,
        relative,
    };
    let verified = verify_loaded_batch(archive, &loaded)?;
    receipt(&loaded.value, hash, &verified)
}

pub fn read_observations(
    archive: &LocalParquetArchive,
    chain: &ChainId,
    source: &SourceId,
    range: RawObservationRange,
) -> Result<RawObservationIterator, ArchiveError> {
    let count = range
        .end_offset()
        .checked_sub(range.start_offset())
        .and_then(|span| span.checked_add(1))
        .ok_or(ArchiveError::InvalidInput(
            "raw observation read range overflows",
        ))?;
    if count > archive.config().max_read_blocks() {
        return Err(ArchiveError::InvalidInput(
            "raw observation range exceeds configured record limit",
        ));
    }
    let catalog =
        load_current_catalog(archive, chain, source)?.ok_or(ArchiveError::RangeUnavailable)?;
    let mut references = catalog
        .value
        .batches
        .iter()
        .filter(|reference| {
            reference.cursor_epoch == range.epoch()
                && ranges_overlap(
                    reference.start_offset,
                    reference.end_offset,
                    range.start_offset(),
                    range.end_offset(),
                )
        })
        .cloned()
        .collect::<Vec<_>>();
    references.sort_by_key(|reference| reference.start_offset);

    let mut total_bytes = 0_u64;
    let mut observations = Vec::new();
    for reference in &references {
        let loaded = load_batch_ref(archive, reference)?;
        total_bytes = total_bytes
            .checked_add(loaded.value.object.size_bytes)
            .ok_or(ArchiveError::InvalidInput(
                "raw observation byte count overflows",
            ))?;
        if total_bytes > archive.config().max_read_bytes() {
            return Err(ArchiveError::InvalidInput(
                "raw observation range exceeds configured byte limit",
            ));
        }
        let (decoded, _) = verify_and_decode(archive, &loaded.value)?;
        observations.extend(decoded.into_iter().filter(|observation| {
            observation.cursor().offset() >= range.start_offset()
                && observation.cursor().offset() <= range.end_offset()
        }));
    }
    if u64::try_from(observations.len()).ok() != Some(count) {
        return Err(ArchiveError::RangeUnavailable);
    }
    observations.sort_by_key(|observation| observation.cursor().offset());
    for (index, observation) in observations.iter().enumerate() {
        let offset = u64::try_from(index)
            .map_err(|_| ArchiveError::InvalidInput("raw observation index exceeds u64"))?;
        let expected =
            range
                .start_offset()
                .checked_add(offset)
                .ok_or(ArchiveError::InvalidInput(
                    "raw observation cursor overflows",
                ))?;
        if observation.source_id() != source
            || observation.cursor().epoch() != range.epoch()
            || observation.cursor().offset() != expected
        {
            return Err(ArchiveError::RangeUnavailable);
        }
    }
    Ok(Box::new(observations.into_iter().map(Ok)))
}

pub fn verify_raw_manifest(
    archive: &LocalParquetArchive,
    manifest_id: &ManifestId,
) -> Result<VerifiedRawManifest, ArchiveError> {
    let hash = manifest::hash_from_manifest_id(manifest_id)?;
    let relative = global_manifest_relative(hash);
    let loaded = load_batch_at(archive.root(), &relative)?;
    if loaded.hash != hash || loaded.relative != relative {
        return Err(ArchiveError::ManifestVerification(
            "raw manifest ID does not bind exact bytes and path",
        ));
    }
    verify_loaded_batch(archive, &loaded)
}

pub(crate) fn inspect_source(
    archive: &LocalParquetArchive,
    chain: &ChainId,
    source: &SourceId,
) -> Result<Option<ArchiveInspection>, ArchiveError> {
    let Some(catalog) = load_current_catalog(archive, chain, source)? else {
        return Ok(None);
    };
    let mut observations = 0_u64;
    let mut seen = BTreeSet::new();
    let mut objects = Vec::new();
    for reference in &catalog.value.batches {
        let loaded = load_batch_ref(archive, reference)?;
        if !seen.insert(loaded.hash) {
            return Err(ArchiveError::ManifestVerification(
                "raw catalog references a batch manifest more than once",
            ));
        }
        let verified = verify_loaded_batch(archive, &loaded)?;
        observations = observations
            .checked_add(verified.object().row_count())
            .ok_or(ArchiveError::InvalidInput(
                "raw inspection observation count overflows",
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

fn batch_descriptor(batch: &RawObservationBatch) -> Result<RawBatchDescriptorV1, ArchiveError> {
    let first = batch
        .observations()
        .first()
        .ok_or(ArchiveError::InvalidInput("raw observation batch is empty"))?;
    let last = batch
        .observations()
        .last()
        .ok_or(ArchiveError::InvalidInput("raw observation batch is empty"))?;
    let partition = manifest::partition_for(first.received().wall_micros())?;
    for observation in batch.observations().iter().skip(1) {
        if manifest::partition_for(observation.received().wall_micros())? != partition {
            return Err(ArchiveError::InvalidInput(
                "raw observation batch crosses an hour partition",
            ));
        }
    }
    Ok(RawBatchDescriptorV1 {
        chain_id: batch.chain_id().as_str().to_owned(),
        source_id: first.source_id().as_str().to_owned(),
        source_version: first.source_version().to_owned(),
        observation_class: observation_class_name(first.observation_class())?,
        cursor_epoch: first.cursor().epoch().to_owned(),
        start_offset: first.cursor().offset(),
        end_offset: last.cursor().offset(),
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
    for observation in batch.observations() {
        hash_frame(&mut hasher, batch.chain_id().as_str().as_bytes())?;
        hash_frame(&mut hasher, observation.source_id().as_str().as_bytes())?;
        hash_frame(&mut hasher, observation.source_version().as_bytes())?;
        hash_frame(
            &mut hasher,
            observation_class_name(observation.observation_class())?.as_bytes(),
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
    descriptor: &RawBatchDescriptorV1,
    schema_fingerprint: [u8; 32],
) -> Result<ObjectDescriptorV1, ArchiveError> {
    let source = SourceId::new(descriptor.source_id.clone())
        .map_err(|_| ArchiveError::InvalidInput("raw observation source ID"))?;
    let chain = ChainId::new(descriptor.chain_id.clone())
        .map_err(|_| ArchiveError::InvalidInput("raw observation chain ID"))?;
    let partition = manifest::partition_for(descriptor.first_received_wall_micros)?;
    let parent = dataset_relative(&chain, &source)
        .join(partition)
        .join("objects")
        .join(format!(
            "epoch={}",
            manifest::encoded_component(&descriptor.cursor_epoch)
        ))
        .join(format!(
            "offsets={}-{}",
            descriptor.start_offset, descriptor.end_offset
        ));
    let mut staged = fs::create_parquet_staging_file(archive.root(), &parent)?;
    let record_batch = raw_record_batch(batch)?;
    let compression = ZstdLevel::try_new(3)
        .map_err(|_| ArchiveError::InvalidInput("invalid Parquet compression level"))?;
    let properties = WriterProperties::builder()
        .set_created_by("hyperliquid-alpha-desk/raw-archive-writer-v1".to_owned())
        .set_compression(Compression::ZSTD(compression))
        .set_statistics_enabled(EnabledStatistics::Page)
        .set_key_value_metadata(Some(vec![
            KeyValue::new("alpha_desk.dataset".to_owned(), RAW_DATASET.to_owned()),
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
        .map_err(|_| ArchiveError::Io("syncing raw Parquet object"))?;
    let (hash, size_bytes) = hash_file(staged.as_file_mut())?;
    let relative = parent.join(format!("part-{}.parquet", hex::encode(hash)));
    fs::publish_staged_immutable(archive.root(), &relative, staged)?;
    let published = fs::read_regular(archive.root(), &relative, size_bytes)?;
    if u64::try_from(published.len()).ok() != Some(size_bytes)
        || <[u8; 32]>::from(Sha256::digest(&published)) != hash
    {
        return Err(ArchiveError::CorruptObject(path_string(&relative)?));
    }
    Ok(ObjectDescriptorV1 {
        relative_path: path_string(&relative)?,
        sha256: hex::encode(hash),
        size_bytes,
        row_count: u64::try_from(batch.observations().len())
            .map_err(|_| ArchiveError::InvalidInput("raw row count exceeds u64"))?,
        schema_fingerprint_sha256: hex::encode(schema_fingerprint),
    })
}

fn raw_record_batch(batch: &RawObservationBatch) -> Result<RecordBatch, ArchiveError> {
    let observations = batch.observations();
    let chain_ids = StringArray::from_iter_values(std::iter::repeat_n(
        batch.chain_id().as_str(),
        observations.len(),
    ));
    let source_ids =
        StringArray::from_iter_values(observations.iter().map(|value| value.source_id().as_str()));
    let versions =
        StringArray::from_iter_values(observations.iter().map(SourceObservation::source_version));
    let classes = StringArray::from_iter_values(
        observations
            .iter()
            .map(|value| observation_class_name(value.observation_class()))
            .collect::<Result<Vec<_>, _>>()?,
    );
    let epochs =
        StringArray::from_iter_values(observations.iter().map(|value| value.cursor().epoch()));
    let offsets =
        UInt64Array::from_iter_values(observations.iter().map(|value| value.cursor().offset()));
    let wall_times = Int64Array::from_iter_values(
        observations
            .iter()
            .map(|value| value.received().wall_micros()),
    );
    let monotonic_times = UInt64Array::from_iter_values(
        observations
            .iter()
            .map(|value| value.received().monotonic_nanos()),
    );
    let parsers = StringArray::from_iter_values(
        observations
            .iter()
            .map(SourceObservation::parser_schema_version),
    );
    let mut hashes = FixedSizeBinaryBuilder::with_capacity(observations.len(), 32);
    let mut warnings = Vec::with_capacity(observations.len());
    let mut payloads = BinaryBuilder::new();
    for observation in observations {
        hashes
            .append_value(observation.content_hash().as_bytes())
            .map_err(|error| ArchiveError::Codec(error.to_string()))?;
        warnings.push(
            serde_json::to_string(observation.warnings())
                .map_err(|_| ArchiveError::Codec("serializing parse warnings".into()))?,
        );
        payloads.append_value(observation.payload());
    }
    let columns: Vec<ArrayRef> = vec![
        Arc::new(chain_ids),
        Arc::new(source_ids),
        Arc::new(versions),
        Arc::new(classes),
        Arc::new(epochs),
        Arc::new(offsets),
        Arc::new(wall_times),
        Arc::new(monotonic_times),
        Arc::new(parsers),
        Arc::new(hashes.finish()),
        Arc::new(StringArray::from(warnings)),
        Arc::new(payloads.finish()),
    ];
    RecordBatch::try_new(schema::raw_schema(), columns)
        .map_err(|error| ArchiveError::Codec(error.to_string()))
}

fn publish_partition(
    archive: &LocalParquetArchive,
    catalog: Option<&Loaded<RawCatalogV1>>,
    batch: &RawBatchManifestV1,
    batch_relative: &Path,
    batch_hash: [u8; 32],
    durable_at: KnownTime,
) -> Result<RawPartitionManifestRefV1, ArchiveError> {
    let source = SourceId::new(batch.batch.source_id.clone())
        .map_err(|_| ArchiveError::InvalidInput("raw observation source ID"))?;
    let chain = ChainId::new(batch.batch.chain_id.clone())
        .map_err(|_| ArchiveError::InvalidInput("raw observation chain ID"))?;
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
    batches.sort_by(|left, right| {
        (&left.cursor_epoch, left.start_offset).cmp(&(&right.cursor_epoch, right.start_offset))
    });
    validate_refs(&batches)?;
    let generation = previous
        .as_ref()
        .map(|loaded| {
            loaded
                .value
                .generation
                .checked_add(1)
                .ok_or(ArchiveError::InvalidInput(
                    "raw partition generation overflows",
                ))
        })
        .transpose()?
        .unwrap_or(1);
    let value = RawPartitionManifestV1 {
        schema: RAW_PARTITION_MANIFEST_SCHEMA_V1.to_owned(),
        chain_id: chain.as_str().to_owned(),
        source_id: source.as_str().to_owned(),
        dataset: RAW_DATASET.to_owned(),
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
    Ok(RawPartitionManifestRefV1 {
        partition,
        manifest_relative_path: path_string(&relative)?,
        manifest_sha256: hex::encode(hash),
    })
}

fn publish_catalog(
    archive: &LocalParquetArchive,
    previous: Option<&Loaded<RawCatalogV1>>,
    batch: &RawBatchManifestV1,
    batch_relative: &Path,
    batch_hash: [u8; 32],
    partition: RawPartitionManifestRefV1,
    durable_at: KnownTime,
) -> Result<(), ArchiveError> {
    let source = SourceId::new(batch.batch.source_id.clone())
        .map_err(|_| ArchiveError::InvalidInput("raw observation source ID"))?;
    let chain = ChainId::new(batch.batch.chain_id.clone())
        .map_err(|_| ArchiveError::InvalidInput("raw observation chain ID"))?;
    let dataset = dataset_relative(&chain, &source);
    let mut batches = previous
        .map(|loaded| loaded.value.batches.clone())
        .unwrap_or_default();
    batches.push(raw_batch_ref(batch, batch_relative, batch_hash)?);
    batches.sort_by(|left, right| {
        (&left.cursor_epoch, left.start_offset).cmp(&(&right.cursor_epoch, right.start_offset))
    });
    validate_refs(&batches)?;
    let generation = previous
        .map(|loaded| {
            loaded
                .value
                .generation
                .checked_add(1)
                .ok_or(ArchiveError::InvalidInput(
                    "raw catalog generation overflows",
                ))
        })
        .transpose()?
        .unwrap_or(1);
    let mut partitions = previous
        .map(|loaded| loaded.value.partitions.clone())
        .unwrap_or_default();
    partitions.insert(partition.partition.clone(), partition);
    let catalog = RawCatalogV1 {
        schema: RAW_CATALOG_SCHEMA_V1.to_owned(),
        chain_id: chain.as_str().to_owned(),
        source_id: source.as_str().to_owned(),
        dataset: RAW_DATASET.to_owned(),
        generation,
        producer_build_id: archive.config().producer_build_id().to_owned(),
        created_at_micros: durable_at.unix_micros(),
        previous_manifest_sha256: previous.map(|loaded| hex::encode(loaded.hash)),
        partitions,
        batches,
    };
    let bytes = canonical_json(&catalog)?;
    let hash = manifest::sha256(&bytes);
    let relative = dataset
        .join("manifests")
        .join(format!("catalog-{}.json", hex::encode(hash)));
    fs::publish_immutable(archive.root(), &relative, &bytes)?;
    let pointer = CurrentPointerV1 {
        schema: CURRENT_POINTER_SCHEMA_V1.to_owned(),
        manifest_relative_path: path_string(&relative)?,
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
) -> Result<Option<Loaded<RawCatalogV1>>, ArchiveError> {
    let dataset = dataset_relative(chain, source);
    let current = dataset.join("CURRENT");
    let bytes = match fs::read_manifest(archive.root(), &current) {
        Ok(bytes) => bytes,
        Err(ArchiveError::Io(_)) if !archive.root().join(&current).exists() => return Ok(None),
        Err(error) => return Err(error),
    };
    let pointer: CurrentPointerV1 = serde_json::from_slice(&bytes)
        .map_err(|_| ArchiveError::ManifestVerification("invalid raw current pointer JSON"))?;
    if pointer.schema != CURRENT_POINTER_SCHEMA_V1 {
        return Err(ArchiveError::ManifestVerification(
            "unsupported raw current pointer schema",
        ));
    }
    let hash = manifest::parse_hash(&pointer.manifest_sha256)?;
    let expected = dataset
        .join("manifests")
        .join(format!("catalog-{}.json", hex::encode(hash)));
    if Path::new(&pointer.manifest_relative_path) != expected {
        return Err(ArchiveError::ManifestVerification(
            "raw current pointer does not bind exact catalog path",
        ));
    }
    let loaded = load_catalog_at(archive.root(), &expected)?;
    if loaded.hash != hash {
        return Err(ArchiveError::ManifestVerification(
            "raw current pointer hash mismatch",
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
    head: &Loaded<RawCatalogV1>,
) -> Result<(), ArchiveError> {
    let mut seen = BTreeSet::new();
    let mut current = head.clone();
    loop {
        if !seen.insert(current.hash) {
            return Err(ArchiveError::ManifestVerification(
                "raw catalog chain contains a cycle",
            ));
        }
        let Some(previous_hash) = current.value.previous_manifest_sha256.as_deref() else {
            return if current.value.generation == 1 {
                Ok(())
            } else {
                Err(ArchiveError::ManifestVerification(
                    "raw catalog root generation is invalid",
                ))
            };
        };
        let previous_hash = manifest::parse_hash(previous_hash)?;
        let previous_relative = dataset
            .join("manifests")
            .join(format!("catalog-{}.json", hex::encode(previous_hash)));
        let previous = load_catalog_at(root, &previous_relative)?;
        let chain = ChainId::new(current.value.chain_id.clone())
            .map_err(|_| ArchiveError::ManifestVerification("raw catalog chain ID is invalid"))?;
        let source = SourceId::new(current.value.source_id.clone())
            .map_err(|_| ArchiveError::ManifestVerification("raw catalog source ID is invalid"))?;
        validate_catalog(&previous.value, &chain, &source)?;
        if previous.hash != previous_hash
            || previous.value.generation.checked_add(1) != Some(current.value.generation)
            || current.value.batches.len() != previous.value.batches.len().saturating_add(1)
            || !previous
                .value
                .batches
                .iter()
                .all(|reference| current.value.batches.contains(reference))
        {
            return Err(ArchiveError::ManifestVerification(
                "raw catalog chain is not append-only",
            ));
        }
        verify_partition_transition(root, dataset, &previous.value, &current.value)?;
        current = previous;
    }
}

fn verify_partition_transition(
    root: &Path,
    dataset: &Path,
    previous: &RawCatalogV1,
    current: &RawCatalogV1,
) -> Result<(), ArchiveError> {
    let chain = ChainId::new(current.chain_id.clone())
        .map_err(|_| ArchiveError::ManifestVerification("raw catalog chain ID is invalid"))?;
    let source = SourceId::new(current.source_id.clone())
        .map_err(|_| ArchiveError::ManifestVerification("raw catalog source ID is invalid"))?;
    let mut changes = 0_u64;
    for (partition, prior_ref) in &previous.partitions {
        let next_ref =
            current
                .partitions
                .get(partition)
                .ok_or(ArchiveError::ManifestVerification(
                    "raw catalog removed a partition",
                ))?;
        if prior_ref == next_ref {
            continue;
        }
        changes = changes.checked_add(1).ok_or(ArchiveError::InvalidInput(
            "raw catalog partition change count overflows",
        ))?;
        let prior_hash = manifest::parse_hash(&prior_ref.manifest_sha256)?;
        let next = load_partition_ref(root, dataset, &chain, &source, next_ref)?;
        let expected_previous = hex::encode(prior_hash);
        if next.value.previous_manifest_sha256.as_deref() != Some(expected_previous.as_str()) {
            return Err(ArchiveError::ManifestVerification(
                "raw partition update does not extend the prior generation",
            ));
        }
    }
    for (partition, reference) in &current.partitions {
        if previous.partitions.contains_key(partition) {
            continue;
        }
        changes = changes.checked_add(1).ok_or(ArchiveError::InvalidInput(
            "raw catalog partition change count overflows",
        ))?;
        let added = load_partition_ref(root, dataset, &chain, &source, reference)?;
        if added.value.generation != 1 || added.value.previous_manifest_sha256.is_some() {
            return Err(ArchiveError::ManifestVerification(
                "new raw partition does not begin at generation one",
            ));
        }
    }
    if changes != 1 {
        return Err(ArchiveError::ManifestVerification(
            "raw catalog transition must change exactly one partition",
        ));
    }
    Ok(())
}

fn verify_catalog_partitions(
    root: &Path,
    dataset: &Path,
    catalog: &Loaded<RawCatalogV1>,
) -> Result<(), ArchiveError> {
    let chain = ChainId::new(catalog.value.chain_id.clone())
        .map_err(|_| ArchiveError::ManifestVerification("raw catalog chain ID is invalid"))?;
    let source = SourceId::new(catalog.value.source_id.clone())
        .map_err(|_| ArchiveError::ManifestVerification("raw catalog source ID is invalid"))?;
    let mut batches = Vec::new();
    for (partition, reference) in &catalog.value.partitions {
        if reference.partition != *partition {
            return Err(ArchiveError::ManifestVerification(
                "raw catalog partition key and reference disagree",
            ));
        }
        let loaded = load_partition_ref(root, dataset, &chain, &source, reference)?;
        verify_partition_chain(root, dataset, &chain, &source, &loaded)?;
        for batch_ref in &loaded.value.batches {
            let batch = load_batch_ref_at(root, batch_ref)?;
            if batch.value.batch.chain_id != chain.as_str()
                || batch.value.batch.source_id != source.as_str()
                || manifest::partition_for(batch.value.batch.first_received_wall_micros)?
                    != *partition
            {
                return Err(ArchiveError::ManifestVerification(
                    "raw partition references a batch outside its identity or hour",
                ));
            }
        }
        batches.extend(loaded.value.batches);
    }
    batches.sort_by(|left, right| {
        (&left.cursor_epoch, left.start_offset).cmp(&(&right.cursor_epoch, right.start_offset))
    });
    if batches != catalog.value.batches {
        return Err(ArchiveError::ManifestVerification(
            "raw catalog batches disagree with partition manifests",
        ));
    }
    Ok(())
}

fn load_catalog_at(root: &Path, relative: &Path) -> Result<Loaded<RawCatalogV1>, ArchiveError> {
    let bytes = fs::read_manifest(root, relative)?;
    let hash = manifest::sha256(&bytes);
    let value = serde_json::from_slice(&bytes)
        .map_err(|_| ArchiveError::ManifestVerification("invalid raw catalog JSON"))?;
    Ok(Loaded {
        value,
        hash,
        relative: relative.to_path_buf(),
    })
}

fn validate_catalog(
    catalog: &RawCatalogV1,
    chain: &ChainId,
    source: &SourceId,
) -> Result<(), ArchiveError> {
    if catalog.schema != RAW_CATALOG_SCHEMA_V1
        || catalog.chain_id != chain.as_str()
        || catalog.source_id != source.as_str()
        || catalog.dataset != RAW_DATASET
        || catalog.generation == 0
        || catalog.producer_build_id.is_empty()
        || catalog.created_at_micros < 0
        || catalog.partitions.is_empty()
        || catalog.batches.is_empty()
    {
        return Err(ArchiveError::ManifestVerification(
            "raw catalog metadata is invalid",
        ));
    }
    if let Some(previous) = &catalog.previous_manifest_sha256 {
        manifest::parse_hash(previous)?;
    }
    if (catalog.generation == 1) != catalog.previous_manifest_sha256.is_none() {
        return Err(ArchiveError::ManifestVerification(
            "raw catalog generation and previous hash disagree",
        ));
    }
    let dataset = dataset_relative(chain, source);
    for (partition, reference) in &catalog.partitions {
        validate_partition_name(partition)?;
        let hash = manifest::parse_hash(&reference.manifest_sha256)?;
        let expected = partition_manifest_relative(&dataset, partition, hash);
        if reference.partition != *partition
            || Path::new(&reference.manifest_relative_path) != expected
        {
            return Err(ArchiveError::ManifestVerification(
                "raw catalog partition reference is invalid",
            ));
        }
    }
    validate_refs(&catalog.batches)
}

fn validate_partition(
    value: &RawPartitionManifestV1,
    chain: &ChainId,
    source: &SourceId,
    partition: &str,
) -> Result<(), ArchiveError> {
    if value.schema != RAW_PARTITION_MANIFEST_SCHEMA_V1
        || value.chain_id != chain.as_str()
        || value.source_id != source.as_str()
        || value.dataset != RAW_DATASET
        || value.partition != partition
        || value.generation == 0
        || value.producer_build_id.is_empty()
        || value.created_at_micros < 0
        || value.batches.is_empty()
    {
        return Err(ArchiveError::ManifestVerification(
            "raw partition manifest metadata is invalid",
        ));
    }
    validate_partition_name(partition)?;
    if let Some(previous) = &value.previous_manifest_sha256 {
        manifest::parse_hash(previous)?;
    }
    if (value.generation == 1) != value.previous_manifest_sha256.is_none() {
        return Err(ArchiveError::ManifestVerification(
            "raw partition generation and previous hash disagree",
        ));
    }
    validate_refs(&value.batches)
}

fn validate_partition_name(partition: &str) -> Result<(), ArchiveError> {
    let timestamp = format!("{partition}:00:00");
    if NaiveDateTime::parse_from_str(&timestamp, "date=%Y-%m-%d/hour=%H:%M:%S").is_err()
        || fs::validate_relative(Path::new(partition)).is_err()
    {
        return Err(ArchiveError::ManifestVerification(
            "raw partition name is invalid",
        ));
    }
    Ok(())
}

fn validate_refs(references: &[RawBatchRefV1]) -> Result<(), ArchiveError> {
    let mut previous: Option<&RawBatchRefV1> = None;
    for reference in references {
        if reference.cursor_epoch.is_empty() || reference.start_offset > reference.end_offset {
            return Err(ArchiveError::ManifestVerification(
                "raw catalog batch reference is invalid",
            ));
        }
        manifest::parse_hash(&reference.manifest_sha256)?;
        fs::validate_relative(Path::new(&reference.manifest_relative_path))?;
        if let Some(prior) = previous
            && ((&prior.cursor_epoch, prior.start_offset)
                >= (&reference.cursor_epoch, reference.start_offset)
                || (prior.cursor_epoch == reference.cursor_epoch
                    && prior.end_offset >= reference.start_offset))
        {
            return Err(ArchiveError::ManifestVerification(
                "raw catalog references overlap or are not ordered",
            ));
        }
        previous = Some(reference);
    }
    Ok(())
}

fn load_batch_ref(
    archive: &LocalParquetArchive,
    reference: &RawBatchRefV1,
) -> Result<Loaded<RawBatchManifestV1>, ArchiveError> {
    load_batch_ref_at(archive.root(), reference)
}

fn load_batch_ref_at(
    root: &Path,
    reference: &RawBatchRefV1,
) -> Result<Loaded<RawBatchManifestV1>, ArchiveError> {
    let hash = manifest::parse_hash(&reference.manifest_sha256)?;
    let expected = global_manifest_relative(hash);
    if Path::new(&reference.manifest_relative_path) != expected {
        return Err(ArchiveError::ManifestVerification(
            "raw batch reference does not bind exact manifest path",
        ));
    }
    let loaded = load_batch_at(root, &expected)?;
    if loaded.hash != hash
        || loaded.value.batch.cursor_epoch != reference.cursor_epoch
        || loaded.value.batch.start_offset != reference.start_offset
        || loaded.value.batch.end_offset != reference.end_offset
    {
        return Err(ArchiveError::ManifestVerification(
            "raw batch reference content mismatch",
        ));
    }
    Ok(loaded)
}

fn load_partition_ref(
    root: &Path,
    dataset: &Path,
    chain: &ChainId,
    source: &SourceId,
    reference: &RawPartitionManifestRefV1,
) -> Result<Loaded<RawPartitionManifestV1>, ArchiveError> {
    let hash = manifest::parse_hash(&reference.manifest_sha256)?;
    let expected = partition_manifest_relative(dataset, &reference.partition, hash);
    if Path::new(&reference.manifest_relative_path) != expected {
        return Err(ArchiveError::ManifestVerification(
            "raw partition reference does not bind exact manifest path",
        ));
    }
    let loaded = load_partition_at(root, &expected)?;
    if loaded.hash != hash {
        return Err(ArchiveError::ManifestVerification(
            "raw partition reference hash mismatch",
        ));
    }
    validate_partition(&loaded.value, chain, source, &reference.partition)?;
    Ok(loaded)
}

fn load_partition_at(
    root: &Path,
    relative: &Path,
) -> Result<Loaded<RawPartitionManifestV1>, ArchiveError> {
    let bytes = fs::read_manifest(root, relative)?;
    let hash = manifest::sha256(&bytes);
    let value: RawPartitionManifestV1 = serde_json::from_slice(&bytes)
        .map_err(|_| ArchiveError::ManifestVerification("invalid raw partition manifest JSON"))?;
    Ok(Loaded {
        value,
        hash,
        relative: relative.to_path_buf(),
    })
}

fn verify_partition_chain(
    root: &Path,
    dataset: &Path,
    chain: &ChainId,
    source: &SourceId,
    head: &Loaded<RawPartitionManifestV1>,
) -> Result<(), ArchiveError> {
    let mut seen = BTreeSet::new();
    let mut current = head.clone();
    loop {
        if !seen.insert(current.hash) {
            return Err(ArchiveError::ManifestVerification(
                "raw partition chain contains a cycle",
            ));
        }
        let Some(previous_hash) = current.value.previous_manifest_sha256.as_deref() else {
            return if current.value.generation == 1 {
                Ok(())
            } else {
                Err(ArchiveError::ManifestVerification(
                    "raw partition root generation is invalid",
                ))
            };
        };
        let previous_hash = manifest::parse_hash(previous_hash)?;
        let previous_relative =
            partition_manifest_relative(dataset, &current.value.partition, previous_hash);
        let previous = load_partition_at(root, &previous_relative)?;
        validate_partition(&previous.value, chain, source, &current.value.partition)?;
        if previous.hash != previous_hash
            || previous.value.generation.checked_add(1) != Some(current.value.generation)
            || current.value.batches.len() != previous.value.batches.len().saturating_add(1)
            || !previous
                .value
                .batches
                .iter()
                .all(|reference| current.value.batches.contains(reference))
        {
            return Err(ArchiveError::ManifestVerification(
                "raw partition chain is not append-only",
            ));
        }
        current = previous;
    }
}

fn load_batch_at(root: &Path, relative: &Path) -> Result<Loaded<RawBatchManifestV1>, ArchiveError> {
    let bytes = fs::read_manifest(root, relative)?;
    let hash = manifest::sha256(&bytes);
    let value: RawBatchManifestV1 = serde_json::from_slice(&bytes)
        .map_err(|_| ArchiveError::ManifestVerification("invalid raw batch manifest JSON"))?;
    validate_batch_manifest(&value)?;
    Ok(Loaded {
        value,
        hash,
        relative: relative.to_path_buf(),
    })
}

fn validate_batch_manifest(value: &RawBatchManifestV1) -> Result<(), ArchiveError> {
    if value.schema != RAW_BATCH_MANIFEST_SCHEMA_V1
        || value.producer_build_id.is_empty()
        || value.created_at_micros < 0
        || value.batch.start_offset > value.batch.end_offset
        || value.object.row_count
            != value
                .batch
                .end_offset
                .checked_sub(value.batch.start_offset)
                .and_then(|span| span.checked_add(1))
                .ok_or(ArchiveError::ManifestVerification(
                    "raw batch cursor range overflows",
                ))?
        || value.object.size_bytes == 0
    {
        return Err(ArchiveError::ManifestVerification(
            "raw batch manifest metadata is invalid",
        ));
    }
    SourceId::new(value.batch.source_id.clone())
        .map_err(|_| ArchiveError::ManifestVerification("raw source ID is invalid"))?;
    ChainId::new(value.batch.chain_id.clone())
        .map_err(|_| ArchiveError::ManifestVerification("raw chain ID is invalid"))?;
    parse_observation_class(&value.batch.observation_class)?;
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

fn verify_loaded_batch(
    archive: &LocalParquetArchive,
    loaded: &Loaded<RawBatchManifestV1>,
) -> Result<VerifiedRawManifest, ArchiveError> {
    let (_, object) = verify_and_decode(archive, &loaded.value)?;
    let manifest_id = manifest::manifest_id(loaded.hash)?;
    Ok(VerifiedRawManifest::new(
        manifest_id,
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
    value: &RawBatchManifestV1,
) -> Result<(Vec<SourceObservation>, RawArchiveObject), ArchiveError> {
    let source = SourceId::new(value.batch.source_id.clone())
        .map_err(|_| ArchiveError::ManifestVerification("raw source ID is invalid"))?;
    let chain = ChainId::new(value.batch.chain_id.clone())
        .map_err(|_| ArchiveError::ManifestVerification("raw chain ID is invalid"))?;
    let object_hash = manifest::parse_hash(&value.object.sha256)?;
    let partition = manifest::partition_for(value.batch.first_received_wall_micros)?;
    let expected = dataset_relative(&chain, &source)
        .join(partition)
        .join("objects")
        .join(format!(
            "epoch={}",
            manifest::encoded_component(&value.batch.cursor_epoch)
        ))
        .join(format!(
            "offsets={}-{}",
            value.batch.start_offset, value.batch.end_offset
        ))
        .join(format!("part-{}.parquet", hex::encode(object_hash)));
    if Path::new(&value.object.relative_path) != expected {
        return Err(ArchiveError::ManifestVerification(
            "raw manifest does not bind exact object path",
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
            "raw Parquet row count does not match manifest",
        ));
    }
    let reconstructed = RawObservationBatch::try_new(
        chain.clone(),
        observations.clone(),
        manifest::parse_hash(&value.batch.spool_manifest_blake3)?,
        manifest::parse_hash(&value.batch.spool_segment_blake3)?,
    )?;
    if rolling_content_hash(&reconstructed)?
        != manifest::parse_hash(&value.batch.rolling_content_sha256)?
    {
        return Err(ArchiveError::ManifestVerification(
            "raw rolling content hash mismatch",
        ));
    }
    let range = RawObservationRange::try_new(
        value.batch.cursor_epoch.clone(),
        value.batch.start_offset,
        value.batch.end_offset,
    )?;
    let object = RawArchiveObject::try_new(
        expected,
        object_hash,
        value.object.size_bytes,
        value.object.row_count,
        chain,
        source,
        range,
    )?;
    Ok((observations, object))
}

fn decode_raw_batch(
    batch: &RecordBatch,
    manifest_value: &RawBatchManifestV1,
    max_payload_bytes: u64,
    output: &mut Vec<SourceObservation>,
) -> Result<(), ArchiveError> {
    let chains = column::<StringArray>(batch, 0)?;
    let sources = column::<StringArray>(batch, 1)?;
    let versions = column::<StringArray>(batch, 2)?;
    let classes = column::<StringArray>(batch, 3)?;
    let epochs = column::<StringArray>(batch, 4)?;
    let offsets = column::<UInt64Array>(batch, 5)?;
    let walls = column::<Int64Array>(batch, 6)?;
    let monotonic = column::<UInt64Array>(batch, 7)?;
    let parsers = column::<StringArray>(batch, 8)?;
    let hashes = column::<FixedSizeBinaryArray>(batch, 9)?;
    let warnings = column::<StringArray>(batch, 10)?;
    let payloads = column::<BinaryArray>(batch, 11)?;
    let payload_limit = usize::try_from(max_payload_bytes)
        .map_err(|_| ArchiveError::InvalidInput("raw payload limit exceeds address space"))?;
    for row in 0..batch.num_rows() {
        let warning_values: Vec<ParseWarning> = serde_json::from_str(warnings.value(row))
            .map_err(|_| ArchiveError::ManifestVerification("invalid raw warning JSON"))?;
        let source = SourceId::new(sources.value(row).to_owned())
            .map_err(|_| ArchiveError::ManifestVerification("raw source ID is invalid"))?;
        let observation = SourceObservation::new(
            source,
            versions.value(row),
            parse_observation_class(classes.value(row))?,
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
            || observation_class_name(observation.observation_class())?
                != manifest_value.batch.observation_class
            || observation.cursor().epoch() != manifest_value.batch.cursor_epoch
            || observation.parser_schema_version() != manifest_value.batch.parser_schema_version
            || hashes.value(row) != observation.content_hash().as_bytes()
        {
            return Err(ArchiveError::ManifestVerification(
                "raw Parquet query columns disagree with authoritative payload",
            ));
        }
        output.push(observation);
    }
    Ok(())
}

fn column<T: Array + 'static>(batch: &RecordBatch, index: usize) -> Result<&T, ArchiveError> {
    batch
        .column(index)
        .as_any()
        .downcast_ref::<T>()
        .ok_or(ArchiveError::SchemaMismatch)
}

fn observation_class_name(value: ObservationClass) -> Result<String, ArchiveError> {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .ok_or_else(|| ArchiveError::Codec("serializing observation class".into()))
}

fn parse_observation_class(value: &str) -> Result<ObservationClass, ArchiveError> {
    serde_json::from_value(serde_json::Value::String(value.to_owned()))
        .map_err(|_| ArchiveError::ManifestVerification("raw observation class is not supported"))
}

fn dataset_relative(chain: &ChainId, source: &SourceId) -> PathBuf {
    PathBuf::from(format!(
        "chain={}",
        manifest::encoded_component(chain.as_str())
    ))
    .join(format!("dataset={RAW_DATASET}"))
    .join(format!(
        "source={}",
        manifest::encoded_component(source.as_str())
    ))
}

fn global_manifest_relative(hash: [u8; 32]) -> PathBuf {
    PathBuf::from("_manifests")
        .join("raw")
        .join(format!("manifest-{}.json", hex::encode(hash)))
}

fn partition_manifest_relative(dataset: &Path, partition: &str, hash: [u8; 32]) -> PathBuf {
    dataset
        .join(partition)
        .join("manifests")
        .join(format!("partition-{}.json", hex::encode(hash)))
}

fn raw_batch_ref(
    batch: &RawBatchManifestV1,
    relative: &Path,
    hash: [u8; 32],
) -> Result<RawBatchRefV1, ArchiveError> {
    Ok(RawBatchRefV1 {
        cursor_epoch: batch.batch.cursor_epoch.clone(),
        start_offset: batch.batch.start_offset,
        end_offset: batch.batch.end_offset,
        manifest_relative_path: path_string(relative)?,
        manifest_sha256: hex::encode(hash),
    })
}

fn find_exact_ref<'a>(
    catalog: &'a RawCatalogV1,
    descriptor: &RawBatchDescriptorV1,
) -> Option<&'a RawBatchRefV1> {
    catalog.batches.iter().find(|reference| {
        reference.cursor_epoch == descriptor.cursor_epoch
            && reference.start_offset == descriptor.start_offset
            && reference.end_offset == descriptor.end_offset
    })
}

fn conflicting_range(source: &SourceId, descriptor: &RawBatchDescriptorV1) -> ArchiveError {
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

fn receipt(
    value: &RawBatchManifestV1,
    manifest_hash: [u8; 32],
    verified: &VerifiedRawManifest,
) -> Result<RawObservationReceipt, ArchiveError> {
    let manifest_id = manifest::manifest_id(manifest_hash)?;
    let durable_at = KnownTime::from_unix_micros(value.created_at_micros)
        .map_err(|_| ArchiveError::ManifestVerification("invalid raw durable time"))?;
    RawObservationReceipt::try_new(
        format!("raw-archive-receipt-v1-{}", hex::encode(manifest_hash)),
        manifest_id,
        verified.object().chain_id().clone(),
        verified.object().source_id().clone(),
        value.batch.cursor_epoch.clone(),
        value.batch.start_offset,
        value.batch.end_offset,
        verified.spool_manifest_blake3(),
        verified.spool_segment_blake3(),
        verified.rolling_content_sha256(),
        verified.object().sha256(),
        manifest_hash,
        verified.schema_fingerprint(),
        durable_at,
    )
}

fn hash_file(file: &mut File) -> Result<([u8; 32], u64), ArchiveError> {
    file.seek(SeekFrom::Start(0))
        .map_err(|_| ArchiveError::Io("seeking raw Parquet object"))?;
    let mut hasher = Sha256::new();
    let mut length = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|_| ArchiveError::Io("hashing raw Parquet object"))?;
        if read == 0 {
            break;
        }
        length = length
            .checked_add(
                u64::try_from(read)
                    .map_err(|_| ArchiveError::InvalidInput("raw object size exceeds u64"))?,
            )
            .ok_or(ArchiveError::InvalidInput("raw object size overflows"))?;
        hasher.update(&buffer[..read]);
    }
    Ok((hasher.finalize().into(), length))
}

fn path_string(path: &Path) -> Result<String, ArchiveError> {
    path.to_str()
        .map(ToOwned::to_owned)
        .ok_or(ArchiveError::UnsafePath)
}
