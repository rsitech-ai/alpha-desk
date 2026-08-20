#![forbid(unsafe_code)]

use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use hyperliquid_capabilities::{
    CoverageDiff, MANIFEST_RELATIVE, MATRIX_RELATIVE, coverage_report, diff_reports,
    encode_coverage_report, find_workspace_root, format_diff, load_manifest, parse_coverage_report,
    render_coverage_matrix, validate_manifest,
};

const USAGE: &str = "usage: hyperliquid-capabilities validate [--root <path>]
       hyperliquid-capabilities render-docs [--check] [--root <path>]
       hyperliquid-capabilities coverage [--root <path>]
       hyperliquid-capabilities diff --left <report.json> --right <report.json>";

enum Command {
    Validate,
    RenderDocs { check: bool },
    Coverage,
    Diff { left: PathBuf, right: PathBuf },
}

struct Args {
    command: Command,
    root: Option<PathBuf>,
}

fn main() -> ExitCode {
    match run(env::args_os().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            if error == USAGE {
                eprintln!("{USAGE}");
                ExitCode::from(2)
            } else {
                eprintln!("{error}");
                ExitCode::from(1)
            }
        }
    }
}

fn run(raw: Vec<OsString>) -> Result<(), String> {
    let args = parse_args(&raw)?;
    match args.command {
        Command::Validate => {
            let manifest = load_validated_manifest(args.root.as_deref())?;
            println!("capability-manifest:ok rows={}", manifest.capability.len());
            Ok(())
        }
        Command::RenderDocs { check } => {
            let root = resolve_root(args.root.as_deref())?;
            let manifest = load_validated_from_root(&root)?;
            let generated = render_coverage_matrix(&manifest);
            let path = root.join(MATRIX_RELATIVE);
            if check {
                let committed = fs::read_to_string(&path)
                    .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
                if committed != generated {
                    return Err(format!(
                        "coverage-matrix: generated output differs from {MATRIX_RELATIVE}"
                    ));
                }
                println!("coverage-matrix:ok");
                return Ok(());
            }
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)
                    .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
            }
            fs::write(&path, generated)
                .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
            println!("coverage-matrix:wrote {MATRIX_RELATIVE}");
            Ok(())
        }
        Command::Coverage => {
            let manifest = load_validated_manifest(args.root.as_deref())?;
            let encoded = encode_coverage_report(&coverage_report(&manifest))?;
            print!("{encoded}");
            Ok(())
        }
        Command::Diff { left, right } => {
            let left_report = load_report(&left)?;
            let right_report = load_report(&right)?;
            let diff = diff_reports(&left_report, &right_report);
            if diff.is_empty() {
                println!("coverage-diff:identical");
                return Ok(());
            }
            print_diff(&diff);
            Err("coverage-diff:changed".to_owned())
        }
    }
}

fn parse_args(raw: &[OsString]) -> Result<Args, String> {
    let mut args = raw.iter();
    let command = args
        .next()
        .and_then(|value| value.to_str())
        .ok_or_else(|| USAGE.to_owned())?;

    let mut check = false;
    let mut root = None;
    let mut left = None;
    let mut right = None;
    while let Some(flag) = args.next() {
        let flag = flag.to_str().ok_or_else(|| USAGE.to_owned())?;
        match flag {
            "--check" if !check => check = true,
            "--root" if root.is_none() => {
                root = Some(PathBuf::from(args.next().ok_or_else(|| USAGE.to_owned())?));
            }
            "--left" if left.is_none() => {
                left = Some(PathBuf::from(args.next().ok_or_else(|| USAGE.to_owned())?));
            }
            "--right" if right.is_none() => {
                right = Some(PathBuf::from(args.next().ok_or_else(|| USAGE.to_owned())?));
            }
            _ => return Err(USAGE.to_owned()),
        }
    }

    let parsed = match command {
        "validate" if !check && left.is_none() && right.is_none() => Command::Validate,
        "render-docs" if left.is_none() && right.is_none() => Command::RenderDocs { check },
        "coverage" if !check && left.is_none() && right.is_none() => Command::Coverage,
        "diff" if !check && root.is_none() => {
            let left = left.ok_or_else(|| USAGE.to_owned())?;
            let right = right.ok_or_else(|| USAGE.to_owned())?;
            Command::Diff { left, right }
        }
        _ => return Err(USAGE.to_owned()),
    };
    Ok(Args {
        command: parsed,
        root,
    })
}

fn load_validated_manifest(
    root: Option<&Path>,
) -> Result<hyperliquid_capabilities::Manifest, String> {
    load_validated_from_root(&resolve_root(root)?)
}

fn load_validated_from_root(root: &Path) -> Result<hyperliquid_capabilities::Manifest, String> {
    let path = root.join(MANIFEST_RELATIVE);
    let manifest = load_manifest(&path)?;
    if let Err(errors) = validate_manifest(&manifest) {
        return Err(errors.join("\n"));
    }
    Ok(manifest)
}

fn resolve_root(root: Option<&Path>) -> Result<PathBuf, String> {
    match root {
        Some(path) => Ok(path.to_path_buf()),
        None => find_workspace_root(&env::current_dir().map_err(|error| error.to_string())?),
    }
}

fn load_report(path: &Path) -> Result<hyperliquid_capabilities::CoverageReport, String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    parse_coverage_report(&text)
}

fn print_diff(diff: &CoverageDiff) {
    let formatted = format_diff(diff);
    if !formatted.is_empty() {
        println!("{formatted}");
    }
}
