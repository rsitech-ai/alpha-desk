use domain_types::{Address, BlockHeight, EventId, VaultId};
use serde::{Deserialize, Serialize};

use crate::StateKey;

use super::{
    AccountStateError,
    codec::{decode_wire, encode_wire, require_record_bytes, state_key},
};

const SUBACCOUNT_NAMESPACE: &str = "account-subaccount-master.v1";
const VAULT_RELATION_NAMESPACE: &str = "account-vault-relation.v1";
const SUBACCOUNT_SCHEMA: &str = "hyperliquid-alpha-desk/account-subaccount-master/v1";
const VAULT_RELATION_SCHEMA: &str = "hyperliquid-alpha-desk/account-vault-relation/v1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubaccountMasterCurrentRecordV1 {
    pub(super) subaccount_id: Address,
    pub(super) master_account_id: Address,
    pub(super) first_event_id: EventId,
    pub(super) last_event_id: EventId,
    pub(super) first_block_height: BlockHeight,
    pub(super) last_block_height: BlockHeight,
}

impl SubaccountMasterCurrentRecordV1 {
    pub fn state_key(subaccount_id: &Address) -> Result<StateKey, AccountStateError> {
        state_key(SUBACCOUNT_NAMESPACE, &[subaccount_id.as_bytes()])
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, AccountStateError> {
        let wire: SubaccountWire = decode_wire(bytes)?;
        if wire.schema != SUBACCOUNT_SCHEMA {
            return Err(AccountStateError::InvalidRecord);
        }
        let record = Self {
            subaccount_id: Address::parse_api(&wire.subaccount_id)
                .map_err(|_| AccountStateError::InvalidRecord)?,
            master_account_id: Address::parse_api(&wire.master_account_id)
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
        if Self::state_key(&record.subaccount_id)? == *key {
            Ok(record)
        } else {
            Err(AccountStateError::KeyMismatch)
        }
    }

    pub(super) fn encode(&self) -> Result<Vec<u8>, AccountStateError> {
        self.validate()?;
        encode_wire(&SubaccountWire {
            schema: SUBACCOUNT_SCHEMA.to_owned(),
            subaccount_id: self.subaccount_id.to_api_string(),
            master_account_id: self.master_account_id.to_api_string(),
            first_event_id: self.first_event_id.as_str().to_owned(),
            last_event_id: self.last_event_id.as_str().to_owned(),
            first_block_height: self.first_block_height.get(),
            last_block_height: self.last_block_height.get(),
        })
    }

    fn validate(&self) -> Result<(), AccountStateError> {
        if self.subaccount_id != self.master_account_id
            && self.first_block_height <= self.last_block_height
        {
            Ok(())
        } else {
            Err(AccountStateError::InvalidRecord)
        }
    }

    #[must_use]
    pub const fn subaccount_id(&self) -> Address {
        self.subaccount_id
    }

    #[must_use]
    pub const fn master_account_id(&self) -> Address {
        self.master_account_id
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
pub struct AccountVaultRelationCurrentRecordV1 {
    pub(super) account_id: Address,
    pub(super) vault_id: VaultId,
    pub(super) first_event_id: EventId,
    pub(super) last_event_id: EventId,
    pub(super) first_block_height: BlockHeight,
    pub(super) last_block_height: BlockHeight,
}

impl AccountVaultRelationCurrentRecordV1 {
    pub fn state_key(
        account_id: &Address,
        vault_id: &VaultId,
    ) -> Result<StateKey, AccountStateError> {
        state_key(
            VAULT_RELATION_NAMESPACE,
            &[account_id.as_bytes(), vault_id.as_str().as_bytes()],
        )
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, AccountStateError> {
        let wire: AccountVaultWire = decode_wire(bytes)?;
        if wire.schema != VAULT_RELATION_SCHEMA {
            return Err(AccountStateError::InvalidRecord);
        }
        let record = Self {
            account_id: Address::parse_api(&wire.account_id)
                .map_err(|_| AccountStateError::InvalidRecord)?,
            vault_id: VaultId::new(wire.vault_id).map_err(|_| AccountStateError::InvalidRecord)?,
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
        if Self::state_key(&record.account_id, &record.vault_id)? == *key {
            Ok(record)
        } else {
            Err(AccountStateError::KeyMismatch)
        }
    }

    pub(super) fn encode(&self) -> Result<Vec<u8>, AccountStateError> {
        self.validate()?;
        encode_wire(&AccountVaultWire {
            schema: VAULT_RELATION_SCHEMA.to_owned(),
            account_id: self.account_id.to_api_string(),
            vault_id: self.vault_id.as_str().to_owned(),
            first_event_id: self.first_event_id.as_str().to_owned(),
            last_event_id: self.last_event_id.as_str().to_owned(),
            first_block_height: self.first_block_height.get(),
            last_block_height: self.last_block_height.get(),
        })
    }

    fn validate(&self) -> Result<(), AccountStateError> {
        let same_endpoint = Address::parse_api(self.vault_id.as_str())
            .is_ok_and(|vault_address| vault_address == self.account_id);
        if !same_endpoint && self.first_block_height <= self.last_block_height {
            Ok(())
        } else {
            Err(AccountStateError::InvalidRecord)
        }
    }

    #[must_use]
    pub const fn account_id(&self) -> Address {
        self.account_id
    }

    #[must_use]
    pub const fn vault_id(&self) -> &VaultId {
        &self.vault_id
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
struct SubaccountWire {
    schema: String,
    subaccount_id: String,
    master_account_id: String,
    first_event_id: String,
    last_event_id: String,
    first_block_height: u64,
    last_block_height: u64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AccountVaultWire {
    schema: String,
    account_id: String,
    vault_id: String,
    first_event_id: String,
    last_event_id: String,
    first_block_height: u64,
    last_block_height: u64,
}
