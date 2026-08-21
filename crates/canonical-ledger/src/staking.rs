use std::str::FromStr;

use canonical_events::{CanonicalEventEnvelope, EventKind, EventPayload};
use domain_types::{Address, BlockHeight, EventId, Quantity};
use serde::{Deserialize, Serialize};

use crate::{
    ApplyContext, EventReducer, ReducerError, StateKey, StateMutation, StateView,
    opaque::{
        DecodedStakingDelegation, decode_staking_delegate, decode_staking_deposit_amount,
        decode_staking_undelegate, decode_staking_withdraw_completed,
        decode_staking_withdraw_queued,
    },
    record_codec::{RecordCodecError, decode_json, encode_json, framed_key},
};

const FACT_NAMESPACE: &str = "staking-fact.v1";
const LIQUID_NAMESPACE: &str = "staking-liquid-current.v1";
const PENDING_NAMESPACE: &str = "staking-pending-current.v1";
const DELEGATION_NAMESPACE: &str = "staking-delegation-current.v1";
const FACT_SCHEMA: &str = "hyperliquid-alpha-desk/staking-fact/v1";
const LIQUID_SCHEMA: &str = "hyperliquid-alpha-desk/staking-liquid-current/v1";
const PENDING_SCHEMA: &str = "hyperliquid-alpha-desk/staking-pending-current/v1";
const DELEGATION_SCHEMA: &str = "hyperliquid-alpha-desk/staking-delegation-current/v1";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CanonicalStakingReducerV1;

impl CanonicalStakingReducerV1 {
    pub const VERSION: &'static str = "hyperliquid-alpha-desk-canonical-staking@1.0.0";
}

impl EventReducer for CanonicalStakingReducerV1 {
    fn reducer_set_version(&self) -> &str {
        Self::VERSION
    }

    fn supports(&self, event: &CanonicalEventEnvelope) -> bool {
        event.schema_version() == "1.0.0"
            && matches!(
                event.event_kind(),
                EventKind::StakingDeposit
                    | EventKind::StakingDelegated
                    | EventKind::StakingUndelegated
                    | EventKind::StakingWithdrawalQueued
                    | EventKind::StakingWithdrawalCompleted
            )
    }

    fn reduce(
        &self,
        state: &StateView<'_>,
        event: &CanonicalEventEnvelope,
        _context: &ApplyContext<'_>,
    ) -> Result<Vec<StateMutation>, ReducerError> {
        if !self.supports(event) {
            return Err(unsupported());
        }
        let fact_key = StakingFactRecordV1::state_key(event.event_id()).map_err(codec_error)?;
        if state.contains_key(&fact_key) {
            return Err(reducer_error(
                "staking_state.event_identity_collision",
                "staking event identity is already present",
            ));
        }
        match event.payload() {
            EventPayload::StakingDeposit(payload) => {
                let decoded = decode_staking_deposit_amount(payload)?;
                require_account(event, decoded.account_id)?;
                let fact = StakingFactRecordV1::from_event(event, decoded.account_id, None)?;
                Ok(vec![
                    StateMutation::put(fact_key, fact.encode().map_err(codec_error)?),
                    liquid_mutation(
                        state,
                        decoded.account_id,
                        decoded.amount,
                        Flow::Credit,
                        event,
                    )?,
                ])
            }
            EventPayload::StakingDelegated(payload) => {
                let decoded = decode_staking_delegate(payload)?;
                reduce_delegation(state, event, fact_key, decoded, true)
            }
            EventPayload::StakingUndelegated(payload) => {
                let decoded = decode_staking_undelegate(payload)?;
                reduce_delegation(state, event, fact_key, decoded, false)
            }
            EventPayload::StakingWithdrawalQueued(payload) => {
                let decoded = decode_staking_withdraw_queued(payload)?;
                require_account(event, decoded.account_id)?;
                let fact = StakingFactRecordV1::from_event(event, decoded.account_id, None)?;
                Ok(vec![
                    StateMutation::put(fact_key, fact.encode().map_err(codec_error)?),
                    liquid_mutation(
                        state,
                        decoded.account_id,
                        decoded.amount,
                        Flow::Debit,
                        event,
                    )?,
                    pending_mutation(
                        state,
                        decoded.account_id,
                        decoded.amount,
                        Flow::Credit,
                        event,
                    )?,
                ])
            }
            EventPayload::StakingWithdrawalCompleted(payload) => {
                let decoded = decode_staking_withdraw_completed(payload)?;
                require_account(event, decoded.account_id)?;
                let fact = StakingFactRecordV1::from_event(event, decoded.account_id, None)?;
                Ok(vec![
                    StateMutation::put(fact_key, fact.encode().map_err(codec_error)?),
                    pending_mutation(
                        state,
                        decoded.account_id,
                        decoded.amount,
                        Flow::Debit,
                        event,
                    )?,
                ])
            }
            _ => Err(unsupported()),
        }
    }
}

