use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
    sync::Arc,
};

use arrow::{
    array::{
        ArrayRef, BinaryBuilder, FixedSizeBinaryBuilder, Int64Array, StringArray, UInt32Array,
        UInt64Array,
    },
    record_batch::RecordBatch,
};
use canonical_events::BlockEnvelope;
use domain_types::KnownTime;
use parquet::{
    arrow::ArrowWriter,
    basic::{Compression, ZstdLevel},
    file::{
        metadata::KeyValue,
        properties::{EnabledStatistics, WriterProperties},
    },
};
use sha2::{Digest, Sha256};
use storage_ports::{ArchiveError, ArchiveReceipt};

use super::{
    LocalParquetArchive, fs,
    manifest::{
        self, BLOCK_MANIFEST_SCHEMA_V1, BlockDescriptorV1, BlockManifestRefV1, BlockManifestV1,
        CANONICAL_DATASET, CATALOG_MANIFEST_SCHEMA_V1, CURRENT_POINTER_SCHEMA_V1,
        CatalogManifestV1, CurrentPointerV1, ObjectDescriptorV1, PARTITION_MANIFEST_SCHEMA_V1,
        PartitionManifestRefV1, PartitionManifestV1, PartitionTransitionV1,
    },
    reader, schema,
};

pub fn append_block(
    archive: &LocalParquetArchive,
    block: &BlockEnvelope,
    durable_at: KnownTime,
) -> Result<ArchiveReceipt, ArchiveError> {
    let dataset = reader::dataset_relative(block.chain_id());
    let _process_lock = fs::open_writer_lock(archive.root(), &dataset.join(".writer.lock"))?;
    let partition = manifest::partition_for(block.block_time().unix_micros())?;
    let catalog = reader::load_current_catalog(archive, block.chain_id())?;
    let partition_head = reader::load_current_partition(archive, block.chain_id(), &partition)?;

    validate_catalog_partition_relationship(
        archive,
        block,
        catalog.as_ref(),
        partition_head.as_ref(),
        &partition,
    )?;

    if let Some(existing) = find_existing_block(partition_head.as_ref(), block.block_height().get())
    {
        let block_manifest = reader::load_block_manifest_ref(archive, existing)?;
        let descriptor = block_manifest
            .value
            .blocks
            .iter()
            .find(|descriptor| descriptor.block_height == block.block_height().get())
            .ok_or(ArchiveError::ManifestVerification(
                "block manifest does not contain referenced block",
            ))?;
        let expected_hash = manifest::parse_hash(&descriptor.canonical_block_blake3)?;
        if expected_hash != block.canonical_block_hash() {
            return Err(ArchiveError::ConflictingBlock(block.block_height()));
        }
        let manifest_id = manifest::manifest_id(block_manifest.hash)?;
        reader::verify_block_manifest(archive, &manifest_id)?;
        if !catalog_references_partition(catalog.as_ref(), &partition, partition_head.as_ref())? {
            let partition_head = partition_head
                .as_ref()
                .ok_or(ArchiveError::ManifestVerification("missing partition head"))?;
            publish_catalog(
                archive,
                block,
                catalog.as_ref(),
                &partition,
                partition_head,
                durable_at,
            )?;
        }
        return receipt_from_block_manifest(
            &block_manifest.value,
            block_manifest.hash,
            block.block_height().get(),
        );
    }

    let schema_fingerprint = schema::canonical_schema_fingerprint()?;
    let object = write_parquet_object(
        archive,
        std::slice::from_ref(block),
        &dataset,
        &partition,
        schema_fingerprint,
    )?;
    let block_manifest = BlockManifestV1 {
        schema: BLOCK_MANIFEST_SCHEMA_V1.to_owned(),
        producer_build_id: archive.config().producer_build_id().to_owned(),
        created_at_micros: durable_at.unix_micros(),
        input_object_count: 1,
        rolling_content_sha256: hex::encode(rolling_content_hash(std::slice::from_ref(block))?),
        blocks: vec![BlockDescriptorV1::from_block(block)?],
        object,
    };
    let block_manifest_bytes = manifest::canonical_json(&block_manifest)?;
    let block_manifest_hash = manifest::sha256(&block_manifest_bytes);
    let block_manifest_relative = reader::global_block_manifest_relative(block_manifest_hash);
    fs::publish_immutable(
        archive.root(),
        &block_manifest_relative,
        &block_manifest_bytes,
    )?;

    let block_reference = BlockManifestRefV1 {
        block_height: block.block_height().get(),
        canonical_block_blake3: hex::encode(block.canonical_block_hash()),
        manifest_relative_path: path_string(&block_manifest_relative)?,
        manifest_sha256: hex::encode(block_manifest_hash),
    };
    let partition_manifest = publish_partition(
        archive,
        block,
        partition_head.as_ref(),
        &dataset,
        &partition,
        block_reference,
        durable_at,
    )?;
    publish_catalog(
        archive,
        block,
        catalog.as_ref(),
        &partition,
        &partition_manifest,
        durable_at,
    )?;

    let manifest_id = manifest::manifest_id(block_manifest_hash)?;
    reader::verify_block_manifest(archive, &manifest_id)?;
    receipt_from_block_manifest(
        &block_manifest,
        block_manifest_hash,
        block.block_height().get(),
    )
}

