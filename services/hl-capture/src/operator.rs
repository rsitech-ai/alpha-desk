use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;

use crate::EgressBudgetSnapshot;
use crate::status::{CaptureHealth, read_status, read_status_snapshot_bytes};

const MAX_REQUEST_BYTES: usize = 8_192;
const POLL_INTERVAL: Duration = Duration::from_millis(250);
const HEALTH_SCHEMA: &str = "hl.capture.health.v1";
const INFO_BUDGET_SCHEMA: &str = "hl.capture.info-budget.v1";

#[derive(Serialize)]
struct InfoBudgetStatusDoc<'a> {
    schema_version: &'static str,
    egress_id: &'a str,
    ceiling_weight_per_minute: u32,
    envelope_weight_per_minute: u32,
    available_priority: u32,
    available_general: u32,
    circuit_open_until_millis: Option<u64>,
    http_429_count: u64,
    requests_ok: u64,
}

pub fn encode_info_budget_status(
    snapshot: &EgressBudgetSnapshot,
) -> Result<Vec<u8>, OperatorError> {
    serde_json::to_vec(&InfoBudgetStatusDoc {
        schema_version: INFO_BUDGET_SCHEMA,
        egress_id: snapshot.egress_id(),
        ceiling_weight_per_minute: snapshot.ceiling_weight_per_minute(),
        envelope_weight_per_minute: snapshot.envelope_weight_per_minute(),
        available_priority: snapshot.available_priority(),
        available_general: snapshot.available_general(),
        circuit_open_until_millis: snapshot.circuit_open_until_millis(),
        http_429_count: snapshot.http_429_count(),
        requests_ok: snapshot.requests_ok(),
    })
    .map_err(|_| OperatorError::Serialization)
}

#[must_use]
pub fn info_budget_status_path(status_path: &Path) -> PathBuf {
    status_path.with_file_name("info-budget.json")
}

pub fn write_info_budget_snapshot(
    status_path: &Path,
    snapshot: &EgressBudgetSnapshot,
) -> Result<(), OperatorError> {
    let encoded = encode_info_budget_status(snapshot)?;
    let path = info_budget_status_path(status_path);
    let parent = path.parent().ok_or(OperatorError::Io)?;
    let temporary = parent.join("info-budget.json.tmp");
    std::fs::write(&temporary, &encoded).map_err(|_| OperatorError::Io)?;
    std::fs::rename(&temporary, &path).map_err(|_| OperatorError::Io)
}

pub async fn serve_operator_status(
    status_path: PathBuf,
    listen: SocketAddr,
    cancellation: CancellationToken,
) -> Result<(), OperatorError> {
    if !listen.ip().is_loopback() {
        return Err(OperatorError::UnsafeBind);
    }
    let listener = TcpListener::bind(listen)
        .await
        .map_err(|_| OperatorError::Bind)?;
    accept_operator_status(listener, status_path, cancellation).await
}

pub async fn accept_operator_status(
    listener: TcpListener,
    status_path: PathBuf,
    cancellation: CancellationToken,
) -> Result<(), OperatorError> {
    loop {
        tokio::select! {
            () = cancellation.cancelled() => return Ok(()),
            accepted = listener.accept() => {
                let (stream, _) = accepted.map_err(|_| OperatorError::Accept)?;
                let path = status_path.clone();
                let request_cancellation = cancellation.child_token();
                tokio::spawn(async move {
                    if let Err(error) = handle_connection(stream, &path, request_cancellation).await {
                        tracing::debug!(
                            reason_code = error.reason_code(),
                            "operator status connection closed"
                        );
                    }
                });
            }
        }
    }
}

async fn handle_connection(
    mut stream: TcpStream,
    status_path: &Path,
    cancellation: CancellationToken,
) -> Result<(), OperatorError> {
    let mut buffer = vec![0_u8; MAX_REQUEST_BYTES];
    let mut filled = 0_usize;
    let header_end = loop {
        if filled == buffer.len() {
            write_response(
                &mut stream,
                431,
                "text/plain; charset=utf-8",
                b"request too large",
            )
            .await?;
            return Err(OperatorError::RequestTooLarge);
        }
        let read = tokio::select! {
            () = cancellation.cancelled() => return Ok(()),
            read = stream.read(&mut buffer[filled..]) => {
                read.map_err(|_| OperatorError::Io)?
            }
        };
        if read == 0 {
            return Ok(());
        }
        filled = filled.saturating_add(read);
        if let Some(index) = find_header_end(&buffer[..filled]) {
            break index;
        }
    };
    let header =
        std::str::from_utf8(&buffer[..header_end]).map_err(|_| OperatorError::InvalidRequest)?;
    let request_line = header.lines().next().ok_or(OperatorError::InvalidRequest)?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().ok_or(OperatorError::InvalidRequest)?;
    let target = parts.next().ok_or(OperatorError::InvalidRequest)?;
    let path = target.split('?').next().unwrap_or(target);
    match (method, path) {
        ("OPTIONS", _) => {
            write_raw(
                &mut stream,
                b"HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type\r\nAccess-Control-Max-Age: 600\r\n\r\n",
            )
            .await
        }
        ("GET", "/healthz") => write_health(&mut stream, status_path).await,
        ("GET", "/status") => write_status(&mut stream, status_path).await,
        ("GET", "/info-budget") => write_info_budget(&mut stream, status_path).await,
        ("GET", "/events") => write_events(&mut stream, status_path, cancellation).await,
        ("GET", _) => {
            write_response(&mut stream, 404, "text/plain; charset=utf-8", b"not found").await
        }
        _ => write_response(
            &mut stream,
            405,
            "text/plain; charset=utf-8",
            b"method not allowed",
        )
        .await,
    }
}

