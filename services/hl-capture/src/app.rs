use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use domain_types::{BlockHeight, ChainId, KnownTime};
use storage_ports::{CaptureProgressStore, ProgressError};
use tokio::sync::watch;
use tokio::time::{MissedTickBehavior, interval};
use tokio_util::sync::CancellationToken;

use crate::coordinator::{CaptureCoordinator, CoordinatorError};
use crate::{
    AppError, CaptureHealth, CaptureStatus, OwnedTask, StatusError, StatusWriter, read_status,
    run_owned_tasks,
};

const MAX_RECOVERY_BLOCKS: usize = 10_000_000;
const MAX_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(60);
const MAX_SHUTDOWN_GRACE: Duration = Duration::from_secs(300);
const MAX_BUILD_ID_BYTES: usize = 256;
const RECOVERY_RETRY_DELAY: Duration = Duration::from_millis(250);
const RECOVERING_REASON: &str = "capture_runtime.recovering";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RuntimeHealthSnapshot {
    health: CaptureHealth,
    ready: bool,
    reason_code: Option<&'static str>,
}

#[derive(Debug)]
pub(crate) struct CaptureRuntimeHealth {
    sender: watch::Sender<RuntimeHealthSnapshot>,
}

impl CaptureRuntimeHealth {
    fn new() -> Self {
        let (sender, _receiver) = watch::channel(RuntimeHealthSnapshot {
            health: CaptureHealth::Red,
            ready: false,
            reason_code: Some(RECOVERING_REASON),
        });
        Self { sender }
    }

    pub(crate) fn set_ready(&self) {
        self.sender.send_replace(RuntimeHealthSnapshot {
            health: CaptureHealth::Green,
            ready: true,
            reason_code: None,
        });
    }

    pub(crate) fn set_retryable(&self, reason_code: &'static str) {
        self.sender.send_replace(RuntimeHealthSnapshot {
            health: CaptureHealth::Yellow,
            ready: false,
            reason_code: Some(reason_code),
        });
    }

    pub(crate) fn set_latched(&self, reason_code: &'static str) {
        self.sender.send_replace(RuntimeHealthSnapshot {
            health: CaptureHealth::Red,
            ready: false,
            reason_code: Some(reason_code),
        });
    }

    fn snapshot(&self) -> RuntimeHealthSnapshot {
        *self.sender.borrow()
    }
}

#[derive(Debug, Clone)]
pub struct CaptureRuntimeConfig {
    chain_id: ChainId,
    first_height: BlockHeight,
    recovery_limit: usize,
    heartbeat_interval: Duration,
    shutdown_grace: Duration,
    build_id: String,
}

impl CaptureRuntimeConfig {
    pub fn try_new(
        chain_id: ChainId,
        first_height: BlockHeight,
        recovery_limit: usize,
        heartbeat_interval: Duration,
        shutdown_grace: Duration,
        build_id: impl Into<String>,
    ) -> Result<Self, CaptureRuntimeError> {
        let build_id = build_id.into();
        if !(1..=MAX_RECOVERY_BLOCKS).contains(&recovery_limit)
            || heartbeat_interval.is_zero()
            || heartbeat_interval > MAX_HEARTBEAT_INTERVAL
            || shutdown_grace.is_zero()
            || shutdown_grace > MAX_SHUTDOWN_GRACE
            || build_id.is_empty()
            || build_id.trim() != build_id
            || build_id.len() > MAX_BUILD_ID_BYTES
            || build_id.chars().any(char::is_control)
        {
            return Err(CaptureRuntimeError::InvalidConfig);
        }
        Ok(Self {
            chain_id,
            first_height,
            recovery_limit,
            heartbeat_interval,
            shutdown_grace,
            build_id,
        })
    }
}

pub struct CaptureRuntime {
    config: CaptureRuntimeConfig,
    coordinator: Arc<CaptureCoordinator>,
    progress: Arc<dyn CaptureProgressStore>,
    status_writer: Arc<StatusWriter>,
    health: Arc<CaptureRuntimeHealth>,
}

impl std::fmt::Debug for CaptureRuntime {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CaptureRuntime")
            .field("config", &self.config)
            .field("status_path", &self.status_writer.path())
            .finish_non_exhaustive()
    }
}

impl CaptureRuntime {
    #[must_use]
    pub fn new(
        config: CaptureRuntimeConfig,
        coordinator: Arc<CaptureCoordinator>,
        progress: Arc<dyn CaptureProgressStore>,
        status_writer: Arc<StatusWriter>,
    ) -> Self {
        Self {
            config,
            coordinator,
            progress,
            status_writer,
            health: Arc::new(CaptureRuntimeHealth::new()),
        }
    }