fn reduce_delegation(
    state: &StateView<'_>,
    event: &CanonicalEventEnvelope,
    fact_key: StateKey,
    decoded: DecodedStakingDelegation,
    delegating: bool,
) -> Result<Vec<StateMutation>, ReducerError> {
    require_account(event, decoded.account_id)?;
    let fact = StakingFactRecordV1::from_event(
        event,
        decoded.account_id,
        Some(decoded.validator.as_str()),
    )?;
    let (liquid_side, delegated_side) = if delegating {
        (Flow::Debit, Flow::Credit)
    } else {
        (Flow::Credit, Flow::Debit)
    };
    Ok(vec![
        StateMutation::put(fact_key, fact.encode().map_err(codec_error)?),
        liquid_mutation(
            state,
            decoded.account_id,
            decoded.amount,
            liquid_side,
            event,
        )?,
        delegation_mutation(
            state,
            decoded.account_id,
            &decoded.validator,
            decoded.amount,
            delegated_side,
            event,
        )?,
    ])
}

fn require_account(
    event: &CanonicalEventEnvelope,
    account_id: Address,
) -> Result<(), ReducerError> {
    match event.account_addresses() {
        [observed] if *observed == account_id => Ok(()),
        _ => Err(reducer_error(
            "staking_state.identity_mismatch",
            "staking envelope account must match the payload user",
        )),
    }
}

#[derive(Debug, Clone, Copy)]
enum Flow {
    Credit,
    Debit,
}

fn liquid_mutation(
    state: &StateView<'_>,
    account_id: Address,
    amount: Quantity,
    side: Flow,
    event: &CanonicalEventEnvelope,
) -> Result<StateMutation, ReducerError> {
    let key = StakingLiquidCurrentRecordV1::state_key(&account_id).map_err(codec_error)?;
    let current = match state.get(&key) {
        Some(bytes) => StakingLiquidCurrentRecordV1::decode_at(&key, bytes).map_err(codec_error)?,
        None => StakingLiquidCurrentRecordV1::empty(account_id, amount, event)?,
    };
    let record = current.apply(amount, side, event.event_id(), event.block_height())?;
    Ok(StateMutation::put(
        key,
        record.encode().map_err(codec_error)?,
    ))
}

fn pending_mutation(
    state: &StateView<'_>,
    account_id: Address,
    amount: Quantity,
    side: Flow,
    event: &CanonicalEventEnvelope,
) -> Result<StateMutation, ReducerError> {
    let key = StakingPendingCurrentRecordV1::state_key(&account_id).map_err(codec_error)?;
    let current = match state.get(&key) {
        Some(bytes) => {
            StakingPendingCurrentRecordV1::decode_at(&key, bytes).map_err(codec_error)?
        }
        None => StakingPendingCurrentRecordV1::empty(account_id, amount, event)?,
    };
    let record = current.apply(amount, side, event.event_id(), event.block_height())?;
    Ok(StateMutation::put(
        key,
        record.encode().map_err(codec_error)?,
    ))
}

