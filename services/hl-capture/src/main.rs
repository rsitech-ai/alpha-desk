#![forbid(unsafe_code)]

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use domain_types::{ChainId, SourceId};
use hl_capture::{
    AuthorizedRestoreRequest, CaptureConfig, connect_capture, read_status,
    run_configured_maintenance_cycle, run_configured_restore,
};
use serde::Serialize;
use tokio_util::sync::CancellationToken;

const MAX_CONFIG_BYTES: u64 = 1024 * 1024;
const USAGE: &str = "usage: hl-capture <check-config|status|run|maintain> --config <path> [--json]\n       hl-capture fixture-replay --config <path> --blocks <count> [--block-delay-millis <ms>]\n       hl-capture restore --config <path> --dest <path> --backup-root <path> --chain <id> --source <id> --plan-digest <hex> --backup-receipt <hex> --i-approve-restore";

#[tokio::main]
async fn main() -> ExitCode {
    match execute(std::env::args_os().skip(1).collect()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(CliError::Usage) => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
        Err(error) => {
            let output = ErrorOutput {
                schema_version: "hl.capture.error.v1",
                reason_code: error.reason_code(),
            };
            match serde_json::to_string(&output) {
                Ok(encoded) => eprintln!("{encoded}"),
                Err(_) => eprintln!(
                    "{{\"schema_version\":\"hl.capture.error.v1\",\"reason_code\":\"capture_cli.serialization\"}}"
                ),
            }
            ExitCode::FAILURE
        }
    }
}

async fn execute(arguments: Vec<OsString>) -> Result<(), CliError> {
    let command = Command::parse(arguments)?;
    match command {
        Command::CheckConfig { config_path } => {
            load_config(&config_path)?;
            println!("{{\"schema_version\":\"hl.capture.check.v1\",\"valid\":true}}");
            Ok(())
        }
        Command::Status {
            config_path,
            json: true,
        } => {
            let config = load_config(&config_path)?;
            let status = read_status(config.runtime().status_path())
                .map_err(|error| CliError::Stable(error.reason_code()))?;
            let encoded = serde_json::to_string(&status)
                .map_err(|_| CliError::Stable("capture_cli.serialization"))?;
            println!("{encoded}");
            Ok(())
        }
        Command::Status { json: false, .. } => Err(CliError::Usage),
        Command::Run { config_path } => {
            let config = load_config(&config_path)?;
            run_capture(&config).await
        }
        Command::Maintain { config_path } => {
            let config = load_config(&config_path)?;
            let report = run_configured_maintenance_cycle(&config)
                .map_err(|error| CliError::Stable(error.reason_code()))?;
            let encoded = serde_json::to_string(&MaintainOutput {
                schema_version: "hl.capture.maintain.v1",
                status: report.status().clone(),
            })
            .map_err(|_| CliError::Stable("capture_cli.serialization"))?;
            println!("{encoded}");
            Ok(())
        }
        Command::FixtureReplay {
            config_path,
            block_count,
            block_delay,
        } => {
            let config = load_config(&config_path)?;
            run_fixture(&config, block_count, block_delay).await
        }
        Command::Restore {
            config_path,
            dest,
            backup_root,
            chain,
            source,
            plan_digest,
            backup_receipt,
        } => {
            let backup_receipt =
                backup_receipt.ok_or(CliError::Stable("capture_restore.receipt_required"))?;
            let config = load_config(&config_path)?;
            let chain = ChainId::new(chain)
                .map_err(|_| CliError::Stable("capture_restore.invalid_identity"))?;
            let source = SourceId::new(source)
                .map_err(|_| CliError::Stable("capture_restore.invalid_identity"))?;
            let request = AuthorizedRestoreRequest::try_new(
                dest,
                backup_root,
                chain,
                source,
                plan_digest,
                backup_receipt,
            )
            .map_err(|error| CliError::Stable(error.reason_code()))?;
            let receipt = run_configured_restore(&config, &request)
                .map_err(|error| CliError::Stable(error.reason_code()))?;
            let encoded = serde_json::to_string(&RestoreOutput {
                schema_version: "hl.capture.restore.v1",
                plan_digest: hex::encode(receipt.plan_digest()),
                restored_files: receipt.restored_files(),
                restored_bytes: receipt.restored_bytes(),
            })
            .map_err(|_| CliError::Stable("capture_cli.serialization"))?;
            println!("{encoded}");
            Ok(())
        }
    }
}

