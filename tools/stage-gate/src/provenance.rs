use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
    time::Duration,
};

use rustix::fs::{FileType, Mode, OFlags, fstat, open};
use serde::de::DeserializeOwned;
use sha2::{Digest as _, Sha256};
use tempfile::NamedTempFile;

use crate::{
    approvals::TrustPolicy,
    canonical::canonicalize_json_str,
    config::BuilderConfig,
    process::{CommandSpec, OutputPolicy, run_command},
    remote::{RemoteProof, RemoteRequirement, parse_and_validate},
    reports::{BuilderEvidenceValidation, BuilderReport, validate_builder_evidence},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SignedEvidence {
    pub role: String,
    pub payload_path: PathBuf,
    pub signature_path: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedEvidence<T> {
    pub value: T,
    pub canonical_bytes: Vec<u8>,
    pub sha256: String,
    pub signer_role: String,
    pub signer_fingerprint: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SignedEvidenceErrorCode {
    ReadFailed,
    NonCanonical,
    InvalidPayload,
    InvalidDetachedSignature,
    UntrustedRole,
    SignerSeparationFailed,
    IdentityMismatch,
}

#[derive(Debug, thiserror::Error)]
#[error("signed evidence rejected: {code:?}: {detail}")]
pub struct SignedEvidenceError {
    code: SignedEvidenceErrorCode,
    detail: String,
}

impl SignedEvidenceError {
    #[must_use]
    pub const fn code(&self) -> SignedEvidenceErrorCode {
        self.code
    }
}

pub fn verify_signed_builder_report(
    evidence: &SignedEvidence,
    local: &BuilderReport,
    builder_config: &BuilderConfig,
    policy: &TrustPolicy,
    verifier: PathBuf,
    max_bytes: usize,
) -> Result<VerifiedEvidence<BuilderReport>, SignedEvidenceError> {
    let expected = trusted_fingerprint(policy, &evidence.role)?;
    if evidence.role != "builder-b" {
        return Err(error(
            SignedEvidenceErrorCode::UntrustedRole,
            "second builder evidence must use builder-b role",
        ));
    }
    validate_signer_separation(policy)?;
    let (report, canonical_bytes, signer_fingerprint) =
        verify_and_parse(evidence, policy, verifier, max_bytes, expected)?;
    let report: BuilderReport = report;
    let expected_builder_id = format!("builder-b:{signer_fingerprint}");
    if report.builder_identity.builder_id != expected_builder_id
        || report.builder_identity.signer_role != evidence.role
        || report.builder_identity.signer_fingerprint != signer_fingerprint
        || report.builder_identity.builder_id == local.builder_identity.builder_id
        || validate_builder_evidence(builder_config, &report) != BuilderEvidenceValidation::Valid
        || report.comparison_projection().ok() != local.comparison_projection().ok()
    {
        return Err(error(
            SignedEvidenceErrorCode::IdentityMismatch,
            "Builder B report is not bound to its distinct signer and deterministic evidence",
        ));
    }
    Ok(verified(
        report,
        canonical_bytes,
        evidence.role.clone(),
        signer_fingerprint,
    ))
}

pub fn verify_signed_remote_proof(
    evidence: &SignedEvidence,
    requirement: &RemoteRequirement,
    policy: &TrustPolicy,
    verifier: PathBuf,
    max_bytes: usize,
) -> Result<VerifiedEvidence<RemoteProof>, SignedEvidenceError> {
    let expected = trusted_fingerprint(policy, &evidence.role)?;
    if evidence.role != "github-ci" {
        return Err(error(
            SignedEvidenceErrorCode::UntrustedRole,
            "remote proof must use github-ci role",
        ));
    }
    validate_signer_separation(policy)?;
    let (_parsed, canonical_bytes, signer_fingerprint) =
        verify_and_parse::<serde_json::Value>(evidence, policy, verifier, max_bytes, expected)?;
    let proof = parse_and_validate(&canonical_bytes, requirement).map_err(|reasons| {
        error(
            SignedEvidenceErrorCode::InvalidPayload,
            format!("remote proof binding failed: {reasons:?}"),
        )
    })?;
    Ok(verified(
        proof,
        canonical_bytes,
        evidence.role.clone(),
        signer_fingerprint,
    ))
}

fn verify_and_parse<T: DeserializeOwned>(
    evidence: &SignedEvidence,
    policy: &TrustPolicy,
    verifier: PathBuf,
    max_bytes: usize,
    expected_fingerprint: String,
) -> Result<(T, Vec<u8>, String), SignedEvidenceError> {
    let (payload_snapshot, payload_bytes) = snapshot(&evidence.payload_path, max_bytes)?;
    let (signature_snapshot, _) = snapshot(&evidence.signature_path, max_bytes)?;
    let (keyring_snapshot, _) = snapshot(&policy.keyring_path, max_bytes)?;
    let source = std::str::from_utf8(&payload_bytes).map_err(|_| {
        error(
            SignedEvidenceErrorCode::InvalidPayload,
            "payload is not UTF-8",
        )
    })?;
    let canonical = canonicalize_json_str(source).map_err(|canonical_error| {
        error(
            SignedEvidenceErrorCode::InvalidPayload,
            canonical_error.to_string(),
        )
    })?;
    if canonical != payload_bytes {
        return Err(error(
            SignedEvidenceErrorCode::NonCanonical,
            "payload bytes are not exact canonical JSON",
        ));
    }
    let parsed = serde_json::from_slice(&payload_bytes).map_err(|parse_error| {
        error(
            SignedEvidenceErrorCode::InvalidPayload,
            parse_error.to_string(),
        )
    })?;
    let output = run_openpgp_verifier(
        verifier,
        keyring_snapshot.path(),
        signature_snapshot.path(),
        payload_snapshot.path(),
    )
    .map_err(|spawn_error| {
        error(
            SignedEvidenceErrorCode::InvalidDetachedSignature,
            spawn_error,
        )
    })?;
    if !output.success || !clean_validsig(output.status.as_bytes(), &expected_fingerprint) {
        return Err(error(
            SignedEvidenceErrorCode::InvalidDetachedSignature,
            "verifier did not return one clean VALIDSIG for the pinned fingerprint",
        ));
    }
    Ok((parsed, payload_bytes, expected_fingerprint))
}

fn trusted_fingerprint(policy: &TrustPolicy, role: &str) -> Result<String, SignedEvidenceError> {
    policy
        .reviewers
        .iter()
        .find(|reviewer| reviewer.role == role)
        .map(|reviewer| reviewer.fingerprint.clone())
        .filter(|fingerprint| {
            fingerprint.len() == 40
                && fingerprint
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
        .ok_or_else(|| {
            error(
                SignedEvidenceErrorCode::UntrustedRole,
                format!("no pinned full fingerprint for role {role}"),
            )
        })
}

fn validate_signer_separation(policy: &TrustPolicy) -> Result<(), SignedEvidenceError> {
    let by_role = policy
        .reviewers
        .iter()
        .map(|reviewer| (reviewer.role.as_str(), reviewer.fingerprint.as_str()))
        .collect::<BTreeMap<_, _>>();
    let required = ["platform-data", "independent", "builder-b", "github-ci"];
    let fingerprints = required
        .iter()
        .filter_map(|role| by_role.get(role).copied())
        .collect::<BTreeSet<_>>();
    if fingerprints.len() != required.len() {
        return Err(error(
            SignedEvidenceErrorCode::SignerSeparationFailed,
            "all four Stage 0 signer roles must exist and use distinct fingerprints",
        ));
    }
    Ok(())
}

fn snapshot(
    path: &Path,
    max_bytes: usize,
) -> Result<(NamedTempFile, Vec<u8>), SignedEvidenceError> {
    let bytes = read_regular_nofollow(path, max_bytes).map_err(|read_error| {
        error(
            SignedEvidenceErrorCode::ReadFailed,
            format!("{}: {read_error}", path.display()),
        )
    })?;
    let mut snapshot = NamedTempFile::new().map_err(|create_error| {
        error(
            SignedEvidenceErrorCode::ReadFailed,
            create_error.to_string(),
        )
    })?;
    snapshot
        .write_all(&bytes)
        .and_then(|()| snapshot.as_file().sync_all())
        .map_err(|write_error| {
            error(SignedEvidenceErrorCode::ReadFailed, write_error.to_string())
        })?;
    Ok((snapshot, bytes))
}

fn read_regular_nofollow(path: &Path, max_bytes: usize) -> std::io::Result<Vec<u8>> {
    let fd = open(
        path,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(std::io::Error::from)?;
    let stat = fstat(&fd).map_err(std::io::Error::from)?;
    if !FileType::from_raw_mode(stat.st_mode).is_file() {
        return Err(std::io::Error::other("evidence is not a regular file"));
    }
    let mut file = File::from(fd);
    let mut bytes = Vec::new();
    std::io::Read::by_ref(&mut file)
        .take(max_bytes.saturating_add(1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > max_bytes {
        return Err(std::io::Error::other("evidence exceeds configured limit"));
    }
    Ok(bytes)
}

fn valid_signatures(status: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(status)
        .lines()
        .filter_map(|line| line.strip_prefix("[GNUPG:] VALIDSIG "))
        .filter_map(|fields| fields.split_ascii_whitespace().next())
        .map(str::to_ascii_lowercase)
        .collect()
}

fn has_failure_status(status: &[u8]) -> bool {
    const FAILURES: [&str; 7] = [
        "BADSIG",
        "ERRSIG",
        "REVKEYSIG",
        "EXPKEYSIG",
        "EXPSIG",
        "NO_PUBKEY",
        "NODATA",
    ];
    String::from_utf8_lossy(status).lines().any(|line| {
        FAILURES
            .iter()
            .any(|failure| line.starts_with(&format!("[GNUPG:] {failure}")))
    })
}

#[derive(Debug)]
pub(crate) struct OpenPgpVerifierOutput {
    pub success: bool,
    pub status: String,
}

pub(crate) fn run_openpgp_verifier(
    verifier: PathBuf,
    keyring: &Path,
    signature: &Path,
    payload: &Path,
) -> Result<OpenPgpVerifierOutput, String> {
    run_openpgp_verifier_with_timeout(
        verifier,
        keyring,
        signature,
        payload,
        Duration::from_secs(30),
        Duration::from_secs(2),
    )
}

fn run_openpgp_verifier_with_timeout(
    verifier: PathBuf,
    keyring: &Path,
    signature: &Path,
    payload: &Path,
    timeout: Duration,
    termination_grace: Duration,
) -> Result<OpenPgpVerifierOutput, String> {
    let cwd = std::env::current_dir().map_err(|error| error.to_string())?;
    let outcome = run_command(
        &CommandSpec {
            program: verifier,
            args: vec![
                "--status-fd".into(),
                "1".into(),
                "--keyring".into(),
                keyring.as_os_str().to_owned(),
                signature.as_os_str().to_owned(),
                payload.as_os_str().to_owned(),
            ],
            cwd,
            env: Vec::new(),
            timeout,
            termination_grace,
        },
        &OutputPolicy {
            max_bytes_per_stream: 64 * 1024,
            redactions: Vec::new(),
        },
    )
    .map_err(|error| error.to_string())?;
    Ok(OpenPgpVerifierOutput {
        success: outcome.success,
        status: outcome.stdout.text,
    })
}

pub(crate) fn clean_validsig(status: &[u8], expected: &str) -> bool {
    let signatures = valid_signatures(status);
    !has_failure_status(status) && signatures.len() == 1 && signatures[0] == expected
}

fn verified<T>(
    value: T,
    canonical_bytes: Vec<u8>,
    signer_role: String,
    signer_fingerprint: String,
) -> VerifiedEvidence<T> {
    let sha256 = hex::encode(Sha256::digest(&canonical_bytes));
    VerifiedEvidence {
        value,
        canonical_bytes,
        sha256,
        signer_role,
        signer_fingerprint,
    }
}

fn error(code: SignedEvidenceErrorCode, detail: impl Into<String>) -> SignedEvidenceError {
    SignedEvidenceError {
        code,
        detail: detail.into(),
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::{
        fs,
        os::unix::fs::PermissionsExt as _,
        time::{Duration, Instant},
    };

    use tempfile::TempDir;

    use super::run_openpgp_verifier_with_timeout;

    #[test]
    fn sleeping_verifier_is_killed_at_the_bounded_deadline() {
        let directory = TempDir::new().unwrap();
        let verifier = directory.path().join("sleeping-gpgv");
        fs::write(&verifier, "#!/bin/sh\n/bin/sleep 30\n").unwrap();
        fs::set_permissions(&verifier, fs::Permissions::from_mode(0o700)).unwrap();
        let started = Instant::now();

        let error = run_openpgp_verifier_with_timeout(
            verifier,
            directory.path().join("keyring").as_path(),
            directory.path().join("signature").as_path(),
            directory.path().join("payload").as_path(),
            Duration::from_millis(50),
            Duration::from_millis(50),
        )
        .expect_err("a verifier that never returns must time out");

        assert!(error.contains("timed out"), "{error}");
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "bounded verifier exceeded one second"
        );
    }
}
