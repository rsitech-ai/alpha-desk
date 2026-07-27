use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use test_fixtures::{FixtureEntry, FixtureManifest};

const SOURCE_DIGEST: &str = "dbca188380b064253b36c62435221b9ec1c3c35c9eef60b1296f5faa861ec28e";
const EXPECTED_DIGEST: &str = "0d926e5ff7da1b6248cf88a2bc91c65f848dd1a7ef08dc66a1d0bdbc0cb3d0b2";

fn valid_bundle() -> (tempfile::TempDir, FixtureManifest) {
    let temporary = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temporary.path().join("blocks")).unwrap();
    std::fs::create_dir_all(temporary.path().join("expected")).unwrap();
    std::fs::write(
        temporary.path().join("blocks/trade.json"),
        b"{\"schema\":\"hl.source.fixture.v1\"}\n",
    )
    .unwrap();
    std::fs::write(
        temporary.path().join("expected/trade.canonical.json"),
        b"{\"schema_version\":\"1.0.0\"}\n",
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

fn sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
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
fn lexical_path_aliases_are_rejected_before_normalization() {
    for alias in [
        "blocks/./trade.json",
        "blocks/../blocks/trade.json",
        "blocks//trade.json",
        "blocks/trade.json/",
        "blocks\\trade.json",
        "/blocks/trade.json",
        "C:/blocks/trade.json",
    ] {
        let (temporary, mut manifest) = valid_bundle();
        manifest.fixture[0].source_path = alias.to_owned();

        let error = manifest.verify(temporary.path()).unwrap_err();

        assert!(
            error.to_string().contains("non-canonical fixture path")
                || error.to_string().contains("unsafe fixture path"),
            "{alias:?}: {error}"
        );
    }
}

#[test]
fn lexical_aliases_cannot_bypass_duplicate_path_rejection() {
    let (temporary, mut manifest) = valid_bundle();
    manifest.fixture.push(FixtureEntry {
        id: "alias".to_owned(),
        source_path: "blocks/./trade.json".to_owned(),
        source_sha256: SOURCE_DIGEST.to_owned(),
        source_schema: "hl.source.fixture.v1".to_owned(),
        expected_path: "expected/./trade.canonical.json".to_owned(),
        expected_sha256: EXPECTED_DIGEST.to_owned(),
        expected_schema: "1.0.0".to_owned(),
    });

    let error = manifest.verify(temporary.path()).unwrap_err();

    assert!(
        error.to_string().contains("non-canonical fixture path")
            || error.to_string().contains("duplicate fixture path")
    );
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
fn entry_id_must_match_its_exact_source_and_expected_paths() {
    let (temporary, mut manifest) = valid_bundle();
    manifest.fixture[0].id = "different".to_owned();

    let error = manifest.verify(temporary.path()).unwrap_err();

    assert!(error.to_string().contains("fixture entry pairing mismatch"));
}

#[test]
fn cross_paired_expected_outputs_are_rejected() {
    let temporary = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temporary.path().join("blocks")).unwrap();
    std::fs::create_dir_all(temporary.path().join("expected")).unwrap();
    for id in ["a", "b"] {
        std::fs::write(
            temporary.path().join(format!("blocks/{id}.json")),
            b"{\"schema\":\"hl.source.fixture.v1\"}\n",
        )
        .unwrap();
        std::fs::write(
            temporary
                .path()
                .join(format!("expected/{id}.canonical.json")),
            b"{\"schema_version\":\"1.0.0\"}\n",
        )
        .unwrap();
    }
    let fixture = |id: &str, expected_id: &str| FixtureEntry {
        id: id.to_owned(),
        source_path: format!("blocks/{id}.json"),
        source_sha256: SOURCE_DIGEST.to_owned(),
        source_schema: "hl.source.fixture.v1".to_owned(),
        expected_path: format!("expected/{expected_id}.canonical.json"),
        expected_sha256: EXPECTED_DIGEST.to_owned(),
        expected_schema: "1.0.0".to_owned(),
    };
    let manifest = FixtureManifest {
        version: 1,
        fixture: vec![fixture("a", "b"), fixture("b", "a")],
    };

    let error = manifest.verify(temporary.path()).unwrap_err();

    assert!(error.to_string().contains("fixture entry pairing mismatch"));
}

#[test]
fn verifier_requires_valid_json_and_truthful_schema_claims() {
    let (temporary, mut wrong_schema) = valid_bundle();
    wrong_schema.fixture[0].source_schema = "wrong.schema".to_owned();
    let error = wrong_schema.verify(temporary.path()).unwrap_err();
    assert!(error.to_string().contains("fixture schema mismatch"));

    let (temporary, mut malformed) = valid_bundle();
    let malformed_bytes = b"schema = \"hl.source.fixture.v1\"\n";
    std::fs::write(temporary.path().join("blocks/trade.json"), malformed_bytes).unwrap();
    malformed.fixture[0].source_sha256 = sha256(malformed_bytes);
    let error = malformed.verify(temporary.path()).unwrap_err();
    assert!(error.to_string().contains("parse fixture JSON"));
}

#[test]
fn verifier_rejects_empty_or_non_string_schema_fields() {
    for bytes in [
        b"{\"schema\":\"\"}\n".as_slice(),
        b"{\"schema\":1}\n".as_slice(),
    ] {
        let (temporary, mut manifest) = valid_bundle();
        std::fs::write(temporary.path().join("blocks/trade.json"), bytes).unwrap();
        manifest.fixture[0].source_sha256 = sha256(bytes);

        let error = manifest.verify(temporary.path()).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("missing non-empty string field schema")
        );
    }
}

#[test]
fn verifier_rejects_unsafe_ids_before_they_reach_cli_output() {
    for id in [
        "evil\nmanifest-ok",
        "evil\rcarriage",
        "evil\u{1b}escape",
        "evil\ttab",
        "evil:colon",
    ] {
        let (temporary, mut manifest) = valid_bundle();
        manifest.fixture[0].id = id.to_owned();

        let error = manifest.verify(temporary.path()).unwrap_err();

        assert!(
            error.to_string().contains("unsafe fixture id"),
            "{id:?}: {error}"
        );
    }
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
    std::fs::create_dir(temporary.path().join("expected/nested")).unwrap();
    std::fs::rename(
        temporary.path().join("expected/trade.canonical.json"),
        temporary
            .path()
            .join("expected/nested/trade.canonical.json"),
    )
    .unwrap();
    nested.fixture[0].id = "nested/trade".to_owned();
    nested.fixture[0].source_path = "blocks/nested/trade.json".to_owned();
    nested.fixture[0].expected_path = "expected/nested/trade.canonical.json".to_owned();
    let error = nested.verify(temporary.path()).unwrap_err();
    assert!(error.to_string().contains("symlink"));
}

#[test]
fn non_regular_declared_paths_are_rejected() {
    let (temporary, mut manifest) = valid_bundle();
    std::fs::create_dir(temporary.path().join("blocks/not-a-file.json")).unwrap();
    std::fs::rename(
        temporary.path().join("expected/trade.canonical.json"),
        temporary.path().join("expected/not-a-file.canonical.json"),
    )
    .unwrap();
    manifest.fixture[0].id = "not-a-file".to_owned();
    manifest.fixture[0].source_path = "blocks/not-a-file.json".to_owned();
    manifest.fixture[0].expected_path = "expected/not-a-file.canonical.json".to_owned();

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
