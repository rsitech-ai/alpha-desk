use std::fs;
use test_fixtures::FixtureManifest;

fn write_pair(root: &std::path::Path, id: &str) {
    let source = format!("{{\"schema\":\"hl.source.fixture.v1\",\"id\":\"{id}\"}}\n");
    let expected = format!("{{\"schema_version\":\"1.0.0\",\"id\":\"{id}\"}}\n");
    let source_path = root.join(format!("blocks/{id}.json"));
    let expected_path = root.join(format!("expected/{id}.canonical.json"));
    fs::create_dir_all(source_path.parent().unwrap()).unwrap();
    fs::create_dir_all(expected_path.parent().unwrap()).unwrap();
    fs::write(source_path, source).unwrap();
    fs::write(expected_path, expected).unwrap();
}

#[test]
fn generator_scans_bytewise_and_atomic_output_is_stable() {
    let temporary = tempfile::tempdir().unwrap();
    fs::create_dir_all(temporary.path().join("blocks")).unwrap();
    fs::create_dir_all(temporary.path().join("expected")).unwrap();
    write_pair(temporary.path(), "z-last");
    write_pair(temporary.path(), "a-first");

    let manifest = FixtureManifest::generate(temporary.path()).unwrap();
    assert_eq!(manifest.fixture_ids_sorted(), ["a-first", "z-last"]);
    assert_eq!(
        manifest
            .fixture
            .iter()
            .map(|entry| entry.source_path.as_str())
            .collect::<Vec<_>>(),
        ["blocks/a-first.json", "blocks/z-last.json"]
    );

    let first_hash = manifest.write_atomic(temporary.path()).unwrap();
    let first_bytes = fs::read(temporary.path().join("manifest.toml")).unwrap();
    let second_hash = FixtureManifest::generate(temporary.path())
        .unwrap()
        .write_atomic(temporary.path())
        .unwrap();
    let second_bytes = fs::read(temporary.path().join("manifest.toml")).unwrap();

    assert_eq!(first_hash, second_hash);
    assert_eq!(first_bytes, second_bytes);
    assert!(first_bytes.ends_with(b"\n"));
    assert!(
        fs::read_dir(temporary.path())
            .unwrap()
            .filter_map(Result::ok)
            .all(|entry| !entry
                .file_name()
                .to_string_lossy()
                .starts_with(".manifest.toml.tmp"))
    );
    FixtureManifest::load(temporary.path().join("manifest.toml"))
        .unwrap()
        .verify(temporary.path())
        .unwrap();
}

#[test]
fn generator_rejects_unpaired_source_and_expected_files() {
    let temporary = tempfile::tempdir().unwrap();
    fs::create_dir_all(temporary.path().join("blocks")).unwrap();
    fs::create_dir_all(temporary.path().join("expected")).unwrap();
    fs::write(
        temporary.path().join("blocks/orphan.json"),
        b"{\"schema\":\"hl.source.fixture.v1\"}\n",
    )
    .unwrap();
    let error = FixtureManifest::generate(temporary.path()).unwrap_err();
    assert!(error.to_string().contains("no deterministic expected pair"));

    fs::remove_file(temporary.path().join("blocks/orphan.json")).unwrap();
    fs::write(
        temporary.path().join("expected/orphan.canonical.json"),
        b"{\"schema_version\":\"1.0.0\"}\n",
    )
    .unwrap();
    let error = FixtureManifest::generate(temporary.path()).unwrap_err();
    assert!(error.to_string().contains("no deterministic source pair"));
}

#[test]
fn nested_canonical_ids_generate_and_verify_in_bytewise_order() {
    let temporary = tempfile::tempdir().unwrap();
    fs::create_dir_all(temporary.path().join("blocks")).unwrap();
    fs::create_dir_all(temporary.path().join("expected")).unwrap();
    write_pair(temporary.path(), "z/last");
    write_pair(temporary.path(), "a/deep/first");

    let manifest = FixtureManifest::generate(temporary.path()).unwrap();

    assert_eq!(manifest.fixture_ids_sorted(), ["a/deep/first", "z/last"]);
    manifest.verify(temporary.path()).unwrap();
}

#[test]
fn generator_rejects_control_and_nonportable_ids() {
    for id in [
        "evil\nmanifest-ok",
        "evil\rcarriage",
        "evil\u{1b}escape",
        "evil\ttab",
        "evil:colon",
        "Uppercase",
        ".hidden",
    ] {
        let temporary = tempfile::tempdir().unwrap();
        fs::create_dir_all(temporary.path().join("blocks")).unwrap();
        fs::create_dir_all(temporary.path().join("expected")).unwrap();
        write_pair(temporary.path(), id);

        let error = FixtureManifest::generate(temporary.path()).unwrap_err();

        assert!(
            error.to_string().contains("unsafe fixture id"),
            "{id:?}: {error}"
        );
    }
}
