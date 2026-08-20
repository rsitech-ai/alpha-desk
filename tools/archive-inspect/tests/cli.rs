use std::process::Command;

#[test]
fn verify_and_count_reject_empty_archive_with_stable_reason_code() {
    let temporary = tempfile::tempdir().expect("temporary archive");
    let binary = env!("CARGO_BIN_EXE_archive-inspect");

    let verify = Command::new(binary)
        .arg("verify")
        .arg(temporary.path())
        .output()
        .expect("run verify");
    assert_eq!(verify.status.code(), Some(1));
    assert!(verify.stdout.is_empty());
    assert_eq!(
        String::from_utf8(verify.stderr).expect("UTF-8 stderr"),
        "ERROR archive_inspect.empty_archive\n"
    );

    let count = Command::new(binary)
        .arg("count")
        .arg(temporary.path())
        .output()
        .expect("run count");
    assert_eq!(count.status.code(), Some(1));
    assert!(count.stdout.is_empty());
    assert_eq!(
        String::from_utf8(count.stderr).expect("UTF-8 stderr"),
        "ERROR archive_inspect.empty_archive\n"
    );
}

#[test]
fn invalid_command_returns_usage_exit_code() {
    let output = Command::new(env!("CARGO_BIN_EXE_archive-inspect"))
        .arg("unknown")
        .arg(".")
        .output()
        .expect("run archive-inspect");
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(
        String::from_utf8(output.stderr).expect("UTF-8 stderr"),
        "usage: archive-inspect <verify|count|scrub|stats|health> <archive-root>\n       archive-inspect <import-plan|import-publish|import-approve> <archive-root> <chain> <source>\n       archive-inspect <import-backup|import-reclaim> <archive-root> <chain> <source> <backup-root>\n"
    );
}
