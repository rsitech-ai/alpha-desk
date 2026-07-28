#![cfg(unix)]

use std::{fs, os::unix::fs::symlink, path::Path};

use stage_gate::output::{OutputErrorCode, OutputObservation, OutputRoot};
use tempfile::TempDir;

#[test]
fn retained_output_root_continues_in_retained_directory_after_parent_swap() {
    let repository = TempDir::new().unwrap();
    let root = repository.path().join("target/stage-gates");
    fs::create_dir_all(&root).unwrap();
    let output = OutputRoot::open(repository.path(), Path::new("target/stage-gates")).unwrap();
    let retained = repository.path().join("target/stage-gates-retained");
    let outside = TempDir::new().unwrap();
    fs::rename(&root, &retained).unwrap();
    symlink(outside.path(), &root).unwrap();

    output
        .write_atomic(Path::new("stage-0.json"), b"retained-directory")
        .unwrap();

    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
    assert_eq!(
        fs::read(retained.join("stage-0.json")).unwrap(),
        b"retained-directory"
    );
}

#[test]
fn retained_output_root_never_reopens_parent_path_before_rename() {
    let repository = TempDir::new().unwrap();
    let root = repository.path().join("target/stage-gates");
    fs::create_dir_all(&root).unwrap();
    let output = OutputRoot::open(repository.path(), Path::new("target/stage-gates")).unwrap();
    let retained = repository.path().join("target/stage-gates-retained");
    let outside = TempDir::new().unwrap();

    output
        .write_atomic_observed(
            Path::new("stage-0.json"),
            b"retained-directory",
            |observation| {
                assert_eq!(observation, OutputObservation::BeforeRename);
                fs::rename(&root, &retained).unwrap();
                symlink(outside.path(), &root).unwrap();
            },
        )
        .unwrap();

    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
    assert_eq!(
        fs::read(retained.join("stage-0.json")).unwrap(),
        b"retained-directory"
    );
}

#[test]
fn stale_output_invalidation_unlinks_symlink_without_following_target() {
    let repository = TempDir::new().unwrap();
    let root = repository.path().join("target/stage-gates");
    fs::create_dir_all(&root).unwrap();
    let victim = repository.path().join("victim");
    fs::write(&victim, b"original\n").unwrap();
    symlink(&victim, root.join("stage-0.json")).unwrap();
    let output = OutputRoot::open(repository.path(), Path::new("target/stage-gates")).unwrap();

    output.remove_if_exists(Path::new("stage-0.json")).unwrap();

    assert_eq!(fs::read(&victim).unwrap(), b"original\n");
    assert!(!root.join("stage-0.json").exists());
}

#[test]
fn final_target_symlink_race_is_removed_without_following_target() {
    let repository = TempDir::new().unwrap();
    let root = repository.path().join("target/stage-gates");
    fs::create_dir_all(&root).unwrap();
    let victim = repository.path().join("victim");
    fs::write(&victim, b"original\n").unwrap();
    let output = OutputRoot::open(repository.path(), Path::new("target/stage-gates")).unwrap();

    let error = output
        .write_atomic_observed(
            Path::new("stage-0.json"),
            b"must-not-overwrite",
            |observation| {
                assert_eq!(observation, OutputObservation::BeforeRename);
                symlink(&victim, root.join("stage-0.json")).unwrap();
            },
        )
        .expect_err("a changed final directory entry must stop publication");

    assert_eq!(error.code(), OutputErrorCode::UnsafeTarget);
    assert_eq!(fs::read(&victim).unwrap(), b"original\n");
    assert!(!root.join("stage-0.json").exists());
}

#[test]
fn output_root_rejects_empty_dot_parent_and_repeated_separator_components() {
    let repository = TempDir::new().unwrap();

    for unsafe_root in [
        "",
        ".",
        "..",
        "target/./stage-gates",
        "target/../stage-gates",
        "target//stage-gates",
        "target/stage-gates/",
    ] {
        let error = OutputRoot::open(repository.path(), Path::new(unsafe_root))
            .expect_err("lexically ambiguous output roots must fail closed");
        assert_eq!(
            error.code(),
            OutputErrorCode::UnsafeRoot,
            "unsafe root {unsafe_root:?}"
        );
    }
}
