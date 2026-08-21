use std::str::FromStr;

use api_contracts::{
    decode_account_class_transfer, decode_internal_transfer, decode_non_user_order_cancelled,
    decode_reward_claimed, decode_spot_genesis_applied, decode_staking_delegated,
    decode_staking_deposit, decode_staking_undelegated, decode_staking_withdrawal_completed,
    decode_staking_withdrawal_queued, decode_validator_reward_paid, decode_vault_created,
    decode_vault_distribution,
};
use canonical_events::{
    AccountClassTransfer, InternalTransfer, NonUserOrderCancelled, RewardClaimed,
    SpotGenesisApplied, StakingDelegated, StakingDeposit, StakingUndelegated,
    StakingWithdrawalCompleted, StakingWithdrawalQueued, ValidatorRewardPaid, VaultCreated,
    VaultDistribution,
};
use domain_types::{Address, AssetId, OrderId, Quantity, QuoteAmount, VaultId};

use crate::ReducerError;

pub(crate) struct DecodedNonUserCancel {
    pub order_id: OrderId,
    pub remaining_quantity: Quantity,
}

pub(crate) struct DecodedInternalTransfer {
    pub from_account_id: Address,
    pub to_account_id: Address,
    pub amount: QuoteAmount,
    pub fee: QuoteAmount,
}

pub(crate) struct DecodedAccountClassTransfer {
    pub account_id: Address,
    pub amount: QuoteAmount,
    pub to_perp: bool,
}

pub(crate) struct DecodedVaultCreated {
    pub vault_id: VaultId,
    pub amount: QuoteAmount,
    pub fee: QuoteAmount,
}

pub(crate) struct DecodedVaultDistribution {
    pub vault_id: VaultId,
    pub amount: QuoteAmount,
}

pub(crate) struct DecodedRewardClaimed {
    pub account_id: Address,
    pub amount: QuoteAmount,
}

pub(crate) struct DecodedSpotGenesis {
    pub token: AssetId,
    pub amount: Quantity,
}

pub(crate) struct DecodedStakingAmount {
    pub account_id: Address,
    pub amount: Quantity,
}

pub(crate) struct DecodedStakingDelegation {
    pub account_id: Address,
    pub validator: String,
    pub amount: Quantity,
}

pub(crate) struct DecodedValidatorReward {
    pub validator: String,
    pub amount: Quantity,
}

pub(crate) fn decode_non_user_cancel(
    payload: &NonUserOrderCancelled,
) -> Result<DecodedNonUserCancel, ReducerError> {
    let wire = decode_non_user_order_cancelled(payload.encoded()).map_err(|_| payload_error())?;
    let remaining_quantity = parse_quantity(&wire.remaining_quantity)?;
    if remaining_quantity.raw() < 0 {
        return Err(reducer_error(
            "order_state.invalid_quantity",
            "order quantity must be nonnegative",
        ));
    }
    if invalid_text(&wire.reason, 1_024) {
        return Err(reducer_error(
            "order_state.invalid_cancellation",
            "system cancellation reason contract is invalid",
        ));
    }
    Ok(DecodedNonUserCancel {
        order_id: OrderId::new(wire.order_id).map_err(|_| payload_error())?,
        remaining_quantity,
    })
}

pub(crate) fn decode_internal(
    payload: &InternalTransfer,
) -> Result<DecodedInternalTransfer, ReducerError> {
    let wire = decode_internal_transfer(payload.encoded()).map_err(|_| payload_error())?;
    Ok(DecodedInternalTransfer {
        from_account_id: parse_account(&wire.from_account_id)?,
        to_account_id: parse_account(&wire.to_account_id)?,
        amount: parse_positive_quote(&wire.amount)?,
        fee: parse_nonnegative_quote(&wire.fee)?,
    })
}

pub(crate) fn decode_class_transfer(
    payload: &AccountClassTransfer,
) -> Result<DecodedAccountClassTransfer, ReducerError> {
    let wire = decode_account_class_transfer(payload.encoded()).map_err(|_| payload_error())?;
    Ok(DecodedAccountClassTransfer {
        account_id: parse_account(&wire.account_id)?,
        amount: parse_positive_quote(&wire.amount)?,
        to_perp: wire.to_perp,
    })
}

pub(crate) fn decode_vault_create(
    payload: &VaultCreated,
) -> Result<DecodedVaultCreated, ReducerError> {
    let wire = decode_vault_created(payload.encoded()).map_err(|_| payload_error())?;
    Ok(DecodedVaultCreated {
        vault_id: VaultId::new(wire.vault_id).map_err(|_| payload_error())?,
        amount: parse_positive_quote(&wire.amount)?,
        fee: parse_nonnegative_quote(&wire.fee)?,
    })
}

pub(crate) fn decode_vault_dist(
    payload: &VaultDistribution,
) -> Result<DecodedVaultDistribution, ReducerError> {
    let wire = decode_vault_distribution(payload.encoded()).map_err(|_| payload_error())?;
    Ok(DecodedVaultDistribution {
        vault_id: VaultId::new(wire.vault_id).map_err(|_| payload_error())?,
        amount: parse_positive_quote(&wire.amount)?,
    })
}

pub(crate) fn decode_reward(payload: &RewardClaimed) -> Result<DecodedRewardClaimed, ReducerError> {
    let wire = decode_reward_claimed(payload.encoded()).map_err(|_| payload_error())?;
    Ok(DecodedRewardClaimed {
        account_id: parse_account(&wire.account_id)?,
        amount: parse_positive_quote(&wire.amount)?,
    })
}

