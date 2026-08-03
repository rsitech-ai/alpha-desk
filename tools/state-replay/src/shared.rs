use super::*;

pub(super) fn validate_replay_counts(
    block_count: u64,
    checkpoint_after: u64,
    iterations: u64,
    extra_full_passes: u64,
    rejection_blocks: u64,
) -> Result<(), FixtureRunError> {
    let passes = iterations
        .checked_add(extra_full_passes)
        .ok_or(FixtureRunError::InvalidConfig)?;
    let total_blocks = block_count
        .checked_mul(passes)
        .and_then(|count| count.checked_add(rejection_blocks))
        .ok_or(FixtureRunError::InvalidConfig)?;
    if !(2..=MAX_BLOCKS).contains(&block_count)
        || checkpoint_after == 0
        || checkpoint_after >= block_count
        || iterations == 0
        || iterations > MAX_ITERATIONS
        || total_blocks > MAX_TOTAL_REPLAY_BLOCKS
    {
        return Err(FixtureRunError::InvalidConfig);
    }
    Ok(())
}

pub(super) fn create_private_output_root(path: &Path) -> Result<PathBuf, FixtureRunError> {
    let output = resolve_output_path(path)?;
    if fs::symlink_metadata(&output).is_ok() {
        return Err(FixtureRunError::OutputExists);
    }
    let canonical_parent = output.parent().ok_or(FixtureRunError::UnsafeOutput)?;
    let mut builder = fs::DirBuilder::new();
    builder.mode(0o700);
    builder
        .create(&output)
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::AlreadyExists => FixtureRunError::OutputExists,
            _ => FixtureRunError::Io("creating evidence root"),
        })?;
    fs::set_permissions(&output, fs::Permissions::from_mode(0o700))
        .map_err(|_| FixtureRunError::Io("setting evidence root permissions"))?;
    OpenOptions::new()
        .read(true)
        .open(canonical_parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| FixtureRunError::Io("syncing evidence root parent"))?;
    Ok(output)
}

pub(super) fn resolve_output_path(path: &Path) -> Result<PathBuf, FixtureRunError> {
    if path.as_os_str().is_empty()
        || path.as_os_str().len() > MAX_OUTPUT_PATH_BYTES
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(FixtureRunError::UnsafeOutput);
    }
    let name = path
        .file_name()
        .filter(|name| !name.is_empty())
        .ok_or(FixtureRunError::UnsafeOutput)?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let parent_metadata =
        fs::symlink_metadata(parent).map_err(|_| FixtureRunError::UnsafeOutput)?;
    if parent_metadata.file_type().is_symlink() || !parent_metadata.is_dir() {
        return Err(FixtureRunError::UnsafeOutput);
    }
    if parent_metadata.permissions().mode() & 0o022 != 0 {
        return Err(FixtureRunError::UnsafeOutput);
    }
    let canonical_parent = parent
        .canonicalize()
        .map_err(|_| FixtureRunError::UnsafeOutput)?;
    Ok(canonical_parent.join(name))
}

pub(super) fn harden_private_tree(root: &Path) -> Result<(), FixtureRunError> {
    let metadata = fs::symlink_metadata(root)
        .map_err(|_| FixtureRunError::Io("reading evidence permissions"))?;
    if metadata.file_type().is_symlink() {
        return Err(FixtureRunError::UnsafeOutput);
    }
    fs::set_permissions(
        root,
        fs::Permissions::from_mode(if metadata.is_dir() { 0o700 } else { 0o600 }),
    )
    .map_err(|_| FixtureRunError::Io("setting evidence permissions"))?;
    if metadata.is_dir() {
        for child in
            fs::read_dir(root).map_err(|_| FixtureRunError::Io("reading evidence directory"))?
        {
            harden_private_tree(
                &child
                    .map_err(|_| FixtureRunError::Io("reading evidence entry"))?
                    .path(),
            )?;
        }
    }
    Ok(())
}

pub(super) fn replay_request(
    chain: &ChainId,
    start: u64,
    end: u64,
    manifests: Vec<domain_types::ManifestId>,
    expected_start_state_hash: [u8; 32],
    schema_fingerprint: [u8; 32],
) -> Result<ReplayRequest, FixtureRunError> {
    ReplayRequest::try_new(
        chain.clone(),
        BlockRange::new(BlockHeight::new(start), BlockHeight::new(end))?,
        manifests,
        expected_start_state_hash,
        "canonical_events",
        schema_fingerprint,
    )
    .map_err(|_| FixtureRunError::InvalidConfig)
}

pub(super) fn fixture_time(height: u64) -> Result<ProtocolTime, FixtureRunError> {
    let offset = i64::try_from(height).map_err(|_| FixtureRunError::InvalidConfig)?;
    let micros = FIXTURE_EPOCH_MICROS
        .checked_add(offset)
        .ok_or(FixtureRunError::InvalidConfig)?;
    Ok(ProtocolTime::from_unix_micros(micros)?)
}

pub(super) fn source_hashes(height: u64) -> Result<BTreeMap<SourceId, [u8; 32]>, FixtureRunError> {
    Ok(BTreeMap::from([(
        SourceId::new("state-replay-fixture")?,
        *blake3::hash(&height.to_be_bytes()).as_bytes(),
    )]))
}

pub(super) fn validate_atomic_rejection(
    error: &replay_engine::ReplayError,
    expected_source_reason: &str,
    state_hash_before: [u8; 32],
    state_hash_after: [u8; 32],
) -> Result<(), FixtureRunError> {
    if error.reason_code() != "replay.block_quarantined"
        || error.source_reason_code() != Some(expected_source_reason)
        || error.progress().applied_block_count() != 0
        || state_hash_after != state_hash_before
    {
        return Err(FixtureRunError::Invariant(
            "rejected canonical block did not preserve the pre-block state",
        ));
    }
    Ok(())
}

