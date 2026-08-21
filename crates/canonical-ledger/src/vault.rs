use std::str::FromStr;

use canonical_events::{CanonicalEventEnvelope, EventKind, EventPayload};
use domain_types::{Address, BlockHeight, EventId, QuoteAmount, VaultId};
use serde::{Deserialize, Serialize};

use crate::{
    ApplyContext, EventReducer, ReducerError, StateKey, StateMutation, StateView,
    account::AccountQuoteFlowScopeV1,
    opaque::{
        DecodedVaultCreated, DecodedVaultDistribution, decode_vault_create, decode_vault_dist,
    },
    record_codec::{RecordCodecError, decode_json, encode_json, framed_key},
};

const FACT_NAMESPACE: &str = "vault-fact.v1";
const CURRENT_NAMESPACE: &str = "vault-current.v1";
const FACT_SCHEMA: &str = "hyperliquid-alpha-desk/vault-fact/v1";
const CURRENT_SCHEMA: &str = "hyperliquid-alpha-desk/vault-current/v1";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CanonicalVaultReducerV1;

impl CanonicalVaultReducerV1 {
    pub const VERSION: &'static str = "hyperliquid-alpha-desk-canonical-vault@1.0.0";
}

impl EventReducer for CanonicalVaultReducerV1 {
    fn reducer_set_version(&self) -> &str {
        Self::VERSION
    }

    fn supports(&self, event: &CanonicalEventEnvelope) -> bool {
        event.schema_version() == "1.0.0"
            && matches!(
                event.event_kind(),
                EventKind::VaultCreated | EventKind::VaultDistribution
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
                "vault_state.unsupported_event",
                "vault reducer received an unsupported event",
            ));
        }
        let fact_key = VaultFactRecordV1::state_key(event.event_id()).map_err(codec_error)?;
        if state.contains_key(&fact_key) {
            return Err(reducer_error(
                "vault_state.event_identity_collision",
                "vault event identity is already present",
            ));
        }
        match event.payload() {
            EventPayload::VaultCreated(payload) => {
                reduce_created(state, event, decode_vault_create(payload)?, fact_key)
            }
            EventPayload::VaultDistribution(payload) => {
                reduce_distribution(state, event, decode_vault_dist(payload)?, fact_key)
            }
            _ => Err(reducer_error(
                "vault_state.unsupported_event",
                "vault reducer received an unsupported event",
            )),
        }
    }
}

fn reduce_created(
    state: &StateView<'_>,
    event: &CanonicalEventEnvelope,
    decoded: DecodedVaultCreated,
    fact_key: StateKey,
) -> Result<Vec<StateMutation>, ReducerError> {
    let account = optional_single_account(event)?;
    let current_key = VaultCurrentRecordV1::state_key(&decoded.vault_id).map_err(codec_error)?;
    if state.contains_key(&current_key) {
        return Err(reducer_error(
            "vault_state.vault_id_collision",
            "vault identity is already present",
        ));
    }
    let fact = VaultFactRecordV1::from_event(event, Some(&decoded.vault_id))?;
    let current = VaultCurrentRecordV1 {
        vault_id: decoded.vault_id.clone(),
        created_event_id: event.event_id().clone(),
        last_event_id: event.event_id().clone(),
        first_block_height: event.block_height(),
        last_block_height: event.block_height(),
        creation_amount: decoded.amount,
        creation_fee: decoded.fee,
    };
    let mut mutations = vec![
        StateMutation::put(fact_key, fact.encode().map_err(codec_error)?),
        StateMutation::put(current_key, current.encode().map_err(codec_error)?),
        crate::account::vault_principal_put(
            state,
            &decoded.vault_id,
            decoded.amount,
            crate::account::FlowSide::Credit,
            event.event_id(),
            event.block_height(),
        )?,
    ];
    if let Some(account_id) = account {
        let debit = decoded
            .amount
            .checked_add(decoded.fee)
            .map_err(|_| arithmetic_error())?;
        mutations.push(crate::account::quote_flow_put(
            state,
            account_id,
            AccountQuoteFlowScopeV1::VaultPrincipal {
                vault_id: decoded.vault_id,
            },
            debit,
            crate::account::FlowSide::Debit,
            event.event_id(),
            event.block_height(),
        )?);
    }
    Ok(mutations)
}

