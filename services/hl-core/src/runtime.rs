use canonical_ledger::{LedgerLimits, StateImageLimits, WatermarkOnlyReducerV1};
use canonical_state_store::SyncedWriteBatchStore;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::{
    CanonicalPullSource, CoreConfig, CoreConfigError, CoreStatusHandle, DeadLetterError,
    DeadLetterSink, FileDeadLetterSink, JetStreamPullSource, JetStreamReplayError,
    JetStreamReplayReport, JetStreamReplaySession, StatusError,
    consumer::persist_connect_fail_closed,
};

type CoreSession = JetStreamReplaySession<WatermarkOnlyReducerV1, SyncedWriteBatchStore>;

pub struct CoreRuntime {
    config: CoreConfig,
    session: Option<CoreSession>,
    status: CoreStatusHandle,
}

impl CoreRuntime {
    #[must_use]
    pub fn from_config(config: CoreConfig) -> Self {
        Self {
            config,
            session: None,
            status: CoreStatusHandle::starting(None),
        }
    }

    pub fn open(config: CoreConfig) -> Result<Self, CoreRuntimeError> {
        let mut runtime = Self::from_config(config);
        runtime.open_store()?;
        Ok(runtime)
    }

    #[must_use]
    pub fn status(&self) -> &CoreStatusHandle {
        &self.status
    }

    pub async fn run_jetstream(
        mut self,
        cancellation: CancellationToken,
    ) -> Result<JetStreamReplayReport, CoreRuntimeError> {
        let jetstream = self.config.jetstream_config()?;
        let status_cancellation = cancellation.child_token();
        let status_task = self
            .spawn_status_server(status_cancellation.clone())
            .await?;
        let mut dead_letter = match FileDeadLetterSink::open(self.config.dead_letter_path()) {
            Ok(dead_letter) => dead_letter,
            Err(error) => {
                return self
                    .latch_fail_closed(error.into(), cancellation, status_cancellation, status_task)
                    .await;
            }
        };
        let durable_name = jetstream.durable_name().to_owned();
        let mut source = match JetStreamPullSource::connect(jetstream).await {
            Ok(source) => source,
            Err(error) => {
                let error = persist_connect_fail_closed(&mut dead_letter, &durable_name, error);
                return self
                    .latch_fail_closed(error.into(), cancellation, status_cancellation, status_task)
                    .await;
            }
        };
        if let Err(error) = self.open_store() {
            return self
                .latch_fail_closed(error, cancellation, status_cancellation, status_task)
                .await;
        }
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
        mut self,
        source: &mut Src,
        cancellation: CancellationToken,
    ) -> Result<JetStreamReplayReport, CoreRuntimeError> {
        let status_cancellation = cancellation.child_token();
        let status_task = self
            .spawn_status_server(status_cancellation.clone())
            .await?;
        if let Err(error) = self.open_store() {
            return self
                .latch_fail_closed(error, cancellation, status_cancellation, status_task)
                .await;
        }
        let dead_letter = match FileDeadLetterSink::open(self.config.dead_letter_path()) {
            Ok(dead_letter) => dead_letter,
            Err(error) => {
                return self
                    .latch_fail_closed(error.into(), cancellation, status_cancellation, status_task)
                    .await;
            }
        };
        self.consume_with_status(
            source,
            dead_letter,
            cancellation,
            status_cancellation,
            status_task,
        )
        .await
    }

    fn open_store(&mut self) -> Result<(), CoreRuntimeError> {
        if self.session.is_some() {
            return Ok(());
        }
        let session = Self::open_session(&self.config)?;
        let last_applied = session
            .ledger()
            .checkpoint()
            .map(|checkpoint| checkpoint.block_height().get());
        self.status.set_last_applied_watermark(last_applied);
        self.session = Some(session);
        Ok(())
    }

    fn open_session(config: &CoreConfig) -> Result<CoreSession, CoreRuntimeError> {
        let store =
            SyncedWriteBatchStore::open(config.store_path(), StateImageLimits::production())
                .map_err(CoreRuntimeError::Store)?;
        let jetstream = config.jetstream_config()?;
        Ok(JetStreamReplaySession::open(
            config.chain_id(),
            config.first_height(),
            WatermarkOnlyReducerV1,
            LedgerLimits::production(),
            store,
            StateImageLimits::production(),
        )?
        .with_fetch_batch(jetstream.fetch_batch())?
        .with_dead_letter_consumer(jetstream.durable_name()))
    }

    fn opened(session: &Option<CoreSession>) -> &CoreSession {
        session
            .as_ref()
            .expect("file-store is opened before JetStream consume")
    }

    fn opened_mut(session: &mut Option<CoreSession>) -> &mut CoreSession {
        session
            .as_mut()
            .expect("file-store is opened before JetStream consume")
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

    async fn latch_fail_closed(
        &self,
        error: CoreRuntimeError,
        cancellation: CancellationToken,
        status_cancellation: CancellationToken,
        status_task: Option<JoinHandle<Result<(), StatusError>>>,
    ) -> Result<JetStreamReplayReport, CoreRuntimeError> {
        self.status.fail_closed(error.reason_code());
        if status_task.is_some() {
            cancellation.cancelled().await;
        }
        Self::stop_status(status_cancellation, status_task, Err(error)).await
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
        let (last_height, state_hash) = {
            let session = Self::opened(&self.session);
            (
                session
                    .ledger()
                    .checkpoint()
                    .map(|checkpoint| checkpoint.block_height()),
                session.ledger().state_hash(),
            )
        };
        let mut report = JetStreamReplayReport {
            applied: 0,
            already_applied: 0,
            last_height,
            state_hash,
            live_qualified: false,
            stage_2_qualified: false,
        };
        self.status.record(&report);
        loop {
            tokio::select! {
                biased;
                () = cancellation.cancelled() => return Ok(report),
                result = Self::opened_mut(&mut self.session).consume_available(source, dead_letter) => {
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
