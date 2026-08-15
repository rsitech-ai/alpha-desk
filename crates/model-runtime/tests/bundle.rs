use std::collections::BTreeMap;

use domain_types::{Decimal, ModelVersion};
use ed25519_dalek::SigningKey;
use model_runtime::{
    ArtifactKind, ModelError, ModelRegistry, ModelState, SignedBundle, TransitionEvidence,
    canonical_message, score_research_bundle, sign_files,
};

fn signing_key() -> SigningKey {
    SigningKey::from_bytes(&[7_u8; 32])
}

fn approved_keys() -> Vec<[u8; 32]> {
    vec![signing_key().verifying_key().to_bytes()]
}

fn linear_files() -> BTreeMap<String, Vec<u8>> {
    let mut files = BTreeMap::new();
    files.insert(
        "manifest.toml".to_owned(),
        br#"model_id = "linear-synthetic-v1"
semantic_version = "0.1.0"
feature_set_version = "features-v1"
artifact_kind = "deterministic-linear-v1"
review_expires_unix_micros = 4102444800000000
approved_use = ["synthetic-research"]
prohibited_use = ["production-inference", "live-trading"]
"#
        .to_vec(),
    );
    files.insert(
        "feature-schema.json".to_owned(),
        br#"{"ordered_features":["flow","crowding"]}"#.to_vec(),
    );
    files.insert("preprocessing.json".to_owned(), b"{}".to_vec());
    files.insert("calibration.json".to_owned(), b"{}".to_vec());
    files.insert("evaluation.json".to_owned(), b"{}".to_vec());
    files.insert("training-data-manifest.json".to_owned(), b"{}".to_vec());
    files.insert(
        "model-card.md".to_owned(),
        b"synthetic linear fixture".to_vec(),
    );
    files.insert(
        "model.linear-v1.json".to_owned(),
        br#"{"kind":"deterministic-linear-v1","weights":["0.5","0.25"],"intercept":"0.1"}"#
            .to_vec(),
    );
    files
}

fn signed_linear() -> SignedBundle {
    let mut files = linear_files();
    let signature = sign_files(&files, &signing_key());
    files.insert("signature.ed25519".to_owned(), signature.to_vec());
    SignedBundle::verify_default(files, &approved_keys()).unwrap()
}

#[test]
fn unsigned_bundle_fails_closed() {
    let mut files = linear_files();
    files.insert("signature.ed25519".to_owned(), Vec::new());
    let error = SignedBundle::verify_default(files, &approved_keys()).unwrap_err();
    assert_eq!(error, ModelError::Unsigned);
}

#[test]
fn missing_signature_file_fails_closed() {
    let files = linear_files();
    let error = SignedBundle::verify_default(files, &approved_keys()).unwrap_err();
    assert_eq!(
        error,
        ModelError::MissingFile {
            name: "signature.ed25519",
        }
    );
}

#[test]
fn tampered_artifact_fails_signature() {
    let mut files = linear_files();
    let signature = sign_files(&files, &signing_key());
    files.insert("signature.ed25519".to_owned(), signature.to_vec());
    files.insert(
        "model.linear-v1.json".to_owned(),
        br#"{"kind":"deterministic-linear-v1","weights":["9","9"],"intercept":"9"}"#.to_vec(),
    );
    let error = SignedBundle::verify_default(files, &approved_keys()).unwrap_err();
    assert_eq!(error, ModelError::InvalidSignature);
}

#[test]
fn unknown_key_fails_signature() {
    let mut files = linear_files();
    let signature = sign_files(&files, &signing_key());
    files.insert("signature.ed25519".to_owned(), signature.to_vec());
    let other = SigningKey::from_bytes(&[9_u8; 32])
        .verifying_key()
        .to_bytes();
    let error = SignedBundle::verify_default(files, &[other]).unwrap_err();
    assert_eq!(error, ModelError::InvalidSignature);
}

#[test]
fn empty_key_list_fails_closed() {
    let mut files = linear_files();
    let signature = sign_files(&files, &signing_key());
    files.insert("signature.ed25519".to_owned(), signature.to_vec());
    let error = SignedBundle::verify_default(files, &[]).unwrap_err();
    assert_eq!(error, ModelError::NoApprovedKeys);
}

