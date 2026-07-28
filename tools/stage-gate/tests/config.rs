use stage_gate::config::{ConfigErrorCode, GateConfig};

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
app_source = "rsitech-ai/alpha-desk"
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
fn valid_configuration_is_accepted() {
    GateConfig::parse(VALID_CONFIG).expect("valid configuration must parse");
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
    let source = VALID_CONFIG.replace(
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
    );

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
fn builder_report_output_must_stay_under_the_ignored_gate_root() {
    let source = VALID_CONFIG.replace(
        "builder_report_output_path = \"target/stage-gates/stage-0.builder.json\"",
        "builder_report_output_path = \"stage-0.builder.json\"",
    );

    let error =
        GateConfig::parse(&source).expect_err("builder report output must remain contained");

    assert_eq!(error.code(), ConfigErrorCode::UnsafeOutput);
}
