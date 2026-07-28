use std::{
    env,
    path::{Path, PathBuf},
    process::ExitCode,
};

use stage_gate::{approvals::GateStatus, gate::run_gate};

fn main() -> ExitCode {
    match parse_args().and_then(|args| {
        run_gate(&args.repository, &args.config, &args.output).map_err(|error| error.to_string())
    }) {
        Ok(report) => {
            eprintln!(
                "stage-gate:{}:{:?}:{}",
                report.stage_id,
                report.overall_result,
                report.reason_codes.len()
            );
            match report.overall_result {
                GateStatus::Pass => ExitCode::SUCCESS,
                GateStatus::Blocked | GateStatus::NotRun => ExitCode::from(2),
                GateStatus::Fail => ExitCode::from(1),
            }
        }
        Err(error) => {
            eprintln!("stage-gate:error:{error}");
            ExitCode::from(1)
        }
    }
}

struct Arguments {
    repository: PathBuf,
    config: PathBuf,
    output: PathBuf,
}

fn parse_args() -> Result<Arguments, String> {
    let mut arguments = env::args_os().skip(1);
    if arguments.next().as_deref() != Some("run".as_ref()) {
        return Err(usage());
    }
    let config = arguments.next().ok_or_else(usage)?;
    let mut repository = env::current_dir().map_err(|error| error.to_string())?;
    let mut output = None;
    while let Some(flag) = arguments.next() {
        match flag.to_str() {
            Some("--repository") => {
                repository = arguments.next().map(PathBuf::from).ok_or_else(usage)?;
            }
            Some("--output") => {
                output = arguments.next().map(PathBuf::from);
                if output.is_none() {
                    return Err(usage());
                }
            }
            _ => return Err(usage()),
        }
    }
    Ok(Arguments {
        repository,
        config: Path::new(&config).to_path_buf(),
        output: output.ok_or_else(usage)?,
    })
}

fn usage() -> String {
    "usage: stage-gate run <config> [--repository <path>] --output <path>".to_owned()
}
