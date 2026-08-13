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
                "PASS chains={} raw_sources={} blocks={} canonical_events={} raw_observations={} objects={} v3_sources={} v3_logical_rows={} v3_logical_manifests={}",
                inspection.canonical_chains(),
                inspection.raw_sources(),
                inspection.canonical_blocks(),
                inspection.canonical_events(),
                inspection.raw_observations(),
                inspection.objects().len(),
                summary.v3().map_or(0, |v3| v3.sources().len()),
                summary.v3().map_or(0, V3InspectSummary::logical_row_count),
                summary
                    .v3()
                    .map_or(0, V3InspectSummary::logical_manifest_count),
            );
        }
        Some("count") => {
            let summary = count(root).await?;
            println!(
                "PASS canonical_events={} canonical_objects={} v3_sources={} v3_logical_rows={} v3_logical_manifests={}",
                summary.canonical_events(),
                summary.canonical_objects(),
                summary.v3_sources(),
                summary.v3_logical_rows(),
                summary.v3_logical_manifests()
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
    println!(
        "PASS command={command} sources={} logical_manifests={} packed_ranges={} logical_rows={}",
        summary.sources().len(),
        summary.logical_manifest_count(),
        summary.sources().iter().fold(0_u64, |total, source| {
            total.saturating_add(source.scrub().packed_range_count())
        }),
        summary.logical_row_count(),
    );
}

#[derive(Debug, thiserror::Error)]
enum CliError {
    #[error("{USAGE}")]
    Usage,
    #[error(transparent)]
    Inspect(#[from] InspectError),
}
