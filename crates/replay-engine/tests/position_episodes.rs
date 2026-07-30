use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;

use canonical_archive::{ArchiveConfig, LocalParquetArchive};
use canonical_events::{
    AssetContextUpdated, BlockEnvelope, CanonicalEventEnvelope, CanonicalEventInput,
    ConfirmationClass, DexCreated, EventPayload, MarketCreated, SourceEvidence, TradeMatched,
    TradeParticipantRoleV1, TradeParticipantV1,
};
use canonical_ledger::{
    ApplyContext, CanonicalAccountReducerV1, CanonicalLedger, CanonicalMarketReducerV1,
    CanonicalPositionEpisodeReducerV1, CanonicalPositionReducerV1, CanonicalTradeReducerSetV2,
    EventReducer, LedgerLimits, ReducerError, StateMutation, StateView,
};
use domain_types::{
    Address, AssetId, BlockHeight, BlockRange, ChainId, DexId, KnownTime, ManifestId, MarketId,
    OrderId, PositionQuantity, Price, ProtocolTime, Quantity, SourceId, TradeId, TransactionId,
};
use replay_engine::{
    ReplayCancellation, ReplayLimits, ReplayOutcome, ReplayRequest, SerialReplayEngine,
};
use storage_ports::CanonicalArchive;

const BUYER: Address = Address::from_bytes([0x11; 20]);
const SELLER: Address = Address::from_bytes([0x22; 20]);
const OPERATOR: Address = Address::from_bytes([0x33; 20]);

#[derive(Debug, Clone, Copy, Default)]
struct EpisodeReplayReducer {
    market: CanonicalMarketReducerV1,
    trade: CanonicalTradeReducerSetV2,
    account: CanonicalAccountReducerV1,
    quantity: CanonicalPositionReducerV1,
    episode: CanonicalPositionEpisodeReducerV1,
}

impl EventReducer for EpisodeReplayReducer {
    fn reducer_set_version(&self) -> &str {
        "position-episode-replay-test@1.0.0"
    }

    fn supports(&self, event: &CanonicalEventEnvelope) -> bool {
        EventReducer::supports(&self.market, event)
            || EventReducer::supports(&self.trade, event)
            || EventReducer::supports(&self.account, event)
            || EventReducer::supports(&self.quantity, event)
            || EventReducer::supports(&self.episode, event)
    }

    fn reduce(
        &self,
        state: &StateView<'_>,
        event: &CanonicalEventEnvelope,
        context: &ApplyContext<'_>,
    ) -> Result<Vec<StateMutation>, ReducerError> {
        if EventReducer::supports(&self.market, event) {
            return EventReducer::reduce(&self.market, state, event, context);
        }
        let mut mutations = Vec::new();
        for child in [
            &self.trade as &dyn EventReducer,
            &self.account,
            &self.quantity,
            &self.episode,
        ] {
            if child.supports(event) {
                mutations.extend(child.reduce(state, event, context)?);
            }
        }
        let mut keys = BTreeSet::new();
        if !mutations.iter().all(|mutation| keys.insert(mutation.key())) {
            return Err(ReducerError::try_new(
                "position_episode.duplicate_mutation_key",
                "replay test children emitted duplicate keys",
            )
            .unwrap());
        }
        Ok(mutations)
    }
}

