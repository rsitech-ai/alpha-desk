use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use domain_types::{BlockHeight, ChainId, KnownTime};
use storage_ports::{CaptureProgressStore, ProgressError};
use tokio::sync::watch;
use tokio::time::{MissedTickBehavior, interval};
use tokio_util::sync::CancellationToken;

use crate::coordinator::{CaptureCoordinator, CoordinatorError};
use crate::{
    AppError, AuxiliarySourceStatus, CaptureHealth, CaptureSourceHealth, CaptureStatus,
    CommittedSourceClass, FailoverDecision, FailoverReason, OwnedTask, StatusError, StatusWriter,
    read_status, run_owned_tasks,
};

const MAX_RECOVERY_BLOCKS: usize = 10_000_000;
const MAX_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(60);
const MAX_SHUTDOWN_GRACE: Duration = Duration::from_secs(300);
const MAX_BUILD_ID_BYTES: usize = 256;
const RECOVERY_RETRY_DELAY: Duration = Duration::from_millis(250);
const RECOVERING_REASON: &str = "capture_runtime.recovering";
const DISK_HEALTHY_BASIS_POINTS: u16 = 2_000;
const LOW_DISK_REASON: &str = "capture_disk.low_space";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RuntimeHealthSnapshot {
    health: CaptureHealth,
    ready: bool,
    reason_code: Option<&'static str>,
    active_committed_source: CommittedSourceClass,
    primary_source_health: CaptureSourceHealth,
    independent_source_health: Option<CaptureSourceHealth>,
    failover_height: Option<BlockHeight>,
    failover_reason: Option<FailoverReason>,
    latest_primary_height: Option<BlockHeight>,
    latest_independent_height: Option<BlockHeight>,
    next_expected_capture_height: Option<BlockHeight>,
    capture_backlog_records: u64,
    oldest_pending_capture_height: Option<BlockHeight>,
    disk_free_basis_points: Option<u16>,
    auxiliary_sources: BTreeMap<String, AuxiliarySourceStatus>,
}

#[derive(Debug)]
pub(crate) struct CaptureRuntimeHealth {
    sender: watch::Sender<RuntimeHealthSnapshot>,
}

impl CaptureRuntimeHealth {
    pub(crate) fn new() -> Self {
        let (sender, _receiver) = watch::channel(RuntimeHealthSnapshot {
            health: CaptureHealth::Red,
            ready: false,
            reason_code: Some(RECOVERING_REASON),
            active_committed_source: CommittedSourceClass::LocallyVerifiedCommitted,
            primary_source_health: CaptureSourceHealth::Healthy,
            independent_source_health: None,
            failover_height: None,
            failover_reason: None,
            latest_primary_height: None,
            latest_independent_height: None,
            next_expected_capture_height: None,
            capture_backlog_records: 0,
            oldest_pending_capture_height: None,
            disk_free_basis_points: None,
            auxiliary_sources: BTreeMap::new(),
        });
        Self { sender }
    }

    pub(crate) fn set_ready(&self) {
        self.sender.send_modify(|snapshot| {
            let active_health = match snapshot.active_committed_source {
                CommittedSourceClass::LocallyVerifiedCommitted => snapshot.primary_source_health,
                CommittedSourceClass::IndependentCommitted => snapshot
                    .independent_source_health
                    .unwrap_or(CaptureSourceHealth::Starting),
            };
            match active_health {
                CaptureSourceHealth::Starting => {
                    snapshot.health = CaptureHealth::Yellow;
                    snapshot.ready = false;
                    snapshot.reason_code = Some("capture_source.active_starting");
                    return;
                }
                CaptureSourceHealth::RangeUnavailable => {
                    snapshot.health = CaptureHealth::Red;
                    snapshot.ready = false;
                    snapshot.reason_code = Some(match snapshot.active_committed_source {
                        CommittedSourceClass::LocallyVerifiedCommitted => {
                            "capture_source.primary_range_unavailable"
                        }
                        CommittedSourceClass::IndependentCommitted => {
                            "capture_failover.independent_range_unavailable"
                        }
                    });
                    return;
                }
                CaptureSourceHealth::Healthy => {}
            }
            if snapshot.active_committed_source == CommittedSourceClass::IndependentCommitted {
                snapshot.health = CaptureHealth::Yellow;
                snapshot.reason_code = Some("capture_failover.independent_source_active");
            } else if snapshot
                .disk_free_basis_points
                .is_some_and(|basis_points| basis_points < DISK_HEALTHY_BASIS_POINTS)
            {
                snapshot.health = CaptureHealth::Yellow;
                snapshot.reason_code = Some(LOW_DISK_REASON);
            } else {
                snapshot.health = CaptureHealth::Green;
                snapshot.reason_code = None;
            }
            snapshot.ready = true;
        });
    }

