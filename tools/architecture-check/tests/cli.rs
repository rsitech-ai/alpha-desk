use std::path::{Path, PathBuf};
use std::process::{Command, Output};

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

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).expect("diagnostics must be UTF-8")
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
