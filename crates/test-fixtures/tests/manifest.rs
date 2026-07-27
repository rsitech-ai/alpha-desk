use std::path::{Path, PathBuf};
use test_fixtures::{FixtureEntry, FixtureManifest};

const SOURCE_DIGEST: &str = "ca3d163bab055381827226140568f3bef7eaac187cebd76878e0b63e9e442356";
const EXPECTED_DIGEST: &str = "e5f1eb4d806641698a35efe20e098efd20d7d57a9b90ee69079d5bb650920726";

fn valid_bundle() -> (tempfile::TempDir, FixtureManifest) {
    let temporary = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temporary.path().join("blocks")).unwrap();
    std::fs::create_dir_all(temporary.path().join("expected")).unwrap();
    std::fs::write(temporary.path().join("blocks/trade.json"), b"{}\n").unwrap();
    std::fs::write(
        temporary.path().join("expected/trade.canonical.json"),
        b"{\"ok\":true}\n",
    )
    .unwrap();
    (
        temporary,
        FixtureManifest {
            version: 1,
            fixture: vec![FixtureEntry {
                id: "trade".to_owned(),
                source_path: "blocks/trade.json".to_owned(),
                source_sha256: SOURCE_DIGEST.to_owned(),
                source_schema: "hl.source.fixture.v1".to_owned(),
                expected_path: "expected/trade.canonical.json".to_owned(),
                expected_sha256: EXPECTED_DIGEST.to_owned(),
                expected_schema: "1.0.0".to_owned(),
            }],
        },
    )
}

#[test]
fn every_declared_fixture_and_expected_output_matches_its_hash() {
    let root = Path::new("../../fixtures/golden");
    let manifest = FixtureManifest::load(root.join("manifest.toml")).unwrap();
    manifest.verify(root).unwrap();
}

#[test]
fn undeclared_files_are_rejected() {
    let temporary = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temporary.path().join("blocks")).unwrap();
    std::fs::create_dir_all(temporary.path().join("expected")).unwrap();
    std::fs::write(temporary.path().join("blocks/orphan.json"), b"{}\n").unwrap();
    let error = FixtureManifest::empty()
        .verify(temporary.path())
        .unwrap_err();
    assert!(error.to_string().contains("undeclared fixture file"));
}

#[test]
fn missing_declared_files_are_rejected() {
    let (temporary, manifest) = valid_bundle();
    std::fs::remove_file(temporary.path().join("blocks/trade.json")).unwrap();

    let error = manifest.verify(temporary.path()).unwrap_err();

    assert!(error.to_string().contains("missing fixture file"));
}

#[test]
fn duplicate_ids_and_paths_are_rejected() {
    let (temporary, mut duplicate_id) = valid_bundle();
    duplicate_id.fixture.push(duplicate_id.fixture[0].clone());
    let error = duplicate_id.verify(temporary.path()).unwrap_err();
    assert!(error.to_string().contains("duplicate fixture id"));

    let (_, mut duplicate_path) = valid_bundle();
    duplicate_path.fixture.push(FixtureEntry {
        id: "other".to_owned(),
        source_path: "blocks/trade.json".to_owned(),
        source_sha256: SOURCE_DIGEST.to_owned(),
        source_schema: "hl.source.fixture.v1".to_owned(),
        expected_path: "expected/other.canonical.json".to_owned(),
        expected_sha256: EXPECTED_DIGEST.to_owned(),
        expected_schema: "1.0.0".to_owned(),
    });
    let error = duplicate_path.verify(temporary.path()).unwrap_err();
    assert!(error.to_string().contains("duplicate fixture path"));
}

#[test]
fn malformed_or_uppercase_sha256_digests_are_rejected() {
    for digest in [
        "abc",
        "gggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggg",
        "CA3D163BAB055381827226140568F3BEF7EAAC187CEBD76878E0B63E9E442356",
    ] {
        let (temporary, mut manifest) = valid_bundle();
        manifest.fixture[0].source_sha256 = digest.to_owned();

        let error = manifest.verify(temporary.path()).unwrap_err();

        assert!(
            error.to_string().contains("invalid lowercase SHA-256"),
            "{digest}: {error}"
        );
    }
}

#[test]
fn digest_mismatches_are_rejected() {
    let (temporary, mut manifest) = valid_bundle();
    manifest.fixture[0].source_sha256 = EXPECTED_DIGEST.to_owned();

    let error = manifest.verify(temporary.path()).unwrap_err();

    assert!(error.to_string().contains("fixture digest mismatch"));
}

