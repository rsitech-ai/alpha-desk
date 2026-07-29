use super::*;

pub fn run_trade_e2e(config: &TradeRunConfig) -> Result<TradeEvidence, FixtureRunError> {
    validate_replay_counts(
        config.block_count,
        config.checkpoint_after,
        config.iterations,
        2,
    )?;
    let output_root = create_private_output_root(&config.output_root)?;
    let archive_root = output_root.join("archive");
    let checkpoint_root = output_root.join("checkpoints");
    let archive = LocalParquetArchive::open(
        &archive_root,
        ArchiveConfig::deterministic_fixture(
            "state-replay-trade-e2e-v1",
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
                .append_block(&trade_block(height, &chain, "1.0.0")?)?
                .manifest_id()
                .clone(),
        );
    }
    let verified = archive.verify_manifest(
        manifests
            .first()
            .ok_or(FixtureRunError::Invariant("trade manifest set is empty"))?,
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
        let mut ledger = empty_trade_ledger(chain.clone())?;
        let request = replay_request(
            &chain,
            START_HEIGHT,
            end_height,
            manifests.clone(),
            ledger.state_hash(),
            schema_fingerprint,
        )?;
        let ReplayOutcome::Completed(receipt) =
            SerialReplayEngine::new(&archive, &mut ledger, ReplayLimits::production())
                .run(&request, &NeverCancel)?
        else {
            return Err(FixtureRunError::Invariant(
                "uncancelled trade replay was cancelled",
            ));
        };
        match (expected_state_hash, expected_receipt_hash) {
            (Some(state_hash), Some(receipt_hash)) => {
                if ledger.state_hash() != state_hash || receipt.receipt_hash() != receipt_hash {
                    return Err(FixtureRunError::Invariant(
                        "independent trade replays diverged",
                    ));
                }
            }
            (None, None) => {
                expected_state_hash = Some(ledger.state_hash());
                expected_receipt_hash = Some(receipt.receipt_hash());
            }
            _ => {
                return Err(FixtureRunError::Invariant(
                    "trade replay expectation initialization is inconsistent",
                ));
            }
        }
    }
    let replay_elapsed_micros =
        u64::try_from(replay_started.elapsed().as_micros()).unwrap_or(u64::MAX);
    let expected_state_hash = expected_state_hash.ok_or(FixtureRunError::Invariant(
        "no trade replay expectation was produced",
    ))?;
    let expected_receipt_hash = expected_receipt_hash.ok_or(FixtureRunError::Invariant(
        "no trade replay receipt was produced",
    ))?;

    let checkpoint_end = START_HEIGHT
        .checked_add(config.checkpoint_after - 1)
        .ok_or(FixtureRunError::InvalidConfig)?;
    let checkpoint_len =
        usize::try_from(config.checkpoint_after).map_err(|_| FixtureRunError::InvalidConfig)?;
    let mut partial = empty_trade_ledger(chain.clone())?;
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
            "uncancelled trade checkpoint replay was cancelled",
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
        CanonicalTradeReducerV1,
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
            "uncancelled trade checkpoint resume was cancelled",
        ));
    };
    if resumed.state_hash() != expected_state_hash {
        return Err(FixtureRunError::Invariant(
            "trade checkpoint resume final state diverged",
        ));
    }
    let state_summary = summarize_trade_state(&resumed, end_height, config.block_count)?;

    let rejection_height = end_height
        .checked_add(1)
        .ok_or(FixtureRunError::InvalidConfig)?;
    let malformed = poison_block(rejection_height, &chain)?;
    let mut malformed_ledger = CanonicalLedger::try_from_state_image(
        resumed.state_image().clone(),
        CanonicalTradeReducerV1,
        LedgerLimits::production(),
    )?;
    let malformed_before = malformed_ledger.state_hash();
    let direct_error = match malformed_ledger.apply_block(&malformed) {
        Err(error) => error,
        Ok(_) => {
            return Err(FixtureRunError::Invariant(
                "trade reducer accepted a malformed trade before replay",
            ));
        }
    };
    if malformed_ledger.state_hash() != malformed_before {
        return Err(FixtureRunError::Invariant(
            "malformed trade changed state before archive replay",
        ));
    }
    let reducer_reason_code = direct_error
        .reducer_reason_code()
        .ok_or(FixtureRunError::Invariant(
            "malformed trade reducer reason is absent",
        ))?
        .to_owned();
    let malformed_manifest = archive.append_block(&malformed)?.manifest_id().clone();
    let malformed_request = replay_request(
        &chain,
        rejection_height,
        rejection_height,
        vec![malformed_manifest],
        malformed_before,
        schema_fingerprint,
    )?;
    let malformed_error =
        match SerialReplayEngine::new(&archive, &mut malformed_ledger, ReplayLimits::production())
            .run(&malformed_request, &NeverCancel)
        {
            Err(error) => error,
            Ok(_) => {
                return Err(FixtureRunError::Invariant(
                    "trade reducer accepted a malformed trade",
                ));
            }
        };
    let malformed_after = malformed_ledger.state_hash();
    validate_atomic_rejection(
        &malformed_error,
        "ledger.reducer_failed",
        malformed_before,
        malformed_after,
    )?;

    let unsupported_archive = LocalParquetArchive::open(
        output_root.join("unsupported-archive"),
        ArchiveConfig::deterministic_fixture(
            "state-replay-trade-unsupported-v1",
            KnownTime::from_unix_micros(FIXTURE_EPOCH_MICROS)?,
        )?,
    )?;
    let unsupported_manifest = unsupported_archive
        .append_block(&trade_block(rejection_height, &chain, "1.1.0")?)?
        .manifest_id()
        .clone();
    let mut unsupported_ledger = CanonicalLedger::try_from_state_image(
        resumed.state_image().clone(),
        CanonicalTradeReducerV1,
        LedgerLimits::production(),
    )?;
    let unsupported_before = unsupported_ledger.state_hash();
    let unsupported_request = replay_request(
        &chain,
        rejection_height,
        rejection_height,
        vec![unsupported_manifest],
        unsupported_before,
        schema_fingerprint,
    )?;
    let unsupported_error = match SerialReplayEngine::new(
        &unsupported_archive,
        &mut unsupported_ledger,
        ReplayLimits::production(),
    )
    .run(&unsupported_request, &NeverCancel)
    {
        Err(error) => error,
        Ok(_) => {
            return Err(FixtureRunError::Invariant(
                "trade reducer accepted an unsupported schema",
            ));
        }
    };
    let unsupported_after = unsupported_ledger.state_hash();
    validate_atomic_rejection(
        &unsupported_error,
        "ledger.unsupported_event",
        unsupported_before,
        unsupported_after,
    )?;

    let report = TradeReport {
        schema_version: TRADE_REPORT_SCHEMA,
        evidence_class: TRADE_EVIDENCE_CLASS,
        state_semantics: "canonical_trade_facts",
        source_qualification: "synthetic_unassessed",
        reducer_set_version: CanonicalTradeReducerV1::VERSION,
        stage_1_qualified: false,
        stage_2_qualified: false,
        live_source_qualified: false,
        account_state_qualified: false,
        order_state_qualified: false,
        position_state_qualified: false,
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
        trade_record_count: state_summary.trade_record_count,
        participant_record_count: state_summary.participant_record_count,
        reconciliation_record_count: state_summary.reconciliation_record_count,
        passed_reconciliation_count: state_summary.passed_reconciliation_count,
        sample_reconciliation: state_summary.sample_reconciliation,
        malformed_trade: rejection_report(
            rejection_height,
            &malformed_error,
            Some(reducer_reason_code),
            malformed_before,
            malformed_after,
        )?,
        unsupported_schema: rejection_report(
            rejection_height,
            &unsupported_error,
            None,
            unsupported_before,
            unsupported_after,
        )?,
    };
    let report_path = output_root.join(REPORT_FILE);
    publish_report(&report_path, &report)?;
    Ok(TradeEvidence { report_path })
}

