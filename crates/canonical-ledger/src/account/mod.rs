mod cashflow;
mod codec;
mod modes;
mod relations;

use std::collections::BTreeSet;

pub use cashflow::{
    AccountQuantityFlowCurrentRecordV1, AccountQuantityFlowScopeV1,
    AccountQuoteFlowCurrentRecordV1, AccountQuoteFlowScopeV1, VaultPrincipalFlowCurrentRecordV1,
    VaultShareFlowCurrentRecordV1,
};
pub use codec::AccountStateError;
pub use modes::{AccountModeCurrentRecordV1, LeverageCurrentRecordV1, MarginModeCurrentRecordV1};
pub use relations::{AccountVaultRelationCurrentRecordV1, SubaccountMasterCurrentRecordV1};

use canonical_events::{CanonicalEventEnvelope, EventKind, EventPayload};
use domain_types::{
    Address, AssetId, BlockHeight, EventId, FeeTypeV1, MarketId, ValueError, VaultId,
};
use serde::{Deserialize, Serialize};

use crate::{
    ApplyContext, AssetContextCurrentRecordV1, EventReducer, MarketCurrentRecordV1,
    MarketMetadataResolutionV1, ReducerError, StateKey, StateMutation, StateView,
    account::codec::require_record_bytes,
    opaque::{decode_class_transfer, decode_internal, decode_reward, decode_spot_genesis},
};

pub(crate) use cashflow::{
    FlowSide, quote_flow_mutation as quote_flow_put,
    vault_principal_mutation as vault_principal_put,
};
pub(crate) use relations::vault_relation_mutation as vault_relation_put;

use self::{
    cashflow::{
        quantity_flow_mutation, quote_flow_mutation, vault_principal_mutation, vault_share_mutation,
    },
    codec::{decode_hash, decode_wire, encode_wire, state_key},
    modes::{account_mode_mutation, leverage_mutation, margin_mode_mutation},
    relations::{subaccount_relation_mutation, vault_relation_mutation},
};

const FACT_NAMESPACE: &str = "account-fact.v1";
const FACT_SCHEMA: &str = "hyperliquid-alpha-desk/account-fact/v1";
const MAX_FACT_ACCOUNTS: usize = 3;
const MAX_FACT_MARKETS: usize = 1;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CanonicalAccountReducerV1;

impl CanonicalAccountReducerV1 {
    pub const VERSION: &'static str = "hyperliquid-alpha-desk-canonical-account@1.0.0";
}

impl EventReducer for CanonicalAccountReducerV1 {
    fn reducer_set_version(&self) -> &str {
        Self::VERSION
    }

    fn supports(&self, event: &CanonicalEventEnvelope) -> bool {
        event.schema_version() == "1.0.0"
            && matches!(
                event.event_kind(),
                EventKind::DepositCredited
                    | EventKind::WithdrawalDebited
                    | EventKind::SpotTransfer
                    | EventKind::PerpTransfer
                    | EventKind::SubaccountTransfer
                    | EventKind::VaultDeposit
                    | EventKind::VaultWithdrawal
                    | EventKind::FeeCharged
                    | EventKind::BuilderFeeCharged
                    | EventKind::FundingPaid
                    | EventKind::FundingReceived
                    | EventKind::ReferralReward
                    | EventKind::AccountModeChanged
                    | EventKind::MarginModeChanged
                    | EventKind::LeverageChanged
                    | EventKind::InternalTransfer
                    | EventKind::AccountClassTransfer
                    | EventKind::RewardClaimed
                    | EventKind::SpotGenesisApplied
            )
    }

