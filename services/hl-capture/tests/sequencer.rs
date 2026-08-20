use std::collections::BTreeMap;

use canonical_events::{
    BlockEnvelope, CanonicalEventEnvelope, CanonicalEventInput, ConfirmationClass, EventPayload,
    SourceEvidence, TradeMatched,
};
use domain_types::{
    BlockHeight, ChainId, KnownTime, Price, ProtocolTime, Quantity, SourceId, TransactionId,
};
use hl_capture::{
    BlockCandidate, CandidateError, CanonicalSequencer, QuarantineReason, SequencerConfig,
    SequencerDecision, SequencerError, SequencerHealth,
};
use hl_protocol::{ObservationClass, PublicationLane, SourceAdmission, SourceTrust};

fn known(micros: i64) -> KnownTime {
    KnownTime::from_unix_micros(micros).expect("known test time")
}

fn confirmation_for(trust: SourceTrust) -> ConfirmationClass {
    match trust {
        SourceTrust::LocallyVerifiedCommitted => ConfirmationClass::CommittedPrimary,
        SourceTrust::IndependentCommitted => ConfirmationClass::CommittedIndependent,
        SourceTrust::ThirdPartyProvisional => ConfirmationClass::ProvisionalSource,
        SourceTrust::ReconciledSnapshot
        | SourceTrust::RecoveryOnly
        | SourceTrust::MempoolProvisional => {
            panic!("unsupported fixture trust {trust:?}")
        }
    }
}

fn admission_for(trust: SourceTrust) -> SourceAdmission {
    let class = match trust {
        SourceTrust::LocallyVerifiedCommitted | SourceTrust::IndependentCommitted => {
            ObservationClass::CommittedBlock
        }
        SourceTrust::ThirdPartyProvisional => ObservationClass::ProvisionalFeed,
        SourceTrust::ReconciledSnapshot
        | SourceTrust::RecoveryOnly
        | SourceTrust::MempoolProvisional => {
            panic!("unsupported fixture trust {trust:?}")
        }
    };
    SourceAdmission::new(trust, class).expect("valid source admission")
}

fn classified_block(
    height: u64,
    source_id: &str,
    confirmation: ConfirmationClass,
    payload_seed: u64,
) -> (SourceId, BlockEnvelope) {
    let source_id = SourceId::new(source_id).expect("source ID");
    let block_time_micros =
        i64::try_from(height % 1_000_000).expect("bounded fixture height") * 1_000;
    let event = CanonicalEventEnvelope::from_input(CanonicalEventInput {
        schema_version: "1.0.0".to_owned(),
        chain_id: ChainId::new("mainnet").expect("chain"),
        block_height: BlockHeight::new(height),
        block_time: ProtocolTime::from_unix_micros(block_time_micros).expect("block time"),
        transaction_id: TransactionId::new(format!("tx-{height}")).expect("transaction"),
        transaction_index: 0,
        canonical_event_index: 0,
        market_ids: Vec::new(),
        account_ids: Vec::new(),
        source_evidence: vec![
            SourceEvidence::try_new(
                source_id.clone(),
                "node-v1",
                format!("block:{height}"),
                [u8::try_from(payload_seed).expect("small fixture seed"); 32],
            )
            .expect("source evidence"),
        ],
        confirmation_class: confirmation,
        observed_at: known(2_000),
        ingested_at: known(3_000),
        canonicalized_at: known(4_000),
        parser_version: "canonical-parser-v1".to_owned(),
        payload: EventPayload::TradeMatched(TradeMatched::without_identities(
            Price::parse_at_scale("65000", 6).expect("price"),
            Quantity::parse_at_scale("0.01", 8).expect("quantity"),
            payload_seed,
        )),
    })
    .expect("canonical event");
    let block = BlockEnvelope::try_new(
        ChainId::new("mainnet").expect("chain"),
        BlockHeight::new(height),
        ProtocolTime::from_unix_micros(block_time_micros).expect("block time"),
        confirmation,
        vec![event],
        BTreeMap::from([(source_id.clone(), [0x55; 32])]),
    )
    .expect("canonical block");
    (source_id, block)
}

