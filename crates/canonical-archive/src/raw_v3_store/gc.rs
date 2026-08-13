use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    path::{Path, PathBuf},
};

use domain_types::{ChainId, KnownTime, SourceId};
use serde::{Deserialize, Serialize};
use storage_ports::{ArchiveError, RawArchiveRootLeaseIdentity};

use super::{
    RawV3Archive, dataset_relative, load_current_root, load_logical_commit, load_pack_manifest,
    load_packs_for_tree, load_verified_root, root_relative, walk_logical_leaves,
};
use crate::{
    fs, manifest,
    raw_v3::{
        self, RootBundleV3, SequenceLeafEntryV3, SequenceStorageRefV3,
        parse_logical_commit_manifest, root_bundle_hash,
    },
};

const RAW_GC_PLAN_SCHEMA_V3: &str = "hyperliquid-alpha-desk/archive-raw-gc-plan/v3";
const GC_PLAN_HASH_DOMAIN_V3: &[u8] = b"hyperliquid-alpha-desk:archive-raw-gc-plan:v3\0";
const MAX_DELETION_JOURNAL_BYTES: u64 = 16 * 1024 * 1024;
const MAX_GC_PLAN_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RawArchiveGcPlan {
    schema: String,
    root_sha256: String,
    backup_receipt_sha256: String,
    retention_horizon_seconds: u64,
    created_at_micros: i64,
    files: Vec<GcPlanFileV3>,
    #[serde(skip)]
    digest: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct GcPlanFileV3 {
    relative_path: String,
    object_sha256: String,
    byte_len: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GcPlanWireV3 {
    schema: String,
    root_sha256: String,
    backup_receipt_sha256: String,
    retention_horizon_seconds: u64,
    created_at_micros: i64,
    files: Vec<GcPlanFileV3>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
enum DeletionJournalRecordV3 {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawArchiveGcReceipt {
    plan_digest: [u8; 32],
    unlinked_files: u64,
    unlinked_bytes: u64,
}

impl RawArchiveGcPlan {
    #[must_use]
    pub const fn digest(&self) -> [u8; 32] {
        self.digest
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

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

impl RawArchiveGcReceipt {
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

pub fn plan_packed_object_gc(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
    backup_receipt: [u8; 32],
) -> Result<RawArchiveGcPlan, ArchiveError> {
    if backup_receipt == [0; 32] {
        return Err(ArchiveError::InvalidInput(
            "GC backup receipt must be a nonzero digest",
        ));
    }
    let (root, journal_bytes) =
        load_current_root(archive, chain, source)?.ok_or(ArchiveError::RangeUnavailable)?;
    let current_hash = root_bundle_hash(&root)?;
    let now = archive.now()?;
    let _leases = exclusive_gc_leases(archive, chain, source, current_hash)?;
    let files = eligible_files(
        archive,
        chain,
        source,
        &root,
        &journal_bytes,
        current_hash,
        now,
    )?;
    let plan = RawArchiveGcPlan {
        schema: RAW_GC_PLAN_SCHEMA_V3.to_owned(),
        root_sha256: hex::encode(current_hash),
        backup_receipt_sha256: hex::encode(backup_receipt),
        retention_horizon_seconds: archive.workload.retention_horizon_seconds(),
        created_at_micros: now.unix_micros(),
        files,
        digest: [0; 32],
    };
    let digest = gc_plan_digest(&plan)?;
    let plan = RawArchiveGcPlan { digest, ..plan };
    archive.workload.validate_backlog(
        0,
        plan.files.iter().try_fold(0_u64, |total, file| {
            total
                .checked_add(file.byte_len)
                .ok_or(ArchiveError::InvalidInput("GC eligible bytes overflow"))
        })?,
        u64::try_from(plan.files.len())
            .map_err(|_| ArchiveError::InvalidInput("GC eligible inode count exceeds u64"))?,
    )?;
    let bytes = manifest::canonical_json(&plan)?;
    let dataset = dataset_relative(chain, source);
    fs::publish_immutable(&archive.root, &plan_relative(&dataset, digest), &bytes)?;
    Ok(plan)
}

pub fn execute_packed_object_gc(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
    plan_digest: [u8; 32],
    backup_receipt: [u8; 32],
) -> Result<RawArchiveGcReceipt, ArchiveError> {
    if backup_receipt == [0; 32] {
        return Err(ArchiveError::InvalidInput(
            "GC backup receipt must be a nonzero digest",
        ));
    }
    let dataset = dataset_relative(chain, source);
    let plan = load_plan(archive, &dataset, plan_digest)?;
    if manifest::parse_hash(&plan.backup_receipt_sha256)? != backup_receipt {
        return Err(ArchiveError::ManifestVerification(
            "GC backup receipt does not match the authorized plan",
        ));
    }
    let (root, _) =
        load_current_root(archive, chain, source)?.ok_or(ArchiveError::RangeUnavailable)?;
    let current_hash = root_bundle_hash(&root)?;
    if hex::encode(current_hash) != plan.root_sha256 {
        return Err(ArchiveError::ManifestVerification(
            "CURRENT moved after the GC plan was authorized",
        ));
    }
    let _leases = exclusive_gc_leases(archive, chain, source, current_hash)?;
    let journal_relative = deletion_journal_relative(&dataset, plan_digest);
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
                &DeletionJournalRecordV3::Planned {
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
                    current_hash,
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
                &DeletionJournalRecordV3::Unlinked {
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
            &DeletionJournalRecordV3::DirectorySynced {
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
    Ok(RawArchiveGcReceipt {
        plan_digest,
        unlinked_files,
        unlinked_bytes,
    })
}

fn exclusive_gc_leases(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
    current_hash: [u8; 32],
) -> Result<Vec<File>, ArchiveError> {
    let dataset = dataset_relative(chain, source);
    let checkpoint_hash = super::checkpoint::checkpoint_root_sha256(archive, chain, source)?;
    let mut leases = Vec::new();
    let mut locked = BTreeSet::new();
    for hash in list_root_hashes(archive, &dataset)? {
        if hash == current_hash || checkpoint_hash == Some(hash) {
            continue;
        }
        lock_exclusive_root(archive, chain, source, &dataset, hash, &mut leases)?;
        locked.insert(hash);
    }
    for hash in list_lease_hashes(archive, &dataset)? {
        if hash == current_hash || checkpoint_hash == Some(hash) || locked.contains(&hash) {
            continue;
        }
        lock_exclusive_root(archive, chain, source, &dataset, hash, &mut leases)?;
    }
    Ok(leases)
}

fn lock_exclusive_root(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
    dataset: &Path,
    hash: [u8; 32],
    leases: &mut Vec<File>,
) -> Result<(), ArchiveError> {
    let relative = dataset.join(RawArchiveRootLeaseIdentity::new(hash).relative_path());
    leases.push(fs::open_exclusive_lease(&archive.root, &relative)?);
    let root_path = root_relative(dataset, hash);
    if !fs::exists_regular(&archive.root, &root_path)? {
        return Ok(());
    }
    let (loaded, _) = load_verified_root(archive, chain, source, hash)?;
    if root_bundle_hash(&loaded)? != hash {
        return Err(ArchiveError::ManifestVerification(
            "exclusive GC lease does not bind the selected root",
        ));
    }
    Ok(())
}

fn eligible_files(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
    current: &RootBundleV3,
    journal_bytes: &[u8],
    current_hash: [u8; 32],
    now: KnownTime,
) -> Result<Vec<GcPlanFileV3>, ArchiveError> {
    let dataset = dataset_relative(chain, source);
    let checkpoint_hash = super::checkpoint::checkpoint_root_sha256(archive, chain, source)?;
    let horizon = archive.workload.retention_horizon_seconds();
    let mut protected = BTreeSet::new();
    collect_root_paths(
        archive,
        chain,
        source,
        current,
        journal_bytes,
        &mut protected,
        true,
    )?;
    protected.insert(root_relative(&dataset, current_hash));
    if let Some(hash) = checkpoint_hash {
        let (root, bytes) = load_verified_root(archive, chain, source, hash)?;
        collect_root_paths(archive, chain, source, &root, &bytes, &mut protected, false)?;
        protected.insert(root_relative(&dataset, hash));
    }
    let mut eligible = BTreeMap::new();
    collect_packed_input_objects(
        archive,
        chain,
        source,
        current,
        journal_bytes,
        now,
        horizon,
        &protected,
        &mut eligible,
    )?;
    for hash in list_root_hashes(archive, &dataset)? {
        if hash == current_hash || checkpoint_hash == Some(hash) {
            continue;
        }
        let (root, bytes) = load_verified_root(archive, chain, source, hash)?;
        if !retention_elapsed(root.created_at_micros(), horizon, now)? {
            collect_root_paths(archive, chain, source, &root, &bytes, &mut protected, false)?;
            continue;
        }
        insert_eligible(
            archive,
            root_relative(&dataset, hash),
            &protected,
            &mut eligible,
        )?;
        insert_eligible(
            archive,
            dataset.join(RawArchiveRootLeaseIdentity::new(hash).relative_path()),
            &protected,
            &mut eligible,
        )?;
    }
    for hash in list_lease_hashes(archive, &dataset)? {
        if hash == current_hash || checkpoint_hash == Some(hash) {
            continue;
        }
        if fs::exists_regular(&archive.root, &root_relative(&dataset, hash))? {
            continue;
        }
        insert_eligible(
            archive,
            dataset.join(RawArchiveRootLeaseIdentity::new(hash).relative_path()),
            &protected,
            &mut eligible,
        )?;
    }
    eligible.retain(|path, _| !protected.contains(path));
    Ok(eligible.into_values().collect())
}

#[allow(clippy::too_many_arguments)]
fn collect_packed_input_objects(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
    root: &RootBundleV3,
    journal_bytes: &[u8],
    now: KnownTime,
    horizon: u64,
    protected: &BTreeSet<PathBuf>,
    eligible: &mut BTreeMap<PathBuf, GcPlanFileV3>,
) -> Result<(), ArchiveError> {
    let packs = load_packs_for_tree(archive, chain, source, root.sequence_root(), journal_bytes)?;
    walk_logical_leaves(root.sequence_root(), journal_bytes, &packs, &mut |entry| {
        if entry.storage().pack_manifest_sha256()?.is_none() {
            return Ok(false);
        }
        let pack = load_pack_manifest(archive, chain, source, entry)?;
        if !retention_elapsed(pack.created_at_micros(), horizon, now)? {
            return Ok(false);
        }
        for input in pack.inputs() {
            let commit = parse_logical_commit_manifest(input.canonical_manifest_json().as_bytes())?;
            insert_eligible(
                archive,
                PathBuf::from(commit.object().relative_path()),
                protected,
                eligible,
            )?;
            insert_eligible(
                archive,
                super::logical_manifest_relative(input.manifest_sha256()?),
                protected,
                eligible,
            )?;
        }
        Ok(false)
    })?;
    Ok(())
}

fn collect_root_paths(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
    root: &RootBundleV3,
    journal_bytes: &[u8],
    protected: &mut BTreeSet<PathBuf>,
    skip_packed_inputs: bool,
) -> Result<(), ArchiveError> {
    let dataset = dataset_relative(chain, source);
    protected.insert(PathBuf::from(root.journal_prefix().relative_path()));
    let packs = load_packs_for_tree(archive, chain, source, root.sequence_root(), journal_bytes)?;
    walk_logical_leaves(root.sequence_root(), journal_bytes, &packs, &mut |entry| {
        protect_leaf(
            archive,
            chain,
            source,
            &dataset,
            entry,
            protected,
            skip_packed_inputs,
        )?;
        Ok(false)
    })?;
    Ok(())
}

fn protect_leaf(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
    dataset: &Path,
    entry: &SequenceLeafEntryV3,
    protected: &mut BTreeSet<PathBuf>,
    skip_packed_inputs: bool,
) -> Result<(), ArchiveError> {
    match entry.storage() {
        SequenceStorageRefV3::Logical {
            manifest_relative_path,
            manifest_sha256,
        } => {
            protected.insert(PathBuf::from(manifest_relative_path));
            let hash = manifest::parse_hash(manifest_sha256)?;
            let commit = load_logical_commit(archive, Path::new(manifest_relative_path), hash)?;
            protected.insert(PathBuf::from(commit.object().relative_path()));
        }
        SequenceStorageRefV3::Packed {
            pack_manifest_relative_path,
            ..
        } => {
            protected.insert(PathBuf::from(pack_manifest_relative_path));
            let pack = load_pack_manifest(archive, chain, source, entry)?;
            protected.insert(dataset.join(pack.object().relative_path()));
            for input in pack.inputs() {
                if skip_packed_inputs {
                    continue;
                }
                let commit =
                    parse_logical_commit_manifest(input.canonical_manifest_json().as_bytes())?;
                protected.insert(super::logical_manifest_relative(input.manifest_sha256()?));
                protected.insert(PathBuf::from(commit.object().relative_path()));
            }
        }
    }
    Ok(())
}

fn insert_eligible(
    archive: &RawV3Archive,
    relative: PathBuf,
    protected: &BTreeSet<PathBuf>,
    eligible: &mut BTreeMap<PathBuf, GcPlanFileV3>,
) -> Result<(), ArchiveError> {
    if protected.contains(&relative) {
        return Ok(());
    }
    fs::validate_relative(&relative)?;
    if !fs::exists_regular(&archive.root, &relative)? {
        return Ok(());
    }
    let (digest, byte_len) =
        fs::regular_digest(&archive.root, &relative, archive.config.max_read_bytes())?;
    eligible.insert(
        relative.clone(),
        GcPlanFileV3 {
            relative_path: crate::raw::path_string(&relative)?,
            object_sha256: hex::encode(digest),
            byte_len,
        },
    );
    Ok(())
}

fn recheck_before_unlink(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
    expected_root: [u8; 32],
    backup_receipt: [u8; 32],
    plan: &RawArchiveGcPlan,
    file: &GcPlanFileV3,
) -> Result<(), ArchiveError> {
    let (root, _) =
        load_current_root(archive, chain, source)?.ok_or(ArchiveError::RangeUnavailable)?;
    if root_bundle_hash(&root)? != expected_root {
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

fn load_plan(
    archive: &RawV3Archive,
    dataset: &Path,
    digest: [u8; 32],
) -> Result<RawArchiveGcPlan, ArchiveError> {
    let bytes = fs::read_regular(
        &archive.root,
        &plan_relative(dataset, digest),
        MAX_GC_PLAN_BYTES,
    )?;
    let wire: GcPlanWireV3 = serde_json::from_slice(&bytes)
        .map_err(|_| ArchiveError::ManifestVerification("invalid raw V3 GC plan JSON"))?;
    if wire.schema != RAW_GC_PLAN_SCHEMA_V3 {
        return Err(ArchiveError::ManifestVerification(
            "unsupported raw V3 GC plan schema",
        ));
    }
    let plan = RawArchiveGcPlan {
        schema: wire.schema,
        root_sha256: wire.root_sha256,
        backup_receipt_sha256: wire.backup_receipt_sha256,
        retention_horizon_seconds: wire.retention_horizon_seconds,
        created_at_micros: wire.created_at_micros,
        files: wire.files,
        digest: [0; 32],
    };
    let computed = gc_plan_digest(&plan)?;
    if computed != digest
        || manifest::canonical_json(&RawArchiveGcPlan {
            digest: computed,
            ..plan.clone()
        })? != bytes
    {
        return Err(ArchiveError::ManifestVerification(
            "GC plan digest or canonical bytes are invalid",
        ));
    }
    Ok(RawArchiveGcPlan {
        digest: computed,
        ..plan
    })
}

fn gc_plan_digest(plan: &RawArchiveGcPlan) -> Result<[u8; 32], ArchiveError> {
    let root = manifest::parse_hash(&plan.root_sha256)?;
    let backup = manifest::parse_hash(&plan.backup_receipt_sha256)?;
    let mut evidence = Vec::new();
    evidence.extend_from_slice(&root);
    evidence.extend_from_slice(&backup);
    evidence.extend_from_slice(&plan.retention_horizon_seconds.to_be_bytes());
    evidence.extend_from_slice(&plan.created_at_micros.to_be_bytes());
    for file in &plan.files {
        let path = file.relative_path.as_bytes();
        let path_len = u64::try_from(path.len())
            .map_err(|_| ArchiveError::InvalidInput("GC plan path exceeds u64"))?;
        evidence.extend_from_slice(&path_len.to_be_bytes());
        evidence.extend_from_slice(path);
        evidence.extend_from_slice(&manifest::parse_hash(&file.object_sha256)?);
        evidence.extend_from_slice(&file.byte_len.to_be_bytes());
    }
    raw_v3::domain_hash(GC_PLAN_HASH_DOMAIN_V3, &evidence)
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
        let record: DeletionJournalRecordV3 = serde_json::from_str(line)
            .map_err(|_| ArchiveError::ManifestVerification("invalid deletion journal record"))?;
        match record {
            DeletionJournalRecordV3::Planned {
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
            DeletionJournalRecordV3::Unlinked {
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
            DeletionJournalRecordV3::DirectorySynced { relative_path } => {
                states.insert(relative_path, DeletionState::DirectorySynced);
            }
        }
    }
    Ok(states)
}

fn append_record(
    archive: &RawV3Archive,
    relative: &Path,
    record: &DeletionJournalRecordV3,
) -> Result<(), ArchiveError> {
    let line = serde_json::to_vec(record)
        .map_err(|_| ArchiveError::Codec("serializing deletion journal record".into()))?;
    fs::append_journal_line(&archive.root, relative, &line, MAX_DELETION_JOURNAL_BYTES)
}

fn list_root_hashes(archive: &RawV3Archive, dataset: &Path) -> Result<Vec<[u8; 32]>, ArchiveError> {
    list_named_hashes(archive, &dataset.join("roots"), "root-", ".json")
}

fn list_lease_hashes(
    archive: &RawV3Archive,
    dataset: &Path,
) -> Result<Vec<[u8; 32]>, ArchiveError> {
    list_named_hashes(archive, &dataset.join("leases"), "root-", ".lease")
}

fn list_named_hashes(
    archive: &RawV3Archive,
    relative: &Path,
    prefix: &str,
    suffix: &str,
) -> Result<Vec<[u8; 32]>, ArchiveError> {
    let names = fs::list_regular_names(&archive.root, relative)?;
    let mut hashes = Vec::new();
    for name in names {
        hashes.push(parse_named_hash(&name, prefix, suffix)?);
    }
    hashes.sort();
    hashes.dedup();
    Ok(hashes)
}

fn journal_matches_plan(
    file: &GcPlanFileV3,
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

fn parse_named_hash(name: &str, prefix: &str, suffix: &str) -> Result<[u8; 32], ArchiveError> {
    let hex = name
        .strip_prefix(prefix)
        .and_then(|value| value.strip_suffix(suffix))
        .ok_or(ArchiveError::ManifestVerification(
            "archive root filename is not content-addressed",
        ))?;
    manifest::parse_hash(hex)
}

fn plan_relative(dataset: &Path, digest: [u8; 32]) -> PathBuf {
    dataset
        .join("gc")
        .join(format!("plan-{}.json", hex::encode(digest)))
}

fn deletion_journal_relative(dataset: &Path, digest: [u8; 32]) -> PathBuf {
    dataset
        .join("gc")
        .join(format!("deletion-{}.log", hex::encode(digest)))
}

fn retention_elapsed(
    created_at_micros: i64,
    horizon_seconds: u64,
    now: KnownTime,
) -> Result<bool, ArchiveError> {
    let horizon_micros = i64::try_from(horizon_seconds.checked_mul(1_000_000).ok_or(
        ArchiveError::InvalidInput("retention horizon overflows microseconds"),
    )?)
    .map_err(|_| ArchiveError::InvalidInput("retention horizon exceeds i64 microseconds"))?;
    let age =
        now.unix_micros()
            .checked_sub(created_at_micros)
            .ok_or(ArchiveError::InvalidInput(
                "archive clock is before object creation",
            ))?;
    Ok(age >= horizon_micros)
}
