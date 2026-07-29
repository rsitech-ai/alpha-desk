use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use domain_types::{BlockHeight, ChainId, KnownTime};
use storage_ports::{CaptureProgressStore, ProgressError};
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
        }
    }

    #[must_use]
    pub const fn chain_id(&self) -> &ChainId {
        &self.config.chain_id
    }

    pub async fn run(
        self,
        cancellation: CancellationToken,
        mut tasks: Vec<OwnedTask>,
    ) -> Result<(), CaptureRuntimeError> {
        if tasks.is_empty() {
            return Err(CaptureRuntimeError::InvalidConfig);
        }
        self.progress
            .initialize_chain(&self.config.chain_id, self.config.first_height)
            .await
            .map_err(progress_error)?;
        self.coordinator
            .recover_startup(&self.config.chain_id, self.config.recovery_limit)
            .await
            .map_err(coordinator_error)?;
        self.write_snapshot(CaptureHealth::Green, true, None)
            .await?;

        let status_cancellation = cancellation.child_token();
        let status_context = StatusContext {
            chain_id: self.config.chain_id.clone(),
            build_id: self.config.build_id.clone(),
            recovery_limit: self.config.recovery_limit,
            progress: Arc::clone(&self.progress),
            writer: Arc::clone(&self.status_writer),
        };
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
                            .write(CaptureHealth::Green, true, None)
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
        if self
            .write_snapshot(final_health, false, final_reason.clone())
            .await
            .is_err()
        {
            let last_verified = read_status(self.status_writer.path()).map_err(status_error)?;
            self.status_writer
                .write(&last_verified.into_terminal(now()?, final_health, final_reason))
                .map_err(status_error)?;
        }
        result.map_err(CaptureRuntimeError::Lifecycle)
    }

    async fn write_snapshot(
        &self,
        health: CaptureHealth,
        ready: bool,
        last_error_reason: Option<String>,
    ) -> Result<(), CaptureRuntimeError> {
        StatusContext {
            chain_id: self.config.chain_id.clone(),
            build_id: self.config.build_id.clone(),
            recovery_limit: self.config.recovery_limit,
            progress: Arc::clone(&self.progress),
            writer: Arc::clone(&self.status_writer),
        }
        .write(health, ready, last_error_reason)
        .await
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
    async fn write(
        &self,
        health: CaptureHealth,
        ready: bool,
        last_error_reason: Option<String>,
    ) -> Result<(), CaptureRuntimeError> {
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
        let status = CaptureStatus::new(now()?, &self.build_id, self.chain_id.clone(), health)
            .with_readiness(ready)
            .with_durable_height(cursor.map(|cursor| cursor.committed_block_height()))
            .with_pending_blocks(pending_blocks)
            .with_archive_manifest_id(archive_manifest_id)
            .with_last_error_reason(last_error_reason);
        self.writer.write(&status).map_err(status_error)
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

fn coordinator_error(_error: CoordinatorError) -> CaptureRuntimeError {
    CaptureRuntimeError::Coordinator
}

fn progress_error(_error: ProgressError) -> CaptureRuntimeError {
    CaptureRuntimeError::Progress
}

fn status_error(_error: StatusError) -> CaptureRuntimeError {
    CaptureRuntimeError::Status
}
