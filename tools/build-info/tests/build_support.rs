#[allow(dead_code)]
#[path = "../../../crates/telemetry/build_support.rs"]
mod build_support;

use std::fs;
use std::process::Command;

use build_support::{
    BuildProfile, BuildSourceMode, BuildSupportError, fingerprint_schema_material,
    fingerprint_schema_tree, load_build_inputs, parse_source_date_epoch, source_dirty,
};

const FIXTURE_GIT_SHA: &str = "0123456789abcdef0123456789abcdef01234567";
const SINGLE_SCHEMA_MATERIAL: &str = concat!(
    "alpha-desk-schema-material-v1\n",
    "0000000000000007612e70726f746f0000000000000003616263\n",
);

#[test]
fn source_date_epoch_accepts_unsigned_seconds_and_development_absence() {
    assert_eq!(
        parse_source_date_epoch(Some("1784894400"), BuildProfile::Development),
        Ok(Some(1_784_894_400))
    );
    assert_eq!(
        parse_source_date_epoch(None, BuildProfile::Development),
        Ok(None)
    );
}

#[test]
fn source_date_epoch_rejects_malformed_negative_and_overflow_values() {
    for value in ["", "today", "-1", "18446744073709551616"] {
        assert_eq!(
            parse_source_date_epoch(Some(value), BuildProfile::Development),
            Err(BuildSupportError::InvalidSourceDateEpoch)
        );
    }
}

#[test]
fn release_requires_source_date_epoch() {
    assert_eq!(
        parse_source_date_epoch(None, BuildProfile::Release),
        Err(BuildSupportError::ReleaseEpochRequired)
    );
}

#[test]
fn schema_fingerprint_is_order_independent_and_path_delimited() {
    let first = tempfile::tempdir().expect("temporary directory must be available");
    let second = tempfile::tempdir().expect("temporary directory must be available");
    fs::create_dir_all(first.path().join("z")).expect("fixture directory must be created");
    fs::create_dir_all(second.path().join("z")).expect("fixture directory must be created");

    fs::write(first.path().join("z/two.proto"), b"second").expect("fixture file must be written");
    fs::write(first.path().join("one.proto"), b"first").expect("fixture file must be written");
    fs::write(second.path().join("one.proto"), b"first").expect("fixture file must be written");
    fs::write(second.path().join("z/two.proto"), b"second").expect("fixture file must be written");

    assert_eq!(
        fingerprint_schema_tree(first.path()).expect("first fingerprint must succeed"),
        fingerprint_schema_tree(second.path()).expect("second fingerprint must succeed")
    );

    fs::remove_file(second.path().join("one.proto")).expect("fixture file must be removed");
    fs::create_dir_all(second.path().join("on")).expect("fixture directory must be created");
    fs::write(second.path().join("on/e.proto"), b"first").expect("fixture file must be written");

    assert_ne!(
        fingerprint_schema_tree(first.path()).expect("first fingerprint must succeed"),
        fingerprint_schema_tree(second.path()).expect("changed path must change fingerprint")
    );
}

#[test]
fn dirty_detection_tracks_only_relevant_tracked_and_untracked_source_state() {
    let repository = tempfile::tempdir().expect("temporary repository must be available");
    fs::create_dir_all(repository.path().join("crates/demo/src"))
        .expect("source directory must be created");
    fs::create_dir_all(repository.path().join("target"))
        .expect("ignored directory must be created");
    fs::write(repository.path().join(".gitignore"), "target/\n")
        .expect("ignore file must be written");
    fs::write(repository.path().join("Cargo.toml"), "[workspace]\n")
        .expect("workspace manifest must be written");
    fs::write(
        repository.path().join("crates/demo/src/lib.rs"),
        "pub fn value() {}\n",
    )
    .expect("source file must be written");

    run_git(repository.path(), &["init"]);
    run_git(repository.path(), &["add", "."]);
    run_git(
        repository.path(),
        &[
            "-c",
            "user.name=Task Test",
            "-c",
            "user.email=task@example.invalid",
            "commit",
            "-m",
            "fixture",
        ],
    );
    assert!(!source_dirty(repository.path()).expect("clean tree must be inspected"));

    fs::write(
        repository.path().join("crates/demo/src/lib.rs"),
        "pub fn changed() {}\n",
    )
    .expect("tracked source must be changed");
    assert!(source_dirty(repository.path()).expect("tracked source must be inspected"));
    fs::write(
        repository.path().join("crates/demo/src/lib.rs"),
        "pub fn value() {}\n",
    )
    .expect("tracked source must be restored");

    fs::write(
        repository.path().join("crates/demo/src/new.rs"),
        "pub fn new() {}\n",
    )
    .expect("untracked source must be written");
    assert!(source_dirty(repository.path()).expect("untracked source must be inspected"));
    fs::remove_file(repository.path().join("crates/demo/src/new.rs"))
        .expect("untracked source must be removed");

    fs::write(repository.path().join("target/noise"), "ignored")
        .expect("ignored output must be written");
    assert!(!source_dirty(repository.path()).expect("ignored output must be ignored"));
}

