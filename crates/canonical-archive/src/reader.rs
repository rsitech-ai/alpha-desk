use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use arrow::{
    array::{
        Array, BinaryArray, FixedSizeBinaryArray, Int64Array, StringArray, UInt32Array, UInt64Array,
    },
    record_batch::RecordBatch,
};
use canonical_events::{BlockEnvelope, CanonicalEventEnvelope};
use domain_types::{BlockHeight, BlockRange, ChainId, ManifestId, ProtocolTime, SourceId};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use sha2::{Digest, Sha256};
use storage_ports::{ArchiveError, ArchiveObject, BlockIterator, VerifiedManifest};

use super::{
    LocalParquetArchive, fs,
    inspection::{ArchiveDataset, ArchiveInspection, InspectedObject},
    manifest::{
        self, BLOCK_MANIFEST_SCHEMA_V1, BlockDescriptorV1, BlockManifestRefV1, BlockManifestV1,
        CANONICAL_DATASET, CATALOG_MANIFEST_SCHEMA_V1, CURRENT_POINTER_SCHEMA_V1,
        CatalogManifestV1, CurrentPointerV1, PARTITION_MANIFEST_SCHEMA_V1, PartitionManifestRefV1,
        PartitionManifestV1, PartitionTransitionV1,
    },
    schema,
};

const CURRENT_FILE: &str = "CURRENT";

#[derive(Debug, Clone)]
pub(crate) struct LoadedManifest<T> {
    pub value: T,
    pub hash: [u8; 32],
    pub relative_path: PathBuf,
}

pub(crate) fn dataset_relative(chain: &ChainId) -> PathBuf {
    PathBuf::from(format!(
        "chain={}",
        manifest::encoded_component(chain.as_str())
    ))
    .join(format!("dataset={CANONICAL_DATASET}"))
}

pub(crate) fn load_current_catalog(
    archive: &LocalParquetArchive,
    chain: &ChainId,
) -> Result<Option<LoadedManifest<CatalogManifestV1>>, ArchiveError> {
    let dataset = dataset_relative(chain);
    let pointer = dataset.join(CURRENT_FILE);
    let Some(pointer) = load_pointer(archive.root(), &pointer)? else {
        return Ok(None);
    };
    let loaded = load_catalog_at(archive.root(), Path::new(&pointer.manifest_relative_path))?;
    let pointer_hash = manifest::parse_hash(&pointer.manifest_sha256)?;
    let expected_relative = dataset
        .join("manifests")
        .join(format!("catalog-{}.json", hex::encode(pointer_hash)));
    if loaded.hash != pointer_hash || loaded.relative_path != expected_relative {
        return Err(ArchiveError::ManifestVerification(
            "catalog current pointer does not bind its exact manifest path and hash",
        ));
    }
    validate_catalog(&loaded.value, chain)?;
    verify_catalog_chain(archive.root(), &dataset, &loaded)?;
    Ok(Some(loaded))
}

pub(crate) fn load_current_partition(
    archive: &LocalParquetArchive,
    chain: &ChainId,
    partition: &str,
) -> Result<Option<LoadedManifest<PartitionManifestV1>>, ArchiveError> {
    let dataset = dataset_relative(chain);
    let pointer_path = dataset.join(partition).join(CURRENT_FILE);
    let Some(pointer) = load_pointer(archive.root(), &pointer_path)? else {
        return Ok(None);
    };
    let loaded = load_partition_at(archive.root(), Path::new(&pointer.manifest_relative_path))?;
    let pointer_hash = manifest::parse_hash(&pointer.manifest_sha256)?;
    let expected_relative = dataset
        .join(partition)
        .join("manifests")
        .join(format!("partition-{}.json", hex::encode(pointer_hash)));
    if loaded.hash != pointer_hash || loaded.relative_path != expected_relative {
        return Err(ArchiveError::ManifestVerification(
            "partition current pointer does not bind its exact manifest path and hash",
        ));
    }
    validate_partition(&loaded.value, chain, partition)?;
    verify_partition_chain(archive.root(), &dataset, &loaded)?;
    Ok(Some(loaded))
}

