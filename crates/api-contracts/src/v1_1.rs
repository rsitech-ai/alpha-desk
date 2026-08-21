use prost::Message;

use crate::{PayloadCodecError, generated, unwrap_payload, wrap_payload};

macro_rules! string_payload {
    ($wire:ident, $proto:ident, $encode:ident, $decode:ident, $kind:literal { $($field:ident),+ $(,)? }) => {
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub struct $wire {
            $(pub $field: String,)+
        }

        pub fn $encode(value: &$wire) -> Result<Vec<u8>, PayloadCodecError> {
            Ok(wrap_payload(
                $kind,
                generated::hl::canonical::v1::$proto {
                    $($field: value.$field.clone(),)+
                }
                .encode_to_vec(),
            ))
        }

        pub fn $decode(bytes: &[u8]) -> Result<$wire, PayloadCodecError> {
            let message = generated::hl::canonical::v1::$proto::decode(
                unwrap_payload($kind, bytes)?.as_slice(),
            )
            .map_err(|source| PayloadCodecError::Decode {
                kind: $kind.to_owned(),
                source,
            })?;
            Ok($wire {
                $($field: message.$field,)+
            })
        }
    };
}

string_payload!(
    WireNonUserOrderCancelled,
    NonUserOrderCancelled,
    encode_non_user_order_cancelled,
    decode_non_user_order_cancelled,
    "NonUserOrderCancelled" {
        order_id,
        reason,
        remaining_quantity,
    }
);
string_payload!(
    WireInternalTransfer,
    InternalTransfer,
    encode_internal_transfer,
    decode_internal_transfer,
    "InternalTransfer" {
        from_account_id,
        to_account_id,
        amount,
        fee,
    }
);
string_payload!(
    WireVaultCreated,
    VaultCreated,
    encode_vault_created,
    decode_vault_created,
    "VaultCreated" { vault_id, amount, fee }
);
string_payload!(
    WireVaultDistribution,
    VaultDistribution,
    encode_vault_distribution,
    decode_vault_distribution,
    "VaultDistribution" { vault_id, amount }
);
string_payload!(
    WireVaultLeaderCommissionPaid,
    VaultLeaderCommissionPaid,
    encode_vault_leader_commission_paid,
    decode_vault_leader_commission_paid,
    "VaultLeaderCommissionPaid" {
        vault_id,
        account_id,
        amount,
    }
);
string_payload!(
    WireRewardClaimed,
    RewardClaimed,
    encode_reward_claimed,
    decode_reward_claimed,
    "RewardClaimed" { account_id, amount }
);
string_payload!(
    WireSpotGenesisApplied,
    SpotGenesisApplied,
    encode_spot_genesis_applied,
    decode_spot_genesis_applied,
    "SpotGenesisApplied" { token, amount }
);
string_payload!(
    WireStakingDeposit,
    StakingDeposit,
    encode_staking_deposit,
    decode_staking_deposit,
    "StakingDeposit" { account_id, amount }
);
string_payload!(
    WireStakingDelegated,
    StakingDelegated,
    encode_staking_delegated,
    decode_staking_delegated,
    "StakingDelegated" {
        account_id,
        validator,
        amount,
    }
);
string_payload!(
    WireStakingUndelegated,
    StakingUndelegated,
    encode_staking_undelegated,
    decode_staking_undelegated,
    "StakingUndelegated" {
        account_id,
        validator,
        amount,
    }
);
string_payload!(
    WireStakingWithdrawalQueued,
    StakingWithdrawalQueued,
    encode_staking_withdrawal_queued,
    decode_staking_withdrawal_queued,
    "StakingWithdrawalQueued" { account_id, amount }
);
string_payload!(
    WireStakingWithdrawalCompleted,
    StakingWithdrawalCompleted,
    encode_staking_withdrawal_completed,
    decode_staking_withdrawal_completed,
    "StakingWithdrawalCompleted" { account_id, amount }
);
string_payload!(
    WireValidatorRewardPaid,
    ValidatorRewardPaid,
    encode_validator_reward_paid,
    decode_validator_reward_paid,
    "ValidatorRewardPaid" { validator, amount }
);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireAccountClassTransfer {
    pub account_id: String,
    pub amount: String,
    pub to_perp: bool,
}

pub fn encode_account_class_transfer(
    value: &WireAccountClassTransfer,
) -> Result<Vec<u8>, PayloadCodecError> {
    Ok(wrap_payload(
        "AccountClassTransfer",
        generated::hl::canonical::v1::AccountClassTransfer {
            account_id: value.account_id.clone(),
            amount: value.amount.clone(),
            to_perp: value.to_perp,
        }
        .encode_to_vec(),
    ))
}

pub fn decode_account_class_transfer(
    bytes: &[u8],
) -> Result<WireAccountClassTransfer, PayloadCodecError> {
    let message = generated::hl::canonical::v1::AccountClassTransfer::decode(
        unwrap_payload("AccountClassTransfer", bytes)?.as_slice(),
    )
    .map_err(|source| PayloadCodecError::Decode {
        kind: "AccountClassTransfer".to_owned(),
        source,
    })?;
    Ok(WireAccountClassTransfer {
        account_id: message.account_id,
        amount: message.amount,
        to_perp: message.to_perp,
    })
}