pub(crate) fn write_parquet_object(
    archive: &LocalParquetArchive,
    blocks: &[BlockEnvelope],
    dataset: &Path,
    partition: &str,
    schema_fingerprint: [u8; 32],
) -> Result<ObjectDescriptorV1, ArchiveError> {
    let first = blocks.first().ok_or(ArchiveError::InvalidInput(
        "canonical archive bundle is empty",
    ))?;
    let last = blocks.last().ok_or(ArchiveError::InvalidInput(
        "canonical archive bundle is empty",
    ))?;
    let parent = dataset
        .join(partition)
        .join("objects")
        .join(format!("block_start={}", first.block_height().get()))
        .join(format!("block_end={}", last.block_height().get()));
    let mut staged = fs::create_parquet_staging_file(archive.root(), &parent)?;
    let compression = ZstdLevel::try_new(3)
        .map_err(|_| ArchiveError::InvalidInput("invalid Parquet compression level"))?;
    let properties = WriterProperties::builder()
        .set_created_by("hyperliquid-alpha-desk/archive-writer-v1".to_owned())
        .set_compression(Compression::ZSTD(compression))
        .set_statistics_enabled(EnabledStatistics::Page)
        .set_key_value_metadata(Some(vec![
            KeyValue::new(
                "alpha_desk.dataset".to_owned(),
                CANONICAL_DATASET.to_owned(),
            ),
            KeyValue::new(
                "alpha_desk.schema_fingerprint_sha256".to_owned(),
                hex::encode(schema_fingerprint),
            ),
        ]))
        .build();
    {
        let file = staged.as_file_mut();
        let mut writer = ArrowWriter::try_new(file, schema::canonical_schema(), Some(properties))
            .map_err(|error| ArchiveError::Codec(error.to_string()))?;
        for block in blocks {
            writer
                .write(&block_record_batch(block)?)
                .map_err(|error| ArchiveError::Codec(error.to_string()))?;
        }
        writer
            .close()
            .map_err(|error| ArchiveError::Codec(error.to_string()))?;
    }
    staged
        .as_file_mut()
        .sync_all()
        .map_err(|_| ArchiveError::Io("syncing Parquet object"))?;
    let (object_hash, size_bytes) = hash_file(staged.as_file_mut())?;
    let relative = parent.join(format!("part-{}.parquet", hex::encode(object_hash)));
    fs::publish_staged_immutable(archive.root(), &relative, staged)?;
    verify_published_hash(archive.root(), &relative, object_hash, size_bytes)?;

    Ok(ObjectDescriptorV1 {
        relative_path: path_string(&relative)?,
        sha256: hex::encode(object_hash),
        size_bytes,
        row_count: blocks.iter().try_fold(0_u64, |total, block| {
            total
                .checked_add(
                    u64::try_from(block.events().len()).map_err(|_| {
                        ArchiveError::InvalidInput("canonical event count exceeds u64")
                    })?,
                )
                .ok_or(ArchiveError::InvalidInput(
                    "canonical bundle event count overflows",
                ))
        })?,
        schema_fingerprint_sha256: hex::encode(schema_fingerprint),
    })
}

