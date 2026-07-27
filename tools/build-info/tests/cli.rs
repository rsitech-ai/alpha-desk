use std::process::Command;

#[test]
fn print_emits_one_fixed_order_json_line() {
    let output = Command::new(env!("CARGO_BIN_EXE_build-info"))
        .arg("print")
        .output()
        .expect("build-info must run");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).expect("stdout must be UTF-8");
    assert_eq!(stdout.lines().count(), 1);
    assert!(stdout.ends_with('\n'));

    let ordered_fields = [
        "\"git_sha\"",
        "\"dirty\"",
        "\"rustc_version\"",
        "\"target_triple\"",
        "\"build_epoch\"",
        "\"reproducible\"",
        "\"schema_fingerprint\"",
        "\"cargo_lock_sha256\"",
    ];
    let mut cursor = 0;
    for field in ordered_fields {
        let relative = stdout[cursor..]
            .find(field)
            .expect("all provenance fields must be present");
        cursor += relative + field.len();
    }
    assert!(!stdout.contains("/Users/"));
}

#[test]
fn unsupported_command_fails_without_polluting_stdout() {
    let output = Command::new(env!("CARGO_BIN_EXE_build-info"))
        .arg("unknown")
        .output()
        .expect("build-info must run");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).expect("stderr must be UTF-8");
    assert!(stderr.contains("usage: build-info print"));
    assert!(!stderr.chars().any(|character| {
        character.is_control() && character != '\n' && character != '\r' && character != '\t'
    }));
}
