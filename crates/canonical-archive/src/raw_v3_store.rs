use std::{
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::Mutex,
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
use sha2::{Digest, Sha256};
use storage_ports::{
    ArchiveError, CursorPolicy, LocalRecordSequence, LocalRecordSequenceRange,
    RAW_ARCHIVE_MAXIMUM_INDEX_PACK_BYTES, RawArchiveCapacityBudgets,
    RawArchiveDurableFormatEnvelope, RawArchiveObject, RawArchiveProductionCapacityAdmission,
    RawArchiveWorkloadEnvelope, RawObservationArchive, RawObservationBatch, RawObservationIterator,
    RawObservationRange, RawObservationReceipt, SequencedRawObservationIterator,
    VerifiedRawManifest,
};

use super::{
    ArchiveConfig, fs, manifest, raw, raw_policy,
    raw_v3::{
        self, IndexPackBytes, JournalGenerationBuilderV3, LogicalCommitDescriptorV3,
        LogicalCommitManifestV3, LogicalObjectDescriptorV3, MAX_JOURNAL_BYTES, RAW_BYTE_DATASET_V3,
        RootBundleV3, SequenceLeafEntryV3, SequenceNodeRefV3, SequenceStorageRefV3,
        append_logical_entry, journal_file_identity, journal_needs_rotation,
        load_sequence_internal, load_sequence_leaf, logical_object_relative_path,
        pack_journal_leaves, parse_logical_commit_manifest, parse_root_bundle, root_bundle_hash,
        seed_rotated_journal_root,
    },
    schema,
};

#[derive(Debug)]
pub struct RawV3Archive {
    root: PathBuf,
    config: ArchiveConfig,
    writer: Mutex<()>,
    workload: RawArchiveWorkloadEnvelope,
    admission: RawArchiveProductionCapacityAdmission,
}

impl RawV3Archive {
    pub fn open(
        root: impl AsRef<Path>,
        config: ArchiveConfig,
        workload: RawArchiveWorkloadEnvelope,
        budgets: RawArchiveCapacityBudgets,
    ) -> Result<Self, ArchiveError> {
        let admission = RawArchiveProductionCapacityAdmission::evaluate(
            workload,
            RawArchiveDurableFormatEnvelope::production(),
            budgets,
        )?;
        let root = root.as_ref();
        if root.as_os_str().is_empty() {
            return Err(ArchiveError::UnsafePath);
        }
        if root.exists() {
            let metadata = std::fs::symlink_metadata(root)
                .map_err(|_| ArchiveError::Io("inspecting archive root"))?;
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Err(ArchiveError::UnsafePath);
            }
        } else {
            std::fs::create_dir_all(root).map_err(|_| ArchiveError::Io("creating archive root"))?;
        }
        let root = root
            .canonicalize()
            .map_err(|_| ArchiveError::Io("canonicalizing archive root"))?;
        Ok(Self {
            root,
            config,
            writer: Mutex::new(()),
            workload,
            admission,
        })
    }

    #[must_use]
    pub const fn admission(&self) -> RawArchiveProductionCapacityAdmission {
        self.admission
    }

    fn now(&self) -> Result<KnownTime, ArchiveError> {
        self.config.now()
    }

    pub fn pack_index(&self, chain: &ChainId, source: &SourceId) -> Result<[u8; 32], ArchiveError> {
        let _in_process = self.writer.lock().map_err(|_| ArchiveError::WriterBusy)?;
        let _process_lock =
            fs::open_writer_lock(&self.root, &raw_policy::writer_lock_relative(chain, source))?;
        raw_policy::ensure_append_policy(
            &self.root,
            chain,
            source,
            raw_policy::RawPolicy::MonotonicByteV3,
        )?;
        pack_index_locked(self, chain, source, self.now()?)
    }
}

impl RawObservationArchive for RawV3Archive {
    fn append_batch(
        &self,
        batch: &RawObservationBatch,
    ) -> Result<RawObservationReceipt, ArchiveError> {
        let _in_process = self.writer.lock().map_err(|_| ArchiveError::WriterBusy)?;
        append_batch(self, batch, self.now()?)
    }

    fn read_observations(
        &self,
        chain: &ChainId,
        source: &SourceId,
        range: RawObservationRange,
    ) -> Result<RawObservationIterator, ArchiveError> {
        read_observations(self, chain, source, range)
    }

    fn read_observations_by_sequence(
        &self,
        chain: &ChainId,
        source: &SourceId,
        range: LocalRecordSequenceRange,
    ) -> Result<SequencedRawObservationIterator, ArchiveError> {
        read_observations_by_sequence(self, chain, source, range)
    }

    fn verify_raw_manifest(
        &self,
        manifest: &ManifestId,
    ) -> Result<VerifiedRawManifest, ArchiveError> {
        verify_raw_manifest(self, manifest)
    }

    fn contains_raw_cursor_epoch(
        &self,
        chain: &ChainId,
        source: &SourceId,
        cursor_epoch: &str,
    ) -> Result<bool, ArchiveError> {
        contains_raw_cursor_epoch(self, chain, source, cursor_epoch)
    }
}

