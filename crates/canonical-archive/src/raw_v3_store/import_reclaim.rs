use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use domain_types::{ChainId, SourceId};
use serde::{Deserialize, Serialize};
use storage_ports::ArchiveError;

use super::{
    RawV3Archive, load_current_root, load_import_root, load_pack_manifest, load_packs_for_tree,
    walk_logical_leaves,
};
use crate::{
    LocalParquetArchive, fs, manifest,
    raw_v3::{self, SequenceLeafEntryV3, SequenceStorageRefV3, root_bundle_hash},
};

const RAW_BACKUP_RECEIPT_SCHEMA_V3: &str = "hyperliquid-alpha-desk/archive-raw-backup-receipt/v3";
const BACKUP_RECEIPT_HASH_DOMAIN_V3: &[u8] =
    b"hyperliquid-alpha-desk:archive-raw-backup-receipt:v3\0";
const RECLAIM_PLAN_SCHEMA: &str = "hyperliquid-alpha-desk/archive-raw-v2-import-reclaim-plan/v1";
const RECLAIM_PLAN_HASH_DOMAIN: &[u8] =
    b"hyperliquid-alpha-desk:archive-raw-v2-import-reclaim-plan:v1\0";
const MAX_BACKUP_RECEIPT_BYTES: u64 = 16 * 1024 * 1024;
const MAX_RECLAIM_PLAN_BYTES: u64 = 16 * 1024 * 1024;
const MAX_DELETION_JOURNAL_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReclaimFile {
    relative_path: String,
    object_sha256: String,
    byte_len: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RawArchiveBackupReceipt {
    schema: String,
    root_sha256: String,
    chain_id: String,
    source_id: String,
    created_at_micros: i64,
    files: Vec<ReclaimFile>,
    #[serde(skip)]
    digest: [u8; 32],
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BackupReceiptWire {
    schema: String,
    root_sha256: String,
    chain_id: String,
    source_id: String,
    created_at_micros: i64,
    files: Vec<ReclaimFile>,
}

impl RawArchiveBackupReceipt {
    #[must_use]
    pub const fn digest(&self) -> [u8; 32] {
        self.digest
    }

    #[must_use]
    pub fn root_sha256(&self) -> &str {
        &self.root_sha256
    }

    #[must_use]
    pub(super) fn chain_id(&self) -> &str {
        &self.chain_id
    }

    #[must_use]
    pub(super) fn source_id(&self) -> &str {
        &self.source_id
    }

    pub fn files(&self) -> impl Iterator<Item = (&str, &str, u64)> {
        self.files.iter().map(|file| {
            (
                file.relative_path.as_str(),
                file.object_sha256.as_str(),
                file.byte_len,
            )
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RawV2ImportReclaimPlan {
    schema: String,
    root_sha256: String,
    backup_receipt_sha256: String,
    created_at_micros: i64,
    files: Vec<ReclaimFile>,
    #[serde(skip)]
    digest: [u8; 32],
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReclaimPlanWire {
    schema: String,
    root_sha256: String,
    backup_receipt_sha256: String,
    created_at_micros: i64,
    files: Vec<ReclaimFile>,
}

impl RawV2ImportReclaimPlan {
    #[must_use]
    pub const fn digest(&self) -> [u8; 32] {
        self.digest
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawV2ImportReclaimReceipt {
    plan_digest: [u8; 32],
    unlinked_files: u64,
    unlinked_bytes: u64,
}

impl RawV2ImportReclaimReceipt {
    #[must_use]
    pub const fn plan_digest(&self) -> [u8; 32] {
        self.plan_digest
    }

    #[must_use]
    pub const fn unlinked_files(&self) -> u64 {
        self.unlinked_files
    }

    #[must_use]
    pub const fn unlinked_bytes(&self) -> u64 {
        self.unlinked_bytes
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
enum DeletionJournalRecord {
    Planned {
        relative_path: String,
        object_sha256: String,
        byte_len: u64,
    },
    Unlinked {
        relative_path: String,
        object_sha256: String,
    },
    DirectorySynced {
        relative_path: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeletionState {
    Planned {
        object_sha256: [u8; 32],
        byte_len: u64,
    },
    Unlinked {
        object_sha256: [u8; 32],
    },
    DirectorySynced,
}

pub fn backup_v2_import_originals(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
    backup_root: &Path,
) -> Result<RawArchiveBackupReceipt, ArchiveError> {
    let backup_root = canonicalize_backup_root(backup_root)?;
    let approved = require_approved_import(archive, chain, source)?;
    let files = covered_originals_on_disk(archive, chain, source, &approved)?;
    for file in &files {
        let path = PathBuf::from(&file.relative_path);
        fs::validate_relative(&path)?;
        let bytes = fs::read_regular(&archive.root, &path, file.byte_len.max(1))?;
        if u64::try_from(bytes.len()).ok() != Some(file.byte_len)
            || manifest::sha256(&bytes) != manifest::parse_hash(&file.object_sha256)?
        {
            return Err(ArchiveError::ManifestVerification(
                "eligible object changed while writing the backup receipt",
            ));
        }
        fs::publish_immutable(&backup_root, &path, &bytes)?;
    }
    write_backup_receipt_files(
        archive,
        chain,
        source,
        approved.v3_root_sha256,
        files
            .into_iter()
            .map(|file| (file.relative_path, file.object_sha256, file.byte_len))
            .collect(),
        &backup_root,
        Path::new("_manifests/raw-byte-v3/imports"),
    )
}

pub(super) fn write_backup_receipt_files(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
    root_sha256: [u8; 32],
    files: Vec<(String, String, u64)>,
    backup_root: &Path,
    relative_dir: &Path,
) -> Result<RawArchiveBackupReceipt, ArchiveError> {
    let files = files
        .into_iter()
        .map(|(relative_path, object_sha256, byte_len)| ReclaimFile {
            relative_path,
            object_sha256,
            byte_len,
        })
        .collect();
    let receipt = RawArchiveBackupReceipt {
        schema: RAW_BACKUP_RECEIPT_SCHEMA_V3.to_owned(),
        root_sha256: hex::encode(root_sha256),
        chain_id: chain.as_str().to_owned(),
        source_id: source.as_str().to_owned(),
        created_at_micros: archive.now()?.unix_micros(),
        files,
        digest: [0; 32],
    };
    let digest = backup_receipt_digest(&receipt)?;
    let receipt = RawArchiveBackupReceipt { digest, ..receipt };
    let bytes = manifest::canonical_json(&receipt)?;
    let relative = relative_dir.join(format!("backup-receipt-{}.json", hex::encode(digest)));
    fs::publish_immutable(&archive.root, &relative, &bytes)?;
    fs::publish_immutable(backup_root, &relative, &bytes)?;
    Ok(receipt)
}

pub fn plan_v2_import_reclaim(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
    backup_receipt: [u8; 32],
) -> Result<RawV2ImportReclaimPlan, ArchiveError> {
    reject_zero_receipt(backup_receipt)?;
    let approved = require_approved_import(archive, chain, source)?;
    let receipt = load_verified_backup_receipt(archive, chain, source, backup_receipt)?;
    if receipt.root_sha256 != hex::encode(approved.v3_root_sha256)
        || receipt.chain_id != chain.as_str()
        || receipt.source_id != source.as_str()
    {
        return Err(ArchiveError::ManifestVerification(
            "backup receipt does not bind this archive root",
        ));
    }
    let backed_up: BTreeMap<_, _> = receipt
        .files
        .iter()
        .map(|file| {
            (
                file.relative_path.clone(),
                (file.object_sha256.clone(), file.byte_len),
            )
        })
        .collect();
    let mut files = covered_originals_on_disk(archive, chain, source, &approved)?;
    files.retain(|file| {
        backed_up
            .get(&file.relative_path)
            .is_some_and(|(hash, len)| *hash == file.object_sha256 && *len == file.byte_len)
    });
    if files.len() != receipt.files.len() {
        return Err(ArchiveError::ManifestVerification(
            "GC plan file is not covered by the verified backup receipt",
        ));
    }
    let plan = RawV2ImportReclaimPlan {
        schema: RECLAIM_PLAN_SCHEMA.to_owned(),
        root_sha256: hex::encode(approved.v3_root_sha256),
        backup_receipt_sha256: hex::encode(backup_receipt),
        created_at_micros: archive.now()?.unix_micros(),
        files,
        digest: [0; 32],
    };
    let digest = reclaim_plan_digest(&plan)?;
    let plan = RawV2ImportReclaimPlan { digest, ..plan };
    let bytes = manifest::canonical_json(&plan)?;
    fs::publish_immutable(&archive.root, &reclaim_plan_relative(digest), &bytes)?;
    Ok(plan)
}

pub fn execute_v2_import_reclaim(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
    plan_digest: [u8; 32],
    backup_receipt: [u8; 32],
) -> Result<RawV2ImportReclaimReceipt, ArchiveError> {
    reject_zero_receipt(backup_receipt)?;
    let approved = require_approved_import(archive, chain, source)?;
    let plan = load_plan(archive, plan_digest)?;
    if manifest::parse_hash(&plan.backup_receipt_sha256)? != backup_receipt {
        return Err(ArchiveError::ManifestVerification(
            "GC backup receipt does not match the authorized plan",
        ));
    }
    if hex::encode(approved.v3_root_sha256) != plan.root_sha256 {
        return Err(ArchiveError::ManifestVerification(
            "CURRENT moved after the GC plan was authorized",
        ));
    }
    let receipt = load_verified_backup_receipt(archive, chain, source, backup_receipt)?;
    assert_plan_files_covered_by_backup(&plan, &receipt)?;
    let journal_relative = deletion_journal_relative(plan_digest);
    let mut states = load_deletion_states(archive, &journal_relative)?;
    let mut unlinked_files = 0_u64;
    let mut unlinked_bytes = 0_u64;
    for file in &plan.files {
        let path = PathBuf::from(&file.relative_path);
        fs::validate_relative(&path)?;
        let hash = manifest::parse_hash(&file.object_sha256)?;
        let mut state = states.get(&file.relative_path).copied();
        if state == Some(DeletionState::DirectorySynced) {
            unlinked_files = unlinked_files.saturating_add(1);
            unlinked_bytes = unlinked_bytes.saturating_add(file.byte_len);
            continue;
        }
        let exists = fs::exists_regular(&archive.root, &path)?;
        if state.is_none() {
            if !exists {
                return Err(ArchiveError::ManifestVerification(
                    "eligible object is missing before unlink; restore from backup",
                ));
            }
            append_record(
                archive,
                &journal_relative,
                &DeletionJournalRecord::Planned {
                    relative_path: file.relative_path.clone(),
                    object_sha256: file.object_sha256.clone(),
                    byte_len: file.byte_len,
                },
            )?;
            let planned = DeletionState::Planned {
                object_sha256: hash,
                byte_len: file.byte_len,
            };
            state = Some(planned);
            states.insert(file.relative_path.clone(), planned);
        }
        journal_matches_plan(file, hash, state)?;
        if !matches!(state, Some(DeletionState::Unlinked { .. })) {
            if exists {
                recheck_before_unlink(
                    archive,
                    chain,
                    source,
                    &approved,
                    backup_receipt,
                    &plan,
                    file,
                )?;
                fs::unlink_regular_matching(&archive.root, &path, hash, file.byte_len)?;
            } else if !matches!(state, Some(DeletionState::Planned { .. })) {
                return Err(ArchiveError::ManifestVerification(
                    "eligible object vanished without a matching deletion journal",
                ));
            }
            append_record(
                archive,
                &journal_relative,
                &DeletionJournalRecord::Unlinked {
                    relative_path: file.relative_path.clone(),
                    object_sha256: file.object_sha256.clone(),
                },
            )?;
            states.insert(
                file.relative_path.clone(),
                DeletionState::Unlinked {
                    object_sha256: hash,
                },
            );
        }
        append_record(
            archive,
            &journal_relative,
            &DeletionJournalRecord::DirectorySynced {
                relative_path: file.relative_path.clone(),
            },
        )?;
        unlinked_files = unlinked_files
            .checked_add(1)
            .ok_or(ArchiveError::InvalidInput("unlinked file count overflows"))?;
        unlinked_bytes = unlinked_bytes
            .checked_add(file.byte_len)
            .ok_or(ArchiveError::InvalidInput("unlinked byte count overflows"))?;
    }
    Ok(RawV2ImportReclaimReceipt {
        plan_digest,
        unlinked_files,
        unlinked_bytes,
    })
}

struct ApprovedImport {
    v3_root_sha256: [u8; 32],
    v2_catalog_sha256: [u8; 32],
}

fn require_approved_import(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
) -> Result<ApprovedImport, ArchiveError> {
    let (root, _) = load_current_root(archive, chain, source)?.ok_or(
        ArchiveError::ManifestVerification("raw V2 import reclaim requires published V3 CURRENT"),
    )?;
    let current_hash = root_bundle_hash(&root)?;
    let (import_root, _) = load_import_root(archive, chain, source)?.ok_or(
        ArchiveError::ManifestVerification("raw V2 import candidate is missing"),
    )?;
    if root_bundle_hash(&import_root)? != current_hash {
        return Err(ArchiveError::ManifestVerification(
            "raw V3 CURRENT does not match the verified IMPORT root",
        ));
    }
    let cutover = super::import::load_verified_cutover(archive, chain, source, current_hash)?;
    let v2 = LocalParquetArchive::open(archive.root(), archive.config().clone())?;
    let catalog_hash = crate::raw_v2::load_catalog_sha256(&v2, chain, source)?;
    if catalog_hash != cutover.v2_catalog_sha256 {
        return Err(ArchiveError::ManifestVerification(
            "raw V2 catalog does not match the verified cutover",
        ));
    }
    Ok(ApprovedImport {
        v3_root_sha256: current_hash,
        v2_catalog_sha256: catalog_hash,
    })
}

fn covered_originals_on_disk(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
    approved: &ApprovedImport,
) -> Result<Vec<ReclaimFile>, ArchiveError> {
    let expected = packed_v2_originals(archive, chain, source, approved)?;
    let mut files = Vec::new();
    let mut missing = 0_usize;
    for (relative_path, (object_sha256, byte_len)) in expected {
        let path = PathBuf::from(&relative_path);
        fs::validate_relative(&path)?;
        if !fs::exists_regular(&archive.root, &path)? {
            missing = missing.checked_add(1).ok_or(ArchiveError::InvalidInput(
                "missing original count overflows",
            ))?;
            continue;
        }
        let (digest, length) = fs::regular_digest(&archive.root, &path, byte_len.max(1))?;
        if digest != object_sha256 || length != byte_len {
            return Err(ArchiveError::ManifestVerification(
                "imported V2 original does not match the verified pack coverage",
            ));
        }
        files.push(ReclaimFile {
            relative_path,
            object_sha256: hex::encode(object_sha256),
            byte_len,
        });
    }
    if missing > 0 && !files.is_empty() {
        return Err(ArchiveError::ManifestVerification(
            "imported V2 originals are only partially present",
        ));
    }
    Ok(files)
}

fn packed_v2_originals(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
    approved: &ApprovedImport,
) -> Result<BTreeMap<String, ([u8; 32], u64)>, ArchiveError> {
    let (root, journal_bytes) = load_current_root(archive, chain, source)?.ok_or(
        ArchiveError::ManifestVerification("raw V2 import reclaim requires published V3 CURRENT"),
    )?;
    if root_bundle_hash(&root)? != approved.v3_root_sha256 {
        return Err(ArchiveError::ManifestVerification(
            "CURRENT moved after import approval",
        ));
    }
    let packs = load_packs_for_tree(archive, chain, source, root.sequence_root(), &journal_bytes)?;
    let mut files = BTreeMap::new();
    walk_logical_leaves(root.sequence_root(), &journal_bytes, &packs, &mut |entry| {
        collect_packed_v2_original(archive, chain, source, entry, &mut files)?;
        Ok(false)
    })?;
    if files.is_empty() {
        return Err(ArchiveError::ManifestVerification(
            "verified V3 packs do not cover any V2 originals",
        ));
    }
    Ok(files)
}

fn collect_packed_v2_original(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
    entry: &SequenceLeafEntryV3,
    files: &mut BTreeMap<String, ([u8; 32], u64)>,
) -> Result<(), ArchiveError> {
    match entry.storage() {
        SequenceStorageRefV3::Packed { .. } => {}
        SequenceStorageRefV3::Logical { .. } => {
            return Err(ArchiveError::ManifestVerification(
                "import reclaim CURRENT contains unpacked logical leaves",
            ));
        }
    }
    let pack = load_pack_manifest(archive, chain, source, entry)?;
    for input in pack.inputs() {
        if input.original_schema() != "raw-v2" {
            return Err(ArchiveError::ManifestVerification(
                "import reclaim CURRENT contains non-imported originals",
            ));
        }
        let evidence = crate::raw_v2::validate_embedded_manifest_v2(
            input.canonical_manifest_json().as_bytes().to_vec(),
            input.manifest_sha256()?,
        )?;
        let previous = files.insert(
            evidence.object_relative_path.clone(),
            (evidence.object_sha256, evidence.object_size_bytes),
        );
        if previous
            .is_some_and(|prior| prior != (evidence.object_sha256, evidence.object_size_bytes))
        {
            return Err(ArchiveError::ManifestVerification(
                "packed V2 original coverage is inconsistent",
            ));
        }
    }
    Ok(())
}

fn load_verified_backup_receipt(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
    digest: [u8; 32],
) -> Result<RawArchiveBackupReceipt, ArchiveError> {
    load_verified_backup_receipt_at(
        archive,
        chain,
        source,
        digest,
        &backup_receipt_relative(digest),
    )
}

pub(super) fn load_verified_backup_receipt_at(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
    digest: [u8; 32],
    relative: &Path,
) -> Result<RawArchiveBackupReceipt, ArchiveError> {
    reject_zero_receipt(digest)?;
    let Some(bytes) = fs::try_read_regular(&archive.root, relative, MAX_BACKUP_RECEIPT_BYTES)?
    else {
        return Err(ArchiveError::ManifestVerification(
            "GC backup receipt artifact is missing or invalid",
        ));
    };
    let wire: BackupReceiptWire = serde_json::from_slice(&bytes).map_err(|_| {
        ArchiveError::ManifestVerification("GC backup receipt artifact is missing or invalid")
    })?;
    if wire.schema != RAW_BACKUP_RECEIPT_SCHEMA_V3 {
        return Err(ArchiveError::ManifestVerification(
            "GC backup receipt artifact is missing or invalid",
        ));
    }
    let receipt = RawArchiveBackupReceipt {
        schema: wire.schema,
        root_sha256: wire.root_sha256,
        chain_id: wire.chain_id,
        source_id: wire.source_id,
        created_at_micros: wire.created_at_micros,
        files: wire.files,
        digest: [0; 32],
    };
    let computed = backup_receipt_digest(&receipt)?;
    if computed != digest
        || manifest::canonical_json(&RawArchiveBackupReceipt {
            digest: computed,
            ..receipt.clone()
        })? != bytes
    {
        return Err(ArchiveError::ManifestVerification(
            "GC backup receipt artifact is missing or invalid",
        ));
    }
    if receipt.chain_id != chain.as_str() || receipt.source_id != source.as_str() {
        return Err(ArchiveError::ManifestVerification(
            "backup receipt does not bind this archive root",
        ));
    }
    Ok(RawArchiveBackupReceipt {
        digest: computed,
        ..receipt
    })
}

fn load_plan(
    archive: &RawV3Archive,
    digest: [u8; 32],
) -> Result<RawV2ImportReclaimPlan, ArchiveError> {
    let Some(bytes) = fs::try_read_regular(
        &archive.root,
        &reclaim_plan_relative(digest),
        MAX_RECLAIM_PLAN_BYTES,
    )?
    else {
        return Err(ArchiveError::ManifestVerification(
            "raw V2 import reclaim plan is missing or invalid",
        ));
    };
    let wire: ReclaimPlanWire = serde_json::from_slice(&bytes).map_err(|_| {
        ArchiveError::ManifestVerification("invalid raw V2 import reclaim plan JSON")
    })?;
    if wire.schema != RECLAIM_PLAN_SCHEMA {
        return Err(ArchiveError::ManifestVerification(
            "unsupported raw V2 import reclaim plan schema",
        ));
    }
    let plan = RawV2ImportReclaimPlan {
        schema: wire.schema,
        root_sha256: wire.root_sha256,
        backup_receipt_sha256: wire.backup_receipt_sha256,
        created_at_micros: wire.created_at_micros,
        files: wire.files,
        digest: [0; 32],
    };
    let computed = reclaim_plan_digest(&plan)?;
    if computed != digest
        || manifest::canonical_json(&RawV2ImportReclaimPlan {
            digest: computed,
            ..plan.clone()
        })? != bytes
    {
        return Err(ArchiveError::ManifestVerification(
            "GC plan digest or canonical bytes are invalid",
        ));
    }
    Ok(RawV2ImportReclaimPlan {
        digest: computed,
        ..plan
    })
}

fn backup_receipt_digest(receipt: &RawArchiveBackupReceipt) -> Result<[u8; 32], ArchiveError> {
    let root = manifest::parse_hash(&receipt.root_sha256)?;
    let mut evidence = Vec::new();
    evidence.extend_from_slice(&root);
    let chain = receipt.chain_id.as_bytes();
    let chain_len = u64::try_from(chain.len())
        .map_err(|_| ArchiveError::InvalidInput("backup receipt chain id exceeds u64"))?;
    evidence.extend_from_slice(&chain_len.to_be_bytes());
    evidence.extend_from_slice(chain);
    let source = receipt.source_id.as_bytes();
    let source_len = u64::try_from(source.len())
        .map_err(|_| ArchiveError::InvalidInput("backup receipt source id exceeds u64"))?;
    evidence.extend_from_slice(&source_len.to_be_bytes());
    evidence.extend_from_slice(source);
    evidence.extend_from_slice(&receipt.created_at_micros.to_be_bytes());
    append_file_evidence(&mut evidence, &receipt.files)?;
    raw_v3::domain_hash(BACKUP_RECEIPT_HASH_DOMAIN_V3, &evidence)
}

fn reclaim_plan_digest(plan: &RawV2ImportReclaimPlan) -> Result<[u8; 32], ArchiveError> {
    let root = manifest::parse_hash(&plan.root_sha256)?;
    let backup = manifest::parse_hash(&plan.backup_receipt_sha256)?;
    let mut evidence = Vec::new();
    evidence.extend_from_slice(&root);
    evidence.extend_from_slice(&backup);
    evidence.extend_from_slice(&plan.created_at_micros.to_be_bytes());
    append_file_evidence(&mut evidence, &plan.files)?;
    raw_v3::domain_hash(RECLAIM_PLAN_HASH_DOMAIN, &evidence)
}

fn append_file_evidence(evidence: &mut Vec<u8>, files: &[ReclaimFile]) -> Result<(), ArchiveError> {
    for file in files {
        let path = file.relative_path.as_bytes();
        let path_len = u64::try_from(path.len())
            .map_err(|_| ArchiveError::InvalidInput("backup receipt path exceeds u64"))?;
        evidence.extend_from_slice(&path_len.to_be_bytes());
        evidence.extend_from_slice(path);
        evidence.extend_from_slice(&manifest::parse_hash(&file.object_sha256)?);
        evidence.extend_from_slice(&file.byte_len.to_be_bytes());
    }
    Ok(())
}

fn assert_plan_files_covered_by_backup(
    plan: &RawV2ImportReclaimPlan,
    receipt: &RawArchiveBackupReceipt,
) -> Result<(), ArchiveError> {
    let backed_up: BTreeMap<_, _> = receipt
        .files
        .iter()
        .map(|file| {
            (
                file.relative_path.as_str(),
                (file.object_sha256.as_str(), file.byte_len),
            )
        })
        .collect();
    for file in &plan.files {
        if !backed_up
            .get(file.relative_path.as_str())
            .is_some_and(|(hash, len)| *hash == file.object_sha256 && *len == file.byte_len)
        {
            return Err(ArchiveError::ManifestVerification(
                "GC plan file is not covered by the verified backup receipt",
            ));
        }
    }
    Ok(())
}

fn recheck_before_unlink(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
    approved: &ApprovedImport,
    backup_receipt: [u8; 32],
    plan: &RawV2ImportReclaimPlan,
    file: &ReclaimFile,
) -> Result<(), ArchiveError> {
    let live = require_approved_import(archive, chain, source)?;
    if live.v3_root_sha256 != approved.v3_root_sha256
        || live.v2_catalog_sha256 != approved.v2_catalog_sha256
    {
        return Err(ArchiveError::ManifestVerification(
            "CURRENT moved immediately before unlink",
        ));
    }
    if manifest::parse_hash(&plan.backup_receipt_sha256)? != backup_receipt {
        return Err(ArchiveError::ManifestVerification(
            "GC backup receipt changed immediately before unlink",
        ));
    }
    let path = PathBuf::from(&file.relative_path);
    let (digest, length) = fs::regular_digest(&archive.root, &path, file.byte_len.max(1))?;
    if hex::encode(digest) != file.object_sha256 || length != file.byte_len {
        return Err(ArchiveError::ManifestVerification(
            "eligible object changed immediately before unlink",
        ));
    }
    Ok(())
}

fn load_deletion_states(
    archive: &RawV3Archive,
    relative: &Path,
) -> Result<BTreeMap<String, DeletionState>, ArchiveError> {
    let Some(bytes) = fs::try_read_regular(&archive.root, relative, MAX_DELETION_JOURNAL_BYTES)?
    else {
        return Ok(BTreeMap::new());
    };
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| ArchiveError::ManifestVerification("deletion journal is not UTF-8"))?;
    let mut states = BTreeMap::new();
    for line in text.lines() {
        if line.is_empty() {
            continue;
        }
        let record: DeletionJournalRecord = serde_json::from_str(line)
            .map_err(|_| ArchiveError::ManifestVerification("invalid deletion journal record"))?;
        match record {
            DeletionJournalRecord::Planned {
                relative_path,
                object_sha256,
                byte_len,
            } => {
                states.insert(
                    relative_path,
                    DeletionState::Planned {
                        object_sha256: manifest::parse_hash(&object_sha256)?,
                        byte_len,
                    },
                );
            }
            DeletionJournalRecord::Unlinked {
                relative_path,
                object_sha256,
            } => {
                states.insert(
                    relative_path,
                    DeletionState::Unlinked {
                        object_sha256: manifest::parse_hash(&object_sha256)?,
                    },
                );
            }
            DeletionJournalRecord::DirectorySynced { relative_path } => {
                states.insert(relative_path, DeletionState::DirectorySynced);
            }
        }
    }
    Ok(states)
}

fn append_record(
    archive: &RawV3Archive,
    relative: &Path,
    record: &DeletionJournalRecord,
) -> Result<(), ArchiveError> {
    let line = serde_json::to_vec(record)
        .map_err(|_| ArchiveError::Codec("serializing deletion journal record".into()))?;
    fs::append_journal_line(&archive.root, relative, &line, MAX_DELETION_JOURNAL_BYTES)
}

fn journal_matches_plan(
    file: &ReclaimFile,
    plan_hash: [u8; 32],
    state: Option<DeletionState>,
) -> Result<(), ArchiveError> {
    match state {
        Some(DeletionState::Planned {
            object_sha256,
            byte_len,
        }) if object_sha256 == plan_hash && byte_len == file.byte_len => Ok(()),
        Some(DeletionState::Unlinked { object_sha256 }) if object_sha256 == plan_hash => Ok(()),
        Some(DeletionState::DirectorySynced) => Ok(()),
        None => Ok(()),
        Some(_) => Err(ArchiveError::ManifestVerification(
            "deletion journal does not match the authorized plan",
        )),
    }
}

fn canonicalize_backup_root(root: &Path) -> Result<PathBuf, ArchiveError> {
    if root.as_os_str().is_empty() {
        return Err(ArchiveError::UnsafePath);
    }
    let metadata =
        std::fs::symlink_metadata(root).map_err(|_| ArchiveError::Io("inspecting backup root"))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(ArchiveError::UnsafePath);
    }
    root.canonicalize()
        .map_err(|_| ArchiveError::Io("canonicalizing backup root"))
}

fn reject_zero_receipt(backup_receipt: [u8; 32]) -> Result<(), ArchiveError> {
    if backup_receipt == [0; 32] {
        return Err(ArchiveError::InvalidInput(
            "GC backup receipt must be a nonzero digest",
        ));
    }
    Ok(())
}

fn backup_receipt_relative(digest: [u8; 32]) -> PathBuf {
    PathBuf::from("_manifests")
        .join("raw-byte-v3")
        .join("imports")
        .join(format!("backup-receipt-{}.json", hex::encode(digest)))
}

fn reclaim_plan_relative(digest: [u8; 32]) -> PathBuf {
    PathBuf::from("_manifests")
        .join("raw-byte-v3")
        .join("imports")
        .join(format!("reclaim-plan-{}.json", hex::encode(digest)))
}

fn deletion_journal_relative(digest: [u8; 32]) -> PathBuf {
    PathBuf::from("_manifests")
        .join("raw-byte-v3")
        .join("imports")
        .join(format!("reclaim-deletion-{}.log", hex::encode(digest)))
}
