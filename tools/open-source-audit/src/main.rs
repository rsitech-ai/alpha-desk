use std::env;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, ExitCode};

use open_source_audit::{AuditPolicy, audit_paths};

const USAGE: &str = "usage: open-source-audit check --policy <path> --root <path>";

fn main() -> ExitCode {
    match run(env::args_os().skip(1).collect()) {
        Ok(code) => code,
        Err(error) => {
            if error == USAGE {
                eprintln!("{USAGE}");
            } else {
                eprintln!("ERROR {error}");
            }
            ExitCode::from(2)
        }
    }
}

fn run(arguments: Vec<std::ffi::OsString>) -> Result<ExitCode, String> {
    let (policy_path, root) = parse_arguments(arguments)?;
    let root = root
        .canonicalize()
        .map_err(|error| format!("input.missing_root: {error}"))?;
    let policy_path = resolve_inside_root(&root, &policy_path)?;
    let policy_source = fs::read_to_string(&policy_path)
        .map_err(|error| format!("input.policy_read_failed: {error}"))?;
    let policy = AuditPolicy::from_toml(&policy_source).map_err(|error| error.to_string())?;
    let inventory = git_inventory(&root)?;
    let report = audit_paths(&root, &inventory, &policy).map_err(|error| error.to_string())?;

    if report.is_pass() {
        println!("PASS files={}", inventory.len());
        return Ok(ExitCode::SUCCESS);
    }
    for finding in report.findings() {
        println!(
            "FAIL {} {}",
            finding.reason_code(),
            escaped_path(finding.path())
        );
    }
    Ok(ExitCode::from(1))
}

fn parse_arguments(arguments: Vec<std::ffi::OsString>) -> Result<(PathBuf, PathBuf), String> {
    if arguments.len() != 5 || arguments.first().and_then(|value| value.to_str()) != Some("check") {
        return Err(USAGE.to_owned());
    }
    let mut policy = None;
    let mut root = None;
    let mut index = 1;
    while index < arguments.len() {
        let flag = arguments[index].to_str().ok_or_else(|| USAGE.to_owned())?;
        let value = arguments.get(index + 1).ok_or_else(|| USAGE.to_owned())?;
        match flag {
            "--policy" if policy.is_none() => policy = Some(PathBuf::from(value)),
            "--root" if root.is_none() => root = Some(PathBuf::from(value)),
            _ => return Err(USAGE.to_owned()),
        }
        index += 2;
    }
    match (policy, root) {
        (Some(policy), Some(root)) => Ok((policy, root)),
        _ => Err(USAGE.to_owned()),
    }
}

fn resolve_inside_root(root: &Path, candidate: &Path) -> Result<PathBuf, String> {
    if candidate.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err("input.unsafe_policy_path: policy must be repository-relative".to_owned());
    }
    let resolved = root.join(candidate);
    let canonical = resolved
        .canonicalize()
        .map_err(|error| format!("input.policy_read_failed: {error}"))?;
    if !canonical.starts_with(root) {
        return Err("input.unsafe_policy_path: policy escapes repository root".to_owned());
    }
    Ok(canonical)
}

fn git_inventory(root: &Path) -> Result<Vec<PathBuf>, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
        ])
        .output()
        .map_err(|error| format!("input.git_unavailable: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "input.git_inventory_failed: exit={:?}",
            output.status.code()
        ));
    }

    let mut inventory = Vec::new();
    for raw in output.stdout.split(|byte| *byte == 0) {
        if raw.is_empty() {
            continue;
        }
        let text =
            std::str::from_utf8(raw).map_err(|_| "input.non_utf8_inventory_path".to_owned())?;
        let path = PathBuf::from(text);
        match fs::symlink_metadata(root.join(&path)) {
            Ok(_) => inventory.push(path),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("input.inventory_metadata_failed: {error}")),
        }
    }
    inventory.sort();
    inventory.dedup();
    Ok(inventory)
}

fn escaped_path(path: &Path) -> String {
    path.to_string_lossy()
        .chars()
        .flat_map(char::escape_default)
        .collect()
}
