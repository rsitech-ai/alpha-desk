use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use api_contracts::{WireHealthAssessment, WireHealthState};
use bytes::Bytes;
use http::header::{AUTHORIZATION, CONNECTION, CONTENT_TYPE, UPGRADE};
use http::{HeaderName, Method, Request, Response, StatusCode};
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use serde::Serialize;
use serde_json::Value;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::auth::credentials_match;
use crate::budget::{BudgetError, QueryBudgets};
use crate::config::{ApiConfig, AuthMode};
use crate::error::ErrorBody;
use crate::openapi::openapi_yaml;
use crate::snapshot::{
    HEALTH_SCHEMA_VERSION, SnapshotError, load_canonical_health, load_capture_status,
};

const JSON_CONTENT_TYPE: &str = "application/json; charset=utf-8";
const YAML_CONTENT_TYPE: &str = "application/yaml; charset=utf-8";
const MAX_BODY_BYTES: usize = 16 * 1024;

#[derive(Clone)]
pub struct AppState {
    inner: Arc<ApiConfig>,
}

pub struct ApiHandle {
    addr: SocketAddr,
    shutdown: Option<oneshot::Sender<()>>,
    join: Option<JoinHandle<()>>,
}

impl ApiHandle {
    #[must_use]
    pub const fn addr(&self) -> SocketAddr {
        self.addr
    }
}

impl Drop for ApiHandle {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(join) = self.join.take() {
            join.abort();
        }
    }
}

pub async fn spawn_local(config: ApiConfig) -> Result<ApiHandle, std::io::Error> {
    let listener = TcpListener::bind(config.bind()).await?;
    let addr = listener.local_addr()?;
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let state = AppState::from_config(config);
    let join = tokio::spawn(async move {
        let _ = serve(listener, state, shutdown_rx).await;
    });
    Ok(ApiHandle {
        addr,
        shutdown: Some(shutdown_tx),
        join: Some(join),
    })
}

pub async fn serve(
    listener: TcpListener,
    state: AppState,
    mut shutdown: oneshot::Receiver<()>,
) -> Result<(), std::io::Error> {
    loop {
        tokio::select! {
            _ = &mut shutdown => return Ok(()),
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                let state = state.clone();
                tokio::spawn(async move {
                    let io = TokioIo::new(stream);
                    let service = service_fn(move |request| {
                        let state = state.clone();
                        async move { Ok::<_, Infallible>(state.respond(request).await) }
                    });
                    let _ = http1::Builder::new()
                        .keep_alive(false)
                        .serve_connection(io, service)
                        .await;
                });
            }
        }
    }
}

impl AppState {
    #[must_use]
    pub fn from_config(config: ApiConfig) -> Self {
        Self {
            inner: Arc::new(config),
        }
    }

    #[must_use]
    pub fn handle(&self, request: Request<Bytes>) -> (StatusCode, Vec<u8>) {
        let response = self.dispatch(&request);
        let status = response.status();
        let body = response
            .into_body()
            .into_inner()
            .unwrap_or_default()
            .to_vec();
        (status, body)
    }

