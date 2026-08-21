use prost::Message;
use prost_types::FileDescriptorSet;
use std::{fs, process::Command};

fn generator() -> Command {
    Command::new(env!("CARGO_BIN_EXE_schema-generate"))
}

#[test]
fn contracts_command_exports_exact_descriptor_and_rust_set_without_overwrite() {
    let temporary = tempfile::tempdir().expect("temporary directory must be available");
    let descriptor = temporary.path().join("current.pb");
    let rust_output = temporary.path().join("rust");

    let output = generator()
        .args(["contracts", "--descriptor"])
        .arg(&descriptor)
        .arg("--rust-out")
        .arg(&rust_output)
        .output()
        .expect("schema generator must run");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let descriptor_set = FileDescriptorSet::decode(
        fs::read(&descriptor)
            .expect("descriptor output must be readable")
            .as_slice(),
    )
    .expect("descriptor output must decode");
    let mut descriptor_names = descriptor_set
        .file
        .iter()
        .map(|file| file.name.as_deref().expect("descriptor name is required"))
        .collect::<Vec<_>>();
    descriptor_names.sort_unstable();
    assert_eq!(
        descriptor_names,
        [
            "canonical/v1/events.proto",
            "canonical/v1/snapshots.proto",
            "common/v1/types.proto",
            "health/v1/health.proto",
            "stream/v1/envelope.proto",
        ]
    );

    let mut rust_names = fs::read_dir(&rust_output)
        .expect("generated Rust output must be readable")
        .map(|entry| {
            entry
                .expect("generated Rust entry must be readable")
                .file_name()
                .into_string()
                .expect("generated Rust names must be UTF-8")
        })
        .collect::<Vec<_>>();
    rust_names.sort();
    assert_eq!(
        rust_names,
        [
            "hl.canonical.v1.rs",
            "hl.common.v1.rs",
            "hl.health.v1.rs",
            "hl.stream.v1.rs",
        ]
    );

    fs::write(rust_output.join("sentinel"), b"keep").expect("sentinel must be written");
    let second_descriptor = temporary.path().join("second.pb");
    let overwrite = generator()
        .args(["contracts", "--descriptor"])
        .arg(&second_descriptor)
        .arg("--rust-out")
        .arg(&rust_output)
        .output()
        .expect("schema generator overwrite check must run");
    assert!(!overwrite.status.success());
    assert!(String::from_utf8_lossy(&overwrite.stderr).contains("not empty"));
    assert!(!second_descriptor.exists());
    assert_eq!(
        fs::read(rust_output.join("sentinel")).expect("sentinel must remain readable"),
        b"keep"
    );

    let empty_rust_output = temporary.path().join("empty-rust");
    fs::create_dir(&empty_rust_output).expect("empty Rust output must be created");
    let invalid_descriptor = temporary.path().join("missing-parent/current.pb");
    let invalid = generator()
        .args(["contracts", "--descriptor"])
        .arg(&invalid_descriptor)
        .arg("--rust-out")
        .arg(&empty_rust_output)
        .output()
        .expect("descriptor parent validation must run");
    assert!(!invalid.status.success());
    assert!(!invalid_descriptor.exists());
    assert_eq!(
        fs::read_dir(&empty_rust_output)
            .expect("Rust output must remain readable")
            .count(),
        0,
        "failed contract export must not leave partial Rust artifacts"
    );

    let overlapping_output = temporary.path().join("overlapping");
    fs::create_dir(&overlapping_output).expect("overlapping output must be created");
    let overlapping_descriptor = overlapping_output.join("current.pb");
    let overlap = generator()
        .args(["contracts", "--descriptor"])
        .arg(&overlapping_descriptor)
        .arg("--rust-out")
        .arg(&overlapping_output)
        .output()
        .expect("overlapping output validation must run");
    assert!(!overlap.status.success());
    assert!(String::from_utf8_lossy(&overlap.stderr).contains("must not overlap"));
    assert!(!overlapping_descriptor.exists());
    assert_eq!(
        fs::read_dir(&overlapping_output)
            .expect("overlapping output must remain readable")
            .count(),
        0,
        "overlap rejection must not leave partial artifacts"
    );
}

