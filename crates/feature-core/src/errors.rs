use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum FeatureError {
    #[error("empty identifier {field}")]
    EmptyIdentifier { field: &'static str },
    #[error("unsupported {what}")]
    Unsupported { what: &'static str },
    #[error("malformed {what}: {reason}")]
    Malformed {
        what: &'static str,
        reason: &'static str,
    },
    #[error("feature key {namespace}/{name}@{version} is not registered")]
    UnregisteredKey {
        namespace: String,
        name: String,
        version: u32,
    },
    #[error("duplicate event id")]
    DuplicateEventId,
    #[error("window capacity exceeded")]
    WindowCapacityExceeded,
    #[error("insufficient history")]
    InsufficientHistory,
    #[error("scale {scale} is not supported")]
    UnsupportedScale { scale: u32 },
    #[error("all-zero content hash is not a valid evidence digest")]
    ZeroContentHash,
    #[error("known_at precedes effective_at")]
    TemporalInversion,
    #[error("superseded_at does not follow known_at")]
    InvalidSupersession,
}
