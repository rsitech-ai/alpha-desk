use std::fs;
use std::path::Path;
use std::process::Command;

use tempfile::TempDir;

fn git(root: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .status()
        .expect("run git");
    assert!(status.success(), "git command failed: {args:?}");
}

fn repository() -> TempDir {
    let root = tempfile::tempdir().expect("temporary repository");
    git(root.path(), &["init", "-q"]);
    root
}

fn write(root: &Path, path: &str, content: &str) {
    let destination = root.join(path);
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).expect("create fixture parent");
    }
    fs::write(destination, content).expect("write fixture");
}

fn write_policy(root: &Path) {
    write(
        root,
        "config/open-source-policy.toml",
        r#"
schema_version = 1
max_file_bytes = 1048576
public = ["README.md", "config", "src"]
private = []
generated_review_required = []
excluded = []
forbidden_path_prefixes = ["bootstrap/source.part."]
allowed_binary_paths = []
"#,
    );
}

#[test]
fn check_audits_tracked_and_untracked_nonignored_files() {
    let root = repository();
    write_policy(root.path());
    write(root.path(), "README.md", "# fixture\n");
    write(root.path(), "src/lib.rs", "pub const SAFE: bool = true;\n");
    git(root.path(), &["add", "README.md", "config"]);
    write(
        root.path(),
        "src/private.rs",
        "private_signal_threshold = \"0.42\"\n",
    );

    let output = Command::new(env!("CARGO_BIN_EXE_open-source-audit"))
        .args([
            "check",
            "--policy",
            "config/open-source-policy.toml",
            "--root",
        ])
        .arg(root.path())
        .output()
        .expect("run audit");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(output.stdout).expect("utf8 stdout"),
        "FAIL private.alpha_threshold src/private.rs\n"
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn check_passes_with_a_stable_file_count() {
    let root = repository();
    write_policy(root.path());
    write(root.path(), "README.md", "# fixture\n");
    write(root.path(), "src/lib.rs", "pub const SAFE: bool = true;\n");
    git(root.path(), &["add", "."]);

    let output = Command::new(env!("CARGO_BIN_EXE_open-source-audit"))
        .args([
            "check",
            "--policy",
            "config/open-source-policy.toml",
            "--root",
        ])
        .arg(root.path())
        .output()
        .expect("run audit");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).expect("utf8 stdout"),
        "PASS files=3\n"
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn malformed_arguments_and_policy_errors_are_actionable() {
    let root = repository();
    write(root.path(), "config.toml", "schema_version = 99\n");

    let malformed = Command::new(env!("CARGO_BIN_EXE_open-source-audit"))
        .output()
        .expect("run audit");
    assert_eq!(malformed.status.code(), Some(2));
    assert_eq!(
        String::from_utf8(malformed.stderr).expect("utf8 stderr"),
        "usage: open-source-audit check --policy <path> --root <path>\n"
    );

    let invalid_policy = Command::new(env!("CARGO_BIN_EXE_open-source-audit"))
        .args(["check", "--policy", "config.toml", "--root"])
        .arg(root.path())
        .output()
        .expect("run audit");
    assert_eq!(invalid_policy.status.code(), Some(2));
    assert!(
        String::from_utf8(invalid_policy.stderr)
            .expect("utf8 stderr")
            .starts_with("ERROR policy.")
    );
}