pub(crate) fn load_block_manifest_ref(
    archive: &LocalParquetArchive,
    reference: &BlockManifestRefV1,
) -> Result<LoadedManifest<BlockManifestV1>, ArchiveError> {
    let loaded = load_block_at(archive.root(), Path::new(&reference.manifest_relative_path))?;
    let reference_hash = manifest::parse_hash(&reference.manifest_sha256)?;
    if loaded.hash != reference_hash
        || loaded.relative_path != global_block_manifest_relative(reference_hash)
    {
        return Err(ArchiveError::ManifestVerification(
            "block manifest reference does not bind its exact path and hash",
        ));
    }
    if !loaded.value.blocks.iter().any(|block| {
        block.block_height == reference.block_height
            && block.canonical_block_blake3 == reference.canonical_block_blake3
    }) {
        return Err(ArchiveError::ManifestVerification(
            "block manifest reference content mismatch",
        ));
    }
    Ok(loaded)
}

pub(crate) fn partition_chain_contains(
    archive: &LocalParquetArchive,
    chain: &ChainId,
    head: &LoadedManifest<PartitionManifestV1>,
    target: [u8; 32],
) -> Result<bool, ArchiveError> {
    let dataset = dataset_relative(chain);
    partition_chain_contains_root(archive.root(), &dataset, head, target)
}

fn partition_chain_contains_root(
    root: &Path,
    dataset: &Path,
    head: &LoadedManifest<PartitionManifestV1>,
    target: [u8; 32],
) -> Result<bool, ArchiveError> {
    let mut current = head.clone();
    loop {
        if current.hash == target {
            return Ok(true);
        }
        let Some(previous) = current.value.previous_manifest_sha256.as_deref() else {
            return Ok(false);
        };
        let previous_hash = manifest::parse_hash(previous)?;
        let relative = dataset
            .join(&current.value.partition)
            .join("manifests")
            .join(format!("partition-{}.json", hex::encode(previous_hash)));
        current = load_partition_at(root, &relative)?;
        if current.hash != previous_hash {
            return Err(ArchiveError::ManifestVerification(
                "partition manifest chain is broken",
            ));
        }
    }
}

pub fn verify_block_manifest(
    archive: &LocalParquetArchive,
    manifest_id: &ManifestId,
) -> Result<VerifiedManifest, ArchiveError> {
    let hash = manifest::hash_from_manifest_id(manifest_id)?;
    let relative = global_block_manifest_relative(hash);
    let loaded = load_block_at(archive.root(), &relative)?;
    if loaded.hash != hash {
        return Err(ArchiveError::ManifestVerification(
            "manifest ID does not match block manifest bytes",
        ));
    }
    let (blocks, object) = verify_and_decode_bundle(archive, &loaded.value)?;
    let first = blocks
        .first()
        .ok_or(ArchiveError::ManifestVerification("empty block bundle"))?;
    let last = blocks
        .last()
        .ok_or(ArchiveError::ManifestVerification("empty block bundle"))?;
    let range = BlockRange::new(first.block_height(), last.block_height())
        .map_err(|_| ArchiveError::ManifestVerification("invalid block range"))?;
    let schema_fingerprint = schema::canonical_schema_fingerprint()?;
    VerifiedManifest::try_new(
        manifest_id.clone(),
        first.chain_id().clone(),
        loaded.value.object.row_count,
        range,
        hash,
        None,
        BTreeMap::from([(CANONICAL_DATASET.to_owned(), schema_fingerprint)]),
        Vec::new(),
        vec![object],
    )
}

pub fn read_manifest_blocks(
    archive: &LocalParquetArchive,
    manifest_id: &ManifestId,
) -> Result<BlockIterator, ArchiveError> {
    let hash = manifest::hash_from_manifest_id(manifest_id)?;
    let relative = global_block_manifest_relative(hash);
    let loaded = load_block_at(archive.root(), &relative)?;
    if loaded.hash != hash {
        return Err(ArchiveError::ManifestVerification(
            "manifest ID does not match block manifest bytes",
        ));
    }
    let (blocks, _) = verify_and_decode_bundle(archive, &loaded.value)?;
    Ok(Box::new(blocks.into_iter().map(Ok)))
}

