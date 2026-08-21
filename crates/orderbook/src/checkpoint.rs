use domain_types::{BlockHeight, MarketId};
use serde::{Deserialize, Serialize};

use crate::book::{BookHealth, OrderBook, RestingOrder};
use crate::store::{
    L4_CHECKPOINT_SCHEMA, L4StoreError, MAX_RECORD_BYTES, decode_canonical, decode_resting_order,
    encode_canonical, encode_resting_order, state_hash,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct L4CheckpointV1 {
    market_id: MarketId,
    sequence: u64,
    as_of_block: BlockHeight,
    max_orders: usize,
    orders: Vec<RestingOrder>,
    state_hash: [u8; 32],
}

impl L4CheckpointV1 {
    pub fn capture(book: &OrderBook) -> Result<Self, L4StoreError> {
        if !matches!(book.health(), BookHealth::Healthy) {
            return Err(L4StoreError::Unhealthy);
        }
        let mut orders: Vec<RestingOrder> = book.active_orders().cloned().collect();
        orders.sort_by(|left, right| left.order_id.as_str().cmp(right.order_id.as_str()));
        let mut checkpoint = Self {
            market_id: book.market_id().clone(),
            sequence: book.sequence(),
            as_of_block: book.as_of_block(),
            max_orders: book.max_orders(),
            orders,
            state_hash: [0; 32],
        };
        let bytes = checkpoint.canonical_bytes()?;
        checkpoint.state_hash = state_hash(&bytes);
        Ok(checkpoint)
    }

    pub fn restore(&self) -> Result<OrderBook, L4StoreError> {
        if self.market_id.as_str().is_empty() {
            return Err(L4StoreError::InvalidRecord);
        }
        let mut book = OrderBook::awaiting_snapshot_bounded(
            self.market_id.clone(),
            self.as_of_block,
            self.max_orders,
        );
        book.apply_snapshot(self.sequence, self.as_of_block, self.orders.clone());
        if !matches!(book.health(), BookHealth::Healthy) {
            return Err(L4StoreError::Unhealthy);
        }
        let restored = Self::capture(&book)?;
        if restored.state_hash != self.state_hash {
            return Err(L4StoreError::HashMismatch);
        }
        Ok(book)
    }

    #[must_use]
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    #[must_use]
    pub const fn as_of_block(&self) -> BlockHeight {
        self.as_of_block
    }

    #[must_use]
    pub const fn state_hash(&self) -> [u8; 32] {
        self.state_hash
    }

    #[must_use]
    pub fn orders(&self) -> &[RestingOrder] {
        &self.orders
    }

    pub fn encode(&self) -> Result<Vec<u8>, L4StoreError> {
        let bytes = encode_canonical(&self.wire()?)?;
        if bytes.len() > MAX_RECORD_BYTES {
            return Err(L4StoreError::LimitExceeded);
        }
        Ok(bytes)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, L4StoreError> {
        let wire: CheckpointWire = decode_canonical(bytes)?;
        if wire.schema != L4_CHECKPOINT_SCHEMA {
            return Err(L4StoreError::InvalidRecord);
        }
        let market_id = MarketId::new(&wire.market_id).map_err(|_| L4StoreError::InvalidRecord)?;
        let mut orders = Vec::with_capacity(wire.orders.len());
        for encoded in &wire.orders {
            orders.push(decode_resting_order(encoded)?);
        }
        let mut hash = [0_u8; 32];
        hex::decode_to_slice(&wire.state_hash, &mut hash)
            .map_err(|_| L4StoreError::InvalidRecord)?;
        let checkpoint = Self {
            market_id,
            sequence: wire.sequence,
            as_of_block: BlockHeight::new(wire.as_of_block),
            max_orders: wire.max_orders,
            orders,
            state_hash: hash,
        };
        let expected = state_hash(&checkpoint.canonical_bytes()?);
        if expected != hash {
            return Err(L4StoreError::HashMismatch);
        }
        Ok(checkpoint)
    }

    fn canonical_bytes(&self) -> Result<Vec<u8>, L4StoreError> {
        let mut wire = self.wire()?;
        wire.state_hash = String::new();
        encode_canonical(&wire)
    }

    fn wire(&self) -> Result<CheckpointWire, L4StoreError> {
        let mut orders = Vec::with_capacity(self.orders.len());
        for order in &self.orders {
            orders.push(encode_resting_order(order)?);
        }
        Ok(CheckpointWire {
            schema: L4_CHECKPOINT_SCHEMA.to_owned(),
            market_id: self.market_id.as_str().to_owned(),
            sequence: self.sequence,
            as_of_block: self.as_of_block.get(),
            max_orders: self.max_orders,
            orders,
            state_hash: hex::encode(self.state_hash),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct CheckpointWire {
    schema: String,
    market_id: String,
    sequence: u64,
    as_of_block: u64,
    max_orders: usize,
    orders: Vec<Vec<u8>>,
    state_hash: String,
}