#[test]
fn episode_replay_is_byte_identical_when_repeated_and_resumed() {
    let fixture = ReplayFixture::with_blocks(vec![
        market_block(100),
        trade_block(101, "trd-open", "1", BUYER, "0", SELLER, "0"),
        trade_block(102, "trd-reversal", "1.5", SELLER, "-1", BUYER, "1"),
    ]);

    let mut first = ledger(100);
    let mut second = ledger(100);
    let first_request = fixture.request(100, 102, fixture.manifests.clone(), first.state_hash());
    let second_request = fixture.request(100, 102, fixture.manifests.clone(), second.state_hash());
    let first_receipt =
        SerialReplayEngine::new(&fixture.archive, &mut first, ReplayLimits::production())
            .run(&first_request, &NeverCancel)
            .unwrap();
    let second_receipt =
        SerialReplayEngine::new(&fixture.archive, &mut second, ReplayLimits::production())
            .run(&second_request, &NeverCancel)
            .unwrap();
    let (ReplayOutcome::Completed(first_receipt), ReplayOutcome::Completed(second_receipt)) =
        (first_receipt, second_receipt)
    else {
        panic!("both replays must complete");
    };
    assert_eq!(
        first_receipt.canonical_bytes(),
        second_receipt.canonical_bytes()
    );
    assert_eq!(first.state_hash(), second.state_hash());
    assert_eq!(
        first.state_image().canonical_bytes(),
        second.state_image().canonical_bytes()
    );

    let mut partial = ledger(100);
    let partial_request = fixture.request(
        100,
        101,
        fixture.manifests[..2].to_vec(),
        partial.state_hash(),
    );
    SerialReplayEngine::new(&fixture.archive, &mut partial, ReplayLimits::production())
        .run(&partial_request, &NeverCancel)
        .unwrap();
    let mut resumed = CanonicalLedger::try_from_state_image(
        partial.state_image().clone(),
        EpisodeReplayReducer::default(),
        LedgerLimits::production(),
    )
    .unwrap();
    let resume_request = fixture.request(
        102,
        102,
        fixture.manifests[2..].to_vec(),
        resumed.state_hash(),
    );
    SerialReplayEngine::new(&fixture.archive, &mut resumed, ReplayLimits::production())
        .run(&resume_request, &NeverCancel)
        .unwrap();
    assert_eq!(resumed.state_hash(), first.state_hash());
    assert_eq!(
        resumed.state_image().canonical_bytes(),
        first.state_image().canonical_bytes()
    );
}

struct ReplayFixture {
    _temporary: tempfile::TempDir,
    archive: LocalParquetArchive,
    manifests: Vec<ManifestId>,
    schema_fingerprint: [u8; 32],
}

