use canonical_ledger::{ApplyOutcome, CanonicalLedger, EventReducer};
use domain_types::BlockHeight;
use storage_ports::{CanonicalArchive, VerifiedManifest};

use crate::{
    ReplayError, ReplayLimits, ReplayOutcome, ReplayProgress, ReplayReceipt, ReplayRequest,
    ReplayStatus, receipt::ReplayManifestIdentity,
};

pub trait ReplayCancellation {
    fn is_cancelled(&self) -> bool;
}

pub struct SerialReplayEngine<'a, A, R> {
    archive: &'a A,
    ledger: &'a mut CanonicalLedger<R>,
    limits: ReplayLimits,
}

impl<'a, A, R> SerialReplayEngine<'a, A, R>
where
    A: CanonicalArchive,
    R: EventReducer,
{
    #[must_use]
    pub const fn new(
        archive: &'a A,
        ledger: &'a mut CanonicalLedger<R>,
        limits: ReplayLimits,
    ) -> Self {
        Self {
            archive,
            ledger,
            limits,
        }
    }

    pub fn run<C: ReplayCancellation>(
        &mut self,
        request: &ReplayRequest,
        cancellation: &C,
    ) -> Result<ReplayOutcome, ReplayError> {
        let start_state_hash = self.ledger.state_hash();
        let empty_progress = self.progress(0, None);
        if request
            .block_count()
            .ok()
            .is_none_or(|count| count > self.limits.max_blocks())
            || request.manifests().len() > self.limits.max_manifests()
        {
            return Err(ReplayError::LimitExceeded {
                progress: empty_progress,
            });
        }
        if start_state_hash != request.expected_start_state_hash() {
            return Err(ReplayError::StartStateMismatch {
                progress: empty_progress,
            });
        }
        if self.ledger.next_height().ok() != Some(request.range().start_inclusive) {
            return Err(ReplayError::StartHeightMismatch {
                progress: empty_progress,
            });
        }
        let manifests = self.preflight(request)?;
        let mut applied = 0_u64;
        let mut last_height = None;

        for manifest in &manifests {
            let mut expected = manifest.block_range.start_inclusive;
            let blocks = self
                .archive
                .read_manifest_blocks(&manifest.manifest_id)
                .map_err(|error| ReplayError::Archive {
                    source_reason_code: error.reason_code(),
                    progress: self.progress(applied, last_height),
                })?;
            for block in blocks {
                if cancellation.is_cancelled() {
                    let receipt = self.receipt(
                        ReplayStatus::Cancelled,
                        request,
                        start_state_hash,
                        applied,
                        last_height,
                        manifests,
                    );
                    return Ok(ReplayOutcome::Cancelled(receipt));
                }
                let block = block.map_err(|error| ReplayError::Archive {
                    source_reason_code: error.reason_code(),
                    progress: self.progress(applied, last_height),
                })?;
                if block.chain_id() != request.chain_id()
                    || block.block_height() != expected
                    || block.block_height() > manifest.block_range.end_inclusive
                {
                    return Err(ReplayError::ArchiveContent {
                        progress: self.progress(applied, last_height),
                    });
                }
                match self.ledger.apply_block(&block) {
                    Ok(ApplyOutcome::Applied(_)) => {}
                    Ok(ApplyOutcome::AlreadyApplied(_)) => {
                        return Err(ReplayError::ArchiveContent {
                            progress: self.progress(applied, last_height),
                        });
                    }
                    Err(error) => {
                        return Err(ReplayError::BlockQuarantined {
                            height: block.block_height(),
                            source_reason_code: error.reason_code(),
                            reducer_reason_code: error.reducer_reason_code().map(str::to_owned),
                            progress: self.progress(applied, last_height),
                        });
                    }
                }
                applied = applied.checked_add(1).ok_or(ReplayError::LimitExceeded {
                    progress: self.progress(applied, last_height),
                })?;
                last_height = Some(block.block_height());
                if block.block_height() == manifest.block_range.end_inclusive {
                    expected = block.block_height();
                } else {
                    expected = BlockHeight::new(block.block_height().get().checked_add(1).ok_or(
                        ReplayError::ArchiveContent {
                            progress: self.progress(applied, last_height),
                        },
                    )?);
                }
            }
            if last_height != Some(manifest.block_range.end_inclusive)
                || expected != manifest.block_range.end_inclusive
            {
                return Err(ReplayError::ArchiveContent {
                    progress: self.progress(applied, last_height),
                });
            }
        }

        let receipt = self.receipt(
            ReplayStatus::Completed,
            request,
            start_state_hash,
            applied,
            last_height,
            manifests,
        );
        Ok(ReplayOutcome::Completed(receipt))
    }

    fn preflight(
        &self,
        request: &ReplayRequest,
    ) -> Result<Vec<ReplayManifestIdentity>, ReplayError> {
        let mut expected_start = request.range().start_inclusive;
        let mut planned_block_count = 0_u64;
        let mut identities = Vec::with_capacity(request.manifests().len());
        for (index, manifest_id) in request.manifests().iter().enumerate() {
            let verified = self.archive.verify_manifest(manifest_id).map_err(|error| {
                ReplayError::ManifestPlan {
                    source_reason_code: error.reason_code(),
                    progress: self.progress(0, None),
                }
            })?;
            validate_manifest(request, manifest_id, &verified, expected_start).map_err(
                |reason| ReplayError::ManifestPlan {
                    source_reason_code: reason,
                    progress: self.progress(0, None),
                },
            )?;
            let manifest_block_count = verified
                .block_range()
                .end_inclusive
                .get()
                .checked_sub(verified.block_range().start_inclusive.get())
                .and_then(|span| span.checked_add(1))
                .ok_or(ReplayError::ManifestPlan {
                    source_reason_code: "replay.manifest_range_incomplete",
                    progress: self.progress(0, None),
                })?;
            planned_block_count = planned_block_count
                .checked_add(manifest_block_count)
                .ok_or(ReplayError::ManifestPlan {
                    source_reason_code: "replay.manifest_range_incomplete",
                    progress: self.progress(0, None),
                })?;
            if index + 1 < request.manifests().len() {
                expected_start = next_height(verified.block_range().end_inclusive).ok_or(
                    ReplayError::ManifestPlan {
                        source_reason_code: "replay.manifest_height_exhausted",
                        progress: self.progress(0, None),
                    },
                )?;
            }
            identities.push(ReplayManifestIdentity {
                manifest_id: verified.manifest_id().clone(),
                manifest_sha256: verified.manifest_sha256(),
                block_range: verified.block_range(),
            });
        }
        if identities.last().is_none_or(|identity| {
            identity.block_range.end_inclusive != request.range().end_inclusive
        }) || request.block_count().ok() != Some(planned_block_count)
        {
            return Err(ReplayError::ManifestPlan {
                source_reason_code: "replay.manifest_range_incomplete",
                progress: self.progress(0, None),
            });
        }
        Ok(identities)
    }

    fn progress(&self, applied: u64, last_height: Option<BlockHeight>) -> ReplayProgress {
        ReplayProgress::new(applied, last_height, self.ledger.state_hash())
    }

    fn receipt(
        &self,
        status: ReplayStatus,
        request: &ReplayRequest,
        start_state_hash: [u8; 32],
        applied: u64,
        last_height: Option<BlockHeight>,
        manifests: Vec<ReplayManifestIdentity>,
    ) -> ReplayReceipt {
        let last_hash = if applied == 0 {
            None
        } else {
            self.ledger
                .checkpoint()
                .map(|checkpoint| checkpoint.canonical_block_hash())
        };
        ReplayReceipt::new(
            status,
            request.chain_id().clone(),
            request.range(),
            start_state_hash,
            self.ledger.state_hash(),
            self.ledger.state_image().reducer_set_version().to_owned(),
            applied,
            last_height,
            last_hash,
            manifests,
        )
    }
}

fn validate_manifest(
    request: &ReplayRequest,
    manifest_id: &domain_types::ManifestId,
    verified: &VerifiedManifest,
    expected_start: BlockHeight,
) -> Result<(), &'static str> {
    if verified.chain_id() != request.chain_id() {
        return Err("replay.manifest_chain_mismatch");
    }
    if verified.manifest_id() != manifest_id
        || verified.block_range().start_inclusive != expected_start
        || verified.block_range().end_inclusive > request.range().end_inclusive
    {
        return Err("replay.manifest_range_mismatch");
    }
    if verified.schema_fingerprints().get(request.schema_dataset())
        != Some(&request.expected_schema_fingerprint())
    {
        return Err("replay.manifest_schema_mismatch");
    }
    Ok(())
}

fn next_height(height: BlockHeight) -> Option<BlockHeight> {
    height.get().checked_add(1).map(BlockHeight::new)
}
