use telemetry::{BuildProvenance, ProvenanceError};

#[test]
fn compile_time_provenance_has_complete_validated_identifiers() {
    let build = BuildProvenance::current().expect("build script must emit valid metadata");

    assert_eq!(build.git_sha.len(), 40);
    assert!(build.git_sha.bytes().all(|byte| byte.is_ascii_hexdigit()));
    assert!(!build.rustc_version.is_empty());
    assert!(!build.target_triple.is_empty());
    assert_eq!(build.schema_fingerprint.len(), 64);
    assert_eq!(build.cargo_lock_sha256.len(), 64);
    assert_eq!(build.reproducible, build.build_epoch.is_some());
}

#[test]
fn provenance_json_uses_fixed_field_order_and_no_workspace_path() {
    let build = BuildProvenance::try_new(
        "0123456789abcdef0123456789abcdef01234567",
        false,
        "rustc 1.97.1 (stable)",
        "aarch64-apple-darwin",
        Some(1_784_894_400),
        "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
        "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef",
    )
    .expect("literal provenance must be valid");

    assert_eq!(
        build.to_json().expect("serialization must succeed"),
        "{\"git_sha\":\"0123456789abcdef0123456789abcdef01234567\",\"dirty\":false,\"rustc_version\":\"rustc 1.97.1 (stable)\",\"target_triple\":\"aarch64-apple-darwin\",\"build_epoch\":1784894400,\"reproducible\":true,\"schema_fingerprint\":\"abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789\",\"cargo_lock_sha256\":\"1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef\"}"
    );
}

#[test]
fn provenance_rejects_malformed_compile_time_identifiers() {
    assert_eq!(
        BuildProvenance::try_new(
            "short",
            false,
            "rustc 1.97.1",
            "aarch64-apple-darwin",
            None,
            "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
            "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef",
        ),
        Err(ProvenanceError::InvalidGitSha)
    );

    assert_eq!(
        BuildProvenance::try_new(
            "0123456789ABCDEF0123456789ABCDEF01234567",
            false,
            "rustc 1.97.1",
            "aarch64-apple-darwin",
            None,
            "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
            "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef",
        ),
        Err(ProvenanceError::InvalidGitSha)
    );
}
