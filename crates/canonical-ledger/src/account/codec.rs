use serde::{Serialize, de::DeserializeOwned};

use crate::StateKey;

pub(super) const MAX_RECORD_BYTES: usize = 16 * 1024;
const MAX_STATE_KEY_BYTES: usize = 64 * 1024;
const KEY_FRAME_BYTES: usize = size_of::<u64>();

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AccountStateError {
    #[error("account-state key is invalid")]
    InvalidKey,
    #[error("account-state record cannot be decoded")]
    Codec,
    #[error("account-state record bytes are not canonical")]
    NonCanonical,
    #[error("account-state record is invalid")]
    InvalidRecord,
    #[error("account-state record identity does not match its key")]
    KeyMismatch,
    #[error("account-state record exceeds its deterministic bound")]
    LimitExceeded,
}

impl AccountStateError {
    #[must_use]
    pub const fn reason_code(&self) -> &'static str {
        match self {
            Self::InvalidKey => "account_state.codec.invalid_key",
            Self::Codec => "account_state.codec.decode",
            Self::NonCanonical => "account_state.codec.noncanonical",
            Self::InvalidRecord => "account_state.codec.invalid_record",
            Self::KeyMismatch => "account_state.codec.key_mismatch",
            Self::LimitExceeded => "account_state.codec.limit_exceeded",
        }
    }
}

pub(super) fn state_key(
    namespace: &str,
    identities: &[&[u8]],
) -> Result<StateKey, AccountStateError> {
    let encoded_len = identities.iter().try_fold(0_usize, |total, identity| {
        if identity.is_empty() {
            return Err(AccountStateError::InvalidKey);
        }
        total
            .checked_add(KEY_FRAME_BYTES)
            .and_then(|length| length.checked_add(identity.len()))
            .ok_or(AccountStateError::InvalidKey)
    })?;
    if encoded_len > MAX_STATE_KEY_BYTES {
        return Err(AccountStateError::InvalidKey);
    }

    let mut encoded = Vec::new();
    encoded
        .try_reserve_exact(encoded_len)
        .map_err(|_| AccountStateError::InvalidKey)?;
    for identity in identities {
        let length = u64::try_from(identity.len()).map_err(|_| AccountStateError::InvalidKey)?;
        encoded.extend_from_slice(&length.to_be_bytes());
        encoded.extend_from_slice(identity);
    }
    StateKey::try_new(namespace, encoded).map_err(|_| AccountStateError::InvalidKey)
}

pub(super) fn encode_wire<T: Serialize>(wire: &T) -> Result<Vec<u8>, AccountStateError> {
    let bytes = serde_json::to_vec(wire).map_err(|_| AccountStateError::Codec)?;
    if bytes.len() > MAX_RECORD_BYTES {
        return Err(AccountStateError::LimitExceeded);
    }
    Ok(bytes)
}

pub(super) fn decode_wire<T>(bytes: &[u8]) -> Result<T, AccountStateError>
where
    T: DeserializeOwned + Serialize,
{
    if bytes.is_empty() || bytes.len() > MAX_RECORD_BYTES {
        return Err(AccountStateError::LimitExceeded);
    }
    let wire = serde_json::from_slice(bytes).map_err(|_| AccountStateError::Codec)?;
    if encode_wire(&wire)? != bytes {
        return Err(AccountStateError::NonCanonical);
    }
    Ok(wire)
}

pub(super) fn require_record_bytes(
    encoded: &[u8],
    original: &[u8],
) -> Result<(), AccountStateError> {
    if encoded == original {
        Ok(())
    } else {
        Err(AccountStateError::NonCanonical)
    }
}

pub(super) fn decode_hash(value: &str) -> Result<[u8; 32], AccountStateError> {
    if value.len() != 64 || value.bytes().any(|byte| byte.is_ascii_uppercase()) {
        return Err(AccountStateError::InvalidRecord);
    }
    let mut hash = [0_u8; 32];
    hex::decode_to_slice(value, &mut hash).map_err(|_| AccountStateError::InvalidRecord)?;
    Ok(hash)
}