fn candidate(
    height: u64,
    source_id: &str,
    trust: SourceTrust,
    payload_seed: u64,
) -> BlockCandidate {
    let confirmation = confirmation_for(trust);
    let (source_id, block) = classified_block(height, source_id, confirmation, payload_seed);
    BlockCandidate::try_new(source_id, admission_for(trust), block).expect("valid candidate")
}

fn sequencer(max_pending_blocks: usize, retained_committed_blocks: usize) -> CanonicalSequencer {
    CanonicalSequencer::new(
        SequencerConfig::try_new(
            ChainId::new("mainnet").expect("chain"),
            BlockHeight::new(100),
            max_pending_blocks,
            retained_committed_blocks,
        )
        .expect("sequencer config"),
    )
}

fn with_source_block_hash(candidate: BlockCandidate, byte: u8) -> BlockCandidate {
    let (source_id, admission, block) = candidate.into_parts();
    let replacement = BlockEnvelope::try_new(
        block.chain_id().clone(),
        block.block_height(),
        block.block_time(),
        block.confirmation_class(),
        block.events().to_vec(),
        BTreeMap::from([(source_id.clone(), [byte; 32])]),
    )
    .expect("replacement source evidence");
    BlockCandidate::try_new(source_id, admission, replacement).expect("candidate")
}

fn with_event_source_content_hash(candidate: BlockCandidate, byte: u8) -> BlockCandidate {
    let (source_id, admission, block) = candidate.into_parts();
    let original = &block.events()[0];
    let original_evidence = &original.source_evidence()[0];
    let evidence = match original_evidence.source_event_index() {
        Some(index) => SourceEvidence::try_new_indexed(
            source_id.clone(),
            original_evidence.source_version(),
            original_evidence.source_offset(),
            [byte; 32],
            index,
        ),
        None => SourceEvidence::try_new(
            source_id.clone(),
            original_evidence.source_version(),
            original_evidence.source_offset(),
            [byte; 32],
        ),
    }
    .expect("replacement event evidence");
    let event = CanonicalEventEnvelope::from_input(CanonicalEventInput {
        schema_version: original.schema_version().to_owned(),
        chain_id: original.chain_id().clone(),
        block_height: original.block_height(),
        block_time: original.block_time(),
        transaction_id: original.transaction_id().clone(),
        transaction_index: original.transaction_index(),
        canonical_event_index: original.canonical_event_index(),
        market_ids: original.market_ids().to_vec(),
        account_ids: original.account_addresses().to_vec(),
        source_evidence: vec![evidence],
        confirmation_class: original.confirmation_class(),
        observed_at: original.observed_at(),
        ingested_at: original.ingested_at(),
        canonicalized_at: original.canonicalized_at(),
        parser_version: original.parser_version().to_owned(),
        payload: original.payload().clone(),
    })
    .expect("replacement event");
    let replacement = BlockEnvelope::try_new(
        block.chain_id().clone(),
        block.block_height(),
        block.block_time(),
        block.confirmation_class(),
        vec![event],
        block.source_block_hashes().clone(),
    )
    .expect("replacement block");
    BlockCandidate::try_new(source_id, admission, replacement).expect("candidate")
}

fn committed_heights(decisions: &[SequencerDecision]) -> Vec<u64> {
    decisions
        .iter()
        .filter_map(|decision| match decision {
            SequencerDecision::Commit(block) => Some(block.block_height().get()),
            _ => None,
        })
        .collect()
}

