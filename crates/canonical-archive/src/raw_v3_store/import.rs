use std::path::{Path, PathBuf};

use domain_types::{ChainId, ManifestId, SourceId};
use hl_protocol::SourceObservation;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use storage_ports::{
    ArchiveError, LocalRecordSequence, LocalRecordSequenceRange,
    RAW_ARCHIVE_MAXIMUM_DATA_PACK_BYTES, RAW_ARCHIVE_MAXIMUM_PACK_LOGICAL_INPUTS,
    RawArchiveCheckpointEntriesV2, RawArchiveCheckpointEntryV2,
};

use super::{
    RawV3Archive, dataset_relative, journal_relative, load_current_root, load_import_root,
    load_packs_for_tree, pack_manifest_relative, replay_root_by_sequence, root_pointer_bytes,
    root_relative, walk_logical_leaves, write_packed_object,
};
use crate::{
    LocalParquetArchive, fs, manifest, raw, raw_policy, raw_v2,
    raw_v3::{
        self, IndexPackBytes, JournalGenerationBuilderV3, MAX_JOURNAL_BYTES, PackedLogicalInputV3,
        RAW_BYTE_DATASET_V3, RawPackManifestV3, RootBundleV3, SequenceLeafEntryV3,
        SequenceNodeRefV3, SequenceStorageRefV3, append_logical_entry, journal_file_identity,
        journal_needs_rotation, root_bundle_hash,
    },
};

const IMPORT_PLAN_SCHEMA: &str = "hyperliquid-alpha-desk/archive-raw-v2-import-plan/v1";
const IMPORT_REPORT_SCHEMA: &str = "hyperliquid-alpha-desk/archive-raw-v2-import-report/v1";
const CUTOVER_SCHEMA: &str = "hyperliquid-alpha-desk/archive-raw-cutover/v1";
const PARITY_HASH_DOMAIN: &[u8] = b"hyperliquid-alpha-desk/raw-v2-import-parity/v1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawV2ImportReceipt {
    manifest_id: ManifestId,
    manifest_sha256: [u8; 32],
    first_local_sequence: u64,
    last_local_sequence: u64,
    partition: String,
    row_count: u64,
}

impl RawV2ImportReceipt {
    #[must_use]
    pub const fn manifest_id(&self) -> &ManifestId {
        &self.manifest_id
    }

    #[must_use]
    pub const fn manifest_sha256(&self) -> [u8; 32] {
        self.manifest_sha256
    }

    #[must_use]
    pub const fn first_local_sequence(&self) -> u64 {
        self.first_local_sequence
    }

    #[must_use]
    pub const fn last_local_sequence(&self) -> u64 {
        self.last_local_sequence
    }

    #[must_use]
    pub fn partition(&self) -> &str {
        &self.partition
    }

