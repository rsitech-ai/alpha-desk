#![forbid(unsafe_code)]

use std::{ffi::OsString, fs, path::Path, process::ExitCode};

use hl_core::{LocalReplayError, inspect_local_replay_block};

const USAGE: &str = "usage: hl-core inspect-block <path>";

fn main() -> ExitCode {
    match run(std::env::args_os().collect()) {
        Ok(report) => {
            println!(
                "INSPECT admitted={} applied={} confirmation={}",
                report.admitted(),
                report.applied(),
                report.confirmation()
            );
            ExitCode::SUCCESS
        }
        Err(CliError::Usage | CliError::NonUtf8Argument(_)) => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
        Err(CliError::Read) => {
            eprintln!("ERROR core.replay_source");
            ExitCode::from(1)
        }
        Err(CliError::Inspect(error)) => {
            eprintln!("ERROR {}", error.reason_code());
            ExitCode::from(1)
        }
    }
}

fn run(arguments: Vec<OsString>) -> Result<hl_core::LocalBlockInspectReport, CliError> {
    let arguments = arguments
        .into_iter()
        .map(|argument| argument.into_string().map_err(CliError::NonUtf8Argument))
        .collect::<Result<Vec<_>, _>>()?;
    match arguments.as_slice() {
        [_, command, path] if command == "inspect-block" => inspect_block(Path::new(path)),
        _ => Err(CliError::Usage),
    }
}

fn inspect_block(path: &Path) -> Result<hl_core::LocalBlockInspectReport, CliError> {
    let json = fs::read_to_string(path).map_err(|_| CliError::Read)?;
    Ok(inspect_local_replay_block(&json)?)
}

#[derive(Debug, thiserror::Error)]
enum CliError {
    #[error("{USAGE}")]
    Usage,
    #[error("argument is not valid UTF-8")]
    NonUtf8Argument(OsString),
    #[error("local replay block file could not be read")]
    Read,
    #[error(transparent)]
    Inspect(#[from] LocalReplayError),
}
