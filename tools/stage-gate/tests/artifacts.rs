#![cfg(unix)]

use std::{fs, os::unix::fs::symlink, path::PathBuf};

use stage_gate::artifacts::{
    ArtifactErrorCode, ArtifactRequest, collect_artifacts, collect_artifacts_observed,
};
use tempfile::TempDir;

const ALPHA_SHA256: &str = "b6a98d9ce9a2d9149288fa3df42d377c3e42737afdcdaf714e33c0a100b51060";

#[test]
fn regular_files_inside_the_root_produce_stable_sorted_metadata() {
    let root = TempDir::new().unwrap();
    fs::create_dir(root.path().join("dist")).unwrap();
    fs::write(root.path().join("dist/zeta.bin"), b"zeta\n").unwrap();
    fs::write(root.path().join("dist/alpha.bin"), b"alpha\n").unwrap();
    let requests = vec![
        request("zeta", "dist/zeta.bin"),
        request("alpha", "dist/alpha.bin"),
    ];

    let manifest = collect_artifacts(root.path(), &requests).unwrap();

    assert_eq!(manifest.schema_version, 1);
    assert_eq!(manifest.artifacts[0].logical_name, "alpha");
    assert_eq!(manifest.artifacts[0].relative_path, "dist/alpha.bin");
    assert_eq!(manifest.artifacts[0].kind, "executable");
    assert_eq!(manifest.artifacts[0].size_bytes, 6);
    assert_eq!(manifest.artifacts[0].sha256, ALPHA_SHA256);
    assert_eq!(manifest.artifacts[0].producer, "cargo-build");
    assert_eq!(manifest.artifacts[0].target_triple, "aarch64-apple-darwin");
    assert_eq!(manifest.artifacts[0].profile, "release");
    assert_eq!(manifest.artifacts[1].logical_name, "zeta");
}

#[test]
fn symlink_artifact_is_rejected() {
    let root = TempDir::new().unwrap();
    fs::write(root.path().join("real.bin"), b"alpha\n").unwrap();
    symlink("real.bin", root.path().join("link.bin")).unwrap();

    let error = collect_artifacts(root.path(), &[request("linked", "link.bin")])
        .expect_err("symlink artifacts must fail closed");

    assert_eq!(error.code(), ArtifactErrorCode::Symlink);
}

#[test]
fn non_regular_artifact_is_rejected() {
    let root = TempDir::new().unwrap();
    fs::create_dir(root.path().join("directory")).unwrap();

    let error = collect_artifacts(root.path(), &[request("directory", "directory")])
        .expect_err("directories are not artifacts");

    assert_eq!(error.code(), ArtifactErrorCode::NonRegular);
}

#[test]
fn artifact_outside_root_is_rejected() {
    let root = TempDir::new().unwrap();
    let outside = root.path().parent().unwrap().join("outside-stage-gate.bin");
    fs::write(&outside, b"outside\n").unwrap();

    let error = collect_artifacts(
        root.path(),
        &[request("outside", "../outside-stage-gate.bin")],
    )
    .expect_err("parent traversal must fail closed");

    assert_eq!(error.code(), ArtifactErrorCode::OutsideRoot);
    fs::remove_file(outside).unwrap();
}

#[test]
fn duplicate_logical_name_or_path_is_rejected() {
    let root = TempDir::new().unwrap();
    fs::write(root.path().join("artifact.bin"), b"alpha\n").unwrap();

    let duplicate_name = collect_artifacts(
        root.path(),
        &[
            request("same", "artifact.bin"),
            request("same", "artifact.bin"),
        ],
    )
    .expect_err("duplicate logical names must fail closed");
    assert_eq!(duplicate_name.code(), ArtifactErrorCode::Duplicate);

    let duplicate_path = collect_artifacts(
        root.path(),
        &[
            request("first", "artifact.bin"),
            request("second", "artifact.bin"),
        ],
    )
    .expect_err("duplicate paths must fail closed");
    assert_eq!(duplicate_path.code(), ArtifactErrorCode::Duplicate);
}

#[test]
fn missing_artifact_is_rejected() {
    let root = TempDir::new().unwrap();

    let error = collect_artifacts(root.path(), &[request("missing", "missing.bin")])
        .expect_err("missing artifact must fail closed");

    assert_eq!(error.code(), ArtifactErrorCode::Missing);
}

#[test]
fn artifact_changed_during_hashing_is_rejected() {
    let root = TempDir::new().unwrap();
    fs::write(root.path().join("artifact.bin"), b"alpha\n").unwrap();
    let artifact_path = root.path().join("artifact.bin");

    let error = collect_artifacts_observed(
        root.path(),
        &[request("mutable", "artifact.bin")],
        |logical_name, phase| {
            if logical_name == "mutable" && phase.is_after_hash() {
                fs::write(&artifact_path, b"omega\n").unwrap();
            }
        },
    )
    .expect_err("a TOCTOU change must fail closed");

    assert_eq!(error.code(), ArtifactErrorCode::ChangedDuringRead);
}

fn request(logical_name: &str, relative_path: &str) -> ArtifactRequest {
    ArtifactRequest {
        logical_name: logical_name.to_owned(),
        relative_path: PathBuf::from(relative_path),
        kind: "executable".to_owned(),
        producer: "cargo-build".to_owned(),
        target_triple: "aarch64-apple-darwin".to_owned(),
        profile: "release".to_owned(),
        expected_sha256: None,
    }
}