#[derive(Debug)]
enum Command {
    CheckConfig {
        config_path: PathBuf,
    },
    Status {
        config_path: PathBuf,
        json: bool,
    },
    Run {
        config_path: PathBuf,
    },
    Maintain {
        config_path: PathBuf,
    },
    FixtureReplay {
        config_path: PathBuf,
        block_count: u64,
        block_delay: Duration,
    },
    Restore {
        config_path: PathBuf,
        dest: PathBuf,
        backup_root: PathBuf,
        chain: String,
        source: String,
        plan_digest: [u8; 32],
        backup_receipt: Option<[u8; 32]>,
    },
}

impl Command {
    fn parse(arguments: Vec<OsString>) -> Result<Self, CliError> {
        let mut arguments = arguments.into_iter();
        let command = arguments.next().ok_or(CliError::Usage)?;
        let command = command.to_str().ok_or(CliError::Usage)?;
        if command == "restore" {
            return Self::parse_restore(arguments);
        }
        let mut config_path = None;
        let mut json = false;
        let mut block_count = None;
        let mut block_delay_millis = None;
        while let Some(argument) = arguments.next() {
            match argument.to_str() {
                Some("--config") if config_path.is_none() => {
                    config_path = Some(PathBuf::from(arguments.next().ok_or(CliError::Usage)?));
                }
                Some("--json") if !json => json = true,
                Some("--blocks") if block_count.is_none() => {
                    block_count = Some(parse_u64(arguments.next().ok_or(CliError::Usage)?)?);
                }
                Some("--block-delay-millis") if block_delay_millis.is_none() => {
                    block_delay_millis = Some(parse_u64(arguments.next().ok_or(CliError::Usage)?)?);
                }
                _ => return Err(CliError::Usage),
            }
        }
        let config_path = config_path.ok_or(CliError::Usage)?;
        match command {
            "check-config" if !json && block_count.is_none() && block_delay_millis.is_none() => {
                Ok(Self::CheckConfig { config_path })
            }
            "status" if block_count.is_none() && block_delay_millis.is_none() => {
                Ok(Self::Status { config_path, json })
            }
            "run" if !json && block_count.is_none() && block_delay_millis.is_none() => {
                Ok(Self::Run { config_path })
            }
            "maintain" if !json && block_count.is_none() && block_delay_millis.is_none() => {
                Ok(Self::Maintain { config_path })
            }
            "fixture-replay" if !json => {
                let block_count = block_count
                    .filter(|count| (1..=10_000_000).contains(count))
                    .ok_or(CliError::Usage)?;
                let block_delay_millis = block_delay_millis.unwrap_or(0);
                if block_delay_millis > 60_000 {
                    return Err(CliError::Usage);
                }
                Ok(Self::FixtureReplay {
                    config_path,
                    block_count,
                    block_delay: Duration::from_millis(block_delay_millis),
                })
            }
            _ => Err(CliError::Usage),
        }
    }

    fn parse_restore(mut arguments: impl Iterator<Item = OsString>) -> Result<Self, CliError> {
        let mut config_path = None;
        let mut dest = None;
        let mut backup_root = None;
        let mut chain = None;
        let mut source = None;
        let mut plan_digest = None;
        let mut backup_receipt = None;
        let mut approved = false;
        while let Some(argument) = arguments.next() {
            match argument.to_str() {
                Some("--config") if config_path.is_none() => {
                    config_path = Some(PathBuf::from(arguments.next().ok_or(CliError::Usage)?));
                }
                Some("--dest") if dest.is_none() => {
                    dest = Some(PathBuf::from(arguments.next().ok_or(CliError::Usage)?));
                }
                Some("--backup-root") if backup_root.is_none() => {
                    backup_root = Some(PathBuf::from(arguments.next().ok_or(CliError::Usage)?));
                }
                Some("--chain") if chain.is_none() => {
                    chain = Some(parse_identity(arguments.next().ok_or(CliError::Usage)?)?);
                }
                Some("--source") if source.is_none() => {
                    source = Some(parse_identity(arguments.next().ok_or(CliError::Usage)?)?);
                }
                Some("--plan-digest") if plan_digest.is_none() => {
                    plan_digest = Some(parse_digest(arguments.next().ok_or(CliError::Usage)?)?);
                }
                Some("--backup-receipt") if backup_receipt.is_none() => {
                    backup_receipt = Some(parse_digest(arguments.next().ok_or(CliError::Usage)?)?);
                }
                Some("--i-approve-restore") if !approved => approved = true,
                _ => return Err(CliError::Usage),
            }
        }
        if !approved {
            return Err(CliError::Usage);
        }
        Ok(Self::Restore {
            config_path: config_path.ok_or(CliError::Usage)?,
            dest: dest.ok_or(CliError::Usage)?,
            backup_root: backup_root.ok_or(CliError::Usage)?,
            chain: chain.ok_or(CliError::Usage)?,
            source: source.ok_or(CliError::Usage)?,
            plan_digest: plan_digest.ok_or(CliError::Usage)?,
            backup_receipt,
        })
    }
}