async fn write_health(stream: &mut TcpStream, status_path: &Path) -> Result<(), OperatorError> {
    match read_status(status_path) {
        Ok(status) if status.live_ready() => {
            let health = match status.health() {
                CaptureHealth::Green => "green",
                CaptureHealth::Yellow => "yellow",
                CaptureHealth::Red => "red",
            };
            let body = format!(
                "{{\"schema_version\":\"{HEALTH_SCHEMA}\",\"ok\":true,\"health\":\"{health}\",\"ready\":true}}"
            );
            write_response(stream, 200, "application/json", body.as_bytes()).await
        }
        Ok(_) => {
            let body = format!(
                "{{\"schema_version\":\"{HEALTH_SCHEMA}\",\"ok\":false,\"reason_code\":\"{}\",\"ready\":false}}",
                OperatorError::NotReady.reason_code()
            );
            write_response(stream, 503, "application/json", body.as_bytes()).await
        }
        Err(error) => {
            let body = format!(
                "{{\"schema_version\":\"{HEALTH_SCHEMA}\",\"ok\":false,\"reason_code\":\"{}\"}}",
                error.reason_code()
            );
            write_response(stream, 503, "application/json", body.as_bytes()).await
        }
    }
}

async fn write_status(stream: &mut TcpStream, status_path: &Path) -> Result<(), OperatorError> {
    match load_status_bytes(status_path) {
        Ok(body) => write_response(stream, 200, "application/json", &body).await,
        Err(error) => {
            let body = format!(
                "{{\"schema_version\":\"hl.capture.error.v1\",\"reason_code\":\"{}\"}}",
                error.reason_code()
            );
            write_response(stream, 503, "application/json", body.as_bytes()).await
        }
    }
}

async fn write_info_budget(
    stream: &mut TcpStream,
    status_path: &Path,
) -> Result<(), OperatorError> {
    let path = info_budget_status_path(status_path);
    match std::fs::read(&path) {
        Ok(body) if !body.is_empty() => {
            write_response(stream, 200, "application/json", &body).await
        }
        Ok(_) | Err(_) => {
            write_response(stream, 404, "text/plain; charset=utf-8", b"not found").await
        }
    }
}

async fn write_events(
    stream: &mut TcpStream,
    status_path: &Path,
    cancellation: CancellationToken,
) -> Result<(), OperatorError> {
    write_raw(
        stream,
        b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\nAccess-Control-Allow-Origin: *\r\n\r\n",
    )
    .await?;
    let mut previous = Vec::new();
    loop {
        match load_status_bytes(status_path) {
            Ok(body) if body != previous => {
                write_raw(stream, b"event: status\ndata: ").await?;
                write_raw(stream, &body).await?;
                write_raw(stream, b"\n\n").await?;
                previous = body;
            }
            Ok(_) => {}
            Err(error) => {
                let payload = format!(
                    "event: disconnected\ndata: {{\"schema_version\":\"hl.capture.error.v1\",\"reason_code\":\"{}\"}}\n\n",
                    error.reason_code()
                );
                if payload.as_bytes() != previous {
                    write_raw(stream, payload.as_bytes()).await?;
                    previous = payload.into_bytes();
                }
            }
        }
        tokio::select! {
            () = cancellation.cancelled() => return Ok(()),
            () = tokio::time::sleep(POLL_INTERVAL) => {}
        }
    }
}

fn load_status_bytes(path: &Path) -> Result<Vec<u8>, OperatorError> {
    read_status_snapshot_bytes(path).map_err(OperatorError::Status)
}

fn find_header_end(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|window| window == b"\r\n\r\n")
}

async fn write_response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
) -> Result<(), OperatorError> {
    let reason = match status {
        200 => "OK",
        204 => "No Content",
        404 => "Not Found",
        405 => "Method Not Allowed",
        431 => "Request Header Fields Too Large",
        503 => "Service Unavailable",
        _ => "Error",
    };
    let header = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n",
        body.len()
    );
    write_raw(stream, header.as_bytes()).await?;
    write_raw(stream, body).await
}

async fn write_raw(stream: &mut TcpStream, bytes: &[u8]) -> Result<(), OperatorError> {
    stream
        .write_all(bytes)
        .await
        .map_err(|_| OperatorError::Io)?;
    stream.flush().await.map_err(|_| OperatorError::Io)
}

#[derive(Debug, thiserror::Error, Clone, Copy, PartialEq, Eq)]
pub enum OperatorError {
    #[error("operator status bind address is not loopback")]
    UnsafeBind,
    #[error("operator status listener bind failed")]
    Bind,
    #[error("operator status accept failed")]
    Accept,
    #[error("operator status connection I/O failed")]
    Io,
    #[error("operator status HTTP request is invalid")]
    InvalidRequest,
    #[error("operator status HTTP request exceeded its size limit")]
    RequestTooLarge,
    #[error("operator status serialization failed")]
    Serialization,
    #[error("operator healthz snapshot is not live-ready")]
    NotReady,
    #[error("operator status snapshot is unavailable")]
    Status(crate::StatusError),
}

impl OperatorError {
    #[must_use]
    pub const fn reason_code(self) -> &'static str {
        match self {
            Self::UnsafeBind => "capture_operator.unsafe_bind",
            Self::Bind => "capture_operator.bind",
            Self::Accept => "capture_operator.accept",
            Self::Io => "capture_operator.io",
            Self::InvalidRequest => "capture_operator.invalid_request",
            Self::RequestTooLarge => "capture_operator.request_too_large",
            Self::Serialization => "capture_operator.serialization",
            Self::NotReady => "capture_health.not_ready",
            Self::Status(error) => error.reason_code(),
        }
    }
}
