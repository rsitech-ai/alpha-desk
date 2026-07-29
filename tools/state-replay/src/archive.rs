use super::*;

pub fn run_archive_e2e(config: &ArchiveRunConfig) -> Result<ArchiveEvidence, FixtureRunError> {
    let (chain, range, block_count) = validate_archive_run_config(config)?;
    let archive_root = validate_archive_root(&config.archive_root)?;
    let output_candidate = resolve_output_path(&config.output_root)?;
    if output_candidate.starts_with(&archive_root) || archive_root.starts_with(&output_candidate) {
        return Err(FixtureRunError::UnsafeOutput);
    }
    let archive = LocalParquetArchive::open(
        &archive_root,
        ArchiveConfig::production("state-replay-archive-e2e-v1")?
            .with_read_limits(block_count, 2 * 1_024 * 1_024 * 1_024)?,
    )?;
    let verified_manifests = archive.plan_range(&chain, range)?;
    let (checkpoint_index, schema_fingerprint) =
        validate_archive_plan(config, &verified_manifests)?;
    let manifests = verified_manifests
        .iter()
        .map(|manifest| manifest.manifest_id().clone())
        .collect::<Vec<_>>();

    let output_root = create_private_output_root(&config.output_root)?;
    let checkpoint_root = output_root.join("checkpoints");
    let replay_started = Instant::now();
    let mut expected_state_hash = None;
    let mut expected_receipt_hash = None;
    for _ in 0..config.iterations {
        let mut ledger = empty_ledger_at(chain.clone(), BlockHeight::new(config.start_height))?;
        let request = replay_request(
            &chain,
            config.start_height,
            config.end_height,
            manifests.clone(),
            ledger.state_hash(),
            schema_fingerprint,
        )?;
        let ReplayOutcome::Completed(receipt) =
            SerialReplayEngine::new(&archive, &mut ledger, ReplayLimits::production())
                .run(&request, &NeverCancel)?
        else {
            return Err(FixtureRunError::Invariant(
                "uncancelled operator archive replay was cancelled",
            ));
        };
        match (expected_state_hash, expected_receipt_hash) {
            (Some(state_hash), Some(receipt_hash)) => {
                if ledger.state_hash() != state_hash || receipt.receipt_hash() != receipt_hash {
                    return Err(FixtureRunError::Invariant(
                        "independent operator archive replays diverged",
                    ));
                }
            }
            (None, None) => {
                expected_state_hash = Some(ledger.state_hash());
                expected_receipt_hash = Some(receipt.receipt_hash());
            }
            _ => {
                return Err(FixtureRunError::Invariant(
                    "operator replay expectation initialization is inconsistent",
                ));
            }
        }
    }
    let replay_elapsed_micros =
        u64::try_from(replay_started.elapsed().as_micros()).unwrap_or(u64::MAX);
    let expected_state_hash = expected_state_hash.ok_or(FixtureRunError::Invariant(
        "no operator archive replay expectation was produced",
    ))?;
    let expected_receipt_hash = expected_receipt_hash.ok_or(FixtureRunError::Invariant(
        "no operator archive replay receipt was produced",
    ))?;

    let prefix = &verified_manifests[..=checkpoint_index];
    let prefix_ids = prefix
        .iter()
        .map(|manifest| manifest.manifest_id().clone())
        .collect::<Vec<_>>();
    let mut partial = empty_ledger_at(chain.clone(), BlockHeight::new(config.start_height))?;
    let partial_request = replay_request(
        &chain,
        config.start_height,
        config.checkpoint_height,
        prefix_ids,
        partial.state_hash(),
        schema_fingerprint,
    )?;
    let ReplayOutcome::Completed(_) =
        SerialReplayEngine::new(&archive, &mut partial, ReplayLimits::production())
            .run(&partial_request, &NeverCancel)?
    else {
        return Err(FixtureRunError::Invariant(
            "uncancelled operator checkpoint replay was cancelled",
        ));
    };
    let checkpoint_manifest = &verified_manifests[checkpoint_index];
    let artifact = CheckpointArtifact::try_new(
        partial
            .checkpoint()
            .ok_or(FixtureRunError::Invariant("checkpoint watermark is absent"))?,
        partial.state_image().clone(),
        checkpoint_manifest.manifest_id().clone(),
        checkpoint_manifest.manifest_sha256(),
        schema_fingerprint,
    )?;
    let checkpoint_store =
        LocalCheckpointStore::open(&checkpoint_root, StateImageLimits::production())?;
    let published = checkpoint_store.publish(&artifact)?;
    let compatibility = CheckpointCompatibility::try_new(
        chain.clone(),
        artifact.checkpoint().reducer_set_version(),
        artifact.archive_manifest_id().clone(),
        artifact.archive_manifest_sha256(),
        artifact.schema_fingerprint(),
    )?;
    let loaded = checkpoint_store.load(
        published.receipt().checkpoint_id(),
        &compatibility,
        StateImageLimits::production(),
    )?;
    let mut resumed = CanonicalLedger::try_from_state_image(
        loaded.state_image().clone(),
        WatermarkOnlyReducerV1,
        LedgerLimits::production(),
    )?;
    let resume_start = config
        .checkpoint_height
        .checked_add(1)
        .ok_or(FixtureRunError::InvalidConfig)?;
    let suffix_ids = verified_manifests[checkpoint_index + 1..]
        .iter()
        .map(|manifest| manifest.manifest_id().clone())
        .collect::<Vec<_>>();
    let resume_request = replay_request(
        &chain,
        resume_start,
        config.end_height,
        suffix_ids,
        resumed.state_hash(),
        schema_fingerprint,
    )?;
    let ReplayOutcome::Completed(resume_receipt) =
        SerialReplayEngine::new(&archive, &mut resumed, ReplayLimits::production())
            .run(&resume_request, &NeverCancel)?
    else {
        return Err(FixtureRunError::Invariant(
            "uncancelled operator archive resume was cancelled",
        ));
    };
    if resumed.state_hash() != expected_state_hash {
        return Err(FixtureRunError::Invariant(
            "operator checkpoint resume final state diverged",
        ));
    }

    let report = ArchiveReport {
        schema_version: ARCHIVE_REPORT_SCHEMA,
        evidence_class: ARCHIVE_EVIDENCE_CLASS,
        state_semantics: "watermark_only",
        source_qualification: "unassessed",
        stage_2_qualified: false,
        live_source_qualified: false,
        chain_id: chain.as_str(),
        start_height: config.start_height,
        end_height: config.end_height,
        block_count,
        checkpoint_height: config.checkpoint_height,
        iterations_completed: config.iterations,
        expected_final_state_hash: hex::encode(expected_state_hash),
        deterministic_replay_receipt_hash: hex::encode(expected_receipt_hash),
        checkpoint_id: artifact.checkpoint_id().as_str(),
        resumed_final_state_hash: hex::encode(resumed.state_hash()),
        resume_receipt_hash: hex::encode(resume_receipt.receipt_hash()),
        replay_elapsed_micros,
        manifests: verified_manifests
            .iter()
            .map(ManifestReport::from)
            .collect(),
    };
    let report_path = output_root.join(REPORT_FILE);
    publish_report(&report_path, &report)?;
    Ok(ArchiveEvidence { report_path })
}

