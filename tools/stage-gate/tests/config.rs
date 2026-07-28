use stage_gate::config::{ConfigErrorCode, GateConfig};

#[test]
fn published_schema_covers_signed_provenance_and_check_termination_fields() {
    let schema_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../config/stage-gates/schema-v1.json");
    let schema: serde_json::Value =
        serde_json::from_slice(&std::fs::read(schema_path).unwrap()).unwrap();

    for field in [
        "second_builder_report_path",
        "second_builder_signature_path",
        "signer_role",
    ] {
        assert!(
            schema["properties"]["comparison"]["required"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value == field)
        );
    }
    for field in [
        "proof_path",
        "signature_path",
        "signer_role",
        "repository",
        "repository_id",
        "repository_owner_id",
        "workflow",
        "workflow_ref",
        "trigger_workflow_id",
        "trigger_workflow_name",
        "trigger_workflow_path",
        "event_name",
        "git_ref",
        "signing_check_name",
        "required_checks",
    ] {
        assert!(
            schema["properties"]["remote"]["required"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value == field)
        );
    }
    assert_eq!(
        schema["$defs"]["check"]["properties"]["termination_grace_seconds"]["minimum"],
        1
    );
    assert_eq!(
        schema["properties"]["output_root"]["const"],
        "target/stage-gates"
    );
    assert_eq!(
        schema["properties"]["builder_report_output_path"]["const"],
        "target/stage-gates/stage-0.builder.json"
    );
    assert_eq!(
        schema["properties"]["approvals"]["properties"]["required_roles"]["minItems"],
        2
    );
    assert_eq!(
        schema["properties"]["approvals"]["properties"]["required_roles"]["maxItems"],
        2
    );
    assert_eq!(
        schema["properties"]["approvals"]["properties"]["required_roles"]["uniqueItems"],
        true
    );
    assert_eq!(
        schema["properties"]["approvals"]["properties"]["evidence"]["maxItems"],
        2
    );
    assert!(
        schema["properties"]["remote"]["properties"]
            .get("workflow_sha")
            .is_none()
    );
    assert!(
        schema["properties"]["remote"]["properties"]
            .get("trigger_workflow_sha")
            .is_none()
    );
    assert_eq!(
        schema["properties"]["remote"]["properties"]["workflow"]["const"],
        ".github/workflows/stage-0-evidence.yml"
    );
    assert_eq!(
        schema["properties"]["remote"]["properties"]["trigger_workflow_name"]["const"],
        "CI"
    );
    assert_eq!(
        schema["properties"]["remote"]["properties"]["trigger_workflow_path"]["const"],
        ".github/workflows/ci.yml"
    );
    assert!(
        schema["properties"]["remote"]["properties"]["workflow_ref"]["pattern"]
            .as_str()
            .unwrap()
            .ends_with("@refs/heads/main$")
    );
}

const VALID_CONFIG: &str = r#"
schema_version = 1
stage_id = "stage-0-foundations"
schema_path = "config/stage-gates/schema-v1.json"
output_root = "target/stage-gates"
builder_report_output_path = "target/stage-gates/stage-0.builder.json"
whole_gate_timeout_seconds = 3600
max_output_bytes = 16384
allowed_programs = ["just", "gpgv"]
program_roots = ["/usr/bin", "/bin"]

[design]
tag = "design-approved-v1.0.0"
object = "32e520a68e6596027fa0dc9673ddb70706474fef"
commit = "412c380054d16f22549c46a59a5fe0617bc60138"

[comparison]
second_builder_report_path = "target/stage-gates/inputs/builder-b.json"
second_builder_signature_path = "target/stage-gates/inputs/builder-b.json.asc"
signer_role = "builder-b"

[builder]
target_tool = "just"

[[builder.tools]]
id = "just"
program = "just"
args = ["--version"]
expected_output_contains = "just"

[approvals]
policy_path = "config/stage-gates/trusted-reviewers.toml"
required_roles = ["platform-data", "independent"]
gpgv_program = "gpgv"
known_limitations = []

[[approvals.evidence]]
role = "platform-data"
statement_path = "target/stage-gates/inputs/platform-data.json"
signature_path = "target/stage-gates/inputs/platform-data.json.asc"

[[approvals.evidence]]
role = "independent"
statement_path = "target/stage-gates/inputs/independent.json"
signature_path = "target/stage-gates/inputs/independent.json.asc"

