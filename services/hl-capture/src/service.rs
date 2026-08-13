use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use canonical_archive::{ArchiveConfig, LocalParquetArchive, RawV3Archive};
use domain_types::BlockHeight;
use storage_ports::{CanonicalArchive, CaptureProgressStore, RawObservationArchive};
use tokio_postgres::config::{Host, SslMode};
use tokio_util::sync::CancellationToken;

use crate::bus::{JetStreamAuthentication, JetStreamConfig, ReconnectingJetStreamPublisher};
use crate::coordinator::{
    BlockingCanonicalArchive, CaptureCoordinator, NoCoordinatorFaults, SystemAcknowledgementClock,
};
use crate::progress::ReconnectingPostgresProgressStore;
use crate::secret::read_protected_secret;
use crate::source_runtime::{auxiliary_node_task, committed_node_tasks};
use crate::{
    AppError, BlockingRawSegmentArchive, CaptureConfig, CaptureRawObservationArchive,
    CaptureRuntime, CaptureRuntimeConfig, CaptureRuntimeError, OwnedTask, RawArchiveFormat,
    RawSegmentArchive, StatusWriter, synthetic_fixture_block,
};

const BUILD_ID: &str = concat!("hl-capture/", env!("CARGO_PKG_VERSION"));
const STATUS_HEARTBEAT: Duration = Duration::from_secs(1);
const MAX_FIXTURE_BLOCKS: u64 = 10_000_000;
const MAX_FIXTURE_DELAY: Duration = Duration::from_secs(60);

pub struct ConnectedCapture {
    config: CaptureConfig,
    runtime: CaptureRuntime,
    coordinator: Arc<CaptureCoordinator>,
    progress: Arc<dyn CaptureProgressStore>,
    raw_archive: Arc<dyn RawSegmentArchive>,
    failover_store: Arc<crate::FailoverStore>,
    infrastructure_tasks: Vec<OwnedTask>,
}

impl std::fmt::Debug for ConnectedCapture {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ConnectedCapture")
            .field(
                "infrastructure_task_count",
                &self.infrastructure_tasks.len(),
            )
            .finish_non_exhaustive()
    }
}

impl ConnectedCapture {
    pub async fn run(mut self, cancellation: CancellationToken) -> Result<(), CaptureRuntimeError> {
        let health = self.runtime.health();
        let source_tasks = committed_node_tasks(
            &self.config,
            Arc::clone(&self.progress),
            Arc::clone(&self.coordinator),
            Arc::clone(&self.raw_archive),
            Arc::clone(&self.failover_store),
            Arc::clone(&health),
            cancellation.child_token(),
        )
        .map_err(|_| CaptureRuntimeError::InvalidConfig)?;
        self.infrastructure_tasks.extend(source_tasks);
        if let Some(auxiliary_task) = auxiliary_node_task(
            &self.config,
            Arc::clone(&self.raw_archive),
            health,
            cancellation.child_token(),
        )
        .map_err(|_| CaptureRuntimeError::InvalidConfig)?
        {
            self.infrastructure_tasks.push(auxiliary_task);
        }
        self.runtime
            .run(cancellation, self.infrastructure_tasks)
            .await
    }