pub(super) fn rejection_report(
    height: u64,
    error: &replay_engine::ReplayError,
    reducer_reason_code: Option<String>,
    state_hash_before: [u8; 32],
    state_hash_after: [u8; 32],
) -> Result<RejectionReport, FixtureRunError> {
    Ok(RejectionReport {
        height,
        reason_code: error.reason_code(),
        source_reason_code: error
            .source_reason_code()
            .ok_or(FixtureRunError::Invariant(
                "rejection source reason is absent",
            ))?,
        reducer_reason_code,
        applied_block_count: error.progress().applied_block_count(),
        state_hash_before: hex::encode(state_hash_before),
        state_hash_after: hex::encode(state_hash_after),
    })
}

pub(super) fn publish_report(path: &Path, report: &impl Serialize) -> Result<(), FixtureRunError> {
    let mut bytes = serde_json::to_vec_pretty(report)?;
    bytes.push(b'\n');
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::AlreadyExists => FixtureRunError::OutputExists,
            _ => FixtureRunError::Io("creating evidence report"),
        })?;
    file.write_all(&bytes)
        .map_err(|_| FixtureRunError::Io("writing evidence report"))?;
    file.sync_all()
        .map_err(|_| FixtureRunError::Io("syncing evidence report"))?;
    let directory = OpenOptions::new()
        .read(true)
        .open(path.parent().ok_or(FixtureRunError::UnsafeOutput)?)
        .map_err(|_| FixtureRunError::Io("opening evidence root"))?;
    directory
        .sync_all()
        .map_err(|_| FixtureRunError::Io("syncing evidence root"))
}

#[derive(Debug, Serialize)]

pub(super) struct RejectionReport {
    height: u64,
    reason_code: &'static str,
    source_reason_code: &'static str,
    reducer_reason_code: Option<String>,
    applied_block_count: u64,
    state_hash_before: String,
    state_hash_after: String,
}

#[derive(Debug, Serialize)]
pub(super) struct NeverCancel;

impl ReplayCancellation for NeverCancel {
    fn is_cancelled(&self) -> bool {
        false
    }
}

pub(super) fn open_deterministic_archive(
    path: &Path,
    archive_id: &str,
) -> Result<LocalParquetArchive, FixtureRunError> {
    Ok(LocalParquetArchive::open(
        path,
        ArchiveConfig::deterministic_fixture(
            archive_id,
            KnownTime::from_unix_micros(FIXTURE_EPOCH_MICROS)?,
        )?,
    )?)
}

pub(super) fn append_fixture_blocks(
    archive: &LocalParquetArchive,
    blocks: &[BlockEnvelope],
) -> Result<Vec<domain_types::ManifestId>, FixtureRunError> {
    blocks
        .iter()
        .map(|block| Ok(archive.append_block(block)?.manifest_id().clone()))
        .collect()
}

pub(super) fn canonical_events_schema_fingerprint(
    archive: &LocalParquetArchive,
    manifests: &[domain_types::ManifestId],
) -> Result<[u8; 32], FixtureRunError> {
    Ok(*archive
        .verify_manifest(
            manifests
                .first()
                .ok_or(FixtureRunError::Invariant("missing fixture manifest"))?,
        )?
        .schema_fingerprints()
        .get("canonical_events")
        .ok_or(FixtureRunError::Invariant(
            "missing canonical schema fingerprint",
        ))?)
}

pub(super) fn empty_ledger_at(
    chain: ChainId,
    first_height: BlockHeight,
) -> Result<CanonicalLedger<WatermarkOnlyReducerV1>, FixtureRunError> {
    Ok(CanonicalLedger::try_new(
        chain,
        first_height,
        WatermarkOnlyReducerV1,
        LedgerLimits::production(),
    )?)
}

pub(super) fn poison_block(height: u64, chain: &ChainId) -> Result<BlockEnvelope, FixtureRunError> {
    let time = fixture_time(height)?;
    let payload = EventPayload::TradeMatched(TradeMatched::without_identities(
        Price::parse_at_scale("65000", 6)?,
        Quantity::parse_at_scale("0.01", 8)?,
        1,
    ));
    let payload_hash = *blake3::hash(&payload.encode_to_vec()?).as_bytes();
    let event = CanonicalEventEnvelope::from_input(CanonicalEventInput {
        schema_version: "1.0.0".to_owned(),
        chain_id: chain.clone(),
        block_height: BlockHeight::new(height),
        block_time: time,
        transaction_id: TransactionId::new(format!("state-replay-poison-{height}"))?,
        transaction_index: 0,
        canonical_event_index: 0,
        market_ids: vec![MarketId::new("perp:BTC")?],
        account_ids: vec![
            Address::from_bytes([0x11; 20]),
            Address::from_bytes([0x22; 20]),
        ],
        source_evidence: vec![SourceEvidence::try_new(
            SourceId::new("state-replay-fixture")?,
            "v1",
            height.to_string(),
            payload_hash,
        )?],
        confirmation_class: ConfirmationClass::CommittedPrimary,
        observed_at: KnownTime::from_unix_micros(time.unix_micros())?,
        ingested_at: KnownTime::from_unix_micros(time.unix_micros())?,
        canonicalized_at: KnownTime::from_unix_micros(time.unix_micros())?,
        parser_version: "state-replay-fixture-v1".to_owned(),
        payload,
    })?;
    Ok(BlockEnvelope::try_new(
        chain.clone(),
        BlockHeight::new(height),
        time,
        ConfirmationClass::CommittedPrimary,
        vec![event],
        source_hashes(height)?,
    )?)
}