    fn reduce(
        &self,
        state: &StateView<'_>,
        event: &CanonicalEventEnvelope,
        _context: &ApplyContext<'_>,
    ) -> Result<Vec<StateMutation>, ReducerError> {
        if !self.supports(event) {
            return Err(reducer_error(
                "account_state.unsupported_event",
                "account reducer received an unsupported event",
            ));
        }

        validate_event_identities(event)?;
        validate_prerequisites(state, event)?;

        let fact_key =
            AccountFactRecordV1::state_key(event.event_id()).map_err(codec_reducer_error)?;
        if state.contains_key(&fact_key) {
            return Err(reducer_error(
                "account_state.event_identity_collision",
                "account event identity is already present",
            ));
        }
        let fact = AccountFactRecordV1::from_event(event)?;
        let mut mutations = vec![StateMutation::put(
            fact_key,
            fact.encode().map_err(codec_reducer_error)?,
        )];
        let height = event.block_height();
        let event_id = event.event_id();

        match event.payload() {
            EventPayload::DepositCredited(payload) => {
                mutations.push(quantity_flow_mutation(
                    state,
                    payload.account_id,
                    AccountQuantityFlowScopeV1::ExternalAsset {
                        asset_id: payload.asset_id.clone(),
                    },
                    payload.amount,
                    FlowSide::Credit,
                    event_id,
                    height,
                )?);
            }
            EventPayload::WithdrawalDebited(payload) => {
                mutations.push(quantity_flow_mutation(
                    state,
                    payload.account_id,
                    AccountQuantityFlowScopeV1::ExternalAsset {
                        asset_id: payload.asset_id.clone(),
                    },
                    payload.amount,
                    FlowSide::Debit,
                    event_id,
                    height,
                )?);
            }
            EventPayload::SpotTransfer(payload) => {
                for (account_id, side) in [
                    (payload.from_account_id, FlowSide::Debit),
                    (payload.to_account_id, FlowSide::Credit),
                ] {
                    mutations.push(quantity_flow_mutation(
                        state,
                        account_id,
                        AccountQuantityFlowScopeV1::SpotTransferAsset {
                            asset_id: payload.asset_id.clone(),
                        },
                        payload.amount,
                        side,
                        event_id,
                        height,
                    )?);
                }
            }
            EventPayload::PerpTransfer(payload) => {
                for (account_id, side) in [
                    (payload.from_account_id, FlowSide::Debit),
                    (payload.to_account_id, FlowSide::Credit),
                ] {
                    mutations.push(quote_flow_mutation(
                        state,
                        account_id,
                        AccountQuoteFlowScopeV1::DefaultPerpQuote,
                        payload.quote_amount,
                        side,
                        event_id,
                        height,
                    )?);
                }
            }
            EventPayload::SubaccountTransfer(payload) => {
                for (account_id, side) in [
                    (payload.from_account_id, FlowSide::Debit),
                    (payload.to_account_id, FlowSide::Credit),
                ] {
                    mutations.push(quantity_flow_mutation(
                        state,
                        account_id,
                        AccountQuantityFlowScopeV1::SubaccountTransferAsset {
                            asset_id: payload.asset_id.clone(),
                        },
                        payload.amount,
                        side,
                        event_id,
                        height,
                    )?);
                }
                mutations.push(subaccount_relation_mutation(
                    state,
                    payload.master_account_id,
                    payload.from_account_id,
                    payload.to_account_id,
                    event_id,
                    height,
                )?);
            }
            EventPayload::VaultDeposit(payload) => {
                mutations.push(quote_flow_mutation(
                    state,
                    payload.account_id,
                    AccountQuoteFlowScopeV1::VaultPrincipal {
                        vault_id: payload.vault_id.clone(),
                    },
                    payload.amount,
                    FlowSide::Debit,
                    event_id,
                    height,
                )?);
                mutations.push(quantity_flow_mutation(
                    state,
                    payload.account_id,
                    AccountQuantityFlowScopeV1::VaultShares {
                        vault_id: payload.vault_id.clone(),
                    },
                    payload.shares_issued,
                    FlowSide::Credit,
                    event_id,
                    height,
                )?);
                mutations.push(vault_principal_mutation(
                    state,
                    &payload.vault_id,
                    payload.amount,
                    FlowSide::Credit,
                    event_id,
                    height,
                )?);
                mutations.push(vault_share_mutation(
                    state,
                    &payload.vault_id,
                    payload.shares_issued,
                    FlowSide::Credit,
                    event_id,
                    height,
                )?);
                mutations.push(vault_relation_mutation(
                    state,
                    payload.account_id,
                    &payload.vault_id,
                    event_id,
                    height,
                )?);
            }
            EventPayload::VaultWithdrawal(payload) => {
                mutations.push(quote_flow_mutation(
                    state,
                    payload.account_id,
                    AccountQuoteFlowScopeV1::VaultPrincipal {
                        vault_id: payload.vault_id.clone(),
                    },
                    payload.amount,
                    FlowSide::Credit,
                    event_id,
                    height,
                )?);
                mutations.push(quantity_flow_mutation(
                    state,
                    payload.account_id,
                    AccountQuantityFlowScopeV1::VaultShares {
                        vault_id: payload.vault_id.clone(),
                    },
                    payload.shares_redeemed,
                    FlowSide::Debit,
                    event_id,
                    height,
                )?);
                mutations.push(vault_principal_mutation(
                    state,
                    &payload.vault_id,
                    payload.amount,
                    FlowSide::Debit,
                    event_id,
                    height,
                )?);
                mutations.push(vault_share_mutation(
                    state,
                    &payload.vault_id,
                    payload.shares_redeemed,
                    FlowSide::Debit,
                    event_id,
                    height,
                )?);
                mutations.push(vault_relation_mutation(
                    state,
                    payload.account_id,
                    &payload.vault_id,
                    event_id,
                    height,
                )?);
            }
            EventPayload::FeeCharged(payload) => {
                let side = if payload.fee_type == FeeTypeV1::MakerRebate {
                    FlowSide::Credit
                } else {
                    FlowSide::Debit
                };
                mutations.push(quantity_flow_mutation(
                    state,
                    payload.account_id,
                    AccountQuantityFlowScopeV1::FeeAsset {
                        asset_id: payload.asset_id.clone(),
                    },
                    payload.amount,
                    side,
                    event_id,
                    height,
                )?);
            }
            EventPayload::BuilderFeeCharged(payload) => {
                for (account_id, side) in [
                    (payload.account_id, FlowSide::Debit),
                    (payload.builder_account_id, FlowSide::Credit),
                ] {
                    mutations.push(quantity_flow_mutation(
                        state,
                        account_id,
                        AccountQuantityFlowScopeV1::BuilderFeeAsset {
                            asset_id: payload.asset_id.clone(),
                        },
                        payload.amount,
                        side,
                        event_id,
                        height,
                    )?);
                }
            }
            EventPayload::FundingPaid(payload) => {
                mutations.push(quote_flow_mutation(
                    state,
                    payload.account_id,
                    AccountQuoteFlowScopeV1::MarketFunding {
                        market_id: payload.market_id.clone(),
                    },
                    payload.amount,
                    FlowSide::Debit,
                    event_id,
                    height,
                )?);
            }
            EventPayload::FundingReceived(payload) => {
                mutations.push(quote_flow_mutation(
                    state,
                    payload.account_id,
                    AccountQuoteFlowScopeV1::MarketFunding {
                        market_id: payload.market_id.clone(),
                    },
                    payload.amount,
                    FlowSide::Credit,
                    event_id,
                    height,
                )?);
            }
            EventPayload::ReferralReward(payload) => {
                mutations.push(quantity_flow_mutation(
                    state,
                    payload.referrer_account_id,
                    AccountQuantityFlowScopeV1::ReferralRewardAsset {
                        asset_id: payload.asset_id.clone(),
                    },
                    payload.amount,
                    FlowSide::Credit,
                    event_id,
                    height,
                )?);
            }
            EventPayload::AccountModeChanged(payload) => {
                mutations.push(account_mode_mutation(
                    state,
                    payload.account_id,
                    payload.previous_mode,
                    payload.new_mode,
                    event_id,
                    height,
                )?);
            }
            EventPayload::MarginModeChanged(payload) => {
                mutations.push(margin_mode_mutation(
                    state,
                    payload.account_id,
                    &payload.market_id,
                    payload.previous_mode,
                    payload.new_mode,
                    event_id,
                    height,
                )?);
            }
            EventPayload::LeverageChanged(payload) => {
                mutations.push(leverage_mutation(
                    state,
                    payload.account_id,
                    &payload.market_id,
                    payload.previous_leverage,
                    payload.new_leverage,
                    event_id,
                    height,
                )?);
            }
            EventPayload::InternalTransfer(payload) => {
                let decoded = decode_internal(payload)?;
                let debit = decoded
                    .amount
                    .checked_add(decoded.fee)
                    .map_err(flow_reducer_error)?;
                mutations.push(quote_flow_mutation(
                    state,
                    decoded.from_account_id,
                    AccountQuoteFlowScopeV1::DefaultPerpQuote,
                    debit,
                    FlowSide::Debit,
                    event_id,
                    height,
                )?);
                mutations.push(quote_flow_mutation(
                    state,
                    decoded.to_account_id,
                    AccountQuoteFlowScopeV1::DefaultPerpQuote,
                    decoded.amount,
                    FlowSide::Credit,
                    event_id,
                    height,
                )?);
            }
            EventPayload::AccountClassTransfer(payload) => {
                let decoded = decode_class_transfer(payload)?;
                let (debit_scope, credit_scope) = if decoded.to_perp {
                    (
                        AccountQuoteFlowScopeV1::SpotClassQuote,
                        AccountQuoteFlowScopeV1::DefaultPerpQuote,
                    )
                } else {
                    (
                        AccountQuoteFlowScopeV1::DefaultPerpQuote,
                        AccountQuoteFlowScopeV1::SpotClassQuote,
                    )
                };
                mutations.push(quote_flow_mutation(
                    state,
                    decoded.account_id,
                    debit_scope,
                    decoded.amount,
                    FlowSide::Debit,
                    event_id,
                    height,
                )?);
                mutations.push(quote_flow_mutation(
                    state,
                    decoded.account_id,
                    credit_scope,
                    decoded.amount,
                    FlowSide::Credit,
                    event_id,
                    height,
                )?);
            }
            EventPayload::RewardClaimed(payload) => {
                let decoded = decode_reward(payload)?;
                mutations.push(quote_flow_mutation(
                    state,
                    decoded.account_id,
                    AccountQuoteFlowScopeV1::RewardClaimedQuote,
                    decoded.amount,
                    FlowSide::Credit,
                    event_id,
                    height,
                )?);
            }
            EventPayload::SpotGenesisApplied(payload) => {
                let decoded = decode_spot_genesis(payload)?;
                if let [account_id] = event.account_addresses() {
                    mutations.push(quantity_flow_mutation(
                        state,
                        *account_id,
                        AccountQuantityFlowScopeV1::SpotGenesisAsset {
                            asset_id: decoded.token,
                        },
                        decoded.amount,
                        FlowSide::Credit,
                        event_id,
                        height,
                    )?);
                }
            }
            _ => {
                return Err(reducer_error(
                    "account_state.unsupported_event",
                    "account reducer received an unsupported event",
                ));
            }
        }

        ensure_unique_mutation_keys(&mutations)?;
        Ok(mutations)
    }
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

