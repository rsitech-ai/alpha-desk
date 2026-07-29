use std::{collections::BTreeSet, path::Path};

use canonical_events::BlockEnvelope;
use domain_types::{BlockRange, ChainId, KnownTime};
use storage_ports::{ArchiveError, CompactionReceipt};

use super::{
    LocalParquetArchive, fs,
    manifest::{
        self, BLOCK_MANIFEST_SCHEMA_V1, BlockDescriptorV1, BlockManifestRefV1, BlockManifestV1,
        PARTITION_MANIFEST_SCHEMA_V1, PartitionManifestV1, PartitionTransitionV1,
    },
    reader, schema, writer,
};

pub fn compact_range(
    archive: &LocalParquetArchive,
    chain: &ChainId,
    range: BlockRange,
    completed_at: KnownTime,
) -> Result<CompactionReceipt, ArchiveError> {
    let requested_count = range
        .end_inclusive
        .get()
        .checked_sub(range.start_inclusive.get())
        .and_then(|span| span.checked_add(1))
        .ok_or(ArchiveError::InvalidInput("compaction range overflows"))?;
    if requested_count < 2 {
        return Err(ArchiveError::InvalidInput(
            "compaction requires at least two blocks",
        ));
    }
    let dataset = reader::dataset_relative(chain);
    let _process_lock = fs::open_writer_lock(archive.root(), &dataset.join(".writer.lock"))?;
    let blocks = reader::read_range(archive, chain, range)?.collect::<Result<Vec<_>, _>>()?;
    let first = blocks.first().ok_or(ArchiveError::RangeUnavailable)?;
    let partition = manifest::partition_for(first.block_time().unix_micros())?;
    for block in &blocks {
        if manifest::partition_for(block.block_time().unix_micros())? != partition {
            return Err(ArchiveError::InvalidInput(
                "compaction range crosses an archive partition",
            ));
        }
    }
    let catalog =
        reader::load_current_catalog(archive, chain)?.ok_or(ArchiveError::RangeUnavailable)?;
    let partition_head = reader::load_current_partition(archive, chain, &partition)?
        .ok_or(ArchiveError::RangeUnavailable)?;
    let references = references_for_range(&partition_head.value.blocks, range)?;

    let unique_inputs = references
        .iter()
        .map(|reference| manifest::parse_hash(&reference.manifest_sha256))
        .collect::<Result<BTreeSet<_>, _>>()?;
    if unique_inputs.len() == 1 {
        let manifest_hash = *unique_inputs
            .first()
            .ok_or(ArchiveError::RangeUnavailable)?;
        let manifest_id = manifest::manifest_id(manifest_hash)?;
        let verified = reader::verify_block_manifest(archive, &manifest_id)?;
        if verified.block_range() == range {
            return receipt_from_existing(archive, &references[0], range, &blocks);
        }
    }
    let input_object_count = u64::try_from(unique_inputs.len())
        .map_err(|_| ArchiveError::InvalidInput("compaction input count exceeds u64"))?;
    if input_object_count < 2 {
        return Err(ArchiveError::InvalidInput(
            "compaction range is already represented by one object",
        ));
    }

    let schema_fingerprint = schema::canonical_schema_fingerprint()?;
    let object =
        writer::write_parquet_object(archive, &blocks, &dataset, &partition, schema_fingerprint)?;
    let rolling_content_sha256 = writer::rolling_content_hash(&blocks)?;
    let block_manifest = BlockManifestV1 {
        schema: BLOCK_MANIFEST_SCHEMA_V1.to_owned(),
        producer_build_id: archive.config().producer_build_id().to_owned(),
        created_at_micros: completed_at.unix_micros(),
        input_object_count,
        rolling_content_sha256: hex::encode(rolling_content_sha256),
        blocks: blocks
            .iter()
            .map(BlockDescriptorV1::from_block)
            .collect::<Result<Vec<_>, _>>()?,
        object,
    };
    let manifest_bytes = manifest::canonical_json(&block_manifest)?;
    let manifest_hash = manifest::sha256(&manifest_bytes);
    let manifest_relative = reader::global_block_manifest_relative(manifest_hash);
    fs::publish_immutable(archive.root(), &manifest_relative, &manifest_bytes)?;

    let mut next_references = partition_head.value.blocks.clone();
    for reference in &mut next_references {
        if reference.block_height >= range.start_inclusive.get()
            && reference.block_height <= range.end_inclusive.get()
        {
            reference.manifest_relative_path = writer::path_string(&manifest_relative)?;
            reference.manifest_sha256 = hex::encode(manifest_hash);
        }
    }
    manifest::validate_block_ref_order(&next_references)?;
    let compacted_partition = publish_partition(
        archive,
        &partition_head,
        &dataset,
        &partition,
        next_references,
        completed_at,
    )?;
    writer::publish_catalog(
        archive,
        first,
        Some(&catalog),
        &partition,
        &compacted_partition,
        completed_at,
    )?;

    let manifest_id = manifest::manifest_id(manifest_hash)?;
    let verified = reader::verify_block_manifest(archive, &manifest_id)?;
    if verified.block_range() != range
        || verified.row_count() != block_manifest.object.row_count
        || reader::read_range(archive, chain, range)?.collect::<Result<Vec<_>, _>>()? != blocks
    {
        return Err(ArchiveError::ManifestVerification(
            "compacted generation does not reproduce its verified inputs",
        ));
    }
    CompactionReceipt::try_new(
        manifest_id,
        range,
        input_object_count,
        manifest::parse_hash(&block_manifest.object.sha256)?,
        block_manifest.object.row_count,
        rolling_content_sha256,
        completed_at,
    )
}

