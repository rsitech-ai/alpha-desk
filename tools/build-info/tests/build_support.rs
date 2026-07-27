#[allow(dead_code)]
#[path = "../../../crates/telemetry/build_support.rs"]
mod build_support;

use std::fs;
use std::process::Command;

use build_support::{
    BuildProfile, BuildSupportError, fingerprint_schema_tree, parse_source_date_epoch, source_dirty,
};

#[test]
fn source_date_epoch_accepts_unsigned_seconds_and_development_absence() {
    assert_eq!(
        parse_source_date_epoch(Some("1784894400"), BuildProfile::Development),
        Ok(Some(1_784_894_400))
    );
    assert_eq!(
        parse_source_date_epoch(None, BuildProfile::Development),
        Ok(None)
    );
}

#[test]
fn source_date_epoch_rejects_malformed_negative_and_overflow_values() {
    for value in ["", "today", "-1", "18446744073709551616"] {
        assert_eq!(
            parse_source_date_epoch(Some(value), BuildProfile::Development),
            Err(BuildSupportError::InvalidSourceDateEpoch)
        );
    }
}

#[test]
fn release_requires_source_date_epoch() {
    assert_eq!(
        parse_source_date_epoch(None, BuildProfile::Release),
        Err(BuildSupportError::ReleaseEpochRequired)
    );
}

#[test]
fn schema_fingerprint_is_order_independent_and_path_delimited() {
    let first = tempfile::tempdir().expect("temporary directory must be available");
    let second = tempfile::tempdir().expect("temporary directory must be available");
    fs::create_dir_all(first.path().join("z")).expect("fixture directory must be created");
    fs::create_dir_all(second.path().join("z")).expect("fixture directory must be created");

    fs::write(first.path().join("z/two.proto"), b"second").expect("fixture file must be written");
    fs::write(first.path().join("one.proto"), b"first").expect("fixture file must be written");
    fs::write(second.path().join("one.proto"), b"first").expect("fixture file must be written");
    fs::write(second.path().join("z/two.proto"), b"second").expect("fixture file must be written");

    assert_eq!(
        fingerprint_schema_tree(first.path()).expect("first fingerprint must succeed"),
        fingerprint_schema_tree(second.path()).expect("second fingerprint must succeed")
    );

    fs::remove_file(second.path().join("one.proto")).expect("fixture file must be removed");
    fs::create_dir_all(second.path().join("on")).expect("fixture directory must be created");
    fs::write(second.path().join("on/e.proto"), b"first").expect("fixture file must be written");

    assert_ne!(
        fingerprint_schema_tree(first.path()).expect("first fingerprint must succeed"),
        fingerprint_schema_tree(second.path()).expect("changed path must change fingerprint")
    );
}

#[test]
fn dirty_detection_tracks_only_relevant_tracked_and_untracked_source_state() {
    let repository = tempfile::tempdir().expect("temporary repository must be available");
    fs::create_dir_all(repository.path().join("crates/demo/src"))
        .expect("source directory must be created");
    fs::create_dir_all(repository.path().join("target"))
        .expect("ignored directory must be created");
    fs::write(repository.path().join(".gitignore"), "target/\n")
        .expect("ignore file must be written");
    fs::write(repository.path().join("Cargo.toml"), "[workspace]\n")
        .expect("workspace manifest must be written");
    fs::write(
        repository.path().join("crates/demo/src/lib.rs"),
        "pub fn value() {}\n",
    )
    .expect("source file must be written");

    run_git(repository.path(), &["init"]);
    run_git(repository.path(), &["add", "."]);
    run_git(
        repository.path(),
        &[
            "-c",
            "user.name=Task Test",
            "-c",
            "user.email=task@example.invalid",
            "commit",
            "-m",
            "fixture",
        ],
    );
    assert!(!source_dirty(repository.path()).expect("clean tree must be inspected"));

    fs::write(
        repository.path().join("crates/demo/src/lib.rs"),
        "pub fn changed() {}\n",
    )
    .expect("tracked source must be changed");
    assert!(source_dirty(repository.path()).expect("tracked source must be inspected"));
    fs::write(
        repository.path().join("crates/demo/src/lib.rs"),
        "pub fn value() {}\n",
    )
    .expect("tracked source must be restored");

    fs::write(
        repository.path().join("crates/demo/src/new.rs"),
        "pub fn new() {}\n",
    )
    .expect("untracked source must be written");
    assert!(source_dirty(repository.path()).expect("untracked source must be inspected"));
    fs::remove_file(repository.path().join("crates/demo/src/new.rs"))
        .expect("untracked source must be removed");

    fs::write(repository.path().join("target/noise"), "ignored")
        .expect("ignored output must be written");
    assert!(!source_dirty(repository.path()).expect("ignored output must be ignored"));
}

fn run_git(repository: &std::path::Path, arguments: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(arguments)
        .status()
        .expect("Git must run");
    assert!(status.success(), "Git command failed: {arguments:?}");
}