    #[must_use]
    pub const fn chain_id(&self) -> &ChainId {
        &self.config.chain_id
    }

    pub(crate) fn health(&self) -> Arc<CaptureRuntimeHealth> {
        Arc::clone(&self.health)
    }

    pub async fn run(
        self,
        cancellation: CancellationToken,
        mut tasks: Vec<OwnedTask>,
    ) -> Result<(), CaptureRuntimeError> {
        if tasks.is_empty() {
            return Err(CaptureRuntimeError::InvalidConfig);
        }
        let status_context = StatusContext {
            chain_id: self.config.chain_id.clone(),
            build_id: self.config.build_id.clone(),
            recovery_limit: self.config.recovery_limit,
            progress: Arc::clone(&self.progress),
            writer: Arc::clone(&self.status_writer),
        };
        status_context
            .write_without_progress(self.health.snapshot())
            .map_err(status_error)?;

        let recovery_cancellation = cancellation.child_token();
        let recovery_chain = self.config.chain_id.clone();
        let recovery_first_height = self.config.first_height;
        let recovery_limit = self.config.recovery_limit;
        let recovery_progress = Arc::clone(&self.progress);
        let recovery_coordinator = Arc::clone(&self.coordinator);
        let recovery_health = Arc::clone(&self.health);
        tasks.push(OwnedTask::new("recovery-supervisor", async move {
            loop {
                if recovery_cancellation.is_cancelled() {
                    return Ok(());
                }
                match recovery_progress
                    .initialize_chain(&recovery_chain, recovery_first_height)
                    .await
                {
                    Ok(_) => {}
                    Err(ProgressError::Storage(_)) => {
                        recovery_health.set_retryable("capture_progress.storage");
                        wait_for_retry(&recovery_cancellation).await;
                        continue;
                    }
                    Err(error) => {
                        recovery_health.set_latched(error.reason_code());
                        return Err(AppError::TaskFailed {
                            task: "recovery-supervisor",
                            reason_code: error.reason_code(),
                        });
                    }
                }
                match recovery_coordinator
                    .recover_startup(&recovery_chain, recovery_limit)
                    .await
                {
                    Ok(_) => {
                        recovery_health.set_ready();
                        recovery_cancellation.cancelled().await;
                        return Ok(());
                    }
                    Err(CoordinatorError::Publication | CoordinatorError::Progress) => {
                        recovery_health.set_retryable("capture_runtime.downstream_unavailable");
                        wait_for_retry(&recovery_cancellation).await;
                    }
                    Err(error) => {
                        recovery_health.set_latched(error.reason_code());
                        return Err(AppError::TaskFailed {
                            task: "recovery-supervisor",
                            reason_code: error.reason_code(),
                        });
                    }
                }
            }
        }));

        let status_cancellation = cancellation.child_token();
        let status_health = Arc::clone(&self.health);
        let heartbeat_interval = self.config.heartbeat_interval;
        tasks.push(OwnedTask::new("status-heartbeat", async move {
            let mut heartbeat = interval(heartbeat_interval);
            heartbeat.set_missed_tick_behavior(MissedTickBehavior::Skip);
            heartbeat.tick().await;
            loop {
                tokio::select! {
                    () = status_cancellation.cancelled() => return Ok(()),
                    _ = heartbeat.tick() => {
                        status_context
                            .write_current(status_health.snapshot())
                            .await
                            .map_err(|_| AppError::TaskFailed {
                                task: "status-heartbeat",
                                reason_code: "capture_status.refresh",
                            })?;
                    }
                }
            }
        }));

        let result = run_owned_tasks(cancellation, self.config.shutdown_grace, tasks).await;
        let final_health = if result.is_ok() {
            CaptureHealth::Yellow
        } else {
            CaptureHealth::Red
        };
        let final_reason = result
            .as_ref()
            .err()
            .map(|error| error.reason_code().to_owned());
        let terminal_time = now()?;
        let terminal = match read_status(self.status_writer.path()) {
            Ok(status) => status.into_terminal(terminal_time, final_health, final_reason.clone()),
            _ => CaptureStatus::new(
                terminal_time,
                &self.config.build_id,
                self.config.chain_id.clone(),
                final_health,
            )
            .with_last_error_reason(final_reason),
        };
        self.status_writer.write(&terminal).map_err(status_error)?;
        result.map_err(CaptureRuntimeError::Lifecycle)
    }
}

#[derive(Clone)]
struct StatusContext {
    chain_id: ChainId,
    build_id: String,
    recovery_limit: usize,
    progress: Arc<dyn CaptureProgressStore>,
    writer: Arc<StatusWriter>,
}

