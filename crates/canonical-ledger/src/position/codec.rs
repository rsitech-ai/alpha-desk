use serde::{Serialize, de::DeserializeOwned};

use domain_types::{Address, MarketId};

use crate::StateKey;

pub(super) const MAX_RECORD_BYTES: usize = 16 * 1024;
const MAX_STATE_KEY_BYTES: usize = 64 * 1024;
const KEY_FRAME_BYTES: usize = size_of::<u64>();

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PositionStateError {
    #[error("position-state key is invalid")]
    InvalidKey,
    #[error("position-state record cannot be decoded")]
    Codec,
    #[error("position-state record bytes are not canonical")]
    NonCanonical,
    #[error("position-state record is invalid")]
    InvalidRecord,
    #[error("position-state record identity does not match its key")]
    KeyMismatch,
    #[error("position-state record exceeds its deterministic bound")]
    LimitExceeded,
}

impl PositionStateError {
    #[must_use]
    pub const fn reason_code(&self) -> &'static str {
        match self {
            Self::InvalidKey => "position_state.codec.invalid_key",
            Self::Codec => "position_state.codec.decode",
            Self::NonCanonical => "position_state.codec.noncanonical",
            Self::InvalidRecord => "position_state.codec.invalid_record",
            Self::KeyMismatch => "position_state.codec.key_mismatch",
            Self::LimitExceeded => "position_state.codec.limit_exceeded",
        }
    }
}

pub(super) fn state_key(
    namespace: &str,
    identities: &[&[u8]],
) -> Result<StateKey, PositionStateError> {
    let encoded_len = identities.iter().try_fold(0_usize, |total, identity| {
        if identity.is_empty() {
            return Err(PositionStateError::InvalidKey);
        }
        total
            .checked_add(KEY_FRAME_BYTES)
            .and_then(|length| length.checked_add(identity.len()))
            .ok_or(PositionStateError::InvalidKey)
    })?;
    if encoded_len > MAX_STATE_KEY_BYTES {
        return Err(PositionStateError::InvalidKey);
    }

    let mut encoded = Vec::new();
    encoded
        .try_reserve_exact(encoded_len)
        .map_err(|_| PositionStateError::InvalidKey)?;
    for identity in identities {
        let length = u64::try_from(identity.len()).map_err(|_| PositionStateError::InvalidKey)?;
        encoded.extend_from_slice(&length.to_be_bytes());
        encoded.extend_from_slice(identity);
    }
    StateKey::try_new(namespace, encoded).map_err(|_| PositionStateError::InvalidKey)
}

pub(super) fn decode_account_market_key(
    key: &StateKey,
) -> Result<(Address, MarketId), PositionStateError> {
    let encoded = key.key();
    let mut offset = 0_usize;
    let account = decode_key_frame(encoded, &mut offset)?;
    if account.len() != 20 {
        return Err(PositionStateError::InvalidKey);
    }
    let account = Address::from_bytes(
        account
            .try_into()
            .map_err(|_| PositionStateError::InvalidKey)?,
    );
    let market = decode_key_frame(encoded, &mut offset)?;
    if offset != encoded.len() {
        return Err(PositionStateError::InvalidKey);
    }
    let market = std::str::from_utf8(market).map_err(|_| PositionStateError::InvalidKey)?;
    let market = MarketId::new(market.to_owned()).map_err(|_| PositionStateError::InvalidKey)?;
    Ok((account, market))
}

fn decode_key_frame<'a>(
    encoded: &'a [u8],
    offset: &mut usize,
) -> Result<&'a [u8], PositionStateError> {
    let length_end = offset
        .checked_add(KEY_FRAME_BYTES)
        .ok_or(PositionStateError::InvalidKey)?;
    let length_bytes = encoded
        .get(*offset..length_end)
        .ok_or(PositionStateError::InvalidKey)?;
    let length = u64::from_be_bytes(
        length_bytes
            .try_into()
            .map_err(|_| PositionStateError::InvalidKey)?,
    );
    let length = usize::try_from(length).map_err(|_| PositionStateError::InvalidKey)?;
    if length == 0 {
        return Err(PositionStateError::InvalidKey);
    }
    let frame_end = length_end
        .checked_add(length)
        .ok_or(PositionStateError::InvalidKey)?;
    let frame = encoded
        .get(length_end..frame_end)
        .ok_or(PositionStateError::InvalidKey)?;
    *offset = frame_end;
    Ok(frame)
}

pub(super) fn encode_wire<T: Serialize>(wire: &T) -> Result<Vec<u8>, PositionStateError> {
    let bytes = serde_json::to_vec(wire).map_err(|_| PositionStateError::Codec)?;
    if bytes.len() > MAX_RECORD_BYTES {
        return Err(PositionStateError::LimitExceeded);
    }
    Ok(bytes)
}

pub(super) fn decode_wire<T>(bytes: &[u8]) -> Result<T, PositionStateError>
where
    T: DeserializeOwned + Serialize,
{
    if bytes.is_empty() || bytes.len() > MAX_RECORD_BYTES {
        return Err(PositionStateError::LimitExceeded);
    }
    let wire = serde_json::from_slice(bytes).map_err(|_| PositionStateError::Codec)?;
    if encode_wire(&wire)? != bytes {
        return Err(PositionStateError::NonCanonical);
    }
    Ok(wire)
}

pub(super) fn require_record_bytes(
    encoded: &[u8],
    original: &[u8],
) -> Result<(), PositionStateError> {
    if encoded == original {
        Ok(())
    } else {
        Err(PositionStateError::NonCanonical)
    }
}
