#![forbid(unsafe_code)]

use std::{ffi::OsString, path::PathBuf, process::ExitCode};

use state_replay::{FixtureRunConfig, FixtureRunError, run_fixture_e2e};

const USAGE: &str =
    "usage: state-replay fixture-e2e --output PATH --blocks N --checkpoint-after N --iterations N";

fn main() -> ExitCode {
    match run(std::env::args_os().collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(CliError::Usage) => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
        Err(CliError::Run(error)) => {
            eprintln!("ERROR {}", error.reason_code());
            ExitCode::from(1)
        }
    }
}

fn run(arguments: Vec<OsString>) -> Result<(), CliError> {
    let mut arguments = arguments.into_iter();
    let _program = arguments.next().ok_or(CliError::Usage)?;
    if arguments
        .next()
        .and_then(|value| value.into_string().ok())
        .as_deref()
        != Some("fixture-e2e")
    {
        return Err(CliError::Usage);
    }
    let mut output = None;
    let mut blocks = None;
    let mut checkpoint_after = None;
    let mut iterations = None;
    while let Some(flag) = arguments.next() {
        let flag = flag.into_string().map_err(|_| CliError::Usage)?;
        let value = arguments.next().ok_or(CliError::Usage)?;
        match flag.as_str() {
            "--output" if output.is_none() => output = Some(PathBuf::from(value)),
            "--blocks" if blocks.is_none() => blocks = Some(parse_u64(value)?),
            "--checkpoint-after" if checkpoint_after.is_none() => {
                checkpoint_after = Some(parse_u64(value)?);
            }
            "--iterations" if iterations.is_none() => iterations = Some(parse_u64(value)?),
            _ => return Err(CliError::Usage),
        }
    }
    let config = FixtureRunConfig::new(
        output.ok_or(CliError::Usage)?,
        blocks.ok_or(CliError::Usage)?,
        checkpoint_after.ok_or(CliError::Usage)?,
        iterations.ok_or(CliError::Usage)?,
    );
    let _evidence = run_fixture_e2e(&config)?;
    println!(
        "PASS evidence_class=synthetic_fixture stage_2_qualified=false live_source_qualified=false"
    );
    Ok(())
}

fn parse_u64(value: OsString) -> Result<u64, CliError> {
    value
        .into_string()
        .map_err(|_| CliError::Usage)?
        .parse()
        .map_err(|_| CliError::Usage)
}

#[derive(Debug, thiserror::Error)]
enum CliError {
    #[error("{USAGE}")]
    Usage,
    #[error(transparent)]
    Run(#[from] FixtureRunError),
}
