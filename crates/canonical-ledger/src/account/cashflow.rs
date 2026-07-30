use std::str::FromStr;

use domain_types::{
    Address, AssetId, BlockHeight, EventId, MarketId, Quantity, QuoteAmount, VaultId,
};
use serde::{Deserialize, Serialize};

use crate::StateKey;

use super::{
    AccountStateError,
    codec::{decode_wire, encode_wire, require_record_bytes, state_key},
};

const QUANTITY_NAMESPACE: &str = "account-quantity-flow-current.v1";
const QUOTE_NAMESPACE: &str = "account-quote-flow-current.v1";
const VAULT_PRINCIPAL_NAMESPACE: &str = "vault-principal-flow-current.v1";
const VAULT_SHARE_NAMESPACE: &str = "vault-share-flow-current.v1";

const QUANTITY_SCHEMA: &str = "hyperliquid-alpha-desk/account-quantity-flow-current/v1";
const QUOTE_SCHEMA: &str = "hyperliquid-alpha-desk/account-quote-flow-current/v1";
const VAULT_PRINCIPAL_SCHEMA: &str = "hyperliquid-alpha-desk/vault-principal-flow-current/v1";
const VAULT_SHARE_SCHEMA: &str = "hyperliquid-alpha-desk/vault-share-flow-current/v1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountQuantityFlowScopeV1 {
    ExternalAsset { asset_id: AssetId },
    SpotTransferAsset { asset_id: AssetId },
    SubaccountTransferAsset { asset_id: AssetId },
    FeeAsset { asset_id: AssetId },
    BuilderFeeAsset { asset_id: AssetId },
    ReferralRewardAsset { asset_id: AssetId },
    VaultShares { vault_id: VaultId },
}

