mod cashflow;
mod codec;
mod modes;
mod relations;

pub use cashflow::{
    AccountQuantityFlowCurrentRecordV1, AccountQuantityFlowScopeV1,
    AccountQuoteFlowCurrentRecordV1, AccountQuoteFlowScopeV1, VaultPrincipalFlowCurrentRecordV1,
    VaultShareFlowCurrentRecordV1,
};
pub use codec::AccountStateError;
pub use modes::{AccountModeCurrentRecordV1, LeverageCurrentRecordV1, MarginModeCurrentRecordV1};
pub use relations::{AccountVaultRelationCurrentRecordV1, SubaccountMasterCurrentRecordV1};

use canonical_events::EventKind;
use domain_types::{Address, AssetId, BlockHeight, EventId, MarketId, VaultId};
use serde::{Deserialize, Serialize};

use crate::{StateKey, account::codec::require_record_bytes};

use self::codec::{decode_hash, decode_wire, encode_wire, state_key};

const FACT_NAMESPACE: &str = "account-fact.v1";
const FACT_SCHEMA: &str = "hyperliquid-alpha-desk/account-fact/v1";
const MAX_FACT_ACCOUNTS: usize = 3;
const MAX_FACT_MARKETS: usize = 1;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CanonicalAccountReducerV1;