fn reduce_distribution(
    state: &StateView<'_>,
    event: &CanonicalEventEnvelope,
    decoded: DecodedVaultDistribution,
    fact_key: StateKey,
) -> Result<Vec<StateMutation>, ReducerError> {
    let account = optional_single_account(event)?;
    let current_key = VaultCurrentRecordV1::state_key(&decoded.vault_id).map_err(codec_error)?;
    let mut current = load_current(state, &current_key)?;
    current.last_event_id = event.event_id().clone();
    current.last_block_height = event.block_height();
    let fact = VaultFactRecordV1::from_event(event, Some(&decoded.vault_id))?;
    let mut mutations = vec![
        StateMutation::put(fact_key, fact.encode().map_err(codec_error)?),
        StateMutation::put(current_key, current.encode().map_err(codec_error)?),
        crate::account::vault_principal_put(
            state,
            &decoded.vault_id,
            decoded.amount,
            crate::account::FlowSide::Debit,
            event.event_id(),
            event.block_height(),
        )?,
    ];
    if let Some(account_id) = account {
        mutations.push(crate::account::quote_flow_put(
            state,
            account_id,
            AccountQuoteFlowScopeV1::VaultPrincipal {
                vault_id: decoded.vault_id,
            },
            decoded.amount,
            crate::account::FlowSide::Credit,
            event.event_id(),
            event.block_height(),
        )?);
    }
    Ok(mutations)
}

fn optional_single_account(
    event: &CanonicalEventEnvelope,
) -> Result<Option<Address>, ReducerError> {
    match event.account_addresses() {
        [] => Ok(None),
        [account] => Ok(Some(*account)),
        _ => Err(reducer_error(
            "vault_state.ambiguous_accounts",
            "vault event with a lumped amount cannot split across multiple accounts",
        )),
    }
}

