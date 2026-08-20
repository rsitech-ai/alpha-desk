#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const MANIFEST_RELATIVE: &str = "config/hyperliquid/capabilities.toml";
pub const MATRIX_RELATIVE: &str = "docs/hyperliquid/coverage-matrix.md";
const SCHEMA_VERSION: u32 = 1;
const KNOWN_NETWORKS: [&str; 2] = ["mainnet", "testnet"];

pub const REST_INFO_WEIGHT_2: &[&str] = &[
    "allMids",
    "clearinghouseState",
    "exchangeStatus",
    "l2Book",
    "orderStatus",
    "spotClearinghouseState",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Implemented,
    ImplementedUnqualified,
    QualifiedLive,
    QualifiedReplay,
    Degraded,
    Partial,
    Planned,
    Unsupported,
    UnsupportedByNetwork,
    SourceUnavailable,
    SchemaUnknown,
    DisabledByPolicy,
}

impl Status {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Implemented => "implemented",
            Self::ImplementedUnqualified => "implemented_unqualified",
            Self::QualifiedLive => "qualified_live",
            Self::QualifiedReplay => "qualified_replay",
            Self::Degraded => "degraded",
            Self::Partial => "partial",
            Self::Planned => "planned",
            Self::Unsupported => "unsupported",
            Self::UnsupportedByNetwork => "unsupported_by_network",
            Self::SourceUnavailable => "source_unavailable",
            Self::SchemaUnknown => "schema_unknown",
            Self::DisabledByPolicy => "disabled_by_policy",
        }
    }

    #[must_use]
    pub const fn requires_fixture_set(self) -> bool {
        match self {
            Self::Implemented
            | Self::ImplementedUnqualified
            | Self::QualifiedLive
            | Self::QualifiedReplay => true,
            Self::Degraded
            | Self::Partial
            | Self::Planned
            | Self::Unsupported
            | Self::UnsupportedByNetwork
            | Self::SourceUnavailable
            | Self::SchemaUnknown
            | Self::DisabledByPolicy => false,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceRole {
    Committed,
    Provisional,
    Reconciliation,
    Enrichment,
    #[serde(rename = "evidence-only")]
    EvidenceOnly,
}

impl SourceRole {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Committed => "committed",
            Self::Provisional => "provisional",
            Self::Reconciliation => "reconciliation",
            Self::Enrichment => "enrichment",
            Self::EvidenceOnly => "evidence-only",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StateTarget {
    CommittedState,
    ReconciledSnapshot,
    CanonicalState,
    L4Book,
    PositionState,
    CanonicalEvent,
    ReferenceSnapshot,
}

impl StateTarget {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CommittedState => "committed_state",
            Self::ReconciledSnapshot => "reconciled_snapshot",
            Self::CanonicalState => "canonical_state",
            Self::L4Book => "l4_book",
            Self::PositionState => "position_state",
            Self::CanonicalEvent => "canonical_event",
            Self::ReferenceSnapshot => "reference_snapshot",
        }
    }

    #[must_use]
    pub const fn is_state_affecting(self) -> bool {
        match self {
            Self::CommittedState
            | Self::ReconciledSnapshot
            | Self::CanonicalState
            | Self::L4Book
            | Self::PositionState
            | Self::CanonicalEvent => true,
            Self::ReferenceSnapshot => false,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct UnsupportedNetwork {
    pub network: String,
    pub reason: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Capability {
    pub id: String,
    pub source: String,
    pub network: Vec<String>,
    pub transport: String,
    pub identifier: String,
    pub domain: String,
    pub source_role: SourceRole,
    pub request_cost: String,
    pub pagination: String,
    pub parser: String,
    #[serde(default)]
    pub fixture_set: String,
    pub retention: String,
    pub freshness_target_ms: u64,
    pub owner: String,
    pub state_target: StateTarget,
    pub status: Status,
    #[serde(default)]
    pub limitations: String,
    #[serde(default)]
    pub unsupported_networks: Vec<UnsupportedNetwork>,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema_version: u32,
    pub capability: Vec<Capability>,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CoverageRow {
    pub id: String,
    pub source: String,
    pub transport: String,
    pub identifier: String,
    pub domain: String,
    pub status: String,
    pub owner: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CoverageReport {
    pub schema_version: u32,
    pub rows: Vec<CoverageRow>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CoverageDiff {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub changed: Vec<String>,
}

impl CoverageDiff {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.changed.is_empty()
    }
}

#[must_use]
pub fn rest_info_base_weight(identifier: &str) -> u32 {
    if REST_INFO_WEIGHT_2.contains(&identifier) {
        2
    } else if identifier == "userRole" {
        60
    } else {
        20
    }
}

#[must_use]
pub fn parse_request_cost_base_weight(request_cost: &str) -> Option<u32> {
    let rest = request_cost.strip_prefix("base:")?;
    let number = rest.split_whitespace().next()?;
    if number.is_empty() || !number.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    number.parse().ok()
}

pub fn parse_manifest(text: &str) -> Result<Manifest, String> {
    toml::from_str(text).map_err(|error| format!("invalid manifest: {error}"))
}

pub fn load_manifest(path: &Path) -> Result<Manifest, String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("cannot read {}: {}", path.display(), error))?;
    parse_manifest(&text)
}

pub fn validate_manifest(manifest: &Manifest) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();
    if manifest.schema_version != SCHEMA_VERSION {
        errors.push(format!(
            "schema_version must be {SCHEMA_VERSION}, got {}",
            manifest.schema_version
        ));
    }
    if manifest.capability.is_empty() {
        errors.push("manifest has no capability records".to_owned());
    }

    let mut seen = BTreeSet::new();
    for capability in &manifest.capability {
        validate_capability(capability, &mut seen, &mut errors);
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

pub fn find_workspace_root(start: &Path) -> Result<PathBuf, String> {
    let mut current = if start.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        start.to_path_buf()
    };
    if let Ok(canonical) = current.canonicalize() {
        current = canonical;
    }
    loop {
        if current.join("Cargo.toml").is_file() && current.join(MANIFEST_RELATIVE).is_file() {
            return Ok(current);
        }
        if !current.pop() {
            return Err(format!(
                "cannot find workspace root from {}",
                start.display()
            ));
        }
    }
}

pub fn coverage_report(manifest: &Manifest) -> CoverageReport {
    let mut rows: Vec<_> = manifest
        .capability
        .iter()
        .map(|capability| CoverageRow {
            id: capability.id.clone(),
            source: capability.source.clone(),
            transport: capability.transport.clone(),
            identifier: capability.identifier.clone(),
            domain: capability.domain.clone(),
            status: capability.status.as_str().to_owned(),
            owner: capability.owner.clone(),
        })
        .collect();
    rows.sort_by(|left, right| left.id.cmp(&right.id));
    CoverageReport {
        schema_version: SCHEMA_VERSION,
        rows,
    }
}

pub fn render_coverage_matrix(manifest: &Manifest) -> String {
    let mut rows: Vec<_> = manifest.capability.iter().collect();
    rows.sort_by(|left, right| left.id.cmp(&right.id));

    let mut out = String::from(
        "# Hyperliquid coverage matrix\n\
         \n\
         Generated by `hyperliquid-capabilities render-docs` from `config/hyperliquid/capabilities.toml`. \
         Edit the manifest, not this file.\n\
         \n\
         | id | source | transport | identifier | domain | status | owner | limitations |\n\
         | --- | --- | --- | --- | --- | --- | --- | --- |\n",
    );
    for capability in rows {
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} | {} |\n",
            escape_cell(&capability.id),
            escape_cell(&capability.source),
            escape_cell(&capability.transport),
            escape_cell(&capability.identifier),
            escape_cell(&capability.domain),
            escape_cell(capability.status.as_str()),
            escape_cell(&capability.owner),
            escape_cell(&capability.limitations),
        ));
    }
    out
}

pub fn diff_reports(left: &CoverageReport, right: &CoverageReport) -> CoverageDiff {
    let left_map = row_map(left);
    let right_map = row_map(right);
    let mut added = Vec::new();
    let mut removed = Vec::new();
    let mut changed = Vec::new();

    for id in left_map.keys() {
        if !right_map.contains_key(id) {
            removed.push((*id).clone());
        }
    }
    for (id, right_row) in &right_map {
        match left_map.get(id) {
            None => added.push((*id).clone()),
            Some(left_row) if left_row != right_row => changed.push((*id).clone()),
            Some(_) => {}
        }
    }
    added.sort();
    removed.sort();
    changed.sort();
    CoverageDiff {
        added,
        removed,
        changed,
    }
}

pub fn parse_coverage_report(text: &str) -> Result<CoverageReport, String> {
    serde_json::from_str(text).map_err(|error| format!("invalid coverage report: {error}"))
}

pub fn encode_coverage_report(report: &CoverageReport) -> Result<String, String> {
    let mut encoded = serde_json::to_string_pretty(report)
        .map_err(|error| format!("cannot encode coverage report: {error}"))?;
    encoded.push('\n');
    Ok(encoded)
}

pub fn format_diff(diff: &CoverageDiff) -> String {
    let mut lines = Vec::new();
    for id in &diff.added {
        lines.push(format!("added: {id}"));
    }
    for id in &diff.removed {
        lines.push(format!("removed: {id}"));
    }
    for id in &diff.changed {
        lines.push(format!("changed: {id}"));
    }
    lines.join("\n")
}

fn validate_capability(
    capability: &Capability,
    seen: &mut BTreeSet<String>,
    errors: &mut Vec<String>,
) {
    if capability.id.trim().is_empty() {
        errors.push("capability id is empty".to_owned());
        return;
    }
    if !seen.insert(capability.id.clone()) {
        errors.push(format!("duplicate capability id: {}", capability.id));
    }
    if capability.parser.trim().is_empty() {
        errors.push(format!("missing parser: {}", capability.id));
    }
    if capability.owner.trim().is_empty() {
        errors.push(format!("missing owner: {}", capability.id));
    }
    if capability.status.requires_fixture_set()
        && (capability.fixture_set.trim().is_empty() || capability.fixture_set == "none")
    {
        errors.push(format!("missing fixture set: {}", capability.id));
    }
    if capability.identifier.trim().is_empty() {
        errors.push(format!("missing identifier: {}", capability.id));
    }
    if capability.freshness_target_ms == 0 {
        errors.push(format!(
            "freshness_target_ms must be greater than 0: {}",
            capability.id
        ));
    }
    if capability.transport == "rest_info" {
        let expected = rest_info_base_weight(&capability.identifier);
        if parse_request_cost_base_weight(&capability.request_cost) != Some(expected) {
            errors.push(format!(
                "request_cost must use base:{expected}: {}",
                capability.id
            ));
        }
    }
    validate_networks(capability, errors);
    if capability.parser == "opaque_continue" && capability.state_target.is_state_affecting() {
        errors.push(format!(
            "state-affecting capability cannot be opaque_continue: {}",
            capability.id
        ));
    }
}

fn validate_networks(capability: &Capability, errors: &mut Vec<String>) {
    if capability.network.is_empty() {
        errors.push(format!("missing network: {}", capability.id));
    }
    for skipped in &capability.unsupported_networks {
        if skipped.network.trim().is_empty() || skipped.reason.trim().is_empty() {
            errors.push(format!(
                "unsupported network requires a reason: {}",
                capability.id
            ));
        }
    }
    for network in KNOWN_NETWORKS {
        if capability.network.iter().any(|value| value == network) {
            continue;
        }
        let explained = capability
            .unsupported_networks
            .iter()
            .any(|skipped| skipped.network == network && !skipped.reason.trim().is_empty());
        if !explained {
            errors.push(format!(
                "unsupported network requires a reason: {} ({network})",
                capability.id
            ));
        }
    }
}

fn row_map(report: &CoverageReport) -> BTreeMap<&String, &CoverageRow> {
    report.rows.iter().map(|row| (&row.id, row)).collect()
}

fn escape_cell(value: &str) -> String {
    value.replace('|', "\\|").replace('\n', " ")
}