fn validate_archive_run_config(
    config: &ArchiveRunConfig,
) -> Result<(ChainId, BlockRange, u64), FixtureRunError> {
    if config.iterations == 0
        || config.iterations > MAX_ITERATIONS
        || config.start_height >= config.end_height
        || config.checkpoint_height < config.start_height
        || config.checkpoint_height >= config.end_height
    {
        return Err(FixtureRunError::InvalidConfig);
    }
    let block_count = config
        .end_height
        .checked_sub(config.start_height)
        .and_then(|span| span.checked_add(1))
        .ok_or(FixtureRunError::InvalidConfig)?;
    let passes = config
        .iterations
        .checked_add(1)
        .ok_or(FixtureRunError::InvalidConfig)?;
    let total_blocks = block_count
        .checked_mul(passes)
        .ok_or(FixtureRunError::InvalidConfig)?;
    if block_count > MAX_BLOCKS || total_blocks > MAX_TOTAL_REPLAY_BLOCKS {
        return Err(FixtureRunError::InvalidConfig);
    }
    let chain = ChainId::new(config.chain_id.clone())?;
    let range = BlockRange::new(
        BlockHeight::new(config.start_height),
        BlockHeight::new(config.end_height),
    )?;
    Ok((chain, range, block_count))
}

fn validate_archive_plan(
    config: &ArchiveRunConfig,
    manifests: &[VerifiedManifest],
) -> Result<(usize, [u8; 32]), FixtureRunError> {
    let first = manifests
        .first()
        .ok_or(FixtureRunError::Invariant("archive plan is empty"))?;
    let last = manifests
        .last()
        .ok_or(FixtureRunError::Invariant("archive plan is empty"))?;
    let schema_fingerprint =
        *first
            .schema_fingerprints()
            .get("canonical_events")
            .ok_or(FixtureRunError::Invariant(
                "canonical schema fingerprint is absent",
            ))?;
    let mut expected_start = config.start_height;
    let mut checkpoint_index = None;
    for (index, manifest) in manifests.iter().enumerate() {
        let range = manifest.block_range();
        if manifest.chain_id().as_str() != config.chain_id
            || range.start_inclusive.get() != expected_start
            || manifest.schema_fingerprints().get("canonical_events") != Some(&schema_fingerprint)
            || range.end_inclusive.get() > config.end_height
        {
            return Err(FixtureRunError::InvalidConfig);
        }
        if range.end_inclusive.get() == config.checkpoint_height {
            checkpoint_index = Some(index);
        }
        expected_start = range
            .end_inclusive
            .get()
            .checked_add(1)
            .ok_or(FixtureRunError::InvalidConfig)?;
    }
    if first.block_range().start_inclusive.get() != config.start_height
        || last.block_range().end_inclusive.get() != config.end_height
    {
        return Err(FixtureRunError::InvalidConfig);
    }
    let checkpoint_index = checkpoint_index.ok_or(FixtureRunError::InvalidConfig)?;
    Ok((checkpoint_index, schema_fingerprint))
}

