use std::{
    env,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::ExitCode,
};

use stage_gate::{
    approvals::GateStatus,
    config::validate_config_document,
    gate::{BuilderProducer, run_gate_with_producer},
    output::OutputRoot,
    reports::valid_builder_id,
};

fn main() -> ExitCode {
    let raw_arguments = env::args_os().skip(1).collect::<Vec<_>>();
    if raw_arguments.first().and_then(|value| value.to_str()) == Some("validate-config") {
        return validate_config(&raw_arguments);
    }
    let parsed = parse_run_args(&raw_arguments);
    if let Err(error) =
        invalidate_targeted_stage_zero_outputs(parsed.targeted_stage_zero_repository.as_deref())
    {
        eprintln!("stage-gate:error:{error}");
        return ExitCode::from(1);
    }
    let args = match parsed.arguments {
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

struct ParsedRunArguments {
    targeted_stage_zero_repository: Option<PathBuf>,
    arguments: Result<Arguments, String>,
}

fn parse_run_args(raw_arguments: &[OsString]) -> ParsedRunArguments {
    let mut arguments = raw_arguments.iter().cloned();
    if arguments.next().as_deref() != Some("run".as_ref()) {
        return ParsedRunArguments {
            targeted_stage_zero_repository: None,
            arguments: Err(usage()),
        };
    }
    let Some(config) = arguments.next() else {
        return ParsedRunArguments {
            targeted_stage_zero_repository: None,
            arguments: Err(usage()),
        };
    };
    let mut repository = match env::current_dir() {
        Ok(repository) => repository,
        Err(error) => {
            return ParsedRunArguments {
                targeted_stage_zero_repository: None,
                arguments: Err(error.to_string()),
            };
        }
    };
    let mut output = None;
    let mut builder_id = None;
    let mut builder_role = None;
    let mut builder_fingerprint = None;
    let parse_error = loop {
        let Some(flag) = arguments.next() else {
            break None;
        };
        match flag.to_str() {
            Some("--repository") => {
                let Some(value) = arguments.next() else {
                    break Some(usage());
                };
                repository = PathBuf::from(value);
            }
            Some("--output") => {
                output = arguments.next().map(PathBuf::from);
                if output.is_none() {
                    break Some(usage());
                }
            }
            Some("--builder-id") => {
                builder_id = arguments.next().and_then(|value| value.into_string().ok());
                if builder_id.is_none() {
                    break Some(usage());
                }
            }
            Some("--builder-role") => {
                builder_role = arguments.next().and_then(|value| value.into_string().ok());
                if builder_role.is_none() {
                    break Some(usage());
                }
            }
            Some("--builder-fingerprint") => {
                builder_fingerprint = arguments.next().and_then(|value| value.into_string().ok());
                if builder_fingerprint.is_none() {
                    break Some(usage());
                }
            }
            _ => break Some(usage()),
        }
    };
    let targeted_stage_zero_repository =
        targeted_stage_zero_repository(Path::new(&config), &repository);
    if let Some(error) = parse_error {
        return ParsedRunArguments {
            targeted_stage_zero_repository,
            arguments: Err(error),
        };
    }
    let producer = match (builder_id, builder_role, builder_fingerprint) {
        (Some(builder_id), None, None) if valid_builder_id(&builder_id) => {
            BuilderProducer::local(builder_id)
        }
        (None, Some(role), Some(fingerprint)) => {
            match BuilderProducer::builder_b(&role, &fingerprint) {
                Ok(producer) => producer,
                Err(error) => {
                    return ParsedRunArguments {
                        targeted_stage_zero_repository,
                        arguments: Err(error.to_string()),
                    };
                }
            }
        }
        _ => {
            return ParsedRunArguments {
                targeted_stage_zero_repository,
                arguments: Err(usage()),
            };
        }
    };
    let Some(output) = output else {
        return ParsedRunArguments {
            targeted_stage_zero_repository,
            arguments: Err(usage()),
        };
    };
    ParsedRunArguments {
        targeted_stage_zero_repository,
        arguments: Ok(Arguments {
            repository,
            config: Path::new(&config).to_path_buf(),
            output,
            producer,
        }),
    }
}

fn validate_config(raw_arguments: &[OsString]) -> ExitCode {
    let [command, config_path, schema_flag, schema_path] = raw_arguments else {
        eprintln!("stage-gate:error:{}", validation_usage());
        return ExitCode::from(1);
    };
    if command != "validate-config" || schema_flag != "--schema" {
        eprintln!("stage-gate:error:{}", validation_usage());
        return ExitCode::from(1);
    }
    let config_path = Path::new(config_path);
    let schema_path = Path::new(schema_path);
    let result = fs::read_to_string(config_path)
        .map_err(|error| format!("configuration could not be read: {error}"))
        .and_then(|source| {
            fs::read_to_string(schema_path)
                .map_err(|error| format!("schema could not be read: {error}"))
                .and_then(|schema| {
                    validate_config_document(&source, &schema)
                        .map(|_| ())
                        .map_err(|error| error.to_string())
                })
        });
    match result {
        Ok(()) => {
            println!("stage-gate:config-valid:structure+semantics");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("stage-gate:error:{error}");
            ExitCode::from(1)
        }
    }
}

fn invalidate_targeted_stage_zero_outputs(repository: Option<&Path>) -> Result<(), String> {
    let Some(repository) = repository else {
        return Ok(());
    };
    let root = OutputRoot::open(repository, Path::new("target/stage-gates"))
        .map_err(|error| format!("could not open fixed Stage 0 output scope: {error}"))?;
    for output in [
        Path::new("stage-0.json"),
        Path::new("stage-0.builder.json"),
        Path::new("stage-0-builder-report.json"),
    ] {
        root.remove_if_exists(output)
            .map_err(|error| format!("could not invalidate fixed Stage 0 output: {error}"))?;
    }
    Ok(())
}

fn targeted_stage_zero_repository(config: &Path, repository: &Path) -> Option<PathBuf> {
    let repository = repository.canonicalize().ok()?;
    if !repository.join(".git").exists() {
        return None;
    }
    let fixed_config = Path::new("config/stage-gates/stage-0.toml");
    let is_fixed_config = if config.is_absolute() {
        config.canonicalize().ok()? == repository.join(fixed_config).canonicalize().ok()?
    } else {
        config == fixed_config
    };
    is_fixed_config.then_some(repository)
}

fn usage() -> String {
    concat!(
        "usage: stage-gate run <config> [--repository <path>] --output <path> ",
        "(--builder-id <validated-id> | ",
        "--builder-role builder-b --builder-fingerprint <40-lower-hex>)",
    )
    .to_owned()
}

fn validation_usage() -> String {
    "usage: stage-gate validate-config <config> --schema <schema>".to_owned()
}
