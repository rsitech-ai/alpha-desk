use canonical_ledger::{LedgerLimits, StateImageLimits, WatermarkOnlyReducerV1};
use canonical_state_store::SyncedWriteBatchStore;
use tokio_util::sync::CancellationToken;

use crate::{
    CanonicalPullSource, CoreConfig, CoreConfigError, CoreStatusHandle, JetStreamPullSource,
    JetStreamReplayError, JetStreamReplayReport, JetStreamReplaySession, StatusError,
};

pub struct CoreRuntime {
    config: CoreConfig,
    session: JetStreamReplaySession<WatermarkOnlyReducerV1, SyncedWriteBatchStore>,
    status: CoreStatusHandle,
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
        let last_applied = session
            .ledger()
            .checkpoint()
            .map(|checkpoint| checkpoint.block_height().get());
        Ok(Self {
            config,
            session,
            status: CoreStatusHandle::starting(last_applied),
        })
    }

    #[must_use]
    pub fn status(&self) -> &CoreStatusHandle {
        &self.status
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
        let status_cancellation = cancellation.child_token();
        let status_task = match self.config.status_listen() {
            Some(listen) => {
                let listener = crate::status::bind_loopback(listen).await?;
                self.status
                    .set_listen_addr(listener.local_addr().map_err(|_| StatusError::Bind)?);
                let status = self.status.clone();
                let status_cancellation = status_cancellation.clone();
                Some(tokio::spawn(async move {
                    crate::accept_status(listener, status, status_cancellation).await
                }))
            }
            None => None,
        };
        self.status.mark_ready();
        let result = self.consume_until_cancelled(source, cancellation).await;
        status_cancellation.cancel();
        if let Some(task) = status_task {
            match task.await {
                Ok(Ok(())) => {}
                Ok(Err(error)) if result.is_ok() => return Err(error.into()),
                Err(_) if result.is_ok() => return Err(StatusError::Accept.into()),
                Ok(Err(_)) | Err(_) => {}
            }
        }
        result
    }

    async fn consume_until_cancelled<Src: CanonicalPullSource>(
        &mut self,
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
        self.status.record(&report);
        loop {
            tokio::select! {
                biased;
                () = cancellation.cancelled() => return Ok(report),
                result = self.session.consume_available(source) => {
                    report = match result {
                        Ok(report) => {
                            debug_assert!(!report.live_qualified);
                            debug_assert!(!report.stage_2_qualified);
                            self.status.record(&report);
                            report
                        }
                        Err(error) => {
                            self.status.fail_closed(error.reason_code());
                            return Err(error.into());
                        }
                    };
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
    Status(#[from] StatusError),
    #[error(transparent)]
    Replay(#[from] JetStreamReplayError),
}

impl CoreRuntimeError {
    #[must_use]
    pub fn reason_code(&self) -> &'static str {
        match self {
            Self::Config(error) => error.reason_code(),
            Self::Store(_) => "core_runtime.store",
            Self::Status(error) => error.reason_code(),
            Self::Replay(error) => error.reason_code(),
        }
    }
}