pub fn read_range(
    archive: &LocalParquetArchive,
    chain: &ChainId,
    range: BlockRange,
) -> Result<BlockIterator, ArchiveError> {
    let block_count = range
        .end_inclusive
        .get()
        .checked_sub(range.start_inclusive.get())
        .and_then(|span| span.checked_add(1))
        .ok_or(ArchiveError::InvalidInput("archive read range overflows"))?;
    if block_count > archive.config().max_read_blocks() {
        return Err(ArchiveError::InvalidInput(
            "archive read range exceeds configured block limit",
        ));
    }

    let catalog = load_current_catalog(archive, chain)?.ok_or(ArchiveError::RangeUnavailable)?;
    let mut references = Vec::new();
    for partition_reference in catalog.value.partitions.values() {
        let partition = load_partition_reference(archive, chain, partition_reference)?;
        for block in partition.value.blocks {
            if block.block_height >= range.start_inclusive.get()
                && block.block_height <= range.end_inclusive.get()
            {
                references.push(block);
            }
        }
    }
    references.sort_by_key(|block| block.block_height);
    if u64::try_from(references.len()).ok() != Some(block_count) {
        return Err(ArchiveError::RangeUnavailable);
    }
    for (offset, reference) in references.iter().enumerate() {
        let offset = u64::try_from(offset)
            .map_err(|_| ArchiveError::InvalidInput("archive range index exceeds u64"))?;
        let expected = range
            .start_inclusive
            .get()
            .checked_add(offset)
            .ok_or(ArchiveError::InvalidInput("archive range height overflows"))?;
        if reference.block_height != expected {
            return Err(ArchiveError::RangeUnavailable);
        }
    }

    let mut total_bytes = 0_u64;
    let mut blocks = Vec::with_capacity(references.len());
    let mut verified_bundles: BTreeMap<[u8; 32], Vec<BlockEnvelope>> = BTreeMap::new();
    for reference in &references {
        let manifest = load_block_manifest_ref(archive, reference)?;
        if let std::collections::btree_map::Entry::Vacant(entry) =
            verified_bundles.entry(manifest.hash)
        {
            total_bytes = total_bytes
                .checked_add(manifest.value.object.size_bytes)
                .ok_or(ArchiveError::InvalidInput(
                    "archive read byte count overflows",
                ))?;
            if total_bytes > archive.config().max_read_bytes() {
                return Err(ArchiveError::InvalidInput(
                    "archive read range exceeds configured byte limit",
                ));
            }
            let (bundle, _) = verify_and_decode_bundle(archive, &manifest.value)?;
            entry.insert(bundle);
        }
        let block = verified_bundles
            .get(&manifest.hash)
            .and_then(|bundle| {
                bundle
                    .iter()
                    .find(|block| block.block_height().get() == reference.block_height)
            })
            .ok_or(ArchiveError::ManifestVerification(
                "verified bundle does not contain referenced block",
            ))?;
        blocks.push(block.clone());
    }
    Ok(Box::new(blocks.into_iter().map(Ok)))
}

pub(crate) fn inspect_chain(
    archive: &LocalParquetArchive,
    chain: &ChainId,
) -> Result<Option<ArchiveInspection>, ArchiveError> {
    let Some(catalog) = load_current_catalog(archive, chain)? else {
        return Ok(None);
    };
    let mut block_count = 0_u64;
    let mut event_count = 0_u64;
    let mut seen_manifests = BTreeSet::new();
    let mut objects = Vec::new();
    for partition_reference in catalog.value.partitions.values() {
        let partition = load_partition_reference(archive, chain, partition_reference)?;
        block_count = block_count
            .checked_add(u64::try_from(partition.value.blocks.len()).map_err(|_| {
                ArchiveError::InvalidInput("archive partition block count exceeds u64")
            })?)
            .ok_or(ArchiveError::InvalidInput(
                "archive inspection block count overflows",
            ))?;
        for reference in &partition.value.blocks {
            let loaded = load_block_manifest_ref(archive, reference)?;
            if !seen_manifests.insert(loaded.hash) {
                continue;
            }
            let (_, object) = verify_and_decode_bundle(archive, &loaded.value)?;
            event_count =
                event_count
                    .checked_add(object.row_count())
                    .ok_or(ArchiveError::InvalidInput(
                        "archive inspection event count overflows",
                    ))?;
            objects.push(InspectedObject::new(
                ArchiveDataset::CanonicalEvents,
                object.relative_path().to_path_buf(),
                object.sha256(),
                object.size_bytes(),
                object.row_count(),
            ));
        }
    }
    Ok(Some(ArchiveInspection::canonical(
        block_count,
        event_count,
        objects,
    )))
}

pub(crate) fn global_block_manifest_relative(hash: [u8; 32]) -> PathBuf {
    PathBuf::from("_manifests")
        .join("blocks")
        .join(format!("manifest-{}.json", hex::encode(hash)))
}