#[test]
fn packaged_mode_uses_only_validated_package_local_provenance_inputs() {
    let package = tempfile::tempdir().expect("temporary package must be available");
    fs::write(
        package.path().join("Cargo.toml"),
        "[package]\nname='probe'\n",
    )
    .expect("package manifest must be written");
    fs::write(package.path().join("Cargo.lock"), b"packaged-lock\n")
        .expect("package lock must be written");
    fs::write(
        package.path().join(".cargo_vcs_info.json"),
        r#"{"git":{"sha1":"0123456789abcdef0123456789abcdef01234567","dirty":true},"path_in_vcs":"crates/telemetry"}"#,
    )
    .expect("Cargo VCS metadata must be written");
    fs::write(
        package.path().join("schema-fingerprint-v1.material"),
        concat!(
            "alpha-desk-schema-material-v1\n",
            "0000000000000007612e70726f746f0000000000000003616263\n",
        ),
    )
    .expect("schema material must be written");

    let inputs = load_build_inputs(package.path()).expect("packaged inputs must validate");

    assert_eq!(inputs.mode, BuildSourceMode::Packaged);
    assert_eq!(inputs.git_sha, "0123456789abcdef0123456789abcdef01234567");
    assert!(inputs.dirty);
    assert_eq!(
        inputs.schema_fingerprint,
        "8e374603024ed8febb912642bcb7a620532e98b71881fb43af45cce9d4f9dc72"
    );
    assert_eq!(
        inputs.cargo_lock_sha256,
        "bcdb69944feb1e40a395c27bad24352daab52dbe834140705ca57dbab5805e58"
    );
}

#[test]
fn packaged_mode_treats_omitted_dirty_as_clean_and_accepts_explicit_booleans() {
    let cases = [
        (
            "omitted",
            r#"{"git":{"sha1":"0123456789abcdef0123456789abcdef01234567"},"path_in_vcs":"crates/telemetry"}"#,
            false,
        ),
        (
            "explicit false",
            r#"{"git":{"sha1":"0123456789abcdef0123456789abcdef01234567","dirty":false},"path_in_vcs":"crates/telemetry"}"#,
            false,
        ),
        (
            "explicit true",
            r#"{"git":{"sha1":"0123456789abcdef0123456789abcdef01234567","dirty":true},"path_in_vcs":"crates/telemetry"}"#,
            true,
        ),
    ];

    for (name, vcs_metadata, expected_dirty) in cases {
        let package = packaged_fixture(vcs_metadata, SINGLE_SCHEMA_MATERIAL);
        let inputs = load_build_inputs(package.path())
            .unwrap_or_else(|error| panic!("{name} dirty metadata must validate: {error}"));
        assert_eq!(inputs.mode, BuildSourceMode::Packaged, "{name}");
        assert_eq!(inputs.dirty, expected_dirty, "{name}");
    }
}

