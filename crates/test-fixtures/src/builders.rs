use canonical_events::{
    CanonicalEventEnvelope, ConfirmationClass, ContractError, EventPayload, TradeMatched,
};
use domain_types::{
    Address, BlockHeight, EventId, MarketId, Price, ProtocolTime, Quantity, TransactionId,
    ValueError,
};

const FIXTURE_EPOCH_MICROS: i64 = 1_721_779_200_000_000;

#[derive(Debug, thiserror::Error)]
pub enum ScenarioBuildError {
    #[error("fixture block time overflow for height {height}")]
    BlockTimeOverflow { height: u64 },
    #[error("invalid deterministic fixture field {field}: {source}")]
    InvalidValue {
        field: &'static str,
        #[source]
        source: ValueError,
    },
    #[error("canonical fixture construction failed: {0}")]
    Contract(#[from] ContractError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TradeScenarioBuilder {
    block_height: BlockHeight,
    transaction_index: u32,
    event_index: u32,
    seed: u64,
}

impl TradeScenarioBuilder {
    #[must_use]
    pub const fn at_block(block_height: u64) -> Self {
        Self {
            block_height: BlockHeight::new(block_height),
            transaction_index: 0,
            event_index: 0,
            seed: 0,
        }
    }

    #[must_use]
    pub const fn with_seed(mut self, seed: u64) -> Self {
        self.seed = seed;
        self
    }

    pub fn matched_trade(
        self,
        buyer: Address,
        seller: Address,
    ) -> Result<CanonicalEventEnvelope, ScenarioBuildError> {
        let height = self.block_height.get();
        let height_micros =
            i64::try_from(height).map_err(|_| ScenarioBuildError::BlockTimeOverflow { height })?;
        let block_time_micros = FIXTURE_EPOCH_MICROS
            .checked_add(height_micros)
            .ok_or(ScenarioBuildError::BlockTimeOverflow { height })?;
        let block_time = ProtocolTime::from_unix_micros(block_time_micros).map_err(|source| {
            ScenarioBuildError::InvalidValue {
                field: "block_time_micros",
                source,
            }
        })?;
        let transaction_id =
            TransactionId::new(format!("fixture-tx-{height}-{}", self.transaction_index)).map_err(
                |source| ScenarioBuildError::InvalidValue {
                    field: "transaction_id",
                    source,
                },
            )?;
        let event_id = EventId::new(format!(
            "fixture-mainnet-{height}-{}-{}",
            self.transaction_index, self.event_index
        ))
        .map_err(|source| ScenarioBuildError::InvalidValue {
            field: "event_id",
            source,
        })?;
        let market_id =
            MarketId::new("perp:BTC").map_err(|source| ScenarioBuildError::InvalidValue {
                field: "market_id",
                source,
            })?;
        let price = Price::parse_at_scale("65000", 6).map_err(|source| {
            ScenarioBuildError::InvalidValue {
                field: "price",
                source,
            }
        })?;
        let quantity = Quantity::parse_at_scale("0.01", 8).map_err(|source| {
            ScenarioBuildError::InvalidValue {
                field: "quantity",
                source,
            }
        })?;

        CanonicalEventEnvelope::try_new(
            "1.0.0",
            "mainnet",
            self.block_height,
            block_time,
            transaction_id,
            self.transaction_index,
            self.event_index,
            event_id,
            vec![market_id],
            vec![buyer, seller],
            ConfirmationClass::CommittedPrimary,
            EventPayload::TradeMatched(TradeMatched::without_identities(
                price, quantity, self.seed,
            )),
            "fixture-parser-v1",
        )
        .map_err(ScenarioBuildError::from)
    }
}