impl CanonicalAccountReducerV1 {
    pub const VERSION: &'static str = "hyperliquid-alpha-desk-canonical-account@1.0.0";
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountFactRecordV1 {
    event_id: EventId,
    event_kind: EventKind,
    account_ids: Vec<Address>,
    market_ids: Vec<MarketId>,
    asset_id: Option<AssetId>,
    vault_id: Option<VaultId>,
    block_height: BlockHeight,
    payload_hash: [u8; 32],
    rule_version: String,
}

impl AccountFactRecordV1 {
    pub fn state_key(event_id: &EventId) -> Result<StateKey, AccountStateError> {
        state_key(FACT_NAMESPACE, &[event_id.as_str().as_bytes()])
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, AccountStateError> {
        let wire: FactWire = decode_wire(bytes)?;
        if wire.schema != FACT_SCHEMA || wire.rule_version != CanonicalAccountReducerV1::VERSION {
            return Err(AccountStateError::InvalidRecord);
        }
        if wire.account_ids.len() > MAX_FACT_ACCOUNTS || wire.market_ids.len() > MAX_FACT_MARKETS {
            return Err(AccountStateError::InvalidRecord);
        }
        let record = Self {
            event_id: EventId::new(wire.event_id).map_err(|_| AccountStateError::InvalidRecord)?,
            event_kind: EventKind::try_from(wire.event_kind.as_str())
                .map_err(|_| AccountStateError::InvalidRecord)?,
            account_ids: wire
                .account_ids
                .iter()
                .map(|value| {
                    Address::parse_api(value).map_err(|_| AccountStateError::InvalidRecord)
                })
                .collect::<Result<_, _>>()?,
            market_ids: wire
                .market_ids
                .into_iter()
                .map(MarketId::new)
                .collect::<Result<_, _>>()
                .map_err(|_| AccountStateError::InvalidRecord)?,
            asset_id: wire
                .asset_id
                .map(AssetId::new)
                .transpose()
                .map_err(|_| AccountStateError::InvalidRecord)?,
            vault_id: wire
                .vault_id
                .map(VaultId::new)
                .transpose()
                .map_err(|_| AccountStateError::InvalidRecord)?,
            block_height: BlockHeight::new(wire.block_height),
            payload_hash: decode_hash(&wire.payload_blake3)?,
            rule_version: wire.rule_version,
        };
        record.validate()?;
        require_record_bytes(&record.encode()?, bytes)?;
        Ok(record)
    }

    pub fn decode_at(key: &StateKey, bytes: &[u8]) -> Result<Self, AccountStateError> {
        let record = Self::decode(bytes)?;
        if Self::state_key(&record.event_id)? == *key {
            Ok(record)
        } else {
            Err(AccountStateError::KeyMismatch)
        }
    }

    pub(super) fn encode(&self) -> Result<Vec<u8>, AccountStateError> {
        self.validate()?;
        encode_wire(&FactWire {
            schema: FACT_SCHEMA.to_owned(),
            event_id: self.event_id.as_str().to_owned(),
            event_kind: self.event_kind.as_wire_name().to_owned(),
            account_ids: self
                .account_ids
                .iter()
                .map(|value| value.to_api_string())
                .collect(),
            market_ids: self
                .market_ids
                .iter()
                .map(|value| value.as_str().to_owned())
                .collect(),
            asset_id: self
                .asset_id
                .as_ref()
                .map(|value| value.as_str().to_owned()),
            vault_id: self
                .vault_id
                .as_ref()
                .map(|value| value.as_str().to_owned()),
            block_height: self.block_height.get(),
            payload_blake3: hex::encode(self.payload_hash),
            rule_version: self.rule_version.clone(),
        })
    }

    fn validate(&self) -> Result<(), AccountStateError> {
        if self.rule_version != CanonicalAccountReducerV1::VERSION
            || self.account_ids.len() > MAX_FACT_ACCOUNTS
            || self.market_ids.len() > MAX_FACT_MARKETS
            || !valid_fact_shape(
                self.event_kind,
                self.account_ids.len(),
                self.market_ids.len(),
                self.asset_id.is_some(),
                self.vault_id.is_some(),
            )
        {
            Err(AccountStateError::InvalidRecord)
        } else {
            Ok(())
        }
    }

    #[must_use]
    pub const fn event_id(&self) -> &EventId {
        &self.event_id
    }

    #[must_use]
    pub const fn event_kind(&self) -> EventKind {
        self.event_kind
    }

    #[must_use]
    pub fn account_ids(&self) -> &[Address] {
        &self.account_ids
    }

    #[must_use]
    pub fn market_ids(&self) -> &[MarketId] {
        &self.market_ids
    }

    #[must_use]
    pub const fn asset_id(&self) -> Option<&AssetId> {
        self.asset_id.as_ref()
    }

    #[must_use]
    pub const fn vault_id(&self) -> Option<&VaultId> {
        self.vault_id.as_ref()
    }

    #[must_use]
    pub const fn block_height(&self) -> BlockHeight {
        self.block_height
    }

    #[must_use]
    pub const fn payload_hash(&self) -> [u8; 32] {
        self.payload_hash
    }

    #[must_use]
    pub fn rule_version(&self) -> &str {
        &self.rule_version
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FactWire {
    schema: String,
    event_id: String,
    event_kind: String,
    account_ids: Vec<String>,
    market_ids: Vec<String>,
    asset_id: Option<String>,
    vault_id: Option<String>,
    block_height: u64,
    payload_blake3: String,
    rule_version: String,
}

const fn valid_fact_shape(
    event_kind: EventKind,
    account_count: usize,
    market_count: usize,
    has_asset: bool,
    has_vault: bool,
) -> bool {
    match event_kind {
        EventKind::DepositCredited | EventKind::WithdrawalDebited | EventKind::FeeCharged => {
            account_count == 1 && market_count == 0 && has_asset && !has_vault
        }
        EventKind::SpotTransfer | EventKind::BuilderFeeCharged | EventKind::ReferralReward => {
            account_count == 2 && market_count == 0 && has_asset && !has_vault
        }
        EventKind::SubaccountTransfer => {
            account_count == 3 && market_count == 0 && has_asset && !has_vault
        }
        EventKind::PerpTransfer => {
            account_count == 2 && market_count == 0 && !has_asset && !has_vault
        }
        EventKind::VaultDeposit | EventKind::VaultWithdrawal => {
            account_count == 1 && market_count == 0 && !has_asset && has_vault
        }
        EventKind::FundingPaid | EventKind::FundingReceived => {
            account_count == 1 && market_count == 1 && !has_asset && !has_vault
        }
        EventKind::AccountModeChanged => {
            account_count == 1 && market_count == 0 && !has_asset && !has_vault
        }
        EventKind::MarginModeChanged | EventKind::LeverageChanged => {
            account_count == 1 && market_count == 1 && !has_asset && !has_vault
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use domain_types::{AccountAbstractionModeV1, Leverage, MarginModeV1, Quantity, QuoteAmount};

    use super::*;

    const ACCOUNT_A: Address = Address::from_bytes([0x11; 20]);
    const ACCOUNT_B: Address = Address::from_bytes([0x22; 20]);

    #[test]
    fn crate_private_encoders_round_trip_every_record_family_and_scope() {
        let first_event_id = EventId::new("event-first").unwrap();
        let last_event_id = EventId::new("event-last").unwrap();
        let asset_id = AssetId::new("USDC").unwrap();
        let market_id = MarketId::new("perp:BTC").unwrap();
        let vault_id = VaultId::new("vault-a").unwrap();

        let fact = AccountFactRecordV1 {
            event_id: last_event_id.clone(),
            event_kind: EventKind::DepositCredited,
            account_ids: vec![ACCOUNT_A],
            market_ids: Vec::new(),
            asset_id: Some(asset_id.clone()),
            vault_id: None,
            block_height: BlockHeight::new(8),
            payload_hash: [0x44; 32],
            rule_version: CanonicalAccountReducerV1::VERSION.to_owned(),
        };
        assert_eq!(
            AccountFactRecordV1::decode(&fact.encode().unwrap()).unwrap(),
            fact
        );

        let quantity_scopes = [
            AccountQuantityFlowScopeV1::ExternalAsset {
                asset_id: asset_id.clone(),
            },
            AccountQuantityFlowScopeV1::SpotTransferAsset {
                asset_id: asset_id.clone(),
            },
            AccountQuantityFlowScopeV1::SubaccountTransferAsset {
                asset_id: asset_id.clone(),
            },
            AccountQuantityFlowScopeV1::FeeAsset {
                asset_id: asset_id.clone(),
            },
            AccountQuantityFlowScopeV1::BuilderFeeAsset {
                asset_id: asset_id.clone(),
            },
            AccountQuantityFlowScopeV1::ReferralRewardAsset {
                asset_id: asset_id.clone(),
            },
            AccountQuantityFlowScopeV1::VaultShares {
                vault_id: vault_id.clone(),
            },
        ];
        for scope in quantity_scopes {
            let record = AccountQuantityFlowCurrentRecordV1 {
                account_id: ACCOUNT_A,
                scope,
                credits: Quantity::from_str("2.00").unwrap(),
                debits: Quantity::from_str("1.00").unwrap(),
                last_event_id: last_event_id.clone(),
                last_block_height: BlockHeight::new(8),
            };
            assert_eq!(
                AccountQuantityFlowCurrentRecordV1::decode(&record.encode().unwrap()).unwrap(),
                record
            );
        }

        let quote_scopes = [
            AccountQuoteFlowScopeV1::DefaultPerpQuote,
            AccountQuoteFlowScopeV1::MarketFunding {
                market_id: market_id.clone(),
            },
            AccountQuoteFlowScopeV1::VaultPrincipal {
                vault_id: vault_id.clone(),
            },
        ];
        for scope in quote_scopes {
            let record = AccountQuoteFlowCurrentRecordV1 {
                account_id: ACCOUNT_A,
                scope,
                credits: QuoteAmount::from_str("2.00").unwrap(),
                debits: QuoteAmount::from_str("1.00").unwrap(),
                last_event_id: last_event_id.clone(),
                last_block_height: BlockHeight::new(8),
            };
            assert_eq!(
                AccountQuoteFlowCurrentRecordV1::decode(&record.encode().unwrap()).unwrap(),
                record
            );
        }

        let principal = VaultPrincipalFlowCurrentRecordV1 {
            vault_id: vault_id.clone(),
            deposits: QuoteAmount::from_str("2.00").unwrap(),
            withdrawals: QuoteAmount::from_str("1.00").unwrap(),
            last_event_id: last_event_id.clone(),
            last_block_height: BlockHeight::new(8),
        };
        assert_eq!(
            VaultPrincipalFlowCurrentRecordV1::decode(&principal.encode().unwrap()).unwrap(),
            principal
        );

        let shares = VaultShareFlowCurrentRecordV1 {
            vault_id: vault_id.clone(),
            shares_issued: Quantity::from_str("2.00").unwrap(),
            shares_redeemed: Quantity::from_str("1.00").unwrap(),
            last_event_id: last_event_id.clone(),
            last_block_height: BlockHeight::new(8),
        };
        assert_eq!(
            VaultShareFlowCurrentRecordV1::decode(&shares.encode().unwrap()).unwrap(),
            shares
        );

        let subaccount = SubaccountMasterCurrentRecordV1 {
            subaccount_id: ACCOUNT_B,
            master_account_id: ACCOUNT_A,
            first_event_id: first_event_id.clone(),
            last_event_id: last_event_id.clone(),
            first_block_height: BlockHeight::new(5),
            last_block_height: BlockHeight::new(8),
        };
        assert_eq!(
            SubaccountMasterCurrentRecordV1::decode(&subaccount.encode().unwrap()).unwrap(),
            subaccount
        );

        let vault_relation = AccountVaultRelationCurrentRecordV1 {
            account_id: ACCOUNT_A,
            vault_id: vault_id.clone(),
            first_event_id: first_event_id.clone(),
            last_event_id: last_event_id.clone(),
            first_block_height: BlockHeight::new(5),
            last_block_height: BlockHeight::new(8),
        };
        assert_eq!(
            AccountVaultRelationCurrentRecordV1::decode(&vault_relation.encode().unwrap()).unwrap(),
            vault_relation
        );

        let account_mode = AccountModeCurrentRecordV1 {
            account_id: ACCOUNT_A,
            initial_previous: AccountAbstractionModeV1::Standard,
            current: AccountAbstractionModeV1::Standard,
            first_event_id: first_event_id.clone(),
            last_event_id: last_event_id.clone(),
            first_block_height: BlockHeight::new(5),
            last_block_height: BlockHeight::new(8),
        };
        assert_eq!(
            AccountModeCurrentRecordV1::decode(&account_mode.encode().unwrap()).unwrap(),
            account_mode
        );

        let margin_mode = MarginModeCurrentRecordV1 {
            account_id: ACCOUNT_A,
            market_id: market_id.clone(),
            initial_previous: MarginModeV1::Cross,
            current: MarginModeV1::Cross,
            first_event_id: first_event_id.clone(),
            last_event_id: last_event_id.clone(),
            first_block_height: BlockHeight::new(5),
            last_block_height: BlockHeight::new(8),
        };
        assert_eq!(
            MarginModeCurrentRecordV1::decode(&margin_mode.encode().unwrap()).unwrap(),
            margin_mode
        );

        let leverage = LeverageCurrentRecordV1 {
            account_id: ACCOUNT_A,
            market_id,
            initial_previous: Leverage::from_str("3.00").unwrap(),
            current: Leverage::from_str("3.00").unwrap(),
            first_event_id,
            last_event_id,
            first_block_height: BlockHeight::new(5),
            last_block_height: BlockHeight::new(8),
        };
        assert_eq!(
            LeverageCurrentRecordV1::decode(&leverage.encode().unwrap()).unwrap(),
            leverage
        );
    }
}