    fn from_event(event: &CanonicalEventEnvelope) -> Result<Self, ReducerError> {
        let (asset_id, vault_id) = match event.payload() {
            EventPayload::DepositCredited(payload) => (Some(payload.asset_id.clone()), None),
            EventPayload::WithdrawalDebited(payload) => (Some(payload.asset_id.clone()), None),
            EventPayload::SpotTransfer(payload) => (Some(payload.asset_id.clone()), None),
            EventPayload::SubaccountTransfer(payload) => (Some(payload.asset_id.clone()), None),
            EventPayload::FeeCharged(payload) => (Some(payload.asset_id.clone()), None),
            EventPayload::BuilderFeeCharged(payload) => (Some(payload.asset_id.clone()), None),
            EventPayload::ReferralReward(payload) => (Some(payload.asset_id.clone()), None),
            EventPayload::VaultDeposit(payload) => (None, Some(payload.vault_id.clone())),
            EventPayload::VaultWithdrawal(payload) => (None, Some(payload.vault_id.clone())),
            EventPayload::PerpTransfer(_)
            | EventPayload::FundingPaid(_)
            | EventPayload::FundingReceived(_)
            | EventPayload::AccountModeChanged(_)
            | EventPayload::MarginModeChanged(_)
            | EventPayload::LeverageChanged(_)
            | EventPayload::InternalTransfer(_)
            | EventPayload::AccountClassTransfer(_)
            | EventPayload::RewardClaimed(_) => (None, None),
            EventPayload::SpotGenesisApplied(payload) => {
                (Some(decode_spot_genesis(payload)?.token), None)
            }
            _ => {
                return Err(reducer_error(
                    "account_state.unsupported_event",
                    "account reducer received an unsupported event",
                ));
            }
        };
        let record = Self {
            event_id: event.event_id().clone(),
            event_kind: event.event_kind(),
            account_ids: event.account_addresses().to_vec(),
            market_ids: event.market_ids().to_vec(),
            asset_id,
            vault_id,
            block_height: event.block_height(),
            payload_hash: event.payload_hash(),
            rule_version: CanonicalAccountReducerV1::VERSION.to_owned(),
        };
        record.validate().map_err(codec_reducer_error)?;
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
        EventKind::InternalTransfer => {
            account_count == 2 && market_count == 0 && !has_asset && !has_vault
        }
        EventKind::AccountClassTransfer | EventKind::RewardClaimed => {
            account_count == 1 && market_count == 0 && !has_asset && !has_vault
        }
        EventKind::SpotGenesisApplied => {
            account_count <= 1 && market_count == 0 && has_asset && !has_vault
        }
        _ => false,
    }
}