#[test]
fn committed_gap_stops_then_independent_source_drains_contiguous_pending_blocks() {
    let mut sequencer = sequencer(8, 8);

    assert_eq!(
        committed_heights(
            &sequencer
                .observe(candidate(
                    100,
                    "primary",
                    SourceTrust::LocallyVerifiedCommitted,
                    1,
                ))
                .expect("height 100"),
        ),
        vec![100]
    );
    assert_eq!(
        committed_heights(
            &sequencer
                .observe(candidate(
                    101,
                    "primary",
                    SourceTrust::LocallyVerifiedCommitted,
                    1,
                ))
                .expect("height 101"),
        ),
        vec![101]
    );

    let gap = sequencer
        .observe(candidate(
            103,
            "primary",
            SourceTrust::LocallyVerifiedCommitted,
            1,
        ))
        .expect("future height");
    let gap_incident_id = match gap.as_slice() {
        [
            SequencerDecision::RequestGap {
                incident_id,
                start,
                end_inclusive,
            },
        ] if *start == BlockHeight::new(102) && *end_inclusive == BlockHeight::new(102) => {
            incident_id.clone()
        }
        other => panic!("unexpected gap decisions: {other:?}"),
    };
    assert_eq!(sequencer.committed_watermark(), Some(BlockHeight::new(101)));
    assert!(matches!(
        sequencer.health(),
        SequencerHealth::RedGap {
            incident_id,
            start,
            end_inclusive
        } if incident_id == gap_incident_id
            && incident_id.starts_with("inc_")
            && incident_id.len() == 68
            && start == BlockHeight::new(102)
            && end_inclusive == BlockHeight::new(102)
    ));

    let recovered = sequencer
        .observe(candidate(
            102,
            "secondary",
            SourceTrust::IndependentCommitted,
            1,
        ))
        .expect("independent recovery");
    assert_eq!(committed_heights(&recovered), vec![102, 103]);
    assert_eq!(sequencer.committed_watermark(), Some(BlockHeight::new(103)));
    assert_eq!(sequencer.outstanding_gap(), None);
}

#[test]
fn matching_duplicate_is_recorded_without_republication() {
    let mut sequencer = sequencer(8, 8);
    sequencer
        .observe(candidate(
            100,
            "primary",
            SourceTrust::LocallyVerifiedCommitted,
            7,
        ))
        .expect("initial commit");

    let duplicate = sequencer
        .observe(candidate(
            100,
            "secondary",
            SourceTrust::IndependentCommitted,
            7,
        ))
        .expect("matching duplicate");

    assert!(matches!(
        duplicate.as_slice(),
        [SequencerDecision::RecordDuplicate {
            block_height,
            source_id
        }] if *block_height == BlockHeight::new(100) && source_id.as_str() == "secondary"
    ));
}

#[test]
fn matching_pending_sources_merge_event_evidence_before_commit() {
    let mut sequencer = sequencer(8, 8);
    sequencer
        .observe(candidate(
            101,
            "primary",
            SourceTrust::LocallyVerifiedCommitted,
            7,
        ))
        .expect("future primary block");
    sequencer
        .observe(candidate(
            101,
            "secondary",
            SourceTrust::IndependentCommitted,
            7,
        ))
        .expect("matching independent block");

    let decisions = sequencer
        .observe(candidate(
            100,
            "primary",
            SourceTrust::LocallyVerifiedCommitted,
            7,
        ))
        .expect("gap recovery");
    let recovered = decisions
        .iter()
        .find_map(|decision| match decision {
            SequencerDecision::Commit(block) if block.block_height() == BlockHeight::new(101) => {
                Some(block)
            }
            _ => None,
        })
        .expect("recovered block 101");
    let source_ids = recovered.events()[0]
        .source_evidence()
        .iter()
        .map(|evidence| evidence.source_id().as_str())
        .collect::<Vec<_>>();

    assert_eq!(source_ids, vec!["primary", "secondary"]);
    assert_eq!(recovered.source_block_hashes().len(), 2);
}

