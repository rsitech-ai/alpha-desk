use std::collections::{BTreeMap, VecDeque};

use bytes::Bytes;
use domain_types::KnownTime;
use hl_protocol::info::{EncodedInfoRequest, InfoParseContext, InfoRegistry, ParsedInfoResponse};
use hl_protocol::{ObservationClass, SourceAdmission, SourceTrust};
use serde_json::Value;

use crate::ConfigError;
use crate::config::{EgressConfig, EgressKind, OFFICIAL_INFO_URLS};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InfoHttpResponse {
    status: u16,
    body: Bytes,
}

impl InfoHttpResponse {
    #[must_use]
    pub fn new(status: u16, body: impl Into<Bytes>) -> Self {
        Self {
            status,
            body: body.into(),
        }
    }

    #[must_use]
    pub const fn status(&self) -> u16 {
        self.status
    }

    #[must_use]
    pub fn body(&self) -> &Bytes {
        &self.body
    }
}

pub trait InfoTransport {
    fn post_info(
        &mut self,
        url: &str,
        request: &EncodedInfoRequest,
    ) -> Result<InfoHttpResponse, EgressError>;
}

#[derive(Debug, Default)]
pub struct ScriptedInfoTransport {
    responses: VecDeque<InfoHttpResponse>,
    posted: Vec<Bytes>,
}

impl ScriptedInfoTransport {
    #[must_use]
    pub fn new(responses: impl IntoIterator<Item = InfoHttpResponse>) -> Self {
        Self {
            responses: responses.into_iter().collect(),
            posted: Vec::new(),
        }
    }

    #[must_use]
    pub fn posted(&self) -> &[Bytes] {
        &self.posted
    }
}

