use domain_types::{ChainId, SourceId};
use storage_ports::{ArchiveError, RawArchiveMaintenanceStatistics};

use super::{RawV3Archive, gc};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawArchiveRetentionRequest {
    backup_receipt: [u8; 32],
    authorized_plan_digest: [u8; 32],
}

impl RawArchiveRetentionRequest {
    pub fn try_new(
        backup_receipt: [u8; 32],
        authorized_plan_digest: [u8; 32],
    ) -> Result<Self, ArchiveError> {
        if backup_receipt == [0; 32] || authorized_plan_digest == [0; 32] {
            return Err(ArchiveError::InvalidInput(
                "retention authorization must be a nonzero digest",
            ));
        }
        Ok(Self {
            backup_receipt,
            authorized_plan_digest,
        })
    }

    #[must_use]
    pub const fn backup_receipt(self) -> [u8; 32] {
        self.backup_receipt
    }

    #[must_use]
    pub const fn authorized_plan_digest(self) -> [u8; 32] {
        self.authorized_plan_digest
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawArchiveRetentionReport {
    gc: gc::RawArchiveGcReceipt,
    statistics: RawArchiveMaintenanceStatistics,
}

impl RawArchiveRetentionReport {
    #[must_use]
    pub const fn gc(&self) -> &gc::RawArchiveGcReceipt {
        &self.gc
    }

    #[must_use]
    pub const fn statistics(&self) -> RawArchiveMaintenanceStatistics {
        self.statistics
    }
}

pub fn apply_authorized_retention(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
    request: RawArchiveRetentionRequest,
) -> Result<RawArchiveRetentionReport, ArchiveError> {
    let gc = gc::execute_packed_object_gc(
        archive,
        chain,
        source,
        request.authorized_plan_digest,
        request.backup_receipt,
    )?;
    let statistics = super::scrub::maintenance_statistics(archive, chain, source)?;
    Ok(RawArchiveRetentionReport { gc, statistics })
}
