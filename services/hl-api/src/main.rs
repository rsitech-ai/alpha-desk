#![forbid(unsafe_code)]

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

use hl_api::{ApiConfig, ConfigError, openapi_yaml};
use serde::Serialize;
use tokio::net::TcpListener;
use tokio::sync::oneshot;

const USAGE: &str = "usage: hl-api <check-config|run|print-openapi> [--config <path>]";

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
                schema_version: "hl.api.error.v1",
                reason_code: error.reason_code(),
            };
            match serde_json::to_string(&output) {
                Ok(encoded) => eprintln!("{encoded}"),
                Err(_) => eprintln!(
                    "{{\"schema_version\":\"hl.api.error.v1\",\"reason_code\":\"api_cli.serialization\"}}"
                ),
            }
            ExitCode::FAILURE
        }
    }
}

async fn execute(arguments: Vec<OsString>) -> Result<(), CliError> {
    match Command::parse(arguments)? {
        Command::PrintOpenapi => {
            print!("{}", openapi_yaml());
            Ok(())
        }
        Command::CheckConfig { config_path } => {
            ApiConfig::from_path(&config_path).map_err(CliError::Config)?;
            println!("{{\"schema_version\":\"hl.api.check.v1\",\"valid\":true}}");
            Ok(())
        }
        Command::Run { config_path } => {
            let config = ApiConfig::from_path(&config_path).map_err(CliError::Config)?;
            run(config).await
        }
    }
}

async fn run(config: ApiConfig) -> Result<(), CliError> {
    let listener = TcpListener::bind(config.bind())
        .await
        .map_err(|_| CliError::Bind)?;
    let addr = listener.local_addr().map_err(|_| CliError::Bind)?;
    let listen = ListenOutput {
        schema_version: "hl.api.listen.v1",
        bind: addr.to_string(),
    };
    println!(
        "{}",
        serde_json::to_string(&listen).map_err(|_| CliError::Serialization)?
    );
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        let _ = shutdown_tx.send(());
    });
    let state = hl_api::AppState::from_config(config);
    hl_api::serve(listener, state, shutdown_rx)
        .await
        .map_err(|_| CliError::Bind)
}

enum Command {
    CheckConfig { config_path: PathBuf },
    Run { config_path: PathBuf },
    PrintOpenapi,
}

impl Command {
    fn parse(arguments: Vec<OsString>) -> Result<Self, CliError> {
        let mut arguments = arguments.into_iter();
        let command = arguments.next().ok_or(CliError::Usage)?;
        match command.to_str() {
            Some("print-openapi") => {
                if arguments.next().is_some() {
                    return Err(CliError::Usage);
                }
                Ok(Self::PrintOpenapi)
            }
            Some("check-config") => Ok(Self::CheckConfig {
                config_path: require_config(arguments)?,
            }),
            Some("run") => Ok(Self::Run {
                config_path: require_config(arguments)?,
            }),
            _ => Err(CliError::Usage),
        }
    }
}

fn require_config(mut arguments: impl Iterator<Item = OsString>) -> Result<PathBuf, CliError> {
    match (arguments.next(), arguments.next(), arguments.next()) {
        (Some(flag), Some(path), None) if flag == "--config" => Ok(PathBuf::from(path)),
        _ => Err(CliError::Usage),
    }
}

#[derive(Debug)]
enum CliError {
    Usage,
    Config(ConfigError),
    Bind,
    Serialization,
}

impl CliError {
    fn reason_code(&self) -> &'static str {
        match self {
            Self::Usage => "api_cli.usage",
            Self::Config(error) => error.reason_code(),
            Self::Bind => "api_cli.bind",
            Self::Serialization => "api_cli.serialization",
        }
    }
}

#[derive(Serialize)]
struct ErrorOutput {
    schema_version: &'static str,
    reason_code: &'static str,
}

#[derive(Serialize)]
struct ListenOutput {
    schema_version: &'static str,
    bind: String,
}