    pub(crate) fn set_retryable(&self, reason_code: &'static str) {
        self.sender.send_modify(|snapshot| {
            snapshot.health = CaptureHealth::Yellow;
            snapshot.ready = false;
            snapshot.reason_code = Some(reason_code);
        });
    }

    pub(crate) fn configure_committed_sources(
        &self,
        has_independent: bool,
        failover: Option<&FailoverDecision>,
    ) {
        self.sender.send_modify(|snapshot| {
            snapshot.primary_source_health = CaptureSourceHealth::Starting;
            snapshot.independent_source_health =
                has_independent.then_some(CaptureSourceHealth::Starting);
            if let Some(decision) = failover {
                snapshot.active_committed_source = CommittedSourceClass::IndependentCommitted;
                snapshot.failover_height = Some(decision.failover_height());
                snapshot.failover_reason = Some(decision.reason());
            }
            refresh_capture_backlog(snapshot);
        });
    }

    pub(crate) fn activate_independent(&self, decision: &FailoverDecision) {
        self.sender.send_modify(|snapshot| {
            snapshot.active_committed_source = CommittedSourceClass::IndependentCommitted;
            snapshot.failover_height = Some(decision.failover_height());
            snapshot.failover_reason = Some(decision.reason());
            snapshot.independent_source_health = Some(CaptureSourceHealth::Healthy);
            snapshot.health = CaptureHealth::Yellow;
            snapshot.ready = true;
            snapshot.reason_code = Some("capture_failover.independent_source_active");
            refresh_capture_backlog(snapshot);
        });
    }

    pub(crate) fn record_source_gap(&self, source: CommittedSourceClass) {
        self.sender.send_modify(|snapshot| match source {
            CommittedSourceClass::LocallyVerifiedCommitted => {
                snapshot.primary_source_health = CaptureSourceHealth::RangeUnavailable;
            }
            CommittedSourceClass::IndependentCommitted => {
                snapshot.independent_source_health = Some(CaptureSourceHealth::RangeUnavailable);
            }
        });
    }

    pub(crate) fn record_source_healthy(&self, source: CommittedSourceClass) {
        self.sender.send_modify(|snapshot| match source {
            CommittedSourceClass::LocallyVerifiedCommitted => {
                snapshot.primary_source_health = CaptureSourceHealth::Healthy;
            }
            CommittedSourceClass::IndependentCommitted => {
                snapshot.independent_source_health = Some(CaptureSourceHealth::Healthy);
            }
        });
    }

    pub(crate) fn set_latched(&self, reason_code: &'static str) {
        self.sender.send_modify(|snapshot| {
            snapshot.health = CaptureHealth::Red;
            snapshot.ready = false;
            snapshot.reason_code = Some(reason_code);
        });
    }

    pub(crate) fn record_capture(
        &self,
        source: CommittedSourceClass,
        captured_height: BlockHeight,
        disk_free_basis_points: u16,
    ) {
        self.sender.send_modify(|snapshot| {
            match source {
                CommittedSourceClass::LocallyVerifiedCommitted => {
                    snapshot.latest_primary_height = Some(
                        snapshot
                            .latest_primary_height
                            .map_or(captured_height, |current| current.max(captured_height)),
                    );
                    snapshot.primary_source_health = CaptureSourceHealth::Healthy;
                }
                CommittedSourceClass::IndependentCommitted => {
                    snapshot.latest_independent_height = Some(
                        snapshot
                            .latest_independent_height
                            .map_or(captured_height, |current| current.max(captured_height)),
                    );
                    snapshot.independent_source_health = Some(CaptureSourceHealth::Healthy);
                }
            }
            snapshot.disk_free_basis_points = Some(disk_free_basis_points);
            refresh_capture_backlog(snapshot);
        });
    }