#[test]
fn conflicting_content_quarantines_deterministically_and_latches_red_health() {
    let mut first = sequencer(8, 8);
    let mut second = sequencer(8, 8);

    first
        .observe(candidate(
            100,
            "primary",
            SourceTrust::LocallyVerifiedCommitted,
            7,
        ))
        .expect("initial primary commit");
    second
        .observe(candidate(
            100,
            "secondary",
            SourceTrust::IndependentCommitted,
            8,
        ))
        .expect("initial independent commit");

    let first_incident = first
        .observe(candidate(
            100,
            "secondary",
            SourceTrust::IndependentCommitted,
            8,
        ))
        .expect("divergence decision");
    let second_incident = second
        .observe(candidate(
            100,
            "primary",
            SourceTrust::LocallyVerifiedCommitted,
            7,
        ))
        .expect("divergence decision");

    let incident_id = match first_incident.as_slice() {
        [SequencerDecision::Quarantine(record)] => {
            assert_eq!(record.block_height(), BlockHeight::new(100));
            assert!(matches!(
                record.reason(),
                QuarantineReason::ConflictingCanonicalBlock { .. }
            ));
            record.incident_id().to_owned()
        }
        other => panic!("unexpected decisions: {other:?}"),
    };
    let reversed_incident_id = match second_incident.as_slice() {
        [SequencerDecision::Quarantine(record)] => record.incident_id(),
        other => panic!("unexpected reversed decisions: {other:?}"),
    };
    assert_eq!(reversed_incident_id, incident_id);
    assert_eq!(
        first.health(),
        SequencerHealth::Red {
            incident_id: incident_id.clone()
        }
    );

    let after_divergence = first
        .observe(candidate(
            101,
            "primary",
            SourceTrust::LocallyVerifiedCommitted,
            7,
        ))
        .expect("latched sequencer");
    assert_eq!(
        after_divergence,
        vec![SequencerDecision::AwaitOperatorResolution { incident_id }]
    );
    assert_eq!(first.committed_watermark(), Some(BlockHeight::new(100)));

    let provisional = first
        .observe(candidate(
            104,
            "provisional",
            SourceTrust::ThirdPartyProvisional,
            3,
        ))
        .expect("provisional displays remain available");
    assert!(matches!(
        provisional.as_slice(),
        [SequencerDecision::PublishProvisional(block)]
            if block.block_height() == BlockHeight::new(104)
    ));
}

#[test]
fn same_source_raw_block_hash_conflict_is_not_hidden_by_equal_canonical_content() {
    let mut sequencer = sequencer(8, 8);
    let first = with_source_block_hash(
        candidate(100, "primary", SourceTrust::LocallyVerifiedCommitted, 7),
        0x11,
    );
    let conflicting = with_source_block_hash(
        candidate(100, "primary", SourceTrust::LocallyVerifiedCommitted, 7),
        0x22,
    );
    sequencer.observe(first).expect("initial commit");

    let decisions = sequencer
        .observe(conflicting)
        .expect("source evidence divergence");
    assert!(matches!(
        decisions.as_slice(),
        [SequencerDecision::Quarantine(record)]
            if matches!(
                record.reason(),
                QuarantineReason::ConflictingSourceBlockHash { source_id, .. }
                    if source_id.as_str() == "primary"
            )
    ));
}

#[test]
fn same_event_source_locator_conflict_is_quarantined_after_commit() {
    let mut sequencer = sequencer(8, 8);
    let first = with_event_source_content_hash(
        candidate(100, "primary", SourceTrust::LocallyVerifiedCommitted, 7),
        0x11,
    );
    let conflicting = with_event_source_content_hash(
        candidate(100, "primary", SourceTrust::LocallyVerifiedCommitted, 7),
        0x22,
    );
    sequencer.observe(first).expect("initial commit");

    let decisions = sequencer
        .observe(conflicting)
        .expect("event evidence divergence");
    assert!(matches!(
        decisions.as_slice(),
        [SequencerDecision::Quarantine(record)]
            if matches!(
                record.reason(),
                QuarantineReason::ConflictingEventSourceEvidence {
                    source_id,
                    existing_hash,
                    conflicting_hash,
                    ..
                } if source_id.as_str() == "primary"
                    && *existing_hash == [0x11; 32]
                    && *conflicting_hash == [0x22; 32]
            )
    ));
}

#[test]
fn provisional_candidate_advances_only_the_provisional_watermark() {
    let mut sequencer = sequencer(8, 8);
    let decisions = sequencer
        .observe(candidate(
            104,
            "provisional",
            SourceTrust::ThirdPartyProvisional,
            3,
        ))
        .expect("provisional block");

    assert!(matches!(
        decisions.as_slice(),
        [SequencerDecision::PublishProvisional(block)]
            if block.block_height() == BlockHeight::new(104)
    ));
    assert_eq!(sequencer.committed_watermark(), None);
    assert_eq!(
        sequencer.provisional_watermark(),
        Some(BlockHeight::new(104))
    );
}