    pub async fn run_fixture(
        mut self,
        cancellation: CancellationToken,
        block_count: u64,
        block_delay: Duration,
    ) -> Result<(), CaptureRuntimeError> {
        if block_count == 0 || block_count > MAX_FIXTURE_BLOCKS || block_delay > MAX_FIXTURE_DELAY {
            return Err(CaptureRuntimeError::InvalidConfig);
        }
        let fixture_cancellation = cancellation.child_token();
        let fixture_coordinator = Arc::clone(&self.coordinator);
        let fixture_progress = Arc::clone(&self.progress);
        let chain_id = fixture_runtime_chain(&self.runtime);
        self.infrastructure_tasks
            .push(OwnedTask::new("synthetic-fixture", async move {
                let first_height = fixture_progress
                    .next_expected_height(&chain_id)
                    .await
                    .map_err(|_| AppError::TaskFailed {
                        task: "synthetic-fixture",
                        reason_code: "capture_fixture.progress",
                    })?;
                for offset in 0..block_count {
                    if fixture_cancellation.is_cancelled() {
                        return Ok(());
                    }
                    let height = first_height
                        .get()
                        .checked_add(offset)
                        .map(BlockHeight::new)
                        .ok_or(AppError::TaskFailed {
                            task: "synthetic-fixture",
                            reason_code: "capture_fixture.height_overflow",
                        })?;
                    let block = synthetic_fixture_block(&chain_id, height).map_err(|error| {
                        AppError::TaskFailed {
                            task: "synthetic-fixture",
                            reason_code: error.reason_code(),
                        }
                    })?;
                    fixture_coordinator
                        .process_block(&block)
                        .await
                        .map_err(|error| AppError::TaskFailed {
                            task: "synthetic-fixture",
                            reason_code: error.reason_code(),
                        })?;
                    if !block_delay.is_zero() {
                        tokio::select! {
                            () = fixture_cancellation.cancelled() => return Ok(()),
                            () = tokio::time::sleep(block_delay) => {}
                        }
                    }
                }
                fixture_cancellation.cancelled().await;
                Ok(())
            }));
        self.runtime
            .run(cancellation, self.infrastructure_tasks)
            .await
    }
}

pub async fn connect_capture(
    config: &CaptureConfig,
    _cancellation: &CancellationToken,
) -> Result<ConnectedCapture, RuntimeConnectError> {
    validate_nats_transport(config.runtime().nats_server_url())?;
    let postgres_secret = read_protected_secret(config.runtime().postgres_url_path())
        .map_err(|_| RuntimeConnectError::Secret)?;
    let postgres_config = tokio_postgres::Config::from_str(&postgres_secret)
        .map_err(|_| RuntimeConnectError::PostgresConfig)?;
    validate_development_postgres(&postgres_config)?;
    let progress: Arc<dyn CaptureProgressStore> = Arc::new(
        ReconnectingPostgresProgressStore::try_new(
            postgres_config,
            Duration::from_millis(config.runtime().postgres_operation_timeout_millis()),
        )
        .map_err(|_| RuntimeConnectError::PostgresConfig)?,
    );
    let archive_config =
        ArchiveConfig::production(BUILD_ID).map_err(|_| RuntimeConnectError::Archive)?;
    let archive = Arc::new(
        LocalParquetArchive::open(config.runtime().archive_path(), archive_config.clone())
            .map_err(|_| RuntimeConnectError::Archive)?,
    );
    let canonical_archive: Arc<dyn CanonicalArchive> = archive.clone();
    let v3_raw = match config.runtime().raw_archive_format() {
        RawArchiveFormat::V2 => None,
        RawArchiveFormat::V3 => {
            let raw_v3 = config
                .runtime()
                .raw_v3()
                .ok_or(RuntimeConnectError::Archive)?;
            Some(Arc::new(
                RawV3Archive::open(
                    config.runtime().archive_path(),
                    archive_config,
                    raw_v3
                        .workload()
                        .map_err(|_| RuntimeConnectError::Archive)?,
                    raw_v3.budgets().map_err(|_| RuntimeConnectError::Archive)?,
                )
                .map_err(|_| RuntimeConnectError::Archive)?,
            ))
        }
    };
    let raw_observation_archive: Arc<dyn RawObservationArchive> =
        Arc::new(CaptureRawObservationArchive::new(archive, v3_raw));
    let raw_archive: Arc<dyn RawSegmentArchive> =
        Arc::new(BlockingRawSegmentArchive::new(raw_observation_archive));
    let failover_store = Arc::new(
        crate::FailoverStore::new(config.runtime().failover_state_path().to_path_buf())
            .map_err(|_| RuntimeConnectError::FailoverState)?,
    );
    failover_store
        .load()
        .map_err(|_| RuntimeConnectError::FailoverState)?;
    let publisher_config = JetStreamConfig::try_new(
        config.runtime().nats_server_url(),
        JetStreamAuthentication::UserPasswordFile {
            username: config.runtime().nats_username().to_owned(),
            password_path: config.runtime().nats_password_path().to_path_buf(),
        },
        Duration::from_millis(config.runtime().publish_timeout_millis()),
        Duration::from_millis(config.runtime().publish_timeout_millis()),
        config.runtime().nats_max_ack_inflight(),
        config.runtime().publisher_ledger_capacity(),
    )
    .map_err(|_| RuntimeConnectError::NatsConfig)?;
    let publisher = Arc::new(ReconnectingJetStreamPublisher::new(publisher_config));
    let coordinator = Arc::new(CaptureCoordinator::new(
        Arc::new(BlockingCanonicalArchive::new(canonical_archive)),
        Arc::clone(&progress),
        publisher,
        Arc::new(SystemAcknowledgementClock),
        Arc::new(NoCoordinatorFaults),
    ));
    let runtime_config = CaptureRuntimeConfig::try_new(
        config.runtime().chain_id(),
        config.runtime().first_height(),
        config.runtime().max_pending_blocks(),
        STATUS_HEARTBEAT,
        Duration::from_millis(config.runtime().shutdown_grace_millis()),
        BUILD_ID,
    )
    .map_err(|_| RuntimeConnectError::RuntimeConfig)?;
    let status_writer = Arc::new(
        StatusWriter::new(config.runtime().status_path().to_path_buf())
            .map_err(|_| RuntimeConnectError::Status)?,
    );
    let runtime = CaptureRuntime::new(
        runtime_config,
        Arc::clone(&coordinator),
        Arc::clone(&progress),
        status_writer,
    );
    Ok(ConnectedCapture {
        config: config.clone(),
        runtime,
        coordinator,
        progress,
        raw_archive,
        failover_store,
        infrastructure_tasks: Vec::new(),
    })
}

