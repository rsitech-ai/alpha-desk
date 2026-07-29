#![forbid(unsafe_code)]

use std::ffi::OsString;
use std::path::Path;
use std::process::ExitCode;

use canonical_inspect::{InspectError, canonicalize};

const USAGE: &str =
    "usage: canonical-inspect canonicalize --root <directory> --manifest <path> --output <path>";

fn main() -> ExitCode {
    match run(std::env::args_os().collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(CliError::Usage | CliError::NonUtf8Argument(_)) => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
        Err(CliError::Inspect(error)) => {
            eprintln!("ERROR {}", error.reason_code());
            ExitCode::from(1)
        }
    }
}

fn run(arguments: Vec<OsString>) -> Result<(), CliError> {
    let arguments = arguments
        .into_iter()
        .map(|argument| argument.into_string().map_err(CliError::NonUtf8Argument))
        .collect::<Result<Vec<_>, _>>()?;
    let [
        _,
        command,
        root_flag,
        root,
        manifest_flag,
        manifest,
        output_flag,
        output,
    ] = arguments.as_slice()
    else {
        return Err(CliError::Usage);
    };
    if command != "canonicalize"
        || root_flag != "--root"
        || manifest_flag != "--manifest"
        || output_flag != "--output"
    {
        return Err(CliError::Usage);
    }
    let summary = canonicalize(Path::new(root), Path::new(manifest), Path::new(output))?;
    println!(
        "canonical-inspect:ok events={} output_sha256={}",
        summary.event_count(),
        summary.output_sha256()
    );
    Ok(())
}

#[derive(Debug, thiserror::Error)]
enum CliError {
    #[error("{USAGE}")]
    Usage,
    #[error("argument is not valid UTF-8: {0:?}")]
    NonUtf8Argument(OsString),
    #[error(transparent)]
    Inspect(#[from] InspectError),
}
