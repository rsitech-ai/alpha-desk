use prost::Message;
use thiserror::Error;

use crate::generated::hl::health::v1::{
    HealthAssessment as ProtoHealthAssessment, HealthState as ProtoHealthState,
};

const INVALID_SCOPE: &str =
    "scope must be non-empty without surrounding whitespace or control characters";
const INVALID_REASON: &str =
    "reason_code must be non-empty without surrounding whitespace or control characters";
const INVALID_SUPPRESSION: &str =
    "suppresses entries must be non-empty without surrounding whitespace or control characters";

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum HealthCodecError {
    #[error("{reason}")]
    Invalid { reason: String },
    #[error("failed to decode health assessment: {source}")]
    Decode {
        #[source]
        source: prost::DecodeError,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum WireHealthState {
    Green,
    Amber,
    Red,
}

impl WireHealthState {
    #[must_use]
    pub const fn proto_name(self) -> &'static str {
        match self {
            Self::Green => "HEALTH_STATE_GREEN",
            Self::Amber => "HEALTH_STATE_AMBER",
            Self::Red => "HEALTH_STATE_RED",
        }
    }

    pub fn parse(name: &str) -> Result<Self, HealthCodecError> {
        match name {
            "HEALTH_STATE_GREEN" => Ok(Self::Green),
            "HEALTH_STATE_AMBER" => Ok(Self::Amber),
            "HEALTH_STATE_RED" => Ok(Self::Red),
            "HEALTH_STATE_UNSPECIFIED" => Err(HealthCodecError::Invalid {
                reason: "health state must not be HEALTH_STATE_UNSPECIFIED".to_owned(),
            }),
            _ => Err(HealthCodecError::Invalid {
                reason: format!("unknown health state {name}"),
            }),
        }
    }

    fn to_proto(self) -> ProtoHealthState {
        match self {
            Self::Green => ProtoHealthState::Green,
            Self::Amber => ProtoHealthState::Amber,
            Self::Red => ProtoHealthState::Red,
        }
    }

    fn from_proto(state: i32) -> Result<Self, HealthCodecError> {
        match ProtoHealthState::try_from(state) {
            Ok(ProtoHealthState::Green) => Ok(Self::Green),
            Ok(ProtoHealthState::Amber) => Ok(Self::Amber),
            Ok(ProtoHealthState::Red) => Ok(Self::Red),
            Ok(ProtoHealthState::Unspecified) => Err(HealthCodecError::Invalid {
                reason: "health state must not be HEALTH_STATE_UNSPECIFIED".to_owned(),
            }),
            Err(_) => Err(HealthCodecError::Invalid {
                reason: format!("unknown health state discriminant {state}"),
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireHealthAssessment {
    pub scope: String,
    pub state: WireHealthState,
    pub reason_code: String,
    pub observed_at_micros: i64,
    pub suppresses: Vec<String>,
}

impl WireHealthAssessment {
    pub fn try_new(
        scope: impl Into<String>,
        state: WireHealthState,
        reason_code: impl Into<String>,
        observed_at_micros: i64,
        suppresses: impl IntoIterator<Item = impl Into<String>>,
    ) -> Result<Self, HealthCodecError> {
        let assessment = Self {
            scope: scope.into(),
            state,
            reason_code: reason_code.into(),
            observed_at_micros,
            suppresses: suppresses.into_iter().map(Into::into).collect(),
        };
        assessment.validate()?;
        Ok(assessment)
    }

    #[must_use]
    pub fn encode_to_vec(&self) -> Vec<u8> {
        ProtoHealthAssessment {
            scope: self.scope.clone(),
            state: self.state.to_proto() as i32,
            reason_code: self.reason_code.clone(),
            observed_at_micros: self.observed_at_micros,
            suppresses: self.suppresses.clone(),
        }
        .encode_to_vec()
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, HealthCodecError> {
        let message = ProtoHealthAssessment::decode(bytes)
            .map_err(|source| HealthCodecError::Decode { source })?;
        let assessment = Self {
            scope: message.scope,
            state: WireHealthState::from_proto(message.state)?,
            reason_code: message.reason_code,
            observed_at_micros: message.observed_at_micros,
            suppresses: message.suppresses,
        };
        assessment.validate()?;
        Ok(assessment)
    }

    fn validate(&self) -> Result<(), HealthCodecError> {
        validate_identifier(&self.scope, INVALID_SCOPE)?;
        validate_identifier(&self.reason_code, INVALID_REASON)?;
        if self.observed_at_micros < 0 {
            return Err(HealthCodecError::Invalid {
                reason: "observed_at_micros must be non-negative".to_owned(),
            });
        }
        for suppression in &self.suppresses {
            validate_identifier(suppression, INVALID_SUPPRESSION)?;
        }
        Ok(())
    }
}

fn validate_identifier(value: &str, reason: &'static str) -> Result<(), HealthCodecError> {
    if value.is_empty() || value.trim() != value || value.chars().any(char::is_control) {
        return Err(HealthCodecError::Invalid {
            reason: reason.to_owned(),
        });
    }
    Ok(())
}