fn validate_event_identities(event: &CanonicalEventEnvelope) -> Result<(), ReducerError> {
    let matches = match event.payload() {
        EventPayload::DepositCredited(payload) => {
            identity_matches(event, &[payload.account_id], &[])
        }
        EventPayload::WithdrawalDebited(payload) => {
            identity_matches(event, &[payload.account_id], &[])
        }
        EventPayload::SpotTransfer(payload) => identity_matches(
            event,
            &[payload.from_account_id, payload.to_account_id],
            &[],
        ),
        EventPayload::PerpTransfer(payload) => identity_matches(
            event,
            &[payload.from_account_id, payload.to_account_id],
            &[],
        ),
        EventPayload::SubaccountTransfer(payload) => identity_matches(
            event,
            &[
                payload.master_account_id,
                payload.from_account_id,
                payload.to_account_id,
            ],
            &[],
        ),
        EventPayload::VaultDeposit(payload) => identity_matches(event, &[payload.account_id], &[]),
        EventPayload::VaultWithdrawal(payload) => {
            identity_matches(event, &[payload.account_id], &[])
        }
        EventPayload::FeeCharged(payload) => identity_matches(event, &[payload.account_id], &[]),
        EventPayload::BuilderFeeCharged(payload) => identity_matches(
            event,
            &[payload.account_id, payload.builder_account_id],
            &[],
        ),
        EventPayload::FundingPaid(payload) => identity_matches(
            event,
            &[payload.account_id],
            std::slice::from_ref(&payload.market_id),
        ),
        EventPayload::FundingReceived(payload) => identity_matches(
            event,
            &[payload.account_id],
            std::slice::from_ref(&payload.market_id),
        ),
        EventPayload::ReferralReward(payload) => identity_matches(
            event,
            &[payload.account_id, payload.referrer_account_id],
            &[],
        ),
        EventPayload::AccountModeChanged(payload) => {
            identity_matches(event, &[payload.account_id], &[])
        }
        EventPayload::MarginModeChanged(payload) => identity_matches(
            event,
            &[payload.account_id],
            std::slice::from_ref(&payload.market_id),
        ),
        EventPayload::LeverageChanged(payload) => identity_matches(
            event,
            &[payload.account_id],
            std::slice::from_ref(&payload.market_id),
        ),
        EventPayload::InternalTransfer(payload) => {
            let decoded = decode_internal(payload)?;
            identity_matches(
                event,
                &[decoded.from_account_id, decoded.to_account_id],
                &[],
            )
        }
        EventPayload::AccountClassTransfer(payload) => {
            let decoded = decode_class_transfer(payload)?;
            identity_matches(event, &[decoded.account_id], &[])
        }
        EventPayload::RewardClaimed(payload) => {
            let decoded = decode_reward(payload)?;
            identity_matches(event, &[decoded.account_id], &[])
        }
        EventPayload::SpotGenesisApplied(_) => {
            event.account_addresses().len() <= 1 && event.market_ids().is_empty()
        }
        _ => false,
    };
    if matches {
        Ok(())
    } else {
        Err(reducer_error(
            "account_state.identity_mismatch",
            "ordered envelope identities do not match the payload",
        ))
    }
}