fn validate_archive_root(path: &Path) -> Result<PathBuf, FixtureRunError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| FixtureRunError::InvalidArchive)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(FixtureRunError::InvalidArchive);
    }
    path.canonicalize()
        .map_err(|_| FixtureRunError::InvalidArchive)
}

#[derive(Debug, Serialize)]
struct ArchiveReport<'a> {
    schema_version: &'static str,
    evidence_class: &'static str,
    state_semantics: &'static str,
    source_qualification: &'static str,
    stage_2_qualified: bool,
    live_source_qualified: bool,
    chain_id: &'a str,
    start_height: u64,
    end_height: u64,
    block_count: u64,
    checkpoint_height: u64,
    iterations_completed: u64,
    expected_final_state_hash: String,
    deterministic_replay_receipt_hash: String,
    checkpoint_id: &'a str,
    resumed_final_state_hash: String,
    resume_receipt_hash: String,
    replay_elapsed_micros: u64,
    manifests: Vec<ManifestReport>,
}

#[derive(Debug, Serialize)]
struct ManifestReport {
    manifest_id: String,
    manifest_sha256: String,
    start_height: u64,
    end_height: u64,
    row_count: u64,
}

impl From<&VerifiedManifest> for ManifestReport {
    fn from(manifest: &VerifiedManifest) -> Self {
        Self {
            manifest_id: manifest.manifest_id().as_str().to_owned(),
            manifest_sha256: hex::encode(manifest.manifest_sha256()),
            start_height: manifest.block_range().start_inclusive.get(),
            end_height: manifest.block_range().end_inclusive.get(),
            row_count: manifest.row_count(),
        }
    }
}
