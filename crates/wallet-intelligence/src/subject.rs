use domain_types::{AccountId, EntityId, Horizon, MarketId, RegimeId};
use serde::{Deserialize, Serialize};

use crate::IntelligenceError;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum IntelligenceSubject {
    Account(AccountId),
    Entity(EntityId),
}

impl IntelligenceSubject {
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Account(_) => "account",
            Self::Entity(_) => "entity",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApplicabilitySupport {
    Supported,
    InsufficientEvidence,
    Unsupported,
}

impl ApplicabilitySupport {
    #[must_use]
    pub const fn as_wire_name(self) -> &'static str {
        match self {
            Self::Supported => "supported",
            Self::InsufficientEvidence => "insufficient_evidence",
            Self::Unsupported => "unsupported",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Applicability {
    pub markets: Vec<MarketId>,
    pub horizons: Vec<Horizon>,
    pub regimes: Vec<RegimeId>,
    pub support: ApplicabilitySupport,
    pub reason_codes: Vec<String>,
}

impl Applicability {
    pub fn try_new(
        markets: Vec<MarketId>,
        horizons: Vec<Horizon>,
        regimes: Vec<RegimeId>,
        support: ApplicabilitySupport,
        reason_codes: Vec<String>,
    ) -> Result<Self, IntelligenceError> {
        if reason_codes.iter().any(|code| code.trim().is_empty()) {
            return Err(IntelligenceError::EmptyIdentifier {
                field: "reason_codes",
            });
        }
        match support {
            ApplicabilitySupport::Supported if !reason_codes.is_empty() => {
                Err(IntelligenceError::Malformed {
                    what: "applicability",
                    reason: "supported applicability cannot carry reason codes",
                })
            }
            ApplicabilitySupport::InsufficientEvidence | ApplicabilitySupport::Unsupported
                if reason_codes.is_empty() =>
            {
                Err(IntelligenceError::Malformed {
                    what: "applicability",
                    reason: "unsupported applicability requires reason codes",
                })
            }
            ApplicabilitySupport::Supported
            | ApplicabilitySupport::InsufficientEvidence
            | ApplicabilitySupport::Unsupported => Ok(Self {
                markets,
                horizons,
                regimes,
                support,
                reason_codes,
            }),
        }
    }
}
