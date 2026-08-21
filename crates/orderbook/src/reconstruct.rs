use std::collections::BTreeMap;

use blake3::Hasher;
use domain_types::{BlockHeight, MarketId};

use crate::book::{BookDiff, BookHealth, DEFAULT_MAX_ORDERS, OrderBook, RestingOrder};
use crate::store::{L4_INPUT_HASH_CONTEXT, content_hash, encode_resting_order};

const DEFAULT_MAX_PROVISIONAL: usize = 4_096;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum L4Error {
    #[error("committed input {id} conflicts with a prior observation")]
    Conflict { id: String },
    #[error("provisional input refused: {0}")]
    ProvisionalRefused(&'static str),
    #[error("l4 book is not healthy: {0}")]
    Unhealthy(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct L4Reconstruction {
    book: OrderBook,
    committed: BTreeMap<String, [u8; 32]>,
    provisional: BTreeMap<String, [u8; 32]>,
    max_provisional: usize,
}

impl L4Reconstruction {
    pub fn awaiting_snapshot(market_id: MarketId, as_of_block: BlockHeight) -> Self {
        Self::awaiting_snapshot_bounded(
            market_id,
            as_of_block,
            DEFAULT_MAX_ORDERS,
            DEFAULT_MAX_PROVISIONAL,
        )
    }

    pub fn awaiting_snapshot_bounded(
        market_id: MarketId,
        as_of_block: BlockHeight,
        max_orders: usize,
        max_provisional: usize,
    ) -> Self {
        Self {
            book: OrderBook::awaiting_snapshot_bounded(market_id, as_of_block, max_orders),
            committed: BTreeMap::new(),
            provisional: BTreeMap::new(),
            max_provisional,
        }
    }

    #[must_use]
    pub const fn book(&self) -> &OrderBook {
        &self.book
    }

    #[must_use]
    pub fn provisional_count(&self) -> usize {
        self.provisional.len()
    }

    pub fn apply_committed_snapshot(
        &mut self,
        id: impl Into<String>,
        sequence: u64,
        as_of_block: BlockHeight,
        orders: Vec<RestingOrder>,
    ) -> Result<(), L4Error> {
        let id = id.into();
        let hash = snapshot_hash(sequence, as_of_block, &orders)?;
        if self.already_committed(&id, hash)? {
            return self.require_healthy();
        }
        self.book.apply_snapshot(sequence, as_of_block, orders);
        self.require_healthy()?;
        self.committed.insert(id, hash);
        Ok(())
    }

    pub fn apply_committed_diff(
        &mut self,
        id: impl Into<String>,
        sequence: u64,
        as_of_block: BlockHeight,
        diff: BookDiff,
    ) -> Result<(), L4Error> {
        let id = id.into();
        let hash = diff_hash(sequence, as_of_block, &diff)?;
        if self.already_committed(&id, hash)? {
            return self.require_healthy();
        }
        self.book.apply_diff(sequence, as_of_block, diff);
        self.require_healthy()?;
        self.committed.insert(id, hash);
        Ok(())
    }

    pub fn observe_provisional(
        &mut self,
        id: impl Into<String>,
        payload_hash: [u8; 32],
    ) -> Result<(), L4Error> {
        let id = id.into();
        if let Some(existing) = self.provisional.get(&id) {
            if *existing == payload_hash {
                return Ok(());
            }
            return Err(L4Error::Conflict { id });
        }
        if self.provisional.len() >= self.max_provisional {
            return Err(L4Error::ProvisionalRefused("unmatched provisional bound"));
        }
        self.provisional.insert(id, payload_hash);
        Ok(())
    }

    fn already_committed(&self, id: &str, hash: [u8; 32]) -> Result<bool, L4Error> {
        if let Some(existing) = self.committed.get(id) {
            if *existing == hash {
                return Ok(true);
            }
            return Err(L4Error::Conflict { id: id.to_owned() });
        }
        Ok(false)
    }

    fn require_healthy(&self) -> Result<(), L4Error> {
        match self.book.health() {
            BookHealth::Healthy => Ok(()),
            BookHealth::AwaitingSnapshot { reason } | BookHealth::Red { reason } => {
                Err(L4Error::Unhealthy(reason.clone()))
            }
        }
    }
}

fn snapshot_hash(
    sequence: u64,
    as_of_block: BlockHeight,
    orders: &[RestingOrder],
) -> Result<[u8; 32], L4Error> {
    let mut hasher = Hasher::new_derive_key(L4_INPUT_HASH_CONTEXT);
    hasher.update(&sequence.to_be_bytes());
    hasher.update(&as_of_block.get().to_be_bytes());
    hasher.update(
        &u64::try_from(orders.len())
            .unwrap_or(u64::MAX)
            .to_be_bytes(),
    );
    for order in orders {
        hasher.update(&canonical_order_bytes(order)?);
    }
    Ok(*hasher.finalize().as_bytes())
}

fn diff_hash(
    sequence: u64,
    as_of_block: BlockHeight,
    diff: &BookDiff,
) -> Result<[u8; 32], L4Error> {
    let mut material = sequence.to_be_bytes().to_vec();
    material.extend_from_slice(&as_of_block.get().to_be_bytes());
    match diff {
        BookDiff::Add { order } => {
            material.extend_from_slice(b"add");
            material.extend_from_slice(&canonical_order_bytes(order)?);
        }
        BookDiff::Update {
            order_id,
            remaining,
            price,
        } => {
            material.extend_from_slice(b"update");
            material.extend_from_slice(order_id.as_str().as_bytes());
            material.extend_from_slice(remaining.to_string().as_bytes());
            material.extend_from_slice(price.to_string().as_bytes());
        }
        BookDiff::Cancel { order_id } => {
            material.extend_from_slice(b"cancel");
            material.extend_from_slice(order_id.as_str().as_bytes());
        }
        BookDiff::Fill {
            order_id,
            fill_quantity,
        } => {
            material.extend_from_slice(b"fill");
            material.extend_from_slice(order_id.as_str().as_bytes());
            material.extend_from_slice(fill_quantity.to_string().as_bytes());
        }
    }
    Ok(content_hash(&material))
}

fn canonical_order_bytes(order: &RestingOrder) -> Result<Vec<u8>, L4Error> {
    encode_resting_order(order)
        .map_err(|_| L4Error::Unhealthy("committed input is not canonical".to_owned()))
}