fn append_batch(
    archive: &RawV3Archive,
    batch: &RawObservationBatch,
    durable_at: KnownTime,
) -> Result<RawObservationReceipt, ArchiveError> {
    if batch.cursor_policy() != CursorPolicy::MonotonicByteOffset {
        return Err(ArchiveError::InvalidInput(
            "raw V3 archive requires monotonic byte offsets",
        ));
    }
    let first = batch
        .observations()
        .first()
        .ok_or(ArchiveError::InvalidInput("raw observation batch is empty"))?;
    admit_batch(archive, batch)?;
    let chain = batch.chain_id();
    let source = first.source_id();
    let _process_lock = fs::open_writer_lock(
        &archive.root,
        &raw_policy::writer_lock_relative(chain, source),
    )?;
    raw_policy::ensure_append_policy(
        &archive.root,
        chain,
        source,
        raw_policy::RawPolicy::MonotonicByteV3,
    )?;

    let mut current = load_current_root(archive, chain, source)?;
    if let Some((root, _)) = current.as_ref()
        && journal_needs_rotation(
            root.journal_prefix().committed_record_count(),
            root.journal_prefix().committed_prefix_length(),
            root.sequence_root().depth(),
        )
    {
        pack_index_locked(archive, chain, source, durable_at)?;
        current = load_current_root(archive, chain, source)?;
    }
    let descriptor = commit_descriptor(batch)?;
    if let Some((root, journal_bytes)) = current.as_ref() {
        archive.workload.validate_backlog(
            root.logical_manifest_count()
                .checked_add(1)
                .ok_or(ArchiveError::InvalidInput(
                    "raw V3 logical commit count overflows",
                ))?,
            0,
            0,
        )?;
        if let Some(existing) = find_exact_logical(archive, root, journal_bytes, &descriptor)? {
            return Ok(existing);
        }
        let expected_next =
            root.head_local_sequence()
                .checked_add(1)
                .ok_or(ArchiveError::InvalidInput(
                    "raw V3 local sequence overflows",
                ))?;
        if descriptor.first_local_sequence() != expected_next {
            return Err(ArchiveError::InvalidInput(
                "raw V3 local sequence does not extend the sequence head",
            ));
        }
    } else {
        archive.workload.validate_backlog(1, 0, 0)?;
        if descriptor.first_local_sequence() != 1 {
            return Err(ArchiveError::InvalidInput(
                "raw V3 first logical commit must start at local sequence one",
            ));
        }
    }

    let schema_fingerprint = schema::raw_schema_fingerprint()?;
    let object = write_object(archive, batch, &descriptor, schema_fingerprint)?;
    let commit = LogicalCommitManifestV3::try_new(
        archive.config.producer_build_id(),
        durable_at,
        descriptor.clone(),
        object,
    )?;
    let commit_bytes = manifest::canonical_json(&commit)?;
    let commit_hash = manifest::sha256(&commit_bytes);
    let commit_relative = logical_manifest_relative(commit_hash);
    fs::publish_immutable(&archive.root, &commit_relative, &commit_bytes)?;

    let dataset = dataset_relative(chain, source);
    let (mut journal, previous_root, previous_pointer, next_generation) = match current.as_ref() {
        None => {
            let generation = 1_u64;
            let identity = journal_file_identity(generation)?;
            let relative = journal_relative(&dataset, generation);
            (
                JournalGenerationBuilderV3::try_new(generation, identity, relative)?,
                None,
                None,
                1_u64,
            )
        }
        Some((root, journal_bytes)) => {
            let prefix = root.journal_prefix();
            let committed_len =
                usize::try_from(prefix.committed_prefix_length()).map_err(|_| {
                    ArchiveError::ManifestVerification("journal prefix exceeds address space")
                })?;
            let committed =
                journal_bytes
                    .get(..committed_len)
                    .ok_or(ArchiveError::ManifestVerification(
                        "journal file is shorter than the committed prefix",
                    ))?;
            let builder = JournalGenerationBuilderV3::try_resume(
                prefix.generation(),
                prefix.file_identity(),
                PathBuf::from(prefix.relative_path()),
                committed.to_vec(),
                prefix,
                chain.as_str(),
                source.as_str(),
            )?;
            let pointer = fs::read_regular(&archive.root, &dataset.join("CURRENT"), 64 * 1024)?;
            (
                builder,
                Some(root.sequence_root().clone()),
                Some(pointer),
                root.generation()
                    .checked_add(1)
                    .ok_or(ArchiveError::InvalidInput(
                        "raw V3 root generation overflows",
                    ))?,
            )
        }
    };

    let packs = match current.as_ref() {
        Some((root, journal_bytes)) => {
            load_packs_for_tree(archive, chain, source, root.sequence_root(), journal_bytes)?
        }
        None => IndexPackBytes::new(),
    };
    let entry = SequenceLeafEntryV3::try_new_logical(
        commit.commit().first_local_sequence(),
        commit.commit().last_local_sequence(),
        raw::path_string(&commit_relative)?,
        commit_hash,
        commit.object().size_bytes(),
        commit.object().row_count(),
        commit.commit().partition()?,
    )?;
    let sequence_root = append_logical_entry(
        &mut journal,
        &packs,
        previous_root.as_ref(),
        chain.clone(),
        source.clone(),
        entry,
    )?;
    let journal_commit = journal.commit_prefix(&sequence_root)?;
    let previous_prefix = current
        .as_ref()
        .map(|(root, _)| {
            let len =
                usize::try_from(root.journal_prefix().committed_prefix_length()).map_err(|_| {
                    ArchiveError::ManifestVerification("journal prefix exceeds address space")
                })?;
            Ok::<_, ArchiveError>(journal_commit.bytes()[..len].to_vec())
        })
        .transpose()?;
    fs::extend_append_only(
        &archive.root,
        Path::new(journal_commit.prefix().relative_path()),
        previous_prefix.as_deref().unwrap_or(&[]),
        journal_commit.bytes(),
        MAX_JOURNAL_BYTES,
    )?;

    let previous_root_hash = current
        .as_ref()
        .map(|(root, _)| root_bundle_hash(root))
        .transpose()?;
    let bundle = RootBundleV3::try_new(
        chain.clone(),
        source.clone(),
        next_generation,
        previous_root_hash,
        &journal_commit,
        durable_at,
    )?;
    let bundle_bytes = raw_v3::canonical_root_bytes(&bundle)?;
    let bundle_hash = root_bundle_hash(&bundle)?;
    let bundle_relative = root_relative(&dataset, bundle_hash);
    fs::publish_immutable(&archive.root, &bundle_relative, &bundle_bytes)?;

    let pointer = manifest::CurrentPointerV1 {
        schema: manifest::CURRENT_POINTER_SCHEMA_V1.to_owned(),
        manifest_relative_path: raw::path_string(&bundle_relative)?,
        manifest_sha256: hex::encode(bundle_hash),
    };
    let pointer_bytes = manifest::canonical_json(&pointer)?;
    fs::publish_current_cas(
        &archive.root,
        &dataset.join("CURRENT"),
        previous_pointer.as_deref(),
        &pointer_bytes,
    )?;

    let readback = load_current_root(archive, chain, source)?.ok_or(
        ArchiveError::ManifestVerification("raw V3 CURRENT readback is missing"),
    )?;
    if root_bundle_hash(&readback.0)? != bundle_hash {
        return Err(ArchiveError::ManifestVerification(
            "raw V3 CURRENT readback does not bind the published root",
        ));
    }
    let verified = verify_logical_at_sequence(
        archive,
        &readback.0,
        &readback.1,
        commit_hash,
        LocalRecordSequenceRange::try_new(
            LocalRecordSequence::try_new(commit.commit().first_local_sequence())?,
            LocalRecordSequence::try_new(commit.commit().last_local_sequence())?,
        )?,
    )?;
    receipt(&commit, commit_hash, &verified, durable_at)
}

