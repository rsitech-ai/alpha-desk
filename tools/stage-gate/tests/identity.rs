#![cfg(unix)]

use std::{
    fs,
    os::unix::fs::{PermissionsExt as _, symlink},
    path::Path,
};

use stage_gate::identity::{
    IdentityErrorCode, capture_executable_identity, parse_rustc_host, resolve_program,
    version_output_matches,
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
    let multicall = trusted.path().join("multicall");
    fs::write(
        &multicall,
        concat!(
            "#!/bin/sh\n",
            "case \"${0##*/}\" in\n",
            "  cargo) printf 'cargo proxy 1.0.0\\n' ;;\n",
            "  *) exit 41 ;;\n",
            "esac\n",
        ),
    )
    .unwrap();
    let mut permissions = fs::metadata(&multicall).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&multicall, permissions).unwrap();
    symlink("multicall", trusted.path().join("cargo")).unwrap();

    let resolved =
        resolve_program("cargo", &[trusted.path().to_path_buf()], Path::new(".")).unwrap();
    let identity = capture_executable_identity("cargo", &resolved, &[])
        .expect("an approved proxy must execute through its selected invocation name");

    assert_eq!(resolved.invocation_path, trusted.path().join("cargo"));
    assert_eq!(
        resolved.executable_path,
        trusted.path().join("multicall").canonicalize().unwrap()
    );
    assert_eq!(identity.resolved_path, resolved.executable_path);
    assert_eq!(identity.version_output, "cargo proxy 1.0.0");
    assert_eq!(identity.sha256.len(), 64);
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