impl InfoTransport for ScriptedInfoTransport {
    fn post_info(
        &mut self,
        url: &str,
        request: &EncodedInfoRequest,
    ) -> Result<InfoHttpResponse, EgressError> {
        let _ = guard_info_request_url(url)?;
        self.posted.push(request.body().clone());
        self.responses
            .pop_front()
            .ok_or(EgressError::NoScriptedResponse)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EgressPolicy {
    id: String,
    kind: EgressKind,
    base_url: String,
}

impl EgressPolicy {
    pub fn from_config(config: &EgressConfig) -> Result<Self, ConfigError> {
        Ok(Self {
            id: config.id().to_owned(),
            kind: config.kind(),
            base_url: config.base_url().to_owned(),
        })
    }

    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[must_use]
    pub const fn kind(&self) -> EgressKind {
        self.kind
    }

    #[must_use]
    pub fn base_url(&self) -> &str {
        &self.base_url
    }
}

#[derive(Debug, Clone)]
pub struct InfoFetch {
    parsed: ParsedInfoResponse<Value>,
    admission: SourceAdmission,
}

impl InfoFetch {
    #[must_use]
    pub const fn parsed(&self) -> &ParsedInfoResponse<Value> {
        &self.parsed
    }

    #[must_use]
    pub const fn admission(&self) -> SourceAdmission {
        self.admission
    }
}

pub fn official_info_post_url(base_url: &str) -> Result<String, EgressError> {
    let url = guard_info_request_url(base_url)?;
    if url.ends_with("/info") {
        Ok(url)
    } else {
        Ok(format!("{url}/info"))
    }
}

pub fn guard_info_request_url(url: &str) -> Result<String, EgressError> {
    if url.is_empty() {
        return Err(EgressError::HostNotAllowlisted);
    }
    if url.contains('@') {
        return Err(EgressError::HostNotAllowlisted);
    }
    if is_exchange_http_path(url) {
        return Err(EgressError::ExchangeForbidden);
    }
    if url.starts_with("http://") {
        return Err(EgressError::TlsRequired);
    }
    if !url.starts_with("https://") {
        return Err(EgressError::TlsRequired);
    }
    let without_query = url.split(['?', '#']).next().unwrap_or(url);
    let trimmed = without_query
        .strip_suffix('/')
        .filter(|stripped| *stripped != "https:/")
        .unwrap_or(without_query);
    if !OFFICIAL_INFO_URLS.contains(&trimmed) && !OFFICIAL_INFO_URLS.contains(&without_query) {
        return Err(EgressError::HostNotAllowlisted);
    }
    Ok(trimmed.to_owned())
}

pub fn fetch_info<T: InfoTransport>(
    transport: &mut T,
    registry: InfoRegistry,
    capability_id: &str,
    params: &BTreeMap<String, Value>,
    received_at: KnownTime,
    archive_ref: &str,
    request_url: &str,
) -> Result<InfoFetch, EgressError> {
    let endpoint = registry.get(capability_id).map_err(EgressError::Info)?;
    let admission = endpoint
        .admission()
        .map_err(|_| EgressError::CommittedLane)?;
    if admission.trust() != SourceTrust::ReconciledSnapshot
        || admission.observation_class() != ObservationClass::Snapshot
        || admission.can_advance_committed_watermark()
    {
        return Err(EgressError::CommittedLane);
    }
    let encoded = endpoint.encode(params).map_err(EgressError::Info)?;
    let _ = official_info_post_url(request_url)?;
    if forbids_exchange_request(encoded.identifier(), encoded.body(), request_url) {
        return Err(EgressError::ExchangeForbidden);
    }
    let response = transport.post_info(request_url, &encoded)?;
    match response.status {
        200 => {
            let context = InfoParseContext::new(
                encoded.content_hash(),
                received_at,
                hl_protocol::info::ArchiveRef::new(archive_ref).map_err(EgressError::Info)?,
            );
            let parsed = endpoint
                .parse(response.body(), &context)
                .map_err(EgressError::Info)?;
            Ok(InfoFetch { parsed, admission })
        }
        429 => Err(EgressError::RateLimited),
        status => Err(EgressError::HttpStatus(status)),
    }
}

const EXCHANGE_ACTIONS: &[&str] = &[
    "order",
    "cancel",
    "cancelByCloid",
    "modify",
    "batchModify",
    "updateLeverage",
];

#[must_use]
pub fn is_exchange_http_path(url: &str) -> bool {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    path == "/exchange"
        || path == "exchange"
        || path.ends_with("/exchange")
        || path.contains("/exchange/")
}

#[must_use]
pub fn forbids_exchange_request(identifier: &str, body: &[u8], url: &str) -> bool {
    identifier == "exchange"
        || is_exchange_http_path(identifier)
        || is_exchange_http_path(url)
        || encoded_body_is_exchange(body)
}

fn encoded_body_is_exchange(body: &[u8]) -> bool {
    let Ok(value) = serde_json::from_slice::<Value>(body) else {
        return false;
    };
    json_is_exchange_action(value.get("type")) || json_is_exchange_action(value.get("action"))
}

fn json_is_exchange_action(node: Option<&Value>) -> bool {
    match node {
        Some(Value::String(kind)) => {
            EXCHANGE_ACTIONS.contains(&kind.as_str()) || is_exchange_http_path(kind)
        }
        Some(Value::Object(map)) => map
            .get("type")
            .and_then(Value::as_str)
            .is_some_and(|kind| EXCHANGE_ACTIONS.contains(&kind) || is_exchange_http_path(kind)),
        _ => false,
    }
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum EgressError {
    #[error("info protocol error")]
    Info(#[from] hl_protocol::info::InfoError),
    #[error("official info cannot publish on the committed lane")]
    CommittedLane,
    #[error("capture refused an /exchange URL or action")]
    ExchangeForbidden,
    #[error("official info returned HTTP {0}")]
    HttpStatus(u16),
    #[error("official info rate-limited the caller")]
    RateLimited,
    #[error("scripted info transport has no remaining response")]
    NoScriptedResponse,
    #[error("info request host is not allowlisted")]
    HostNotAllowlisted,
    #[error("info request must use https")]
    TlsRequired,
    #[error("info request timed out")]
    Timeout,
    #[error("info response exceeded the payload limit")]
    BodyTooLarge,
    #[error("info response used HTTP compression")]
    CompressedBody,
}

impl EgressError {
    #[must_use]
    pub const fn reason_code(&self) -> &'static str {
        match self {
            Self::Info(error) => error.reason_code(),
            Self::CommittedLane => "capture_info.committed_lane",
            Self::ExchangeForbidden => "capture_info.exchange_forbidden",
            Self::HttpStatus(_) => "capture_info.http_status",
            Self::RateLimited => "capture_info.rate_limited",
            Self::NoScriptedResponse => "capture_info.no_scripted_response",
            Self::HostNotAllowlisted => "capture_info.host_not_allowlisted",
            Self::TlsRequired => "capture_info.tls_required",
            Self::Timeout => "capture_info.timeout",
            Self::BodyTooLarge => "capture_info.body_too_large",
            Self::CompressedBody => "capture_info.compressed_body",
        }
    }
}