fn fixture_runtime_chain(runtime: &CaptureRuntime) -> domain_types::ChainId {
    runtime.chain_id().clone()
}

fn validate_development_postgres(
    config: &tokio_postgres::Config,
) -> Result<(), RuntimeConnectError> {
    if config.get_ssl_mode() != SslMode::Disable
        || config.get_hosts().len() != 1
        || !matches!(
            &config.get_hosts()[0],
            Host::Tcp(host) if host == "127.0.0.1" || host == "::1"
        )
    {
        return Err(RuntimeConnectError::UnsafeDevelopmentTransport);
    }
    Ok(())
}

fn validate_nats_transport(server_url: &str) -> Result<(), RuntimeConnectError> {
    let address = server_url
        .parse::<async_nats::ServerAddr>()
        .map_err(|_| RuntimeConnectError::UnsafeDevelopmentTransport)?;
    if address.scheme() == "tls"
        || (address.scheme() == "nats" && matches!(address.host(), "127.0.0.1" | "::1"))
    {
        Ok(())
    } else {
        Err(RuntimeConnectError::UnsafeDevelopmentTransport)
    }
}

#[derive(Debug, thiserror::Error, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeConnectError {
    #[error("capture runtime secret file is unavailable or unsafe")]
    Secret,
    #[error("capture PostgreSQL configuration is invalid")]
    PostgresConfig,
    #[error("capture development transport must be encrypted or loopback-only")]
    UnsafeDevelopmentTransport,
    #[error("capture archive initialization failed")]
    Archive,
    #[error("capture NATS configuration is invalid")]
    NatsConfig,
    #[error("capture runtime configuration is invalid")]
    RuntimeConfig,
    #[error("capture status initialization failed")]
    Status,
    #[error("capture committed-source failover state is invalid")]
    FailoverState,
}

impl RuntimeConnectError {
    #[must_use]
    pub const fn reason_code(self) -> &'static str {
        match self {
            Self::Secret => "capture_connect.secret",
            Self::PostgresConfig => "capture_connect.postgres_config",
            Self::UnsafeDevelopmentTransport => "capture_connect.unsafe_transport",
            Self::Archive => "capture_connect.archive",
            Self::NatsConfig => "capture_connect.nats_config",
            Self::RuntimeConfig => "capture_connect.runtime_config",
            Self::Status => "capture_connect.status",
            Self::FailoverState => "capture_connect.failover_state",
        }
    }
}