fn references_for_range(
    references: &[BlockManifestRefV1],
    range: BlockRange,
) -> Result<Vec<BlockManifestRefV1>, ArchiveError> {
    let selected = references
        .iter()
        .filter(|reference| {
            reference.block_height >= range.start_inclusive.get()
                && reference.block_height <= range.end_inclusive.get()
        })
        .cloned()
        .collect::<Vec<_>>();
    let expected = range
        .end_inclusive
        .get()
        .checked_sub(range.start_inclusive.get())
        .and_then(|span| span.checked_add(1))
        .ok_or(ArchiveError::InvalidInput("compaction range overflows"))?;
    if u64::try_from(selected.len()).ok() != Some(expected) {
        return Err(ArchiveError::RangeUnavailable);
    }
    Ok(selected)
}

fn publish_partition(
    archive: &LocalParquetArchive,
    previous: &reader::LoadedManifest<PartitionManifestV1>,
    dataset: &Path,
    partition: &str,
    blocks: Vec<BlockManifestRefV1>,
    completed_at: KnownTime,
) -> Result<reader::LoadedManifest<PartitionManifestV1>, ArchiveError> {
    let generation = previous
        .value
        .generation
        .checked_add(1)
        .ok_or(ArchiveError::InvalidInput(
            "partition manifest generation overflows",
        ))?;
    let value = PartitionManifestV1 {
        schema: PARTITION_MANIFEST_SCHEMA_V1.to_owned(),
        chain_id: previous.value.chain_id.clone(),
        dataset: previous.value.dataset.clone(),
        partition: partition.to_owned(),
        generation,
        producer_build_id: archive.config().producer_build_id().to_owned(),
        created_at_micros: completed_at.unix_micros(),
        previous_manifest_sha256: Some(hex::encode(previous.hash)),
        transition: PartitionTransitionV1::Compaction,
        blocks,
    };
    let bytes = manifest::canonical_json(&value)?;
    let hash = manifest::sha256(&bytes);
    let relative = dataset
        .join(partition)
        .join("manifests")
        .join(format!("partition-{}.json", hex::encode(hash)));
    fs::publish_immutable(archive.root(), &relative, &bytes)?;
    writer::publish_pointer(
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

fn receipt_from_existing(
    archive: &LocalParquetArchive,
    reference: &BlockManifestRefV1,
    range: BlockRange,
    blocks: &[BlockEnvelope],
) -> Result<CompactionReceipt, ArchiveError> {
    let loaded = reader::load_block_manifest_ref(archive, reference)?;
    let input_object_count = loaded.value.input_object_count;
    let completed_at = KnownTime::from_unix_micros(loaded.value.created_at_micros)
        .map_err(|_| ArchiveError::ManifestVerification("invalid compaction completion time"))?;
    CompactionReceipt::try_new(
        manifest::manifest_id(loaded.hash)?,
        range,
        input_object_count,
        manifest::parse_hash(&loaded.value.object.sha256)?,
        loaded.value.object.row_count,
        writer::rolling_content_hash(blocks)?,
        completed_at,
    )
}
