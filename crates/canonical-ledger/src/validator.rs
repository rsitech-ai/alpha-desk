use std::str::FromStr;

use canonical_events::{CanonicalEventEnvelope, EventKind, EventPayload};
use domain_types::{BlockHeight, EventId, Quantity};
use serde::{Deserialize, Serialize};

use crate::{
    ApplyContext, EventReducer, ReducerError, StateKey, StateMutation, StateView,
    opaque::decode_validator_reward,
    record_codec::{RecordCodecError, decode_json, encode_json, framed_key},
};

const FACT_NAMESPACE: &str = "validator-fact.v1";
const CURRENT_NAMESPACE: &str = "validator-reward-current.v1";
const FACT_SCHEMA: &str = "hyperliquid-alpha-desk/validator-fact/v1";
const CURRENT_SCHEMA: &str = "hyperliquid-alpha-desk/validator-reward-current/v1";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CanonicalValidatorReducerV1;

impl CanonicalValidatorReducerV1 {
    pub const VERSION: &'static str = "hyperliquid-alpha-desk-canonical-validator@1.0.0";
}

impl EventReducer for CanonicalValidatorReducerV1 {
    fn reducer_set_version(&self) -> &str {
        Self::VERSION
    }

    fn supports(&self, event: &CanonicalEventEnvelope) -> bool {
        event.schema_version() == "1.0.0" && event.event_kind() == EventKind::ValidatorRewardPaid
    }

    fn reduce(
        &self,
        state: &StateView<'_>,
        event: &CanonicalEventEnvelope,
        _context: &ApplyContext<'_>,
    ) -> Result<Vec<StateMutation>, ReducerError> {
        if !self.supports(event) {
            return Err(reducer_error(
                "validator_state.unsupported_event",
                "validator reducer received an unsupported event",
            ));
        }
        if !event.account_addresses().is_empty() {
            return Err(reducer_error(
                "validator_state.account_inferred",
                "validator rewards are not account-owned",
            ));
        }
        let EventPayload::ValidatorRewardPaid(payload) = event.payload() else {
            return Err(reducer_error(
                "validator_state.unsupported_event",
                "validator reducer received an unsupported event",
            ));
        };
        let decoded = decode_validator_reward(payload)?;
        let fact_key = ValidatorFactRecordV1::state_key(event.event_id()).map_err(codec_error)?;
        if state.contains_key(&fact_key) {
            return Err(reducer_error(
                "validator_state.event_identity_collision",
                "validator event identity is already present",
            ));
        }
        let current_key =
            ValidatorRewardCurrentRecordV1::state_key(&decoded.validator).map_err(codec_error)?;
        let current = match state.get(&current_key) {
            Some(bytes) => {
                let current = ValidatorRewardCurrentRecordV1::decode_at(&current_key, bytes)
                    .map_err(codec_error)?;
                ValidatorRewardCurrentRecordV1 {
                    validator: decoded.validator.clone(),
                    credits: current
                        .credits
                        .checked_add(decoded.amount)
                        .map_err(|_| arithmetic())?,
                    last_event_id: event.event_id().clone(),
                    last_block_height: event.block_height(),
                }
            }
            None => ValidatorRewardCurrentRecordV1 {
                validator: decoded.validator.clone(),
                credits: decoded.amount,
                last_event_id: event.event_id().clone(),
                last_block_height: event.block_height(),
            },
        };
        let fact = ValidatorFactRecordV1 {
            event_id: event.event_id().clone(),
            validator: decoded.validator,
            block_height: event.block_height(),
            payload_hash: event.payload_hash(),
        };
        Ok(vec![
            StateMutation::put(fact_key, fact.encode().map_err(codec_error)?),
            StateMutation::put(current_key, current.encode().map_err(codec_error)?),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatorFactRecordV1 {
    event_id: EventId,
    validator: String,
    block_height: BlockHeight,
    payload_hash: [u8; 32],
}

impl ValidatorFactRecordV1 {
    pub fn state_key(event_id: &EventId) -> Result<StateKey, RecordCodecError> {
        framed_key(FACT_NAMESPACE, &[event_id.as_str().as_bytes()])
    }

    fn encode(&self) -> Result<Vec<u8>, RecordCodecError> {
        encode_json(&ValidatorFactWire {
            schema: FACT_SCHEMA.to_owned(),
            event_id: self.event_id.as_str().to_owned(),
            validator: self.validator.clone(),
            block_height: self.block_height.get(),
            payload_blake3: hex::encode(self.payload_hash),
            rule_version: CanonicalValidatorReducerV1::VERSION.to_owned(),
        })
    }

    #[must_use]
    pub fn validator(&self) -> &str {
        &self.validator
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatorRewardCurrentRecordV1 {
    validator: String,
    credits: Quantity,
    last_event_id: EventId,
    last_block_height: BlockHeight,
}

impl ValidatorRewardCurrentRecordV1 {
    pub fn state_key(validator: &str) -> Result<StateKey, RecordCodecError> {
        framed_key(CURRENT_NAMESPACE, &[validator.as_bytes()])
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, RecordCodecError> {
        let wire: ValidatorCurrentWire = decode_json(bytes)?;
        if wire.schema != CURRENT_SCHEMA || wire.validator.is_empty() {
            return Err(RecordCodecError::InvalidRecord);
        }
        let record = Self {
            validator: wire.validator,
            credits: Quantity::from_str(&wire.credits)
                .map_err(|_| RecordCodecError::InvalidRecord)?,
            last_event_id: EventId::new(wire.last_event_id)
                .map_err(|_| RecordCodecError::InvalidRecord)?,
            last_block_height: BlockHeight::new(wire.last_block_height),
        };
        if record.encode()? != bytes {
            return Err(RecordCodecError::NonCanonical);
        }
        Ok(record)
    }

    pub fn decode_at(key: &StateKey, bytes: &[u8]) -> Result<Self, RecordCodecError> {
        let record = Self::decode(bytes)?;
        if Self::state_key(&record.validator)? == *key {
            Ok(record)
        } else {
            Err(RecordCodecError::KeyMismatch)
        }
    }

    fn encode(&self) -> Result<Vec<u8>, RecordCodecError> {
        encode_json(&ValidatorCurrentWire {
            schema: CURRENT_SCHEMA.to_owned(),
            validator: self.validator.clone(),
            credits: self.credits.to_string(),
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
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ValidatorFactWire {
    schema: String,
    event_id: String,
    validator: String,
    block_height: u64,
    payload_blake3: String,
    rule_version: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ValidatorCurrentWire {
    schema: String,
    validator: String,
    credits: String,
    last_event_id: String,
    last_block_height: u64,
}

fn codec_error(error: RecordCodecError) -> ReducerError {
    match error {
        RecordCodecError::InvalidKey => reducer_error(
            "validator_state.codec_invalid_key",
            "validator state key encoding failed",
        ),
        RecordCodecError::LimitExceeded => reducer_error(
            "validator_state.codec_limit_exceeded",
            "validator state record exceeds its deterministic bound",
        ),
        RecordCodecError::Codec
        | RecordCodecError::NonCanonical
        | RecordCodecError::InvalidRecord
        | RecordCodecError::KeyMismatch => reducer_error(
            "validator_state.codec_failed",
            "validator state record is not canonical",
        ),
    }
}

fn arithmetic() -> ReducerError {
    reducer_error(
        "validator_state.flow_arithmetic",
        "validator reward quantity arithmetic failed",
    )
}

fn reducer_error(reason_code: &'static str, message: &'static str) -> ReducerError {
    ReducerError::from_static(reason_code, message)
}