#[test]
fn packaged_mode_rejects_present_non_boolean_dirty_values() {
    for (name, dirty_value) in [
        ("null", "null"),
        ("string", r#""false""#),
        ("number", "0"),
        ("object", "{}"),
        ("array", "[]"),
    ] {
        let vcs_metadata = format!(
            r#"{{"git":{{"sha1":"{FIXTURE_GIT_SHA}","dirty":{dirty_value}}},"path_in_vcs":"crates/telemetry"}}"#
        );
        let package = packaged_fixture(&vcs_metadata, SINGLE_SCHEMA_MATERIAL);
        assert_eq!(
            load_build_inputs(package.path()),
            Err(BuildSupportError::InvalidMetadata("packaged VCS metadata")),
            "{name}"
        );
    }
}

#[test]
fn schema_material_accepts_canonical_multi_record_material() {
    let fixture = tempfile::tempdir().expect("temporary material fixture must be available");
    let material_path = fixture.path().join("schema-fingerprint-v1.material");
    fs::write(
        &material_path,
        encode_schema_material(&[
            ("a.proto", b"abc".as_slice()),
            ("z/two.proto", b"second".as_slice()),
        ]),
    )
    .expect("canonical material must be written");

    assert_eq!(
        fingerprint_schema_material(&material_path)
            .expect("canonical multi-record material must validate"),
        "b3f60770bff7bbdd2982571171b714bf767ed4c89bab717c01343d18f4f3e8f8"
    );
}

#[test]
fn schema_material_rejects_malformed_encodings_and_records() {
    let valid = encode_schema_material(&[("a.proto", b"abc".as_slice())]);
    let valid_payload = valid
        .strip_prefix("alpha-desk-schema-material-v1\n")
        .expect("test material must have the canonical header");
    let cases = [
        ("empty payload", material_document("")),
        (
            "uppercase hex",
            material_document(&valid_payload.replacen('e', "E", 1)),
        ),
        ("odd hex", material_document("0")),
        (
            "absolute path",
            encode_schema_material(&[("/a.proto", b"a".as_slice())]),
        ),
        (
            "backslash path",
            encode_schema_material(&[(r"a\b.proto", b"a".as_slice())]),
        ),
        (
            "dot path segment",
            encode_schema_material(&[("a/./b.proto", b"a".as_slice())]),
        ),
        (
            "dotdot path segment",
            encode_schema_material(&[("a/../b.proto", b"a".as_slice())]),
        ),
        (
            "empty path segment",
            encode_schema_material(&[("a//b.proto", b"a".as_slice())]),
        ),
        (
            "duplicate path",
            encode_schema_material(&[("a.proto", b"a".as_slice()), ("a.proto", b"b".as_slice())]),
        ),
        (
            "reversed paths",
            encode_schema_material(&[("z.proto", b"a".as_slice()), ("a.proto", b"b".as_slice())]),
        ),
        ("length overflow", material_document("ffffffffffffffff")),
        ("truncated path length", material_document("00000000")),
        ("truncated path", material_document("000000000000000761")),
        (
            "truncated content length",
            material_document("00000000000000016100000000"),
        ),
        (
            "truncated content",
            material_document("000000000000000161000000000000000361"),
        ),
        ("trailing partial bytes", format!("{valid}00")),
    ];

    for (name, encoded) in cases {
        let fixture = tempfile::tempdir().expect("temporary material fixture must be available");
        let material_path = fixture.path().join("schema-fingerprint-v1.material");
        fs::write(&material_path, encoded).expect("malformed material must be written");
        assert_eq!(
            fingerprint_schema_material(&material_path),
            Err(BuildSupportError::InvalidMetadata(
                "packaged schema material"
            )),
            "{name}"
        );
    }
}

#[test]
fn checkout_mode_requires_live_schema_and_material_exact_match() {
    let workspace = tempfile::tempdir().expect("temporary checkout must be available");
    fs::create_dir_all(workspace.path().join("crates/telemetry"))
        .expect("telemetry fixture directory must be created");
    fs::create_dir_all(workspace.path().join("schemas/proto"))
        .expect("schema fixture directory must be created");
    fs::write(workspace.path().join("Cargo.toml"), "[workspace]\n")
        .expect("workspace manifest must be written");
    fs::write(workspace.path().join("Cargo.lock"), b"checkout-lock\n")
        .expect("workspace lock must be written");
    fs::write(workspace.path().join("schemas/proto/a.proto"), b"abc")
        .expect("live schema must be written");
    fs::write(
        workspace
            .path()
            .join("crates/telemetry/schema-fingerprint-v1.material"),
        SINGLE_SCHEMA_MATERIAL,
    )
    .expect("schema material must be written");
    run_git(workspace.path(), &["init"]);
    run_git(workspace.path(), &["add", "."]);
    run_git(
        workspace.path(),
        &[
            "-c",
            "user.name=Task Test",
            "-c",
            "user.email=task@example.invalid",
            "commit",
            "-m",
            "fixture",
        ],
    );

    let inputs = load_build_inputs(&workspace.path().join("crates/telemetry"))
        .expect("matching live schema and material must validate");
    assert_eq!(inputs.mode, BuildSourceMode::Checkout);
    assert_eq!(
        inputs.schema_fingerprint,
        "8e374603024ed8febb912642bcb7a620532e98b71881fb43af45cce9d4f9dc72"
    );

    fs::write(workspace.path().join("schemas/proto/a.proto"), b"changed")
        .expect("live schema must be changed");
    assert_eq!(
        load_build_inputs(&workspace.path().join("crates/telemetry")),
        Err(BuildSupportError::InvalidMetadata(
            "packaged schema material"
        ))
    );
}

#[test]
fn packaged_mode_rejects_unverifiable_vcs_and_schema_material() {
    let package = tempfile::tempdir().expect("temporary package must be available");
    fs::write(
        package.path().join("Cargo.toml"),
        "[package]\nname='probe'\n",
    )
    .expect("package manifest must be written");
    fs::write(package.path().join("Cargo.lock"), b"packaged-lock\n")
        .expect("package lock must be written");
    fs::write(
        package.path().join(".cargo_vcs_info.json"),
        r#"{"git":{"sha1":"UPPERCASE","dirty":false},"path_in_vcs":""}"#,
    )
    .expect("invalid Cargo VCS metadata must be written");
    fs::write(
        package.path().join("schema-fingerprint-v1.material"),
        "alpha-desk-schema-material-v1\n00\n",
    )
    .expect("invalid schema material must be written");

    assert!(matches!(
        load_build_inputs(package.path()),
        Err(BuildSupportError::InvalidMetadata("packaged VCS metadata"))
    ));

    fs::write(
        package.path().join(".cargo_vcs_info.json"),
        r#"{"git":{"sha1":"0123456789abcdef0123456789abcdef01234567","dirty":false},"path_in_vcs":""}"#,
    )
    .expect("valid Cargo VCS metadata must be written");
    assert!(matches!(
        load_build_inputs(package.path()),
        Err(BuildSupportError::InvalidMetadata(
            "packaged schema material"
        ))
    ));
}

fn packaged_fixture(vcs_metadata: &str, schema_material: &str) -> tempfile::TempDir {
    let package = tempfile::tempdir().expect("temporary package must be available");
    fs::write(
        package.path().join("Cargo.toml"),
        "[package]\nname='probe'\n",
    )
    .expect("package manifest must be written");
    fs::write(package.path().join("Cargo.lock"), b"packaged-lock\n")
        .expect("package lock must be written");
    fs::write(package.path().join(".cargo_vcs_info.json"), vcs_metadata)
        .expect("Cargo VCS metadata must be written");
    fs::write(
        package.path().join("schema-fingerprint-v1.material"),
        schema_material,
    )
    .expect("schema material must be written");
    package
}

fn encode_schema_material(records: &[(&str, &[u8])]) -> String {
    let mut material = Vec::new();
    for (path, content) in records {
        material.extend_from_slice(
            &u64::try_from(path.len())
                .expect("fixture path length must fit in u64")
                .to_be_bytes(),
        );
        material.extend_from_slice(path.as_bytes());
        material.extend_from_slice(
            &u64::try_from(content.len())
                .expect("fixture content length must fit in u64")
                .to_be_bytes(),
        );
        material.extend_from_slice(content);
    }

    let mut encoded = String::from("alpha-desk-schema-material-v1\n");
    for byte in material {
        use std::fmt::Write as _;
        write!(&mut encoded, "{byte:02x}").expect("writing to a String must succeed");
    }
    encoded.push('\n');
    encoded
}

fn material_document(payload: &str) -> String {
    format!("alpha-desk-schema-material-v1\n{payload}\n")
}

fn run_git(repository: &std::path::Path, arguments: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(arguments)
        .status()
        .expect("Git must run");
    assert!(status.success(), "Git command failed: {arguments:?}");
}
