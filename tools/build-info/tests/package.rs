use std::{path::PathBuf, process::Command};

#[test]
fn telemetry_package_verifies_from_a_fresh_isolated_target() {
    let target = tempfile::tempdir().expect("fresh package target must be available");
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root must resolve");

    let output = Command::new(env!("CARGO"))
        .current_dir(workspace)
        .args(["package", "-p", "telemetry", "--allow-dirty", "--offline"])
        .env("SOURCE_DATE_EPOCH", "1784894400")
        .env("CARGO_TARGET_DIR", target.path())
        .output()
        .expect("fresh-target Cargo package regression must run");

    assert!(
        output.status.success(),
        "fresh-target package verification failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
