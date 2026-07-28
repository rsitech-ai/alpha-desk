use std::{
    env,
    path::{Path, PathBuf},
    process::ExitCode,
};

use stage_gate::{
    approvals::GateStatus,
    gate::{BuilderProducer, run_gate_with_producer},
};

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(args) => args,
        Err(error) => {
            eprintln!("stage-gate:error:{error}");
            return ExitCode::from(1);
        }
    };
    match run_gate_with_producer(&args.repository, &args.config, &args.output, args.producer) {
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
    producer: BuilderProducer,
}

fn parse_args() -> Result<Arguments, String> {
    let mut arguments = env::args_os().skip(1);
    if arguments.next().as_deref() != Some("run".as_ref()) {
        return Err(usage());
    }
    let config = arguments.next().ok_or_else(usage)?;
    let mut repository = env::current_dir().map_err(|error| error.to_string())?;
    let mut output = None;
    let mut builder_role = None;
    let mut builder_fingerprint = None;
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
            Some("--builder-role") => {
                builder_role = arguments.next().and_then(|value| value.into_string().ok());
                if builder_role.is_none() {
                    return Err(usage());
                }
            }
            Some("--builder-fingerprint") => {
                builder_fingerprint = arguments.next().and_then(|value| value.into_string().ok());
                if builder_fingerprint.is_none() {
                    return Err(usage());
                }
            }
            _ => return Err(usage()),
        }
    }
    let producer = match (builder_role, builder_fingerprint) {
        (None, None) => BuilderProducer::local(
            env::var("STAGE_GATE_BUILDER_ID").unwrap_or_else(|_| "local-unidentified".to_owned()),
        ),
        (Some(role), Some(fingerprint)) => {
            BuilderProducer::builder_b(&role, &fingerprint).map_err(|error| error.to_string())?
        }
        _ => return Err(usage()),
    };
    Ok(Arguments {
        repository,
        config: Path::new(&config).to_path_buf(),
        output: output.ok_or_else(usage)?,
        producer,
    })
}

fn usage() -> String {
    concat!(
        "usage: stage-gate run <config> [--repository <path>] --output <path> ",
        "[--builder-role builder-b --builder-fingerprint <40-lower-hex>]",
    )
    .to_owned()
}
