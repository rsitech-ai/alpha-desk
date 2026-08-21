use crate::publication::{
    BLOCK_COMMITTED_SUBJECT, BLOCK_PROVISIONAL_SUBJECT, BlockMarkerError, CanonicalSubject,
    HEALTH_SOURCE_SUBJECT, SNAPSHOT_ACCOUNT_SUBJECT, SNAPSHOT_ECOSYSTEM_SUBJECT,
    SNAPSHOT_MARKET_SUBJECT,
};

const HEALTH_DATA_SUBJECT: &str = "hl.v1.health.data";
const HEALTH_MODEL_SUBJECT: &str = "hl.v1.health.model";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreInputSubject {
    Committed(CanonicalSubject),
    BlockProvisional,
    SnapshotAccount,
    SnapshotMarket,
    SnapshotEcosystem,
    HealthData,
    HealthSource,
    HealthModel,
}

impl CoreInputSubject {
    pub fn parse(value: &str) -> Result<Self, BlockMarkerError> {
        if let Ok(committed) = CanonicalSubject::parse(value) {
            return Ok(Self::Committed(committed));
        }
        match value {
            BLOCK_PROVISIONAL_SUBJECT => Ok(Self::BlockProvisional),
            SNAPSHOT_ACCOUNT_SUBJECT => Ok(Self::SnapshotAccount),
            SNAPSHOT_MARKET_SUBJECT => Ok(Self::SnapshotMarket),
            SNAPSHOT_ECOSYSTEM_SUBJECT => Ok(Self::SnapshotEcosystem),
            HEALTH_DATA_SUBJECT => Ok(Self::HealthData),
            HEALTH_SOURCE_SUBJECT => Ok(Self::HealthSource),
            HEALTH_MODEL_SUBJECT => Ok(Self::HealthModel),
            _ => Err(BlockMarkerError::UnexpectedSubject),
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Committed(subject) => match subject {
                CanonicalSubject::BlockCommitted => BLOCK_COMMITTED_SUBJECT,
                CanonicalSubject::EventFill => "hl.v1.event.fill",
                CanonicalSubject::EventOrder => "hl.v1.event.order",
                CanonicalSubject::EventLedger => "hl.v1.event.ledger",
                CanonicalSubject::EventMarketMeta => "hl.v1.event.market_meta",
                CanonicalSubject::EventOracle => "hl.v1.event.oracle",
            },
            Self::BlockProvisional => BLOCK_PROVISIONAL_SUBJECT,
            Self::SnapshotAccount => SNAPSHOT_ACCOUNT_SUBJECT,
            Self::SnapshotMarket => SNAPSHOT_MARKET_SUBJECT,
            Self::SnapshotEcosystem => SNAPSHOT_ECOSYSTEM_SUBJECT,
            Self::HealthData => HEALTH_DATA_SUBJECT,
            Self::HealthSource => HEALTH_SOURCE_SUBJECT,
            Self::HealthModel => HEALTH_MODEL_SUBJECT,
        }
    }

    #[must_use]
    pub const fn can_advance_committed_watermark(self) -> bool {
        matches!(self, Self::Committed(CanonicalSubject::BlockCommitted))
    }

    #[must_use]
    pub const fn is_provisional_lane(self) -> bool {
        matches!(
            self,
            Self::BlockProvisional
                | Self::SnapshotAccount
                | Self::SnapshotMarket
                | Self::SnapshotEcosystem
        )
    }
}