pub(crate) fn block_record_batch(block: &BlockEnvelope) -> Result<RecordBatch, ArchiveError> {
    let events = block.events();
    let count = events.len();
    let chain_ids =
        StringArray::from_iter_values(std::iter::repeat_n(block.chain_id().as_str(), count));
    let heights = UInt64Array::from_value(block.block_height().get(), count);
    let times = Int64Array::from_value(block.block_time().unix_micros(), count);
    let confirmations = StringArray::from_iter_values(std::iter::repeat_n(
        manifest::confirmation_name(block.confirmation_class()),
        count,
    ));
    let transaction_ids =
        StringArray::from_iter_values(events.iter().map(|event| event.transaction_id().as_str()));
    let transaction_indices =
        UInt32Array::from_iter_values(events.iter().map(|event| event.transaction_index()));
    let event_indices =
        UInt32Array::from_iter_values(events.iter().map(|event| event.canonical_event_index()));
    let event_ids =
        StringArray::from_iter_values(events.iter().map(|event| event.event_id().as_str()));
    let event_kinds =
        StringArray::from_iter_values(events.iter().map(|event| event.event_kind().as_wire_name()));
    let schema_versions =
        StringArray::from_iter_values(events.iter().map(|event| event.schema_version()));

    let mut block_hashes = FixedSizeBinaryBuilder::with_capacity(count, 32);
    let mut payload_hashes = FixedSizeBinaryBuilder::with_capacity(count, 32);
    let mut envelopes = BinaryBuilder::new();
    for event in events {
        block_hashes
            .append_value(block.canonical_block_hash())
            .map_err(|error| ArchiveError::Codec(error.to_string()))?;
        payload_hashes
            .append_value(event.payload_hash())
            .map_err(|error| ArchiveError::Codec(error.to_string()))?;
        let encoded = event
            .encode_to_vec()
            .map_err(|error| ArchiveError::Codec(error.to_string()))?;
        envelopes.append_value(encoded);
    }

    let columns: Vec<ArrayRef> = vec![
        Arc::new(chain_ids),
        Arc::new(heights),
        Arc::new(times),
        Arc::new(block_hashes.finish()),
        Arc::new(confirmations),
        Arc::new(transaction_ids),
        Arc::new(transaction_indices),
        Arc::new(event_indices),
        Arc::new(event_ids),
        Arc::new(event_kinds),
        Arc::new(schema_versions),
        Arc::new(payload_hashes.finish()),
        Arc::new(envelopes.finish()),
    ];
    RecordBatch::try_new(schema::canonical_schema(), columns)
        .map_err(|error| ArchiveError::Codec(error.to_string()))
}

fn publish_partition(
    archive: &LocalParquetArchive,
    block: &BlockEnvelope,
    previous: Option<&reader::LoadedManifest<PartitionManifestV1>>,
    dataset: &Path,
    partition: &str,
    block_reference: BlockManifestRefV1,
    durable_at: KnownTime,
) -> Result<reader::LoadedManifest<PartitionManifestV1>, ArchiveError> {
    let mut blocks = previous
        .map(|loaded| loaded.value.blocks.clone())
        .unwrap_or_default();
    blocks.push(block_reference);
    blocks.sort_by_key(|reference| reference.block_height);
    manifest::validate_block_ref_order(&blocks)?;
    let generation = previous
        .map(|loaded| {
            loaded
                .value
                .generation
                .checked_add(1)
                .ok_or(ArchiveError::InvalidInput(
                    "partition manifest generation overflows",
                ))
        })
        .transpose()?
        .unwrap_or(1);
    let value = PartitionManifestV1 {
        schema: PARTITION_MANIFEST_SCHEMA_V1.to_owned(),
        chain_id: block.chain_id().as_str().to_owned(),
        dataset: CANONICAL_DATASET.to_owned(),
        partition: partition.to_owned(),
        generation,
        producer_build_id: archive.config().producer_build_id().to_owned(),
        created_at_micros: durable_at.unix_micros(),
        previous_manifest_sha256: previous.map(|loaded| hex::encode(loaded.hash)),
        transition: PartitionTransitionV1::Append,
        blocks,
    };
    let bytes = manifest::canonical_json(&value)?;
    let hash = manifest::sha256(&bytes);
    let relative = dataset
        .join(partition)
        .join("manifests")
        .join(format!("partition-{}.json", hex::encode(hash)));
    fs::publish_immutable(archive.root(), &relative, &bytes)?;
    publish_pointer(
        archive.root(),
        &dataset.join(partition).join("CURRENT"),
        &relative,
        hash,
    )?;
    Ok(reader::LoadedManifest {
        value,
        hash,
        relative_path: relative,
    })
}

