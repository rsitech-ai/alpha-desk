use super::*;

pub fn run_fixture_e2e(config: &FixtureRunConfig) -> Result<FixtureEvidence, FixtureRunError> {
    validate_config(config)?;
    let output_root = create_private_output_root(&config.output_root)?;
    let archive_root = output_root.join("archive");
    let checkpoint_root = output_root.join("checkpoints");
    let archive = LocalParquetArchive::open(
        &archive_root,
        ArchiveConfig::deterministic_fixture(
            "state-replay-e2e-v1",
            KnownTime::from_unix_micros(FIXTURE_EPOCH_MICROS)?,
        )?,
    )?;
    let chain = ChainId::new(CHAIN)?;
    let end_height = START_HEIGHT
        .checked_add(config.block_count - 1)
        .ok_or(FixtureRunError::InvalidConfig)?;
    let mut manifests = Vec::with_capacity(
        usize::try_from(config.block_count).map_err(|_| FixtureRunError::InvalidConfig)?,
    );
    for height in START_HEIGHT..=end_height {
        manifests.push(
            archive
                .append_block(&empty_block(height, &chain)?)?
                .manifest_id()
                .clone(),
        );
    }
    let verified = archive.verify_manifest(
        manifests
            .first()
            .ok_or(FixtureRunError::Invariant("fixture manifest set is empty"))?,
    )?;
    let schema_fingerprint = *verified
        .schema_fingerprints()
        .get("canonical_events")
        .ok_or(FixtureRunError::Invariant(
            "canonical schema fingerprint is absent",
        ))?;

    let replay_started = Instant::now();
    let mut expected_state_hash = None;
    let mut expected_receipt_hash = None;
    for _ in 0..config.iterations {
        let mut ledger = empty_ledger(chain.clone())?;
        let request = replay_request(
            &chain,
            START_HEIGHT,
            end_height,
            manifests.clone(),
            ledger.state_hash(),
            schema_fingerprint,
        )?;
        let outcome = SerialReplayEngine::new(&archive, &mut ledger, ReplayLimits::production())
            .run(&request, &NeverCancel)?;
        let ReplayOutcome::Completed(receipt) = outcome else {
            return Err(FixtureRunError::Invariant(
                "uncancelled fixture replay was cancelled",
            ));
        };
        match (expected_state_hash, expected_receipt_hash) {
            (Some(state_hash), Some(receipt_hash)) => {
                if ledger.state_hash() != state_hash || receipt.receipt_hash() != receipt_hash {
                    return Err(FixtureRunError::Invariant(
                        "independent fixture replays diverged",
                    ));
                }
            }
            (None, None) => {
                expected_state_hash = Some(ledger.state_hash());
                expected_receipt_hash = Some(receipt.receipt_hash());
            }
            _ => {
                return Err(FixtureRunError::Invariant(
                    "replay expectation initialization is inconsistent",
                ));
            }
        }
    }
    let replay_elapsed_micros =
        u64::try_from(replay_started.elapsed().as_micros()).unwrap_or(u64::MAX);
    let expected_state_hash = expected_state_hash.ok_or(FixtureRunError::Invariant(
        "no full replay expectation was produced",
    ))?;
    let expected_receipt_hash = expected_receipt_hash.ok_or(FixtureRunError::Invariant(
        "no full replay receipt was produced",
    ))?;

    let checkpoint_count = config.checkpoint_after;
    let checkpoint_end = START_HEIGHT
        .checked_add(checkpoint_count - 1)
        .ok_or(FixtureRunError::InvalidConfig)?;
    let checkpoint_len =
        usize::try_from(checkpoint_count).map_err(|_| FixtureRunError::InvalidConfig)?;
    let mut partial = empty_ledger(chain.clone())?;
    let partial_request = replay_request(
        &chain,
        START_HEIGHT,
        checkpoint_end,
        manifests[..checkpoint_len].to_vec(),
        partial.state_hash(),
        schema_fingerprint,
    )?;
    let ReplayOutcome::Completed(_) =
        SerialReplayEngine::new(&archive, &mut partial, ReplayLimits::production())
            .run(&partial_request, &NeverCancel)?
    else {
        return Err(FixtureRunError::Invariant(
            "uncancelled checkpoint replay was cancelled",
        ));
    };
    let checkpoint_manifest = archive.verify_manifest(&manifests[checkpoint_len - 1])?;
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
    let resume_start = checkpoint_end
        .checked_add(1)
        .ok_or(FixtureRunError::InvalidConfig)?;
    let resume_request = replay_request(
        &chain,
        resume_start,
        end_height,
        manifests[checkpoint_len..].to_vec(),
        resumed.state_hash(),
        schema_fingerprint,
    )?;
    let ReplayOutcome::Completed(resume_receipt) =
        SerialReplayEngine::new(&archive, &mut resumed, ReplayLimits::production())
            .run(&resume_request, &NeverCancel)?
    else {
        return Err(FixtureRunError::Invariant(
            "uncancelled checkpoint resume was cancelled",
        ));
    };
    if resumed.state_hash() != expected_state_hash {
        return Err(FixtureRunError::Invariant(
            "checkpoint resume final state diverged",
        ));
    }

    let poison_height = end_height
        .checked_add(1)
        .ok_or(FixtureRunError::InvalidConfig)?;
    let poison_manifest = archive
        .append_block(&poison_block(poison_height, &chain)?)?
        .manifest_id()
        .clone();
    let poison_before = resumed.state_hash();
    let poison_request = replay_request(
        &chain,
        poison_height,
        poison_height,
        vec![poison_manifest],
        poison_before,
        schema_fingerprint,
    )?;
    let poison_error =
        match SerialReplayEngine::new(&archive, &mut resumed, ReplayLimits::production())
            .run(&poison_request, &NeverCancel)
        {
            Err(error) => error,
            Ok(_) => {
                return Err(FixtureRunError::Invariant(
                    "watermark-only reducer accepted a poison trade",
                ));
            }
        };
    let poison_after = resumed.state_hash();
    if poison_error.reason_code() != "replay.block_quarantined"
        || poison_error.source_reason_code() != Some("ledger.unsupported_event")
        || poison_error.progress().applied_block_count() != 0
        || poison_after != poison_before
    {
        return Err(FixtureRunError::Invariant(
            "poison block did not preserve the pre-block state",
        ));
    }

    let report = FixtureReport {
        schema_version: REPORT_SCHEMA,
        evidence_class: EVIDENCE_CLASS,
        stage_2_qualified: false,
        live_source_qualified: false,
        chain_id: CHAIN,
        start_height: START_HEIGHT,
        end_height,
        block_count: config.block_count,
        checkpoint_after: config.checkpoint_after,
        iterations_completed: config.iterations,
        expected_final_state_hash: hex::encode(expected_state_hash),
        deterministic_replay_receipt_hash: hex::encode(expected_receipt_hash),
        checkpoint_id: artifact.checkpoint_id().as_str(),
        resumed_final_state_hash: hex::encode(resumed.state_hash()),
        resume_receipt_hash: hex::encode(resume_receipt.receipt_hash()),
        replay_elapsed_micros,
        poison: PoisonReport {
            height: poison_height,
            reason_code: poison_error.reason_code(),
            source_reason_code: poison_error
                .source_reason_code()
                .ok_or(FixtureRunError::Invariant("poison source reason is absent"))?,
            applied_block_count: poison_error.progress().applied_block_count(),
            state_hash_before: hex::encode(poison_before),
            state_hash_after: hex::encode(poison_after),
        },
    };
    let report_path = output_root.join(REPORT_FILE);
    publish_report(&report_path, &report)?;
    Ok(FixtureEvidence { report_path })
}

