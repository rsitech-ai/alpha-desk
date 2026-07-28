#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorDisposition {
    Retry,
    Quarantine,
    Stop,
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum SourceError {
    #[error("temporary source disconnect")]
    TemporaryDisconnect(String),
    #[error("malformed source payload")]
    MalformedPayload(String),
    #[error("source schema drift")]
    SchemaDrift(String),
    #[error("source cursor regression")]
    CursorRegression,
    #[error("source authentication or configuration failure")]
    Configuration(String),
    #[error("historical range unavailable")]
    RangeUnavailable,
    #[error("source operation cancelled")]
    Cancelled,
    #[error("backpressure deadline exceeded")]
    BackpressureTimeout,
}

impl SourceError {
    #[must_use]
    pub const fn reason_code(&self) -> &'static str {
        match self {
            Self::TemporaryDisconnect(_) => "source.temporary_disconnect",
            Self::MalformedPayload(_) => "source.malformed_payload",
            Self::SchemaDrift(_) => "source.schema_drift",
            Self::CursorRegression => "source.cursor_regression",
            Self::Configuration(_) => "source.configuration",
            Self::RangeUnavailable => "source.range_unavailable",
            Self::Cancelled => "source.cancelled",
            Self::BackpressureTimeout => "source.backpressure_timeout",
        }
    }

    #[must_use]
    pub const fn disposition(&self) -> ErrorDisposition {
        match self {
            Self::TemporaryDisconnect(_) | Self::BackpressureTimeout => ErrorDisposition::Retry,
            Self::MalformedPayload(_)
            | Self::SchemaDrift(_)
            | Self::CursorRegression
            | Self::RangeUnavailable => ErrorDisposition::Quarantine,
            Self::Configuration(_) | Self::Cancelled => ErrorDisposition::Stop,
        }
    }
}
