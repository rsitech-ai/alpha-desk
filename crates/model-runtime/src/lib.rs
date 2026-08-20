#![forbid(unsafe_code)]

mod bundle;
mod error;
mod inference;
mod registry;
mod schema;
mod signature;

pub use bundle::{
    ArtifactKind, BundleManifest, REQUIRED_FILES, SignedBundle, canonical_message, sign_files,
};
pub use error::ModelError;
pub use inference::{ResearchScore, score_research_bundle};
pub use registry::{ModelRegistry, ModelState, RegistryRecord, TransitionEvidence};
pub use schema::FeatureSchema;
pub use signature::{BundleSignatureVerifier, Ed25519Verifier, verify_against_approved_keys};