#[test]
fn valid_linear_bundle_verifies_and_scores() {
    let bundle = signed_linear();
    let mut registry = ModelRegistry::new();
    let version = registry.register(&bundle).unwrap();
    registry
        .advance(
            &version,
            ModelState::ResearchPassed,
            TransitionEvidence::SyntheticResearch,
        )
        .unwrap();
    let score = score_research_bundle(
        &registry,
        &version,
        &bundle,
        &["flow".to_owned(), "crowding".to_owned()],
        &[
            Decimal::parse_at_scale("2", 8).unwrap(),
            Decimal::parse_at_scale("4", 8).unwrap(),
        ],
    )
    .unwrap();
    assert_eq!(score.value(), Decimal::parse_at_scale("2.1", 8).unwrap());
}

#[test]
fn schema_mismatch_is_rejected() {
    let bundle = signed_linear();
    let mut registry = ModelRegistry::new();
    let version = registry.register(&bundle).unwrap();
    registry
        .advance(
            &version,
            ModelState::ResearchPassed,
            TransitionEvidence::SyntheticResearch,
        )
        .unwrap();
    let error = score_research_bundle(
        &registry,
        &version,
        &bundle,
        &["crowding".to_owned(), "flow".to_owned()],
        &[
            Decimal::parse_at_scale("2", 8).unwrap(),
            Decimal::parse_at_scale("4", 8).unwrap(),
        ],
    )
    .unwrap_err();
    assert_eq!(error, ModelError::SchemaMismatch);
}

#[test]
fn onnx_bundle_verifies_but_production_inference_fails_closed() {
    let mut files = linear_files();
    files.insert(
        "manifest.toml".to_owned(),
        br#"model_id = "onnx-placeholder-v1"
semantic_version = "0.1.0"
feature_set_version = "features-v1"
artifact_kind = "onnx"
review_expires_unix_micros = 4102444800000000
approved_use = ["synthetic-research"]
prohibited_use = ["production-inference", "live-trading"]
"#
        .to_vec(),
    );
    files.remove("model.linear-v1.json");
    files.insert("model.onnx".to_owned(), b"not-a-real-onnx-runtime".to_vec());
    let signature = sign_files(&files, &signing_key());
    files.insert("signature.ed25519".to_owned(), signature.to_vec());
    let bundle = SignedBundle::verify_default(files, &approved_keys()).unwrap();
    assert_eq!(bundle.manifest().artifact_kind(), ArtifactKind::Onnx);
    let mut registry = ModelRegistry::new();
    let version = registry.register(&bundle).unwrap();
    registry
        .advance(
            &version,
            ModelState::ResearchPassed,
            TransitionEvidence::SyntheticResearch,
        )
        .unwrap();
    let error = score_research_bundle(
        &registry,
        &version,
        &bundle,
        &["flow".to_owned(), "crowding".to_owned()],
        &[
            Decimal::parse_at_scale("1", 8).unwrap(),
            Decimal::parse_at_scale("1", 8).unwrap(),
        ],
    )
    .unwrap_err();
    assert_eq!(error, ModelError::OnnxProductionUnavailable);
}