impl AccountQuantityFlowScopeV1 {
    const fn as_wire_name(&self) -> &'static str {
        match self {
            Self::ExternalAsset { .. } => "external_asset",
            Self::SpotTransferAsset { .. } => "spot_transfer_asset",
            Self::SubaccountTransferAsset { .. } => "subaccount_transfer_asset",
            Self::FeeAsset { .. } => "fee_asset",
            Self::BuilderFeeAsset { .. } => "builder_fee_asset",
            Self::ReferralRewardAsset { .. } => "referral_reward_asset",
            Self::VaultShares { .. } => "vault_shares",
        }
    }

    fn parse(
        scope: &str,
        asset_id: Option<String>,
        vault_id: Option<String>,
    ) -> Result<Self, AccountStateError> {
        let asset = || {
            asset_id
                .clone()
                .ok_or(AccountStateError::InvalidRecord)
                .and_then(|value| AssetId::new(value).map_err(|_| AccountStateError::InvalidRecord))
        };
        match (scope, vault_id) {
            ("external_asset", None) => Ok(Self::ExternalAsset { asset_id: asset()? }),
            ("spot_transfer_asset", None) => Ok(Self::SpotTransferAsset { asset_id: asset()? }),
            ("subaccount_transfer_asset", None) => {
                Ok(Self::SubaccountTransferAsset { asset_id: asset()? })
            }
            ("fee_asset", None) => Ok(Self::FeeAsset { asset_id: asset()? }),
            ("builder_fee_asset", None) => Ok(Self::BuilderFeeAsset { asset_id: asset()? }),
            ("referral_reward_asset", None) => Ok(Self::ReferralRewardAsset { asset_id: asset()? }),
            ("vault_shares", Some(vault_id)) if asset_id.is_none() => Ok(Self::VaultShares {
                vault_id: VaultId::new(vault_id).map_err(|_| AccountStateError::InvalidRecord)?,
            }),
            _ => Err(AccountStateError::InvalidRecord),
        }
    }

    fn identity(&self) -> &[u8] {
        match self {
            Self::ExternalAsset { asset_id }
            | Self::SpotTransferAsset { asset_id }
            | Self::SubaccountTransferAsset { asset_id }
            | Self::FeeAsset { asset_id }
            | Self::BuilderFeeAsset { asset_id }
            | Self::ReferralRewardAsset { asset_id } => asset_id.as_str().as_bytes(),
            Self::VaultShares { vault_id } => vault_id.as_str().as_bytes(),
        }
    }

    fn asset_id(&self) -> Option<String> {
        match self {
            Self::ExternalAsset { asset_id }
            | Self::SpotTransferAsset { asset_id }
            | Self::SubaccountTransferAsset { asset_id }
            | Self::FeeAsset { asset_id }
            | Self::BuilderFeeAsset { asset_id }
            | Self::ReferralRewardAsset { asset_id } => Some(asset_id.as_str().to_owned()),
            Self::VaultShares { .. } => None,
        }
    }

    fn vault_id(&self) -> Option<String> {
        match self {
            Self::VaultShares { vault_id } => Some(vault_id.as_str().to_owned()),
            Self::ExternalAsset { .. }
            | Self::SpotTransferAsset { .. }
            | Self::SubaccountTransferAsset { .. }
            | Self::FeeAsset { .. }
            | Self::BuilderFeeAsset { .. }
            | Self::ReferralRewardAsset { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountQuantityFlowCurrentRecordV1 {
    pub(super) account_id: Address,
    pub(super) scope: AccountQuantityFlowScopeV1,
    pub(super) credits: Quantity,
    pub(super) debits: Quantity,
    pub(super) last_event_id: EventId,
    pub(super) last_block_height: BlockHeight,
}

impl AccountQuantityFlowCurrentRecordV1 {
    pub fn state_key(
        account_id: &Address,
        scope: &AccountQuantityFlowScopeV1,
    ) -> Result<StateKey, AccountStateError> {
        state_key(
            QUANTITY_NAMESPACE,
            &[
                account_id.as_bytes(),
                scope.as_wire_name().as_bytes(),
                scope.identity(),
            ],
        )
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, AccountStateError> {
        let wire: QuantityFlowWire = decode_wire(bytes)?;
        if wire.schema != QUANTITY_SCHEMA {
            return Err(AccountStateError::InvalidRecord);
        }
        let record = Self {
            account_id: Address::parse_api(&wire.account_id)
                .map_err(|_| AccountStateError::InvalidRecord)?,
            scope: AccountQuantityFlowScopeV1::parse(&wire.scope, wire.asset_id, wire.vault_id)?,
            credits: Quantity::from_str(&wire.credits)
                .map_err(|_| AccountStateError::InvalidRecord)?,
            debits: Quantity::from_str(&wire.debits)
                .map_err(|_| AccountStateError::InvalidRecord)?,
            last_event_id: EventId::new(wire.last_event_id)
                .map_err(|_| AccountStateError::InvalidRecord)?,
            last_block_height: BlockHeight::new(wire.last_block_height),
        };
        record.validate()?;
        require_record_bytes(&record.encode()?, bytes)?;
        Ok(record)
    }

    pub fn decode_at(key: &StateKey, bytes: &[u8]) -> Result<Self, AccountStateError> {
        let record = Self::decode(bytes)?;
        if Self::state_key(&record.account_id, &record.scope)? == *key {
            Ok(record)
        } else {
            Err(AccountStateError::KeyMismatch)
        }
    }

    pub(super) fn encode(&self) -> Result<Vec<u8>, AccountStateError> {
        self.validate()?;
        encode_wire(&QuantityFlowWire {
            schema: QUANTITY_SCHEMA.to_owned(),
            account_id: self.account_id.to_api_string(),
            scope: self.scope.as_wire_name().to_owned(),
            asset_id: self.scope.asset_id(),
            vault_id: self.scope.vault_id(),
            credits: self.credits.to_string(),
            debits: self.debits.to_string(),
            last_event_id: self.last_event_id.as_str().to_owned(),
            last_block_height: self.last_block_height.get(),
        })
    }

    fn validate(&self) -> Result<(), AccountStateError> {
        if valid_quantity_totals(self.credits, self.debits) {
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
    pub const fn scope(&self) -> &AccountQuantityFlowScopeV1 {
        &self.scope
    }

    #[must_use]
    pub const fn credits(&self) -> Quantity {
        self.credits
    }

    #[must_use]
    pub const fn debits(&self) -> Quantity {
        self.debits
    }

    #[must_use]
    pub const fn last_event_id(&self) -> &EventId {
        &self.last_event_id
    }

    #[must_use]
    pub const fn last_block_height(&self) -> BlockHeight {
        self.last_block_height
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountQuoteFlowScopeV1 {
    DefaultPerpQuote,
    MarketFunding { market_id: MarketId },
    VaultPrincipal { vault_id: VaultId },
}

impl AccountQuoteFlowScopeV1 {
    const fn as_wire_name(&self) -> &'static str {
        match self {
            Self::DefaultPerpQuote => "default_perp_quote",
            Self::MarketFunding { .. } => "market_funding",
            Self::VaultPrincipal { .. } => "vault_principal",
        }
    }

    fn parse(
        scope: &str,
        market_id: Option<String>,
        vault_id: Option<String>,
    ) -> Result<Self, AccountStateError> {
        match (scope, market_id, vault_id) {
            ("default_perp_quote", None, None) => Ok(Self::DefaultPerpQuote),
            ("market_funding", Some(market_id), None) => Ok(Self::MarketFunding {
                market_id: MarketId::new(market_id)
                    .map_err(|_| AccountStateError::InvalidRecord)?,
            }),
            ("vault_principal", None, Some(vault_id)) => Ok(Self::VaultPrincipal {
                vault_id: VaultId::new(vault_id).map_err(|_| AccountStateError::InvalidRecord)?,
            }),
            _ => Err(AccountStateError::InvalidRecord),
        }
    }

    fn identity(&self) -> Option<&[u8]> {
        match self {
            Self::DefaultPerpQuote => None,
            Self::MarketFunding { market_id } => Some(market_id.as_str().as_bytes()),
            Self::VaultPrincipal { vault_id } => Some(vault_id.as_str().as_bytes()),
        }
    }

    fn market_id(&self) -> Option<String> {
        match self {
            Self::MarketFunding { market_id } => Some(market_id.as_str().to_owned()),
            Self::DefaultPerpQuote | Self::VaultPrincipal { .. } => None,
        }
    }

    fn vault_id(&self) -> Option<String> {
        match self {
            Self::VaultPrincipal { vault_id } => Some(vault_id.as_str().to_owned()),
            Self::DefaultPerpQuote | Self::MarketFunding { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountQuoteFlowCurrentRecordV1 {
    pub(super) account_id: Address,
    pub(super) scope: AccountQuoteFlowScopeV1,
    pub(super) credits: QuoteAmount,
    pub(super) debits: QuoteAmount,
    pub(super) last_event_id: EventId,
    pub(super) last_block_height: BlockHeight,
}

impl AccountQuoteFlowCurrentRecordV1 {
    pub fn state_key(
        account_id: &Address,
        scope: &AccountQuoteFlowScopeV1,
    ) -> Result<StateKey, AccountStateError> {
        if let Some(identity) = scope.identity() {
            state_key(
                QUOTE_NAMESPACE,
                &[
                    account_id.as_bytes(),
                    scope.as_wire_name().as_bytes(),
                    identity,
                ],
            )
        } else {
            state_key(
                QUOTE_NAMESPACE,
                &[account_id.as_bytes(), scope.as_wire_name().as_bytes()],
            )
        }
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, AccountStateError> {
        let wire: QuoteFlowWire = decode_wire(bytes)?;
        if wire.schema != QUOTE_SCHEMA {
            return Err(AccountStateError::InvalidRecord);
        }
        let record = Self {
            account_id: Address::parse_api(&wire.account_id)
                .map_err(|_| AccountStateError::InvalidRecord)?,
            scope: AccountQuoteFlowScopeV1::parse(&wire.scope, wire.market_id, wire.vault_id)?,
            credits: QuoteAmount::from_str(&wire.credits)
                .map_err(|_| AccountStateError::InvalidRecord)?,
            debits: QuoteAmount::from_str(&wire.debits)
                .map_err(|_| AccountStateError::InvalidRecord)?,
            last_event_id: EventId::new(wire.last_event_id)
                .map_err(|_| AccountStateError::InvalidRecord)?,
            last_block_height: BlockHeight::new(wire.last_block_height),
        };
        record.validate()?;
        require_record_bytes(&record.encode()?, bytes)?;
        Ok(record)
    }

    pub fn decode_at(key: &StateKey, bytes: &[u8]) -> Result<Self, AccountStateError> {
        let record = Self::decode(bytes)?;
        if Self::state_key(&record.account_id, &record.scope)? == *key {
            Ok(record)
        } else {
            Err(AccountStateError::KeyMismatch)
        }
    }

    pub(super) fn encode(&self) -> Result<Vec<u8>, AccountStateError> {
        self.validate()?;
        encode_wire(&QuoteFlowWire {
            schema: QUOTE_SCHEMA.to_owned(),
            account_id: self.account_id.to_api_string(),
            scope: self.scope.as_wire_name().to_owned(),
            market_id: self.scope.market_id(),
            vault_id: self.scope.vault_id(),
            credits: self.credits.to_string(),
            debits: self.debits.to_string(),
            last_event_id: self.last_event_id.as_str().to_owned(),
            last_block_height: self.last_block_height.get(),
        })
    }

    fn validate(&self) -> Result<(), AccountStateError> {
        if valid_quote_totals(self.credits, self.debits) {
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
    pub const fn scope(&self) -> &AccountQuoteFlowScopeV1 {
        &self.scope
    }

    #[must_use]
    pub const fn credits(&self) -> QuoteAmount {
        self.credits
    }

    #[must_use]
    pub const fn debits(&self) -> QuoteAmount {
        self.debits
    }

    #[must_use]
    pub const fn last_event_id(&self) -> &EventId {
        &self.last_event_id
    }

    #[must_use]
    pub const fn last_block_height(&self) -> BlockHeight {
        self.last_block_height
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultPrincipalFlowCurrentRecordV1 {
    pub(super) vault_id: VaultId,
    pub(super) deposits: QuoteAmount,
    pub(super) withdrawals: QuoteAmount,
    pub(super) last_event_id: EventId,
    pub(super) last_block_height: BlockHeight,
}

impl VaultPrincipalFlowCurrentRecordV1 {
    pub fn state_key(vault_id: &VaultId) -> Result<StateKey, AccountStateError> {
        state_key(VAULT_PRINCIPAL_NAMESPACE, &[vault_id.as_str().as_bytes()])
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, AccountStateError> {
        let wire: VaultPrincipalWire = decode_wire(bytes)?;
        if wire.schema != VAULT_PRINCIPAL_SCHEMA {
            return Err(AccountStateError::InvalidRecord);
        }
        let record = Self {
            vault_id: VaultId::new(wire.vault_id).map_err(|_| AccountStateError::InvalidRecord)?,
            deposits: QuoteAmount::from_str(&wire.deposits)
                .map_err(|_| AccountStateError::InvalidRecord)?,
            withdrawals: QuoteAmount::from_str(&wire.withdrawals)
                .map_err(|_| AccountStateError::InvalidRecord)?,
            last_event_id: EventId::new(wire.last_event_id)
                .map_err(|_| AccountStateError::InvalidRecord)?,
            last_block_height: BlockHeight::new(wire.last_block_height),
        };
        record.validate()?;
        require_record_bytes(&record.encode()?, bytes)?;
        Ok(record)
    }

    pub fn decode_at(key: &StateKey, bytes: &[u8]) -> Result<Self, AccountStateError> {
        let record = Self::decode(bytes)?;
        if Self::state_key(&record.vault_id)? == *key {
            Ok(record)
        } else {
            Err(AccountStateError::KeyMismatch)
        }
    }

    pub(super) fn encode(&self) -> Result<Vec<u8>, AccountStateError> {
        self.validate()?;
        encode_wire(&VaultPrincipalWire {
            schema: VAULT_PRINCIPAL_SCHEMA.to_owned(),
            vault_id: self.vault_id.as_str().to_owned(),
            deposits: self.deposits.to_string(),
            withdrawals: self.withdrawals.to_string(),
            last_event_id: self.last_event_id.as_str().to_owned(),
            last_block_height: self.last_block_height.get(),
        })
    }

    fn validate(&self) -> Result<(), AccountStateError> {
        if valid_quote_totals(self.deposits, self.withdrawals) {
            Ok(())
        } else {
            Err(AccountStateError::InvalidRecord)
        }
    }

    #[must_use]
    pub const fn vault_id(&self) -> &VaultId {
        &self.vault_id
    }

    #[must_use]
    pub const fn deposits(&self) -> QuoteAmount {
        self.deposits
    }

    #[must_use]
    pub const fn withdrawals(&self) -> QuoteAmount {
        self.withdrawals
    }

    #[must_use]
    pub const fn last_event_id(&self) -> &EventId {
        &self.last_event_id
    }

    #[must_use]
    pub const fn last_block_height(&self) -> BlockHeight {
        self.last_block_height
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultShareFlowCurrentRecordV1 {
    pub(super) vault_id: VaultId,
    pub(super) shares_issued: Quantity,
    pub(super) shares_redeemed: Quantity,
    pub(super) last_event_id: EventId,
    pub(super) last_block_height: BlockHeight,
}

impl VaultShareFlowCurrentRecordV1 {
    pub fn state_key(vault_id: &VaultId) -> Result<StateKey, AccountStateError> {
        state_key(VAULT_SHARE_NAMESPACE, &[vault_id.as_str().as_bytes()])
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, AccountStateError> {
        let wire: VaultShareWire = decode_wire(bytes)?;
        if wire.schema != VAULT_SHARE_SCHEMA {
            return Err(AccountStateError::InvalidRecord);
        }
        let record = Self {
            vault_id: VaultId::new(wire.vault_id).map_err(|_| AccountStateError::InvalidRecord)?,
            shares_issued: Quantity::from_str(&wire.shares_issued)
                .map_err(|_| AccountStateError::InvalidRecord)?,
            shares_redeemed: Quantity::from_str(&wire.shares_redeemed)
                .map_err(|_| AccountStateError::InvalidRecord)?,
            last_event_id: EventId::new(wire.last_event_id)
                .map_err(|_| AccountStateError::InvalidRecord)?,
            last_block_height: BlockHeight::new(wire.last_block_height),
        };
        record.validate()?;
        require_record_bytes(&record.encode()?, bytes)?;
        Ok(record)
    }

    pub fn decode_at(key: &StateKey, bytes: &[u8]) -> Result<Self, AccountStateError> {
        let record = Self::decode(bytes)?;
        if Self::state_key(&record.vault_id)? == *key {
            Ok(record)
        } else {
            Err(AccountStateError::KeyMismatch)
        }
    }

    pub(super) fn encode(&self) -> Result<Vec<u8>, AccountStateError> {
        self.validate()?;
        encode_wire(&VaultShareWire {
            schema: VAULT_SHARE_SCHEMA.to_owned(),
            vault_id: self.vault_id.as_str().to_owned(),
            shares_issued: self.shares_issued.to_string(),
            shares_redeemed: self.shares_redeemed.to_string(),
            last_event_id: self.last_event_id.as_str().to_owned(),
            last_block_height: self.last_block_height.get(),
        })
    }

    fn validate(&self) -> Result<(), AccountStateError> {
        if valid_quantity_totals(self.shares_issued, self.shares_redeemed) {
            Ok(())
        } else {
            Err(AccountStateError::InvalidRecord)
        }
    }

    #[must_use]
    pub const fn vault_id(&self) -> &VaultId {
        &self.vault_id
    }

    #[must_use]
    pub const fn shares_issued(&self) -> Quantity {
        self.shares_issued
    }

    #[must_use]
    pub const fn shares_redeemed(&self) -> Quantity {
        self.shares_redeemed
    }

    #[must_use]
    pub const fn last_event_id(&self) -> &EventId {
        &self.last_event_id
    }

    #[must_use]
    pub const fn last_block_height(&self) -> BlockHeight {
        self.last_block_height
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct QuantityFlowWire {
    schema: String,
    account_id: String,
    scope: String,
    asset_id: Option<String>,
    vault_id: Option<String>,
    credits: String,
    debits: String,
    last_event_id: String,
    last_block_height: u64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct QuoteFlowWire {
    schema: String,
    account_id: String,
    scope: String,
    market_id: Option<String>,
    vault_id: Option<String>,
    credits: String,
    debits: String,
    last_event_id: String,
    last_block_height: u64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct VaultPrincipalWire {
    schema: String,
    vault_id: String,
    deposits: String,
    withdrawals: String,
    last_event_id: String,
    last_block_height: u64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct VaultShareWire {
    schema: String,
    vault_id: String,
    shares_issued: String,
    shares_redeemed: String,
    last_event_id: String,
    last_block_height: u64,
}

const fn valid_quantity_totals(left: Quantity, right: Quantity) -> bool {
    left.raw() >= 0 && right.raw() >= 0 && left.scale() == right.scale()
}

const fn valid_quote_totals(left: QuoteAmount, right: QuoteAmount) -> bool {
    left.raw() >= 0 && right.raw() >= 0 && left.scale() == right.scale()
}
