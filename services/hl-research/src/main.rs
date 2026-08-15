#![forbid(unsafe_code)]

use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use hl_research::{
    ResearchError, ResearchStatus, run_evaluate_folds_bytes, run_holdout_isolation_bytes,
    run_promote_bytes, run_shadow_capture_bytes, run_synthetic_bytes, run_walk_forward_bytes,
};
use serde::Serialize;

const USAGE: &str = "usage: hl-research <status|run-synthetic|walk-forward|holdout-isolation|shadow-capture|evaluate-folds|promote> [--fixture <path>] [--bundle <dir>] [--approved-key <hex>] [--output <path>]";

#[tokio::main]
async fn main() -> ExitCode {
    match execute(std::env::args_os().skip(1).collect()) {
        Ok(output) => {
            print!("{output}");
            ExitCode::SUCCESS
        }
        Err(ResearchError::Usage) => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
        Err(error) => {
            let output = ErrorOutput {
                schema_version: "hl.research.error.v1",
                reason_code: error.reason_code(),
            };
            match serde_json::to_string(&output) {
                Ok(encoded) => eprintln!("{encoded}"),
                Err(_) => eprintln!(
                    "{{\"schema_version\":\"hl.research.error.v1\",\"reason_code\":\"hl_research.serialization\"}}"
                ),
            }
            ExitCode::FAILURE
        }
    }
}

fn execute(arguments: Vec<OsString>) -> Result<String, ResearchError> {
    let command = Command::parse(arguments)?;
    match command {
        Command::Status => write_json(ResearchStatus::current()),
        Command::RunSynthetic {
            fixture,
            bundle,
            approved_key,
            output,
        } => {
            let bytes = fs::read(&fixture).map_err(|_| ResearchError::InvalidFixture)?;
            let key = match approved_key {
                Some(hex_key) => {
                    let decoded =
                        hex::decode(hex_key).map_err(|_| ResearchError::InvalidFixture)?;
                    let bytes: [u8; 32] = decoded
                        .try_into()
                        .map_err(|_| ResearchError::InvalidFixture)?;
                    Some(bytes)
                }
                None => None,
            };
            let report = run_synthetic_bytes(&bytes, bundle.as_deref(), key)?;
            let encoded =
                serde_json::to_string(&report).map_err(|_| ResearchError::InvalidFixture)?;
            if let Some(path) = output {
                fs::write(path, encoded.as_bytes()).map_err(|_| ResearchError::InvalidFixture)?;
            }
            Ok(encoded + "\n")
        }
        Command::WalkForward { fixture } => {
            write_json(run_walk_forward_bytes(&read_fixture(&fixture)?)?)
        }
        Command::HoldoutIsolation { fixture } => {
            write_json(run_holdout_isolation_bytes(&read_fixture(&fixture)?)?)
        }
        Command::ShadowCapture { fixture } => {
            write_json(run_shadow_capture_bytes(&read_fixture(&fixture)?)?)
        }
        Command::EvaluateFolds { fixture } => {
            write_json(run_evaluate_folds_bytes(&read_fixture(&fixture)?)?)
        }
        Command::Promote { fixture } => write_json(run_promote_bytes(&read_fixture(&fixture)?)?),
    }
}

fn read_fixture(path: &std::path::Path) -> Result<Vec<u8>, ResearchError> {
    fs::read(path).map_err(|_| ResearchError::InvalidFixture)
}

fn write_json<T: Serialize>(value: T) -> Result<String, ResearchError> {
    serde_json::to_string(&value)
        .map_err(|_| ResearchError::InvalidFixture)
        .map(|encoded| encoded + "\n")
}

#[derive(Debug)]
enum Command {
    Status,
    RunSynthetic {
        fixture: PathBuf,
        bundle: Option<PathBuf>,
        approved_key: Option<String>,
        output: Option<PathBuf>,
    },
    WalkForward {
        fixture: PathBuf,
    },
    HoldoutIsolation {
        fixture: PathBuf,
    },
    ShadowCapture {
        fixture: PathBuf,
    },
    EvaluateFolds {
        fixture: PathBuf,
    },
    Promote {
        fixture: PathBuf,
    },
}

impl Command {
    fn parse(arguments: Vec<OsString>) -> Result<Self, ResearchError> {
        let mut args = arguments.into_iter();
        let verb = args
            .next()
            .ok_or(ResearchError::Usage)?
            .into_string()
            .map_err(|_| ResearchError::Usage)?;
        match verb.as_str() {
            "status" => {
                if args.next().is_some() {
                    return Err(ResearchError::Usage);
                }
                Ok(Self::Status)
            }
            "run-synthetic" => {
                let mut fixture = None;
                let mut bundle = None;
                let mut approved_key = None;
                let mut output = None;
                while let Some(flag) = args.next() {
                    let flag = flag.into_string().map_err(|_| ResearchError::Usage)?;
                    let value = args
                        .next()
                        .ok_or(ResearchError::Usage)?
                        .into_string()
                        .map_err(|_| ResearchError::Usage)?;
                    match flag.as_str() {
                        "--fixture" => fixture = Some(PathBuf::from(value)),
                        "--bundle" => bundle = Some(PathBuf::from(value)),
                        "--approved-key" => approved_key = Some(value),
                        "--output" => output = Some(PathBuf::from(value)),
                        _ => return Err(ResearchError::Usage),
                    }
                }
                Ok(Self::RunSynthetic {
                    fixture: fixture.ok_or(ResearchError::Usage)?,
                    bundle,
                    approved_key,
                    output,
                })
            }
            "walk-forward" | "holdout-isolation" | "shadow-capture" | "evaluate-folds"
            | "promote" => {
                let mut fixture = None;
                while let Some(flag) = args.next() {
                    let flag = flag.into_string().map_err(|_| ResearchError::Usage)?;
                    let value = args
                        .next()
                        .ok_or(ResearchError::Usage)?
                        .into_string()
                        .map_err(|_| ResearchError::Usage)?;
                    match flag.as_str() {
                        "--fixture" => fixture = Some(PathBuf::from(value)),
                        _ => return Err(ResearchError::Usage),
                    }
                }
                let fixture = fixture.ok_or(ResearchError::Usage)?;
                Ok(match verb.as_str() {
                    "walk-forward" => Self::WalkForward { fixture },
                    "holdout-isolation" => Self::HoldoutIsolation { fixture },
                    "shadow-capture" => Self::ShadowCapture { fixture },
                    "evaluate-folds" => Self::EvaluateFolds { fixture },
                    "promote" => Self::Promote { fixture },
                    _ => return Err(ResearchError::Usage),
                })
            }
            _ => Err(ResearchError::Usage),
        }
    }
}

#[derive(Serialize)]
struct ErrorOutput {
    schema_version: &'static str,
    reason_code: &'static str,
}