    pub async fn respond(&self, request: Request<Incoming>) -> Response<Full<Bytes>> {
        let (parts, body) = request.into_parts();
        let collected = match body.collect().await {
            Ok(collected) => collected.to_bytes(),
            Err(_) => {
                return error_response(
                    StatusCode::SERVICE_UNAVAILABLE,
                    ErrorBody::new("data_unavailable", "request_body"),
                );
            }
        };
        if collected.len() > MAX_BODY_BYTES {
            return error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                ErrorBody::new("data_unavailable", "request_body"),
            );
        }
        let request = Request::from_parts(parts, collected);
        if let Some(response) = self.ungated_response(&request) {
            return response;
        }
        match self
            .inner
            .query_budgets()
            .execute(request.uri().query(), async { self.route(&request) })
            .await
        {
            Ok(response) => response,
            Err(error) => budget_response(error),
        }
    }

    fn dispatch(&self, request: &Request<Bytes>) -> Response<Full<Bytes>> {
        if let Some(response) = self.ungated_response(request) {
            return response;
        }
        let _permit = match self
            .inner
            .query_budgets()
            .check_and_acquire(request.uri().query())
        {
            Ok(permit) => permit,
            Err(error) => return budget_response(error),
        };
        self.route(request)
    }

    fn ungated_response(&self, request: &Request<Bytes>) -> Option<Response<Full<Bytes>>> {
        if wants_websocket(request) {
            return Some(error_response(
                StatusCode::NOT_IMPLEMENTED,
                ErrorBody::new("not_implemented", "stream.websocket_unspecified"),
            ));
        }
        if request.method() != Method::GET {
            return Some(error_response(
                StatusCode::METHOD_NOT_ALLOWED,
                ErrorBody::new("method_not_allowed", "http.method"),
            ));
        }
        if request.uri().path() == "/healthz" {
            return Some(self.healthz());
        }
        if let Err(error) = self.authorize(request) {
            return Some(error_response(StatusCode::UNAUTHORIZED, error));
        }
        if request.uri().path() == "/readyz" {
            return Some(self.readyz());
        }
        None
    }

    fn route(&self, request: &Request<Bytes>) -> Response<Full<Bytes>> {
        match request.uri().path() {
            "/v1/health" => self.canonical_health(),
            "/v1/capture/status" => self.capture_status(),
            "/v1/stream" | "/v1/stream/canonical-events" => error_response(
                StatusCode::NOT_IMPLEMENTED,
                ErrorBody::new("not_implemented", "stream.websocket_unspecified"),
            ),
            "/v1/openapi.yaml" => yaml_response(openapi_yaml()),
            _ => error_response(
                StatusCode::NOT_FOUND,
                ErrorBody::new("not_found", "http.path"),
            ),
        }
    }

    #[must_use]
    pub fn query_budgets(&self) -> &QueryBudgets {
        self.inner.query_budgets()
    }

    fn authorize(&self, request: &Request<Bytes>) -> Result<(), ErrorBody> {
        match self.inner.auth_mode() {
            AuthMode::LoopbackDev => Ok(()),
            AuthMode::Credential => {
                let expected = self
                    .inner
                    .credential()
                    .ok_or_else(|| ErrorBody::new("unauthorized", "auth.missing_credentials"))?;
                let header = request
                    .headers()
                    .get(AUTHORIZATION)
                    .ok_or_else(|| ErrorBody::new("unauthorized", "auth.missing_bearer"))?;
                let value = header
                    .to_str()
                    .map_err(|_| ErrorBody::new("unauthorized", "auth.invalid_bearer"))?;
                let provided = value
                    .strip_prefix("Bearer ")
                    .ok_or_else(|| ErrorBody::new("unauthorized", "auth.invalid_bearer"))?;
                if credentials_match(provided.as_bytes(), expected) {
                    Ok(())
                } else {
                    Err(ErrorBody::new("unauthorized", "auth.invalid_bearer"))
                }
            }
        }
    }

    fn healthz(&self) -> Response<Full<Bytes>> {
        health_response(
            StatusCode::OK,
            &process_health(WireHealthState::Green, "healthy"),
        )
    }

    fn readyz(&self) -> Response<Full<Bytes>> {
        if self.inner.canonical_health_path().is_none()
            && self.inner.capture_status_path().is_none()
        {
            let assessment = process_named(
                "health:aggregate",
                WireHealthState::Red,
                "no_required_dependencies",
            );
            return health_response(StatusCode::SERVICE_UNAVAILABLE, &assessment);
        }
        let mut parts = vec![process_health(WireHealthState::Green, "healthy")];
        if self.inner.canonical_health_path().is_some() {
            match load_canonical_health(self.inner.canonical_health_path()) {
                Ok(assessment) => parts.push(assessment),
                Err(error) => parts.push(missing_dependency("canonical", error)),
            }
        }
        if self.inner.capture_status_path().is_some() {
            match load_capture_status(self.inner.capture_status_path()) {
                Ok(_) => parts.push(process_named("capture", WireHealthState::Green, "healthy")),
                Err(error) => parts.push(missing_dependency("capture", error)),
            }
        }
        let aggregate = aggregate_health(&parts);
        let status = if aggregate.state == WireHealthState::Green {
            StatusCode::OK
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        };
        health_response(status, &aggregate)
    }

    fn canonical_health(&self) -> Response<Full<Bytes>> {
        match load_canonical_health(self.inner.canonical_health_path()) {
            Ok(assessment) => health_response(StatusCode::OK, &assessment),
            Err(error) => snapshot_unavailable(error),
        }
    }

    fn capture_status(&self) -> Response<Full<Bytes>> {
        match load_capture_status(self.inner.capture_status_path()) {
            Ok(value) => json_value_response(StatusCode::OK, &value),
            Err(error) => snapshot_unavailable(error),
        }
    }
}

fn process_health(state: WireHealthState, reason_code: &str) -> WireHealthAssessment {
    process_named("api:process", state, reason_code)
}

