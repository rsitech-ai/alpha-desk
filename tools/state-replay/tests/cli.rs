use std::process::Command;

#[test]
fn cli_emits_stable_exit_codes_and_runs_the_fixture_evidence_path() {
    let binary = env!("CARGO_BIN_EXE_state-replay");

    let missing = Command::new(binary).output().expect("missing invocation");
    assert_eq!(missing.status.code(), Some(2));
    assert!(missing.stdout.is_empty());
    assert_eq!(
        String::from_utf8(missing.stderr).expect("UTF-8"),
        "usage: state-replay fixture-e2e --output PATH --blocks N --checkpoint-after N --iterations N\n       state-replay archive-e2e --archive PATH --output PATH --chain ID --start-height N --end-height N --checkpoint-height N --iterations N\n"
    );

    let temporary = tempfile::tempdir().expect("temporary root");
    let output = temporary.path().join("evidence");
    let success = Command::new(binary)
        .args([
            "fixture-e2e",
            "--output",
            output.to_str().expect("UTF-8 output"),
            "--blocks",
            "3",
            "--checkpoint-after",
            "1",
            "--iterations",
            "2",
        ])
        .output()
        .expect("successful invocation");
    assert_eq!(success.status.code(), Some(0));
    assert_eq!(
        String::from_utf8(success.stdout).expect("UTF-8"),
        "PASS evidence_class=synthetic_fixture stage_2_qualified=false live_source_qualified=false\n"
    );
    assert!(success.stderr.is_empty());
    assert!(output.join("report.json").is_file());

    let repeated = Command::new(binary)
        .args([
            "fixture-e2e",
            "--output",
            output.to_str().expect("UTF-8 output"),
            "--blocks",
            "3",
            "--checkpoint-after",
            "1",
            "--iterations",
            "1",
        ])
        .output()
        .expect("repeated invocation");
    assert_eq!(repeated.status.code(), Some(1));
    assert!(repeated.stdout.is_empty());
    assert_eq!(
        String::from_utf8(repeated.stderr).expect("UTF-8"),
        "ERROR state_replay.output_exists\n"
    );
}