#[test]
fn holdout_and_shadow_transitions_are_explicitly_unimplemented() {
    let bundle = signed_linear();
    let mut registry = ModelRegistry::new();
    let version = registry.register(&bundle).unwrap();
    registry
        .advance(
            &version,
            ModelState::ResearchPassed,
            TransitionEvidence::SyntheticResearch,
        )
        .unwrap();
    assert_eq!(
        registry
            .advance(
                &version,
                ModelState::HoldoutPassed,
                TransitionEvidence::HoldoutEvaluation,
            )
            .unwrap_err(),
        ModelError::HoldoutNotImplemented
    );
    assert_eq!(
        registry.stamp_holdout_passed(&version).unwrap_err(),
        ModelError::HoldoutNotImplemented
    );
    refuse_holdout_passed(
        &mut registry,
        &version,
        TransitionEvidence::SyntheticResearch,
    );
    refuse_holdout_passed(
        &mut registry,
        &version,
        TransitionEvidence::HoldoutEvaluation,
    );
    refuse_holdout_passed(&mut registry, &version, TransitionEvidence::ShadowLive);
    refuse_holdout_passed(
        &mut registry,
        &version,
        TransitionEvidence::ProductionApproval,
    );
    refuse_holdout_passed(&mut registry, &version, TransitionEvidence::Degrade);
    refuse_holdout_passed(&mut registry, &version, TransitionEvidence::Retire);
    refuse_holdout_passed(&mut registry, &version, TransitionEvidence::Revoke);
    assert_eq!(
        registry.require_loadable(&version).unwrap().state(),
        ModelState::ResearchPassed
    );
    assert_eq!(
        registry
            .advance(&version, ModelState::Shadow, TransitionEvidence::ShadowLive,)
            .unwrap_err(),
        ModelError::ShadowLiveNotImplemented
    );
    assert_eq!(
        registry
            .advance(
                &version,
                ModelState::Production,
                TransitionEvidence::ProductionApproval,
            )
            .unwrap_err(),
        ModelError::ProductionNotImplemented
    );
}

#[test]
fn revoked_bundle_cannot_be_loaded() {
    let bundle = signed_linear();
    let mut registry = ModelRegistry::new();
    let version = registry.register(&bundle).unwrap();
    registry
        .advance(&version, ModelState::Revoked, TransitionEvidence::Revoke)
        .unwrap();
    let error = score_research_bundle(
        &registry,
        &version,
        &bundle,
        &["flow".to_owned(), "crowding".to_owned()],
        &[
            Decimal::parse_at_scale("1", 8).unwrap(),
            Decimal::parse_at_scale("1", 8).unwrap(),
        ],
    )
    .unwrap_err();
    assert_eq!(error, ModelError::Revoked);
}

#[test]
fn canonical_message_changes_when_a_file_changes() {
    let first = linear_files();
    let mut second = first.clone();
    second.insert("model-card.md".to_owned(), b"changed".to_vec());
    assert_ne!(canonical_message(&first), canonical_message(&second));
}

#[test]
fn draft_bundle_cannot_score() {
    let bundle = signed_linear();
    let mut registry = ModelRegistry::new();
    let version = registry.register(&bundle).unwrap();
    let error = score_research_bundle(
        &registry,
        &version,
        &bundle,
        &["flow".to_owned(), "crowding".to_owned()],
        &[
            Decimal::parse_at_scale("1", 8).unwrap(),
            Decimal::parse_at_scale("1", 8).unwrap(),
        ],
    )
    .unwrap_err();
    assert_eq!(error.reason_code(), "model_runtime.illegal_transition");
}

#[test]
fn draft_cannot_skip_to_holdout_passed() {
    let bundle = signed_linear();
    let mut registry = ModelRegistry::new();
    let version = registry.register(&bundle).unwrap();
    refuse_holdout_passed(
        &mut registry,
        &version,
        TransitionEvidence::HoldoutEvaluation,
    );
    assert_eq!(
        registry.stamp_holdout_passed(&version).unwrap_err(),
        ModelError::HoldoutNotImplemented
    );
    assert_eq!(
        score_research_bundle(
            &registry,
            &version,
            &bundle,
            &["flow".to_owned(), "crowding".to_owned()],
            &[
                Decimal::parse_at_scale("1", 8).unwrap(),
                Decimal::parse_at_scale("1", 8).unwrap(),
            ],
        )
        .unwrap_err()
        .reason_code(),
        "model_runtime.illegal_transition"
    );
}

fn refuse_holdout_passed(
    registry: &mut ModelRegistry,
    version: &ModelVersion,
    evidence: TransitionEvidence,
) {
    match evidence {
        TransitionEvidence::SyntheticResearch
        | TransitionEvidence::HoldoutEvaluation
        | TransitionEvidence::ShadowLive
        | TransitionEvidence::ProductionApproval
        | TransitionEvidence::Degrade
        | TransitionEvidence::Retire
        | TransitionEvidence::Revoke => {
            assert_eq!(
                registry
                    .advance(version, ModelState::HoldoutPassed, evidence)
                    .unwrap_err(),
                ModelError::HoldoutNotImplemented
            );
        }
    }
}
