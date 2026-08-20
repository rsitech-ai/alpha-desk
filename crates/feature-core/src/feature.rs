use std::fmt;

use domain_types::{AccountId, EntityId, EvidenceId, KnownTime, MarketId, ProtocolTime};
use serde::{Deserialize, Serialize};

use crate::FeatureError;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct FeatureKey {
    pub namespace: String,
    pub name: String,
    pub version: u32,
}

impl FeatureKey {
    pub fn try_new(
        namespace: impl Into<String>,
        name: impl Into<String>,
        version: u32,
    ) -> Result<Self, FeatureError> {
        let namespace = namespace.into();
        let name = name.into();
        if namespace.is_empty() {
            return Err(FeatureError::EmptyIdentifier { field: "namespace" });
        }
        if name.is_empty() {
            return Err(FeatureError::EmptyIdentifier { field: "name" });
        }
        if namespace.trim() != namespace || name.trim() != name {
            return Err(FeatureError::Malformed {
                what: "feature_key",
                reason: "surrounding whitespace",
            });
        }
        if version == 0 {
            return Err(FeatureError::Malformed {
                what: "feature_key",
                reason: "version must be >= 1",
            });
        }
        Ok(Self {
            namespace,
            name,
            version,
        })
    }
}

impl fmt::Display for FeatureKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}/{}@{}",
            self.namespace, self.name, self.version
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum FeatureSubject {
    Account(AccountId),
    Entity(EntityId),
    Market(MarketId),
    AccountMarket {
        account_id: AccountId,
        market_id: MarketId,
    },
    EntityMarket {
        entity_id: EntityId,
        market_id: MarketId,
    },
}

impl FeatureSubject {
    #[must_use]
    pub const fn subject_type(&self) -> &'static str {
        match self {
            Self::Account(_) => "account",
            Self::Entity(_) => "entity",
            Self::Market(_) => "market",
            Self::AccountMarket { .. } => "account_market",
            Self::EntityMarket { .. } => "entity_market",
        }
    }

    #[must_use]
    pub fn subject_id(&self) -> String {
        match self {
            Self::Account(id) => id.to_string(),
            Self::Entity(id) => id.to_string(),
            Self::Market(id) => id.to_string(),
            Self::AccountMarket {
                account_id,
                market_id,
            } => format!("{account_id}|{market_id}"),
            Self::EntityMarket {
                entity_id,
                market_id,
            } => format!("{entity_id}|{market_id}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MissingReason {
    NotObserved,
    InsufficientHistory,
    Unsupported,
    NotApplicable,
    RedDataHealth,
}

impl MissingReason {
    #[must_use]
    pub const fn as_wire_name(self) -> &'static str {
        match self {
            Self::NotObserved => "not_observed",
            Self::InsufficientHistory => "insufficient_history",
            Self::Unsupported => "unsupported",
            Self::NotApplicable => "not_applicable",
            Self::RedDataHealth => "red_data_health",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FeatureValue {
    Decimal { raw: i128, scale: u32 },
    SignedInteger(i64),
    UnsignedInteger(u64),
    ProbabilityPpm(domain_types::ProbabilityPpm),
    Category(String),
    Boolean(bool),
    Missing(MissingReason),
}

impl FeatureValue {
    pub fn try_decimal(raw: i128, scale: u32) -> Result<Self, FeatureError> {
        if scale > u32::from(domain_types::MAX_DECIMAL_SCALE) {
            return Err(FeatureError::UnsupportedScale { scale });
        }
        Ok(Self::Decimal { raw, scale })
    }

    pub fn try_category(value: impl Into<String>) -> Result<Self, FeatureError> {
        let value = value.into();
        if value.is_empty() {
            return Err(FeatureError::EmptyIdentifier { field: "category" });
        }
        Ok(Self::Category(value))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EvidenceKind {
    CanonicalEvent,
    StateSnapshot,
    BookSnapshot,
    FeatureSnapshot,
    OperatorAnnotation,
    ModelArtifact,
}

impl EvidenceKind {
    #[must_use]
    pub const fn as_wire_name(self) -> &'static str {
        match self {
            Self::CanonicalEvent => "canonical_event",
            Self::StateSnapshot => "state_snapshot",
            Self::BookSnapshot => "book_snapshot",
            Self::FeatureSnapshot => "feature_snapshot",
            Self::OperatorAnnotation => "operator_annotation",
            Self::ModelArtifact => "model_artifact",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceRef {
    pub kind: EvidenceKind,
    pub evidence_id: EvidenceId,
    pub content_hash: [u8; 32],
    pub effective_at: ProtocolTime,
    pub known_at: KnownTime,
}

impl EvidenceRef {
    pub fn try_new(
        kind: EvidenceKind,
        evidence_id: EvidenceId,
        content_hash: [u8; 32],
        effective_at: ProtocolTime,
        known_at: KnownTime,
    ) -> Result<Self, FeatureError> {
        if evidence_id.as_str().is_empty() {
            return Err(FeatureError::EmptyIdentifier {
                field: "evidence_id",
            });
        }
        if content_hash.iter().all(|byte| *byte == 0) {
            return Err(FeatureError::ZeroContentHash);
        }
        if known_at.unix_micros() < effective_at.unix_micros() {
            return Err(FeatureError::TemporalInversion);
        }
        Ok(Self {
            kind,
            evidence_id,
            content_hash,
            effective_at,
            known_at,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeatureManifest {
    keys: Vec<FeatureKey>,
}

impl FeatureManifest {
    pub fn try_new(keys: Vec<FeatureKey>) -> Result<Self, FeatureError> {
        if keys.is_empty() {
            return Err(FeatureError::Malformed {
                what: "feature_manifest",
                reason: "empty key set",
            });
        }
        let mut seen = std::collections::BTreeSet::new();
        for key in &keys {
            if !seen.insert(key.clone()) {
                return Err(FeatureError::Malformed {
                    what: "feature_manifest",
                    reason: "duplicate key",
                });
            }
        }
        Ok(Self { keys })
    }

    pub fn require(&self, key: &FeatureKey) -> Result<(), FeatureError> {
        if self.keys.iter().any(|candidate| candidate == key) {
            Ok(())
        } else {
            Err(FeatureError::UnregisteredKey {
                namespace: key.namespace.clone(),
                name: key.name.clone(),
                version: key.version,
            })
        }
    }
}
