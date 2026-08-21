use domain_types::{KnownTime, SourceId};
use hl_protocol::{NetworkId, SourceCatalogError, SourceCatalogRecord};

#[derive(Debug, thiserror::Error, Clone, Copy, PartialEq, Eq)]
pub enum SourceCatalogStoreError {
    #[error("source catalog record is invalid")]
    InvalidRecord(SourceCatalogError),
    #[error("source catalog update does not continue the current record")]
    Conflict,
    #[error("source catalog storage failed")]
    Storage,
}

impl SourceCatalogStoreError {
    #[must_use]
    pub const fn reason_code(self) -> &'static str {
        match self {
            Self::InvalidRecord(_) => "source_catalog_store.invalid_record",
            Self::Conflict => "source_catalog_store.conflict",
            Self::Storage => "source_catalog_store.storage",
        }
    }
}

pub trait SourceCatalogStore {
    fn publish(
        &self,
        record: SourceCatalogRecord,
    ) -> Result<SourceCatalogRecord, SourceCatalogStoreError>;

    fn current(
        &self,
        network: &NetworkId,
        source_id: &SourceId,
    ) -> Result<Option<SourceCatalogRecord>, SourceCatalogStoreError>;

    fn history(
        &self,
        network: &NetworkId,
        source_id: &SourceId,
    ) -> Result<Vec<SourceCatalogRecord>, SourceCatalogStoreError>;

    fn scheduled_work(
        &self,
        at: KnownTime,
    ) -> Result<Vec<SourceCatalogRecord>, SourceCatalogStoreError>;
}
