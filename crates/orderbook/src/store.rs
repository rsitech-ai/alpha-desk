use domain_types::{Address, BlockHeight, ClientOrderId, MarketId, OrderId, OrderSide};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::book::{L2Level, RestingOrder, TriggerKind};

// ponytail: CF names and length-prefixed keys for l4_orders/l2_book/checkpoints. RocksDB 11.1 stays out: deny.toml blocks librocksdb-sys (GPL-2.0) and crates.io rust-rocksdb wraps ~10.x. T18/T19 own the engine.
pub const CF_L4_ORDERS: &str = "l4_orders";
pub const CF_L2_BOOK: &str = "l2_book";
pub const CF_CHECKPOINTS: &str = "checkpoints";
pub const L4_STORE_SCHEMA: &str = "hyperliquid-alpha-desk/l4-rocksdb-encoding/v1";
pub const L4_CHECKPOINT_SCHEMA: &str = "hyperliquid-alpha-desk/l4-checkpoint/v1";
pub const L4_HASH_CONTEXT: &str = "hyperliquid-alpha-desk/l4-checkpoint-hash/v1";
pub const L4_INPUT_HASH_CONTEXT: &str = "hyperliquid-alpha-desk/l4-input-hash/v1";
pub const MAX_RECORD_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum L4StoreError {
    #[error("l4 record is not valid JSON")]
    Codec,
    #[error("l4 record is not canonical")]
    NonCanonical,
    #[error("l4 record is semantically invalid")]
    InvalidRecord,
    #[error("l4 record exceeds its deterministic bound")]
    LimitExceeded,
    #[error("l4 book is not healthy")]
    Unhealthy,
    #[error("l4 checkpoint hash mismatch")]
    HashMismatch,
}

pub fn l4_order_key(market_id: &MarketId, order_id: &OrderId) -> Vec<u8> {
    framed(&[market_id.as_str().as_bytes(), order_id.as_str().as_bytes()])
}

pub fn l2_book_key(market_id: &MarketId) -> Vec<u8> {
    framed(&[market_id.as_str().as_bytes()])
}

pub fn checkpoint_key(market_id: &MarketId, as_of_block: BlockHeight) -> Vec<u8> {
    framed(&[
        market_id.as_str().as_bytes(),
        &as_of_block.get().to_be_bytes(),
    ])
}

pub fn content_hash(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new_derive_key(L4_INPUT_HASH_CONTEXT);
    hasher.update(bytes);
    *hasher.finalize().as_bytes()
}

pub fn state_hash(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new_derive_key(L4_HASH_CONTEXT);
    hasher.update(bytes);
    *hasher.finalize().as_bytes()
}

pub fn encode_resting_order(order: &RestingOrder) -> Result<Vec<u8>, L4StoreError> {
    encode_canonical(&RestingWire::from_order(order)?)
}

pub fn decode_resting_order(bytes: &[u8]) -> Result<RestingOrder, L4StoreError> {
    let wire: RestingWire = decode_canonical(bytes)?;
    wire.into_order()
}

pub fn encode_l2_book(bids: &[L2Level], asks: &[L2Level]) -> Result<Vec<u8>, L4StoreError> {
    encode_canonical(&L2Wire {
        schema: L4_STORE_SCHEMA.to_owned(),
        bids: encode_levels(bids),
        asks: encode_levels(asks),
    })
}

pub fn decode_l2_book(bytes: &[u8]) -> Result<(Vec<L2Level>, Vec<L2Level>), L4StoreError> {
    let wire: L2Wire = decode_canonical(bytes)?;
    if wire.schema != L4_STORE_SCHEMA {
        return Err(L4StoreError::InvalidRecord);
    }
    Ok((decode_levels(&wire.bids)?, decode_levels(&wire.asks)?))
}

pub fn encode_canonical<T: Serialize>(value: &T) -> Result<Vec<u8>, L4StoreError> {
    let bytes = serde_json::to_vec(value).map_err(|_| L4StoreError::Codec)?;
    if bytes.len() > MAX_RECORD_BYTES {
        return Err(L4StoreError::LimitExceeded);
    }
    Ok(bytes)
}

pub fn decode_canonical<T>(bytes: &[u8]) -> Result<T, L4StoreError>
where
    T: DeserializeOwned + Serialize,
{
    if bytes.is_empty() || bytes.len() > MAX_RECORD_BYTES {
        return Err(L4StoreError::LimitExceeded);
    }
    let value: T = serde_json::from_slice(bytes).map_err(|_| L4StoreError::Codec)?;
    if encode_canonical(&value)? != bytes {
        return Err(L4StoreError::NonCanonical);
    }
    Ok(value)
}

