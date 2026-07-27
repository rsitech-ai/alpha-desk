use std::fs;
use std::process::{Command, Output};
use test_fixtures::FixtureManifest;

const CONTROL_CHARACTERS: [char; 7] = ['\n', '\r', '\t', '\u{1b}', '\u{1}', '\u{7f}', '\u{85}'];

fn write_pair(root: &std::path::Path, id: &str) {
    fs::write(
        root.join(format!("blocks/{id}.json")),
        format!("{{\"schema\":\"hl.source.fixture.v1\",\"id\":\"{id}\"}}\n"),
    )
    .unwrap();
    fs::write(
        root.join(format!("expected/{id}.canonical.json")),
        format!("{{\"schema_version\":\"1.0.0\",\"id\":\"{id}\"}}\n"),
    )
    .unwrap();
}

fn valid_manifest_root() -> tempfile::TempDir {
    let temporary = tempfile::tempdir().unwrap();
    fs::create_dir_all(temporary.path().join("blocks")).unwrap();
    fs::create_dir_all(temporary.path().join("expected")).unwrap();
    write_pair(temporary.path(), "safe");
    FixtureManifest::generate(temporary.path())
        .unwrap()
        .write_atomic(temporary.path())
        .unwrap();
    temporary
}

fn verify(root: &std::path::Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_fixture-inspect"))
        .arg("verify")
        .arg(root.join("manifest.toml"))
        .output()
        .unwrap()
}

fn assert_safe_cli_rejection(output: &Output, case: &str) {
    assert!(!output.status.success(), "{case}");
    assert!(output.stdout.is_empty(), "{case}: {:?}", output.stdout);
    assert!(
        output.stderr.ends_with(b"\n"),
        "{case}: {:?}",
        output.stderr
    );
    assert_eq!(
        output.stderr.iter().filter(|byte| **byte == b'\n').count(),
        1,
        "{case}: {:?}",
        output.stderr
    );
    let diagnostic = std::str::from_utf8(&output.stderr[..output.stderr.len() - 1]).unwrap();
    assert!(
        diagnostic.chars().all(|character| !character.is_control()),
        "{case}: {:?}",
        output.stderr
    );
}

#[test]
fn generate_and_verify_print_a_stable_hash_and_sorted_summary() {
    let temporary = tempfile::tempdir().unwrap();
    fs::create_dir_all(temporary.path().join("blocks")).unwrap();
    fs::create_dir_all(temporary.path().join("expected")).unwrap();
    write_pair(temporary.path(), "z-last");
    write_pair(temporary.path(), "a-first");

    let generated = Command::new(env!("CARGO_BIN_EXE_fixture-inspect"))
        .args(["generate-manifest", "--root"])
        .arg(temporary.path())
        .output()
        .unwrap();
    assert!(
        generated.status.success(),
        "{}",
        String::from_utf8_lossy(&generated.stderr)
    );
    let generated_stdout = String::from_utf8(generated.stdout).unwrap();
    assert!(generated_stdout.starts_with("manifest-sha256:"));
    assert_eq!(generated_stdout.trim().len(), "manifest-sha256:".len() + 64);

    let verified = Command::new(env!("CARGO_BIN_EXE_fixture-inspect"))
        .arg("verify")
        .arg(temporary.path().join("manifest.toml"))
        .output()
        .unwrap();
    assert!(
        verified.status.success(),
        "{}",
        String::from_utf8_lossy(&verified.stderr)
    );
    assert_eq!(
        String::from_utf8(verified.stdout).unwrap(),
        "fixture:a-first:ok\nfixture:z-last:ok\nmanifest:ok\n"
    );

    let basename_verified = Command::new(env!("CARGO_BIN_EXE_fixture-inspect"))
        .current_dir(temporary.path())
        .args(["verify", "manifest.toml"])
        .output()
        .unwrap();
    assert!(
        basename_verified.status.success(),
        "{}",
        String::from_utf8_lossy(&basename_verified.stderr)
    );
    assert_eq!(
        String::from_utf8(basename_verified.stdout).unwrap(),
        "fixture:a-first:ok\nfixture:z-last:ok\nmanifest:ok\n"
    );
}

#[test]
fn invalid_arguments_fail_without_panicking() {
    let output = Command::new(env!("CARGO_BIN_EXE_fixture-inspect"))
        .arg("unknown")
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("usage:"));
}

#[cfg(unix)]
#[test]
fn non_utf8_arguments_have_an_escaped_single_line_diagnostic() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let argument = OsString::from_vec(vec![b'v', 0xff, b'\n', 0x1b]);
    let output = Command::new(env!("CARGO_BIN_EXE_fixture-inspect"))
        .arg(argument)
        .output()
        .unwrap();

    assert_safe_cli_rejection(&output, "non-UTF-8 CLI argument");
}

#[test]
fn rejected_cli_manifest_paths_cannot_inject_diagnostic_controls() {
    let temporary = tempfile::tempdir().unwrap();
    for control in CONTROL_CHARACTERS {
        let output = Command::new(env!("CARGO_BIN_EXE_fixture-inspect"))
            .arg("verify")
            .arg(temporary.path().join(format!("missing{control}.toml")))
            .output()
            .unwrap();

        assert_safe_cli_rejection(&output, &format!("CLI manifest path {control:?}"));
    }
}

#[test]
fn unsafe_ids_cannot_inject_cli_status_or_terminal_controls() {
    for control in CONTROL_CHARACTERS {
        let temporary = valid_manifest_root();
        let mut manifest = FixtureManifest::load(temporary.path().join("manifest.toml")).unwrap();
        manifest.fixture[0].id = format!("evil{control}status");
        manifest.write_atomic(temporary.path()).unwrap();

        let output = verify(temporary.path());

        assert_safe_cli_rejection(&output, &format!("unsafe id {control:?}"));
    }
}

#[test]
fn rejected_manifest_paths_cannot_inject_diagnostic_controls() {
    for control in CONTROL_CHARACTERS {
        let temporary = valid_manifest_root();
        let mut manifest = FixtureManifest::load(temporary.path().join("manifest.toml")).unwrap();
        manifest.fixture[0].source_path = format!("blocks/{control}/../safe.json");
        manifest.write_atomic(temporary.path()).unwrap();

        let output = verify(temporary.path());

        assert_safe_cli_rejection(&output, &format!("path {control:?}"));
    }
}

#[test]
fn rejected_manifest_digests_cannot_inject_diagnostic_controls() {
    for control in CONTROL_CHARACTERS {
        let temporary = valid_manifest_root();
        let mut manifest = FixtureManifest::load(temporary.path().join("manifest.toml")).unwrap();
        manifest.fixture[0].source_sha256 = format!("bad{control}digest");
        manifest.write_atomic(temporary.path()).unwrap();

        let output = verify(temporary.path());

        assert_safe_cli_rejection(&output, &format!("digest {control:?}"));
    }
}

#[test]
fn rejected_manifest_schemas_cannot_inject_diagnostic_controls() {
    for control in CONTROL_CHARACTERS {
        let temporary = valid_manifest_root();
        let mut manifest = FixtureManifest::load(temporary.path().join("manifest.toml")).unwrap();
        manifest.fixture[0].source_schema = format!("wrong{control}schema");
        manifest.write_atomic(temporary.path()).unwrap();

        let output = verify(temporary.path());

        assert_safe_cli_rejection(&output, &format!("schema {control:?}"));
    }
}