fn delegation_mutation(
    state: &StateView<'_>,
    account_id: Address,
    validator: &str,
    amount: Quantity,
    side: Flow,
    event: &CanonicalEventEnvelope,
) -> Result<StateMutation, ReducerError> {
    let key =
        StakingDelegationCurrentRecordV1::state_key(&account_id, validator).map_err(codec_error)?;
    let current = match state.get(&key) {
        Some(bytes) => {
            StakingDelegationCurrentRecordV1::decode_at(&key, bytes).map_err(codec_error)?
        }
        None => {
            if matches!(side, Flow::Debit) {
                return Err(reducer_error(
                    "staking_state.missing_delegation",
                    "undelegate requires an existing delegation",
                ));
            }
            StakingDelegationCurrentRecordV1::empty(
                account_id,
                validator.to_owned(),
                amount,
                event,
            )?
        }
    };
    let record = current.apply(amount, side, event.event_id(), event.block_height())?;
    Ok(StateMutation::put(
        key,
        record.encode().map_err(codec_error)?,
    ))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StakingFactRecordV1 {
    event_id: EventId,
    event_kind: EventKind,
    account_id: Address,
    validator: Option<String>,
    block_height: BlockHeight,
    payload_hash: [u8; 32],
}

impl StakingFactRecordV1 {
    pub fn state_key(event_id: &EventId) -> Result<StateKey, RecordCodecError> {
        framed_key(FACT_NAMESPACE, &[event_id.as_str().as_bytes()])
    }

    fn from_event(
        event: &CanonicalEventEnvelope,
        account_id: Address,
        validator: Option<&str>,
    ) -> Result<Self, ReducerError> {
        let record = Self {
            event_id: event.event_id().clone(),
            event_kind: event.event_kind(),
            account_id,
            validator: validator.map(ToOwned::to_owned),
            block_height: event.block_height(),
            payload_hash: event.payload_hash(),
        };
        record.validate().map_err(codec_error)?;
        Ok(record)
    }

    fn encode(&self) -> Result<Vec<u8>, RecordCodecError> {
        self.validate()?;
        encode_json(&StakingFactWire {
            schema: FACT_SCHEMA.to_owned(),
            event_id: self.event_id.as_str().to_owned(),
            event_kind: self.event_kind.as_wire_name().to_owned(),
            account_id: self.account_id.to_api_string(),
            validator: self.validator.clone(),
            block_height: self.block_height.get(),
            payload_blake3: hex::encode(self.payload_hash),
            rule_version: CanonicalStakingReducerV1::VERSION.to_owned(),
        })
    }

    fn validate(&self) -> Result<(), RecordCodecError> {
        let validator_ok = match self.event_kind {
            EventKind::StakingDelegated | EventKind::StakingUndelegated => self
                .validator
                .as_deref()
                .is_some_and(|value| !value.is_empty()),
            EventKind::StakingDeposit
            | EventKind::StakingWithdrawalQueued
            | EventKind::StakingWithdrawalCompleted => self.validator.is_none(),
            _ => false,
        };
        if validator_ok {
            Ok(())
        } else {
            Err(RecordCodecError::InvalidRecord)
        }
    }

    #[must_use]
    pub const fn event_kind(&self) -> EventKind {
        self.event_kind
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StakingLiquidCurrentRecordV1 {
    account_id: Address,
    credits: Quantity,
    debits: Quantity,
    last_event_id: EventId,
    last_block_height: BlockHeight,
}

impl StakingLiquidCurrentRecordV1 {
    pub fn state_key(account_id: &Address) -> Result<StateKey, RecordCodecError> {
        framed_key(LIQUID_NAMESPACE, &[account_id.as_bytes()])
    }

    fn empty(
        account_id: Address,
        amount: Quantity,
        event: &CanonicalEventEnvelope,
    ) -> Result<Self, ReducerError> {
        let zero = zero_of(amount)?;
        Ok(Self {
            account_id,
            credits: zero,
            debits: zero,
            last_event_id: event.event_id().clone(),
            last_block_height: event.block_height(),
        })
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, RecordCodecError> {
        decode_balance(bytes, LIQUID_SCHEMA).map(
            |(account_id, credits, debits, last_event_id, last_block_height)| Self {
                account_id,
                credits,
                debits,
                last_event_id,
                last_block_height,
            },
        )
    }

    pub fn decode_at(key: &StateKey, bytes: &[u8]) -> Result<Self, RecordCodecError> {
        let record = Self::decode(bytes)?;
        if Self::state_key(&record.account_id)? == *key {
            Ok(record)
        } else {
            Err(RecordCodecError::KeyMismatch)
        }
    }

    fn apply(
        mut self,
        amount: Quantity,
        side: Flow,
        event_id: &EventId,
        height: BlockHeight,
    ) -> Result<Self, ReducerError> {
        match side {
            Flow::Credit => {
                self.credits = self.credits.checked_add(amount).map_err(|_| arithmetic())?;
            }
            Flow::Debit => {
                self.debits = self.debits.checked_add(amount).map_err(|_| arithmetic())?;
            }
        }
        if self.debits > self.credits {
            return Err(reducer_error(
                "staking_state.insufficient_liquid",
                "staking liquid debit exceeds credits",
            ));
        }
        self.last_event_id = event_id.clone();
        self.last_block_height = height;
        Ok(self)
    }

    fn encode(&self) -> Result<Vec<u8>, RecordCodecError> {
        encode_balance(
            LIQUID_SCHEMA,
            &self.account_id,
            self.credits,
            self.debits,
            &self.last_event_id,
            self.last_block_height,
        )
    }

    #[must_use]
    pub const fn credits(&self) -> Quantity {
        self.credits
    }

    #[must_use]
    pub const fn debits(&self) -> Quantity {
        self.debits
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StakingPendingCurrentRecordV1 {
    account_id: Address,
    credits: Quantity,
    debits: Quantity,
    last_event_id: EventId,
    last_block_height: BlockHeight,
}

impl StakingPendingCurrentRecordV1 {
    pub fn state_key(account_id: &Address) -> Result<StateKey, RecordCodecError> {
        framed_key(PENDING_NAMESPACE, &[account_id.as_bytes()])
    }

    fn empty(
        account_id: Address,
        amount: Quantity,
        event: &CanonicalEventEnvelope,
    ) -> Result<Self, ReducerError> {
        let zero = zero_of(amount)?;
        Ok(Self {
            account_id,
            credits: zero,
            debits: zero,
            last_event_id: event.event_id().clone(),
            last_block_height: event.block_height(),
        })
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, RecordCodecError> {
        decode_balance(bytes, PENDING_SCHEMA).map(
            |(account_id, credits, debits, last_event_id, last_block_height)| Self {
                account_id,
                credits,
                debits,
                last_event_id,
                last_block_height,
            },
        )
    }

    pub fn decode_at(key: &StateKey, bytes: &[u8]) -> Result<Self, RecordCodecError> {
        let record = Self::decode(bytes)?;
        if Self::state_key(&record.account_id)? == *key {
            Ok(record)
        } else {
            Err(RecordCodecError::KeyMismatch)
        }
    }

    fn apply(
        mut self,
        amount: Quantity,
        side: Flow,
        event_id: &EventId,
        height: BlockHeight,
    ) -> Result<Self, ReducerError> {
        match side {
            Flow::Credit => {
                self.credits = self.credits.checked_add(amount).map_err(|_| arithmetic())?;
            }
            Flow::Debit => {
                self.debits = self.debits.checked_add(amount).map_err(|_| arithmetic())?;
            }
        }
        if self.debits > self.credits {
            return Err(reducer_error(
                "staking_state.insufficient_pending",
                "staking pending debit exceeds credits",
            ));
        }
        self.last_event_id = event_id.clone();
        self.last_block_height = height;
        Ok(self)
    }

    fn encode(&self) -> Result<Vec<u8>, RecordCodecError> {
        encode_balance(
            PENDING_SCHEMA,
            &self.account_id,
            self.credits,
            self.debits,
            &self.last_event_id,
            self.last_block_height,
        )
    }

    #[must_use]
    pub const fn credits(&self) -> Quantity {
        self.credits
    }

    #[must_use]
    pub const fn debits(&self) -> Quantity {
        self.debits
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StakingDelegationCurrentRecordV1 {
    account_id: Address,
    validator: String,
    credits: Quantity,
    debits: Quantity,
    last_event_id: EventId,
    last_block_height: BlockHeight,
}

impl StakingDelegationCurrentRecordV1 {
    pub fn state_key(account_id: &Address, validator: &str) -> Result<StateKey, RecordCodecError> {
        framed_key(
            DELEGATION_NAMESPACE,
            &[account_id.as_bytes(), validator.as_bytes()],
        )
    }

    fn empty(
        account_id: Address,
        validator: String,
        amount: Quantity,
        event: &CanonicalEventEnvelope,
    ) -> Result<Self, ReducerError> {
        let zero = zero_of(amount)?;
        Ok(Self {
            account_id,
            validator,
            credits: zero,
            debits: zero,
            last_event_id: event.event_id().clone(),
            last_block_height: event.block_height(),
        })
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, RecordCodecError> {
        let wire: DelegationWire = decode_json(bytes)?;
        if wire.schema != DELEGATION_SCHEMA {
            return Err(RecordCodecError::InvalidRecord);
        }
        let record = Self {
            account_id: Address::parse_api(&wire.account_id)
                .map_err(|_| RecordCodecError::InvalidRecord)?,
            validator: wire.validator,
            credits: Quantity::from_str(&wire.credits)
                .map_err(|_| RecordCodecError::InvalidRecord)?,
            debits: Quantity::from_str(&wire.debits)
                .map_err(|_| RecordCodecError::InvalidRecord)?,
            last_event_id: EventId::new(wire.last_event_id)
                .map_err(|_| RecordCodecError::InvalidRecord)?,
            last_block_height: BlockHeight::new(wire.last_block_height),
        };
        if record.validator.is_empty() || record.encode()? != bytes {
            return Err(RecordCodecError::InvalidRecord);
        }
        Ok(record)
    }

    pub fn decode_at(key: &StateKey, bytes: &[u8]) -> Result<Self, RecordCodecError> {
        let record = Self::decode(bytes)?;
        if Self::state_key(&record.account_id, &record.validator)? == *key {
            Ok(record)
        } else {
            Err(RecordCodecError::KeyMismatch)
        }
    }

    fn apply(
        mut self,
        amount: Quantity,
        side: Flow,
        event_id: &EventId,
        height: BlockHeight,
    ) -> Result<Self, ReducerError> {
        match side {
            Flow::Credit => {
                self.credits = self.credits.checked_add(amount).map_err(|_| arithmetic())?;
            }
            Flow::Debit => {
                self.debits = self.debits.checked_add(amount).map_err(|_| arithmetic())?;
            }
        }
        if self.debits > self.credits {
            return Err(reducer_error(
                "staking_state.insufficient_delegation",
                "undelegate exceeds delegated credits",
            ));
        }
        self.last_event_id = event_id.clone();
        self.last_block_height = height;
        Ok(self)
    }

    fn encode(&self) -> Result<Vec<u8>, RecordCodecError> {
        encode_json(&DelegationWire {
            schema: DELEGATION_SCHEMA.to_owned(),
            account_id: self.account_id.to_api_string(),
            validator: self.validator.clone(),
            credits: self.credits.to_string(),
            debits: self.debits.to_string(),
            last_event_id: self.last_event_id.as_str().to_owned(),
            last_block_height: self.last_block_height.get(),
        })
    }

    #[must_use]
    pub fn validator(&self) -> &str {
        &self.validator
    }

    #[must_use]
    pub const fn credits(&self) -> Quantity {
        self.credits
    }

    #[must_use]
    pub const fn debits(&self) -> Quantity {
        self.debits
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StakingFactWire {
    schema: String,
    event_id: String,
    event_kind: String,
    account_id: String,
    validator: Option<String>,
    block_height: u64,
    payload_blake3: String,
    rule_version: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BalanceWire {
    schema: String,
    account_id: String,
    credits: String,
    debits: String,
    last_event_id: String,
    last_block_height: u64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DelegationWire {
    schema: String,
    account_id: String,
    validator: String,
    credits: String,
    debits: String,
    last_event_id: String,
    last_block_height: u64,
}

fn decode_balance(
    bytes: &[u8],
    schema: &str,
) -> Result<(Address, Quantity, Quantity, EventId, BlockHeight), RecordCodecError> {
    let wire: BalanceWire = decode_json(bytes)?;
    if wire.schema != schema {
        return Err(RecordCodecError::InvalidRecord);
    }
    let account_id =
        Address::parse_api(&wire.account_id).map_err(|_| RecordCodecError::InvalidRecord)?;
    let credits = Quantity::from_str(&wire.credits).map_err(|_| RecordCodecError::InvalidRecord)?;
    let debits = Quantity::from_str(&wire.debits).map_err(|_| RecordCodecError::InvalidRecord)?;
    let last_event_id =
        EventId::new(wire.last_event_id).map_err(|_| RecordCodecError::InvalidRecord)?;
    if encode_balance(
        schema,
        &account_id,
        credits,
        debits,
        &last_event_id,
        BlockHeight::new(wire.last_block_height),
    )? != bytes
    {
        return Err(RecordCodecError::NonCanonical);
    }
    Ok((
        account_id,
        credits,
        debits,
        last_event_id,
        BlockHeight::new(wire.last_block_height),
    ))
}

fn encode_balance(
    schema: &str,
    account_id: &Address,
    credits: Quantity,
    debits: Quantity,
    last_event_id: &EventId,
    last_block_height: BlockHeight,
) -> Result<Vec<u8>, RecordCodecError> {
    encode_json(&BalanceWire {
        schema: schema.to_owned(),
        account_id: account_id.to_api_string(),
        credits: credits.to_string(),
        debits: debits.to_string(),
        last_event_id: last_event_id.as_str().to_owned(),
        last_block_height: last_block_height.get(),
    })
}

fn zero_of(amount: Quantity) -> Result<Quantity, ReducerError> {
    amount.checked_sub(amount).map_err(|_| arithmetic())
}

fn codec_error(error: RecordCodecError) -> ReducerError {
    match error {
        RecordCodecError::InvalidKey => reducer_error(
            "staking_state.codec_invalid_key",
            "staking state key encoding failed",
        ),
        RecordCodecError::LimitExceeded => reducer_error(
            "staking_state.codec_limit_exceeded",
            "staking state record exceeds its deterministic bound",
        ),
        RecordCodecError::Codec
        | RecordCodecError::NonCanonical
        | RecordCodecError::InvalidRecord
        | RecordCodecError::KeyMismatch => reducer_error(
            "staking_state.codec_failed",
            "staking state record is not canonical",
        ),
    }
}

fn arithmetic() -> ReducerError {
    reducer_error(
        "staking_state.flow_arithmetic",
        "staking quantity arithmetic failed",
    )
}

fn unsupported() -> ReducerError {
    reducer_error(
        "staking_state.unsupported_event",
        "staking reducer received an unsupported event",
    )
}

fn reducer_error(reason_code: &'static str, message: &'static str) -> ReducerError {
    ReducerError::from_static(reason_code, message)
}
