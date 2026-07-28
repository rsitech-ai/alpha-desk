use std::{collections::BTreeMap, fs, path::Path, process::Command};

use sha2::Digest as _;
use stage_gate::{
    artifacts::{ArtifactManifest, ArtifactRecord},
    config::{BuilderConfig, BuilderToolConfig},
    gate::{GateReasonCode, comparison_gate_reason},
    identity::ExecutableIdentity,
    reports::{
        BuilderEnvironment, BuilderEvidenceValidation, BuilderReport, ComparisonResult, GateResult,
        InputHashErrorCode, aggregate_reports, builder_ids_are_independent,
        comparison_satisfies_reproducibility, hash_committed_inputs, validate_builder_evidence,
    },
};
use tempfile::TempDir;

#[test]
fn full_report_and_normalized_projection_have_distinct_hash_contracts() {
    let first = report("builder-a", "macOS 26.0", "aaa", "log-a");
    let second = report("builder-b", "macOS 26.1", "bbb", "log-b");

    assert_ne!(first.full_hash().unwrap(), second.full_hash().unwrap());
    assert_eq!(
        first.comparison_projection().unwrap(),
        second.comparison_projection().unwrap()
    );
    assert_eq!(
        first.projection_hash().unwrap(),
        second.projection_hash().unwrap()
    );
}

#[test]
fn aggregate_retains_both_full_hashes_and_projection_hashes() {
    let first = report("builder-a", "macOS 26.0", "aaa", "log-a");
    let second = report("builder-b", "macOS 26.1", "bbb", "log-b");

    let aggregate = aggregate_reports(&first, &second).unwrap();

    assert_eq!(aggregate.schema_version, 1);
    assert_eq!(aggregate.comparison, ComparisonResult::Identical);
    assert_eq!(aggregate.target_manifests.len(), 2);
    assert_eq!(
        aggregate.builder_report_hashes,
        vec![first.full_hash().unwrap(), second.full_hash().unwrap()]
    );
    assert_eq!(
        aggregate.projection_hashes,
        vec![
            first.projection_hash().unwrap(),
            second.projection_hash().unwrap()
        ]
    );
    assert_eq!(aggregate.comparison_manifest_sha256().unwrap().len(), 64);
    assert_eq!(aggregate.full_hash().unwrap().len(), 64);
}

#[test]
fn externally_supplied_builder_reports_reject_unknown_top_level_and_nested_fields() {
    let report = report("builder-a", "macOS 26.0", "aaa", "log-a");
    let mut top_level = serde_json::to_value(&report).unwrap();
    top_level["unexpected"] = serde_json::json!(true);
    assert!(serde_json::from_value::<BuilderReport>(top_level).is_err());

    let mut nested = serde_json::to_value(&report).unwrap();
    nested["environment"]["unexpected"] = serde_json::json!(true);
    assert!(serde_json::from_value::<BuilderReport>(nested).is_err());

    let mut executable = serde_json::to_value(&report).unwrap();
    executable["environment"]["toolchains"]["rust"]["unexpected"] = serde_json::json!(true);
    assert!(serde_json::from_value::<BuilderReport>(executable).is_err());
}

#[test]
fn heterogeneous_target_artifacts_are_not_compared_as_identical_bytes() {
    let first = report("builder-a", "macOS 26.0", "aaa", "log-a");
    let mut second = report("builder-b", "Ubuntu 24.04", "bbb", "log-b");
    second.artifacts.artifacts[0].target_triple = "x86_64-unknown-linux-gnu".to_owned();
    second.artifacts.artifacts[0].sha256 = "4".repeat(64);
    second.artifacts.artifacts[0].size_bytes = 7;

    let aggregate = aggregate_reports(&first, &second).unwrap();

    assert_eq!(aggregate.comparison, ComparisonResult::Compatible);
    assert_ne!(
        aggregate.projection_hashes[0],
        aggregate.projection_hashes[1]
    );
    assert!(!comparison_satisfies_reproducibility(aggregate.comparison));
}

#[test]
fn only_identical_comparison_satisfies_stage_zero_reproducibility() {
    assert!(comparison_satisfies_reproducibility(
        ComparisonResult::Identical
    ));
    assert!(!comparison_satisfies_reproducibility(
        ComparisonResult::Compatible
    ));
    assert!(!comparison_satisfies_reproducibility(
        ComparisonResult::Different
    ));
    assert!(!comparison_satisfies_reproducibility(
        ComparisonResult::NotRun
    ));
    assert_eq!(
        comparison_gate_reason(ComparisonResult::Compatible),
        Some(GateReasonCode::SecondBuilderNotIdentical)
    );
    assert_eq!(
        serde_json::to_value(GateReasonCode::SecondBuilderNotIdentical).unwrap(),
        "second_builder_not_identical"
    );
}

