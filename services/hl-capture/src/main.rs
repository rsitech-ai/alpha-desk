#![forbid(unsafe_code)]

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use hl_capture::{CaptureConfig, connect_capture, read_status};
use serde::Serialize;
use tokio_util::sync::CancellationToken;

const MAX_CONFIG_BYTES: u64 = 1024 * 1024;
const USAGE: &str = "usage: hl-capture <check-config|status|run> --config <path> [--json]\n       hl-capture fixture-replay --config <path> --blocks <count> [--block-delay-millis <ms>]";

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
            load_config(&config_path)?;
            Err(CliError::Stable(
                "capture_runtime.committed_source_mapper_unavailable",
            ))
        }
        Command::FixtureReplay {
            config_path,
            block_count,
            block_delay,
        } => {
            let config = load_config(&config_path)?;
            run_fixture(&config, block_count, block_delay).await
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
    FixtureReplay {
        config_path: PathBuf,
        block_count: u64,
        block_delay: Duration,
    },
}

impl Command {
    fn parse(arguments: Vec<OsString>) -> Result<Self, CliError> {
        let mut arguments = arguments.into_iter();
        let command = arguments.next().ok_or(CliError::Usage)?;
        let command = command.to_str().ok_or(CliError::Usage)?;
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