#[test]
fn material_command_writes_canonical_document_and_rejects_unsafe_outputs() {
    let temporary = tempfile::tempdir().expect("temporary directory must be available");
    let schemas = temporary.path().join("schemas");
    fs::create_dir_all(schemas.join("z")).expect("schema directory must be created");
    fs::write(schemas.join("z/two.proto"), b"second").expect("schema must be written");
    fs::write(schemas.join("a.proto"), b"abc").expect("schema must be written");
    let material = temporary.path().join("schema.material");

    let output = generator()
        .args(["material", "--schema-root"])
        .arg(&schemas)
        .arg("--output")
        .arg(&material)
        .output()
        .expect("material generator must run");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read_to_string(&material).expect("material must be readable"),
        concat!(
            "alpha-desk-schema-material-v1\n",
            "0000000000000007612e70726f746f0000000000000003616263",
            "000000000000000b7a2f74776f2e70726f746f00000000000000067365636f6e64\n",
        )
    );
    let material_document = fs::read_to_string(&material).expect("material must remain readable");
    let encoded_lines = material_document.lines().skip(1).collect::<Vec<_>>();
    assert!(
        encoded_lines
            .iter()
            .take(encoded_lines.len().saturating_sub(1))
            .all(|line| line.len() == 120),
        "every non-final encoded line must contain exactly 120 hex characters"
    );
    assert!(
        encoded_lines
            .last()
            .is_some_and(|line| !line.is_empty() && line.len() <= 120)
    );

    let overwrite = generator()
        .args(["material", "--schema-root"])
        .arg(&schemas)
        .arg("--output")
        .arg(&material)
        .output()
        .expect("material overwrite check must run");
    assert!(!overwrite.status.success());
    assert!(String::from_utf8_lossy(&overwrite.stderr).contains("already exists"));

    let inside = schemas.join("generated.material");
    let unsafe_output = generator()
        .args(["material", "--schema-root"])
        .arg(&schemas)
        .arg("--output")
        .arg(&inside)
        .output()
        .expect("inside-root output check must run");
    assert!(!unsafe_output.status.success());
    assert!(String::from_utf8_lossy(&unsafe_output.stderr).contains("inside schema root"));
    assert!(!inside.exists());
}

#[test]
fn material_command_wraps_each_nonfinal_line_at_exactly_120_hex_characters() {
    let temporary = tempfile::tempdir().expect("temporary directory must be available");
    let schemas = temporary.path().join("schemas");
    fs::create_dir(&schemas).expect("schema directory must be created");
    fs::write(schemas.join("large.proto"), vec![0xab; 256]).expect("schema must be written");
    let material = temporary.path().join("large.material");

    let output = generator()
        .args(["material", "--schema-root"])
        .arg(&schemas)
        .arg("--output")
        .arg(&material)
        .output()
        .expect("material generator must run");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let material_document = fs::read_to_string(material).expect("material must be readable");
    let encoded_lines = material_document.lines().skip(1).collect::<Vec<_>>();
    assert!(encoded_lines.len() > 1);
    assert!(
        encoded_lines[..encoded_lines.len() - 1]
            .iter()
            .all(|line| line.len() == 120)
    );
    assert!(
        encoded_lines
            .last()
            .is_some_and(|line| !line.is_empty() && line.len() <= 120)
    );
}

#[cfg(unix)]
#[test]
fn material_command_rejects_symlinked_and_non_utf8_schema_entries() {
    use std::{
        ffi::OsString,
        os::{unix::ffi::OsStringExt, unix::fs::symlink},
    };

    let temporary = tempfile::tempdir().expect("temporary directory must be available");
    let schemas = temporary.path().join("schemas");
    fs::create_dir(&schemas).expect("schema directory must be created");
    let outside = temporary.path().join("outside.proto");
    fs::write(&outside, b"outside").expect("outside schema must be written");
    symlink(&outside, schemas.join("linked.proto")).expect("schema symlink must be created");

    let symlink_output = temporary.path().join("symlink.material");
    let symlink_result = generator()
        .args(["material", "--schema-root"])
        .arg(&schemas)
        .arg("--output")
        .arg(&symlink_output)
        .output()
        .expect("symlink check must run");
    assert!(!symlink_result.status.success());
    assert!(String::from_utf8_lossy(&symlink_result.stderr).contains("symlink or special"));
    assert!(!symlink_output.exists());

    fs::remove_file(schemas.join("linked.proto")).expect("schema symlink must be removed");
    let invalid_name =
        OsString::from_vec(vec![b'i', b'n', 0xff, b'.', b'p', b'r', b'o', b't', b'o']);
    if fs::write(schemas.join(invalid_name), b"invalid").is_err() {
        return;
    }
    let encoding_output = temporary.path().join("encoding.material");
    let encoding_result = generator()
        .args(["material", "--schema-root"])
        .arg(&schemas)
        .arg("--output")
        .arg(&encoding_output)
        .output()
        .expect("path encoding check must run");
    assert!(!encoding_result.status.success());
    assert!(String::from_utf8_lossy(&encoding_result.stderr).contains("UTF-8"));
    assert!(!encoding_output.exists());
}