#[test]
fn independent_builder_ids_must_be_explicit_distinct_and_non_placeholder() {
    assert!(builder_ids_are_independent("macos-arm64-a", "linux-x64-b"));
    assert!(!builder_ids_are_independent(
        "local-unidentified",
        "linux-x64-b"
    ));
    assert!(!builder_ids_are_independent(
        "macos-arm64-a",
        "local-unidentified"
    ));
    assert!(!builder_ids_are_independent("same-builder", "same-builder"));
    assert!(!builder_ids_are_independent("", "linux-x64-b"));
}

#[test]
fn consuming_gate_revalidates_second_builder_tools_and_versions() {
    let config = BuilderConfig {
        target_tool: "rustc".to_owned(),
        tools: vec![
            BuilderToolConfig {
                id: "rustc".to_owned(),
                program: "rustc".to_owned(),
                args: vec!["-vV".to_owned()],
                expected_output_contains: Some("rustc 1.97.1".to_owned()),
            },
            BuilderToolConfig {
                id: "swift".to_owned(),
                program: "swift".to_owned(),
                args: vec!["--version".to_owned()],
                expected_output_contains: Some("Swift version 6.3".to_owned()),
            },
        ],
    };
    let mut second = report("builder-b", "Ubuntu 24.04", "unused", "log-b");
    second.environment.target_triple = "aarch64-apple-darwin".to_owned();
    second.environment.toolchains = BTreeMap::from([
        (
            "rustc".to_owned(),
            ExecutableIdentity {
                id: "rustc".to_owned(),
                resolved_path: "/trusted/rustc".into(),
                sha256: "7".repeat(64),
                version_output: concat!("rustc 1.97.1 (fixture)\n", "host: aarch64-apple-darwin")
                    .to_owned(),
            },
        ),
        (
            "swift".to_owned(),
            ExecutableIdentity {
                id: "swift".to_owned(),
                resolved_path: "/trusted/swift".into(),
                sha256: "8".repeat(64),
                version_output: "Swift version 6.3".to_owned(),
            },
        ),
    ]);
    refresh_toolchain_fingerprint(&mut second);
    assert_eq!(
        validate_builder_evidence(&config, &second),
        BuilderEvidenceValidation::Valid
    );

    second
        .environment
        .toolchains
        .get_mut("swift")
        .unwrap()
        .version_output = "Swift version 6.2".to_owned();
    refresh_toolchain_fingerprint(&mut second);
    assert_eq!(
        validate_builder_evidence(&config, &second),
        BuilderEvidenceValidation::VersionMismatch
    );
    assert_eq!(
        serde_json::to_value(GateReasonCode::SecondBuilderVersionMismatch).unwrap(),
        "second_builder_version_mismatch"
    );

    second.environment.toolchains.remove("swift");
    refresh_toolchain_fingerprint(&mut second);
    assert_eq!(
        validate_builder_evidence(&config, &second),
        BuilderEvidenceValidation::IdentityInvalid
    );
}

#[test]
fn same_target_artifacts_must_match_byte_for_byte() {
    let first = report("builder-a", "macOS 26.0", "aaa", "log-a");
    let mut second = report("builder-b", "macOS 26.1", "bbb", "log-b");
    second.artifacts.artifacts[0].sha256 = "4".repeat(64);

    let aggregate = aggregate_reports(&first, &second).unwrap();

    assert_eq!(aggregate.comparison, ComparisonResult::Different);
}

#[test]
fn platform_independent_inputs_must_match_even_when_builder_targets_differ() {
    let mut first = report("builder-a", "macOS 26.0", "aaa", "log-a");
    first.artifacts.artifacts[0].target_triple = "platform-independent".to_owned();
    let mut second = report("builder-b", "Ubuntu 24.04", "bbb", "log-b");
    second.artifacts.artifacts[0].target_triple = "x86_64-unknown-linux-gnu".to_owned();
    second.artifacts.artifacts[0].sha256 = "4".repeat(64);

    let aggregate = aggregate_reports(&first, &second).unwrap();

    assert_eq!(aggregate.comparison, ComparisonResult::Different);
}

#[test]
fn aggregate_hash_binds_remote_proof_and_committed_trust_registry() {
    let first = report("builder-a", "macOS 26.0", "aaa", "log-a");
    let second = report("builder-b", "macOS 26.1", "bbb", "log-b");
    let unbound = aggregate_reports(&first, &second).unwrap();
    let bound = unbound
        .clone()
        .bind_external_inputs(Some("5".repeat(64)), "6".repeat(64));

    assert_ne!(unbound.full_hash().unwrap(), bound.full_hash().unwrap());
    assert_eq!(
        unbound.comparison_manifest_sha256().unwrap(),
        bound.comparison_manifest_sha256().unwrap()
    );
    assert_eq!(
        bound.remote_proof_sha256.as_deref(),
        Some("5".repeat(64).as_str())
    );
    assert_eq!(bound.trust_registry_sha256, "6".repeat(64));
}