impl StatusContext {
    async fn write_current(
        &self,
        runtime_health: RuntimeHealthSnapshot,
    ) -> Result<(), CaptureRuntimeError> {
        match self.status_from_progress(runtime_health).await {
            Ok(status) => self.writer.write(&status).map_err(status_error),
            Err(CaptureRuntimeError::Progress) => self
                .write_without_progress(RuntimeHealthSnapshot {
                    health: CaptureHealth::Yellow,
                    ready: false,
                    reason_code: Some("capture_progress.storage"),
                })
                .map_err(status_error),
            Err(error) => Err(error),
        }
    }

    async fn status_from_progress(
        &self,
        runtime_health: RuntimeHealthSnapshot,
    ) -> Result<CaptureStatus, CaptureRuntimeError> {
        let cursor = self
            .progress
            .load_cursor(&self.chain_id)
            .await
            .map_err(progress_error)?;
        let pending = self
            .progress
            .pending_blocks(&self.chain_id, self.recovery_limit)
            .await
            .map_err(progress_error)?;
        let archive_manifest_id = match &cursor {
            Some(cursor) => self
                .progress
                .load_archived_block(&self.chain_id, cursor.committed_block_height())
                .await
                .map_err(progress_error)?
                .map(|plan| plan.archive_manifest_id().to_string()),
            None => None,
        };
        let pending_blocks =
            u64::try_from(pending.len()).map_err(|_| CaptureRuntimeError::StatusOverflow)?;
        Ok(CaptureStatus::new(
            now()?,
            &self.build_id,
            self.chain_id.clone(),
            runtime_health.health,
        )
        .with_readiness(runtime_health.ready)
        .with_durable_height(cursor.map(|cursor| cursor.committed_block_height()))
        .with_pending_blocks(pending_blocks)
        .with_archive_manifest_id(archive_manifest_id)
        .with_last_error_reason(runtime_health.reason_code.map(str::to_owned)))
    }

    fn write_without_progress(
        &self,
        runtime_health: RuntimeHealthSnapshot,
    ) -> Result<(), StatusError> {
        let snapshot_at = now().map_err(|_| StatusError::InvalidField)?;
        let status = match read_status(self.writer.path()) {
            Ok(status) if status.belongs_to(&self.build_id, &self.chain_id) => status
                .into_terminal(
                    snapshot_at,
                    runtime_health.health,
                    runtime_health.reason_code.map(str::to_owned),
                ),
            _ => CaptureStatus::new(
                snapshot_at,
                &self.build_id,
                self.chain_id.clone(),
                runtime_health.health,
            )
            .with_readiness(runtime_health.ready)
            .with_last_error_reason(runtime_health.reason_code.map(str::to_owned)),
        };
        self.writer.write(&status)
    }
}

async fn wait_for_retry(cancellation: &CancellationToken) {
    tokio::select! {
        () = cancellation.cancelled() => {},
        () = tokio::time::sleep(RECOVERY_RETRY_DELAY) => {},
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CaptureRuntimeError {
    #[error("capture runtime configuration is invalid")]
    InvalidConfig,
    #[error("capture runtime coordinator startup failed")]
    Coordinator,
    #[error("capture runtime progress operation failed")]
    Progress,
    #[error("capture runtime status operation failed")]
    Status,
    #[error("capture runtime status counter overflowed")]
    StatusOverflow,
    #[error("capture runtime clock failed")]
    Clock,
    #[error("capture runtime lifecycle failed")]
    Lifecycle(#[source] AppError),
}

impl CaptureRuntimeError {
    #[must_use]
    pub const fn reason_code(&self) -> &'static str {
        match self {
            Self::InvalidConfig => "capture_runtime.invalid_config",
            Self::Coordinator => "capture_runtime.coordinator",
            Self::Progress => "capture_runtime.progress",
            Self::Status => "capture_runtime.status",
            Self::StatusOverflow => "capture_runtime.status_overflow",
            Self::Clock => "capture_runtime.clock",
            Self::Lifecycle(error) => error.reason_code(),
        }
    }
}

fn now() -> Result<KnownTime, CaptureRuntimeError> {
    let micros = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| CaptureRuntimeError::Clock)?
        .as_micros();
    let micros = i64::try_from(micros).map_err(|_| CaptureRuntimeError::Clock)?;
    KnownTime::from_unix_micros(micros).map_err(|_| CaptureRuntimeError::Clock)
}

fn progress_error(_error: ProgressError) -> CaptureRuntimeError {
    CaptureRuntimeError::Progress
}

fn status_error(_error: StatusError) -> CaptureRuntimeError {
    CaptureRuntimeError::Status
}