#[test]
fn traversal_absolute_and_out_of_root_paths_are_rejected() {
    let (temporary, mut traversal) = valid_bundle();
    traversal.fixture[0].source_path = "../outside.json".to_owned();
    let error = traversal.verify(temporary.path()).unwrap_err();
    assert!(error.to_string().contains("unsafe fixture path"));

    let (temporary, mut absolute) = valid_bundle();
    absolute.fixture[0].source_path = absolute_fixture_path(temporary.path());
    let error = absolute.verify(temporary.path()).unwrap_err();
    assert!(error.to_string().contains("unsafe fixture path"));

    let (temporary, mut misplaced) = valid_bundle();
    std::fs::write(temporary.path().join("outside.json"), b"{}\n").unwrap();
    misplaced.fixture[0].source_path = "outside.json".to_owned();
    let error = misplaced.verify(temporary.path()).unwrap_err();
    assert!(error.to_string().contains("outside blocks/ or expected/"));
}

fn absolute_fixture_path(root: &Path) -> String {
    root.join("blocks/trade.json")
        .to_string_lossy()
        .into_owned()
}

#[cfg(unix)]
#[test]
fn symlinked_files_and_directories_are_rejected() {
    use std::os::unix::fs::symlink;

    let (temporary, manifest) = valid_bundle();
    let source = temporary.path().join("blocks/trade.json");
    let target = temporary.path().join("blocks/real.json");
    std::fs::rename(&source, &target).unwrap();
    symlink(&target, &source).unwrap();
    let error = manifest.verify(temporary.path()).unwrap_err();
    assert!(error.to_string().contains("symlink"));

    let (temporary, mut nested) = valid_bundle();
    let real = temporary.path().join("real");
    std::fs::create_dir(&real).unwrap();
    std::fs::write(real.join("trade.json"), b"{}\n").unwrap();
    symlink(&real, temporary.path().join("blocks/nested")).unwrap();
    nested.fixture[0].source_path = "blocks/nested/trade.json".to_owned();
    let error = nested.verify(temporary.path()).unwrap_err();
    assert!(error.to_string().contains("symlink"));
}

#[test]
fn non_regular_declared_paths_are_rejected() {
    let (temporary, mut manifest) = valid_bundle();
    std::fs::create_dir(temporary.path().join("blocks/not-a-file")).unwrap();
    manifest.fixture[0].source_path = "blocks/not-a-file".to_owned();

    let error = manifest.verify(temporary.path()).unwrap_err();

    assert!(error.to_string().contains("not a regular file"));
}

#[test]
fn fixture_summary_is_sorted_by_id() {
    let (_, mut manifest) = valid_bundle();
    let first = manifest.fixture[0].clone();
    manifest.fixture = vec![
        FixtureEntry {
            id: "z-last".to_owned(),
            ..first.clone()
        },
        FixtureEntry {
            id: "a-first".to_owned(),
            source_path: "blocks/other.json".to_owned(),
            expected_path: "expected/other.canonical.json".to_owned(),
            ..first
        },
    ];

    assert_eq!(manifest.fixture_ids_sorted(), ["a-first", "z-last"]);
}

#[test]
fn load_rejects_invalid_toml() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("manifest.toml");
    std::fs::write(&path, b"not = [valid\n").unwrap();

    let error = FixtureManifest::load(&path).unwrap_err();

    assert!(error.to_string().contains("parse fixture manifest"));
}

#[cfg(unix)]
#[test]
fn load_rejects_a_symlinked_manifest() {
    use std::os::unix::fs::symlink;

    let temporary = tempfile::tempdir().unwrap();
    let real = temporary.path().join("real.toml");
    let linked = temporary.path().join("manifest.toml");
    std::fs::write(&real, b"version = 1\nfixture = []\n").unwrap();
    symlink(&real, &linked).unwrap();

    let error = FixtureManifest::load(&linked).unwrap_err();

    assert!(error.to_string().contains("symlink"));
}

#[test]
fn paths_with_platform_prefixes_are_rejected() {
    let (temporary, mut manifest) = valid_bundle();
    manifest.fixture[0].source_path = PathBuf::from("/tmp/outside").to_string_lossy().into_owned();

    let error = manifest.verify(temporary.path()).unwrap_err();

    assert!(error.to_string().contains("unsafe fixture path"));
}
