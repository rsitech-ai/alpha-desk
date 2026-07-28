#![forbid(unsafe_code)]

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

use hl_capture::spool::{SpoolError, inspect_spool};

const USAGE: &str = "usage: spool-inspect verify <directory-or-segment>";

#[derive(Debug, thiserror::Error)]
enum CliError {
    #[error("{USAGE}")]
    Usage,
    #[error(transparent)]
    Spool(#[from] SpoolError),
}

fn main() -> ExitCode {
    match run(std::env::args_os().collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(CliError::Usage) => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
        Err(CliError::Spool(error)) => {
            eprintln!("ERROR {}", error.reason_code());
            ExitCode::from(1)
        }
    }
}

fn run(arguments: Vec<OsString>) -> Result<(), CliError> {
    let [_, command, path] = arguments.as_slice() else {
        return Err(CliError::Usage);
    };
    if command.to_str() != Some("verify") {
        return Err(CliError::Usage);
    }
    let report = inspect_spool(PathBuf::from(path))?;
    let chain_tip = report
        .chain_tip()
        .map(hex_string)
        .unwrap_or_else(|| "none".to_owned());
    println!(
        "PASS closed_segments={} open_segments={} records={} chain_tip={chain_tip}",
        report.closed_segments(),
        report.open_segments(),
        report.records(),
    );
    Ok(())
}

fn hex_string(value: [u8; 32]) -> String {
    value.iter().map(|byte| format!("{byte:02x}")).collect()
}