fn load_partition_reference(
    archive: &LocalParquetArchive,
    chain: &ChainId,
    reference: &PartitionManifestRefV1,
) -> Result<LoadedManifest<PartitionManifestV1>, ArchiveError> {
    if reference.partition.is_empty() || reference.partition.contains("..") {
        return Err(ArchiveError::UnsafePath);
    }
    let loaded = load_partition_at(archive.root(), Path::new(&reference.manifest_relative_path))?;
    let reference_hash = manifest::parse_hash(&reference.manifest_sha256)?;
    let expected_relative = dataset_relative(chain)
        .join(&reference.partition)
        .join("manifests")
        .join(format!("partition-{}.json", hex::encode(reference_hash)));
    if loaded.hash != reference_hash || loaded.relative_path != expected_relative {
        return Err(ArchiveError::ManifestVerification(
            "catalog partition reference does not bind its exact path and hash",
        ));
    }
    validate_partition(&loaded.value, chain, &reference.partition)?;
    let dataset = dataset_relative(chain);
    verify_partition_chain(archive.root(), &dataset, &loaded)?;
    Ok(loaded)
}

fn load_pointer(root: &Path, relative: &Path) -> Result<Option<CurrentPointerV1>, ArchiveError> {
    match std::fs::symlink_metadata(root.join(relative)) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(ArchiveError::Io("inspecting current pointer")),
    }
    let bytes = fs::read_manifest(root, relative)?;
    let pointer: CurrentPointerV1 = serde_json::from_slice(&bytes)
        .map_err(|_| ArchiveError::ManifestVerification("invalid current pointer JSON"))?;
    if pointer.schema != CURRENT_POINTER_SCHEMA_V1 {
        return Err(ArchiveError::ManifestVerification(
            "unsupported current pointer schema",
        ));
    }
    fs::validate_relative(Path::new(&pointer.manifest_relative_path))?;
    manifest::parse_hash(&pointer.manifest_sha256)?;
    Ok(Some(pointer))
}

fn load_catalog_at(
    root: &Path,
    relative: &Path,
) -> Result<LoadedManifest<CatalogManifestV1>, ArchiveError> {
    load_json_manifest(root, relative, "catalog")
}

fn load_partition_at(
    root: &Path,
    relative: &Path,
) -> Result<LoadedManifest<PartitionManifestV1>, ArchiveError> {
    load_json_manifest(root, relative, "partition")
}

fn load_block_at(
    root: &Path,
    relative: &Path,
) -> Result<LoadedManifest<BlockManifestV1>, ArchiveError> {
    let loaded: LoadedManifest<BlockManifestV1> = load_json_manifest(root, relative, "block")?;
    if loaded.value.schema != BLOCK_MANIFEST_SCHEMA_V1 {
        return Err(ArchiveError::ManifestVerification(
            "unsupported block manifest schema",
        ));
    }
    if loaded.value.blocks.is_empty() {
        return Err(ArchiveError::ManifestVerification(
            "block manifest bundle is empty",
        ));
    }
    if loaded.value.input_object_count == 0 {
        return Err(ArchiveError::ManifestVerification(
            "block manifest input object count is zero",
        ));
    }
    let mut expected_height = loaded.value.blocks[0].block_height;
    let mut row_count = 0_u64;
    let first_partition = manifest::partition_for(loaded.value.blocks[0].block_time_micros)?;
    let first_chain = loaded.value.blocks[0].chain_id.clone();
    for block in &loaded.value.blocks {
        validate_block_descriptor(block)?;
        if block.chain_id != first_chain
            || manifest::partition_for(block.block_time_micros)? != first_partition
            || block.block_height != expected_height
        {
            return Err(ArchiveError::ManifestVerification(
                "block bundle is not contiguous in one chain partition",
            ));
        }
        expected_height =
            expected_height
                .checked_add(1)
                .ok_or(ArchiveError::ManifestVerification(
                    "block bundle height overflows",
                ))?;
        row_count =
            row_count
                .checked_add(block.event_count)
                .ok_or(ArchiveError::ManifestVerification(
                    "block bundle row count overflows",
                ))?;
    }
    fs::validate_relative(Path::new(&loaded.value.object.relative_path))?;
    manifest::parse_hash(&loaded.value.object.sha256)?;
    manifest::parse_hash(&loaded.value.object.schema_fingerprint_sha256)?;
    manifest::parse_hash(&loaded.value.rolling_content_sha256)?;
    if loaded.value.object.size_bytes == 0 || loaded.value.object.row_count != row_count {
        return Err(ArchiveError::ManifestVerification(
            "block object counts are invalid",
        ));
    }
    Ok(loaded)
}

