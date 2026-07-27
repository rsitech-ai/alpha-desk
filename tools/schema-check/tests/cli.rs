use prost::Message;
use prost_types::FileDescriptorSet;
use std::{
    fs,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

fn temp_dir() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time must be after the Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "alpha-desk-schema-check-{}-{nonce}",
        std::process::id()
    ))
}

#[test]
fn cli_returns_nonzero_with_an_actionable_decode_error() {
    let directory = temp_dir();
    fs::create_dir_all(&directory).unwrap();
    let baseline = directory.join("baseline.pb");
    let current = directory.join("current.pb");
    fs::write(&baseline, [0xff, 0xff]).unwrap();
    fs::write(&current, FileDescriptorSet::default().encode_to_vec()).unwrap();

    let binary = std::env::var("CARGO_BIN_EXE_schema-check")
        .expect("Cargo must provide the schema-check integration-test binary");
    let output = Command::new(binary)
        .args(["check"])
        .arg(&baseline)
        .arg(&current)
        .output()
        .unwrap();

    fs::remove_dir_all(&directory).unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("baseline"));
    assert!(stderr.contains("decode"));
}
