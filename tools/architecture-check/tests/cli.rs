use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use cargo_metadata::Metadata;

static TEMP_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn check_fixture(name: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_architecture-check"))
        .args(["check", "--metadata"])
        .arg(fixture(name))
        .output()
        .expect("architecture-check must start")
}

fn partial_resolve_document() -> serde_json::Value {
    let file = std::fs::File::open(fixture("policy-violations.json"))
        .expect("policy fixture must be readable");
    let mut document: serde_json::Value =
        serde_json::from_reader(file).expect("policy fixture must be valid JSON");

    document["packages"]
        .as_array_mut()
        .expect("packages must be an array")
        .retain(|package| {
            matches!(
                package["name"].as_str(),
                Some("feature-core" | "feature-bridge" | "model-runtime")
            )
        });
    document["workspace_members"]
        .as_array_mut()
        .expect("workspace members must be an array")
        .retain(|id| id.as_str().is_some_and(|id| id.contains("feature-core")));
    document["workspace_default_members"]
        .as_array_mut()
        .expect("workspace default members must be an array")
        .retain(|id| id.as_str().is_some_and(|id| id.contains("feature-core")));
    document["resolve"]["nodes"]
        .as_array_mut()
        .expect("resolve nodes must be an array")
        .retain(|node| {
            node["id"]
                .as_str()
                .is_some_and(|id| id.contains("feature-core"))
        });
    document
}

fn check_document(document: &serde_json::Value) -> Output {
    let sequence = TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "alpha-desk-architecture-check-{}-{sequence}.json",
        std::process::id()
    ));
    std::fs::write(
        &path,
        serde_json::to_vec(document).expect("metadata fixture must serialize"),
    )
    .expect("temporary metadata fixture must be written");

    let output = Command::new(env!("CARGO_BIN_EXE_architecture-check"))
        .args(["check", "--metadata"])
        .arg(&path)
        .output()
        .expect("architecture-check must start");
    std::fs::remove_file(path).expect("temporary metadata fixture must be removed");
    output
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).expect("diagnostics must be UTF-8")
}

#[test]
fn fails_closed_when_reachable_non_workspace_bridge_has_no_resolve_node() {
    let metadata: Metadata = serde_json::from_value(partial_resolve_document())
        .expect("partial resolve fixture must be valid cargo metadata");

    assert_eq!(
        architecture_check::check(&metadata),
        Err(
            "reachable package must have exactly one resolve node: feature-bridge \
             (path+file:///fixture/crates/feature-bridge#0.1.0); found 0"
                .to_owned()
        )
    );
}

#[cfg(unix)]
#[test]
fn non_utf8_control_path_is_escaped_and_returns_metadata_exit_code() {
    use std::os::unix::ffi::OsStringExt;

    let hostile_path = OsString::from_vec(b"/definitely/missing-\n\x1b[2J-\xff.json".to_vec());
    let output = Command::new(env!("CARGO_BIN_EXE_architecture-check"))
        .arg("check")
        .arg("--metadata")
        .arg(hostile_path)
        .output()
        .expect("architecture-check must start");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        output.stderr,
        b"metadata-error: cannot open /definitely/missing-\\x0a\\x1b[2J-\\xff.json: \
          I/O error kind NotFound\n"
    );
}

#[test]
fn hostile_package_name_and_id_are_escaped_byte_for_byte() {
    let mut document = partial_resolve_document();
    let hostile_name = "feature-bridge\n\u{1b}[2J";
    let hostile_id = "path+file:///fixture/crates/feature-bridge\n\u{1b}[2J#0.1.0";

    let packages = document["packages"]
        .as_array_mut()
        .expect("packages must be an array");
    let bridge = packages
        .iter_mut()
        .find(|package| package["name"] == "feature-bridge")
        .expect("bridge package must exist");
    bridge["name"] = hostile_name.into();
    bridge["id"] = hostile_id.into();

    let nodes = document["resolve"]["nodes"]
        .as_array_mut()
        .expect("resolve nodes must be an array");
    let feature_node = nodes
        .iter_mut()
        .find(|node| {
            node["id"]
                .as_str()
                .is_some_and(|id| id.contains("feature-core"))
        })
        .expect("feature node must exist");
    feature_node["dependencies"][0] = hostile_id.into();
    feature_node["deps"][0]["pkg"] = hostile_id.into();

    let output = check_document(&document);

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        output.stderr,
        b"metadata-error: reachable package must have exactly one resolve node: \
          feature-bridge\\x0a\\x1b[2J \
          (path+file:///fixture/crates/feature-bridge\\x0a\\x1b[2J#0.1.0); found 0\n"
    );
}