fn load_json_manifest<T>(
    root: &Path,
    relative: &Path,
    kind: &'static str,
) -> Result<LoadedManifest<T>, ArchiveError>
where
    T: serde::de::DeserializeOwned,
{
    fs::validate_relative(relative)?;
    let bytes = fs::read_manifest(root, relative)?;
    let hash = manifest::sha256(&bytes);
    let value = serde_json::from_slice(&bytes).map_err(|_| {
        ArchiveError::ManifestVerification(match kind {
            "catalog" => "invalid catalog manifest JSON",
            "partition" => "invalid partition manifest JSON",
            _ => "invalid block manifest JSON",
        })
    })?;
    Ok(LoadedManifest {
        value,
        hash,
        relative_path: relative.to_path_buf(),
    })
}

fn validate_catalog(catalog: &CatalogManifestV1, chain: &ChainId) -> Result<(), ArchiveError> {
    if catalog.schema != CATALOG_MANIFEST_SCHEMA_V1
        || catalog.chain_id != chain.as_str()
        || catalog.dataset != CANONICAL_DATASET
        || catalog.producer_build_id.is_empty()
        || catalog.created_at_micros < 0
    {
        return Err(ArchiveError::ManifestVerification(
            "catalog manifest metadata is invalid",
        ));
    }
    if let Some(previous) = &catalog.previous_manifest_sha256 {
        manifest::parse_hash(previous)?;
    }
    for (partition, reference) in &catalog.partitions {
        if partition != &reference.partition || partition.is_empty() || partition.contains("..") {
            return Err(ArchiveError::ManifestVerification(
                "catalog partition reference is invalid",
            ));
        }
        fs::validate_relative(Path::new(&reference.manifest_relative_path))?;
        manifest::parse_hash(&reference.manifest_sha256)?;
    }
    Ok(())
}

fn validate_partition(
    partition: &PartitionManifestV1,
    chain: &ChainId,
    expected_partition: &str,
) -> Result<(), ArchiveError> {
    if partition.schema != PARTITION_MANIFEST_SCHEMA_V1
        || partition.chain_id != chain.as_str()
        || partition.dataset != CANONICAL_DATASET
        || partition.partition != expected_partition
        || partition.producer_build_id.is_empty()
        || partition.created_at_micros < 0
    {
        return Err(ArchiveError::ManifestVerification(
            "partition manifest metadata is invalid",
        ));
    }
    if let Some(previous) = &partition.previous_manifest_sha256 {
        manifest::parse_hash(previous)?;
    }
    manifest::validate_block_ref_order(&partition.blocks)?;
    for block in &partition.blocks {
        fs::validate_relative(Path::new(&block.manifest_relative_path))?;
    }
    Ok(())
}

fn verify_catalog_chain(
    root: &Path,
    dataset: &Path,
    head: &LoadedManifest<CatalogManifestV1>,
) -> Result<(), ArchiveError> {
    let mut seen = BTreeSet::new();
    let mut current = head.clone();
    loop {
        if !seen.insert(current.hash) {
            return Err(ArchiveError::ManifestVerification(
                "catalog manifest chain contains a cycle",
            ));
        }
        let Some(previous) = current.value.previous_manifest_sha256.as_deref() else {
            if current.value.generation != 1 {
                return Err(ArchiveError::ManifestVerification(
                    "catalog root generation is invalid",
                ));
            }
            return Ok(());
        };
        let previous_hash = manifest::parse_hash(previous)?;
        let relative = dataset
            .join("manifests")
            .join(format!("catalog-{}.json", hex::encode(previous_hash)));
        let loaded = load_catalog_at(root, &relative)?;
        let chain = ChainId::new(current.value.chain_id.clone())
            .map_err(|_| ArchiveError::ManifestVerification("invalid catalog chain ID"))?;
        validate_catalog(&loaded.value, &chain)?;
        if loaded.hash != previous_hash
            || loaded.value.generation.checked_add(1) != Some(current.value.generation)
            || loaded.value.chain_id != current.value.chain_id
            || loaded.value.dataset != current.value.dataset
            || !catalog_transition_is_append_only(&loaded.value, &current.value)
            || !catalog_partition_advance_descends(root, dataset, &loaded.value, &current.value)?
        {
            return Err(ArchiveError::ManifestVerification(
                "catalog manifest chain is broken",
            ));
        }
        current = loaded;
    }
}

