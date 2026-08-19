use canonical_ledger::{LedgerLimits, StateImageLimits, WatermarkOnlyReducerV1};
use canonical_state_store::SyncedWriteBatchStore;
use tokio_util::sync::CancellationToken;

use crate::{
    CanonicalPullSource, CoreConfig, CoreConfigError, JetStreamPullSource, JetStreamReplayError,
    JetStreamReplayReport, JetStreamReplaySession,
};

pub struct CoreRuntime {
    config: CoreConfig,
    session: JetStreamReplaySession<WatermarkOnlyReducerV1, SyncedWriteBatchStore>,
}

impl CoreRuntime {
    pub fn open(config: CoreConfig) -> Result<Self, CoreRuntimeError> {
        let store =
            SyncedWriteBatchStore::open(config.store_path(), StateImageLimits::production())
                .map_err(CoreRuntimeError::Store)?;
        let jetstream = config.jetstream_config()?;
        let session = JetStreamReplaySession::open(
            config.chain_id(),
            config.first_height(),
            WatermarkOnlyReducerV1,
            LedgerLimits::production(),
            store,
            StateImageLimits::production(),
        )?
        .with_fetch_batch(jetstream.fetch_batch())?;
        Ok(Self { config, session })
    }

    pub async fn run_jetstream(
        self,
        cancellation: CancellationToken,
    ) -> Result<JetStreamReplayReport, CoreRuntimeError> {
        let mut source = JetStreamPullSource::connect(self.config.jetstream_config()?).await?;
        self.run_source(&mut source, cancellation).await
    }

    pub async fn run_source<Src: CanonicalPullSource>(
        mut self,
        source: &mut Src,
        cancellation: CancellationToken,
    ) -> Result<JetStreamReplayReport, CoreRuntimeError> {
        let mut report = JetStreamReplayReport {
            applied: 0,
            already_applied: 0,
            last_height: self
                .session
                .ledger()
                .checkpoint()
                .map(|checkpoint| checkpoint.block_height()),
            state_hash: self.session.ledger().state_hash(),
            live_qualified: false,
            stage_2_qualified: false,
        };
        loop {
            tokio::select! {
                biased;
                () = cancellation.cancelled() => return Ok(report),
                result = self.session.consume_available(source) => {
                    report = result?;
                    debug_assert!(!report.live_qualified);
                    debug_assert!(!report.stage_2_qualified);
                }
            }
            tokio::select! {
                biased;
                () = cancellation.cancelled() => return Ok(report),
                () = tokio::time::sleep(self.config.idle_poll()) => {}
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CoreRuntimeError {
    #[error(transparent)]
    Config(#[from] CoreConfigError),
    #[error("hl-core file-store could not be opened: {0}")]
    Store(storage_ports::StateStoreError),
    #[error(transparent)]
    Replay(#[from] JetStreamReplayError),
}

impl CoreRuntimeError {
    #[must_use]
    pub fn reason_code(&self) -> &'static str {
        match self {
            Self::Config(error) => error.reason_code(),
            Self::Store(_) => "core_runtime.store",
            Self::Replay(error) => error.reason_code(),
        }
    }
}