#[test]
fn candidate_rejects_missing_source_evidence_and_confirmation_mismatch() {
    let valid = candidate(100, "primary", SourceTrust::LocallyVerifiedCommitted, 1);
    let (_, admission, block) = valid.into_parts();
    let missing = BlockCandidate::try_new(
        SourceId::new("absent").expect("source"),
        admission,
        block.clone(),
    );
    assert_eq!(missing, Err(CandidateError::MissingSourceBlockHash));

    let mismatch = BlockCandidate::try_new(
        SourceId::new("primary").expect("source"),
        admission,
        BlockEnvelope::try_new(
            block.chain_id().clone(),
            block.block_height(),
            block.block_time(),
            ConfirmationClass::ProvisionalSource,
            Vec::new(),
            block.source_block_hashes().clone(),
        )
        .expect("provisional block"),
    );
    assert_eq!(mismatch, Err(CandidateError::ConfirmationMismatch));

    let original = &block.events()[0];
    let foreign_event = CanonicalEventEnvelope::from_input(CanonicalEventInput {
        schema_version: original.schema_version().to_owned(),
        chain_id: original.chain_id().clone(),
        block_height: original.block_height(),
        block_time: original.block_time(),
        transaction_id: original.transaction_id().clone(),
        transaction_index: original.transaction_index(),
        canonical_event_index: original.canonical_event_index(),
        market_ids: original.market_ids().to_vec(),
        account_ids: original.account_addresses().to_vec(),
        source_evidence: vec![
            SourceEvidence::try_new(
                SourceId::new("foreign").expect("foreign source"),
                "node-v1",
                "block:100",
                [0x11; 32],
            )
            .expect("foreign evidence"),
        ],
        confirmation_class: original.confirmation_class(),
        observed_at: original.observed_at(),
        ingested_at: original.ingested_at(),
        canonicalized_at: original.canonicalized_at(),
        parser_version: original.parser_version().to_owned(),
        payload: original.payload().clone(),
    })
    .expect("foreign event");
    let foreign_block = BlockEnvelope::try_new(
        block.chain_id().clone(),
        block.block_height(),
        block.block_time(),
        block.confirmation_class(),
        vec![foreign_event],
        block.source_block_hashes().clone(),
    )
    .expect("foreign block");
    assert_eq!(
        BlockCandidate::try_new(
            SourceId::new("primary").expect("source"),
            admission,
            foreign_block,
        ),
        Err(CandidateError::UnexpectedEventSourceEvidence)
    );
}

#[test]
fn candidate_admission_covers_every_constructible_publication_lane() {
    for trust in SourceTrust::ALL {
        for class in ObservationClass::ALL {
            let Ok(admission) = SourceAdmission::new(trust, class) else {
                continue;
            };
            let confirmation = match admission.publication_lane() {
                PublicationLane::CommittedCandidate => match trust {
                    SourceTrust::LocallyVerifiedCommitted => ConfirmationClass::CommittedPrimary,
                    SourceTrust::IndependentCommitted => ConfirmationClass::CommittedIndependent,
                    SourceTrust::ReconciledSnapshot
                    | SourceTrust::RecoveryOnly
                    | SourceTrust::ThirdPartyProvisional
                    | SourceTrust::MempoolProvisional => {
                        panic!("committed-candidate lane must not admit {trust:?}")
                    }
                },
                PublicationLane::Provisional => match trust {
                    SourceTrust::ThirdPartyProvisional => ConfirmationClass::ProvisionalSource,
                    SourceTrust::LocallyVerifiedCommitted
                    | SourceTrust::IndependentCommitted
                    | SourceTrust::ReconciledSnapshot
                    | SourceTrust::RecoveryOnly
                    | SourceTrust::MempoolProvisional => {
                        panic!("provisional lane must not admit {trust:?}")
                    }
                },
                PublicationLane::Reconciliation
                | PublicationLane::Recovery
                | PublicationLane::Mempool => ConfirmationClass::CommittedPrimary,
            };
            let (source_id, block) = classified_block(100, "primary", confirmation, 1);
            let result = BlockCandidate::try_new(source_id, admission, block);
            match admission.publication_lane() {
                PublicationLane::CommittedCandidate | PublicationLane::Provisional => {
                    let candidate = result.expect("accepted sequencer lanes still construct");
                    assert_eq!(
                        candidate.admission().publication_lane(),
                        admission.publication_lane()
                    );
                    assert_eq!(candidate.block().confirmation_class(), confirmation);
                }
                PublicationLane::Reconciliation
                | PublicationLane::Recovery
                | PublicationLane::Mempool => {
                    let error = result.expect_err("unsupported lanes fail closed");
                    assert_eq!(
                        error,
                        CandidateError::UnsupportedPublicationLane,
                        "{trust:?}/{class:?} must not blur into the canonical sequencer"
                    );
                    assert_eq!(
                        error.reason_code(),
                        "sequencer.unsupported_publication_lane",
                        "{trust:?}/{class:?} must reuse the existing unsupported-lane reason"
                    );
                }
            }
        }
    }
}