fn catalog_partition_advance_descends(
    root: &Path,
    dataset: &Path,
    previous: &CatalogManifestV1,
    current: &CatalogManifestV1,
) -> Result<bool, ArchiveError> {
    let Some((partition, current_reference)) =
        current.partitions.iter().find(|(partition, value)| {
            previous
                .partitions
                .get(*partition)
                .is_some_and(|prior| prior != *value)
        })
    else {
        return Ok(true);
    };
    let previous_reference =
        previous
            .partitions
            .get(partition)
            .ok_or(ArchiveError::ManifestVerification(
                "catalog transition lost a partition reference",
            ))?;
    let current_hash = manifest::parse_hash(&current_reference.manifest_sha256)?;
    let expected_relative = dataset
        .join(partition)
        .join("manifests")
        .join(format!("partition-{}.json", hex::encode(current_hash)));
    if Path::new(&current_reference.manifest_relative_path) != expected_relative {
        return Ok(false);
    }
    let head = load_partition_at(root, &expected_relative)?;
    if head.hash != current_hash {
        return Ok(false);
    }
    partition_chain_contains_root(
        root,
        dataset,
        &head,
        manifest::parse_hash(&previous_reference.manifest_sha256)?,
    )
}

fn verify_partition_chain(
    root: &Path,
    dataset: &Path,
    head: &LoadedManifest<PartitionManifestV1>,
) -> Result<(), ArchiveError> {
    let mut seen = BTreeSet::new();
    let mut current = head.clone();
    loop {
        if !seen.insert(current.hash) {
            return Err(ArchiveError::ManifestVerification(
                "partition manifest chain contains a cycle",
            ));
        }
        let Some(previous) = current.value.previous_manifest_sha256.as_deref() else {
            if current.value.generation != 1
                || current.value.transition != PartitionTransitionV1::Append
            {
                return Err(ArchiveError::ManifestVerification(
                    "partition root generation is invalid",
                ));
            }
            return Ok(());
        };
        let previous_hash = manifest::parse_hash(previous)?;
        let relative = dataset
            .join(&current.value.partition)
            .join("manifests")
            .join(format!("partition-{}.json", hex::encode(previous_hash)));
        let loaded = load_partition_at(root, &relative)?;
        let chain = ChainId::new(current.value.chain_id.clone())
            .map_err(|_| ArchiveError::ManifestVerification("invalid partition chain ID"))?;
        validate_partition(&loaded.value, &chain, &current.value.partition)?;
        if loaded.hash != previous_hash
            || loaded.value.generation.checked_add(1) != Some(current.value.generation)
            || loaded.value.chain_id != current.value.chain_id
            || loaded.value.partition != current.value.partition
            || !partition_transition_is_valid(&loaded.value, &current.value)
        {
            return Err(ArchiveError::ManifestVerification(
                "partition manifest chain is broken",
            ));
        }
        current = loaded;
    }
}

fn validate_block_descriptor(block: &BlockDescriptorV1) -> Result<(), ArchiveError> {
    if block.chain_id.is_empty()
        || block.block_time_micros < 0
        || block.confirmation_class.is_empty()
        || block.source_block_hashes_blake3.is_empty()
    {
        return Err(ArchiveError::ManifestVerification(
            "block manifest metadata is invalid",
        ));
    }
    manifest::parse_hash(&block.canonical_block_blake3)?;
    manifest::parse_confirmation(&block.confirmation_class)?;
    for (source, hash) in &block.source_block_hashes_blake3 {
        SourceId::new(source.clone()).map_err(|_| {
            ArchiveError::ManifestVerification("block manifest source ID is invalid")
        })?;
        manifest::parse_hash(hash)?;
    }
    Ok(())
}

