use std::fs;
use std::path::{Path, PathBuf};

use open_source_audit::{AuditPolicy, Classification, audit_paths};
use tempfile::TempDir;

fn fixture() -> TempDir {
    tempfile::tempdir().expect("temporary audit root")
}

fn write(root: &Path, path: &str, bytes: &[u8]) -> PathBuf {
    let destination = root.join(path);
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).expect("create fixture parent");
    }
    fs::write(&destination, bytes).expect("write fixture");
    destination
}

fn policy(extra: &str) -> AuditPolicy {
    let source = format!(
        r#"
schema_version = 1
max_file_bytes = 1048576
public = ["README.md", "crates"]
private = []
generated_review_required = []
excluded = ["target"]
forbidden_path_prefixes = [".superpowers/", "bootstrap/source.part."]
allowed_binary_paths = ["schemas/proto/baseline/v1.pb"]

[[content_allowlist]]
rule = "private.absolute_user_path"
path = "tests/negative_path_assertion.rs"

{extra}
"#
    );
    AuditPolicy::from_toml(&source).expect("valid policy")
}

#[test]
fn every_tracked_top_level_path_must_be_classified() {
    let root = fixture();
    write(root.path(), "README.md", b"# project\n");
    write(root.path(), "unknown/file.txt", b"unclassified\n");

    let report = audit_paths(
        root.path(),
        &["README.md".into(), "unknown/file.txt".into()],
        &policy(""),
    )
    .expect("audit completes");

    assert_eq!(
        report.reason_codes(),
        vec!["classification.unclassified_top_level"]
    );
}

#[test]
fn classification_is_explicit_and_stable() {
    let audit_policy = policy("");

    assert_eq!(
        audit_policy.classification_for(Path::new("README.md")),
        Some(Classification::Public)
    );
    assert_eq!(
        audit_policy.classification_for(Path::new("crates/domain/src/lib.rs")),
        Some(Classification::Public)
    );
    assert_eq!(
        audit_policy.classification_for(Path::new("target/debug/tool")),
        Some(Classification::Excluded)
    );
    assert_eq!(
        audit_policy.classification_for(Path::new("unlisted/file")),
        None
    );
}

#[test]
fn historical_transport_fragments_are_rejected_even_if_the_root_is_public() {
    let root = fixture();
    write(
        root.path(),
        "bootstrap/source.part.00",
        b"opaque transport fragment",
    );
    let audit_policy = AuditPolicy::from_toml(
        r#"
schema_version = 1
max_file_bytes = 1048576
public = ["bootstrap"]
private = []
generated_review_required = []
excluded = []
forbidden_path_prefixes = ["bootstrap/source.part."]
allowed_binary_paths = []
"#,
    )
    .expect("valid policy");

    let report = audit_paths(
        root.path(),
        &["bootstrap/source.part.00".into()],
        &audit_policy,
    )
    .expect("audit completes");

    assert_eq!(report.reason_codes(), vec!["path.forbidden_prefix"]);
}

#[test]
fn seeded_secret_and_private_alpha_canaries_fail_closed() {
    let root = fixture();
    write(
        root.path(),
        "crates/demo/src/lib.rs",
        b"token = \"ghp_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\"\n\
          private_signal_threshold = \"0.731\"\n",
    );

    let report = audit_paths(root.path(), &["crates/demo/src/lib.rs".into()], &policy(""))
        .expect("audit completes");

    assert_eq!(
        report.reason_codes(),
        vec!["private.alpha_threshold", "secret.github_pat"]
    );
}

#[test]
fn absolute_developer_paths_require_an_exact_rule_and_path_allowlist() {
    let root = fixture();
    write(
        root.path(),
        "crates/demo/src/lib.rs",
        b"const HOME: &str = \"/Users/alice/project\";\n",
    );
    write(
        root.path(),
        "tests/negative_path_assertion.rs",
        b"assert!(!output.contains(\"/Users/\"));\n",
    );

    let report = audit_paths(
        root.path(),
        &[
            "crates/demo/src/lib.rs".into(),
            "tests/negative_path_assertion.rs".into(),
        ],
        &policy(
            r#"
[[classification_overrides]]
path = "tests"
classification = "public"
"#,
        ),
    )
    .expect("audit completes");

    assert_eq!(report.reason_codes(), vec!["private.absolute_user_path"]);
    assert_eq!(
        report.findings()[0].path(),
        Path::new("crates/demo/src/lib.rs")
    );
}

#[test]
fn binary_files_and_oversized_files_require_explicit_review() {
    let root = fixture();
    write(root.path(), "crates/demo/blob.bin", b"abc\0def");
    write(root.path(), "crates/demo/large.txt", &[b'x'; 17]);
    let audit_policy = AuditPolicy::from_toml(
        r#"
schema_version = 1
max_file_bytes = 16
public = ["crates"]
private = []
generated_review_required = []
excluded = []
forbidden_path_prefixes = []
allowed_binary_paths = []
"#,
    )
    .expect("valid policy");

    let report = audit_paths(
        root.path(),
        &[
            "crates/demo/blob.bin".into(),
            "crates/demo/large.txt".into(),
        ],
        &audit_policy,
    )
    .expect("audit completes");

    assert_eq!(
        report.reason_codes(),
        vec!["content.binary_unreviewed", "content.file_too_large"]
    );
}

#[test]
fn missing_and_escaping_inventory_paths_fail_as_input_errors() {
    let root = fixture();
    let missing = audit_paths(root.path(), &["README.md".into()], &policy(""))
        .expect_err("missing inventory file must fail");
    assert_eq!(missing.reason_code(), "input.missing_file");

    let escaping = audit_paths(root.path(), &[PathBuf::from("../outside")], &policy(""))
        .expect_err("escaping inventory path must fail");
    assert_eq!(escaping.reason_code(), "input.unsafe_path");
}
