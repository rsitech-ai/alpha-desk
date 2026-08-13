use std::process::Command;

fn golden(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/golden/books")
        .join(name)
}

#[test]
fn replay_prints_a_stable_summary_for_the_snapshot_diff_fixture() {
    let output = Command::new(env!("CARGO_BIN_EXE_book-inspect"))
        .arg("replay")
        .arg(golden("snapshot-diffs.json"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "PASS id=snapshot-diffs health=healthy sequence=12 orders=2\n"
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn replay_reports_red_health_for_the_gap_fixture() {
    let output = Command::new(env!("CARGO_BIN_EXE_book-inspect"))
        .arg("replay")
        .arg(golden("gap.json"))
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "PASS id=gap health=red sequence=5 orders=0\n"
    );
}

#[test]
fn invalid_arguments_fail_without_panicking() {
    let output = Command::new(env!("CARGO_BIN_EXE_book-inspect"))
        .arg("unknown")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("usage:"));
}

#[test]
fn missing_fixture_fails_closed() {
    let output = Command::new(env!("CARGO_BIN_EXE_book-inspect"))
        .args(["replay", "missing-book-fixture.json"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "book fixture could not be read\n"
    );
}