#[derive(Debug, Serialize)]
struct MaintainOutput {
    schema_version: &'static str,
    #[serde(flatten)]
    status: hl_capture::CaptureMaintenanceStatus,
}

#[derive(Debug, Serialize)]
struct RestoreOutput {
    schema_version: &'static str,
    plan_digest: String,
    restored_files: u64,
    restored_bytes: u64,
}

#[derive(Debug, Serialize)]
struct ErrorOutput {
    schema_version: &'static str,
    reason_code: &'static str,
}

#[derive(Debug)]
enum CliError {
    Usage,
    Stable(&'static str),
}

impl CliError {
    const fn reason_code(&self) -> &'static str {
        match self {
            Self::Usage => "capture_cli.usage",
            Self::Stable(reason_code) => reason_code,
        }
    }
}

fn parse_u64(value: OsString) -> Result<u64, CliError> {
    value
        .to_str()
        .and_then(|value| value.parse().ok())
        .ok_or(CliError::Usage)
}

fn parse_identity(value: OsString) -> Result<String, CliError> {
    let value = value.into_string().map_err(|_| CliError::Usage)?;
    if value.is_empty() {
        return Err(CliError::Usage);
    }
    Ok(value)
}

fn parse_digest(value: OsString) -> Result<[u8; 32], CliError> {
    let value = value
        .to_str()
        .ok_or(CliError::Stable("capture_restore.invalid_digest"))?;
    let decoded =
        hex::decode(value).map_err(|_| CliError::Stable("capture_restore.invalid_digest"))?;
    decoded
        .try_into()
        .map_err(|_| CliError::Stable("capture_restore.invalid_digest"))
}

async fn run_fixture(
    config: &CaptureConfig,
    block_count: u64,
    block_delay: Duration,
) -> Result<(), CliError> {
    let cancellation = CancellationToken::new();
    let connected = connect_capture(config, &cancellation)
        .await
        .map_err(|error| CliError::Stable(error.reason_code()))?;
    let run = connected.run_fixture(cancellation.clone(), block_count, block_delay);
    tokio::pin!(run);
    tokio::select! {
        result = &mut run => {
            result.map_err(|error| CliError::Stable(error.reason_code()))
        }
        result = wait_for_shutdown_signal() => {
            result?;
            cancellation.cancel();
            run.await.map_err(|error| CliError::Stable(error.reason_code()))
        }
    }
}

async fn run_capture(config: &CaptureConfig) -> Result<(), CliError> {
    let cancellation = CancellationToken::new();
    let connected = connect_capture(config, &cancellation)
        .await
        .map_err(|error| CliError::Stable(error.reason_code()))?;
    let run = connected.run(cancellation.clone());
    tokio::pin!(run);
    tokio::select! {
        result = &mut run => {
            result.map_err(|error| CliError::Stable(error.reason_code()))
        }
        result = wait_for_shutdown_signal() => {
            result?;
            cancellation.cancel();
            run.await.map_err(|error| CliError::Stable(error.reason_code()))
        }
    }
}

#[cfg(unix)]
async fn wait_for_shutdown_signal() -> Result<(), CliError> {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .map_err(|_| CliError::Stable("capture_cli.signal"))?;
    tokio::select! {
        result = tokio::signal::ctrl_c() => {
            result.map_err(|_| CliError::Stable("capture_cli.signal"))
        }
        _ = terminate.recv() => Ok(()),
    }
}

#[cfg(not(unix))]
async fn wait_for_shutdown_signal() -> Result<(), CliError> {
    tokio::signal::ctrl_c()
        .await
        .map_err(|_| CliError::Stable("capture_cli.signal"))
}

fn load_config(path: &Path) -> Result<CaptureConfig, CliError> {
    let metadata =
        fs::symlink_metadata(path).map_err(|_| CliError::Stable("capture_cli.config_io"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > MAX_CONFIG_BYTES
    {
        return Err(CliError::Stable("capture_cli.config_io"));
    }
    let source = fs::read_to_string(path).map_err(|_| CliError::Stable("capture_cli.config_io"))?;
    CaptureConfig::from_toml(&source).map_err(|error| CliError::Stable(error.reason_code()))
}