fn load_current(
    state: &StateView<'_>,
    key: &StateKey,
) -> Result<VaultCurrentRecordV1, ReducerError> {
    let bytes = state.get(key).ok_or_else(|| {
        reducer_error(
            "vault_state.missing_vault",
            "vault must exist before distribution",
        )
    })?;
    VaultCurrentRecordV1::decode_at(key, bytes).map_err(codec_error)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultFactRecordV1 {
    event_id: EventId,
    event_kind: EventKind,
    vault_id: Option<VaultId>,
    account_ids: Vec<Address>,
    block_height: BlockHeight,
    payload_hash: [u8; 32],
}

impl VaultFactRecordV1 {
    pub fn state_key(event_id: &EventId) -> Result<StateKey, RecordCodecError> {
        framed_key(FACT_NAMESPACE, &[event_id.as_str().as_bytes()])
    }

    fn from_event(
        event: &CanonicalEventEnvelope,
        vault_id: Option<&VaultId>,
    ) -> Result<Self, ReducerError> {
        let record = Self {
            event_id: event.event_id().clone(),
            event_kind: event.event_kind(),
            vault_id: vault_id.cloned(),
            account_ids: event.account_addresses().to_vec(),
            block_height: event.block_height(),
            payload_hash: event.payload_hash(),
        };
        record.validate().map_err(codec_error)?;
        Ok(record)
    }

    fn encode(&self) -> Result<Vec<u8>, RecordCodecError> {
        self.validate()?;
        encode_json(&VaultFactWire {
            schema: FACT_SCHEMA.to_owned(),
            event_id: self.event_id.as_str().to_owned(),
            event_kind: self.event_kind.as_wire_name().to_owned(),
            vault_id: self
                .vault_id
                .as_ref()
                .map(|value| value.as_str().to_owned()),
            account_ids: self
                .account_ids
                .iter()
                .map(|value| value.to_api_string())
                .collect(),
            block_height: self.block_height.get(),
            payload_blake3: hex::encode(self.payload_hash),
            rule_version: CanonicalVaultReducerV1::VERSION.to_owned(),
        })
    }

    fn validate(&self) -> Result<(), RecordCodecError> {
        if matches!(
            self.event_kind,
            EventKind::VaultCreated | EventKind::VaultDistribution
        ) && self.vault_id.is_some()
            && self.account_ids.len() <= 1
        {
            Ok(())
        } else {
            Err(RecordCodecError::InvalidRecord)
        }
    }

    #[must_use]
    pub const fn event_kind(&self) -> EventKind {
        self.event_kind
    }

    #[must_use]
    pub const fn vault_id(&self) -> Option<&VaultId> {
        self.vault_id.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultCurrentRecordV1 {
    vault_id: VaultId,
    created_event_id: EventId,
    last_event_id: EventId,
    first_block_height: BlockHeight,
    last_block_height: BlockHeight,
    creation_amount: QuoteAmount,
    creation_fee: QuoteAmount,
}

impl VaultCurrentRecordV1 {
    pub fn state_key(vault_id: &VaultId) -> Result<StateKey, RecordCodecError> {
        framed_key(CURRENT_NAMESPACE, &[vault_id.as_str().as_bytes()])
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, RecordCodecError> {
        let wire: VaultCurrentWire = decode_json(bytes)?;
        if wire.schema != CURRENT_SCHEMA {
            return Err(RecordCodecError::InvalidRecord);
        }
        let record = Self {
            vault_id: VaultId::new(wire.vault_id).map_err(|_| RecordCodecError::InvalidRecord)?,
            created_event_id: EventId::new(wire.created_event_id)
                .map_err(|_| RecordCodecError::InvalidRecord)?,
            last_event_id: EventId::new(wire.last_event_id)
                .map_err(|_| RecordCodecError::InvalidRecord)?,
            first_block_height: BlockHeight::new(wire.first_block_height),
            last_block_height: BlockHeight::new(wire.last_block_height),
            creation_amount: QuoteAmount::from_str(&wire.creation_amount)
                .map_err(|_| RecordCodecError::InvalidRecord)?,
            creation_fee: QuoteAmount::from_str(&wire.creation_fee)
                .map_err(|_| RecordCodecError::InvalidRecord)?,
        };
        record.validate()?;
        if record.encode()? != bytes {
            return Err(RecordCodecError::NonCanonical);
        }
        Ok(record)
    }

    pub fn decode_at(key: &StateKey, bytes: &[u8]) -> Result<Self, RecordCodecError> {
        let record = Self::decode(bytes)?;
        if Self::state_key(&record.vault_id)? == *key {
            Ok(record)
        } else {
            Err(RecordCodecError::KeyMismatch)
        }
    }

    fn encode(&self) -> Result<Vec<u8>, RecordCodecError> {
        self.validate()?;
        encode_json(&VaultCurrentWire {
            schema: CURRENT_SCHEMA.to_owned(),
            vault_id: self.vault_id.as_str().to_owned(),
            created_event_id: self.created_event_id.as_str().to_owned(),
            last_event_id: self.last_event_id.as_str().to_owned(),
            first_block_height: self.first_block_height.get(),
            last_block_height: self.last_block_height.get(),
            creation_amount: self.creation_amount.to_string(),
            creation_fee: self.creation_fee.to_string(),
        })
    }

    fn validate(&self) -> Result<(), RecordCodecError> {
        if self.first_block_height <= self.last_block_height
            && self.creation_amount.raw() > 0
            && self.creation_fee.raw() >= 0
        {
            Ok(())
        } else {
            Err(RecordCodecError::InvalidRecord)
        }
    }

    #[must_use]
    pub const fn vault_id(&self) -> &VaultId {
        &self.vault_id
    }

    #[must_use]
    pub const fn creation_amount(&self) -> QuoteAmount {
        self.creation_amount
    }

    #[must_use]
    pub const fn creation_fee(&self) -> QuoteAmount {
        self.creation_fee
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct VaultFactWire {
    schema: String,
    event_id: String,
    event_kind: String,
    vault_id: Option<String>,
    account_ids: Vec<String>,
    block_height: u64,
    payload_blake3: String,
    rule_version: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct VaultCurrentWire {
    schema: String,
    vault_id: String,
    created_event_id: String,
    last_event_id: String,
    first_block_height: u64,
    last_block_height: u64,
    creation_amount: String,
    creation_fee: String,
}

fn codec_error(error: RecordCodecError) -> ReducerError {
    match error {
        RecordCodecError::InvalidKey => reducer_error(
            "vault_state.codec_invalid_key",
            "vault state key encoding failed",
        ),
        RecordCodecError::LimitExceeded => reducer_error(
            "vault_state.codec_limit_exceeded",
            "vault state record exceeds its deterministic bound",
        ),
        RecordCodecError::Codec
        | RecordCodecError::NonCanonical
        | RecordCodecError::InvalidRecord
        | RecordCodecError::KeyMismatch => reducer_error(
            "vault_state.codec_failed",
            "vault state record is not canonical",
        ),
    }
}

fn arithmetic_error() -> ReducerError {
    reducer_error(
        "vault_state.flow_arithmetic",
        "vault amount plus fee could not be added exactly",
    )
}

fn reducer_error(reason_code: &'static str, message: &'static str) -> ReducerError {
    ReducerError::from_static(reason_code, message)
}