    pub(crate) fn record_disk_capacity(&self, disk_free_basis_points: u16) {
        self.sender.send_modify(|snapshot| {
            snapshot.disk_free_basis_points = Some(disk_free_basis_points);
        });
    }

    pub(crate) fn record_next_expected(&self, next_expected: BlockHeight) {
        self.sender.send_modify(|snapshot| {
            snapshot.next_expected_capture_height = Some(next_expected);
            refresh_capture_backlog(snapshot);
        });
    }

    pub(crate) fn configure_auxiliary_sources(&self, source_ids: &[String]) {
        self.sender.send_modify(|snapshot| {
            snapshot.auxiliary_sources = source_ids
                .iter()
                .map(|source_id| {
                    (
                        source_id.clone(),
                        AuxiliarySourceStatus::starting(source_id.clone()),
                    )
                })
                .collect();
        });
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn record_auxiliary_durable(
        &self,
        source_id: &str,
        cursor_epoch: &str,
        tail_cursor_epoch: &str,
        durable_offset: u64,
        local_sequence: u64,
        unread_bytes: u64,
        partial_line: bool,
        last_durable_wall_micros: i64,
        quarantine_reason: Option<&str>,
    ) {
        self.sender.send_modify(|snapshot| {
            if let Some(source) = snapshot.auxiliary_sources.get_mut(source_id) {
                source.record_durable(
                    cursor_epoch,
                    tail_cursor_epoch,
                    durable_offset,
                    local_sequence,
                    unread_bytes,
                    partial_line,
                    last_durable_wall_micros,
                    quarantine_reason,
                );
            }
        });
    }

    pub(crate) fn record_auxiliary_recovered(
        &self,
        source_id: &str,
        durable_cursor: &hl_protocol::SourceCursor,
        local_sequence: u64,
        last_durable_wall_micros: i64,
        quarantine_reason: Option<&str>,
    ) {
        self.sender.send_modify(|snapshot| {
            if let Some(source) = snapshot.auxiliary_sources.get_mut(source_id) {
                source.record_recovered(
                    durable_cursor.epoch(),
                    durable_cursor.offset(),
                    local_sequence,
                    last_durable_wall_micros,
                    quarantine_reason,
                );
            }
        });
    }

    pub(crate) fn record_auxiliary_tail(
        &self,
        source_id: &str,
        cursor_epoch: &str,
        unread_bytes: u64,
        partial_line: bool,
    ) {
        self.sender.send_modify(|snapshot| {
            if let Some(source) = snapshot.auxiliary_sources.get_mut(source_id) {
                source.record_tail(cursor_epoch, unread_bytes, partial_line);
            }
        });
    }

    pub(crate) fn record_auxiliary_buffered(
        &self,
        source_id: &str,
        cursor_epoch: &str,
        spool_records: u64,
        unarchived_records: u64,
        unread_bytes: u64,
        partial_line: bool,
    ) {
        self.sender.send_modify(|snapshot| {
            if let Some(source) = snapshot.auxiliary_sources.get_mut(source_id) {
                source.record_buffered(
                    cursor_epoch,
                    spool_records,
                    unarchived_records,
                    unread_bytes,
                    partial_line,
                );
            }
        });
    }

    pub(crate) fn latch_auxiliary(&self, source_id: &str, reason_code: &str) {
        self.sender.send_modify(|snapshot| {
            if let Some(source) = snapshot.auxiliary_sources.get_mut(source_id) {
                source.latch(reason_code);
            }
        });
    }

    pub(crate) fn retry_auxiliary(&self, source_id: &str, reason_code: &str) {
        self.sender.send_modify(|snapshot| {
            if let Some(source) = snapshot.auxiliary_sources.get_mut(source_id) {
                source.retrying(reason_code);
            }
        });
    }

    pub(crate) fn recover_auxiliary_retry(&self, source_id: &str) {
        self.sender.send_modify(|snapshot| {
            if let Some(source) = snapshot.auxiliary_sources.get_mut(source_id) {
                source.retry_recovered();
            }
        });
    }

    #[cfg(test)]
    pub(crate) fn auxiliary_source_status(&self, source_id: &str) -> Option<AuxiliarySourceStatus> {
        self.sender
            .borrow()
            .auxiliary_sources
            .get(source_id)
            .cloned()
    }

    fn snapshot(&self) -> RuntimeHealthSnapshot {
        self.sender.borrow().clone()
    }
}

fn refresh_capture_backlog(snapshot: &mut RuntimeHealthSnapshot) {
    let latest = match snapshot.active_committed_source {
        CommittedSourceClass::LocallyVerifiedCommitted => snapshot.latest_primary_height,
        CommittedSourceClass::IndependentCommitted => snapshot.latest_independent_height,
    };
    let Some(latest) = latest else {
        snapshot.capture_backlog_records = 0;
        snapshot.oldest_pending_capture_height = None;
        return;
    };
    let Some(next) = snapshot.next_expected_capture_height else {
        snapshot.capture_backlog_records = 0;
        snapshot.oldest_pending_capture_height = None;
        return;
    };
    if latest < next {
        snapshot.capture_backlog_records = 0;
        snapshot.oldest_pending_capture_height = None;
        return;
    }
    snapshot.capture_backlog_records = latest.get().saturating_sub(next.get()).saturating_add(1);
    snapshot.oldest_pending_capture_height = Some(next);
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
        let terminal_source_state = self.health.snapshot();
        let terminal_auxiliary_sources = terminal_source_state
            .auxiliary_sources
            .values()
            .cloned()
            .collect();
        let terminal = match read_status(self.status_writer.path()) {
            Ok(status) => status.into_terminal(terminal_time, final_health, final_reason.clone()),
            _ => CaptureStatus::new(
                terminal_time,
                &self.config.build_id,
                self.config.chain_id.clone(),
                final_health,
            )
            .with_source_state(
                terminal_source_state.active_committed_source,
                terminal_source_state.primary_source_health,
                terminal_source_state.independent_source_health,
                terminal_source_state.failover_height,
                terminal_source_state.failover_reason,
            )
            .with_last_error_reason(final_reason),
        }
        .with_auxiliary_sources(terminal_auxiliary_sources);
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
        match self.status_from_progress(runtime_health.clone()).await {
            Ok(status) => self.writer.write(&status).map_err(status_error),
            Err(CaptureRuntimeError::Progress) => self
                .write_without_progress(RuntimeHealthSnapshot {
                    health: CaptureHealth::Yellow,
                    ready: false,
                    reason_code: Some("capture_progress.storage"),
                    ..runtime_health
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
        let auxiliary_sources = runtime_health.auxiliary_sources.values().cloned().collect();
        Ok(CaptureStatus::new(
            now()?,
            &self.build_id,
            self.chain_id.clone(),
            runtime_health.health,
        )
        .with_readiness(runtime_health.ready)
        .with_source_state(
            runtime_health.active_committed_source,
            runtime_health.primary_source_health,
            runtime_health.independent_source_health,
            runtime_health.failover_height,
            runtime_health.failover_reason,
        )
        .with_durable_height(cursor.map(|cursor| cursor.committed_block_height()))
        .with_pending_blocks(pending_blocks)
        .with_capture_capacity(
            runtime_health.capture_backlog_records,
            runtime_health.oldest_pending_capture_height,
            runtime_health.disk_free_basis_points,
        )
        .with_archive_manifest_id(archive_manifest_id)
        .with_last_error_reason(runtime_health.reason_code.map(str::to_owned))
        .with_auxiliary_sources(auxiliary_sources))
    }

    fn write_without_progress(
        &self,
        runtime_health: RuntimeHealthSnapshot,
    ) -> Result<(), StatusError> {
        let snapshot_at = now().map_err(|_| StatusError::InvalidField)?;
        let auxiliary_sources = runtime_health.auxiliary_sources.values().cloned().collect();
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
        }
        .with_source_state(
            runtime_health.active_committed_source,
            runtime_health.primary_source_health,
            runtime_health.independent_source_health,
            runtime_health.failover_height,
            runtime_health.failover_reason,
        )
        .with_capture_capacity(
            runtime_health.capture_backlog_records,
            runtime_health.oldest_pending_capture_height,
            runtime_health.disk_free_basis_points,
        )
        .with_auxiliary_sources(auxiliary_sources);
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

#[cfg(test)]
mod tests {
    use domain_types::{BlockHeight, ChainId, SourceId};

    use crate::{CommittedSourceClass, FailoverDecision, FailoverReason};

    use super::CaptureRuntimeHealth;

    #[test]
    fn capture_telemetry_tracks_backlog_boundary_and_disk_percentage() {
        let health = CaptureRuntimeHealth::new();
        health.record_next_expected(BlockHeight::new(41));
        health.record_capture(
            CommittedSourceClass::LocallyVerifiedCommitted,
            BlockHeight::new(43),
            2_345,
        );

        let snapshot = health.snapshot();
        assert_eq!(snapshot.capture_backlog_records, 3);
        assert_eq!(
            snapshot.oldest_pending_capture_height,
            Some(BlockHeight::new(41))
        );
        assert_eq!(snapshot.disk_free_basis_points, Some(2_345));

        health.record_next_expected(BlockHeight::new(44));
        let caught_up = health.snapshot();
        assert_eq!(caught_up.capture_backlog_records, 0);
        assert_eq!(caught_up.oldest_pending_capture_height, None);
        assert_eq!(caught_up.disk_free_basis_points, Some(2_345));
    }

    #[test]
    fn ready_health_is_yellow_below_twenty_percent_disk_free() {
        let health = CaptureRuntimeHealth::new();
        health.record_disk_capacity(1_500);

        health.set_ready();

        let snapshot = health.snapshot();
        assert_eq!(snapshot.health, super::CaptureHealth::Yellow);
        assert!(snapshot.ready);
        assert_eq!(snapshot.reason_code, Some("capture_disk.low_space"));
    }

    #[test]
    fn configured_source_must_open_healthy_before_readiness() {
        let health = CaptureRuntimeHealth::new();
        health.configure_committed_sources(true, None);

        health.set_ready();

        let starting = health.snapshot();
        assert_eq!(starting.health, super::CaptureHealth::Yellow);
        assert!(!starting.ready);
        assert_eq!(starting.reason_code, Some("capture_source.active_starting"));

        health.record_source_healthy(CommittedSourceClass::LocallyVerifiedCommitted);
        health.set_ready();
        assert!(health.snapshot().ready);
    }

    #[test]
    fn recovery_cannot_promote_an_active_independent_source_to_green() {
        let health = CaptureRuntimeHealth::new();
        let decision = FailoverDecision::try_new(
            ChainId::new("mainnet").unwrap(),
            SourceId::new("primary-node").unwrap(),
            SourceId::new("independent-node").unwrap(),
            BlockHeight::new(42),
            FailoverReason::PrimaryRangeUnavailable,
        )
        .unwrap();
        health.configure_committed_sources(true, Some(&decision));
        health.record_source_healthy(CommittedSourceClass::IndependentCommitted);

        health.set_ready();

        let snapshot = health.snapshot();
        assert_eq!(snapshot.health, super::CaptureHealth::Yellow);
        assert!(snapshot.ready);
        assert_eq!(
            snapshot.reason_code,
            Some("capture_failover.independent_source_active")
        );
        assert_eq!(
            snapshot.independent_source_health,
            Some(crate::CaptureSourceHealth::Healthy)
        );
    }
}
