use std::str::FromStr;

use domain_types::{
    AccountAbstractionModeV1, Address, BlockHeight, EventId, Leverage, MarginModeV1, MarketId,
};
use serde::{Deserialize, Serialize};

use crate::StateKey;

use super::{
    AccountStateError,
    codec::{decode_wire, encode_wire, require_record_bytes, state_key},
};

const ACCOUNT_MODE_NAMESPACE: &str = "account-mode-current.v1";
const MARGIN_MODE_NAMESPACE: &str = "account-margin-mode-current.v1";
const LEVERAGE_NAMESPACE: &str = "account-leverage-current.v1";
const ACCOUNT_MODE_SCHEMA: &str = "hyperliquid-alpha-desk/account-mode-current/v1";
const MARGIN_MODE_SCHEMA: &str = "hyperliquid-alpha-desk/account-margin-mode-current/v1";
const LEVERAGE_SCHEMA: &str = "hyperliquid-alpha-desk/account-leverage-current/v1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountModeCurrentRecordV1 {
    pub(super) account_id: Address,
    pub(super) initial_previous: AccountAbstractionModeV1,
    pub(super) current: AccountAbstractionModeV1,
    pub(super) first_event_id: EventId,
    pub(super) last_event_id: EventId,
    pub(super) first_block_height: BlockHeight,
    pub(super) last_block_height: BlockHeight,
}