[remote]
proof_path = "target/stage-gates/inputs/github-required-checks.json"
signature_path = "target/stage-gates/inputs/github-required-checks.json.asc"
signer_role = "github-ci"
repository = "s1korrrr/alpha-desk"
repository_id = 1311268858
repository_owner_id = 24563931
workflow = ".github/workflows/stage-0-evidence.yml"
workflow_ref = "s1korrrr/alpha-desk/.github/workflows/stage-0-evidence.yml@refs/heads/main"
trigger_workflow_id = 321251517
trigger_workflow_name = "CI"
trigger_workflow_path = ".github/workflows/ci.yml"
event_name = "push"
git_ref = "refs/heads/main"
signing_check_name = "Stage 0 evidence signing"
required_checks = ["CI / Rust quality"]

[[artifacts]]
id = "cargo-lock"
path = "Cargo.lock"
kind = "input"
producer = "repository"
target_triple = "platform-independent"
profile = "source"

[[checks]]
id = "quality"
program = "just"
args = ["quality"]
cwd = "."
timeout_seconds = 60
"#;

#[test]
fn duplicate_check_ids_are_rejected() {
    let source = format!(
        "{VALID_CONFIG}\n{}",
        r#"
[[checks]]
id = "quality"
program = "just"
args = ["quality"]
cwd = "."
timeout_seconds = 60
"#
    );

    let error = GateConfig::parse(&source).expect_err("duplicate check IDs must fail closed");

    assert_eq!(error.code(), ConfigErrorCode::DuplicateCheckId);
}

#[test]
fn signed_provenance_configuration_is_accepted() {
    GateConfig::parse(VALID_CONFIG)
        .expect("Builder B and GitHub proof signatures and immutable identity must parse");
}

#[test]
fn approval_roles_and_evidence_are_exactly_the_two_independent_stage_zero_roles() {
    for source in [
        VALID_CONFIG.replace(
            "required_roles = [\"platform-data\", \"independent\"]",
            "required_roles = [\"platform-data\", \"independent\", \"observer\"]",
        ),
        VALID_CONFIG.replace(
            "role = \"independent\"\nstatement_path",
            "role = \"observer\"\nstatement_path",
        ),
    ] {
        let error = GateConfig::parse(&source).expect_err("extra approval roles must fail closed");
        assert_eq!(error.code(), ConfigErrorCode::InvalidValue);
    }
}

#[test]
fn unsigned_external_provenance_configuration_is_rejected() {
    let source = VALID_CONFIG
        .replace(
            concat!(
                "second_builder_signature_path = ",
                "\"target/stage-gates/inputs/builder-b.json.asc\"\n",
                "signer_role = \"builder-b\"\n",
            ),
            "",
        )
        .replace(
            concat!(
                "signature_path = ",
                "\"target/stage-gates/inputs/github-required-checks.json.asc\"\n",
                "signer_role = \"github-ci\"\n",
                "repository = \"s1korrrr/alpha-desk\"\n",
                "repository_id = 1311268858\n",
                "repository_owner_id = 24563931\n",
                "workflow = \".github/workflows/stage-0-evidence.yml\"\n",
                concat!(
                    "workflow_ref = ",
                    "\"s1korrrr/alpha-desk/.github/workflows/",
                    "stage-0-evidence.yml@refs/heads/main\"\n",
                ),
            ),
            "",
        );

    let error =
        GateConfig::parse(&source).expect_err("unsigned Builder B and GitHub proof must fail");

    assert_eq!(error.code(), ConfigErrorCode::InvalidToml);
}

#[test]
fn static_signing_workflow_sha_is_rejected_because_runtime_sha_binds_the_implementation() {
    let source = VALID_CONFIG.replace(
        "trigger_workflow_id = 321251517",
        concat!(
            "workflow_sha = \"95c4cd709bee9d11e2f7fc591d2861427a36cc3a\"\n",
            "trigger_workflow_id = 321251517"
        ),
    );

    assert!(
        GateConfig::parse(&source).is_err(),
        "a static source SHA can drift from the workflow_run execution context"
    );
}

#[test]
fn remote_workflow_names_paths_and_refs_are_the_fixed_stage_zero_contract() {
    for source in [
        VALID_CONFIG.replace(
            "workflow = \".github/workflows/stage-0-evidence.yml\"",
            "workflow = \".github/workflows/other.yml\"",
        ),
        VALID_CONFIG.replace(
            "trigger_workflow_name = \"CI\"",
            "trigger_workflow_name = \"Other\"",
        ),
        VALID_CONFIG.replace(
            "trigger_workflow_path = \".github/workflows/ci.yml\"",
            "trigger_workflow_path = \".github/workflows/other.yml\"",
        ),
        VALID_CONFIG.replace("@refs/heads/main\"", "@refs/heads/release\""),
    ] {
        let error =
            GateConfig::parse(&source).expect_err("workflow identity drift must fail closed");
        assert_eq!(error.code(), ConfigErrorCode::InvalidValue);
    }
}