pub fn encode_v1_1_default_payload(kind: &str) -> Option<Result<Vec<u8>, PayloadCodecError>> {
    let message = match kind {
        "NonUserOrderCancelled" => {
            crate::default_message::<generated::hl::canonical::v1::NonUserOrderCancelled>()
        }
        "InternalTransfer" => {
            crate::default_message::<generated::hl::canonical::v1::InternalTransfer>()
        }
        "AccountClassTransfer" => {
            crate::default_message::<generated::hl::canonical::v1::AccountClassTransfer>()
        }
        "VaultCreated" => crate::default_message::<generated::hl::canonical::v1::VaultCreated>(),
        "VaultDistribution" => {
            crate::default_message::<generated::hl::canonical::v1::VaultDistribution>()
        }
        "VaultLeaderCommissionPaid" => {
            crate::default_message::<generated::hl::canonical::v1::VaultLeaderCommissionPaid>()
        }
        "RewardClaimed" => crate::default_message::<generated::hl::canonical::v1::RewardClaimed>(),
        "SpotGenesisApplied" => {
            crate::default_message::<generated::hl::canonical::v1::SpotGenesisApplied>()
        }
        "StakingDeposit" => {
            crate::default_message::<generated::hl::canonical::v1::StakingDeposit>()
        }
        "StakingDelegated" => {
            crate::default_message::<generated::hl::canonical::v1::StakingDelegated>()
        }
        "StakingUndelegated" => {
            crate::default_message::<generated::hl::canonical::v1::StakingUndelegated>()
        }
        "StakingWithdrawalQueued" => {
            crate::default_message::<generated::hl::canonical::v1::StakingWithdrawalQueued>()
        }
        "StakingWithdrawalCompleted" => {
            crate::default_message::<generated::hl::canonical::v1::StakingWithdrawalCompleted>()
        }
        "ValidatorRewardPaid" => {
            crate::default_message::<generated::hl::canonical::v1::ValidatorRewardPaid>()
        }
        _ => return None,
    };
    Some(Ok(wrap_payload(kind, message)))
}

pub fn validate_v1_1_event_payload(
    kind: &str,
    bytes: &[u8],
) -> Option<Result<(), PayloadCodecError>> {
    let result = match kind {
        "NonUserOrderCancelled" => decode_non_user_order_cancelled(bytes).map(|_| ()),
        "InternalTransfer" => decode_internal_transfer(bytes).map(|_| ()),
        "AccountClassTransfer" => decode_account_class_transfer(bytes).map(|_| ()),
        "VaultCreated" => decode_vault_created(bytes).map(|_| ()),
        "VaultDistribution" => decode_vault_distribution(bytes).map(|_| ()),
        "VaultLeaderCommissionPaid" => decode_vault_leader_commission_paid(bytes).map(|_| ()),
        "RewardClaimed" => decode_reward_claimed(bytes).map(|_| ()),
        "SpotGenesisApplied" => decode_spot_genesis_applied(bytes).map(|_| ()),
        "StakingDeposit" => decode_staking_deposit(bytes).map(|_| ()),
        "StakingDelegated" => decode_staking_delegated(bytes).map(|_| ()),
        "StakingUndelegated" => decode_staking_undelegated(bytes).map(|_| ()),
        "StakingWithdrawalQueued" => decode_staking_withdrawal_queued(bytes).map(|_| ()),
        "StakingWithdrawalCompleted" => decode_staking_withdrawal_completed(bytes).map(|_| ()),
        "ValidatorRewardPaid" => decode_validator_reward_paid(bytes).map(|_| ()),
        _ => return None,
    };
    Some(result)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireCanonicalSnapshotEnvelope {
    pub schema_version: String,
    pub family: i32,
    pub class: i32,
    pub chain_id: String,
    pub as_of_block: Option<u64>,
    pub observed_at_micros: i64,
    pub payload_hash: Vec<u8>,
    pub parser_version: String,
    pub payload: Vec<u8>,
}

impl WireCanonicalSnapshotEnvelope {
    #[must_use]
    pub fn encode_to_vec(&self) -> Vec<u8> {
        generated::hl::canonical::v1::CanonicalSnapshotEnvelope::from(self).encode_to_vec()
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, prost::DecodeError> {
        generated::hl::canonical::v1::CanonicalSnapshotEnvelope::decode(bytes).map(Into::into)
    }
}

impl From<&WireCanonicalSnapshotEnvelope>
    for generated::hl::canonical::v1::CanonicalSnapshotEnvelope
{
    fn from(value: &WireCanonicalSnapshotEnvelope) -> Self {
        Self {
            schema_version: value.schema_version.clone(),
            family: value.family,
            class: value.class,
            chain_id: value.chain_id.clone(),
            as_of_block: value.as_of_block,
            observed_at_micros: value.observed_at_micros,
            payload_hash: value.payload_hash.clone(),
            parser_version: value.parser_version.clone(),
            payload: value.payload.clone(),
        }
    }
}

impl From<generated::hl::canonical::v1::CanonicalSnapshotEnvelope>
    for WireCanonicalSnapshotEnvelope
{
    fn from(value: generated::hl::canonical::v1::CanonicalSnapshotEnvelope) -> Self {
        Self {
            schema_version: value.schema_version,
            family: value.family,
            class: value.class,
            chain_id: value.chain_id,
            as_of_block: value.as_of_block,
            observed_at_micros: value.observed_at_micros,
            payload_hash: value.payload_hash,
            parser_version: value.parser_version,
            payload: value.payload,
        }
    }
}

pub fn encode_canonical_snapshot(value: &WireCanonicalSnapshotEnvelope) -> Vec<u8> {
    value.encode_to_vec()
}
