use ed25519_dalek::{Signature, Verifier, VerifyingKey};

use crate::error::ModelError;

pub trait BundleSignatureVerifier {
    fn verify(&self, message: &[u8], signature: &[u8], public_key: &[u8])
    -> Result<(), ModelError>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Ed25519Verifier;

impl BundleSignatureVerifier for Ed25519Verifier {
    fn verify(
        &self,
        message: &[u8],
        signature: &[u8],
        public_key: &[u8],
    ) -> Result<(), ModelError> {
        if signature.is_empty() {
            return Err(ModelError::Unsigned);
        }
        let key_bytes: &[u8; 32] = public_key
            .try_into()
            .map_err(|_| ModelError::InvalidSignature)?;
        let signature_bytes: &[u8; 64] = signature
            .try_into()
            .map_err(|_| ModelError::InvalidSignature)?;
        let key = VerifyingKey::from_bytes(key_bytes).map_err(|_| ModelError::InvalidSignature)?;
        let parsed = Signature::from_bytes(signature_bytes);
        key.verify(message, &parsed)
            .map_err(|_| ModelError::InvalidSignature)
    }
}

pub fn verify_against_approved_keys<V: BundleSignatureVerifier>(
    verifier: &V,
    message: &[u8],
    signature: &[u8],
    approved_keys: &[[u8; 32]],
) -> Result<[u8; 32], ModelError> {
    if signature.is_empty() {
        return Err(ModelError::Unsigned);
    }
    if approved_keys.is_empty() {
        return Err(ModelError::NoApprovedKeys);
    }
    for key in approved_keys {
        if verifier.verify(message, signature, key).is_ok() {
            return Ok(*key);
        }
    }
    Err(ModelError::InvalidSignature)
}
