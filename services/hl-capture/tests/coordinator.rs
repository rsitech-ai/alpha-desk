use std::{
    collections::{BTreeMap, btree_map::Entry},
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use canonical_events::{
    BlockEnvelope, CanonicalEventEnvelope, CanonicalEventInput, ConfirmationClass, EventPayload,
    SourceEvidence, TradeMatched,
};
use domain_types::{
    BlockHeight, ChainId, KnownTime, ManifestId, Price, ProtocolTime, Quantity, SourceId,
    TransactionId,
};
use hl_capture::{
    bus::{CanonicalPublisher, PublicationAck, PublicationError, PublicationMessage, Subject},
    coordinator::{
        AcknowledgementClock, CaptureArchive, CaptureCoordinator, CoordinatorError,
        CoordinatorFaultInjector, CoordinatorFaultPoint, NoCoordinatorFaults,
    },
    progress::InMemoryProgressStore,
};
use storage_ports::{ArchiveError, ArchiveReceipt, CaptureProgressStore};

fn known(micros: i64) -> KnownTime {
    KnownTime::from_unix_micros(micros).expect("known time")
}

fn canonical_block(height: u64, seed: u64) -> BlockEnvelope {
    let block_time_micros = 1_721_779_200_000_000_i64
        .checked_add(i64::try_from(height).expect("height fits i64"))
        .expect("block time");
    let block_time =
        ProtocolTime::from_unix_micros(block_time_micros).expect("protocol block time");
    let source_id = SourceId::new("primary-node").expect("source ID");
    let event = CanonicalEventEnvelope::from_input(CanonicalEventInput {
        schema_version: "1.0.0".to_owned(),
        chain_id: ChainId::new("mainnet").expect("chain ID"),
        block_height: BlockHeight::new(height),
        block_time,
        transaction_id: TransactionId::new(format!("tx-{height}")).expect("transaction ID"),
        transaction_index: 0,
        canonical_event_index: 0,
        market_ids: Vec::new(),
        account_ids: Vec::new(),
        source_evidence: vec![
            SourceEvidence::try_new(
                source_id.clone(),
                "node-v1",
                format!("block-{height}:0"),
                [u8::try_from(seed).unwrap_or(0x7f); 32],
            )
            .expect("source evidence"),
        ],
        confirmation_class: ConfirmationClass::CommittedPrimary,
        observed_at: known(block_time_micros),
        ingested_at: known(block_time_micros + 1),
        canonicalized_at: known(block_time_micros + 2),
        parser_version: "canonical-parser-v1".to_owned(),
        payload: EventPayload::TradeMatched(TradeMatched::without_identities(
            Price::parse_at_scale("65000", 6).expect("price"),
            Quantity::parse_at_scale("0.01", 8).expect("quantity"),
            seed,
        )),
    })
    .expect("canonical event");
    BlockEnvelope::try_new(
        ChainId::new("mainnet").expect("chain ID"),
        BlockHeight::new(height),
        block_time,
        ConfirmationClass::CommittedPrimary,
        vec![event],
        BTreeMap::from([(source_id, [0x55; 32])]),
    )
    .expect("canonical block")
}

#[derive(Debug, Default)]
struct MemoryArchive {
    blocks: Mutex<BTreeMap<BlockHeight, (BlockEnvelope, ArchiveReceipt)>>,
}

impl MemoryArchive {
    fn receipt(block: &BlockEnvelope) -> ArchiveReceipt {
        ArchiveReceipt::try_new(
            format!("receipt-{}", block.block_height().get()),
            ManifestId::new(format!(
                "manifest-{}",
                hex::encode(block.canonical_block_hash())
            ))
            .expect("manifest ID"),
            block.block_height(),
            block.canonical_block_hash(),
            [0x11; 32],
            [0x22; 32],
            [0x33; 32],
            known(1_721_779_300_000_000),
        )
        .expect("archive receipt")
    }

    fn unique_blocks(&self) -> usize {
        self.blocks.lock().expect("archive lock").len()
    }
}

#[async_trait]
impl CaptureArchive for MemoryArchive {
    async fn append_block(&self, block: &BlockEnvelope) -> Result<ArchiveReceipt, ArchiveError> {
        let mut blocks = self.blocks.lock().expect("archive lock");
        let receipt = Self::receipt(block);
        match blocks.entry(block.block_height()) {
            Entry::Vacant(entry) => {
                entry.insert((block.clone(), receipt.clone()));
                Ok(receipt)
            }
            Entry::Occupied(entry) if entry.get().0 == *block => Ok(entry.get().1.clone()),
            Entry::Occupied(_) => Err(ArchiveError::ConflictingBlock(block.block_height())),
        }
    }

    async fn load_block(
        &self,
        chain_id: &ChainId,
        block_height: BlockHeight,
    ) -> Result<BlockEnvelope, ArchiveError> {
        let blocks = self.blocks.lock().expect("archive lock");
        let block = blocks
            .get(&block_height)
            .ok_or(ArchiveError::RangeUnavailable)?
            .0
            .clone();
        if block.chain_id() != chain_id {
            return Err(ArchiveError::RangeUnavailable);
        }
        Ok(block)
    }
}

#[derive(Debug, Default)]
struct MemoryPublisher {
    retained: Mutex<BTreeMap<String, RetainedPublication>>,
    attempts: Mutex<BTreeMap<String, usize>>,
}

#[derive(Debug, Clone, Copy)]
struct RetainedPublication {
    subject: Subject,
    publication_sha256: [u8; 32],
    sequence: u64,
}

impl MemoryPublisher {
    fn retained_count(&self) -> usize {
        self.retained.lock().expect("publisher lock").len()
    }
}

#[async_trait]
impl CanonicalPublisher for MemoryPublisher {
    async fn publish(
        &self,
        message: &PublicationMessage,
    ) -> Result<PublicationAck, PublicationError> {
        *self
            .attempts
            .lock()
            .expect("attempt lock")
            .entry(message.message_id().to_owned())
            .or_default() += 1;
        let mut retained = self.retained.lock().expect("publisher lock");
        let next_sequence = u64::try_from(retained.len())
            .expect("small retained set")
            .checked_add(1)
            .expect("sequence");
        let (sequence, duplicate) = match retained.entry(message.message_id().to_owned()) {
            Entry::Vacant(entry) => {
                entry.insert(RetainedPublication {
                    subject: message.subject(),
                    publication_sha256: message.publication_sha256(),
                    sequence: next_sequence,
                });
                (next_sequence, false)
            }
            Entry::Occupied(entry)
                if entry.get().subject == message.subject()
                    && entry.get().publication_sha256 == message.publication_sha256() =>
            {
                (entry.get().sequence, true)
            }
            Entry::Occupied(_) => {
                return Err(PublicationError::DivergentMessageId {
                    message_id: message.message_id().to_owned(),
                });
            }
        };
        PublicationAck::try_new(message, message.stream().to_owned(), sequence, duplicate)
    }
}

#[derive(Debug)]
struct DeterministicClock;

impl AcknowledgementClock for DeterministicClock {
    fn acknowledged_at(
        &self,
        block_height: BlockHeight,
        ordinal: u32,
    ) -> Result<KnownTime, CoordinatorError> {
        let offset = i64::try_from(block_height.get())
            .expect("test block height")
            .checked_mul(100)
            .and_then(|value| value.checked_add(i64::from(ordinal)))
            .expect("test time offset");
        Ok(known(1_721_779_400_000_000 + offset))
    }
}

#[derive(Debug)]
struct OneShotFault {
    point: Mutex<Option<CoordinatorFaultPoint>>,
}

impl OneShotFault {
    fn new(point: CoordinatorFaultPoint) -> Self {
        Self {
            point: Mutex::new(Some(point)),
        }
    }
}

impl CoordinatorFaultInjector for OneShotFault {
    fn check(&self, point: CoordinatorFaultPoint) -> Result<(), CoordinatorError> {
        let mut selected = self.point.lock().expect("fault lock");
        if selected.as_ref() == Some(&point) {
            selected.take();
            Err(CoordinatorError::InjectedFault(point))
        } else {
            Ok(())
        }
    }
}

#[tokio::test]
async fn every_durable_boundary_recovers_to_one_exact_publication_set() {
    let fault_points = [
        CoordinatorFaultPoint::AfterArchive,
        CoordinatorFaultPoint::AfterJournal,
        CoordinatorFaultPoint::AfterPublish { ordinal: 0 },
        CoordinatorFaultPoint::AfterAcknowledgement { ordinal: 0 },
        CoordinatorFaultPoint::AfterPublish { ordinal: 1 },
        CoordinatorFaultPoint::AfterAcknowledgement { ordinal: 1 },
        CoordinatorFaultPoint::AfterCursor,
    ];

    for point in fault_points {
        let archive = Arc::new(MemoryArchive::default());
        let progress = Arc::new(InMemoryProgressStore::new(32).expect("progress store"));
        let publisher = Arc::new(MemoryPublisher::default());
        let chain = ChainId::new("mainnet").expect("chain ID");
        progress
            .initialize_chain(&chain, BlockHeight::new(42))
            .await
            .expect("initialize chain");
        let block = canonical_block(42, 7);
        let coordinator = CaptureCoordinator::new(
            archive.clone(),
            progress.clone(),
            publisher.clone(),
            Arc::new(DeterministicClock),
            Arc::new(OneShotFault::new(point)),
        );
        assert_eq!(
            coordinator
                .process_block(&block)
                .await
                .expect_err("selected crash boundary"),
            CoordinatorError::InjectedFault(point),
            "{point:?}"
        );

        let restarted = CaptureCoordinator::new(
            archive.clone(),
            progress.clone(),
            publisher.clone(),
            Arc::new(DeterministicClock),
            Arc::new(NoCoordinatorFaults),
        );
        let cursor = restarted
            .process_block(&block)
            .await
            .expect("restart completes exact block");
        assert_eq!(cursor.committed_block_height(), BlockHeight::new(42));
        assert_eq!(cursor.cursor_version(), 1);
        assert_eq!(archive.unique_blocks(), 1);
        assert_eq!(publisher.retained_count(), 2);
        assert_eq!(
            progress
                .load_acknowledgements(&chain, BlockHeight::new(42))
                .await
                .expect("durable acknowledgements")
                .len(),
            2
        );
        assert!(
            progress
                .pending_blocks(&chain, 8)
                .await
                .expect("pending blocks")
                .is_empty()
        );
    }
}

#[tokio::test]
async fn startup_recovery_completes_journalled_blocks_without_source_replay() {
    let archive = Arc::new(MemoryArchive::default());
    let progress = Arc::new(InMemoryProgressStore::new(32).expect("progress store"));
    let publisher = Arc::new(MemoryPublisher::default());
    let chain = ChainId::new("mainnet").expect("chain ID");
    progress
        .initialize_chain(&chain, BlockHeight::new(42))
        .await
        .expect("initialize chain");
    let block = canonical_block(42, 7);
    let first = CaptureCoordinator::new(
        archive.clone(),
        progress.clone(),
        publisher.clone(),
        Arc::new(DeterministicClock),
        Arc::new(OneShotFault::new(CoordinatorFaultPoint::AfterJournal)),
    );
    first
        .process_block(&block)
        .await
        .expect_err("crash after journal");

    let restarted = CaptureCoordinator::new(
        archive,
        progress.clone(),
        publisher.clone(),
        Arc::new(DeterministicClock),
        Arc::new(NoCoordinatorFaults),
    );
    let recovered = restarted
        .recover_pending(&chain, 8)
        .await
        .expect("recover pending journal");
    assert_eq!(recovered, vec![BlockHeight::new(42)]);
    assert_eq!(
        progress
            .load_cursor(&chain)
            .await
            .expect("load cursor")
            .expect("cursor")
            .committed_block_height(),
        BlockHeight::new(42)
    );
    assert_eq!(publisher.retained_count(), 2);
}

#[tokio::test]
async fn startup_recovery_reconciles_archive_only_prefix_without_source_replay() {
    let archive = Arc::new(MemoryArchive::default());
    let progress = Arc::new(InMemoryProgressStore::new(32).expect("progress store"));
    let publisher = Arc::new(MemoryPublisher::default());
    let chain = ChainId::new("mainnet").expect("chain ID");
    progress
        .initialize_chain(&chain, BlockHeight::new(42))
        .await
        .expect("initialize chain");
    let first = canonical_block(42, 7);
    let second = canonical_block(43, 8);
    archive
        .append_block(&first)
        .await
        .expect("archive-only first block");
    archive
        .append_block(&second)
        .await
        .expect("archive-only second block");

    let restarted = CaptureCoordinator::new(
        archive,
        progress.clone(),
        publisher.clone(),
        Arc::new(DeterministicClock),
        Arc::new(NoCoordinatorFaults),
    );
    assert_eq!(
        restarted
            .recover_startup(&chain, 8)
            .await
            .expect("recover contiguous archive prefix"),
        vec![BlockHeight::new(42), BlockHeight::new(43)]
    );
    let cursor = progress
        .load_cursor(&chain)
        .await
        .expect("load cursor")
        .expect("cursor");
    assert_eq!(cursor.committed_block_height(), BlockHeight::new(43));
    assert_eq!(cursor.cursor_version(), 2);
    assert_eq!(publisher.retained_count(), 4);
}