fn empty_trade_ledger(
    chain: ChainId,
) -> Result<CanonicalLedger<CanonicalTradeReducerV1>, FixtureRunError> {
    Ok(CanonicalLedger::try_new(
        chain,
        BlockHeight::new(START_HEIGHT),
        CanonicalTradeReducerV1,
        LedgerLimits::production(),
    )?)
}

fn trade_block(
    height: u64,
    chain: &ChainId,
    schema_version: &str,
) -> Result<BlockEnvelope, FixtureRunError> {
    let time = fixture_time(height)?;
    let market = MarketId::new("perp:BTC")?;
    let payload = EventPayload::TradeMatched(TradeMatched {
        trade_id: Some(TradeId::new(format!("state-replay-trade-{height}"))?),
        market_id: Some(market.clone()),
        maker_order_id: None,
        taker_order_id: None,
        price: Price::parse_at_scale("65000", 6)?,
        quantity: Quantity::parse_at_scale("0.01", 8)?,
        deterministic_seed: height,
    });
    let payload_hash = *blake3::hash(&payload.encode_to_vec()?).as_bytes();
    let event = CanonicalEventEnvelope::from_input(CanonicalEventInput {
        schema_version: schema_version.to_owned(),
        chain_id: chain.clone(),
        block_height: BlockHeight::new(height),
        block_time: time,
        transaction_id: TransactionId::new(format!("state-replay-trade-tx-{height}"))?,
        transaction_index: 0,
        canonical_event_index: 0,
        market_ids: vec![market],
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
        parser_version: "state-replay-trade-fixture-v1".to_owned(),
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

fn summarize_trade_state(
    ledger: &CanonicalLedger<CanonicalTradeReducerV1>,
    end_height: u64,
    expected_trade_count: u64,
) -> Result<TradeStateSummary, FixtureRunError> {
    let mut trade_record_count = 0_u64;
    let mut participant_record_count = 0_u64;
    let mut reconciliation_record_count = 0_u64;
    let mut passed_reconciliation_count = 0_u64;
    for (key, bytes) in ledger.state_image().entries() {
        match key.namespace() {
            "trade.v1" => {
                TradeStateRecordV1::decode_at(key, bytes)?;
                trade_record_count = trade_record_count
                    .checked_add(1)
                    .ok_or(FixtureRunError::Invariant("trade record count overflow"))?;
            }
            "trade-participant.v1" => {
                TradeParticipantRecordV1::decode_at(key, bytes)?;
                participant_record_count =
                    participant_record_count
                        .checked_add(1)
                        .ok_or(FixtureRunError::Invariant(
                            "trade participant record count overflow",
                        ))?;
            }
            "reconciliation.v1" => {
                let reconciliation = TradeReconciliationRecordV1::decode_at(key, bytes)?;
                reconciliation_record_count = reconciliation_record_count.checked_add(1).ok_or(
                    FixtureRunError::Invariant("trade reconciliation record count overflow"),
                )?;
                if reconciliation.passed() {
                    passed_reconciliation_count =
                        passed_reconciliation_count.checked_add(1).ok_or(
                            FixtureRunError::Invariant("passed reconciliation count overflow"),
                        )?;
                }
            }
            _ => {
                return Err(FixtureRunError::Invariant(
                    "trade state contains an unexpected namespace",
                ));
            }
        }
    }
    let expected_participant_count = expected_trade_count
        .checked_mul(2)
        .ok_or(FixtureRunError::InvalidConfig)?;
    if trade_record_count != expected_trade_count
        || participant_record_count != expected_participant_count
        || reconciliation_record_count != expected_trade_count
        || passed_reconciliation_count != expected_trade_count
    {
        return Err(FixtureRunError::Invariant(
            "trade state record cardinality is inconsistent",
        ));
    }

    let sample_trade_id = TradeId::new(format!("state-replay-trade-{end_height}"))?;
    let sample_key = TradeReconciliationRecordV1::state_key(&sample_trade_id)?;
    let sample = TradeReconciliationRecordV1::decode_at(
        &sample_key,
        ledger
            .state_image()
            .entries()
            .get(&sample_key)
            .ok_or(FixtureRunError::Invariant(
                "final trade reconciliation sample is absent",
            ))?,
    )?;
    Ok(TradeStateSummary {
        trade_record_count,
        participant_record_count,
        reconciliation_record_count,
        passed_reconciliation_count,
        sample_reconciliation: ReconciliationSample {
            trade_id: sample.trade_id().as_str().to_owned(),
            status: "passed",
            quantity: sample.quantity().to_string(),
            participant_count: sample.participant_count(),
            block_height: sample.block_height().get(),
            evidence_blake3: hex::encode(sample.evidence_hash()),
        },
    })
}

#[derive(Debug, Serialize)]
struct TradeReport<'a> {
    schema_version: &'static str,
    evidence_class: &'static str,
    state_semantics: &'static str,
    source_qualification: &'static str,
    reducer_set_version: &'static str,
    stage_1_qualified: bool,
    stage_2_qualified: bool,
    live_source_qualified: bool,
    account_state_qualified: bool,
    order_state_qualified: bool,
    position_state_qualified: bool,
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
    trade_record_count: u64,
    participant_record_count: u64,
    reconciliation_record_count: u64,
    passed_reconciliation_count: u64,
    sample_reconciliation: ReconciliationSample,
    malformed_trade: RejectionReport,
    unsupported_schema: RejectionReport,
}

#[derive(Debug)]
struct TradeStateSummary {
    trade_record_count: u64,
    participant_record_count: u64,
    reconciliation_record_count: u64,
    passed_reconciliation_count: u64,
    sample_reconciliation: ReconciliationSample,
}

#[derive(Debug, Serialize)]
struct ReconciliationSample {
    trade_id: String,
    status: &'static str,
    quantity: String,
    participant_count: u8,
    block_height: u64,
    evidence_blake3: String,
}