    #[must_use]
    pub const fn row_count(&self) -> u64 {
        self.row_count
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawV2ImportPlan {
    sha256: [u8; 32],
    chain_id: ChainId,
    source_id: SourceId,
    v2_catalog_generation: u64,
    v2_catalog_sha256: [u8; 32],
    first_local_sequence: u64,
    last_local_sequence: u64,
    batches: Vec<RawV2ImportReceipt>,
}

impl RawV2ImportPlan {
    #[must_use]
    pub const fn sha256(&self) -> [u8; 32] {
        self.sha256
    }

    #[must_use]
    pub const fn chain_id(&self) -> &ChainId {
        &self.chain_id
    }

    #[must_use]
    pub const fn source_id(&self) -> &SourceId {
        &self.source_id
    }

    #[must_use]
    pub const fn v2_catalog_generation(&self) -> u64 {
        self.v2_catalog_generation
    }

    #[must_use]
    pub const fn v2_catalog_sha256(&self) -> [u8; 32] {
        self.v2_catalog_sha256
    }

    #[must_use]
    pub const fn first_local_sequence(&self) -> u64 {
        self.first_local_sequence
    }

    #[must_use]
    pub const fn last_local_sequence(&self) -> u64 {
        self.last_local_sequence
    }

    #[must_use]
    pub fn batches(&self) -> &[RawV2ImportReceipt] {
        &self.batches
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawV2ImportReport {
    plan_sha256: [u8; 32],
    v3_root_sha256: [u8; 32],
    pack_count: u64,
    packed_logical_manifest_count: u64,
    parity_digest: [u8; 32],
}

impl RawV2ImportReport {
    #[must_use]
    pub const fn plan_sha256(&self) -> [u8; 32] {
        self.plan_sha256
    }

    #[must_use]
    pub const fn v3_root_sha256(&self) -> [u8; 32] {
        self.v3_root_sha256
    }

    #[must_use]
    pub const fn pack_count(&self) -> u64 {
        self.pack_count
    }

    #[must_use]
    pub const fn packed_logical_manifest_count(&self) -> u64 {
        self.packed_logical_manifest_count
    }

    #[must_use]
    pub const fn parity_digest(&self) -> [u8; 32] {
        self.parity_digest
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawV2ImportApproval {
    plan_sha256: [u8; 32],
    v3_root_sha256: [u8; 32],
    checkpoint_sha256: [u8; 32],
    parity_digest: [u8; 32],
}

impl RawV2ImportApproval {
    #[must_use]
    pub const fn plan_sha256(&self) -> [u8; 32] {
        self.plan_sha256
    }

    #[must_use]
    pub const fn v3_root_sha256(&self) -> [u8; 32] {
        self.v3_root_sha256
    }

    #[must_use]
    pub const fn checkpoint_sha256(&self) -> [u8; 32] {
        self.checkpoint_sha256
    }

    #[must_use]
    pub const fn parity_digest(&self) -> [u8; 32] {
        self.parity_digest
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
struct ImportBatchDocument {
    manifest_id: String,
    manifest_sha256: String,
    first_local_sequence: u64,
    last_local_sequence: u64,
    partition: String,
    row_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
struct ImportPlanDocument {
    schema: String,
    chain_id: String,
    source_id: String,
    v2_catalog_generation: u64,
    v2_catalog_sha256: String,
    first_local_sequence: u64,
    last_local_sequence: u64,
    batches: Vec<ImportBatchDocument>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
struct ImportReportDocument {
    schema: String,
    plan_sha256: String,
    v3_root_sha256: String,
    pack_count: u64,
    packed_logical_manifest_count: u64,
    parity_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CutoverDocument {
    schema: String,
    chain_id: String,
    source_id: String,
    from_dataset: String,
    to_dataset: String,
    v2_catalog_sha256: String,
    v3_root_sha256: String,
    parity_digest: String,
}

pub fn plan_v2_import(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
) -> Result<RawV2ImportPlan, ArchiveError> {
    let (generation, catalog_hash, batches) = load_v2_batches(archive, chain, source)?;
    let plan = plan_from_batches(chain, source, generation, catalog_hash, &batches)?;
    persist_document(archive, &plan_relative(plan.sha256), &plan_document(&plan)?)?;
    Ok(plan)
}

pub fn publish_v2_import(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
    plan: &RawV2ImportPlan,
) -> Result<RawV2ImportReport, ArchiveError> {
    reject_foreign_plan(plan, chain, source)?;
    let (generation, catalog_hash, batches) = load_v2_batches(archive, chain, source)?;
    bind_plan_to_catalog(plan, generation, catalog_hash, &batches)?;
    if let Some((root, journal_bytes)) = load_import_root(archive, chain, source)? {
        return report_for_import(
            archive,
            chain,
            source,
            plan,
            &root,
            &journal_bytes,
            &batches,
        );
    }
    if load_current_root(archive, chain, source)?.is_some() {
        return Err(ArchiveError::ManifestVerification(
            "raw V3 CURRENT exists without a matching IMPORT candidate",
        ));
    }
    persist_document(archive, &plan_relative(plan.sha256), &plan_document(plan)?)?;
    let durable_at = archive.config().now()?;
    let packed_entries = write_import_packs(archive, chain, source, &batches, durable_at)?;
    let (root, journal_bytes) =
        publish_import_root(archive, chain, source, packed_entries, durable_at)?;
    let report = report_for_import(
        archive,
        chain,
        source,
        plan,
        &root,
        &journal_bytes,
        &batches,
    )?;
    persist_document(
        archive,
        &report_relative(plan.sha256),
        &report_document(&report)?,
    )?;
    Ok(report)
}

pub fn approve_v2_import(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
    plan: &RawV2ImportPlan,
) -> Result<RawV2ImportApproval, ArchiveError> {
    reject_foreign_plan(plan, chain, source)?;
    let (generation, catalog_hash, batches) = load_v2_batches(archive, chain, source)?;
    bind_plan_to_catalog(plan, generation, catalog_hash, &batches)?;
    let (root, journal_bytes) = load_import_root(archive, chain, source)?.ok_or(
        ArchiveError::ManifestVerification("raw V2 import candidate is missing"),
    )?;
    let report = report_for_import(
        archive,
        chain,
        source,
        plan,
        &root,
        &journal_bytes,
        &batches,
    )?;
    let checkpoint_sha256 = super::checkpoint::publish_checkpoint_v2_on(
        archive,
        chain,
        source,
        &root,
        &journal_bytes,
        checkpoint_entries(plan)?,
    )?;
    publish_v3_current(archive, chain, source, report.v3_root_sha256())?;
    publish_cutover(archive, chain, source, plan, &report)?;
    switch_checkpoint(archive, chain, source, checkpoint_sha256)?;
    super::hint::rebuild_receipt_hints(archive, chain, source)?;
    Ok(RawV2ImportApproval {
        plan_sha256: plan.sha256(),
        v3_root_sha256: report.v3_root_sha256(),
        checkpoint_sha256,
        parity_digest: report.parity_digest(),
    })
}

fn load_v2_batches(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
) -> Result<(u64, [u8; 32], Vec<raw_v2::VerifiedV2ImportBatch>), ArchiveError> {
    let v2 = LocalParquetArchive::open(archive.root(), archive.config().clone())?;
    raw_v2::load_verified_import_batches(&v2, chain, source)
}

fn plan_from_batches(
    chain: &ChainId,
    source: &SourceId,
    generation: u64,
    catalog_hash: [u8; 32],
    batches: &[raw_v2::VerifiedV2ImportBatch],
) -> Result<RawV2ImportPlan, ArchiveError> {
    if batches.is_empty() {
        return Err(ArchiveError::InvalidInput(
            "raw V2 import requires at least one verified batch",
        ));
    }
    let receipts = batches
        .iter()
        .map(receipt_from_batch)
        .collect::<Result<Vec<_>, ArchiveError>>()?;
    let first_local_sequence = receipts[0].first_local_sequence;
    let last_local_sequence = receipts
        .last()
        .ok_or(ArchiveError::InvalidInput(
            "raw V2 import requires at least one verified batch",
        ))?
        .last_local_sequence;
    let plan = RawV2ImportPlan {
        sha256: [0; 32],
        chain_id: chain.clone(),
        source_id: source.clone(),
        v2_catalog_generation: generation,
        v2_catalog_sha256: catalog_hash,
        first_local_sequence,
        last_local_sequence,
        batches: receipts,
    };
    let bytes = plan_document(&plan)?;
    Ok(RawV2ImportPlan {
        sha256: manifest::sha256(&bytes),
        ..plan
    })
}

fn receipt_from_batch(
    batch: &raw_v2::VerifiedV2ImportBatch,
) -> Result<RawV2ImportReceipt, ArchiveError> {
    let row_count = u64::try_from(batch.observations.len())
        .map_err(|_| ArchiveError::InvalidInput("raw V2 import row count exceeds u64"))?;
    Ok(RawV2ImportReceipt {
        manifest_id: batch.manifest_id.clone(),
        manifest_sha256: batch.manifest_sha256,
        first_local_sequence: batch.first_local_sequence,
        last_local_sequence: batch.last_local_sequence,
        partition: batch.partition.clone(),
        row_count,
    })
}

fn reject_foreign_plan(
    plan: &RawV2ImportPlan,
    chain: &ChainId,
    source: &SourceId,
) -> Result<(), ArchiveError> {
    if plan.chain_id.as_str() != chain.as_str() || plan.source_id.as_str() != source.as_str() {
        return Err(ArchiveError::InvalidInput(
            "raw V2 import plan does not match the requested source",
        ));
    }
    Ok(())
}

fn bind_plan_to_catalog(
    plan: &RawV2ImportPlan,
    generation: u64,
    catalog_hash: [u8; 32],
    batches: &[raw_v2::VerifiedV2ImportBatch],
) -> Result<(), ArchiveError> {
    if generation != plan.v2_catalog_generation || catalog_hash != plan.v2_catalog_sha256 {
        return Err(ArchiveError::ManifestVerification(
            "raw V2 catalog changed after the import plan was issued",
        ));
    }
    if batches.len() != plan.batches.len() {
        return Err(ArchiveError::ManifestVerification(
            "raw V2 catalog batch count does not match the import plan",
        ));
    }
    for (batch, receipt) in batches.iter().zip(plan.batches.iter()) {
        if batch.manifest_sha256 != receipt.manifest_sha256
            || batch.first_local_sequence != receipt.first_local_sequence
            || batch.last_local_sequence != receipt.last_local_sequence
            || batch.partition != receipt.partition
        {
            return Err(ArchiveError::ManifestVerification(
                "raw V2 catalog receipts do not match the import plan",
            ));
        }
    }
    Ok(())
}

fn report_for_import(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
    plan: &RawV2ImportPlan,
    root: &RootBundleV3,
    journal_bytes: &[u8],
    batches: &[raw_v2::VerifiedV2ImportBatch],
) -> Result<RawV2ImportReport, ArchiveError> {
    let pack_count = count_packed_leaves(archive, chain, source, root, journal_bytes)?;
    let batch_count = u64::try_from(plan.batches.len())
        .map_err(|_| ArchiveError::InvalidInput("raw V2 import batch count exceeds u64"))?;
    if root.chain_id()?.as_str() != chain.as_str()
        || root.source_id()?.as_str() != source.as_str()
        || root.head_local_sequence() != plan.last_local_sequence
        || root.logical_manifest_count() != batch_count
    {
        return Err(ArchiveError::ManifestVerification(
            "IMPORT root coverage does not match the verified V2 import plan",
        ));
    }
    let parity_digest = verify_import_parity(archive, chain, source, root, journal_bytes, batches)?;
    Ok(RawV2ImportReport {
        plan_sha256: plan.sha256,
        v3_root_sha256: root_bundle_hash(root)?,
        pack_count,
        packed_logical_manifest_count: batch_count,
        parity_digest,
    })
}

fn verify_import_parity(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
    root: &RootBundleV3,
    journal_bytes: &[u8],
    batches: &[raw_v2::VerifiedV2ImportBatch],
) -> Result<[u8; 32], ArchiveError> {
    let last = batches
        .last()
        .ok_or(ArchiveError::InvalidInput(
            "raw V2 import requires at least one verified batch",
        ))?
        .last_local_sequence;
    let range = LocalRecordSequenceRange::try_new(
        LocalRecordSequence::try_new(1)?,
        LocalRecordSequence::try_new(last)?,
    )?;
    let replayed = replay_root_by_sequence(archive, chain, source, root, journal_bytes, range)?;
    let expected = flatten_v2_observations(batches)?;
    if replayed.len() != expected.len() {
        return Err(ArchiveError::ManifestVerification(
            "imported V3 replay length does not match verified V2 observations",
        ));
    }
    let mut hasher = Sha256::new();
    hasher.update(PARITY_HASH_DOMAIN);
    for (item, expected) in replayed.iter().zip(expected.iter()) {
        if item.local_sequence() != expected.0
            || !observations_equivalent(item.observation(), &expected.1)
        {
            return Err(ArchiveError::ManifestVerification(
                "imported V3 replay is not byte-identical to verified V2 observations",
            ));
        }
        hasher.update(item.local_sequence().get().to_be_bytes());
        hasher.update(item.observation().content_hash().as_bytes());
    }
    Ok(hasher.finalize().into())
}

fn flatten_v2_observations(
    batches: &[raw_v2::VerifiedV2ImportBatch],
) -> Result<Vec<(LocalRecordSequence, SourceObservation)>, ArchiveError> {
    let mut expected = Vec::new();
    for batch in batches {
        for (index, observation) in batch.observations.iter().enumerate() {
            let advance_by = u64::try_from(index)
                .map_err(|_| ArchiveError::InvalidInput("raw V2 local sequence overflows"))?;
            let sequence = LocalRecordSequence::try_new(batch.first_local_sequence)?
                .checked_advance_by(advance_by)?;
            expected.push((sequence, observation.clone()));
        }
    }
    Ok(expected)
}

fn observations_equivalent(left: &SourceObservation, right: &SourceObservation) -> bool {
    left.source_id() == right.source_id()
        && left.source_version() == right.source_version()
        && left.observation_class() == right.observation_class()
        && left.cursor() == right.cursor()
        && left.received() == right.received()
        && left.parser_schema_version() == right.parser_schema_version()
        && left.payload() == right.payload()
        && left.warnings() == right.warnings()
        && left.content_hash() == right.content_hash()
}

fn write_import_packs(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
    batches: &[raw_v2::VerifiedV2ImportBatch],
    durable_at: domain_types::KnownTime,
) -> Result<Vec<SequenceLeafEntryV3>, ArchiveError> {
    let mut entries = Vec::new();
    for group in pack_groups(batches)? {
        entries.push(write_import_pack(
            archive,
            chain,
            source,
            &batches[group.clone()],
            durable_at,
        )?);
    }
    Ok(entries)
}

fn pack_groups(
    batches: &[raw_v2::VerifiedV2ImportBatch],
) -> Result<Vec<std::ops::Range<usize>>, ArchiveError> {
    let mut groups = Vec::new();
    let mut start = 0_usize;
    let mut bytes = 0_u64;
    for (index, batch) in batches.iter().enumerate() {
        if batch.object_size_bytes > RAW_ARCHIVE_MAXIMUM_DATA_PACK_BYTES {
            return Err(ArchiveError::InvalidInput(
                "raw V2 import batch exceeds the global data-pack bound",
            ));
        }
        let next_len = index - start + 1;
        let same_partition = index == start || batch.partition == batches[start].partition;
        let fits_count = next_len <= RAW_ARCHIVE_MAXIMUM_PACK_LOGICAL_INPUTS;
        let fits_bytes = bytes
            .checked_add(batch.object_size_bytes)
            .is_some_and(|total| total <= RAW_ARCHIVE_MAXIMUM_DATA_PACK_BYTES);
        if index > start && (!same_partition || !fits_count || !fits_bytes) {
            groups.push(start..index);
            start = index;
            bytes = 0;
        }
        bytes = bytes
            .checked_add(batch.object_size_bytes)
            .ok_or(ArchiveError::InvalidInput(
                "imported pack byte count overflows",
            ))?;
    }
    if start < batches.len() {
        groups.push(start..batches.len());
    }
    if groups.is_empty() {
        return Err(ArchiveError::InvalidInput(
            "raw V2 import requires at least one verified batch",
        ));
    }
    Ok(groups)
}

fn write_import_pack(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
    batches: &[raw_v2::VerifiedV2ImportBatch],
    durable_at: domain_types::KnownTime,
) -> Result<SequenceLeafEntryV3, ArchiveError> {
    let mut observations = Vec::new();
    let mut inputs = Vec::new();
    let mut row_start = 0_u64;
    for batch in batches {
        let row_count = u64::try_from(batch.observations.len())
            .map_err(|_| ArchiveError::InvalidInput("packed row count exceeds u64"))?;
        inputs.push(PackedLogicalInputV3::try_new_v2(
            batch.canonical_bytes.clone(),
            batch.manifest_sha256,
            row_start,
        )?);
        row_start = row_start
            .checked_add(row_count)
            .ok_or(ArchiveError::InvalidInput("packed row slice overflows"))?;
        observations.extend(batch.observations.iter().cloned());
    }
    let partition = batches[0].partition.as_str();
    let object = write_packed_object(archive, chain, source, partition, &observations)?;
    let pack = RawPackManifestV3::try_new(inputs, object, durable_at)?;
    let pack_bytes = manifest::canonical_json(&pack)?;
    let pack_hash = manifest::sha256(&pack_bytes);
    let pack_relative = pack_manifest_relative(pack_hash);
    fs::publish_immutable(archive.root(), &pack_relative, &pack_bytes)?;
    SequenceLeafEntryV3::try_new_packed(
        pack.first_local_sequence(),
        pack.last_local_sequence(),
        raw::path_string(&pack_relative)?,
        pack_hash,
        pack.object().size_bytes(),
        pack.object().row_count(),
        pack.logical_manifest_count(),
        pack.partition(),
    )
}

fn publish_import_root(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
    packed_entries: Vec<SequenceLeafEntryV3>,
    durable_at: domain_types::KnownTime,
) -> Result<(RootBundleV3, Vec<u8>), ArchiveError> {
    let dataset = dataset_relative(chain, source);
    let generation = 1_u64;
    let mut journal = JournalGenerationBuilderV3::try_new(
        generation,
        journal_file_identity(generation)?,
        journal_relative(&dataset, generation),
    )?;
    let packs = IndexPackBytes::new();
    let mut sequence_root: Option<SequenceNodeRefV3> = None;
    for entry in packed_entries {
        sequence_root = Some(append_logical_entry(
            &mut journal,
            &packs,
            sequence_root.as_ref(),
            chain.clone(),
            source.clone(),
            entry,
        )?);
        let root = sequence_root.as_ref().ok_or(ArchiveError::InvalidInput(
            "imported sequence root is missing",
        ))?;
        let committed_bytes = u64::try_from(journal.committed_bytes())
            .map_err(|_| ArchiveError::InvalidInput("journal prefix exceeds u64"))?;
        if journal_needs_rotation(
            journal.committed_record_count(),
            committed_bytes,
            root.depth(),
        ) {
            return Err(ArchiveError::InvalidInput(
                "raw V2 import cannot rotate the journal before CURRENT exists",
            ));
        }
    }
    let sequence_root = sequence_root.ok_or(ArchiveError::InvalidInput(
        "raw V2 import requires at least one packed leaf",
    ))?;
    let journal_commit = journal.commit_prefix(&sequence_root)?;
    fs::extend_append_only(
        archive.root(),
        Path::new(journal_commit.prefix().relative_path()),
        &[],
        journal_commit.bytes(),
        MAX_JOURNAL_BYTES,
    )?;
    let bundle = RootBundleV3::try_new(
        chain.clone(),
        source.clone(),
        generation,
        None,
        &journal_commit,
        durable_at,
    )?;
    let bundle_bytes = raw_v3::canonical_root_bytes(&bundle)?;
    let bundle_hash = root_bundle_hash(&bundle)?;
    fs::publish_immutable(
        archive.root(),
        &root_relative(&dataset, bundle_hash),
        &bundle_bytes,
    )?;
    let pointer = root_pointer_bytes(&dataset, bundle_hash)?;
    fs::publish_current_cas(archive.root(), &dataset.join("IMPORT"), None, &pointer)?;
    let (readback, journal_bytes) = load_import_root(archive, chain, source)?.ok_or(
        ArchiveError::ManifestVerification("raw V3 IMPORT pointer readback is missing"),
    )?;
    if root_bundle_hash(&readback)? != bundle_hash {
        return Err(ArchiveError::ManifestVerification(
            "raw V3 IMPORT pointer readback does not bind the published root",
        ));
    }
    Ok((readback, journal_bytes))
}

fn publish_v3_current(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
    expected_root: [u8; 32],
) -> Result<(), ArchiveError> {
    let dataset = dataset_relative(chain, source);
    let pointer = root_pointer_bytes(&dataset, expected_root)?;
    match load_current_root(archive, chain, source)? {
        None => fs::publish_current_cas(archive.root(), &dataset.join("CURRENT"), None, &pointer)?,
        Some((root, _)) if root_bundle_hash(&root)? == expected_root => {}
        Some(_) => {
            return Err(ArchiveError::ManifestVerification(
                "raw V3 CURRENT does not match the verified IMPORT root",
            ));
        }
    }
    let (readback, _) = load_current_root(archive, chain, source)?.ok_or(
        ArchiveError::ManifestVerification("raw V3 CURRENT readback is missing after import"),
    )?;
    if root_bundle_hash(&readback)? != expected_root {
        return Err(ArchiveError::ManifestVerification(
            "raw V3 CURRENT readback does not bind the verified IMPORT root",
        ));
    }
    Ok(())
}

fn publish_cutover(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
    plan: &RawV2ImportPlan,
    report: &RawV2ImportReport,
) -> Result<(), ArchiveError> {
    let document = CutoverDocument {
        schema: CUTOVER_SCHEMA.to_owned(),
        chain_id: chain.as_str().to_owned(),
        source_id: source.as_str().to_owned(),
        from_dataset: raw_policy::BYTE_V2_DATASET.to_owned(),
        to_dataset: RAW_BYTE_DATASET_V3.to_owned(),
        v2_catalog_sha256: hex::encode(plan.v2_catalog_sha256),
        v3_root_sha256: hex::encode(report.v3_root_sha256),
        parity_digest: hex::encode(report.parity_digest),
    };
    let bytes = manifest::canonical_json(&document)?;
    let relative = raw_policy::cutover_relative(chain, source);
    match fs::try_read_regular(archive.root(), &relative, 64 * 1024)? {
        None => fs::publish_current_cas(archive.root(), &relative, None, &bytes)?,
        Some(existing) if existing == bytes => {}
        Some(_) => {
            return Err(ArchiveError::ManifestVerification(
                "raw archive cutover pointer does not match the verified import",
            ));
        }
    }
    let readback = fs::read_regular(archive.root(), &relative, 64 * 1024)?;
    if readback != bytes {
        return Err(ArchiveError::ManifestVerification(
            "raw archive cutover readback does not match the published document",
        ));
    }
    Ok(())
}

fn switch_checkpoint(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
    target: [u8; 32],
) -> Result<(), ArchiveError> {
    match super::checkpoint::load_checkpoint(archive, chain, source)? {
        Some(checkpoint) if checkpoint.sha256() == target => Ok(()),
        Some(_) => Err(ArchiveError::ManifestVerification(
            "checkpoint CURRENT does not match the verified import checkpoint",
        )),
        None => super::checkpoint::switch_checkpoint_current(archive, chain, source, None, target),
    }
}

fn checkpoint_entries(
    plan: &RawV2ImportPlan,
) -> Result<RawArchiveCheckpointEntriesV2, ArchiveError> {
    let mut entries = Vec::with_capacity(plan.batches.len());
    for receipt in &plan.batches {
        entries.push(RawArchiveCheckpointEntryV2::new(
            receipt.manifest_id.clone(),
            receipt.manifest_sha256,
            LocalRecordSequenceRange::try_new(
                LocalRecordSequence::try_new(receipt.first_local_sequence)?,
                LocalRecordSequence::try_new(receipt.last_local_sequence)?,
            )?,
        ));
    }
    RawArchiveCheckpointEntriesV2::try_new(entries)
}

fn count_packed_leaves(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
    root: &RootBundleV3,
    journal_bytes: &[u8],
) -> Result<u64, ArchiveError> {
    let packs = load_packs_for_tree(archive, chain, source, root.sequence_root(), journal_bytes)?;
    let mut count = 0_u64;
    walk_logical_leaves(root.sequence_root(), journal_bytes, &packs, &mut |entry| {
        if matches!(entry.storage(), SequenceStorageRefV3::Packed { .. }) {
            count = count
                .checked_add(1)
                .ok_or(ArchiveError::InvalidInput("imported pack count overflows"))?;
        }
        Ok(false)
    })?;
    Ok(count)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct VerifiedCutover {
    pub(super) v2_catalog_sha256: [u8; 32],
}

pub(super) fn load_verified_cutover(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
    expected_root: [u8; 32],
) -> Result<VerifiedCutover, ArchiveError> {
    let relative = raw_policy::cutover_relative(chain, source);
    let bytes = fs::try_read_regular(archive.root(), &relative, 64 * 1024)?.ok_or(
        ArchiveError::ManifestVerification("raw V2 import reclaim requires a verified cutover"),
    )?;
    let document: CutoverDocument = serde_json::from_slice(&bytes)
        .map_err(|_| ArchiveError::ManifestVerification("invalid raw archive cutover JSON"))?;
    if document.schema != CUTOVER_SCHEMA
        || document.chain_id != chain.as_str()
        || document.source_id != source.as_str()
        || document.from_dataset != raw_policy::BYTE_V2_DATASET
        || document.to_dataset != RAW_BYTE_DATASET_V3
    {
        return Err(ArchiveError::ManifestVerification(
            "raw archive cutover does not bind the verified import",
        ));
    }
    let v3_root = manifest::parse_hash(&document.v3_root_sha256)?;
    let v2_catalog = manifest::parse_hash(&document.v2_catalog_sha256)?;
    manifest::parse_hash(&document.parity_digest)?;
    if v3_root != expected_root {
        return Err(ArchiveError::ManifestVerification(
            "raw archive cutover does not match V3 CURRENT",
        ));
    }
    if manifest::canonical_json(&document)? != bytes {
        return Err(ArchiveError::ManifestVerification(
            "raw archive cutover is not canonical",
        ));
    }
    Ok(VerifiedCutover {
        v2_catalog_sha256: v2_catalog,
    })
}

fn persist_document(
    archive: &RawV3Archive,
    relative: &Path,
    bytes: &[u8],
) -> Result<(), ArchiveError> {
    fs::publish_immutable(archive.root(), relative, bytes)
}

fn plan_document(plan: &RawV2ImportPlan) -> Result<Vec<u8>, ArchiveError> {
    manifest::canonical_json(&ImportPlanDocument {
        schema: IMPORT_PLAN_SCHEMA.to_owned(),
        chain_id: plan.chain_id.as_str().to_owned(),
        source_id: plan.source_id.as_str().to_owned(),
        v2_catalog_generation: plan.v2_catalog_generation,
        v2_catalog_sha256: hex::encode(plan.v2_catalog_sha256),
        first_local_sequence: plan.first_local_sequence,
        last_local_sequence: plan.last_local_sequence,
        batches: plan
            .batches
            .iter()
            .map(|receipt| ImportBatchDocument {
                manifest_id: receipt.manifest_id.as_str().to_owned(),
                manifest_sha256: hex::encode(receipt.manifest_sha256),
                first_local_sequence: receipt.first_local_sequence,
                last_local_sequence: receipt.last_local_sequence,
                partition: receipt.partition.clone(),
                row_count: receipt.row_count,
            })
            .collect(),
    })
}

fn report_document(report: &RawV2ImportReport) -> Result<Vec<u8>, ArchiveError> {
    manifest::canonical_json(&ImportReportDocument {
        schema: IMPORT_REPORT_SCHEMA.to_owned(),
        plan_sha256: hex::encode(report.plan_sha256),
        v3_root_sha256: hex::encode(report.v3_root_sha256),
        pack_count: report.pack_count,
        packed_logical_manifest_count: report.packed_logical_manifest_count,
        parity_digest: hex::encode(report.parity_digest),
    })
}

fn plan_relative(hash: [u8; 32]) -> PathBuf {
    PathBuf::from("_manifests")
        .join("raw-byte-v3")
        .join("imports")
        .join(format!("plan-{}.json", hex::encode(hash)))
}

fn report_relative(hash: [u8; 32]) -> PathBuf {
    PathBuf::from("_manifests")
        .join("raw-byte-v3")
        .join("imports")
        .join(format!("report-{}.json", hex::encode(hash)))
}
