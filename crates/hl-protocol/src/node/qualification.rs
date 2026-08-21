//! Fail-closed qualification claims for paired Hyperliquid node recordings.
//!
//! Decoding a manifest never authorizes committed output. Authorization is an
//! opaque token returned only when the canonical manifest digest is present in
//! the built-in registry. The production registry is intentionally empty until
//! a reviewed, byte-exact operator corpus exists.

use std::{collections::BTreeSet, fmt, path::Path};

use domain_types::{ChainId, SourceId};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::ErrorDisposition;

pub const MAX_QUALIFICATION_MANIFEST_BYTES: usize = 1024 * 1024;
pub const MAX_IDENTITY_BYTES: usize = 256;
const MAX_ARGUMENT_BYTES: usize = 4_096;
const MAX_ARGUMENTS: usize = 128;
const MAX_RECORDING_FILES: usize = 4_096;
const MAX_RELATIVE_PATH_BYTES: usize = 4_096;
const SCHEMA_V1: &str = "hyperliquid-alpha-desk/node-source-qualification/v1";

macro_rules! digest_type {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name([u8; 32]);

        impl $name {
            #[must_use]
            pub const fn from_bytes(bytes: [u8; 32]) -> Self {
                Self(bytes)
            }

            pub fn parse_lower_hex(value: &str) -> Result<Self, NodeSourceQualificationError> {
                if value.len() != 64
                    || value
                        .bytes()
                        .any(|byte| !byte.is_ascii_digit() && !(b'a'..=b'f').contains(&byte))
                {
                    return Err(NodeSourceQualificationError::InvalidDigest);
                }
                let mut bytes = [0_u8; 32];
                hex::decode_to_slice(value, &mut bytes)
                    .map_err(|_| NodeSourceQualificationError::InvalidDigest)?;
                Ok(Self(bytes))
            }

            #[must_use]
            pub const fn as_bytes(&self) -> &[u8; 32] {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&hex::encode(self.0))
            }
        }
    };
}

digest_type!(Sha256Digest);
digest_type!(Blake3Digest);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BoundedIdentity(String);