fn validate_config(config: &FixtureRunConfig) -> Result<(), FixtureRunError> {
    validate_replay_counts(
        config.block_count,
        config.checkpoint_after,
        config.iterations,
        1,
    )
}

fn empty_ledger(
    chain: ChainId,
) -> Result<CanonicalLedger<WatermarkOnlyReducerV1>, FixtureRunError> {
    empty_ledger_at(chain, BlockHeight::new(START_HEIGHT))
}

fn empty_block(height: u64, chain: &ChainId) -> Result<BlockEnvelope, FixtureRunError> {
    Ok(BlockEnvelope::try_new(
        chain.clone(),
        BlockHeight::new(height),
        fixture_time(height)?,
        ConfirmationClass::CommittedPrimary,
        Vec::new(),
        source_hashes(height)?,
    )?)
}

#[derive(Debug, Serialize)]
struct FixtureReport<'a> {
    schema_version: &'static str,
    evidence_class: &'static str,
    stage_2_qualified: bool,
    live_source_qualified: bool,
    chain_id: &'static str,
    start_height: u64,
    end_height: u64,
    block_count: u64,
    checkpoint_after: u64,
    iterations_completed: u64,
    expected_final_state_hash: String,
    deterministic_replay_receipt_hash: String,
    checkpoint_id: &'a str,
    resumed_final_state_hash: String,
    resume_receipt_hash: String,
    replay_elapsed_micros: u64,
    poison: PoisonReport,
}

#[derive(Debug, Serialize)]

struct PoisonReport {
    height: u64,
    reason_code: &'static str,
    source_reason_code: &'static str,
    applied_block_count: u64,
    state_hash_before: String,
    state_hash_after: String,
}