#[test]
fn pending_capacity_exhaustion_fails_without_dropping_buffered_evidence() {
    let mut sequencer = sequencer(1, 8);
    sequencer
        .observe(candidate(
            102,
            "primary",
            SourceTrust::LocallyVerifiedCommitted,
            1,
        ))
        .expect("first future block");

    assert_eq!(
        sequencer.observe(candidate(
            103,
            "primary",
            SourceTrust::LocallyVerifiedCommitted,
            1,
        )),
        Err(SequencerError::PendingCapacityExceeded { limit: 1 })
    );
    assert_eq!(sequencer.pending_block_count(), 1);
    assert_eq!(sequencer.committed_watermark(), None);
}

#[test]
fn evicted_committed_history_requires_archive_verification() {
    let mut sequencer = sequencer(8, 2);
    for height in 100..=102 {
        sequencer
            .observe(candidate(
                height,
                "primary",
                SourceTrust::LocallyVerifiedCommitted,
                1,
            ))
            .expect("contiguous commit");
    }

    let old = candidate(100, "secondary", SourceTrust::IndependentCommitted, 1);
    let expected_hash = old.block().canonical_block_hash();
    let decisions = sequencer
        .observe(old)
        .expect("archive verification decision");

    assert_eq!(
        decisions,
        vec![SequencerDecision::VerifyHistoricalBlock {
            block_height: BlockHeight::new(100),
            source_id: SourceId::new("secondary").expect("source"),
            canonical_block_hash: expected_hash,
        }]
    );
}

#[test]
fn expected_recovery_block_is_accepted_even_when_future_buffer_is_full() {
    let mut sequencer = sequencer(1, 8);
    sequencer
        .observe(candidate(
            102,
            "primary",
            SourceTrust::LocallyVerifiedCommitted,
            1,
        ))
        .expect("future block");

    let recovered = sequencer
        .observe(candidate(
            100,
            "secondary",
            SourceTrust::IndependentCommitted,
            1,
        ))
        .expect("expected recovery block");
    assert_eq!(committed_heights(&recovered), vec![100]);
    assert_eq!(sequencer.pending_block_count(), 1);
    let gap = sequencer
        .outstanding_gap()
        .expect("remaining height 101 gap");
    assert_eq!(gap.start(), BlockHeight::new(101));
    assert_eq!(gap.end_inclusive(), BlockHeight::new(101));
}

#[test]
fn invalid_configuration_is_rejected() {
    let chain = ChainId::new("mainnet").expect("chain");
    assert_eq!(
        SequencerConfig::try_new(chain.clone(), BlockHeight::new(0), 0, 1),
        Err(SequencerError::InvalidCapacity {
            field: "max_pending_blocks"
        })
    );
    assert_eq!(
        SequencerConfig::try_new(chain, BlockHeight::new(0), 1, 0),
        Err(SequencerError::InvalidCapacity {
            field: "retained_committed_blocks"
        })
    );
}

#[test]
fn maximum_height_commits_without_a_partial_state_error() {
    let mut sequencer = CanonicalSequencer::new(
        SequencerConfig::try_new(
            ChainId::new("mainnet").expect("chain"),
            BlockHeight::new(u64::MAX),
            1,
            1,
        )
        .expect("terminal sequencer"),
    );

    let decisions = sequencer
        .observe(candidate(
            u64::MAX,
            "primary",
            SourceTrust::LocallyVerifiedCommitted,
            1,
        ))
        .expect("maximum height commit");
    assert_eq!(committed_heights(&decisions), vec![u64::MAX]);
    assert_eq!(
        sequencer.committed_watermark(),
        Some(BlockHeight::new(u64::MAX))
    );
}