fn admit_batch(archive: &RawV3Archive, batch: &RawObservationBatch) -> Result<(), ArchiveError> {
    for observation in batch.observations() {
        let encoded = u64::try_from(observation.payload().len())
            .map_err(|_| ArchiveError::InvalidInput("raw observation payload exceeds u64"))?;
        archive.workload.validate_record_bytes(encoded)?;
    }
    Ok(())
}

fn commit_descriptor(
    batch: &RawObservationBatch,
) -> Result<LogicalCommitDescriptorV3, ArchiveError> {
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
            "raw V3 batch is missing local sequence evidence",
        ))?;
    let partition = manifest::partition_for(first.received().wall_micros())?;
    for observation in batch.observations().iter().skip(1) {
        if manifest::partition_for(observation.received().wall_micros())? != partition {
            return Err(ArchiveError::InvalidInput(
                "raw observation batch crosses an hour partition",
            ));
        }
    }
    LogicalCommitDescriptorV3::try_new(
        batch.chain_id().clone(),
        first.source_id().clone(),
        first.source_version(),
        raw::observation_class_name(first.observation_class())?,
        first.cursor().epoch(),
        first.cursor().offset(),
        last.cursor().offset(),
        sequence_range.start().get(),
        sequence_range.end().get(),
        first.received().wall_micros(),
        last.received().wall_micros(),
        first.parser_schema_version(),
        batch.spool_manifest_blake3(),
        batch.spool_segment_blake3(),
        rolling_content_hash(batch)?,
    )
}

