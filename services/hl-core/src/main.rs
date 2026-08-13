#![forbid(unsafe_code)]

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use hl_core::{CoreConfig, CoreRuntime};
use serde::Serialize;
use tokio_util::sync::CancellationToken;

const MAX_CONFIG_BYTES: u64 = 1024 * 1024;
const USAGE: &str = "usage: hl-core <check-config|run> --config <path>";

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
                schema_version: "hl.core.error.v1",
                reason_code: error.reason_code(),
            };
            match serde_json::to_string(&output) {
                Ok(encoded) => eprintln!("{encoded}"),
                Err(_) => eprintln!(
                    "{{\"schema_version\":\"hl.core.error.v1\",\"reason_code\":\"core_cli.serialization\"}}"
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
            println!("{{\"schema_version\":\"hl.core.check.v1\",\"valid\":true}}");
            Ok(())
        }
        Command::Run { config_path } => {
            let config = load_config(&config_path)?;
            run_core(config).await
        }
    }
}

#[derive(Debug)]
enum Command {
    CheckConfig { config_path: PathBuf },
    Run { config_path: PathBuf },
}

impl Command {
    fn parse(arguments: Vec<OsString>) -> Result<Self, CliError> {
        let mut arguments = arguments.into_iter();
        let command = arguments.next().ok_or(CliError::Usage)?;
        let command = command.to_str().ok_or(CliError::Usage)?;
        let mut config_path = None;
        while let Some(argument) = arguments.next() {
            match argument.to_str() {
                Some("--config") if config_path.is_none() => {
                    config_path = Some(PathBuf::from(arguments.next().ok_or(CliError::Usage)?));
                }
                _ => return Err(CliError::Usage),
            }
        }
        let config_path = config_path.ok_or(CliError::Usage)?;
        match command {
            "check-config" => Ok(Self::CheckConfig { config_path }),
            "run" => Ok(Self::Run { config_path }),
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
            Self::Usage => "core_cli.usage",
            Self::Stable(reason_code) => reason_code,
        }
    }
}

async fn run_core(config: CoreConfig) -> Result<(), CliError> {
    let cancellation = CancellationToken::new();
    let grace = config.shutdown_grace();
    let runtime = CoreRuntime::from_config(config);
    let run = runtime.run_jetstream(cancellation.clone());
    tokio::pin!(run);
    tokio::select! {
        result = &mut run => {
            result.map(|_| ()).map_err(|error| CliError::Stable(error.reason_code()))
        }
        result = wait_for_shutdown_signal() => {
            result?;
            cancellation.cancel();
            match tokio::time::timeout(grace, run).await {
                Ok(result) => result.map(|_| ()).map_err(|error| CliError::Stable(error.reason_code())),
                Err(_) => Err(CliError::Stable("core_cli.shutdown_timeout")),
            }
        }
    }
}

#[cfg(unix)]
async fn wait_for_shutdown_signal() -> Result<(), CliError> {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .map_err(|_| CliError::Stable("core_cli.signal"))?;
    tokio::select! {
        result = tokio::signal::ctrl_c() => {
            result.map_err(|_| CliError::Stable("core_cli.signal"))
        }
        _ = terminate.recv() => Ok(()),
    }
}

#[cfg(not(unix))]
async fn wait_for_shutdown_signal() -> Result<(), CliError> {
    tokio::signal::ctrl_c()
        .await
        .map_err(|_| CliError::Stable("core_cli.signal"))
}

fn load_config(path: &Path) -> Result<CoreConfig, CliError> {
    let metadata =
        fs::symlink_metadata(path).map_err(|_| CliError::Stable("core_cli.config_io"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > MAX_CONFIG_BYTES
    {
        return Err(CliError::Stable("core_cli.config_io"));
    }
    let source = fs::read_to_string(path).map_err(|_| CliError::Stable("core_cli.config_io"))?;
    CoreConfig::from_toml(&source).map_err(|error| CliError::Stable(error.reason_code()))
}
