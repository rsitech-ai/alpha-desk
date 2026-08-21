use serde::{Serialize, de::DeserializeOwned};

use crate::StateKey;

pub(crate) const MAX_RECORD_BYTES: usize = 16 * 1024;
const MAX_STATE_KEY_BYTES: usize = 64 * 1024;
const KEY_FRAME_BYTES: usize = size_of::<u64>();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordCodecError {
    InvalidKey,
    Codec,
    NonCanonical,
    InvalidRecord,
    KeyMismatch,
    LimitExceeded,
}

pub(crate) fn framed_key(
    namespace: &str,
    identities: &[&[u8]],
) -> Result<StateKey, RecordCodecError> {
    let encoded_len = identities.iter().try_fold(0_usize, |total, identity| {
        if identity.is_empty() {
            return Err(RecordCodecError::InvalidKey);
        }
        total
            .checked_add(KEY_FRAME_BYTES)
            .and_then(|length| length.checked_add(identity.len()))
            .ok_or(RecordCodecError::InvalidKey)
    })?;
    if encoded_len > MAX_STATE_KEY_BYTES {
        return Err(RecordCodecError::InvalidKey);
    }
    let mut encoded = Vec::new();
    encoded
        .try_reserve_exact(encoded_len)
        .map_err(|_| RecordCodecError::InvalidKey)?;
    for identity in identities {
        let length = u64::try_from(identity.len()).map_err(|_| RecordCodecError::InvalidKey)?;
        encoded.extend_from_slice(&length.to_be_bytes());
        encoded.extend_from_slice(identity);
    }
    StateKey::try_new(namespace, encoded).map_err(|_| RecordCodecError::InvalidKey)
}

pub(crate) fn encode_json<T: Serialize>(wire: &T) -> Result<Vec<u8>, RecordCodecError> {
    let bytes = serde_json::to_vec(wire).map_err(|_| RecordCodecError::Codec)?;
    if bytes.len() > MAX_RECORD_BYTES {
        return Err(RecordCodecError::LimitExceeded);
    }
    Ok(bytes)
}

pub(crate) fn decode_json<T>(bytes: &[u8]) -> Result<T, RecordCodecError>
where
    T: DeserializeOwned + Serialize,
{
    if bytes.is_empty() || bytes.len() > MAX_RECORD_BYTES {
        return Err(RecordCodecError::LimitExceeded);
    }
    let value: T = serde_json::from_slice(bytes).map_err(|_| RecordCodecError::Codec)?;
    if encode_json(&value)? != bytes {
        return Err(RecordCodecError::NonCanonical);
    }
    Ok(value)
}