pub(crate) fn decode_spot_genesis(
    payload: &SpotGenesisApplied,
) -> Result<DecodedSpotGenesis, ReducerError> {
    let wire = decode_spot_genesis_applied(payload.encoded()).map_err(|_| payload_error())?;
    Ok(DecodedSpotGenesis {
        token: AssetId::new(wire.token).map_err(|_| payload_error())?,
        amount: parse_positive_quantity(&wire.amount)?,
    })
}

pub(crate) fn decode_staking_deposit_amount(
    payload: &StakingDeposit,
) -> Result<DecodedStakingAmount, ReducerError> {
    let wire = decode_staking_deposit(payload.encoded()).map_err(|_| payload_error())?;
    staking_amount(wire.account_id, wire.amount)
}

pub(crate) fn decode_staking_withdraw_queued(
    payload: &StakingWithdrawalQueued,
) -> Result<DecodedStakingAmount, ReducerError> {
    let wire = decode_staking_withdrawal_queued(payload.encoded()).map_err(|_| payload_error())?;
    staking_amount(wire.account_id, wire.amount)
}

pub(crate) fn decode_staking_withdraw_completed(
    payload: &StakingWithdrawalCompleted,
) -> Result<DecodedStakingAmount, ReducerError> {
    let wire =
        decode_staking_withdrawal_completed(payload.encoded()).map_err(|_| payload_error())?;
    staking_amount(wire.account_id, wire.amount)
}

pub(crate) fn decode_staking_delegate(
    payload: &StakingDelegated,
) -> Result<DecodedStakingDelegation, ReducerError> {
    let wire = decode_staking_delegated(payload.encoded()).map_err(|_| payload_error())?;
    staking_delegation(wire.account_id, wire.validator, wire.amount)
}

pub(crate) fn decode_staking_undelegate(
    payload: &StakingUndelegated,
) -> Result<DecodedStakingDelegation, ReducerError> {
    let wire = decode_staking_undelegated(payload.encoded()).map_err(|_| payload_error())?;
    staking_delegation(wire.account_id, wire.validator, wire.amount)
}

pub(crate) fn decode_validator_reward(
    payload: &ValidatorRewardPaid,
) -> Result<DecodedValidatorReward, ReducerError> {
    let wire = decode_validator_reward_paid(payload.encoded()).map_err(|_| payload_error())?;
    Ok(DecodedValidatorReward {
        validator: require_identity(&wire.validator)?,
        amount: parse_positive_quantity(&wire.amount)?,
    })
}

fn staking_amount(
    account_id: String,
    amount: String,
) -> Result<DecodedStakingAmount, ReducerError> {
    Ok(DecodedStakingAmount {
        account_id: parse_account(&account_id)?,
        amount: parse_positive_quantity(&amount)?,
    })
}

fn staking_delegation(
    account_id: String,
    validator: String,
    amount: String,
) -> Result<DecodedStakingDelegation, ReducerError> {
    Ok(DecodedStakingDelegation {
        account_id: parse_account(&account_id)?,
        validator: require_identity(&validator)?,
        amount: parse_positive_quantity(&amount)?,
    })
}

fn parse_account(value: &str) -> Result<Address, ReducerError> {
    Address::parse_api(value).map_err(|_| payload_error())
}

fn parse_quantity(value: &str) -> Result<Quantity, ReducerError> {
    Quantity::from_str(value).map_err(|_| empty_amount())
}

fn parse_positive_quantity(value: &str) -> Result<Quantity, ReducerError> {
    let quantity = parse_quantity(value)?;
    if quantity.raw() <= 0 {
        return Err(empty_amount());
    }
    Ok(quantity)
}

fn parse_quote(value: &str) -> Result<QuoteAmount, ReducerError> {
    QuoteAmount::from_str(value).map_err(|_| empty_amount())
}

fn parse_positive_quote(value: &str) -> Result<QuoteAmount, ReducerError> {
    let amount = parse_quote(value)?;
    if amount.raw() <= 0 {
        return Err(empty_amount());
    }
    Ok(amount)
}

fn parse_nonnegative_quote(value: &str) -> Result<QuoteAmount, ReducerError> {
    let amount = parse_quote(value)?;
    if amount.raw() < 0 {
        return Err(empty_amount());
    }
    Ok(amount)
}

pub(crate) fn require_identity(value: &str) -> Result<String, ReducerError> {
    if value.is_empty() || value.trim() != value || value.chars().any(char::is_control) {
        return Err(payload_error());
    }
    Ok(value.to_owned())
}

fn invalid_text(value: &str, max_bytes: usize) -> bool {
    value.is_empty()
        || value.len() > max_bytes
        || value.trim() != value
        || value.chars().any(char::is_control)
}

fn payload_error() -> ReducerError {
    reducer_error(
        "canonical_state.invalid_payload",
        "v1.1 payload fields are missing, empty, or not canonical",
    )
}

fn empty_amount() -> ReducerError {
    reducer_error(
        "canonical_state.empty_amount",
        "v1.1 amount fields must be nonempty canonical decimals",
    )
}

fn reducer_error(reason_code: &'static str, message: &'static str) -> ReducerError {
    ReducerError::from_static(reason_code, message)
}
