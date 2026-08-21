use canonical_events::{CanonicalEventEnvelope, EventKind, EventPayload};
use domain_types::{Address, BlockHeight, EventId, VaultId};
use serde::{Deserialize, Serialize};

use crate::{
    ApplyContext, EventReducer, ReducerError, StateKey, StateMutation, StateView,
    opaque::{
        decode_staking_delegate, decode_staking_undelegate, decode_vault_create, decode_vault_dist,
    },
    record_codec::{RecordCodecError, decode_json, encode_json, framed_key},
};

const STAKING_NAMESPACE: &str = "staking-delegation-relation.v1";
const STAKING_SCHEMA: &str = "hyperliquid-alpha-desk/staking-delegation-relation/v1";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CanonicalRelationshipReducerV1;

impl CanonicalRelationshipReducerV1 {
    pub const VERSION: &'static str = "hyperliquid-alpha-desk-canonical-relationships@1.0.0";
}

impl EventReducer for CanonicalRelationshipReducerV1 {
    fn reducer_set_version(&self) -> &str {
        Self::VERSION
    }

    fn supports(&self, event: &CanonicalEventEnvelope) -> bool {
        event.schema_version() == "1.0.0"
            && matches!(
                event.event_kind(),
                EventKind::VaultCreated
                    | EventKind::VaultDistribution
                    | EventKind::StakingDelegated
                    | EventKind::StakingUndelegated
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
                "relationship_state.unsupported_event",
                "relationship reducer received an unsupported event",
            ));
        }
        match event.payload() {
            EventPayload::VaultCreated(payload) => {
                let decoded = decode_vault_create(payload)?;
                vault_relation(state, event, &decoded.vault_id)
            }
            EventPayload::VaultDistribution(payload) => {
                let decoded = decode_vault_dist(payload)?;
                vault_relation(state, event, &decoded.vault_id)
            }
            EventPayload::StakingDelegated(payload) => {
                let decoded = decode_staking_delegate(payload)?;
                staking_relation(state, event, decoded.account_id, &decoded.validator, false)
            }
            EventPayload::StakingUndelegated(payload) => {
                let decoded = decode_staking_undelegate(payload)?;
                staking_relation(state, event, decoded.account_id, &decoded.validator, true)
            }
            _ => Err(reducer_error(
                "relationship_state.unsupported_event",
                "relationship reducer received an unsupported event",
            )),
        }
    }
}

fn vault_relation(
    state: &StateView<'_>,
    event: &CanonicalEventEnvelope,
    vault_id: &VaultId,
) -> Result<Vec<StateMutation>, ReducerError> {
    let [account_id] = event.account_addresses() else {
        return Ok(Vec::new());
    };
    Ok(vec![crate::account::vault_relation_put(
        state,
        *account_id,
        vault_id,
        event.event_id(),
        event.block_height(),
    )?])
}

fn staking_relation(
    state: &StateView<'_>,
    event: &CanonicalEventEnvelope,
    account_id: Address,
    validator: &str,
    require_existing: bool,
) -> Result<Vec<StateMutation>, ReducerError> {
    match event.account_addresses() {
        [observed] if *observed == account_id => {}
        _ => {
            return Err(reducer_error(
                "relationship_state.identity_mismatch",
                "staking relation account must match the payload user",
            ));
        }
    }
    let key = StakingDelegationRelationCurrentRecordV1::state_key(&account_id, validator)
        .map_err(codec_error)?;
    let record = match state.get(&key) {
        Some(bytes) => {
            let current = StakingDelegationRelationCurrentRecordV1::decode_at(&key, bytes)
                .map_err(codec_error)?;
            StakingDelegationRelationCurrentRecordV1 {
                account_id,
                validator: current.validator,
                first_event_id: current.first_event_id,
                last_event_id: event.event_id().clone(),
                first_block_height: current.first_block_height,
                last_block_height: event.block_height(),
            }
        }
        None => {
            if require_existing {
                return Err(reducer_error(
                    "relationship_state.missing_delegation",
                    "undelegate requires an existing staking relation",
                ));
            }
            StakingDelegationRelationCurrentRecordV1 {
                account_id,
                validator: validator.to_owned(),
                first_event_id: event.event_id().clone(),
                last_event_id: event.event_id().clone(),
                first_block_height: event.block_height(),
                last_block_height: event.block_height(),
            }
        }
    };
    Ok(vec![StateMutation::put(
        key,
        record.encode().map_err(codec_error)?,
    )])
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StakingDelegationRelationCurrentRecordV1 {
    account_id: Address,
    validator: String,
    first_event_id: EventId,
    last_event_id: EventId,
    first_block_height: BlockHeight,
    last_block_height: BlockHeight,
}

impl StakingDelegationRelationCurrentRecordV1 {
    pub fn state_key(account_id: &Address, validator: &str) -> Result<StateKey, RecordCodecError> {
        framed_key(
            STAKING_NAMESPACE,
            &[account_id.as_bytes(), validator.as_bytes()],
        )
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, RecordCodecError> {
        let wire: StakingRelationWire = decode_json(bytes)?;
        if wire.schema != STAKING_SCHEMA || wire.validator.is_empty() {
            return Err(RecordCodecError::InvalidRecord);
        }
        let record = Self {
            account_id: Address::parse_api(&wire.account_id)
                .map_err(|_| RecordCodecError::InvalidRecord)?,
            validator: wire.validator,
            first_event_id: EventId::new(wire.first_event_id)
                .map_err(|_| RecordCodecError::InvalidRecord)?,
            last_event_id: EventId::new(wire.last_event_id)
                .map_err(|_| RecordCodecError::InvalidRecord)?,
            first_block_height: BlockHeight::new(wire.first_block_height),
            last_block_height: BlockHeight::new(wire.last_block_height),
        };
        if record.first_block_height > record.last_block_height || record.encode()? != bytes {
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

    fn encode(&self) -> Result<Vec<u8>, RecordCodecError> {
        encode_json(&StakingRelationWire {
            schema: STAKING_SCHEMA.to_owned(),
            account_id: self.account_id.to_api_string(),
            validator: self.validator.clone(),
            first_event_id: self.first_event_id.as_str().to_owned(),
            last_event_id: self.last_event_id.as_str().to_owned(),
            first_block_height: self.first_block_height.get(),
            last_block_height: self.last_block_height.get(),
        })
    }

    #[must_use]
    pub fn validator(&self) -> &str {
        &self.validator
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
struct StakingRelationWire {
    schema: String,
    account_id: String,
    validator: String,
    first_event_id: String,
    last_event_id: String,
    first_block_height: u64,
    last_block_height: u64,
}

fn codec_error(error: RecordCodecError) -> ReducerError {
    match error {
        RecordCodecError::InvalidKey => reducer_error(
            "relationship_state.codec_invalid_key",
            "relationship state key encoding failed",
        ),
        RecordCodecError::LimitExceeded => reducer_error(
            "relationship_state.codec_limit_exceeded",
            "relationship state record exceeds its deterministic bound",
        ),
        RecordCodecError::Codec
        | RecordCodecError::NonCanonical
        | RecordCodecError::InvalidRecord
        | RecordCodecError::KeyMismatch => reducer_error(
            "relationship_state.codec_failed",
            "relationship state record is not canonical",
        ),
    }
}

fn reducer_error(reason_code: &'static str, message: &'static str) -> ReducerError {
    ReducerError::from_static(reason_code, message)
}
