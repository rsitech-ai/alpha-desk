#![forbid(unsafe_code)]

use std::{ffi::OsString, path::Path, process::ExitCode};

use archive_inspect::{
    InspectError, V3InspectSummary, count, health_v3, scrub_v3, stats_v3, verify,
};

const USAGE: &str = "usage: archive-inspect <verify|count|scrub|stats|health> <archive-root>";

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
        Some("scrub") => {
            let summary = scrub_v3(root)?;
            print_v3_summary("scrub", &summary);
        }
        Some("stats") => {
            let summary = stats_v3(root)?;
            print_v3_summary("stats", &summary);
        }
        Some("health") => {
            let summary = health_v3(root)?;
            print_v3_summary("health", &summary);
        }
        _ => return Err(CliError::Usage),
    }
    Ok(())
}

fn print_v3_summary(command: &str, summary: &V3InspectSummary) {
    let mut logical_manifests = 0_u64;
    let mut packed_ranges = 0_u64;
    let mut logical_rows = 0_u64;
    for source in summary.sources() {
        logical_manifests =
            logical_manifests.saturating_add(source.scrub().logical_manifest_count());
        packed_ranges = packed_ranges.saturating_add(source.scrub().packed_range_count());
        logical_rows = logical_rows.saturating_add(source.statistics().logical_row_count());
    }
    println!(
        "PASS command={command} sources={} logical_manifests={logical_manifests} packed_ranges={packed_ranges} logical_rows={logical_rows}",
        summary.sources().len()
    );
}

#[derive(Debug, thiserror::Error)]
enum CliError {
    #[error("{USAGE}")]
    Usage,
    #[error(transparent)]
    Inspect(#[from] InspectError),
}