impl AccountModeCurrentRecordV1 {
    pub fn state_key(account_id: &Address) -> Result<StateKey, AccountStateError> {
        state_key(ACCOUNT_MODE_NAMESPACE, &[account_id.as_bytes()])
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, AccountStateError> {
        let wire: AccountModeWire = decode_wire(bytes)?;
        if wire.schema != ACCOUNT_MODE_SCHEMA {
            return Err(AccountStateError::InvalidRecord);
        }
        let record = Self {
            account_id: Address::parse_api(&wire.account_id)
                .map_err(|_| AccountStateError::InvalidRecord)?,
            initial_previous: AccountAbstractionModeV1::parse_wire(&wire.initial_previous)
                .map_err(|_| AccountStateError::InvalidRecord)?,
            current: AccountAbstractionModeV1::parse_wire(&wire.current)
                .map_err(|_| AccountStateError::InvalidRecord)?,
            first_event_id: EventId::new(wire.first_event_id)
                .map_err(|_| AccountStateError::InvalidRecord)?,
            last_event_id: EventId::new(wire.last_event_id)
                .map_err(|_| AccountStateError::InvalidRecord)?,
            first_block_height: BlockHeight::new(wire.first_block_height),
            last_block_height: BlockHeight::new(wire.last_block_height),
        };
        record.validate()?;
        require_record_bytes(&record.encode()?, bytes)?;
        Ok(record)
    }

    pub fn decode_at(key: &StateKey, bytes: &[u8]) -> Result<Self, AccountStateError> {
        let record = Self::decode(bytes)?;
        if Self::state_key(&record.account_id)? == *key {
            Ok(record)
        } else {
            Err(AccountStateError::KeyMismatch)
        }
    }

    pub(super) fn encode(&self) -> Result<Vec<u8>, AccountStateError> {
        self.validate()?;
        encode_wire(&AccountModeWire {
            schema: ACCOUNT_MODE_SCHEMA.to_owned(),
            account_id: self.account_id.to_api_string(),
            initial_previous: self.initial_previous.as_wire_name().to_owned(),
            current: self.current.as_wire_name().to_owned(),
            first_event_id: self.first_event_id.as_str().to_owned(),
            last_event_id: self.last_event_id.as_str().to_owned(),
            first_block_height: self.first_block_height.get(),
            last_block_height: self.last_block_height.get(),
        })
    }

    fn validate(&self) -> Result<(), AccountStateError> {
        validate_heights(self.first_block_height, self.last_block_height)
    }

    #[must_use]
    pub const fn account_id(&self) -> Address {
        self.account_id
    }

    #[must_use]
    pub const fn initial_previous(&self) -> AccountAbstractionModeV1 {
        self.initial_previous
    }

    #[must_use]
    pub const fn current(&self) -> AccountAbstractionModeV1 {
        self.current
    }

    #[must_use]
    pub const fn first_event_id(&self) -> &EventId {
        &self.first_event_id
    }

    #[must_use]
    pub const fn last_event_id(&self) -> &EventId {
        &self.last_event_id
    }

    #[must_use]
    pub const fn first_block_height(&self) -> BlockHeight {
        self.first_block_height
    }

    #[must_use]
    pub const fn last_block_height(&self) -> BlockHeight {
        self.last_block_height
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarginModeCurrentRecordV1 {
    pub(super) account_id: Address,
    pub(super) market_id: MarketId,
    pub(super) initial_previous: MarginModeV1,
    pub(super) current: MarginModeV1,
    pub(super) first_event_id: EventId,
    pub(super) last_event_id: EventId,
    pub(super) first_block_height: BlockHeight,
    pub(super) last_block_height: BlockHeight,
}

impl MarginModeCurrentRecordV1 {
    pub fn state_key(
        account_id: &Address,
        market_id: &MarketId,
    ) -> Result<StateKey, AccountStateError> {
        state_key(
            MARGIN_MODE_NAMESPACE,
            &[account_id.as_bytes(), market_id.as_str().as_bytes()],
        )
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, AccountStateError> {
        let wire: MarginModeWire = decode_wire(bytes)?;
        if wire.schema != MARGIN_MODE_SCHEMA {
            return Err(AccountStateError::InvalidRecord);
        }
        let record = Self {
            account_id: Address::parse_api(&wire.account_id)
                .map_err(|_| AccountStateError::InvalidRecord)?,
            market_id: MarketId::new(wire.market_id)
                .map_err(|_| AccountStateError::InvalidRecord)?,
            initial_previous: MarginModeV1::parse_wire(&wire.initial_previous)
                .map_err(|_| AccountStateError::InvalidRecord)?,
            current: MarginModeV1::parse_wire(&wire.current)
                .map_err(|_| AccountStateError::InvalidRecord)?,
            first_event_id: EventId::new(wire.first_event_id)
                .map_err(|_| AccountStateError::InvalidRecord)?,
            last_event_id: EventId::new(wire.last_event_id)
                .map_err(|_| AccountStateError::InvalidRecord)?,
            first_block_height: BlockHeight::new(wire.first_block_height),
            last_block_height: BlockHeight::new(wire.last_block_height),
        };
        record.validate()?;
        require_record_bytes(&record.encode()?, bytes)?;
        Ok(record)
    }

    pub fn decode_at(key: &StateKey, bytes: &[u8]) -> Result<Self, AccountStateError> {
        let record = Self::decode(bytes)?;
        if Self::state_key(&record.account_id, &record.market_id)? == *key {
            Ok(record)
        } else {
            Err(AccountStateError::KeyMismatch)
        }
    }

    pub(super) fn encode(&self) -> Result<Vec<u8>, AccountStateError> {
        self.validate()?;
        encode_wire(&MarginModeWire {
            schema: MARGIN_MODE_SCHEMA.to_owned(),
            account_id: self.account_id.to_api_string(),
            market_id: self.market_id.as_str().to_owned(),
            initial_previous: self.initial_previous.as_wire_name().to_owned(),
            current: self.current.as_wire_name().to_owned(),
            first_event_id: self.first_event_id.as_str().to_owned(),
            last_event_id: self.last_event_id.as_str().to_owned(),
            first_block_height: self.first_block_height.get(),
            last_block_height: self.last_block_height.get(),
        })
    }

    fn validate(&self) -> Result<(), AccountStateError> {
        validate_heights(self.first_block_height, self.last_block_height)
    }

    #[must_use]
    pub const fn account_id(&self) -> Address {
        self.account_id
    }

    #[must_use]
    pub const fn market_id(&self) -> &MarketId {
        &self.market_id
    }

    #[must_use]
    pub const fn initial_previous(&self) -> MarginModeV1 {
        self.initial_previous
    }

    #[must_use]
    pub const fn current(&self) -> MarginModeV1 {
        self.current
    }

    #[must_use]
    pub const fn first_event_id(&self) -> &EventId {
        &self.first_event_id
    }

    #[must_use]
    pub const fn last_event_id(&self) -> &EventId {
        &self.last_event_id
    }

    #[must_use]
    pub const fn first_block_height(&self) -> BlockHeight {
        self.first_block_height
    }

    #[must_use]
    pub const fn last_block_height(&self) -> BlockHeight {
        self.last_block_height
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeverageCurrentRecordV1 {
    pub(super) account_id: Address,
    pub(super) market_id: MarketId,
    pub(super) initial_previous: Leverage,
    pub(super) current: Leverage,
    pub(super) first_event_id: EventId,
    pub(super) last_event_id: EventId,
    pub(super) first_block_height: BlockHeight,
    pub(super) last_block_height: BlockHeight,
}

impl LeverageCurrentRecordV1 {
    pub fn state_key(
        account_id: &Address,
        market_id: &MarketId,
    ) -> Result<StateKey, AccountStateError> {
        state_key(
            LEVERAGE_NAMESPACE,
            &[account_id.as_bytes(), market_id.as_str().as_bytes()],
        )
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, AccountStateError> {
        let wire: LeverageWire = decode_wire(bytes)?;
        if wire.schema != LEVERAGE_SCHEMA {
            return Err(AccountStateError::InvalidRecord);
        }
        let record = Self {
            account_id: Address::parse_api(&wire.account_id)
                .map_err(|_| AccountStateError::InvalidRecord)?,
            market_id: MarketId::new(wire.market_id)
                .map_err(|_| AccountStateError::InvalidRecord)?,
            initial_previous: Leverage::from_str(&wire.initial_previous)
                .map_err(|_| AccountStateError::InvalidRecord)?,
            current: Leverage::from_str(&wire.current)
                .map_err(|_| AccountStateError::InvalidRecord)?,
            first_event_id: EventId::new(wire.first_event_id)
                .map_err(|_| AccountStateError::InvalidRecord)?,
            last_event_id: EventId::new(wire.last_event_id)
                .map_err(|_| AccountStateError::InvalidRecord)?,
            first_block_height: BlockHeight::new(wire.first_block_height),
            last_block_height: BlockHeight::new(wire.last_block_height),
        };
        record.validate()?;
        require_record_bytes(&record.encode()?, bytes)?;
        Ok(record)
    }

    pub fn decode_at(key: &StateKey, bytes: &[u8]) -> Result<Self, AccountStateError> {
        let record = Self::decode(bytes)?;
        if Self::state_key(&record.account_id, &record.market_id)? == *key {
            Ok(record)
        } else {
            Err(AccountStateError::KeyMismatch)
        }
    }

    pub(super) fn encode(&self) -> Result<Vec<u8>, AccountStateError> {
        self.validate()?;
        encode_wire(&LeverageWire {
            schema: LEVERAGE_SCHEMA.to_owned(),
            account_id: self.account_id.to_api_string(),
            market_id: self.market_id.as_str().to_owned(),
            initial_previous: self.initial_previous.to_string(),
            current: self.current.to_string(),
            first_event_id: self.first_event_id.as_str().to_owned(),
            last_event_id: self.last_event_id.as_str().to_owned(),
            first_block_height: self.first_block_height.get(),
            last_block_height: self.last_block_height.get(),
        })
    }

    fn validate(&self) -> Result<(), AccountStateError> {
        if self.initial_previous.raw() > 0 && self.current.raw() > 0 {
            validate_heights(self.first_block_height, self.last_block_height)
        } else {
            Err(AccountStateError::InvalidRecord)
        }
    }

    #[must_use]
    pub const fn account_id(&self) -> Address {
        self.account_id
    }

    #[must_use]
    pub const fn market_id(&self) -> &MarketId {
        &self.market_id
    }

    #[must_use]
    pub const fn initial_previous(&self) -> Leverage {
        self.initial_previous
    }

    #[must_use]
    pub const fn current(&self) -> Leverage {
        self.current
    }

    #[must_use]
    pub const fn first_event_id(&self) -> &EventId {
        &self.first_event_id
    }

    #[must_use]
    pub const fn last_event_id(&self) -> &EventId {
        &self.last_event_id
    }

    #[must_use]
    pub const fn first_block_height(&self) -> BlockHeight {
        self.first_block_height
    }

    #[must_use]
    pub const fn last_block_height(&self) -> BlockHeight {
        self.last_block_height
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AccountModeWire {
    schema: String,
    account_id: String,
    initial_previous: String,
    current: String,
    first_event_id: String,
    last_event_id: String,
    first_block_height: u64,
    last_block_height: u64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct MarginModeWire {
    schema: String,
    account_id: String,
    market_id: String,
    initial_previous: String,
    current: String,
    first_event_id: String,
    last_event_id: String,
    first_block_height: u64,
    last_block_height: u64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LeverageWire {
    schema: String,
    account_id: String,
    market_id: String,
    initial_previous: String,
    current: String,
    first_event_id: String,
    last_event_id: String,
    first_block_height: u64,
    last_block_height: u64,
}

const fn validate_heights(first: BlockHeight, last: BlockHeight) -> Result<(), AccountStateError> {
    if first.get() <= last.get() {
        Ok(())
    } else {
        Err(AccountStateError::InvalidRecord)
    }
}
