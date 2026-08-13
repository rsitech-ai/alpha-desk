use std::fs;
use std::path::Path;

const MAX_CREDENTIAL_BYTES: u64 = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialError {
    Missing,
    Invalid,
}

pub fn load_credential(path: &Path) -> Result<Vec<u8>, CredentialError> {
    let metadata = fs::metadata(path).map_err(|_| CredentialError::Missing)?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_CREDENTIAL_BYTES {
        return Err(CredentialError::Invalid);
    }
    let mut bytes = fs::read(path).map_err(|_| CredentialError::Missing)?;
    while bytes
        .last()
        .is_some_and(|byte| matches!(*byte, b'\n' | b'\r'))
    {
        bytes.pop();
    }
    if bytes.is_empty()
        || bytes
            .iter()
            .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control())
    {
        return Err(CredentialError::Invalid);
    }
    Ok(bytes)
}

pub fn credentials_match(provided: &[u8], expected: &[u8]) -> bool {
    if provided.len() != expected.len() {
        return false;
    }
    provided
        .iter()
        .zip(expected)
        .fold(0_u8, |acc, (left, right)| acc | (left ^ right))
        == 0
}