fn identity_matches(
    event: &CanonicalEventEnvelope,
    account_ids: &[Address],
    market_ids: &[MarketId],
) -> bool {
    event.account_addresses() == account_ids && event.market_ids() == market_ids
}

fn validate_prerequisites(
    state: &StateView<'_>,
    event: &CanonicalEventEnvelope,
) -> Result<(), ReducerError> {
    let genesis = match event.payload() {
        EventPayload::SpotGenesisApplied(payload) => Some(decode_spot_genesis(payload)?),
        _ => None,
    };
    let asset_id = match event.payload() {
        EventPayload::DepositCredited(payload) => Some(&payload.asset_id),
        EventPayload::WithdrawalDebited(payload) => Some(&payload.asset_id),
        EventPayload::SpotTransfer(payload) => Some(&payload.asset_id),
        EventPayload::SubaccountTransfer(payload) => Some(&payload.asset_id),
        EventPayload::FeeCharged(payload) => Some(&payload.asset_id),
        EventPayload::BuilderFeeCharged(payload) => Some(&payload.asset_id),
        EventPayload::ReferralReward(payload) => Some(&payload.asset_id),
        EventPayload::SpotGenesisApplied(_) => genesis.as_ref().map(|decoded| &decoded.token),
        _ => None,
    };
    if let Some(asset_id) = asset_id {
        let key = AssetContextCurrentRecordV1::state_key(asset_id).map_err(|_| {
            reducer_error(
                "account_state.codec_error",
                "asset prerequisite key construction failed",
            )
        })?;
        let bytes = state.get(&key).ok_or_else(|| {
            reducer_error(
                "account_state.asset_prerequisite_missing",
                "asset prerequisite is missing",
            )
        })?;
        AssetContextCurrentRecordV1::decode_at(&key, bytes).map_err(|_| {
            reducer_error(
                "account_state.asset_prerequisite_invalid",
                "asset prerequisite is corrupt or key mismatched",
            )
        })?;
    }

    let market_id = match event.payload() {
        EventPayload::FundingPaid(payload) => Some(&payload.market_id),
        EventPayload::FundingReceived(payload) => Some(&payload.market_id),
        EventPayload::MarginModeChanged(payload) => Some(&payload.market_id),
        EventPayload::LeverageChanged(payload) => Some(&payload.market_id),
        _ => None,
    };
    if let Some(market_id) = market_id {
        let key = MarketCurrentRecordV1::state_key(market_id).map_err(|_| {
            reducer_error(
                "account_state.codec_error",
                "market prerequisite key construction failed",
            )
        })?;
        let bytes = state.get(&key).ok_or_else(|| {
            reducer_error(
                "account_state.market_prerequisite_missing",
                "market prerequisite is missing",
            )
        })?;
        let market = MarketCurrentRecordV1::decode_at(&key, bytes).map_err(|_| {
            reducer_error(
                "account_state.market_prerequisite_invalid",
                "market prerequisite is corrupt or key mismatched",
            )
        })?;
        if market.metadata_resolution() != MarketMetadataResolutionV1::Exact {
            return Err(reducer_error(
                "account_state.market_metadata_unresolved",
                "market prerequisite metadata is unresolved",
            ));
        }
    }
    Ok(())
}

