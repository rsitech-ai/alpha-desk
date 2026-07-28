use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use sha2::Digest as _;
use stage_gate::{
    approvals::GateStatus,
    artifacts::{ArtifactManifest, ArtifactRecord},
    config::{BuilderConfig, BuilderToolConfig},
    gate::{GateReasonCode, comparison_gate_reason, gate_status_for_reasons},
    process::{CapturedOutput, CommandOutcome, CommandSpec},
    reports::{
        BuilderEnvironment, BuilderEvidenceValidation, BuilderIdentity, BuilderReport,
        ComparisonResult, ExecutableEvidence, GateResult, InputHashErrorCode, aggregate_reports,
        builder_ids_are_independent, check_evidence_hash, comparison_satisfies_reproducibility,
        hash_committed_inputs, validate_builder_evidence,
    },
};
use tempfile::TempDir;

#[test]
fn full_report_and_normalized_projection_have_distinct_hash_contracts() {
    let first = report("builder-a", "macOS 26.0", "aaa", "log-a");
    let mut second = report("builder-b", "macOS 26.0", "aaa", "log-a");
    second
        .builder_identity
        .resolved_paths
        .insert("cargo-test".to_owned(), "/different-builder/cargo".into());

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
fn comparison_projection_excludes_exactly_the_builder_identity_envelope() {
    let report = report("builder-a", "macOS 26.0", "aaa", "log-a");
    let full = serde_json::to_value(&report).unwrap();
    let projection: serde_json::Value =
        serde_json::from_slice(&report.comparison_projection().unwrap()).unwrap();
    let full_keys = full
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    let projection_keys = projection
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    let excluded = full_keys
        .into_iter()
        .filter(|key| !projection_keys.contains(key))
        .collect::<Vec<_>>();

    assert_eq!(excluded, vec!["builder_identity"]);
    assert_eq!(
        projection_keys,
        vec![
            "artifacts",
            "check_evidence_hashes",
            "check_evidence_normalization",
            "check_results",
            "config_sha256",
            "design_commit",
            "design_tag_object",
            "environment",
            "implementation_commit",
            "resolved_programs",
            "schema_sha256",
            "schema_version",
            "stage_id",
        ]
    );
}

#[test]
fn every_deterministic_builder_report_field_remains_in_the_projection() {
    let baseline = report("builder-a", "macOS 26.0", "aaa", "log-a");
    let baseline_projection = baseline.projection_hash().unwrap();
    let mut mutations = Vec::new();

    let mut mutated = baseline.clone();
    mutated.schema_version = 2;
    mutations.push(mutated);
    let mut mutated = baseline.clone();
    mutated.stage_id = "stage-1".to_owned();
    mutations.push(mutated);
    let mut mutated = baseline.clone();
    mutated.implementation_commit = "0".repeat(40);
    mutations.push(mutated);
    let mut mutated = baseline.clone();
    mutated.design_tag_object = "0".repeat(40);
    mutations.push(mutated);
    let mut mutated = baseline.clone();
    mutated.design_commit = "0".repeat(40);
    mutations.push(mutated);
    let mut mutated = baseline.clone();
    mutated.config_sha256 = "0".repeat(64);
    mutations.push(mutated);
    let mut mutated = baseline.clone();
    mutated.schema_sha256 = "0".repeat(64);
    mutations.push(mutated);
    let mut mutated = baseline.clone();
    mutated.check_evidence_normalization = "different-normalizer".to_owned();
    mutations.push(mutated);
    let mut mutated = baseline.clone();
    mutated.environment.os_version = "normalized-linux-6.8".to_owned();
    mutations.push(mutated);
    let mut mutated = baseline.clone();
    mutated
        .resolved_programs
        .get_mut("cargo-test")
        .unwrap()
        .sha256 = "0".repeat(64);
    mutations.push(mutated);
    let mut mutated = baseline.clone();
    mutated
        .check_evidence_hashes
        .insert("cargo-test".to_owned(), "0".repeat(64));
    mutations.push(mutated);
    let mut mutated = baseline.clone();
    mutated.artifacts.artifacts[0].sha256 = "0".repeat(64);
    mutations.push(mutated);
    let mut mutated = baseline;
    mutated
        .check_results
        .insert("cargo-test".to_owned(), GateResult::Fail);
    mutations.push(mutated);

    for mutation in mutations {
        assert_ne!(mutation.projection_hash().unwrap(), baseline_projection);
    }
}

#[test]
fn semantic_check_evidence_normalizes_only_benign_builder_volatility() {
    let warm_spec = semantic_spec("/builder-a/repository", "/builder-a/home");
    let cold_spec = semantic_spec("/builder-b/repository", "/builder-b/home");
    let warm = outcome(
        concat!(
            "    Finished `test` profile [unoptimized] target(s) in 0.12s\n",
            "     Running unittests /builder-a/repository/target/debug/deps/core-a1\n",
            "compose project alpha-desk-stage0-ab12cd34 ready\n",
            "test tests::orders_are_sorted ... ok\n",
            "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; finished in 0.01s\n",
            "Executed 1 tests, with 0 failures (0 unexpected) in 0.001 (0.002) seconds\n",
        ),
        "",
    );
    let cold = outcome(
        concat!(
            "   Compiling serde v1.0.0\n",
            "    Checking alpha-core v0.1.0 (/builder-b/repository/crates/core)\n",
            "[1/4] Compiling AlphaCore Source.swift\n",
            "    Finished `test` profile [unoptimized] target(s) in 9.87s\n",
            "     Running unittests /builder-b/repository/target/debug/deps/core-a1\n",
            "compose project alpha-desk-stage0-ef56gh78 ready\n",
            "test tests::orders_are_sorted ... ok\n",
            "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; finished in 0.44s\n",
            "Executed 1 tests, with 0 failures (0 unexpected) in 0.777 (0.888) seconds\n",
        ),
        "",
    );

    assert_eq!(
        check_evidence_hash(
            "quality",
            &warm_spec,
            &"a".repeat(64),
            Path::new("/builder-a/repository"),
            &warm,
        )
        .unwrap(),
        check_evidence_hash(
            "quality",
            &cold_spec,
            &"a".repeat(64),
            Path::new("/builder-b/repository"),
            &cold,
        )
        .unwrap(),
        "cold-cache build progress, builder paths, and elapsed timings are non-semantic"
    );
}

#[test]
fn semantic_check_evidence_binds_results_diagnostics_command_and_truncation() {
    let check = semantic_spec("/repo", "/home/builder");
    let baseline = outcome(
        concat!(
            "test tests::orders_are_sorted ... ok\n",
            "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; finished in 0.01s\n",
        ),
        "",
    );
    let expected = check_evidence_hash(
        "quality",
        &check,
        &"a".repeat(64),
        Path::new("/repo"),
        &baseline,
    )
    .unwrap();

    for (scenario, stdout, stderr) in [
        (
            "test-name",
            "test tests::orders_are_filtered ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; finished in 0.01s\n",
            "",
        ),
        (
            "test-count",
            "test tests::orders_are_sorted ... ok\ntest result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; finished in 0.01s\n",
            "",
        ),
        (
            "test-result",
            "test tests::orders_are_sorted ... FAILED\ntest result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; finished in 0.01s\n",
            "",
        ),
        (
            "warning",
            "test tests::orders_are_sorted ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; finished in 0.01s\n",
            "warning: fallback parser was used in 0.30s at /repo/src/lib.rs\n",
        ),
        (
            "skip",
            "test tests::orders_are_sorted ... ignored\ntest result: ok. 0 passed; 0 failed; 1 ignored; 0 measured; finished in 0.01s\n",
            "",
        ),
        (
            "semantic-running-near-miss",
            "Running invariant suite phase A\ntest tests::orders_are_sorted ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; finished in 0.01s\n",
            "",
        ),
        (
            "cargo-progress-near-miss",
            "Compiling cache v1.0.0 extra semantic\ntest tests::orders_are_sorted ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; finished in 0.01s\n",
            "",
        ),
        (
            "cargo-parenthetical-near-miss",
            "Compiling cache v1.0.0 (semantic)\ntest tests::orders_are_sorted ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; finished in 0.01s\n",
            "",
        ),
        (
            "swift-progress-near-miss",
            "Build complete! (not-a-duration)\ntest tests::orders_are_sorted ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; finished in 0.01s\n",
            "",
        ),
        (
            "summary-grammar-near-miss",
            "test tests::orders_are_sorted ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; finished after 0.01s\n",
            "",
        ),
        (
            "retained-whitespace",
            "test  tests::orders_are_sorted ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; finished in 0.01s\n",
            "",
        ),
        (
            "embedded-project-prefix-near-miss",
            "xalpha-desk-stage0-ab12cd34\ntest tests::orders_are_sorted ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; finished in 0.01s\n",
            "",
        ),
    ] {
        assert_ne!(
            check_evidence_hash(
                "quality",
                &check,
                &"a".repeat(64),
                Path::new("/repo"),
                &outcome(stdout, stderr),
            )
            .unwrap(),
            expected,
            "{scenario} must change semantic evidence"
        );
    }
    let no_trailing_newline = outcome(
        "test tests::orders_are_sorted ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; finished in 0.01s",
        "",
    );
    assert_ne!(
        check_evidence_hash(
            "quality",
            &check,
            &"a".repeat(64),
            Path::new("/repo"),
            &no_trailing_newline,
        )
        .unwrap(),
        expected,
        "line framing is semantic"
    );

    let mut changed_exit = baseline.clone();
    changed_exit.success = false;
    changed_exit.exit_code = Some(9);
    assert_ne!(
        check_evidence_hash(
            "quality",
            &check,
            &"a".repeat(64),
            Path::new("/repo"),
            &changed_exit,
        )
        .unwrap(),
        expected
    );
    assert_ne!(
        check_evidence_hash(
            "different-check",
            &check,
            &"a".repeat(64),
            Path::new("/repo"),
            &baseline,
        )
        .unwrap(),
        expected
    );
    assert_ne!(
        check_evidence_hash(
            "quality",
            &check,
            &"b".repeat(64),
            Path::new("/repo"),
            &baseline,
        )
        .unwrap(),
        expected
    );
    let mut changed_program = check.clone();
    changed_program.program = PathBuf::from("/different/toolchain/bin/cargo");
    assert_ne!(
        check_evidence_hash(
            "quality",
            &changed_program,
            &"a".repeat(64),
            Path::new("/repo"),
            &baseline,
        )
        .unwrap(),
        expected
    );
    let mut changed_cwd = check.clone();
    changed_cwd.cwd = PathBuf::from("/repo/crates/core");
    assert_ne!(
        check_evidence_hash(
            "quality",
            &changed_cwd,
            &"a".repeat(64),
            Path::new("/repo"),
            &baseline,
        )
        .unwrap(),
        expected
    );
    let mut changed_args = check.clone();
    changed_args.args.push("--ignored".into());
    assert_ne!(
        check_evidence_hash(
            "quality",
            &changed_args,
            &"a".repeat(64),
            Path::new("/repo"),
            &baseline,
        )
        .unwrap(),
        expected
    );
    let mut changed_env = check.clone();
    changed_env
        .env
        .push(("RUSTFLAGS".to_owned(), "-Dwarnings".to_owned()));
    assert_ne!(
        check_evidence_hash(
            "quality",
            &changed_env,
            &"a".repeat(64),
            Path::new("/repo"),
            &baseline,
        )
        .unwrap(),
        expected
    );
    let mut changed_timeout = check.clone();
    changed_timeout.timeout += std::time::Duration::from_secs(1);
    assert_ne!(
        check_evidence_hash(
            "quality",
            &changed_timeout,
            &"a".repeat(64),
            Path::new("/repo"),
            &baseline,
        )
        .unwrap(),
        expected
    );
    let mut changed_grace = check.clone();
    changed_grace.termination_grace += std::time::Duration::from_secs(1);
    assert_ne!(
        check_evidence_hash(
            "quality",
            &changed_grace,
            &"a".repeat(64),
            Path::new("/repo"),
            &baseline,
        )
        .unwrap(),
        expected
    );
    let mut truncated = baseline;
    truncated.stdout.truncated = true;
    assert_ne!(
        check_evidence_hash(
            "quality",
            &check,
            &"a".repeat(64),
            Path::new("/repo"),
            &truncated,
        )
        .unwrap(),
        expected
    );
}

#[test]
fn present_invalid_external_evidence_is_failure_while_missing_evidence_is_blocked() {
    for reason in [
        GateReasonCode::SecondBuilderEvidenceInvalid,
        GateReasonCode::SecondBuilderIdentityInvalid,
        GateReasonCode::SecondBuilderVersionMismatch,
        GateReasonCode::SecondBuilderNotIdentical,
        GateReasonCode::SecondBuilderMismatch,
        GateReasonCode::ApprovalEvidenceInvalid,
        GateReasonCode::RequiredGithubChecksInvalid,
    ] {
        assert_eq!(
            gate_status_for_reasons(&[reason]),
            GateStatus::Fail,
            "{reason:?}"
        );
    }
    for reason in [
        GateReasonCode::SecondBuilderUnavailable,
        GateReasonCode::PlatformDataApprovalMissing,
        GateReasonCode::IndependentReviewMissing,
        GateReasonCode::RequiredGithubChecksUnavailable,
    ] {
        assert_eq!(
            gate_status_for_reasons(&[reason]),
            GateStatus::Blocked,
            "{reason:?}"
        );
    }
}

#[test]
fn aggregate_retains_both_full_hashes_and_projection_hashes() {
    let first = report("builder-a", "macOS 26.0", "aaa", "log-a");
    let second = report("builder-b", "macOS 26.0", "aaa", "log-a");

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
            ExecutableEvidence {
                id: "rustc".to_owned(),
                sha256: "7".repeat(64),
                version_output: concat!("rustc 1.97.1 (fixture)\n", "host: aarch64-apple-darwin")
                    .to_owned(),
            },
        ),
        (
            "swift".to_owned(),
            ExecutableEvidence {
                id: "swift".to_owned(),
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

fn semantic_spec(repository: &str, home: &str) -> CommandSpec {
    CommandSpec {
        program: Path::new(home).join(".cargo/bin/cargo"),
        evidence_program: None,
        arg0: None,
        args: vec!["+1.97.1".into(), "test".into()],
        cwd: PathBuf::from(repository),
        env: vec![
            ("HOME".to_owned(), home.to_owned()),
            ("CARGO_HOME".to_owned(), format!("{home}/.cargo")),
            (
                "PATH".to_owned(),
                format!("{home}/.cargo/bin:/usr/bin:/bin"),
            ),
        ],
        timeout: std::time::Duration::from_secs(60),
        termination_grace: std::time::Duration::from_secs(2),
    }
}

fn outcome(stdout: &str, stderr: &str) -> CommandOutcome {
    CommandOutcome {
        success: true,
        exit_code: Some(0),
        stdout: CapturedOutput {
            text: stdout.to_owned(),
            total_bytes: stdout.len(),
            truncated: false,
        },
        stderr: CapturedOutput {
            text: stderr.to_owned(),
            total_bytes: stderr.len(),
            truncated: false,
        },
        elapsed: std::time::Duration::from_secs(1),
    }
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
        check_evidence_normalization: "stage-gate-semantic-v1".to_owned(),
        builder_identity: BuilderIdentity {
            builder_id: builder_id.to_owned(),
            signer_role: "local".to_owned(),
            signer_fingerprint: String::new(),
            resolved_paths: BTreeMap::from([
                ("rust".to_owned(), "/trusted/rustc".into()),
                ("cargo-test".to_owned(), "/trusted/cargo".into()),
            ]),
        },
        environment: BuilderEnvironment {
            os_version: os_version.to_owned(),
            target_triple: "aarch64-apple-darwin".to_owned(),
            toolchains: BTreeMap::from([(
                "rust".to_owned(),
                ExecutableEvidence {
                    id: "rust".to_owned(),
                    sha256: "7".repeat(64),
                    version_output: "rustc observed".to_owned(),
                },
            )]),
            toolchain_fingerprint: toolchain_hash.to_owned(),
        },
        resolved_programs: BTreeMap::from([(
            "cargo-test".to_owned(),
            ExecutableEvidence {
                id: "cargo-test".to_owned(),
                sha256: "8".repeat(64),
                version_output: String::new(),
            },
        )]),
        check_evidence_hashes: BTreeMap::from([("cargo-test".to_owned(), log_hash.to_owned())]),
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
