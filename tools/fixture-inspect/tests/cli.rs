use std::fs;
use std::process::Command;
use test_fixtures::FixtureManifest;

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

#[test]
fn unsafe_ids_cannot_inject_cli_status_or_terminal_controls() {
    for id in ["evil\nmanifest:ok", "evil\rcarriage", "evil\u{1b}escape"] {
        let temporary = tempfile::tempdir().unwrap();
        fs::create_dir_all(temporary.path().join("blocks")).unwrap();
        fs::create_dir_all(temporary.path().join("expected")).unwrap();
        write_pair(temporary.path(), "safe");
        let manifest = FixtureManifest::generate(temporary.path()).unwrap();
        manifest.write_atomic(temporary.path()).unwrap();
        let mut manifest = FixtureManifest::load(temporary.path().join("manifest.toml")).unwrap();
        manifest.fixture[0].id = id.to_owned();
        manifest.write_atomic(temporary.path()).unwrap();

        let output = Command::new(env!("CARGO_BIN_EXE_fixture-inspect"))
            .arg("verify")
            .arg(temporary.path().join("manifest.toml"))
            .output()
            .unwrap();

        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert_eq!(
            output.stderr.iter().filter(|byte| **byte == b'\n').count(),
            1
        );
        assert!(!output.stderr.contains(&b'\r'));
        assert!(!output.stderr.contains(&0x1b));
    }
}
