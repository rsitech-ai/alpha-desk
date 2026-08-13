use canonical_ledger::{LedgerLimits, StateImageLimits, WatermarkOnlyReducerV1};
use canonical_state_store::SyncedWriteBatchStore;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::{
    CanonicalPullSource, CoreConfig, CoreConfigError, CoreStatusHandle, DeadLetterError,
    DeadLetterSink, FileDeadLetterSink, JetStreamPullSource, JetStreamReplayError,
    JetStreamReplayReport, JetStreamReplaySession, StatusError,
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
        .with_fetch_batch(jetstream.fetch_batch())?
        .with_dead_letter_consumer(jetstream.durable_name());
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
        let mut dead_letter = FileDeadLetterSink::open(self.config.dead_letter_path())?;
        let config = self.config.jetstream_config()?;
        let status_cancellation = cancellation.child_token();
        let status_task = self
            .spawn_status_server(status_cancellation.clone())
            .await?;
        let mut source = match self
            .session
            .connect_source(|| JetStreamPullSource::connect(config), &mut dead_letter)
            .await
        {
            Ok(source) => source,
            Err(error) => {
                self.status.fail_closed(error.reason_code());
                if status_task.is_some() {
                    cancellation.cancelled().await;
                }
                return Self::stop_status(status_cancellation, status_task, Err(error.into()))
                    .await;
            }
        };
        self.consume_with_status(
            &mut source,
            dead_letter,
            cancellation,
            status_cancellation,
            status_task,
        )
        .await
    }

    pub async fn run_source<Src: CanonicalPullSource>(
        self,
        source: &mut Src,
        cancellation: CancellationToken,
    ) -> Result<JetStreamReplayReport, CoreRuntimeError> {
        let dead_letter = FileDeadLetterSink::open(self.config.dead_letter_path())?;
        let status_cancellation = cancellation.child_token();
        let status_task = self
            .spawn_status_server(status_cancellation.clone())
            .await?;
        self.consume_with_status(
            source,
            dead_letter,
            cancellation,
            status_cancellation,
            status_task,
        )
        .await
    }

    async fn spawn_status_server(
        &self,
        cancellation: CancellationToken,
    ) -> Result<Option<JoinHandle<Result<(), StatusError>>>, CoreRuntimeError> {
        match self.config.status_listen() {
            Some(listen) => {
                let listener = crate::status::bind_loopback(listen).await?;
                self.status
                    .set_listen_addr(listener.local_addr().map_err(|_| StatusError::Bind)?);
                let status = self.status.clone();
                Ok(Some(tokio::spawn(async move {
                    crate::accept_status(listener, status, cancellation).await
                })))
            }
            None => Ok(None),
        }
    }

    async fn consume_with_status<Src, Dlq>(
        mut self,
        source: &mut Src,
        mut dead_letter: Dlq,
        cancellation: CancellationToken,
        status_cancellation: CancellationToken,
        status_task: Option<JoinHandle<Result<(), StatusError>>>,
    ) -> Result<JetStreamReplayReport, CoreRuntimeError>
    where
        Src: CanonicalPullSource,
        Dlq: DeadLetterSink,
    {
        self.status.mark_ready();
        let result = self
            .consume_until_cancelled(source, &mut dead_letter, cancellation)
            .await;
        Self::stop_status(status_cancellation, status_task, result).await
    }

    async fn stop_status(
        status_cancellation: CancellationToken,
        status_task: Option<JoinHandle<Result<(), StatusError>>>,
        result: Result<JetStreamReplayReport, CoreRuntimeError>,
    ) -> Result<JetStreamReplayReport, CoreRuntimeError> {
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

    async fn consume_until_cancelled<Src, Dlq>(
        &mut self,
        source: &mut Src,
        dead_letter: &mut Dlq,
        cancellation: CancellationToken,
    ) -> Result<JetStreamReplayReport, CoreRuntimeError>
    where
        Src: CanonicalPullSource,
        Dlq: DeadLetterSink,
    {
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
                result = self.session.consume_available(source, dead_letter) => {
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
    DeadLetter(#[from] DeadLetterError),
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
            Self::DeadLetter(error) => error.reason_code(),
            Self::Status(error) => error.reason_code(),
            Self::Replay(error) => error.reason_code(),
        }
    }
}
