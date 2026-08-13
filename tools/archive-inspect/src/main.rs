#![forbid(unsafe_code)]

use std::{ffi::OsString, path::Path, process::ExitCode};

use archive_inspect::{InspectError, count, verify};
use canonical_archive::{ArchiveConfig, RawV3Archive};
use domain_types::{ChainId, SourceId};
use storage_ports::{ArchiveError, RawArchiveCapacityBudgets, RawArchiveWorkloadEnvelope};

const USAGE: &str = "usage: archive-inspect <verify|count> <archive-root>
       archive-inspect <import-plan|import-publish|import-approve> <archive-root> <chain> <source>";

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
    match arguments.as_slice() {
        [_, command, root] => {
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
        }
        [_, command, root, chain, source] => {
            let command = command.to_str().ok_or(CliError::Usage)?;
            let chain = chain.to_str().ok_or(CliError::Usage)?;
            let source = source.to_str().ok_or(CliError::Usage)?;
            let chain = ChainId::new(chain).map_err(|_| CliError::Usage)?;
            let source = SourceId::new(source).map_err(|_| CliError::Usage)?;
            let archive = open_v3(Path::new(root))?;
            match command {
                "import-plan" => {
                    let plan = archive.plan_v2_import(&chain, &source)?;
                    println!(
                        "PASS plan_sha256={} catalog_sha256={} batches={} sequences={}-{}",
                        hex_digest(plan.sha256()),
                        hex_digest(plan.v2_catalog_sha256()),
                        plan.batches().len(),
                        plan.first_local_sequence(),
                        plan.last_local_sequence()
                    );
                }
                "import-publish" => {
                    let plan = archive.plan_v2_import(&chain, &source)?;
                    let report = archive.publish_v2_import(&chain, &source, &plan)?;
                    println!(
                        "PASS plan_sha256={} import_root={} packs={} parity={}",
                        hex_digest(report.plan_sha256()),
                        hex_digest(report.v3_root_sha256()),
                        report.pack_count(),
                        hex_digest(report.parity_digest())
                    );
                }
                "import-approve" => {
                    let plan = archive.plan_v2_import(&chain, &source)?;
                    let approval = archive.approve_v2_import(&chain, &source, &plan)?;
                    println!(
                        "PASS plan_sha256={} root={} checkpoint={} parity={}",
                        hex_digest(approval.plan_sha256()),
                        hex_digest(approval.v3_root_sha256()),
                        hex_digest(approval.checkpoint_sha256()),
                        hex_digest(approval.parity_digest())
                    );
                }
                _ => return Err(CliError::Usage),
            }
        }
        _ => return Err(CliError::Usage),
    }
    Ok(())
}

fn open_v3(root: &Path) -> Result<RawV3Archive, InspectError> {
    let config = ArchiveConfig::production("archive-inspect-v1")?;
    let workload = RawArchiveWorkloadEnvelope::try_new(
        100,
        1,
        1_000,
        3_600,
        1_024,
        1_000,
        64 * 1024 * 1024,
        64,
    )
    .map_err(ArchiveError::from)?;
    let budgets = RawArchiveCapacityBudgets::try_new(u64::MAX, u64::MAX, u64::MAX, u64::MAX, true)
        .map_err(ArchiveError::from)?;
    RawV3Archive::open(root, config, workload, budgets).map_err(InspectError::from)
}

fn hex_digest(bytes: [u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[derive(Debug, thiserror::Error)]
enum CliError {
    #[error("{USAGE}")]
    Usage,
    #[error(transparent)]
    Inspect(#[from] InspectError),
}

impl From<ArchiveError> for CliError {
    fn from(error: ArchiveError) -> Self {
        Self::Inspect(error.into())
    }
}
