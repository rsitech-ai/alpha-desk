use canonical_ledger::{
    CanonicalLedger, CheckpointArtifact, CheckpointCompatibility, EventReducer, LedgerLimits,
    StateImageLimits,
};
use domain_types::CheckpointId;
use storage_ports::StateCheckpointStore;

use crate::replay::LocalReplayError;

pub fn load_checkpoint_ledger<R: EventReducer>(
    store: &(impl StateCheckpointStore + ?Sized),
    checkpoint_id: &CheckpointId,
    compatibility: &CheckpointCompatibility,
    reducer: R,
    limits: LedgerLimits,
    image_limits: StateImageLimits,
) -> Result<CanonicalLedger<R>, LocalReplayError> {
    let artifact = store.load(checkpoint_id, compatibility, image_limits)?;
    CanonicalLedger::try_from_state_image(artifact.state_image().clone(), reducer, limits)
        .map_err(LocalReplayError::Ledger)
}

pub fn publish_checkpoint(
    store: &(impl StateCheckpointStore + ?Sized),
    artifact: &CheckpointArtifact,
) -> Result<(), LocalReplayError> {
    store.publish(artifact).map(|_| ())?;
    Ok(())
}