fn rolling_content_hash(batch: &RawObservationBatch) -> Result<[u8; 32], ArchiveError> {
    let mut hasher = Sha256::new();
    hash_frame(&mut hasher, raw_v3::RAW_V3_ROLLING_HASH_DOMAIN)?;
    hash_frame(&mut hasher, b"monotonic-byte-offset")?;
    let sequenced = batch
        .sequenced_observations()
        .ok_or(ArchiveError::InvalidInput(
            "raw V3 batch is missing local sequence evidence",
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
    archive: &RawV3Archive,
    batch: &RawObservationBatch,
    descriptor: &LogicalCommitDescriptorV3,
    schema_fingerprint: [u8; 32],
) -> Result<LogicalObjectDescriptorV3, ArchiveError> {
    let object_hash_placeholder = [0_u8; 32];
    let relative_dir = PathBuf::from(logical_object_relative_path(
        descriptor,
        object_hash_placeholder,
    )?)
    .parent()
    .ok_or(ArchiveError::UnsafePath)?
    .to_path_buf();
    let mut staged = fs::create_parquet_staging_file(&archive.root, &relative_dir)?;
    let record_batch = raw::raw_record_batch(batch)?;
    let compression = ZstdLevel::try_new(3)
        .map_err(|_| ArchiveError::InvalidInput("invalid Parquet compression level"))?;
    let properties = WriterProperties::builder()
        .set_created_by("hyperliquid-alpha-desk/raw-archive-writer-v3".to_owned())
        .set_compression(Compression::ZSTD(compression))
        .set_statistics_enabled(EnabledStatistics::Page)
        .set_key_value_metadata(Some(vec![
            KeyValue::new(
                "alpha_desk.dataset".to_owned(),
                RAW_BYTE_DATASET_V3.to_owned(),
            ),
            KeyValue::new(
                "alpha_desk.cursor_policy".to_owned(),
                "monotonic-byte-offset".to_owned(),
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
        .map_err(|_| ArchiveError::Io("syncing raw V3 Parquet object"))?;
    let (hash, size_bytes) = hash_file(staged.as_file_mut())?;
    let relative = PathBuf::from(logical_object_relative_path(descriptor, hash)?);
    fs::publish_staged_immutable(&archive.root, &relative, staged)?;
    let published = fs::read_regular(&archive.root, &relative, size_bytes)?;
    if u64::try_from(published.len()).ok() != Some(size_bytes)
        || <[u8; 32]>::from(Sha256::digest(&published)) != hash
    {
        return Err(ArchiveError::CorruptObject(raw::path_string(&relative)?));
    }
    LogicalObjectDescriptorV3::try_new(
        relative,
        hash,
        size_bytes,
        u64::try_from(batch.observations().len())
            .map_err(|_| ArchiveError::InvalidInput("raw row count exceeds u64"))?,
    )
}

fn hash_file(file: &mut std::fs::File) -> Result<([u8; 32], u64), ArchiveError> {
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

fn load_current_root(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
) -> Result<Option<(RootBundleV3, Vec<u8>)>, ArchiveError> {
    let dataset = dataset_relative(chain, source);
    let current = dataset.join("CURRENT");
    let Some(bytes) = fs::try_read_regular(&archive.root, &current, 64 * 1024)? else {
        return Ok(None);
    };
    let pointer: manifest::CurrentPointerV1 = serde_json::from_slice(&bytes)
        .map_err(|_| ArchiveError::ManifestVerification("invalid raw V3 current pointer JSON"))?;
    if pointer.schema != manifest::CURRENT_POINTER_SCHEMA_V1 {
        return Err(ArchiveError::ManifestVerification(
            "unsupported raw V3 current pointer schema",
        ));
    }
    let hash = manifest::parse_hash(&pointer.manifest_sha256)?;
    let expected = root_relative(&dataset, hash);
    if Path::new(&pointer.manifest_relative_path) != expected {
        return Err(ArchiveError::ManifestVerification(
            "raw V3 current pointer does not bind exact root path",
        ));
    }
    let root_bytes = fs::read_manifest(&archive.root, &expected)?;
    let root = parse_root_bundle(&root_bytes)?;
    if root_bundle_hash(&root)? != hash {
        return Err(ArchiveError::ManifestVerification(
            "raw V3 current pointer hash mismatch",
        ));
    }
    if root.chain_id()?.as_str() != chain.as_str() || root.source_id()?.as_str() != source.as_str()
    {
        return Err(ArchiveError::ManifestVerification(
            "raw V3 root chain or source mismatch",
        ));
    }
    let prefix = root.journal_prefix();
    let journal_bytes = fs::read_regular(
        &archive.root,
        Path::new(prefix.relative_path()),
        MAX_JOURNAL_BYTES,
    )?;
    let committed_len = usize::try_from(prefix.committed_prefix_length())
        .map_err(|_| ArchiveError::ManifestVerification("journal prefix exceeds address space"))?;
    let committed =
        journal_bytes
            .get(..committed_len)
            .ok_or(ArchiveError::ManifestVerification(
                "journal file is shorter than the committed prefix",
            ))?;
    if raw_v3::journal_prefix_hash(committed)? != prefix.committed_prefix_sha256()? {
        return Err(ArchiveError::ManifestVerification(
            "journal committed prefix hash mismatch",
        ));
    }
    Ok(Some((root, journal_bytes)))
}

fn pack_index_locked(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
    durable_at: KnownTime,
) -> Result<[u8; 32], ArchiveError> {
    let (root, journal_bytes) =
        load_current_root(archive, chain, source)?.ok_or(ArchiveError::RangeUnavailable)?;
    if root.journal_prefix().committed_record_count() <= 1 {
        return root_bundle_hash(&root);
    }
    let dataset = dataset_relative(chain, source);
    let mut packs =
        load_packs_for_tree(archive, chain, source, root.sequence_root(), &journal_bytes)?;
    let (pack, packed_leaves) = pack_journal_leaves(
        chain.clone(),
        source.clone(),
        root.journal_prefix().generation(),
        root.sequence_root(),
        &journal_bytes,
        &packs,
    )?;
    let pack_hash = pack.object_sha256();
    let pack_relative = dataset.join(pack.manifest().object_relative_path());
    fs::publish_immutable(&archive.root, &pack_relative, pack.bytes())?;
    let published = fs::read_regular(
        &archive.root,
        &pack_relative,
        RAW_ARCHIVE_MAXIMUM_INDEX_PACK_BYTES,
    )?;
    pack.verify_bytes(&published)?;
    let manifest_relative = dataset.join(format!(
        "index-packs/{}.manifest.json",
        hex::encode(pack_hash)
    ));
    fs::publish_immutable(
        &archive.root,
        &manifest_relative,
        &manifest::canonical_json(pack.manifest())?,
    )?;
    packs.insert(pack_hash, pack.bytes().to_vec());

    let next_journal_generation =
        root.journal_prefix()
            .generation()
            .checked_add(1)
            .ok_or(ArchiveError::InvalidInput(
                "raw V3 journal generation overflows",
            ))?;
    let mut journal = JournalGenerationBuilderV3::try_new(
        next_journal_generation,
        journal_file_identity(next_journal_generation)?,
        journal_relative(&dataset, next_journal_generation),
    )?;
    let sequence_root = seed_rotated_journal_root(
        &mut journal,
        &packs,
        &packed_leaves,
        &journal_bytes,
        root.sequence_root(),
    )?;
    let journal_commit = journal.commit_prefix(&sequence_root)?;
    fs::extend_append_only(
        &archive.root,
        Path::new(journal_commit.prefix().relative_path()),
        &[],
        journal_commit.bytes(),
        MAX_JOURNAL_BYTES,
    )?;
    let previous_pointer = fs::read_regular(&archive.root, &dataset.join("CURRENT"), 64 * 1024)?;
    let next_generation = root
        .generation()
        .checked_add(1)
        .ok_or(ArchiveError::InvalidInput(
            "raw V3 root generation overflows",
        ))?;
    let previous_root_hash = root_bundle_hash(&root)?;
    let bundle = RootBundleV3::try_new(
        chain.clone(),
        source.clone(),
        next_generation,
        Some(previous_root_hash),
        &journal_commit,
        durable_at,
    )?;
    let bundle_bytes = raw_v3::canonical_root_bytes(&bundle)?;
    let bundle_hash = root_bundle_hash(&bundle)?;
    let bundle_relative = root_relative(&dataset, bundle_hash);
    fs::publish_immutable(&archive.root, &bundle_relative, &bundle_bytes)?;
    let pointer = manifest::CurrentPointerV1 {
        schema: manifest::CURRENT_POINTER_SCHEMA_V1.to_owned(),
        manifest_relative_path: raw::path_string(&bundle_relative)?,
        manifest_sha256: hex::encode(bundle_hash),
    };
    let pointer_bytes = manifest::canonical_json(&pointer)?;
    fs::publish_current_cas(
        &archive.root,
        &dataset.join("CURRENT"),
        Some(&previous_pointer),
        &pointer_bytes,
    )?;
    let readback = load_current_root(archive, chain, source)?.ok_or(
        ArchiveError::ManifestVerification("raw V3 packed CURRENT readback is missing"),
    )?;
    if root_bundle_hash(&readback.0)? != bundle_hash {
        return Err(ArchiveError::ManifestVerification(
            "raw V3 packed CURRENT readback does not bind the published root",
        ));
    }
    Ok(bundle_hash)
}

fn load_packs_for_tree(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
    root: &SequenceNodeRefV3,
    journal_bytes: &[u8],
) -> Result<IndexPackBytes, ArchiveError> {
    let dataset = dataset_relative(chain, source);
    let mut packs = IndexPackBytes::new();
    load_packs_walk(archive, &dataset, journal_bytes, &mut packs, root)?;
    Ok(packs)
}

fn load_packs_walk(
    archive: &RawV3Archive,
    dataset: &Path,
    journal_bytes: &[u8],
    packs: &mut IndexPackBytes,
    node: &SequenceNodeRefV3,
) -> Result<(), ArchiveError> {
    ensure_pack_loaded(archive, dataset, packs, node.locator())?;
    if node.depth() == 0 {
        return Ok(());
    }
    let page = load_sequence_internal(journal_bytes, packs, node)?;
    for child in page.children() {
        load_packs_walk(archive, dataset, journal_bytes, packs, child)?;
    }
    Ok(())
}

fn ensure_pack_loaded(
    archive: &RawV3Archive,
    dataset: &Path,
    packs: &mut IndexPackBytes,
    locator: &raw_v3::SequencePageLocatorV3,
) -> Result<(), ArchiveError> {
    let Some(hash) = locator.index_pack_sha256()? else {
        return Ok(());
    };
    if packs.contains_key(&hash) {
        return Ok(());
    }
    let relative = locator
        .index_pack_relative_path()
        .ok_or(ArchiveError::ManifestVerification(
            "index pack locator is missing a path",
        ))?;
    let path = dataset.join(relative);
    let bytes = fs::read_regular(&archive.root, &path, RAW_ARCHIVE_MAXIMUM_INDEX_PACK_BYTES)?;
    if manifest::sha256(&bytes) != hash {
        return Err(ArchiveError::ManifestVerification(
            "index pack path does not bind exact bytes",
        ));
    }
    packs.insert(hash, bytes);
    Ok(())
}

fn find_exact_logical(
    archive: &RawV3Archive,
    root: &RootBundleV3,
    journal_bytes: &[u8],
    descriptor: &LogicalCommitDescriptorV3,
) -> Result<Option<RawObservationReceipt>, ArchiveError> {
    let packs = load_packs_for_tree(
        archive,
        &root.chain_id()?,
        &root.source_id()?,
        root.sequence_root(),
        journal_bytes,
    )?;
    let Some(entry) = lookup_leaf_covering(
        root.sequence_root(),
        journal_bytes,
        &packs,
        descriptor.first_local_sequence(),
        descriptor.last_local_sequence(),
    )?
    else {
        return Ok(None);
    };
    let Some(manifest_sha256) = entry.storage().logical_manifest_sha256()? else {
        return Err(ArchiveError::ManifestVerification(
            "packed V3 slice lookup is not implemented for uncompacted append retry",
        ));
    };
    let relative = entry.storage().logical_manifest_relative_path().ok_or(
        ArchiveError::ManifestVerification("logical sequence entry is missing a manifest path"),
    )?;
    let loaded = load_logical_commit(archive, Path::new(relative), manifest_sha256)?;
    if loaded.commit() != descriptor {
        return Err(ArchiveError::ConflictingRawRange {
            source_id: descriptor.source_id()?,
            epoch: descriptor.cursor_epoch().to_owned(),
            start: descriptor.start_offset(),
            end: descriptor.end_offset(),
        });
    }
    let verified = verify_loaded_commit(archive, &loaded, manifest_sha256)?;
    let durable_at = KnownTime::from_unix_micros(loaded.created_at_micros())
        .map_err(|_| ArchiveError::InvalidInput("logical commit time"))?;
    Ok(Some(receipt(
        &loaded,
        manifest_sha256,
        &verified,
        durable_at,
    )?))
}

fn lookup_leaf_covering(
    node: &SequenceNodeRefV3,
    journal_bytes: &[u8],
    packs: &IndexPackBytes,
    first: u64,
    last: u64,
) -> Result<Option<SequenceLeafEntryV3>, ArchiveError> {
    if last < node.first_local_sequence() || first > node.last_local_sequence() {
        return Ok(None);
    }
    if node.depth() == 0 {
        let page = load_sequence_leaf(journal_bytes, packs, node)?;
        return Ok(page
            .entries()
            .iter()
            .find(|entry| {
                entry.first_local_sequence() == first && entry.last_local_sequence() == last
            })
            .cloned());
    }
    let page = load_sequence_internal(journal_bytes, packs, node)?;
    for child in page.children() {
        if let Some(entry) = lookup_leaf_covering(child, journal_bytes, packs, first, last)? {
            return Ok(Some(entry));
        }
    }
    Ok(None)
}

fn collect_overlapping_leaves(
    node: &SequenceNodeRefV3,
    journal_bytes: &[u8],
    packs: &IndexPackBytes,
    range: LocalRecordSequenceRange,
    output: &mut Vec<SequenceLeafEntryV3>,
) -> Result<(), ArchiveError> {
    if range.end().get() < node.first_local_sequence()
        || range.start().get() > node.last_local_sequence()
    {
        return Ok(());
    }
    if node.depth() == 0 {
        let page = load_sequence_leaf(journal_bytes, packs, node)?;
        for entry in page.entries() {
            if entry.last_local_sequence() >= range.start().get()
                && entry.first_local_sequence() <= range.end().get()
            {
                output.push(entry.clone());
            }
        }
        return Ok(());
    }
    let page = load_sequence_internal(journal_bytes, packs, node)?;
    for child in page.children() {
        collect_overlapping_leaves(child, journal_bytes, packs, range, output)?;
    }
    Ok(())
}

fn walk_logical_leaves(
    node: &SequenceNodeRefV3,
    journal_bytes: &[u8],
    packs: &IndexPackBytes,
    visit: &mut impl FnMut(&SequenceLeafEntryV3) -> Result<bool, ArchiveError>,
) -> Result<bool, ArchiveError> {
    if node.depth() == 0 {
        let page = load_sequence_leaf(journal_bytes, packs, node)?;
        for entry in page.entries() {
            if visit(entry)? {
                return Ok(true);
            }
        }
        return Ok(false);
    }
    let page = load_sequence_internal(journal_bytes, packs, node)?;
    for child in page.children() {
        if walk_logical_leaves(child, journal_bytes, packs, visit)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn load_logical_commit(
    archive: &RawV3Archive,
    relative: &Path,
    expected_hash: [u8; 32],
) -> Result<LogicalCommitManifestV3, ArchiveError> {
    let bytes = fs::read_manifest(&archive.root, relative)?;
    if manifest::sha256(&bytes) != expected_hash {
        return Err(ArchiveError::ManifestVerification(
            "logical commit path does not bind exact bytes",
        ));
    }
    parse_logical_commit_manifest(&bytes)
}

fn verify_logical_at_sequence(
    archive: &RawV3Archive,
    root: &RootBundleV3,
    journal_bytes: &[u8],
    manifest_sha256: [u8; 32],
    expected_range: LocalRecordSequenceRange,
) -> Result<VerifiedRawManifest, ArchiveError> {
    let packs = load_packs_for_tree(
        archive,
        &root.chain_id()?,
        &root.source_id()?,
        root.sequence_root(),
        journal_bytes,
    )?;
    let entry = lookup_leaf_covering(
        root.sequence_root(),
        journal_bytes,
        &packs,
        expected_range.start().get(),
        expected_range.end().get(),
    )?
    .ok_or(ArchiveError::ManifestVerification(
        "sequence tree does not contain the expected logical range",
    ))?;
    let Some(entry_hash) = entry.storage().logical_manifest_sha256()? else {
        return Err(ArchiveError::ManifestVerification(
            "packed V3 slice verification is not implemented",
        ));
    };
    if entry_hash != manifest_sha256 {
        return Err(ArchiveError::ManifestVerification(
            "sequence tree leaf does not authenticate the logical commit",
        ));
    }
    let relative = entry.storage().logical_manifest_relative_path().ok_or(
        ArchiveError::ManifestVerification("logical sequence entry is missing a manifest path"),
    )?;
    let loaded = load_logical_commit(archive, Path::new(relative), manifest_sha256)?;
    verify_loaded_commit(archive, &loaded, manifest_sha256)
}

fn verify_raw_manifest(
    archive: &RawV3Archive,
    manifest_id: &ManifestId,
) -> Result<VerifiedRawManifest, ArchiveError> {
    let hash = manifest::hash_from_manifest_id(manifest_id)?;
    let relative = logical_manifest_relative(hash);
    let loaded = load_logical_commit(archive, &relative, hash)?;
    let chain = loaded.commit().chain_id()?;
    let source = loaded.commit().source_id()?;
    let (root, journal_bytes) = load_current_root(archive, &chain, &source)?
        .ok_or(ArchiveError::ReceiptIndexRebuildRequired)?;
    let packs = load_packs_for_tree(
        archive,
        &chain,
        &source,
        root.sequence_root(),
        &journal_bytes,
    )?;
    let mut found = false;
    walk_logical_leaves(root.sequence_root(), &journal_bytes, &packs, &mut |entry| {
        if entry.storage().logical_manifest_sha256()? == Some(hash) {
            found = true;
            Ok(true)
        } else {
            Ok(false)
        }
    })?;
    if !found {
        return Err(ArchiveError::ReceiptIndexRebuildRequired);
    }
    verify_loaded_commit(archive, &loaded, hash)
}

fn verify_loaded_commit(
    archive: &RawV3Archive,
    commit: &LogicalCommitManifestV3,
    manifest_hash: [u8; 32],
) -> Result<VerifiedRawManifest, ArchiveError> {
    let (observations, object) = verify_and_decode(archive, commit)?;
    drop(observations);
    Ok(VerifiedRawManifest::new(
        manifest::manifest_id(manifest_hash)?,
        manifest_hash,
        schema::raw_schema_fingerprint()?,
        commit.commit().rolling_content_sha256()?,
        commit.commit().spool_manifest_blake3()?,
        commit.commit().spool_segment_blake3()?,
        object,
    ))
}

fn verify_and_decode(
    archive: &RawV3Archive,
    commit: &LogicalCommitManifestV3,
) -> Result<(Vec<SourceObservation>, RawArchiveObject), ArchiveError> {
    let object_hash = commit.object().sha256()?;
    let expected = PathBuf::from(logical_object_relative_path(commit.commit(), object_hash)?);
    if Path::new(commit.object().relative_path()) != expected {
        return Err(ArchiveError::ManifestVerification(
            "raw V3 manifest does not bind exact object path",
        ));
    }
    let bytes = fs::read_regular(&archive.root, &expected, archive.config.max_read_bytes())
        .map_err(|error| match error {
            ArchiveError::Io(_) => {
                ArchiveError::CorruptObject(commit.object().relative_path().to_owned())
            }
            other => other,
        })?;
    if u64::try_from(bytes.len()).ok() != Some(commit.object().size_bytes())
        || <[u8; 32]>::from(Sha256::digest(&bytes)) != object_hash
    {
        return Err(ArchiveError::CorruptObject(
            commit.object().relative_path().to_owned(),
        ));
    }
    let builder = ParquetRecordBatchReaderBuilder::try_new(bytes::Bytes::from(bytes))
        .map_err(|_| ArchiveError::CorruptObject(commit.object().relative_path().to_owned()))?;
    if builder.schema().fields() != schema::raw_schema().fields() {
        return Err(ArchiveError::SchemaMismatch);
    }
    let reader = builder
        .build()
        .map_err(|_| ArchiveError::CorruptObject(commit.object().relative_path().to_owned()))?;
    let mut observations = Vec::new();
    for batch in reader {
        decode_raw_batch(
            &batch.map_err(|_| {
                ArchiveError::CorruptObject(commit.object().relative_path().to_owned())
            })?,
            commit,
            archive.config.max_read_bytes(),
            &mut observations,
        )?;
    }
    if u64::try_from(observations.len()).ok() != Some(commit.object().row_count()) {
        return Err(ArchiveError::ManifestVerification(
            "raw V3 Parquet row count does not match manifest",
        ));
    }
    let first_observation = observations
        .first()
        .ok_or(ArchiveError::ManifestVerification(
            "raw V3 Parquet object is empty",
        ))?;
    let last_observation = observations
        .last()
        .ok_or(ArchiveError::ManifestVerification(
            "raw V3 Parquet object is empty",
        ))?;
    if first_observation.cursor().offset() != commit.commit().start_offset()
        || last_observation.cursor().offset() != commit.commit().end_offset()
        || first_observation.received().wall_micros()
            != commit.commit().first_received_wall_micros()
        || last_observation.received().wall_micros() != commit.commit().last_received_wall_micros()
    {
        return Err(ArchiveError::ManifestVerification(
            "raw V3 descriptor boundaries disagree with Parquet rows",
        ));
    }
    let first_sequence = LocalRecordSequence::try_new(commit.commit().first_local_sequence())?;
    let reconstructed = RawObservationBatch::try_new_byte_offsets(
        commit.commit().chain_id()?,
        observations.clone(),
        commit.commit().spool_manifest_blake3()?,
        commit.commit().spool_segment_blake3()?,
        first_sequence,
    )?;
    if rolling_content_hash(&reconstructed)? != commit.commit().rolling_content_sha256()? {
        return Err(ArchiveError::ManifestVerification(
            "raw V3 rolling content hash mismatch",
        ));
    }
    let cursor_range = RawObservationRange::try_new(
        commit.commit().cursor_epoch(),
        commit.commit().start_offset(),
        commit.commit().end_offset(),
    )?;
    let object = RawArchiveObject::try_new_byte_offsets(
        PathBuf::from(commit.object().relative_path()),
        object_hash,
        commit.object().size_bytes(),
        commit.object().row_count(),
        commit.commit().chain_id()?,
        commit.commit().source_id()?,
        cursor_range,
        LocalRecordSequenceRange::try_new(
            LocalRecordSequence::try_new(commit.commit().first_local_sequence())?,
            LocalRecordSequence::try_new(commit.commit().last_local_sequence())?,
        )?,
    )?;
    Ok((observations, object))
}

fn decode_raw_batch(
    batch: &RecordBatch,
    commit: &LogicalCommitManifestV3,
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
    let first_partition = commit.commit().partition()?;
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
        if chains.value(row) != commit.commit().chain_id()?.as_str()
            || observation.source_id().as_str() != commit.commit().source_id()?.as_str()
            || observation.source_version() != commit.commit().source_version()
            || raw::observation_class_name(observation.observation_class())?
                != commit.commit().observation_class()
            || observation.cursor().epoch() != commit.commit().cursor_epoch()
            || observation.parser_schema_version() != commit.commit().parser_schema_version()
            || hashes.value(row) != observation.content_hash().as_bytes()
            || manifest::partition_for(observation.received().wall_micros())? != first_partition
        {
            return Err(ArchiveError::ManifestVerification(
                "raw Parquet query columns disagree with authoritative payload",
            ));
        }
        output.push(observation);
    }
    Ok(())
}

fn read_observations_by_sequence(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
    range: LocalRecordSequenceRange,
) -> Result<SequencedRawObservationIterator, ArchiveError> {
    if !raw_policy::ensure_read_policy(
        &archive.root,
        chain,
        source,
        raw_policy::RawPolicy::MonotonicByteV3,
    )? {
        return Err(ArchiveError::RangeUnavailable);
    }
    if range.len() > archive.config.max_read_blocks() {
        return Err(ArchiveError::InvalidInput(
            "raw observation sequence range exceeds configured record limit",
        ));
    }
    let (root, journal_bytes) =
        load_current_root(archive, chain, source)?.ok_or(ArchiveError::RangeUnavailable)?;
    let packs = load_packs_for_tree(archive, chain, source, root.sequence_root(), &journal_bytes)?;
    let mut leaves = Vec::new();
    collect_overlapping_leaves(
        root.sequence_root(),
        &journal_bytes,
        &packs,
        range,
        &mut leaves,
    )?;
    leaves.sort_by_key(SequenceLeafEntryV3::first_local_sequence);
    let mut total_bytes = 0_u64;
    let mut replayed = Vec::new();
    for entry in leaves {
        match entry.storage() {
            SequenceStorageRefV3::Packed { .. } => {
                return Err(ArchiveError::ManifestVerification(
                    "packed V3 read is not implemented",
                ));
            }
            SequenceStorageRefV3::Logical {
                manifest_relative_path,
                manifest_sha256,
            } => {
                let hash = manifest::parse_hash(manifest_sha256)?;
                let loaded = load_logical_commit(archive, Path::new(manifest_relative_path), hash)?;
                total_bytes = total_bytes
                    .checked_add(loaded.object().size_bytes())
                    .ok_or(ArchiveError::InvalidInput(
                        "raw observation byte count overflows",
                    ))?;
                if total_bytes > archive.config.max_read_bytes() {
                    return Err(ArchiveError::InvalidInput(
                        "raw observation sequence range exceeds configured byte limit",
                    ));
                }
                let (observations, _) = verify_and_decode(archive, &loaded)?;
                for (index, observation) in observations.into_iter().enumerate() {
                    let advance_by = u64::try_from(index).map_err(|_| {
                        ArchiveError::InvalidInput("local record sequence overflows")
                    })?;
                    let sequence = LocalRecordSequence::try_new(entry.first_local_sequence())?
                        .checked_advance_by(advance_by)?;
                    if range.contains(sequence) {
                        replayed.push(storage_ports::OwnedSequencedSourceObservation::new(
                            observation,
                            sequence,
                        ));
                    }
                }
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

fn read_observations(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
    range: RawObservationRange,
) -> Result<RawObservationIterator, ArchiveError> {
    if !raw_policy::ensure_read_policy(
        &archive.root,
        chain,
        source,
        raw_policy::RawPolicy::MonotonicByteV3,
    )? {
        return Err(ArchiveError::RangeUnavailable);
    }
    let (root, _) =
        load_current_root(archive, chain, source)?.ok_or(ArchiveError::RangeUnavailable)?;
    let full = LocalRecordSequenceRange::try_new(
        LocalRecordSequence::try_new(1)?,
        LocalRecordSequence::try_new(root.head_local_sequence())?,
    )?;
    let sequenced = read_observations_by_sequence(archive, chain, source, full)?;
    let mut matched = Vec::new();
    for item in sequenced {
        let item = item?;
        let cursor = item.observation().cursor();
        if cursor.epoch() == range.epoch()
            && cursor.offset() >= range.start_offset()
            && cursor.offset() <= range.end_offset()
        {
            matched.push(item.into_observation());
        }
    }
    if matched.is_empty() {
        return Err(ArchiveError::RangeUnavailable);
    }
    Ok(Box::new(matched.into_iter().map(Ok)))
}

fn contains_raw_cursor_epoch(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
    cursor_epoch: &str,
) -> Result<bool, ArchiveError> {
    SourceCursor::new(cursor_epoch.to_owned(), 0)
        .map_err(|_| ArchiveError::InvalidInput("raw cursor epoch"))?;
    if !raw_policy::ensure_read_policy(
        &archive.root,
        chain,
        source,
        raw_policy::RawPolicy::MonotonicByteV3,
    )? {
        return Ok(false);
    }
    let Some((root, journal_bytes)) = load_current_root(archive, chain, source)? else {
        return Ok(false);
    };
    let mut found = false;
    let packs = load_packs_for_tree(archive, chain, source, root.sequence_root(), &journal_bytes)?;
    walk_logical_leaves(root.sequence_root(), &journal_bytes, &packs, &mut |entry| {
        let Some(hash) = entry.storage().logical_manifest_sha256()? else {
            return Ok(false);
        };
        let relative = entry.storage().logical_manifest_relative_path().ok_or(
            ArchiveError::ManifestVerification("logical sequence entry is missing a manifest path"),
        )?;
        let loaded = load_logical_commit(archive, Path::new(relative), hash)?;
        if loaded.commit().cursor_epoch() == cursor_epoch {
            found = true;
            Ok(true)
        } else {
            Ok(false)
        }
    })?;
    Ok(found)
}

fn receipt(
    commit: &LogicalCommitManifestV3,
    manifest_hash: [u8; 32],
    verified: &VerifiedRawManifest,
    durable_at: KnownTime,
) -> Result<RawObservationReceipt, ArchiveError> {
    RawObservationReceipt::try_new_byte_offsets(
        format!("raw-v3-{}", hex::encode(manifest_hash)),
        manifest::manifest_id(manifest_hash)?,
        commit.commit().chain_id()?,
        commit.commit().source_id()?,
        commit.commit().cursor_epoch(),
        commit.commit().start_offset(),
        commit.commit().end_offset(),
        LocalRecordSequenceRange::try_new(
            LocalRecordSequence::try_new(commit.commit().first_local_sequence())?,
            LocalRecordSequence::try_new(commit.commit().last_local_sequence())?,
        )?,
        verified.spool_manifest_blake3(),
        verified.spool_segment_blake3(),
        verified.rolling_content_sha256(),
        verified.object().sha256(),
        manifest_hash,
        verified.schema_fingerprint(),
        durable_at,
    )
}

fn dataset_relative(chain: &ChainId, source: &SourceId) -> PathBuf {
    raw_policy::dataset_relative(chain, source, raw_policy::RawPolicy::MonotonicByteV3)
}

fn logical_manifest_relative(hash: [u8; 32]) -> PathBuf {
    PathBuf::from("_manifests")
        .join("raw-byte-v3")
        .join(format!("manifest-{}.json", hex::encode(hash)))
}

fn root_relative(dataset: &Path, hash: [u8; 32]) -> PathBuf {
    dataset
        .join("roots")
        .join(format!("root-{}.json", hex::encode(hash)))
}

fn journal_relative(dataset: &Path, generation: u64) -> PathBuf {
    dataset
        .join("journals")
        .join(format!("generation-{generation}.log"))
}
