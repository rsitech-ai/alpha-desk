use canonical_ledger::{CanonicalStateReducerV1, LedgerError, LedgerLimits, StateImageLimits};
use canonical_state_store::{LocalCheckpointStore, SyncedWriteBatchStore};

use crate::{
    config::CoreConfig,
    health::{DiskReserve, DiskSpaceProbe, FeatureHealth, ShutdownFlag},
    replay::LocalReplayError,
    state_runtime::StateRuntime,
};

pub struct CoreApp<P> {
    runtime: StateRuntime<CanonicalStateReducerV1, SyncedWriteBatchStore>,
    disk: DiskReserve<P>,
    shutdown: ShutdownFlag,
}

impl<P: DiskSpaceProbe> CoreApp<P> {
    pub fn open(config: &CoreConfig, disk: DiskReserve<P>) -> Result<Self, LocalReplayError> {
        disk.ensure()
            .map_err(|_| LocalReplayError::Store(storage_ports::StateStoreError::ResourceLimit))?;
        let store =
            SyncedWriteBatchStore::open(config.state_path(), StateImageLimits::production())?;
        let _opened_checkpoint_root =
            LocalCheckpointStore::open(config.checkpoint_path(), StateImageLimits::production())?;
        let mode = config
            .resume_mode()
            .map_err(|_| LocalReplayError::MidHistoryResume)?;
        if matches!(mode, crate::state_runtime::ResumeMode::Checkpoint(_)) {
            return Err(LocalReplayError::MidHistoryResume);
        }
        let runtime = StateRuntime::open(
            config.chain_id(),
            config.genesis_height(),
            CanonicalStateReducerV1::try_new()
                .map_err(|_| LocalReplayError::Ledger(LedgerError::InvalidReducerVersion))?,
            LedgerLimits::production(),
            store,
            StateImageLimits::production(),
            mode,
            None,
        )?;
        Ok(Self {
            runtime,
            disk,
            shutdown: ShutdownFlag::new(),
        })
    }

    #[must_use]
    pub fn runtime(&self) -> &StateRuntime<CanonicalStateReducerV1, SyncedWriteBatchStore> {
        &self.runtime
    }

    pub fn runtime_mut(
        &mut self,
    ) -> &mut StateRuntime<CanonicalStateReducerV1, SyncedWriteBatchStore> {
        &mut self.runtime
    }

    #[must_use]
    pub fn health(&self) -> &FeatureHealth {
        self.runtime.health()
    }

    #[must_use]
    pub fn shutdown(&self) -> &ShutdownFlag {
        &self.shutdown
    }

    pub fn request_stop(&self) {
        self.shutdown.request_stop();
    }

    pub fn ensure_resources(&mut self) -> Result<(), LocalReplayError> {
        match self.disk.ensure() {
            Ok(_) => {
                self.runtime.health_mut().observe_disk_pressure(false);
                Ok(())
            }
            Err(_) => {
                self.runtime.health_mut().observe_disk_pressure(true);
                Err(LocalReplayError::Store(
                    storage_ports::StateStoreError::ResourceLimit,
                ))
            }
        }
    }

    #[must_use]
    pub fn latest_height(&self) -> Option<domain_types::BlockHeight> {
        self.runtime
            .ledger()
            .checkpoint()
            .map(|checkpoint| checkpoint.block_height())
    }
}
