#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ModelError {
    #[error("model bundle is unsigned")]
    Unsigned,
    #[error("model bundle signature is invalid")]
    InvalidSignature,
    #[error("model bundle is missing required file {name}")]
    MissingFile { name: &'static str },
    #[error("model bundle manifest is invalid")]
    InvalidManifest,
    #[error("model feature schema does not match")]
    SchemaMismatch,
    #[error("ONNX production inference is not implemented")]
    OnnxProductionUnavailable,
    #[error("model artifact kind is not a signed research adapter")]
    UnsupportedArtifact,
    #[error("registry transition is not permitted from {from} to {to}")]
    IllegalTransition {
        from: &'static str,
        to: &'static str,
    },
    #[error("holdout evaluation is not implemented")]
    HoldoutNotImplemented,
    #[error("shadow-live evaluation is not implemented")]
    ShadowLiveNotImplemented,
    #[error("production model promotion is not implemented")]
    ProductionNotImplemented,
    #[error("revoked or retired models cannot be loaded")]
    Revoked,
    #[error("model is not registered")]
    Unregistered,
    #[error("approved public key list is empty")]
    NoApprovedKeys,
    #[error("model input is invalid")]
    InvalidInput,
}

impl ModelError {
    #[must_use]
    pub const fn reason_code(&self) -> &'static str {
        match self {
            Self::Unsigned => "model_runtime.unsigned",
            Self::InvalidSignature => "model_runtime.invalid_signature",
            Self::MissingFile { .. } => "model_runtime.missing_file",
            Self::InvalidManifest => "model_runtime.invalid_manifest",
            Self::SchemaMismatch => "model_runtime.schema_mismatch",
            Self::OnnxProductionUnavailable => "model_runtime.onnx_production_unavailable",
            Self::UnsupportedArtifact => "model_runtime.unsupported_artifact",
            Self::IllegalTransition { .. } => "model_runtime.illegal_transition",
            Self::HoldoutNotImplemented => "model_runtime.holdout_not_implemented",
            Self::ShadowLiveNotImplemented => "model_runtime.shadow_live_not_implemented",
            Self::ProductionNotImplemented => "model_runtime.production_not_implemented",
            Self::Revoked => "model_runtime.revoked",
            Self::Unregistered => "model_runtime.unregistered",
            Self::NoApprovedKeys => "model_runtime.no_approved_keys",
            Self::InvalidInput => "model_runtime.invalid_input",
        }
    }
}
