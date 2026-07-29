#![forbid(unsafe_code)]

use std::{ffi::OsString, path::Path, process::ExitCode};

use archive_inspect::{InspectError, count, verify};

const USAGE: &str = "usage: archive-inspect <verify|count> <archive-root>";

#[tokio::main]
async fn main() -> ExitCode {
    match run(std::env::args_os().collect()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(CliError::Usage) => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
        Err(CliError::Inspect(error)) => {
            eprintln!("ERROR {}", error.reason_code());
            ExitCode::from(1)
        }
    }
}

async fn run(arguments: Vec<OsString>) -> Result<(), CliError> {
    let [_, command, root] = arguments.as_slice() else {
        return Err(CliError::Usage);
    };
    let root = Path::new(root);
    match command.to_str() {
        Some("verify") => {
            let summary = verify(root)?;
            let inspection = summary.inspection();
            println!(
                "PASS chains={} raw_sources={} blocks={} canonical_events={} raw_observations={} objects={}",
                inspection.canonical_chains(),
                inspection.raw_sources(),
                inspection.canonical_blocks(),
                inspection.canonical_events(),
                inspection.raw_observations(),
                inspection.objects().len()
            );
        }
        Some("count") => {
            let summary = count(root).await?;
            println!(
                "PASS canonical_events={} canonical_objects={}",
                summary.canonical_events(),
                summary.canonical_objects()
            );
        }
        _ => return Err(CliError::Usage),
    }
    Ok(())
}

#[derive(Debug, thiserror::Error)]
enum CliError {
    #[error("{USAGE}")]
    Usage,
    #[error(transparent)]
    Inspect(#[from] InspectError),
}