#[test]
fn committed_config_and_schema_are_hashed_exactly() {
    let repository = TestRepository::new();
    fs::create_dir(repository.path().join("config")).unwrap();
    fs::write(
        repository.path().join("config/gate.toml"),
        b"schema_version = 1\n",
    )
    .unwrap();
    fs::write(
        repository.path().join("config/gate.schema.json"),
        b"{\"type\":\"object\"}\n",
    )
    .unwrap();
    repository.commit_all("add gate inputs");

    let hashes = hash_committed_inputs(
        repository.path(),
        Path::new("config/gate.toml"),
        Path::new("config/gate.schema.json"),
    )
    .unwrap();

    assert_eq!(hashes.config_sha256.len(), 64);
    assert_eq!(hashes.schema_sha256.len(), 64);
    assert_ne!(hashes.config_sha256, hashes.schema_sha256);
}

#[test]
fn untracked_config_or_schema_is_rejected() {
    let repository = TestRepository::new();
    fs::write(repository.path().join("gate.toml"), b"untracked\n").unwrap();

    let error = hash_committed_inputs(
        repository.path(),
        Path::new("gate.toml"),
        Path::new("tracked.txt"),
    )
    .expect_err("gate inputs must be committed");

    assert_eq!(error.code(), InputHashErrorCode::NotCommitted);
}

fn report(
    builder_id: &str,
    os_version: &str,
    toolchain_hash: &str,
    log_hash: &str,
) -> BuilderReport {
    BuilderReport {
        schema_version: 1,
        stage_id: "stage-0".to_owned(),
        implementation_commit: "95c4cd709bee9d11e2f7fc591d2861427a36cc3a".to_owned(),
        design_tag_object: "32e520a68e6596027fa0dc9673ddb70706474fef".to_owned(),
        design_commit: "412c380054d16f22549c46a59a5fe0617bc60138".to_owned(),
        config_sha256: "1".repeat(64),
        schema_sha256: "2".repeat(64),
        builder_id: builder_id.to_owned(),
        environment: BuilderEnvironment {
            os_version: os_version.to_owned(),
            target_triple: "aarch64-apple-darwin".to_owned(),
            toolchains: BTreeMap::from([(
                "rust".to_owned(),
                ExecutableIdentity {
                    id: "rust".to_owned(),
                    resolved_path: "/trusted/rustc".into(),
                    sha256: "7".repeat(64),
                    version_output: "rustc observed".to_owned(),
                },
            )]),
            toolchain_fingerprint: toolchain_hash.to_owned(),
        },
        resolved_programs: BTreeMap::from([(
            "cargo-test".to_owned(),
            ExecutableIdentity {
                id: "cargo-test".to_owned(),
                resolved_path: "/trusted/cargo".into(),
                sha256: "8".repeat(64),
                version_output: String::new(),
            },
        )]),
        command_log_hashes: BTreeMap::from([("cargo-test".to_owned(), log_hash.to_owned())]),
        artifacts: ArtifactManifest {
            schema_version: 1,
            artifacts: vec![ArtifactRecord {
                logical_name: "alpha-desk".to_owned(),
                relative_path: "dist/alpha-desk".to_owned(),
                kind: "executable".to_owned(),
                size_bytes: 6,
                sha256: "3".repeat(64),
                producer: "cargo-build".to_owned(),
                target_triple: "aarch64-apple-darwin".to_owned(),
                profile: "release".to_owned(),
            }],
        },
        check_results: BTreeMap::from([("cargo-test".to_owned(), GateResult::Pass)]),
    }
}

fn refresh_toolchain_fingerprint(report: &mut BuilderReport) {
    report.environment.toolchain_fingerprint = hex::encode(sha2::Sha256::digest(
        stage_gate::canonical::canonicalize(&report.environment.toolchains).unwrap(),
    ));
}

struct TestRepository {
    temp: TempDir,
}

impl TestRepository {
    fn new() -> Self {
        let temp = TempDir::new().unwrap();
        git(temp.path(), ["init", "-q"]);
        git(temp.path(), ["config", "user.name", "Stage Gate Test"]);
        git(
            temp.path(),
            ["config", "user.email", "stage-gate@example.invalid"],
        );
        fs::write(temp.path().join("tracked.txt"), b"tracked\n").unwrap();
        git(temp.path(), ["add", "tracked.txt"]);
        git(temp.path(), ["commit", "-q", "-m", "initial"]);
        Self { temp }
    }

    fn path(&self) -> &Path {
        self.temp.path()
    }

    fn commit_all(&self, message: &str) {
        git(self.path(), ["add", "."]);
        git(self.path(), ["commit", "-q", "-m", message]);
    }
}

fn git<const N: usize>(repository: &Path, args: [&str; N]) {
    let status = Command::new("/usr/bin/git")
        .arg("-C")
        .arg(repository)
        .args(args)
        .status()
        .unwrap();
    assert!(status.success());
}