fn process_named(scope: &str, state: WireHealthState, reason_code: &str) -> WireHealthAssessment {
    WireHealthAssessment::try_new(
        scope,
        state,
        reason_code,
        observed_at_micros(),
        [] as [&str; 0],
    )
    .unwrap_or_else(|_| {
        WireHealthAssessment::try_new(
            "health:invalid",
            WireHealthState::Red,
            "invalid_scope",
            0,
            [] as [&str; 0],
        )
        .expect("literal invalid health assessment")
    })
}

fn missing_dependency(scope: &str, error: SnapshotError) -> WireHealthAssessment {
    process_named(scope, WireHealthState::Red, error.reason_code())
}

fn aggregate_health(parts: &[WireHealthAssessment]) -> WireHealthAssessment {
    let state = parts
        .iter()
        .map(|part| part.state)
        .max()
        .unwrap_or(WireHealthState::Red);
    let observed_at_micros = parts
        .iter()
        .map(|part| part.observed_at_micros)
        .max()
        .unwrap_or(0);
    let mut reasons: Vec<(&str, &str)> = parts
        .iter()
        .filter(|part| part.state != WireHealthState::Green)
        .map(|part| (part.scope.as_str(), part.reason_code.as_str()))
        .collect();
    reasons.sort_by(|left, right| left.0.cmp(right.0).then(left.1.cmp(right.1)));
    let reason_code = if reasons.is_empty() {
        "healthy".to_owned()
    } else {
        reasons
            .into_iter()
            .map(|(scope, reason)| format!("{scope}={reason}"))
            .collect::<Vec<_>>()
            .join(";")
    };
    WireHealthAssessment::try_new(
        "health:aggregate",
        state,
        reason_code,
        observed_at_micros,
        [] as [&str; 0],
    )
    .unwrap_or_else(|_| process_named("health:aggregate", WireHealthState::Red, "invalid_scope"))
}

fn observed_at_micros() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_micros()).ok())
        .unwrap_or(0)
}

fn wants_websocket(request: &Request<Bytes>) -> bool {
    header_has_token(request, UPGRADE, "websocket")
        && header_has_token(request, CONNECTION, "upgrade")
}

fn header_has_token(request: &Request<Bytes>, name: HeaderName, token: &str) -> bool {
    request
        .headers()
        .get_all(name)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .any(|part| part.trim().eq_ignore_ascii_case(token))
}

fn snapshot_unavailable(error: SnapshotError) -> Response<Full<Bytes>> {
    error_response(
        StatusCode::SERVICE_UNAVAILABLE,
        ErrorBody::new("data_unavailable", error.reason_code()),
    )
}

fn budget_response(error: BudgetError) -> Response<Full<Bytes>> {
    error_response(error.status(), error.error_body())
}

fn health_response(status: StatusCode, assessment: &WireHealthAssessment) -> Response<Full<Bytes>> {
    let body = HealthJson {
        schema_version: HEALTH_SCHEMA_VERSION,
        scope: &assessment.scope,
        state: assessment.state.proto_name(),
        reason_code: &assessment.reason_code,
        observed_at_micros: assessment.observed_at_micros,
        suppresses: &assessment.suppresses,
    };
    json_response(status, &body)
}

fn json_response<T: Serialize>(status: StatusCode, body: &T) -> Response<Full<Bytes>> {
    match serde_json::to_vec(body) {
        Ok(bytes) => bytes_response(status, JSON_CONTENT_TYPE, bytes),
        Err(_) => error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            ErrorBody::new("data_unavailable", "serialization"),
        ),
    }
}

fn json_value_response(status: StatusCode, value: &Value) -> Response<Full<Bytes>> {
    json_response(status, value)
}

fn yaml_response(body: &str) -> Response<Full<Bytes>> {
    bytes_response(StatusCode::OK, YAML_CONTENT_TYPE, body.as_bytes().to_vec())
}

fn error_response(status: StatusCode, body: ErrorBody) -> Response<Full<Bytes>> {
    json_response(status, &body)
}

fn bytes_response(status: StatusCode, content_type: &str, body: Vec<u8>) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, content_type)
        .body(Full::new(Bytes::from(body)))
        .unwrap_or_else(|_| {
            Response::builder()
                .status(StatusCode::SERVICE_UNAVAILABLE)
                .header(CONTENT_TYPE, JSON_CONTENT_TYPE)
                .body(Full::new(Bytes::from(
                    r#"{"schema_version":"hl.api.error.v1","code":"data_unavailable","reason_code":"serialization"}"#,
                )))
                .expect("literal error response")
        })
}

#[derive(Serialize)]
struct HealthJson<'a> {
    schema_version: &'static str,
    scope: &'a str,
    state: &'a str,
    reason_code: &'a str,
    observed_at_micros: i64,
    suppresses: &'a [String],
}