pub(crate) fn verify_and_decode_bundle(
    archive: &LocalParquetArchive,
    manifest_value: &BlockManifestV1,
) -> Result<(Vec<BlockEnvelope>, ArchiveObject), ArchiveError> {
    let first = manifest_value
        .blocks
        .first()
        .ok_or(ArchiveError::ManifestVerification("empty block bundle"))?;
    let last = manifest_value
        .blocks
        .last()
        .ok_or(ArchiveError::ManifestVerification("empty block bundle"))?;
    let object_relative = Path::new(&manifest_value.object.relative_path);
    let object_hash = manifest::parse_hash(&manifest_value.object.sha256)?;
    let chain = ChainId::new(first.chain_id.clone())
        .map_err(|_| ArchiveError::ManifestVerification("invalid block chain ID"))?;
    let partition = manifest::partition_for(first.block_time_micros)?;
    let expected_object_relative = dataset_relative(&chain)
        .join(partition)
        .join("objects")
        .join(format!("block_start={}", first.block_height))
        .join(format!("block_end={}", last.block_height))
        .join(format!("part-{}.parquet", hex::encode(object_hash)));
    if object_relative != expected_object_relative {
        return Err(ArchiveError::ManifestVerification(
            "block manifest does not bind the exact object path",
        ));
    }
    let object_bytes = fs::read_regular(
        archive.root(),
        object_relative,
        archive.config().max_read_bytes(),
    )
    .map_err(|error| match error {
        ArchiveError::Io(_) => {
            ArchiveError::CorruptObject(manifest_value.object.relative_path.clone())
        }
        other => other,
    })?;
    let actual_size = u64::try_from(object_bytes.len())
        .map_err(|_| ArchiveError::InvalidInput("archive object exceeds u64"))?;
    let actual_hash: [u8; 32] = Sha256::digest(&object_bytes).into();
    if actual_size != manifest_value.object.size_bytes || actual_hash != object_hash {
        return Err(ArchiveError::CorruptObject(
            manifest_value.object.relative_path.clone(),
        ));
    }
    let expected_schema_fingerprint = schema::canonical_schema_fingerprint()?;
    if expected_schema_fingerprint
        != manifest::parse_hash(&manifest_value.object.schema_fingerprint_sha256)?
    {
        return Err(ArchiveError::SchemaMismatch);
    }

    let bytes = bytes::Bytes::from(object_bytes);
    let builder = ParquetRecordBatchReaderBuilder::try_new(bytes)
        .map_err(|_| ArchiveError::CorruptObject(manifest_value.object.relative_path.clone()))?;
    if builder.schema().fields() != schema::canonical_schema().fields() {
        return Err(ArchiveError::SchemaMismatch);
    }
    let reader = builder
        .build()
        .map_err(|_| ArchiveError::CorruptObject(manifest_value.object.relative_path.clone()))?;
    let mut events_by_height: BTreeMap<u64, Vec<CanonicalEventEnvelope>> = BTreeMap::new();
    for batch in reader {
        let batch = batch.map_err(|_| {
            ArchiveError::CorruptObject(manifest_value.object.relative_path.clone())
        })?;
        decode_bundle_batch(&batch, &manifest_value.blocks, &mut events_by_height)?;
    }
    let mut blocks = Vec::with_capacity(manifest_value.blocks.len());
    for descriptor in &manifest_value.blocks {
        let events = events_by_height
            .remove(&descriptor.block_height)
            .unwrap_or_default();
        if u64::try_from(events.len()).ok() != Some(descriptor.event_count) {
            return Err(ArchiveError::ManifestVerification(
                "Parquet event count does not match block descriptor",
            ));
        }
        blocks.push(reconstruct_block(descriptor, events)?);
    }
    if !events_by_height.is_empty()
        || super::writer::rolling_content_hash(&blocks)?
            != manifest::parse_hash(&manifest_value.rolling_content_sha256)?
    {
        return Err(ArchiveError::ManifestVerification(
            "block bundle rolling content hash mismatch",
        ));
    }

    let range = BlockRange::new(
        BlockHeight::new(first.block_height),
        BlockHeight::new(last.block_height),
    )
    .map_err(|_| ArchiveError::ManifestVerification("invalid object block range"))?;
    let object = ArchiveObject::try_new(
        object_relative.to_path_buf(),
        actual_hash,
        actual_size,
        manifest_value.object.row_count,
        range,
    )?;
    Ok((blocks, object))
}

fn reconstruct_block(
    descriptor: &BlockDescriptorV1,
    events: Vec<CanonicalEventEnvelope>,
) -> Result<BlockEnvelope, ArchiveError> {
    let chain = ChainId::new(descriptor.chain_id.clone())
        .map_err(|_| ArchiveError::ManifestVerification("invalid block chain ID"))?;
    let block_time = ProtocolTime::from_unix_micros(descriptor.block_time_micros)
        .map_err(|_| ArchiveError::ManifestVerification("invalid block time"))?;
    let source_hashes = descriptor
        .source_block_hashes_blake3
        .iter()
        .map(|(source, hash)| {
            Ok((
                SourceId::new(source.clone())
                    .map_err(|_| ArchiveError::ManifestVerification("invalid source ID"))?,
                manifest::parse_hash(hash)?,
            ))
        })
        .collect::<Result<BTreeMap<_, _>, ArchiveError>>()?;
    let block = BlockEnvelope::try_new(
        chain,
        BlockHeight::new(descriptor.block_height),
        block_time,
        manifest::parse_confirmation(&descriptor.confirmation_class)?,
        events,
        source_hashes,
    )
    .map_err(|error| ArchiveError::Codec(error.to_string()))?;
    if block.canonical_block_hash() != manifest::parse_hash(&descriptor.canonical_block_blake3)? {
        return Err(ArchiveError::ManifestVerification(
            "reconstructed canonical block hash mismatch",
        ));
    }
    Ok(block)
}

