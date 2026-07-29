use canonical_events::EventKind;
use domain_types::BlockHeight;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StateKeyError {
    #[error("state namespace is invalid")]
    InvalidNamespace,
    #[error("state key is empty or exceeds its absolute byte limit")]
    InvalidKey,
}

impl StateKeyError {
    #[must_use]
    pub const fn reason_code(&self) -> &'static str {
        match self {
            Self::InvalidNamespace => "ledger.invalid_state_namespace",
            Self::InvalidKey => "ledger.invalid_state_key",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{reason_code}: {message}")]
pub struct ReducerError {
    reason_code: String,
    message: String,
}

impl ReducerError {
    pub fn try_new(
        reason_code: impl Into<String>,
        message: impl Into<String>,
    ) -> Result<Self, LedgerError> {
        let reason_code = reason_code.into();
        let message = message.into();
        if !valid_reason_code(&reason_code) || !valid_message(&message) {
            return Err(LedgerError::InvalidReducerError);
        }
        Ok(Self {
            reason_code,
            message,
        })
    }

    #[must_use]
    pub fn reason_code(&self) -> &str {
        &self.reason_code
    }

    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    pub(crate) fn from_static(reason_code: &'static str, message: &'static str) -> Self {
        assert!(
            valid_reason_code(reason_code) && valid_message(message),
            "internal reducer errors must use validated stable literals"
        );
        Self {
            reason_code: reason_code.to_owned(),
            message: message.to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LedgerError {
    #[error("ledger limits must all be nonzero and internally consistent")]
    InvalidLimits,
    #[error("reducer-set version is invalid")]
    InvalidReducerVersion,
    #[error("reducer-set version changed after ledger construction")]
    ReducerVersionDrift,
    #[error("reducer returned an invalid structured error")]
    InvalidReducerError,
    #[error("canonical block belongs to another chain")]
    ChainMismatch,
    #[error("canonical block is not committed state input")]
    NonCommittedBlock,
    #[error("expected block height {expected:?}, received {actual:?}")]
    HeightDiscontinuity {
        expected: BlockHeight,
        actual: BlockHeight,
    },
    #[error("canonical block at already-applied height has a different hash")]
    CanonicalDivergence,
    #[error("block height cannot advance beyond u64::MAX")]
    HeightExhausted,
    #[error("event kind {kind:?} schema {schema_version} has no qualified reducer")]
    UnsupportedEvent {
        kind: EventKind,
        schema_version: String,
    },
    #[error("qualified reducer failed: {source}")]
    ReducerFailed {
        #[source]
        source: ReducerError,
    },
    #[error("state mutation exceeds a configured deterministic bound")]
    MutationLimitExceeded,
    #[error("state mutation is invalid: {reason}")]
    InvalidMutation { reason: &'static str },
}

impl LedgerError {
    #[must_use]
    pub const fn reason_code(&self) -> &'static str {
        match self {
            Self::InvalidLimits => "ledger.invalid_limits",
            Self::InvalidReducerVersion => "ledger.invalid_reducer_version",
            Self::ReducerVersionDrift => "ledger.reducer_version_drift",
            Self::InvalidReducerError => "ledger.invalid_reducer_error",
            Self::ChainMismatch => "ledger.chain_mismatch",
            Self::NonCommittedBlock => "ledger.non_committed_block",
            Self::HeightDiscontinuity { .. } => "ledger.height_discontinuity",
            Self::CanonicalDivergence => "ledger.canonical_divergence",
            Self::HeightExhausted => "ledger.height_exhausted",
            Self::UnsupportedEvent { .. } => "ledger.unsupported_event",
            Self::ReducerFailed { .. } => "ledger.reducer_failed",
            Self::MutationLimitExceeded => "ledger.mutation_limit_exceeded",
            Self::InvalidMutation { .. } => "ledger.invalid_mutation",
        }
    }

    #[must_use]
    pub fn reducer_reason_code(&self) -> Option<&str> {
        match self {
            Self::ReducerFailed { source } => Some(source.reason_code()),
            _ => None,
        }
    }

    #[must_use]
    pub const fn event_kind(&self) -> Option<EventKind> {
        match self {
            Self::UnsupportedEvent { kind, .. } => Some(*kind),
            _ => None,
        }
    }

    #[must_use]
    pub fn schema_version(&self) -> Option<&str> {
        match self {
            Self::UnsupportedEvent { schema_version, .. } => Some(schema_version),
            _ => None,
        }
    }
}

pub(crate) fn valid_reducer_version(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        && value
            .as_bytes()
            .last()
            .is_some_and(u8::is_ascii_alphanumeric)
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b'+' | b'@')
        })
}

fn valid_reason_code(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        && value
            .as_bytes()
            .last()
            .is_some_and(u8::is_ascii_alphanumeric)
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_')
        })
}

fn valid_message(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 512
        && value.trim() == value
        && !value.bytes().any(|byte| byte.is_ascii_control())
}
