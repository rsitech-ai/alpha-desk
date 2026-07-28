#![cfg(unix)]

use std::{
    fs,
    os::unix::fs::{PermissionsExt as _, symlink},
    path::Path,
};

use stage_gate::identity::{
    IdentityErrorCode, capture_executable_identity, executable_file_identity, parse_rustc_host,
    resolve_program, snapshot_resolved_program, version_output_matches,
};
use tempfile::TempDir;

#[test]
fn resolution_uses_only_committed_roots_and_captures_canonical_executable_hash() {
    let trusted = TempDir::new().unwrap();
    let attacker = TempDir::new().unwrap();
    copy_executable(Path::new("/usr/bin/true"), &trusted.path().join("tool"));
    copy_executable(Path::new("/usr/bin/false"), &attacker.path().join("tool"));

    let resolved =
        resolve_program("tool", &[trusted.path().to_path_buf()], Path::new(".")).unwrap();
    let identity = capture_executable_identity("tool", &resolved, &[]).unwrap();

    assert_eq!(resolved.invocation_path, trusted.path().join("tool"));
    assert_eq!(
        resolved.executable_path,
        trusted.path().join("tool").canonicalize().unwrap()
    );
    assert_eq!(identity.resolved_path, resolved.executable_path);
    assert_eq!(identity.sha256.len(), 64);
}

#[test]
fn non_executable_and_outside_root_symlink_are_rejected() {
    let trusted = TempDir::new().unwrap();
    let plain = trusted.path().join("plain");
    fs::write(&plain, b"not executable\n").unwrap();

    let error = resolve_program("plain", &[trusted.path().to_path_buf()], Path::new("."))
        .expect_err("non-executable candidates must fail closed");
    assert_eq!(error.code(), IdentityErrorCode::NotExecutable);

    symlink("/usr/bin/true", trusted.path().join("escaped")).unwrap();
    let error = resolve_program("escaped", &[trusted.path().to_path_buf()], Path::new("."))
        .expect_err("a canonical target outside its approved root must fail closed");
    assert_eq!(error.code(), IdentityErrorCode::OutsideApprovedRoot);
}

#[test]
fn approved_multicall_symlink_keeps_its_invocation_identity() {
    let trusted = TempDir::new().unwrap();
    let multicall = std::env::current_exe().unwrap().canonicalize().unwrap();
    symlink(&multicall, trusted.path().join("cargo")).unwrap();
    let roots = vec![
        trusted.path().to_path_buf(),
        multicall.parent().unwrap().to_path_buf(),
    ];

    let resolved = resolve_program("cargo", &roots, Path::new(".")).unwrap();
    fs::remove_file(trusted.path().join("cargo")).unwrap();
    symlink("/usr/bin/false", trusted.path().join("cargo")).unwrap();
    let identity = capture_executable_identity(
        "cargo",
        &resolved,
        &[
            "--exact".to_owned(),
            "approved_multicall_identity_helper".to_owned(),
            "--nocapture".to_owned(),
        ],
    )
    .expect("execution must stay bound to the approved target after a proxy swap");

    assert_eq!(resolved.invocation_path, trusted.path().join("cargo"));
    assert_eq!(resolved.executable_path, multicall);
    assert_eq!(identity.resolved_path, resolved.executable_path);
    assert!(identity.version_output.contains("cargo proxy 1.0.0"));
    assert_eq!(identity.sha256.len(), 64);
}

#[test]
fn approved_multicall_identity_helper() {
    let invoked_as = std::env::args_os().next().unwrap();
    if Path::new(&invoked_as)
        .file_name()
        .and_then(|name| name.to_str())
        == Some("cargo")
    {
        println!("cargo proxy 1.0.0");
    }
}

#[test]
fn canonical_target_replacement_is_rejected_before_snapshot() {
    let trusted = TempDir::new().unwrap();
    let target = trusted.path().join("multicall");
    copy_executable(&std::env::current_exe().unwrap(), &target);
    symlink("multicall", trusted.path().join("cargo")).unwrap();
    let resolved =
        resolve_program("cargo", &[trusted.path().to_path_buf()], Path::new(".")).unwrap();
    let identity = executable_file_identity("cargo", &resolved).unwrap();

    copy_executable(Path::new("/usr/bin/false"), &target);

    assert!(
        snapshot_resolved_program(&resolved, &identity.sha256).is_err(),
        "a canonical target replaced after hashing must never be snapshotted for execution"
    );
}

#[test]
fn target_triple_is_parsed_from_observed_rustc_output() {
    assert_eq!(
        parse_rustc_host("rustc 1.97.1\nhost: aarch64-apple-darwin\n").unwrap(),
        "aarch64-apple-darwin"
    );
    assert!(parse_rustc_host("rustc unknown\n").is_err());
}

#[test]
fn declared_builder_version_expectation_matches_and_rejects_mismatch() {
    assert!(version_output_matches(
        "Swift version 6.3.0 (swift-6.3-RELEASE)",
        Some("Swift version 6.3")
    ));
    assert!(!version_output_matches(
        "Swift version 6.2.1 (swift-6.2.1-RELEASE)",
        Some("Swift version 6.3")
    ));
    assert!(version_output_matches("anything", None));
}

fn copy_executable(source: &Path, destination: &Path) {
    fs::copy(source, destination).unwrap();
    let mut permissions = fs::metadata(destination).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(destination, permissions).unwrap();
}