impl ReplayFixture {
    fn with_blocks(blocks: Vec<BlockEnvelope>) -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let archive = LocalParquetArchive::open(
            temporary.path(),
            ArchiveConfig::deterministic_fixture(
                "position-episode-replay",
                KnownTime::from_unix_micros(10_000).unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
        let manifests: Vec<_> = blocks
            .iter()
            .map(|block| archive.append_block(block).unwrap().manifest_id().clone())
            .collect();
        let verified = archive.verify_manifest(&manifests[0]).unwrap();
        let schema_fingerprint = *verified
            .schema_fingerprints()
            .get("canonical_events")
            .unwrap();
        Self {
            _temporary: temporary,
            archive,
            manifests,
            schema_fingerprint,
        }
    }

    fn request(
        &self,
        start: u64,
        end: u64,
        manifests: Vec<ManifestId>,
        expected_start_state_hash: [u8; 32],
    ) -> ReplayRequest {
        ReplayRequest::try_new(
            ChainId::new("mainnet").unwrap(),
            BlockRange::new(BlockHeight::new(start), BlockHeight::new(end)).unwrap(),
            manifests,
            expected_start_state_hash,
            "canonical_events",
            self.schema_fingerprint,
        )
        .unwrap()
    }
}

#[derive(Debug, Clone, Copy)]
struct NeverCancel;

impl ReplayCancellation for NeverCancel {
    fn is_cancelled(&self) -> bool {
        false
    }
}

fn ledger(first_height: u64) -> CanonicalLedger<EpisodeReplayReducer> {
    CanonicalLedger::try_new(
        ChainId::new("mainnet").unwrap(),
        BlockHeight::new(first_height),
        EpisodeReplayReducer::default(),
        LedgerLimits::production(),
    )
    .unwrap()
}

fn market_block(height: u64) -> BlockEnvelope {
    let base = AssetId::new("BTC").unwrap();
    let quote = AssetId::new("USDC").unwrap();
    block(
        height,
        vec![
            event(
                height,
                0,
                EventPayload::DexCreated(DexCreated {
                    dex_id: DexId::new("validator").unwrap(),
                    name: "Validator".to_owned(),
                    operator_account_id: OPERATOR,
                }),
                Vec::new(),
                vec![OPERATOR],
            ),
            event(
                height,
                1,
                EventPayload::AssetContextUpdated(AssetContextUpdated {
                    asset_id: base.clone(),
                    context_version: "btc-v1".to_owned(),
                    context_hash: [1; 32],
                }),
                Vec::new(),
                Vec::new(),
            ),
            event(
                height,
                2,
                EventPayload::AssetContextUpdated(AssetContextUpdated {
                    asset_id: quote.clone(),
                    context_version: "usdc-v1".to_owned(),
                    context_hash: [2; 32],
                }),
                Vec::new(),
                Vec::new(),
            ),
            event(
                height,
                3,
                EventPayload::MarketCreated(MarketCreated {
                    market_id: market(),
                    dex_id: DexId::new("validator").unwrap(),
                    base_asset_id: base,
                    quote_asset_id: quote,
                    tick_size: Price::parse_at_scale("0.1", 6).unwrap(),
                    lot_size: Quantity::parse_at_scale("0.001", 8).unwrap(),
                }),
                vec![market()],
                Vec::new(),
            ),
        ],
    )
}

#[allow(clippy::too_many_arguments)]
fn trade_block(
    height: u64,
    trade_id: &str,
    fill: &str,
    buyer: Address,
    buyer_start: &str,
    seller: Address,
    seller_start: &str,
) -> BlockEnvelope {
    block(
        height,
        vec![event(
            height,
            0,
            EventPayload::TradeMatched(TradeMatched {
                trade_id: Some(TradeId::new(trade_id).unwrap()),
                market_id: Some(market()),
                maker_order_id: None,
                taker_order_id: None,
                price: Price::from_str("100").unwrap(),
                quantity: Quantity::from_str(fill).unwrap(),
                deterministic_seed: height,
                participants: Some(Box::new([
                    TradeParticipantV1 {
                        role: TradeParticipantRoleV1::Buyer,
                        account_id: buyer,
                        start_position: PositionQuantity::from_str(buyer_start).unwrap(),
                        order_id: OrderId::new(format!("buyer-{trade_id}")).unwrap(),
                        twap_id: None,
                        client_order_id: None,
                    },
                    TradeParticipantV1 {
                        role: TradeParticipantRoleV1::Seller,
                        account_id: seller,
                        start_position: PositionQuantity::from_str(seller_start).unwrap(),
                        order_id: OrderId::new(format!("seller-{trade_id}")).unwrap(),
                        twap_id: None,
                        client_order_id: None,
                    },
                ])),
            }),
            vec![market()],
            vec![buyer, seller],
        )],
    )
}

fn event(
    height: u64,
    index: u32,
    payload: EventPayload,
    market_ids: Vec<MarketId>,
    account_ids: Vec<Address>,
) -> CanonicalEventEnvelope {
    let payload_hash = *blake3::hash(&payload.encode_to_vec().unwrap()).as_bytes();
    CanonicalEventEnvelope::from_input(CanonicalEventInput {
        schema_version: "1.0.0".to_owned(),
        chain_id: ChainId::new("mainnet").unwrap(),
        block_height: BlockHeight::new(height),
        block_time: ProtocolTime::from_unix_micros(height as i64).unwrap(),
        transaction_id: TransactionId::new(format!("tx-{height}-{index}")).unwrap(),
        transaction_index: index,
        canonical_event_index: 0,
        market_ids,
        account_ids,
        source_evidence: vec![
            SourceEvidence::try_new_indexed(
                SourceId::new("test-primary").unwrap(),
                "position-episode-replay",
                height.to_string(),
                payload_hash,
                index,
            )
            .unwrap(),
        ],
        confirmation_class: ConfirmationClass::CommittedPrimary,
        observed_at: KnownTime::from_unix_micros(height as i64).unwrap(),
        ingested_at: KnownTime::from_unix_micros(height as i64).unwrap(),
        canonicalized_at: KnownTime::from_unix_micros(height as i64).unwrap(),
        parser_version: "position-episode-replay@1.0.0".to_owned(),
        payload,
    })
    .unwrap()
}

fn block(height: u64, events: Vec<CanonicalEventEnvelope>) -> BlockEnvelope {
    BlockEnvelope::try_new(
        ChainId::new("mainnet").unwrap(),
        BlockHeight::new(height),
        ProtocolTime::from_unix_micros(height as i64).unwrap(),
        ConfirmationClass::CommittedPrimary,
        events,
        BTreeMap::from([(
            SourceId::new("test-primary").unwrap(),
            *blake3::hash(&height.to_be_bytes()).as_bytes(),
        )]),
    )
    .unwrap()
}

fn market() -> MarketId {
    MarketId::new("perp:BTC").unwrap()
}