impl BoundedIdentity {
    pub fn new(value: impl Into<String>) -> Result<Self, NodeSourceQualificationError> {
        let value = value.into();
        validate_text(&value, MAX_IDENTITY_BYTES)?;
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NodeRecordingFileRoleV1 {
    Committed,
    Trade,
    AbciSnapshot,
    L4Snapshot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OutputFileBufferingV1 {
    Enabled,
    Disabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RedistributionClassV1 {
    PrivateOperatorEvidence,
    DerivedRedistributable,
    RedistributableWithAttribution,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeSourceGroupV1 {
    committed_source_id: SourceId,
    trade_source_id: SourceId,
}

impl NodeSourceGroupV1 {
    #[must_use]
    pub const fn committed_source_id(&self) -> &SourceId {
        &self.committed_source_id
    }

    #[must_use]
    pub const fn trade_source_id(&self) -> &SourceId {
        &self.trade_source_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeNativeCursorV1 {
    epoch: BoundedIdentity,
    position: NodeNativePositionV1,
    content_blake3: Blake3Digest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum NodeNativePositionV1 {
    BlockHeight { height: u64 },
    ByteOffset { end_offset: u64 },
}

impl NodeNativeCursorV1 {
    #[must_use]
    pub const fn epoch(&self) -> &BoundedIdentity {
        &self.epoch
    }

    #[must_use]
    pub const fn position(&self) -> NodeNativePositionV1 {
        self.position
    }

    #[must_use]
    pub const fn content_blake3(&self) -> Blake3Digest {
        self.content_blake3
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeRecordingFileV1 {
    relative_path: String,
    role: NodeRecordingFileRoleV1,
    rotation_sequence: u64,
    size_bytes: u64,
    sha256: Sha256Digest,
    first_cursor: NodeNativeCursorV1,
    last_cursor: NodeNativeCursorV1,
}

impl NodeRecordingFileV1 {
    #[must_use]
    pub fn relative_path(&self) -> &str {
        &self.relative_path
    }

    #[must_use]
    pub const fn role(&self) -> NodeRecordingFileRoleV1 {
        self.role
    }

    #[must_use]
    pub const fn rotation_sequence(&self) -> u64 {
        self.rotation_sequence
    }

    #[must_use]
    pub const fn size_bytes(&self) -> u64 {
        self.size_bytes
    }

    #[must_use]
    pub const fn sha256(&self) -> Sha256Digest {
        self.sha256
    }

    #[must_use]
    pub const fn first_cursor(&self) -> &NodeNativeCursorV1 {
        &self.first_cursor
    }

    #[must_use]
    pub const fn last_cursor(&self) -> &NodeNativeCursorV1 {
        &self.last_cursor
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeArtifactV1 {
    name: BoundedIdentity,
    version: BoundedIdentity,
    repository_commit: BoundedIdentity,
    build_argv: Vec<String>,
    binary_sha256: Sha256Digest,
    build_material_sha256: Sha256Digest,
    signature_fingerprint: BoundedIdentity,
    signature_material_sha256: Sha256Digest,
}

impl NodeArtifactV1 {
    #[must_use]
    pub const fn name(&self) -> &BoundedIdentity {
        &self.name
    }

    #[must_use]
    pub const fn version(&self) -> &BoundedIdentity {
        &self.version
    }

    #[must_use]
    pub const fn repository_commit(&self) -> &BoundedIdentity {
        &self.repository_commit
    }

    #[must_use]
    pub fn build_argv(&self) -> &[String] {
        &self.build_argv
    }

    #[must_use]
    pub const fn binary_sha256(&self) -> Sha256Digest {
        self.binary_sha256
    }

    #[must_use]
    pub const fn build_material_sha256(&self) -> Sha256Digest {
        self.build_material_sha256
    }

    #[must_use]
    pub const fn signature_fingerprint(&self) -> &BoundedIdentity {
        &self.signature_fingerprint
    }

    #[must_use]
    pub const fn signature_material_sha256(&self) -> Sha256Digest {
        self.signature_material_sha256
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeCaptureContractV1 {
    argv: Vec<String>,
    output_file_buffering: OutputFileBufferingV1,
    production_recording: bool,
    same_node_instance: bool,
    byte_exact: bool,
    corpus_coverage_complete: bool,
    runtime_material_sha256: Sha256Digest,
}

impl NodeCaptureContractV1 {
    #[must_use]
    pub fn argv(&self) -> &[String] {
        &self.argv
    }

    #[must_use]
    pub const fn output_file_buffering(&self) -> OutputFileBufferingV1 {
        self.output_file_buffering
    }

    #[must_use]
    pub const fn production_recording(&self) -> bool {
        self.production_recording
    }

    #[must_use]
    pub const fn same_node_instance(&self) -> bool {
        self.same_node_instance
    }

    #[must_use]
    pub const fn byte_exact(&self) -> bool {
        self.byte_exact
    }

    #[must_use]
    pub const fn corpus_coverage_complete(&self) -> bool {
        self.corpus_coverage_complete
    }

    #[must_use]
    pub const fn runtime_material_sha256(&self) -> Sha256Digest {
        self.runtime_material_sha256
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeQualificationProfileV1 {
    qualification_profile: BoundedIdentity,
    committed_parser_version: BoundedIdentity,
    committed_parser_material_sha256: Sha256Digest,
    trade_parser_version: BoundedIdentity,
    trade_parser_material_sha256: Sha256Digest,
    mapper_version: BoundedIdentity,
    mapper_material_sha256: Sha256Digest,
    catalog_version: BoundedIdentity,
    catalog_sha256: Sha256Digest,
    time_normalization_rule: BoundedIdentity,
    time_normalization_material_sha256: Sha256Digest,
}

impl NodeQualificationProfileV1 {
    #[must_use]
    pub const fn qualification_profile(&self) -> &BoundedIdentity {
        &self.qualification_profile
    }

    #[must_use]
    pub const fn committed_parser_version(&self) -> &BoundedIdentity {
        &self.committed_parser_version
    }

    #[must_use]
    pub const fn committed_parser_material_sha256(&self) -> Sha256Digest {
        self.committed_parser_material_sha256
    }

    #[must_use]
    pub const fn trade_parser_version(&self) -> &BoundedIdentity {
        &self.trade_parser_version
    }

    #[must_use]
    pub const fn trade_parser_material_sha256(&self) -> Sha256Digest {
        self.trade_parser_material_sha256
    }

    #[must_use]
    pub const fn mapper_version(&self) -> &BoundedIdentity {
        &self.mapper_version
    }

    #[must_use]
    pub const fn mapper_material_sha256(&self) -> Sha256Digest {
        self.mapper_material_sha256
    }

    #[must_use]
    pub const fn catalog_version(&self) -> &BoundedIdentity {
        &self.catalog_version
    }

    #[must_use]
    pub const fn catalog_sha256(&self) -> Sha256Digest {
        self.catalog_sha256
    }

    #[must_use]
    pub const fn time_normalization_rule(&self) -> &BoundedIdentity {
        &self.time_normalization_rule
    }

    #[must_use]
    pub const fn time_normalization_material_sha256(&self) -> Sha256Digest {
        self.time_normalization_material_sha256
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeSourceQualificationManifestV1 {
    recording_id: BoundedIdentity,
    chain_id: ChainId,
    node_instance_id: BoundedIdentity,
    source_group: NodeSourceGroupV1,
    artifact: NodeArtifactV1,
    capture: NodeCaptureContractV1,
    profile: NodeQualificationProfileV1,
    redistribution: RedistributionClassV1,
    files: Vec<NodeRecordingFileV1>,
    canonical_bytes: Vec<u8>,
    manifest_sha256: Sha256Digest,
}

impl NodeSourceQualificationManifestV1 {
    #[must_use]
    pub const fn recording_id(&self) -> &BoundedIdentity {
        &self.recording_id
    }

    #[must_use]
    pub const fn chain_id(&self) -> &ChainId {
        &self.chain_id
    }

    #[must_use]
    pub const fn node_instance_id(&self) -> &BoundedIdentity {
        &self.node_instance_id
    }

    #[must_use]
    pub const fn source_group(&self) -> &NodeSourceGroupV1 {
        &self.source_group
    }

    #[must_use]
    pub const fn artifact(&self) -> &NodeArtifactV1 {
        &self.artifact
    }

    #[must_use]
    pub const fn capture(&self) -> &NodeCaptureContractV1 {
        &self.capture
    }

    #[must_use]
    pub const fn profile(&self) -> &NodeQualificationProfileV1 {
        &self.profile
    }

    #[must_use]
    pub const fn redistribution(&self) -> RedistributionClassV1 {
        self.redistribution
    }

    #[must_use]
    pub fn files(&self) -> &[NodeRecordingFileV1] {
        &self.files
    }

    #[must_use]
    pub fn canonical_bytes(&self) -> &[u8] {
        &self.canonical_bytes
    }

    #[must_use]
    pub const fn manifest_sha256(&self) -> Sha256Digest {
        self.manifest_sha256
    }
}

/// Authority token for the future joined-trade admission path.
///
/// Its fields are private and the production constructor checks the built-in
/// registry. Merely decoding a manifest can never construct this value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualifiedNodeSourceV1 {
    manifest_sha256: Sha256Digest,
    chain_id: ChainId,
    node_instance_id: BoundedIdentity,
    source_group: NodeSourceGroupV1,
}

impl QualifiedNodeSourceV1 {
    #[must_use]
    pub const fn manifest_sha256(&self) -> Sha256Digest {
        self.manifest_sha256
    }

    #[must_use]
    pub const fn chain_id(&self) -> &ChainId {
        &self.chain_id
    }

    #[must_use]
    pub const fn node_instance_id(&self) -> &BoundedIdentity {
        &self.node_instance_id
    }

    #[must_use]
    pub const fn source_group(&self) -> &NodeSourceGroupV1 {
        &self.source_group
    }
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum NodeSourceQualificationError {
    #[error("qualification manifest is empty")]
    EmptyManifest,
    #[error("qualification manifest exceeds the byte limit")]
    ManifestTooLarge,
    #[error("qualification manifest SHA-256 does not match")]
    ManifestDigestMismatch,
    #[error("qualification manifest is invalid")]
    InvalidManifest,
    #[error("qualification manifest JSON is not canonical")]
    NoncanonicalManifest,
    #[error("qualification digest is invalid")]
    InvalidDigest,
    #[error("source profile is not present in the built-in registry")]
    UnqualifiedSourceProfile,
}

impl NodeSourceQualificationError {
    #[must_use]
    pub const fn reason_code(&self) -> &'static str {
        match self {
            Self::EmptyManifest => "source_join.empty_qualification_manifest",
            Self::ManifestTooLarge => "source_join.qualification_manifest_too_large",
            Self::ManifestDigestMismatch => "source_join.qualification_manifest_digest_mismatch",
            Self::InvalidManifest | Self::InvalidDigest => {
                "source_join.invalid_qualification_manifest"
            }
            Self::NoncanonicalManifest => "source_join.noncanonical_qualification_manifest",
            Self::UnqualifiedSourceProfile => "source_join.unqualified_source_profile",
        }
    }

    #[must_use]
    pub const fn disposition(&self) -> ErrorDisposition {
        match self {
            Self::ManifestTooLarge | Self::UnqualifiedSourceProfile => ErrorDisposition::Stop,
            Self::EmptyManifest
            | Self::ManifestDigestMismatch
            | Self::InvalidManifest
            | Self::NoncanonicalManifest
            | Self::InvalidDigest => ErrorDisposition::Quarantine,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireManifestV1 {
    schema: String,
    recording_id: String,
    chain_id: String,
    node_instance_id: String,
    source_group: WireSourceGroupV1,
    artifact: WireArtifactV1,
    capture: WireCaptureV1,
    profile: WireProfileV1,
    redistribution: RedistributionClassV1,
    files: Vec<WireRecordingFileV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireSourceGroupV1 {
    committed_source_id: String,
    trade_source_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireArtifactV1 {
    name: String,
    version: String,
    repository_commit: String,
    build_argv: Vec<String>,
    binary_sha256: String,
    build_material_sha256: String,
    signature_fingerprint: String,
    signature_material_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireCaptureV1 {
    argv: Vec<String>,
    output_file_buffering: OutputFileBufferingV1,
    production_recording: bool,
    same_node_instance: bool,
    byte_exact: bool,
    corpus_coverage_complete: bool,
    runtime_material_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireProfileV1 {
    qualification_profile: String,
    committed_parser_version: String,
    committed_parser_material_sha256: String,
    trade_parser_version: String,
    trade_parser_material_sha256: String,
    mapper_version: String,
    mapper_material_sha256: String,
    catalog_version: String,
    catalog_sha256: String,
    time_normalization_rule: String,
    time_normalization_material_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireRecordingFileV1 {
    relative_path: String,
    role: NodeRecordingFileRoleV1,
    rotation_sequence: u64,
    size_bytes: u64,
    sha256: String,
    first_cursor: WireNativeCursorV1,
    last_cursor: WireNativeCursorV1,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireNativeCursorV1 {
    epoch: String,
    position: NodeNativePositionV1,
    content_blake3: String,
}

pub fn decode_node_source_qualification_manifest_v1(
    bytes: &[u8],
    expected_sha256: Sha256Digest,
) -> Result<NodeSourceQualificationManifestV1, NodeSourceQualificationError> {
    if bytes.is_empty() {
        return Err(NodeSourceQualificationError::EmptyManifest);
    }
    if bytes.len() > MAX_QUALIFICATION_MANIFEST_BYTES {
        return Err(NodeSourceQualificationError::ManifestTooLarge);
    }
    let actual_sha256 = sha256(bytes);
    if actual_sha256 != expected_sha256 {
        return Err(NodeSourceQualificationError::ManifestDigestMismatch);
    }

    let wire: WireManifestV1 =
        serde_json::from_slice(bytes).map_err(|_| NodeSourceQualificationError::InvalidManifest)?;
    let canonical =
        serde_json::to_vec(&wire).map_err(|_| NodeSourceQualificationError::InvalidManifest)?;
    if canonical != bytes {
        return Err(NodeSourceQualificationError::NoncanonicalManifest);
    }
    validate_wire(wire, canonical, actual_sha256)
}

pub fn qualify_node_source_v1(
    bytes: &[u8],
    expected_sha256: Sha256Digest,
) -> Result<QualifiedNodeSourceV1, NodeSourceQualificationError> {
    let manifest = decode_node_source_qualification_manifest_v1(bytes, expected_sha256)?;
    qualify_against_registry(&manifest, BUILTIN_QUALIFIED_MANIFESTS)
}

// Intentionally empty until M4 retains and independently reviews a byte-exact
// same-build operator corpus. This constant is the only production authority.
const BUILTIN_QUALIFIED_MANIFESTS: &[Sha256Digest] = &[];

fn qualify_against_registry(
    manifest: &NodeSourceQualificationManifestV1,
    registry: &[Sha256Digest],
) -> Result<QualifiedNodeSourceV1, NodeSourceQualificationError> {
    if !registry.contains(&manifest.manifest_sha256) {
        return Err(NodeSourceQualificationError::UnqualifiedSourceProfile);
    }
    Ok(QualifiedNodeSourceV1 {
        manifest_sha256: manifest.manifest_sha256,
        chain_id: manifest.chain_id.clone(),
        node_instance_id: manifest.node_instance_id.clone(),
        source_group: manifest.source_group.clone(),
    })
}

fn validate_wire(
    wire: WireManifestV1,
    canonical_bytes: Vec<u8>,
    manifest_sha256: Sha256Digest,
) -> Result<NodeSourceQualificationManifestV1, NodeSourceQualificationError> {
    if wire.schema != SCHEMA_V1 {
        return Err(NodeSourceQualificationError::InvalidManifest);
    }
    let recording_id = BoundedIdentity::new(wire.recording_id)?;
    validate_text(&wire.chain_id, MAX_IDENTITY_BYTES)?;
    let chain_id =
        ChainId::new(wire.chain_id).map_err(|_| NodeSourceQualificationError::InvalidManifest)?;
    let node_instance_id = BoundedIdentity::new(wire.node_instance_id)?;
    let source_group = source_group(wire.source_group)?;
    let artifact = artifact(wire.artifact)?;
    let capture = capture(wire.capture)?;
    let profile = profile(wire.profile)?;
    let files = recording_files(wire.files)?;

    Ok(NodeSourceQualificationManifestV1 {
        recording_id,
        chain_id,
        node_instance_id,
        source_group,
        artifact,
        capture,
        profile,
        redistribution: wire.redistribution,
        files,
        canonical_bytes,
        manifest_sha256,
    })
}

fn source_group(
    wire: WireSourceGroupV1,
) -> Result<NodeSourceGroupV1, NodeSourceQualificationError> {
    validate_text(&wire.committed_source_id, MAX_IDENTITY_BYTES)?;
    validate_text(&wire.trade_source_id, MAX_IDENTITY_BYTES)?;
    if wire.committed_source_id == wire.trade_source_id {
        return Err(NodeSourceQualificationError::InvalidManifest);
    }
    Ok(NodeSourceGroupV1 {
        committed_source_id: SourceId::new(wire.committed_source_id)
            .map_err(|_| NodeSourceQualificationError::InvalidManifest)?,
        trade_source_id: SourceId::new(wire.trade_source_id)
            .map_err(|_| NodeSourceQualificationError::InvalidManifest)?,
    })
}

fn artifact(wire: WireArtifactV1) -> Result<NodeArtifactV1, NodeSourceQualificationError> {
    validate_arguments(&wire.build_argv)?;
    validate_repository_commit(&wire.repository_commit)?;
    Ok(NodeArtifactV1 {
        name: BoundedIdentity::new(wire.name)?,
        version: BoundedIdentity::new(wire.version)?,
        repository_commit: BoundedIdentity::new(wire.repository_commit)?,
        build_argv: wire.build_argv,
        binary_sha256: Sha256Digest::parse_lower_hex(&wire.binary_sha256)?,
        build_material_sha256: Sha256Digest::parse_lower_hex(&wire.build_material_sha256)?,
        signature_fingerprint: BoundedIdentity::new(wire.signature_fingerprint)?,
        signature_material_sha256: Sha256Digest::parse_lower_hex(&wire.signature_material_sha256)?,
    })
}

fn capture(wire: WireCaptureV1) -> Result<NodeCaptureContractV1, NodeSourceQualificationError> {
    validate_arguments(&wire.argv)?;
    require_flag_once(&wire.argv, "--write-trades")?;
    reject_flag(&wire.argv, "--write-fills")?;
    require_flag_once(&wire.argv, "--batch-by-block")?;
    require_flag_value_once(&wire.argv, "--replica-cmds-style", "actions-and-responses")?;
    let disable_count = wire
        .argv
        .iter()
        .filter(|argument| argument.as_str() == "--disable-output-file-buffering")
        .count();
    let buffering_matches = match wire.output_file_buffering {
        OutputFileBufferingV1::Disabled => disable_count == 1,
        OutputFileBufferingV1::Enabled => disable_count == 0,
    };
    if !buffering_matches {
        return Err(NodeSourceQualificationError::InvalidManifest);
    }
    Ok(NodeCaptureContractV1 {
        argv: wire.argv,
        output_file_buffering: wire.output_file_buffering,
        production_recording: wire.production_recording,
        same_node_instance: wire.same_node_instance,
        byte_exact: wire.byte_exact,
        corpus_coverage_complete: wire.corpus_coverage_complete,
        runtime_material_sha256: Sha256Digest::parse_lower_hex(&wire.runtime_material_sha256)?,
    })
}

fn profile(
    wire: WireProfileV1,
) -> Result<NodeQualificationProfileV1, NodeSourceQualificationError> {
    Ok(NodeQualificationProfileV1 {
        qualification_profile: BoundedIdentity::new(wire.qualification_profile)?,
        committed_parser_version: BoundedIdentity::new(wire.committed_parser_version)?,
        committed_parser_material_sha256: Sha256Digest::parse_lower_hex(
            &wire.committed_parser_material_sha256,
        )?,
        trade_parser_version: BoundedIdentity::new(wire.trade_parser_version)?,
        trade_parser_material_sha256: Sha256Digest::parse_lower_hex(
            &wire.trade_parser_material_sha256,
        )?,
        mapper_version: BoundedIdentity::new(wire.mapper_version)?,
        mapper_material_sha256: Sha256Digest::parse_lower_hex(&wire.mapper_material_sha256)?,
        catalog_version: BoundedIdentity::new(wire.catalog_version)?,
        catalog_sha256: Sha256Digest::parse_lower_hex(&wire.catalog_sha256)?,
        time_normalization_rule: BoundedIdentity::new(wire.time_normalization_rule)?,
        time_normalization_material_sha256: Sha256Digest::parse_lower_hex(
            &wire.time_normalization_material_sha256,
        )?,
    })
}

fn recording_files(
    wires: Vec<WireRecordingFileV1>,
) -> Result<Vec<NodeRecordingFileV1>, NodeSourceQualificationError> {
    if wires.is_empty() || wires.len() > MAX_RECORDING_FILES {
        return Err(NodeSourceQualificationError::InvalidManifest);
    }
    let mut paths = BTreeSet::new();
    let mut role_sequences = BTreeSet::new();
    let mut roles = BTreeSet::new();
    let mut previous_key = None;
    let mut files = Vec::with_capacity(wires.len());
    for wire in wires {
        validate_relative_path(&wire.relative_path)?;
        if wire.size_bytes == 0
            || !paths.insert(wire.relative_path.clone())
            || !role_sequences.insert((wire.role, wire.rotation_sequence))
        {
            return Err(NodeSourceQualificationError::InvalidManifest);
        }
        let key = (
            wire.role,
            wire.rotation_sequence,
            wire.relative_path.clone(),
        );
        if previous_key
            .as_ref()
            .is_some_and(|previous| previous >= &key)
        {
            return Err(NodeSourceQualificationError::InvalidManifest);
        }
        previous_key = Some(key);
        roles.insert(wire.role);
        let first_cursor = native_cursor(wire.first_cursor)?;
        let last_cursor = native_cursor(wire.last_cursor)?;
        if first_cursor.epoch != last_cursor.epoch
            || !valid_cursor_range(wire.role, &first_cursor, &last_cursor, wire.size_bytes)
        {
            return Err(NodeSourceQualificationError::InvalidManifest);
        }
        files.push(NodeRecordingFileV1 {
            relative_path: wire.relative_path,
            role: wire.role,
            rotation_sequence: wire.rotation_sequence,
            size_bytes: wire.size_bytes,
            sha256: Sha256Digest::parse_lower_hex(&wire.sha256)?,
            first_cursor,
            last_cursor,
        });
    }
    if !roles.contains(&NodeRecordingFileRoleV1::Committed)
        || !roles.contains(&NodeRecordingFileRoleV1::Trade)
    {
        return Err(NodeSourceQualificationError::InvalidManifest);
    }
    Ok(files)
}

fn native_cursor(
    wire: WireNativeCursorV1,
) -> Result<NodeNativeCursorV1, NodeSourceQualificationError> {
    Ok(NodeNativeCursorV1 {
        epoch: BoundedIdentity::new(wire.epoch)?,
        position: wire.position,
        content_blake3: Blake3Digest::parse_lower_hex(&wire.content_blake3)?,
    })
}

fn valid_cursor_range(
    role: NodeRecordingFileRoleV1,
    first: &NodeNativeCursorV1,
    last: &NodeNativeCursorV1,
    size_bytes: u64,
) -> bool {
    match role {
        NodeRecordingFileRoleV1::Committed
        | NodeRecordingFileRoleV1::AbciSnapshot
        | NodeRecordingFileRoleV1::L4Snapshot => match (first.position, last.position) {
            (
                NodeNativePositionV1::BlockHeight { height: first },
                NodeNativePositionV1::BlockHeight { height: last },
            ) => first <= last,
            (NodeNativePositionV1::BlockHeight { .. }, NodeNativePositionV1::ByteOffset { .. })
            | (NodeNativePositionV1::ByteOffset { .. }, NodeNativePositionV1::BlockHeight { .. })
            | (NodeNativePositionV1::ByteOffset { .. }, NodeNativePositionV1::ByteOffset { .. }) => {
                false
            }
        },
        NodeRecordingFileRoleV1::Trade => match (first.position, last.position) {
            (
                NodeNativePositionV1::ByteOffset { end_offset: first },
                NodeNativePositionV1::ByteOffset { end_offset: last },
            ) => first > 0 && first <= last && last <= size_bytes,
            (
                NodeNativePositionV1::BlockHeight { .. },
                NodeNativePositionV1::BlockHeight { .. },
            )
            | (NodeNativePositionV1::BlockHeight { .. }, NodeNativePositionV1::ByteOffset { .. })
            | (NodeNativePositionV1::ByteOffset { .. }, NodeNativePositionV1::BlockHeight { .. }) => {
                false
            }
        },
    }
}

fn validate_arguments(arguments: &[String]) -> Result<(), NodeSourceQualificationError> {
    if arguments.is_empty() || arguments.len() > MAX_ARGUMENTS {
        return Err(NodeSourceQualificationError::InvalidManifest);
    }
    for argument in arguments {
        validate_text(argument, MAX_ARGUMENT_BYTES)?;
    }
    Ok(())
}

fn require_flag_once(arguments: &[String], flag: &str) -> Result<(), NodeSourceQualificationError> {
    if arguments
        .iter()
        .filter(|argument| argument.as_str() == flag)
        .count()
        != 1
    {
        return Err(NodeSourceQualificationError::InvalidManifest);
    }
    Ok(())
}

fn reject_flag(arguments: &[String], flag: &str) -> Result<(), NodeSourceQualificationError> {
    if arguments.iter().any(|argument| argument == flag) {
        return Err(NodeSourceQualificationError::InvalidManifest);
    }
    Ok(())
}

fn require_flag_value_once(
    arguments: &[String],
    flag: &str,
    value: &str,
) -> Result<(), NodeSourceQualificationError> {
    let positions = arguments
        .iter()
        .enumerate()
        .filter_map(|(index, argument)| (argument == flag).then_some(index))
        .collect::<Vec<_>>();
    if positions.len() != 1 || arguments.get(positions[0] + 1).map(String::as_str) != Some(value) {
        return Err(NodeSourceQualificationError::InvalidManifest);
    }
    Ok(())
}

fn validate_relative_path(value: &str) -> Result<(), NodeSourceQualificationError> {
    if value.is_empty()
        || value.len() > MAX_RELATIVE_PATH_BYTES
        || value.contains('\\')
        || value.chars().any(char::is_control)
        || Path::new(value).is_absolute()
        || value
            .split('/')
            .any(|component| component.is_empty() || component == "." || component == "..")
    {
        return Err(NodeSourceQualificationError::InvalidManifest);
    }
    Ok(())
}

fn validate_repository_commit(value: &str) -> Result<(), NodeSourceQualificationError> {
    if !matches!(value.len(), 40 | 64)
        || value
            .bytes()
            .any(|byte| !byte.is_ascii_digit() && !(b'a'..=b'f').contains(&byte))
    {
        return Err(NodeSourceQualificationError::InvalidManifest);
    }
    Ok(())
}

fn validate_text(value: &str, maximum: usize) -> Result<(), NodeSourceQualificationError> {
    if value.is_empty()
        || value.trim() != value
        || value.len() > maximum
        || value.chars().any(char::is_control)
    {
        return Err(NodeSourceQualificationError::InvalidManifest);
    }
    Ok(())
}

fn sha256(bytes: &[u8]) -> Sha256Digest {
    Sha256Digest::from_bytes(Sha256::digest(bytes).into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_registry_is_the_only_token_constructor() {
        let mut wire = WireManifestV1 {
            schema: SCHEMA_V1.to_owned(),
            recording_id: "recording-a".to_owned(),
            chain_id: "hyperliquid-mainnet".to_owned(),
            node_instance_id: "node-a".to_owned(),
            source_group: WireSourceGroupV1 {
                committed_source_id: "node-a-committed".to_owned(),
                trade_source_id: "node-a-trades".to_owned(),
            },
            artifact: WireArtifactV1 {
                name: "hyperliquid-node".to_owned(),
                version: "v1".to_owned(),
                repository_commit: "0123456789abcdef0123456789abcdef01234567".to_owned(),
                build_argv: vec!["cargo".to_owned(), "build".to_owned()],
                binary_sha256: "11".repeat(32),
                build_material_sha256: "22".repeat(32),
                signature_fingerprint: "0123456789ABCDEF0123456789ABCDEF01234567".to_owned(),
                signature_material_sha256: "33".repeat(32),
            },
            capture: WireCaptureV1 {
                argv: vec![
                    "hl-node".to_owned(),
                    "--write-trades".to_owned(),
                    "--batch-by-block".to_owned(),
                    "--replica-cmds-style".to_owned(),
                    "actions-and-responses".to_owned(),
                    "--disable-output-file-buffering".to_owned(),
                ],
                output_file_buffering: OutputFileBufferingV1::Disabled,
                production_recording: true,
                same_node_instance: true,
                byte_exact: true,
                corpus_coverage_complete: true,
                runtime_material_sha256: "44".repeat(32),
            },
            profile: WireProfileV1 {
                qualification_profile: "profile-v1".to_owned(),
                committed_parser_version: "committed-v1".to_owned(),
                committed_parser_material_sha256: "55".repeat(32),
                trade_parser_version: "trade-v1".to_owned(),
                trade_parser_material_sha256: "66".repeat(32),
                mapper_version: "mapper-v1".to_owned(),
                mapper_material_sha256: "77".repeat(32),
                catalog_version: "catalog-v1".to_owned(),
                catalog_sha256: "88".repeat(32),
                time_normalization_rule: "time-v1".to_owned(),
                time_normalization_material_sha256: "89".repeat(32),
            },
            redistribution: RedistributionClassV1::PrivateOperatorEvidence,
            files: vec![
                file_wire(NodeRecordingFileRoleV1::Committed, "committed/0", "99"),
                file_wire(NodeRecordingFileRoleV1::Trade, "trades/0", "aa"),
            ],
        };
        let bytes = serde_json::to_vec(&wire).expect("wire JSON");
        let digest = sha256(&bytes);
        let manifest = decode_node_source_qualification_manifest_v1(&bytes, digest)
            .expect("valid private fixture");

        assert!(qualify_against_registry(&manifest, &[]).is_err());
        let token = qualify_against_registry(&manifest, &[digest]).expect("private registry token");
        assert_eq!(token.manifest_sha256(), digest);

        wire.profile.catalog_version = "catalog-v2".to_owned();
        let changed_bytes = serde_json::to_vec(&wire).expect("changed wire JSON");
        let changed_digest = sha256(&changed_bytes);
        let changed = decode_node_source_qualification_manifest_v1(&changed_bytes, changed_digest)
            .expect("valid changed claim");
        let error = qualify_against_registry(&changed, &[digest])
            .expect_err("registry must bind the complete original manifest digest");
        assert_eq!(
            error.reason_code(),
            "source_join.unqualified_source_profile"
        );
    }

    fn native_test_cursor(position: NodeNativePositionV1) -> NodeNativeCursorV1 {
        NodeNativeCursorV1 {
            epoch: BoundedIdentity::new("epoch-a").expect("epoch"),
            position,
            content_blake3: Blake3Digest::from_bytes([0x11; 32]),
        }
    }

    fn every_recording_file_role() -> [NodeRecordingFileRoleV1; 4] {
        [
            NodeRecordingFileRoleV1::Committed,
            NodeRecordingFileRoleV1::Trade,
            NodeRecordingFileRoleV1::AbciSnapshot,
            NodeRecordingFileRoleV1::L4Snapshot,
        ]
    }

    fn every_native_position(value: u64) -> [NodeNativePositionV1; 2] {
        [
            NodeNativePositionV1::BlockHeight { height: value },
            NodeNativePositionV1::ByteOffset { end_offset: value },
        ]
    }

    fn expected_valid_cursor_range(
        role: NodeRecordingFileRoleV1,
        first: NodeNativePositionV1,
        last: NodeNativePositionV1,
        size_bytes: u64,
    ) -> bool {
        match role {
            NodeRecordingFileRoleV1::Committed
            | NodeRecordingFileRoleV1::AbciSnapshot
            | NodeRecordingFileRoleV1::L4Snapshot => match (first, last) {
                (
                    NodeNativePositionV1::BlockHeight { height: first },
                    NodeNativePositionV1::BlockHeight { height: last },
                ) => first <= last,
                (
                    NodeNativePositionV1::BlockHeight { .. },
                    NodeNativePositionV1::ByteOffset { .. },
                )
                | (
                    NodeNativePositionV1::ByteOffset { .. },
                    NodeNativePositionV1::BlockHeight { .. },
                )
                | (
                    NodeNativePositionV1::ByteOffset { .. },
                    NodeNativePositionV1::ByteOffset { .. },
                ) => false,
            },
            NodeRecordingFileRoleV1::Trade => match (first, last) {
                (
                    NodeNativePositionV1::ByteOffset { end_offset: first },
                    NodeNativePositionV1::ByteOffset { end_offset: last },
                ) => first > 0 && first <= last && last <= size_bytes,
                (
                    NodeNativePositionV1::BlockHeight { .. },
                    NodeNativePositionV1::BlockHeight { .. },
                )
                | (
                    NodeNativePositionV1::BlockHeight { .. },
                    NodeNativePositionV1::ByteOffset { .. },
                )
                | (
                    NodeNativePositionV1::ByteOffset { .. },
                    NodeNativePositionV1::BlockHeight { .. },
                ) => false,
            },
        }
    }

    #[test]
    fn valid_cursor_range_pins_every_role_and_position_kind() {
        const SIZE_BYTES: u64 = 100;
        for role in every_recording_file_role() {
            match role {
                NodeRecordingFileRoleV1::Committed
                | NodeRecordingFileRoleV1::AbciSnapshot
                | NodeRecordingFileRoleV1::L4Snapshot => {}
                NodeRecordingFileRoleV1::Trade => {}
            }
            for first in every_native_position(10) {
                match first {
                    NodeNativePositionV1::BlockHeight { .. }
                    | NodeNativePositionV1::ByteOffset { .. } => {}
                }
                for last in every_native_position(20) {
                    match last {
                        NodeNativePositionV1::BlockHeight { .. }
                        | NodeNativePositionV1::ByteOffset { .. } => {}
                    }
                    let first_cursor = native_test_cursor(first);
                    let last_cursor = native_test_cursor(last);
                    assert_eq!(
                        valid_cursor_range(role, &first_cursor, &last_cursor, SIZE_BYTES),
                        expected_valid_cursor_range(role, first, last, SIZE_BYTES),
                    );
                }
            }
        }
    }

    #[test]
    fn valid_cursor_range_keeps_existing_role_bounds() {
        let committed = |first, last| {
            valid_cursor_range(
                NodeRecordingFileRoleV1::Committed,
                &native_test_cursor(NodeNativePositionV1::BlockHeight { height: first }),
                &native_test_cursor(NodeNativePositionV1::BlockHeight { height: last }),
                100,
            )
        };
        assert!(committed(0, 0));
        assert!(committed(10, 20));
        assert!(!committed(21, 20));

        let trade = |first, last, size| {
            valid_cursor_range(
                NodeRecordingFileRoleV1::Trade,
                &native_test_cursor(NodeNativePositionV1::ByteOffset { end_offset: first }),
                &native_test_cursor(NodeNativePositionV1::ByteOffset { end_offset: last }),
                size,
            )
        };
        assert!(!trade(0, 10, 100));
        assert!(trade(1, 10, 100));
        assert!(trade(10, 10, 10));
        assert!(!trade(10, 9, 100));
        assert!(!trade(1, 101, 100));
    }

    #[test]
    fn snapshot_file_roles_wire_as_kebab_case() {
        assert_eq!(
            serde_json::to_value(NodeRecordingFileRoleV1::AbciSnapshot).expect("json"),
            serde_json::json!("abci-snapshot")
        );
        assert_eq!(
            serde_json::to_value(NodeRecordingFileRoleV1::L4Snapshot).expect("json"),
            serde_json::json!("l4-snapshot")
        );
    }

    fn file_wire(role: NodeRecordingFileRoleV1, path: &str, digest: &str) -> WireRecordingFileV1 {
        WireRecordingFileV1 {
            relative_path: path.to_owned(),
            role,
            rotation_sequence: 0,
            size_bytes: 1,
            sha256: digest.repeat(32),
            first_cursor: WireNativeCursorV1 {
                epoch: format!("{path}-epoch"),
                position: match role {
                    NodeRecordingFileRoleV1::Committed
                    | NodeRecordingFileRoleV1::AbciSnapshot
                    | NodeRecordingFileRoleV1::L4Snapshot => {
                        NodeNativePositionV1::BlockHeight { height: 0 }
                    }
                    NodeRecordingFileRoleV1::Trade => {
                        NodeNativePositionV1::ByteOffset { end_offset: 1 }
                    }
                },
                content_blake3: "bb".repeat(32),
            },
            last_cursor: WireNativeCursorV1 {
                epoch: format!("{path}-epoch"),
                position: match role {
                    NodeRecordingFileRoleV1::Committed
                    | NodeRecordingFileRoleV1::AbciSnapshot
                    | NodeRecordingFileRoleV1::L4Snapshot => {
                        NodeNativePositionV1::BlockHeight { height: 0 }
                    }
                    NodeRecordingFileRoleV1::Trade => {
                        NodeNativePositionV1::ByteOffset { end_offset: 1 }
                    }
                },
                content_blake3: "cc".repeat(32),
            },
        }
    }
}