fn decode_bundle_batch(
    batch: &RecordBatch,
    blocks: &[BlockDescriptorV1],
    events: &mut BTreeMap<u64, Vec<CanonicalEventEnvelope>>,
) -> Result<(), ArchiveError> {
    let chain = column::<StringArray>(batch, 0)?;
    let heights = column::<UInt64Array>(batch, 1)?;
    let times = column::<Int64Array>(batch, 2)?;
    let block_hashes = column::<FixedSizeBinaryArray>(batch, 3)?;
    let confirmations = column::<StringArray>(batch, 4)?;
    let transaction_ids = column::<StringArray>(batch, 5)?;
    let transaction_indices = column::<UInt32Array>(batch, 6)?;
    let event_indices = column::<UInt32Array>(batch, 7)?;
    let event_ids = column::<StringArray>(batch, 8)?;
    let event_kinds = column::<StringArray>(batch, 9)?;
    let schema_versions = column::<StringArray>(batch, 10)?;
    let payload_hashes = column::<FixedSizeBinaryArray>(batch, 11)?;
    let envelopes = column::<BinaryArray>(batch, 12)?;

    for row in 0..batch.num_rows() {
        let height = heights.value(row);
        let block = blocks
            .binary_search_by_key(&height, |candidate| candidate.block_height)
            .ok()
            .and_then(|index| blocks.get(index))
            .ok_or(ArchiveError::ManifestVerification(
                "Parquet row references a block outside its bundle",
            ))?;
        let event = CanonicalEventEnvelope::decode(envelopes.value(row))
            .map_err(|error| ArchiveError::Codec(error.to_string()))?;
        if chain.value(row) != block.chain_id
            || heights.value(row) != block.block_height
            || times.value(row) != block.block_time_micros
            || block_hashes.value(row) != manifest::parse_hash(&block.canonical_block_blake3)?
            || confirmations.value(row) != block.confirmation_class
            || transaction_ids.value(row) != event.transaction_id().as_str()
            || transaction_indices.value(row) != event.transaction_index()
            || event_indices.value(row) != event.canonical_event_index()
            || event_ids.value(row) != event.event_id().as_str()
            || event_kinds.value(row) != event.event_kind().as_wire_name()
            || schema_versions.value(row) != event.schema_version()
            || payload_hashes.value(row) != event.payload_hash()
        {
            return Err(ArchiveError::ManifestVerification(
                "Parquet query columns disagree with the canonical envelope",
            ));
        }
        events.entry(height).or_default().push(event);
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

fn partition_transition_is_valid(
    previous: &PartitionManifestV1,
    current: &PartitionManifestV1,
) -> bool {
    match current.transition {
        PartitionTransitionV1::Append => {
            if current.blocks.len() != previous.blocks.len().saturating_add(1) {
                return false;
            }
            previous.blocks.iter().all(|reference| {
                current
                    .blocks
                    .binary_search_by_key(&reference.block_height, |candidate| {
                        candidate.block_height
                    })
                    .ok()
                    .and_then(|index| current.blocks.get(index))
                    == Some(reference)
            })
        }
        PartitionTransitionV1::Compaction => {
            current.blocks.len() == previous.blocks.len()
                && current
                    .blocks
                    .iter()
                    .zip(&previous.blocks)
                    .all(|(new, old)| {
                        new.block_height == old.block_height
                            && new.canonical_block_blake3 == old.canonical_block_blake3
                    })
                && current.blocks != previous.blocks
        }
    }
}

fn catalog_transition_is_append_only(
    previous: &CatalogManifestV1,
    current: &CatalogManifestV1,
) -> bool {
    if current.partitions.len() < previous.partitions.len()
        || current.partitions.len() > previous.partitions.len().saturating_add(1)
    {
        return false;
    }
    let changed = previous
        .partitions
        .iter()
        .filter(|(partition, reference)| current.partitions.get(*partition) != Some(*reference))
        .count();
    let added = current
        .partitions
        .keys()
        .filter(|partition| !previous.partitions.contains_key(*partition))
        .count();
    changed + added == 1
}