#[test]
fn an_empty_command_is_rejected() {
    let source = VALID_CONFIG.replace(
        "id = \"quality\"\nprogram = \"just\"",
        "id = \"quality\"\nprogram = \"\"",
    );

    let error = GateConfig::parse(&source).expect_err("an empty program must fail closed");

    assert_eq!(error.code(), ConfigErrorCode::MissingCommand);
}

#[test]
fn an_empty_artifact_list_is_rejected() {
    let source = VALID_CONFIG
        .replace(
            concat!(
                "[[artifacts]]\n",
                "id = \"cargo-lock\"\n",
                "path = \"Cargo.lock\"\n",
                "kind = \"input\"\n",
                "producer = \"repository\"\n",
                "target_triple = \"platform-independent\"\n",
                "profile = \"source\"\n",
            ),
            "",
        )
        .replace("[design]", "artifacts = []\n\n[design]");

    let error = GateConfig::parse(&source).expect_err("missing artifacts must fail closed");

    assert_eq!(error.code(), ConfigErrorCode::MissingArtifact);
}

#[test]
fn malformed_expected_sha256_is_rejected() {
    let source = VALID_CONFIG.replace(
        "kind = \"input\"",
        "kind = \"input\"\nexpected_sha256 = \"not-a-sha256\"",
    );

    let error = GateConfig::parse(&source).expect_err("malformed SHA-256 must fail closed");

    assert_eq!(error.code(), ConfigErrorCode::MalformedHash);
}

#[test]
fn shell_programs_are_rejected_even_when_allowlisted() {
    let source = VALID_CONFIG
        .replace(
            "allowed_programs = [\"just\"]",
            "allowed_programs = [\"sh\"]",
        )
        .replace("program = \"just\"", "program = \"sh\"");

    let error = GateConfig::parse(&source).expect_err("shell execution must fail closed");

    assert_eq!(error.code(), ConfigErrorCode::UnsafeProgram);
}

#[test]
fn approval_verifier_must_be_allowlisted() {
    let source = VALID_CONFIG.replace(
        "allowed_programs = [\"just\", \"gpgv\"]",
        "allowed_programs = [\"just\"]",
    );

    let error = GateConfig::parse(&source).expect_err("gpgv must be explicitly allowlisted");

    assert_eq!(error.code(), ConfigErrorCode::UnsafeProgram);
}

#[test]
fn control_characters_in_arguments_are_rejected() {
    let source = VALID_CONFIG.replace("args = [\"quality\"]", r#"args = ["quality\nnext"]"#);

    let error = GateConfig::parse(&source).expect_err("control characters must fail closed");

    assert_eq!(error.code(), ConfigErrorCode::UnsafeArgument);
}

#[test]
fn parent_relative_working_directories_are_rejected() {
    let source = VALID_CONFIG.replace("cwd = \".\"", "cwd = \"../outside\"");

    let error = GateConfig::parse(&source).expect_err("outside cwd must fail closed");

    assert_eq!(error.code(), ConfigErrorCode::UnsafeWorkingDirectory);
}

#[test]
fn output_roots_outside_target_stage_gates_are_rejected() {
    let source = VALID_CONFIG.replace(
        "output_root = \"target/stage-gates\"",
        "output_root = \"docs/stage-gates\"",
    );

    let error = GateConfig::parse(&source).expect_err("tracked output root must fail closed");

    assert_eq!(error.code(), ConfigErrorCode::UnsafeOutput);
}

#[test]
fn output_root_is_the_single_fixed_stage_gate_root() {
    let source = VALID_CONFIG.replace(
        "output_root = \"target/stage-gates\"",
        "output_root = \"target/stage-gates/custom\"",
    );

    let error = GateConfig::parse(&source).expect_err("nested output roots must fail closed");

    assert_eq!(error.code(), ConfigErrorCode::UnsafeOutput);
}

#[test]
fn builder_report_output_is_the_single_fixed_builder_path() {
    let source = VALID_CONFIG.replace(
        "builder_report_output_path = \"target/stage-gates/stage-0.builder.json\"",
        "builder_report_output_path = \"target/stage-gates/custom-builder.json\"",
    );

    let error = GateConfig::parse(&source).expect_err("custom builder outputs must fail closed");

    assert_eq!(error.code(), ConfigErrorCode::UnsafeOutput);
}
