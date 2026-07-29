use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn run(output: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_canonical-inspect"))
        .args([
            "canonicalize",
            "--root",
            workspace_root().to_str().unwrap(),
            "--manifest",
            "fixtures/canonical/node-v1/inspect.toml",
            "--output",
            output.to_str().unwrap(),
        ])
        .output()
        .expect("canonical-inspect")
}

#[test]
fn canonicalize_emits_a_stable_reviewable_manifest_and_refuses_overwrite() {
    let first_dir = tempfile::tempdir().unwrap();
    let second_dir = tempfile::tempdir().unwrap();
    let first_path = first_dir.path().join("canonical.json");
    let second_path = second_dir.path().join("canonical.json");

    let first = run(&first_path);
    assert!(
        first.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&first.stderr)
    );
    let second = run(&second_path);
    assert!(second.status.success());
    assert_eq!(
        std::fs::read(&first_path).unwrap(),
        std::fs::read(&second_path).unwrap()
    );
    assert_eq!(
        std::fs::read(&first_path).unwrap(),
        std::fs::read(workspace_root().join("fixtures/canonical/node-v1/expected.json")).unwrap()
    );

    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&first_path).unwrap()).unwrap();
    assert_eq!(manifest["schema"], "alpha-desk.canonical-inspect.v1");
    assert_eq!(
        manifest["qualification"],
        "normalized-public-documentation-example"
    );
    assert_eq!(manifest["production_recording"], false);
    assert_eq!(manifest["mapping_disposition"], "mapped-provisional");
    assert_eq!(manifest["event_count"], 1);
    assert_eq!(manifest["events"][0]["source_event_index"], 0);

    let collision = run(&first_path);
    assert!(!collision.status.success());
    assert!(String::from_utf8_lossy(&collision.stderr).contains("canonical_inspect.output_exists"));
}