fn framed(parts: &[&[u8]]) -> Vec<u8> {
    let mut key = Vec::new();
    for part in parts {
        key.extend_from_slice(&(part.len() as u64).to_be_bytes());
        key.extend_from_slice(part);
    }
    key
}

fn encode_levels(levels: &[L2Level]) -> Vec<LevelWire> {
    levels
        .iter()
        .map(|level| LevelWire {
            price: level.price.to_string(),
            quantity: level.quantity.to_string(),
            order_count: level.order_count,
        })
        .collect()
}

fn decode_levels(levels: &[LevelWire]) -> Result<Vec<L2Level>, L4StoreError> {
    levels
        .iter()
        .map(|level| {
            Ok(L2Level {
                price: level
                    .price
                    .parse()
                    .map_err(|_| L4StoreError::InvalidRecord)?,
                quantity: level
                    .quantity
                    .parse()
                    .map_err(|_| L4StoreError::InvalidRecord)?,
                order_count: level.order_count,
            })
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct RestingWire {
    schema: String,
    order_id: String,
    side: String,
    price: String,
    remaining: String,
    original: String,
    sequence: u64,
    account_id: Option<String>,
    client_order_id: Option<String>,
    time_millis: Option<u64>,
    trigger: TriggerWire,
}

impl RestingWire {
    fn from_order(order: &RestingOrder) -> Result<Self, L4StoreError> {
        Ok(Self {
            schema: L4_STORE_SCHEMA.to_owned(),
            order_id: order.order_id.as_str().to_owned(),
            side: order.side.as_wire_name().to_owned(),
            price: order.price.to_string(),
            remaining: order.remaining.to_string(),
            original: order.original.to_string(),
            sequence: order.sequence,
            account_id: order.account_id.map(Address::to_api_string),
            client_order_id: order
                .client_order_id
                .as_ref()
                .map(|value| value.as_str().to_owned()),
            time_millis: order.time_millis,
            trigger: TriggerWire::from_kind(&order.trigger),
        })
    }

    fn into_order(self) -> Result<RestingOrder, L4StoreError> {
        if self.schema != L4_STORE_SCHEMA {
            return Err(L4StoreError::InvalidRecord);
        }
        let mut order = RestingOrder::new(
            OrderId::new(&self.order_id).map_err(|_| L4StoreError::InvalidRecord)?,
            OrderSide::parse_wire(&self.side).map_err(|_| L4StoreError::InvalidRecord)?,
            self.price
                .parse()
                .map_err(|_| L4StoreError::InvalidRecord)?,
            self.remaining
                .parse()
                .map_err(|_| L4StoreError::InvalidRecord)?,
            self.sequence,
        )
        .with_original(
            self.original
                .parse()
                .map_err(|_| L4StoreError::InvalidRecord)?,
        )
        .with_trigger(self.trigger.into_kind()?);
        if let Some(account) = self.account_id {
            order = order.with_account(
                Address::parse_api(&account).map_err(|_| L4StoreError::InvalidRecord)?,
            );
        }
        if let Some(cloid) = self.client_order_id {
            order = order.with_client_order_id(
                ClientOrderId::new(cloid).map_err(|_| L4StoreError::InvalidRecord)?,
            );
        }
        if let Some(time_millis) = self.time_millis {
            order = order.with_time_millis(time_millis);
        }
        Ok(order)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
enum TriggerWire {
    None,
    Untriggered { tpsl: bool, trigger_px: String },
    Activated { trigger_px: String },
}

impl TriggerWire {
    fn from_kind(kind: &TriggerKind) -> Self {
        match kind {
            TriggerKind::None => Self::None,
            TriggerKind::Untriggered { tpsl, trigger_px } => Self::Untriggered {
                tpsl: *tpsl,
                trigger_px: trigger_px.to_string(),
            },
            TriggerKind::Activated { trigger_px } => Self::Activated {
                trigger_px: trigger_px.to_string(),
            },
        }
    }

    fn into_kind(self) -> Result<TriggerKind, L4StoreError> {
        match self {
            Self::None => Ok(TriggerKind::None),
            Self::Untriggered { tpsl, trigger_px } => Ok(TriggerKind::Untriggered {
                tpsl,
                trigger_px: trigger_px
                    .parse()
                    .map_err(|_| L4StoreError::InvalidRecord)?,
            }),
            Self::Activated { trigger_px } => Ok(TriggerKind::Activated {
                trigger_px: trigger_px
                    .parse()
                    .map_err(|_| L4StoreError::InvalidRecord)?,
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct L2Wire {
    schema: String,
    bids: Vec<LevelWire>,
    asks: Vec<LevelWire>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct LevelWire {
    price: String,
    quantity: String,
    order_count: u32,
}