#[test]
fn hostile_package_name_is_escaped_in_policy_diagnostic() {
    let file = std::fs::File::open(fixture("policy-violations.json"))
        .expect("policy fixture must be readable");
    let mut document: serde_json::Value =
        serde_json::from_reader(file).expect("policy fixture must be valid JSON");
    let bridge = document["packages"]
        .as_array_mut()
        .expect("packages must be an array")
        .iter_mut()
        .find(|package| package["name"] == "feature-bridge")
        .expect("bridge package must exist");
    bridge["name"] = "feature-bridge\n\u{1b}[2J".into();

    let output = check_document(&document);

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_ascii());
    assert!(output.stderr.windows(
        b"forbidden-dependency: feature-core -> feature-bridge\\x0a\\x1b[2J -> model-runtime\n"
            .len()
    )
    .any(|window| {
        window
            == b"forbidden-dependency: feature-core -> feature-bridge\\x0a\\x1b[2J -> \
                 model-runtime\n"
    }));
}

#[test]
fn cli_exit_codes_and_trusted_usage_diagnostic_are_stable() {
    let usage = Command::new(env!("CARGO_BIN_EXE_architecture-check"))
        .output()
        .expect("architecture-check must start");
    assert_eq!(usage.status.code(), Some(2));
    assert!(usage.stdout.is_empty());
    assert_eq!(
        usage.stderr,
        b"usage: architecture-check check [--metadata <path>]\n"
    );

    let violation = check_fixture("policy-violations.json");
    assert_eq!(violation.status.code(), Some(1));
    assert!(violation.stdout.is_empty());

    let clean = check_fixture("valid-minimal-workspace.json");
    assert_eq!(clean.status.code(), Some(0));
    assert!(clean.stdout.is_empty());
    assert!(clean.stderr.is_empty());
}

#[test]
fn fails_closed_when_workspace_has_no_members() {
    let output = check_fixture("zero-workspace.json");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        output.stderr,
        b"metadata-error: cargo metadata workspace has no members\n"
    );
}

#[test]
fn rejects_disagreement_between_structured_and_package_id_dependencies() {
    let file = std::fs::File::open(fixture("policy-violations.json"))
        .expect("policy fixture must be readable");
    let mut document: serde_json::Value =
        serde_json::from_reader(file).expect("policy fixture must be valid JSON");
    for node in document["resolve"]["nodes"]
        .as_array_mut()
        .expect("resolve nodes must be an array")
    {
        node["deps"] = serde_json::Value::Array(Vec::new());
    }

    let output = check_document(&document);

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        output.stderr,
        b"metadata-error: resolve node dependency fields disagree: \
          path+file:///fixture/tools/architecture-fixture#0.1.0; deps-only: []; \
          dependencies-only: [path+file:///fixture/services/hl-exec#0.1.0]\n"
    );
}

#[test]
fn rejects_shortest_forbidden_path_using_resolved_package_identity() {
    let output = check_fixture("policy-violations.json");

    assert!(!output.status.success());
    assert!(stderr(&output).contains(
        "forbidden-dependency: feature-core -> feature-bridge -> model-runtime\n\
         rule: feature definitions must not depend on an inference runtime"
    ));
}

#[test]
fn rejects_domain_dependency_on_storage_port() {
    let output = check_fixture("policy-violations.json");

    assert!(!output.status.success());
    assert!(stderr(&output).contains(
        "forbidden-dependency: domain-types -> storage-ports\n\
         rule: domain types must not depend on storage ports"
    ));
}

#[test]
fn rejects_domain_crate_dependency_on_service() {
    let output = check_fixture("policy-violations.json");

    assert!(!output.status.success());
    assert!(stderr(&output).contains(
        "forbidden-dependency: domain-types -> hl-core\n\
         rule: domain crates must not depend on service packages"
    ));
}

#[test]
fn rejects_any_dependency_on_hl_exec() {
    let output = check_fixture("policy-violations.json");

    assert!(!output.status.success());
    assert!(stderr(&output).contains(
        "forbidden-dependency: architecture-fixture -> hl-exec\n\
         rule: hl-exec is excluded from every V1 dependency graph"
    ));
}

#[test]
fn rejects_cargo_valid_cycle_containing_a_dev_edge() {
    let output = check_fixture("invalid-cycle.json");

    assert!(!output.status.success());
    assert!(stderr(&output).contains("cyclic-dependency: a -> b -> a"));
}

#[test]
fn fails_closed_when_resolve_graph_is_missing() {
    let output = check_fixture("missing-resolve.json");

    assert!(!output.status.success());
    assert_eq!(
        stderr(&output),
        "metadata-error: cargo metadata resolve graph is missing\n"
    );
}

#[test]
fn fails_closed_for_unknown_workspace_package() {
    let output = check_fixture("unknown-workspace-package.json");

    assert!(!output.status.success());
    assert_eq!(
        stderr(&output),
        "metadata-error: workspace member is absent from packages: \
         path+file:///fixture/crates/missing#0.1.0\n"
    );
}