pub(crate) fn publish_catalog(
    archive: &LocalParquetArchive,
    block: &BlockEnvelope,
    previous: Option<&reader::LoadedManifest<CatalogManifestV1>>,
    partition: &str,
    partition_head: &reader::LoadedManifest<PartitionManifestV1>,
    durable_at: KnownTime,
) -> Result<reader::LoadedManifest<CatalogManifestV1>, ArchiveError> {
    let dataset = reader::dataset_relative(block.chain_id());
    if catalog_references_partition(previous, partition, Some(partition_head))? {
        return previous.cloned().ok_or(ArchiveError::ManifestVerification(
            "catalog reference unexpectedly missing",
        ));
    }
    let mut partitions = previous
        .map(|loaded| loaded.value.partitions.clone())
        .unwrap_or_default();
    partitions.insert(
        partition.to_owned(),
        PartitionManifestRefV1 {
            partition: partition.to_owned(),
            manifest_relative_path: path_string(&partition_head.relative_path)?,
            manifest_sha256: hex::encode(partition_head.hash),
        },
    );
    let generation = previous
        .map(|loaded| {
            loaded
                .value
                .generation
                .checked_add(1)
                .ok_or(ArchiveError::InvalidInput(
                    "catalog manifest generation overflows",
                ))
        })
        .transpose()?
        .unwrap_or(1);
    let value = CatalogManifestV1 {
        schema: CATALOG_MANIFEST_SCHEMA_V1.to_owned(),
        chain_id: block.chain_id().as_str().to_owned(),
        dataset: CANONICAL_DATASET.to_owned(),
        generation,
        producer_build_id: archive.config().producer_build_id().to_owned(),
        created_at_micros: durable_at.unix_micros(),
        previous_manifest_sha256: previous.map(|loaded| hex::encode(loaded.hash)),
        partitions,
    };
    let bytes = manifest::canonical_json(&value)?;
    let hash = manifest::sha256(&bytes);
    let relative = dataset
        .join("manifests")
        .join(format!("catalog-{}.json", hex::encode(hash)));
    fs::publish_immutable(archive.root(), &relative, &bytes)?;
    publish_pointer(archive.root(), &dataset.join("CURRENT"), &relative, hash)?;
    Ok(reader::LoadedManifest {
        value,
        hash,
        relative_path: relative,
    })
}

pub(crate) fn publish_pointer(
    root: &Path,
    pointer_relative: &Path,
    manifest_relative: &Path,
    manifest_hash: [u8; 32],
) -> Result<(), ArchiveError> {
    let pointer = CurrentPointerV1 {
        schema: CURRENT_POINTER_SCHEMA_V1.to_owned(),
        manifest_relative_path: path_string(manifest_relative)?,
        manifest_sha256: hex::encode(manifest_hash),
    };
    fs::publish_current(root, pointer_relative, &manifest::canonical_json(&pointer)?)
}

fn validate_catalog_partition_relationship(
    archive: &LocalParquetArchive,
    block: &BlockEnvelope,
    catalog: Option<&reader::LoadedManifest<CatalogManifestV1>>,
    partition_head: Option<&reader::LoadedManifest<PartitionManifestV1>>,
    partition: &str,
) -> Result<(), ArchiveError> {
    let Some(reference) = catalog.and_then(|loaded| loaded.value.partitions.get(partition)) else {
        return Ok(());
    };
    let expected = manifest::parse_hash(&reference.manifest_sha256)?;
    let head = partition_head.ok_or(ArchiveError::ManifestVerification(
        "catalog references a missing partition",
    ))?;
    if !reader::partition_chain_contains(archive, block.chain_id(), head, expected)? {
        return Err(ArchiveError::ManifestVerification(
            "partition current pointer does not descend from catalog",
        ));
    }
    Ok(())
}

fn catalog_references_partition(
    catalog: Option<&reader::LoadedManifest<CatalogManifestV1>>,
    partition: &str,
    partition_head: Option<&reader::LoadedManifest<PartitionManifestV1>>,
) -> Result<bool, ArchiveError> {
    let Some(reference) = catalog.and_then(|loaded| loaded.value.partitions.get(partition)) else {
        return Ok(false);
    };
    let Some(head) = partition_head else {
        return Ok(false);
    };
    Ok(
        manifest::parse_hash(&reference.manifest_sha256)? == head.hash
            && Path::new(&reference.manifest_relative_path) == head.relative_path,
    )
}