fn ensure_unique_mutation_keys(mutations: &[StateMutation]) -> Result<(), ReducerError> {
    let mut keys = BTreeSet::new();
    if mutations.iter().all(|mutation| keys.insert(mutation.key())) {
        Ok(())
    } else {
        Err(reducer_error(
            "account_state.duplicate_mutation_key",
            "account event produced duplicate mutation keys",
        ))
    }
}

fn reducer_error(reason_code: &'static str, message: &'static str) -> ReducerError {
    ReducerError::from_static(reason_code, message)
}

fn codec_reducer_error(error: AccountStateError) -> ReducerError {
    ReducerError::from_static(
        error.reason_code(),
        "account state codec or key operation failed",
    )
}

fn current_record_reducer_error(_error: AccountStateError) -> ReducerError {
    reducer_error(
        "account_state.current_record_invalid",
        "existing account current record is corrupt or key mismatched",
    )
}

fn flow_reducer_error(_error: ValueError) -> ReducerError {
    reducer_error(
        "account_state.flow_arithmetic",
        "account flow arithmetic or exact scale expansion failed",
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::str::FromStr;

    use canonical_events::{CanonicalEventEnvelope, ConfirmationClass};
    use domain_types::{AccountAbstractionModeV1, Leverage, MarginModeV1, Quantity, QuoteAmount};

    use super::*;

    const ACCOUNT_A: Address = Address::from_bytes([0x11; 20]);
    const ACCOUNT_B: Address = Address::from_bytes([0x22; 20]);

    #[test]
    fn direct_reduce_misuse_and_duplicate_mutation_keys_fail_explicitly() {
        let event = CanonicalEventEnvelope::fixture().unwrap();
        let entries = BTreeMap::new();
        let state = crate::state::view_entries(&entries);
        let context = ApplyContext::new(
            event.chain_id(),
            event.block_height(),
            event.block_time(),
            ConfirmationClass::CommittedPrimary,
        );
        let error =
            EventReducer::reduce(&CanonicalAccountReducerV1, &state, &event, &context).unwrap_err();
        assert_eq!(error.reason_code(), "account_state.unsupported_event");

        let key = StateKey::try_new("test.v1", vec![1]).unwrap();
        let mutations = vec![
            StateMutation::put(key.clone(), vec![1]),
            StateMutation::put(key, vec![2]),
        ];
        let error = ensure_unique_mutation_keys(&mutations).unwrap_err();
        assert_eq!(error.reason_code(), "account_state.duplicate_mutation_key");
    }

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
            AccountQuantityFlowScopeV1::SpotGenesisAsset {
                asset_id: asset_id.clone(),
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
            AccountQuoteFlowScopeV1::SpotClassQuote,
            AccountQuoteFlowScopeV1::RewardClaimedQuote,
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