fn find_existing_block(
    partition: Option<&reader::LoadedManifest<PartitionManifestV1>>,
    height: u64,
) -> Option<&BlockManifestRefV1> {
    let partition = partition?;
    let index = partition
        .value
        .blocks
        .binary_search_by_key(&height, |reference| reference.block_height)
        .ok()?;
    partition.value.blocks.get(index)
}

fn receipt_from_block_manifest(
    block_manifest: &BlockManifestV1,
    manifest_hash: [u8; 32],
    block_height: u64,
) -> Result<ArchiveReceipt, ArchiveError> {
    let block = block_manifest
        .blocks
        .iter()
        .find(|block| block.block_height == block_height)
        .ok_or(ArchiveError::ManifestVerification(
            "block manifest does not contain receipt block",
        ))?;
    let manifest_id = manifest::manifest_id(manifest_hash)?;
    let durable_at = KnownTime::from_unix_micros(block_manifest.created_at_micros)
        .map_err(|_| ArchiveError::ManifestVerification("invalid archive durability time"))?;
    ArchiveReceipt::try_new(
        format!("archive-receipt-v1-{}", hex::encode(manifest_hash)),
        manifest_id,
        domain_types::BlockHeight::new(block.block_height),
        manifest::parse_hash(&block.canonical_block_blake3)?,
        manifest::parse_hash(&block_manifest.object.sha256)?,
        manifest_hash,
        manifest::parse_hash(&block_manifest.object.schema_fingerprint_sha256)?,
        durable_at,
    )
}

pub(crate) fn rolling_content_hash(blocks: &[BlockEnvelope]) -> Result<[u8; 32], ArchiveError> {
    let mut hasher = Sha256::new();
    for block in blocks {
        let descriptor = BlockDescriptorV1::from_block(block)?;
        let descriptor_bytes = manifest::canonical_json(&descriptor)?;
        let descriptor_length = u64::try_from(descriptor_bytes.len())
            .map_err(|_| ArchiveError::InvalidInput("block descriptor exceeds u64"))?;
        hasher.update(descriptor_length.to_be_bytes());
        hasher.update(descriptor_bytes);
        for event in block.events() {
            let encoded = event
                .encode_to_vec()
                .map_err(|error| ArchiveError::Codec(error.to_string()))?;
            let length = u64::try_from(encoded.len())
                .map_err(|_| ArchiveError::InvalidInput("canonical envelope exceeds u64"))?;
            hasher.update(length.to_be_bytes());
            hasher.update(encoded);
        }
    }
    Ok(hasher.finalize().into())
}

fn hash_file(file: &mut File) -> Result<([u8; 32], u64), ArchiveError> {
    file.seek(SeekFrom::Start(0))
        .map_err(|_| ArchiveError::Io("seeking staged Parquet object"))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut size = 0_u64;
    loop {
        let read_bytes = file
            .read(&mut buffer)
            .map_err(|_| ArchiveError::Io("hashing staged Parquet object"))?;
        if read_bytes == 0 {
            break;
        }
        let read_length = u64::try_from(read_bytes)
            .map_err(|_| ArchiveError::InvalidInput("Parquet read size exceeds u64"))?;
        size = size
            .checked_add(read_length)
            .ok_or(ArchiveError::InvalidInput("Parquet object size overflows"))?;
        hasher.update(&buffer[..read_bytes]);
    }
    Ok((hasher.finalize().into(), size))
}

fn verify_published_hash(
    root: &Path,
    relative: &Path,
    expected_hash: [u8; 32],
    expected_size: u64,
) -> Result<(), ArchiveError> {
    let (mut file, size) = fs::open_regular(root, relative, expected_size)?;
    let (hash, read_size) = hash_file(&mut file)?;
    if size != expected_size || read_size != expected_size || hash != expected_hash {
        return Err(ArchiveError::CorruptObject(
            relative.to_string_lossy().into_owned(),
        ));
    }
    Ok(())
}

pub(crate) fn path_string(path: &Path) -> Result<String, ArchiveError> {
    path.to_str()
        .map(str::to_owned)
        .ok_or(ArchiveError::UnsafePath)
}
