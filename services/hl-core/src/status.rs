use std::net::SocketAddr;
use std::sync::{Arc, Mutex, PoisonError};

use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;

use crate::JetStreamReplayReport;

const STATUS_SCHEMA: &str = "hl.core.status.v1";
const HEALTH_SCHEMA: &str = "hl.core.health.v1";
const MAX_REQUEST_BYTES: usize = 8_192;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CoreStatus {
    schema_version: String,
    ready: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_applied_watermark: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fail_closed_reason: Option<String>,
    live_qualified: bool,
    stage_2_qualified: bool,
}

impl CoreStatus {
    fn starting(last_applied_watermark: Option<u64>) -> Self {
        Self {
            schema_version: STATUS_SCHEMA.to_owned(),
            ready: false,
            last_applied_watermark,
            fail_closed_reason: None,
            live_qualified: false,
            stage_2_qualified: false,
        }
    }

    #[must_use]
    pub fn ready(&self) -> bool {
        self.ready
    }

    #[must_use]
    pub const fn last_applied_watermark(&self) -> Option<u64> {
        self.last_applied_watermark
    }

    #[must_use]
    pub fn fail_closed_reason(&self) -> Option<&str> {
        self.fail_closed_reason.as_deref()
    }

    #[must_use]
    pub const fn live_qualified(&self) -> bool {
        self.live_qualified
    }

    #[must_use]
    pub const fn stage_2_qualified(&self) -> bool {
        self.stage_2_qualified
    }

    fn validate(&self) -> Result<(), StatusError> {
        if self.schema_version != STATUS_SCHEMA
            || self.live_qualified
            || self.stage_2_qualified
            || (self.ready && self.fail_closed_reason.is_some())
        {
            return Err(StatusError::InvalidField);
        }
        if let Some(reason) = &self.fail_closed_reason {
            validate_reason_code(reason)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct CoreStatusHandle {
    inner: Arc<Mutex<CoreStatus>>,
    listen_addr: Arc<Mutex<Option<SocketAddr>>>,
}

impl CoreStatusHandle {
    #[must_use]
    pub fn starting(last_applied_watermark: Option<u64>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(CoreStatus::starting(last_applied_watermark))),
            listen_addr: Arc::new(Mutex::new(None)),
        }
    }

    #[must_use]
    pub fn snapshot(&self) -> CoreStatus {
        self.lock().clone()
    }

    #[must_use]
    pub fn listen_addr(&self) -> Option<SocketAddr> {
        *self.listen_lock()
    }

    pub fn mark_ready(&self) {
        let mut status = self.lock();
        if status.fail_closed_reason.is_none() {
            status.ready = true;
        }
    }

    pub(crate) fn record(&self, report: &JetStreamReplayReport) {
        let mut status = self.lock();
        status.last_applied_watermark = report.last_height.map(domain_types::BlockHeight::get);
        status.live_qualified = false;
        status.stage_2_qualified = false;
        if report.live_qualified || report.stage_2_qualified {
            status.ready = false;
            status.fail_closed_reason = Some("core_status.qualification_claim".to_owned());
        }
    }

    pub fn fail_closed(&self, reason_code: &str) {
        let mut status = self.lock();
        status.ready = false;
        status.live_qualified = false;
        status.stage_2_qualified = false;
        status.fail_closed_reason = Some(if validate_reason_code(reason_code).is_ok() {
            reason_code.to_owned()
        } else {
            "core_status.invalid_reason".to_owned()
        });
    }

    pub(crate) fn set_last_applied_watermark(&self, last_applied_watermark: Option<u64>) {
        self.lock().last_applied_watermark = last_applied_watermark;
    }

    pub(crate) fn set_listen_addr(&self, listen: SocketAddr) {
        *self.listen_lock() = Some(listen);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, CoreStatus> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn listen_lock(&self) -> std::sync::MutexGuard<'_, Option<SocketAddr>> {
        self.listen_addr
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

pub async fn serve_status(
    status: CoreStatusHandle,
    listen: SocketAddr,
    cancellation: CancellationToken,
) -> Result<(), StatusError> {
    let listener = bind_loopback(listen).await?;
    accept_status(listener, status, cancellation).await
}

pub(crate) async fn bind_loopback(listen: SocketAddr) -> Result<TcpListener, StatusError> {
    if !listen.ip().is_loopback() {
        return Err(StatusError::UnsafeBind);
    }
    let listener = TcpListener::bind(listen)
        .await
        .map_err(|_| StatusError::Bind)?;
    let bound = listener.local_addr().map_err(|_| StatusError::Bind)?;
    if !bound.ip().is_loopback() {
        return Err(StatusError::UnsafeBind);
    }
    Ok(listener)
}

pub async fn accept_status(
    listener: TcpListener,
    status: CoreStatusHandle,
    cancellation: CancellationToken,
) -> Result<(), StatusError> {
    let bound = listener.local_addr().map_err(|_| StatusError::Bind)?;
    if !bound.ip().is_loopback() {
        return Err(StatusError::UnsafeBind);
    }
    status.set_listen_addr(bound);
    loop {
        tokio::select! {
            () = cancellation.cancelled() => return Ok(()),
            accepted = listener.accept() => {
                let (stream, _) = accepted.map_err(|_| StatusError::Accept)?;
                let snapshot = status.clone();
                let request_cancellation = cancellation.child_token();
                tokio::spawn(async move {
                    if let Err(error) =
                        handle_connection(stream, snapshot, request_cancellation).await
                    {
                        tracing::debug!(
                            reason_code = error.reason_code(),
                            "core status connection closed"
                        );
                    }
                });
            }
        }
    }
}

async fn handle_connection(
    mut stream: TcpStream,
    status: CoreStatusHandle,
    cancellation: CancellationToken,
) -> Result<(), StatusError> {
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
            return Err(StatusError::RequestTooLarge);
        }
        let read = tokio::select! {
            () = cancellation.cancelled() => return Ok(()),
            read = stream.read(&mut buffer[filled..]) => {
                read.map_err(|_| StatusError::Io)?
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
        std::str::from_utf8(&buffer[..header_end]).map_err(|_| StatusError::InvalidRequest)?;
    let request_line = header.lines().next().ok_or(StatusError::InvalidRequest)?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().ok_or(StatusError::InvalidRequest)?;
    let target = parts.next().ok_or(StatusError::InvalidRequest)?;
    let path = target.split('?').next().unwrap_or(target);
    match (method, path) {
        ("GET", "/healthz") => write_health(&mut stream, &status).await,
        ("GET", "/status") => write_status(&mut stream, &status).await,
        ("GET", "/metrics") => write_metrics(&mut stream, &status).await,
        ("GET", _) => {
            write_response(&mut stream, 404, "text/plain; charset=utf-8", b"not found").await
        }
        _ => {
            write_response(
                &mut stream,
                405,
                "text/plain; charset=utf-8",
                b"method not allowed",
            )
            .await
        }
    }
}

async fn write_health(
    stream: &mut TcpStream,
    status: &CoreStatusHandle,
) -> Result<(), StatusError> {
    let snapshot = status.snapshot();
    snapshot.validate()?;
    let reason = snapshot
        .fail_closed_reason
        .clone()
        .unwrap_or_else(|| "core_status.not_ready".to_owned());
    if snapshot.ready {
        let body = format!(
            "{{\"schema_version\":\"{HEALTH_SCHEMA}\",\"ok\":true,\"ready\":true,\"reason_code\":null,\"live_qualified\":false,\"stage_2_qualified\":false}}"
        );
        write_response(stream, 200, "application/json", body.as_bytes()).await
    } else {
        let body = format!(
            "{{\"schema_version\":\"{HEALTH_SCHEMA}\",\"ok\":false,\"ready\":false,\"reason_code\":\"{reason}\",\"live_qualified\":false,\"stage_2_qualified\":false}}"
        );
        write_response(stream, 503, "application/json", body.as_bytes()).await
    }
}

async fn write_status(
    stream: &mut TcpStream,
    status: &CoreStatusHandle,
) -> Result<(), StatusError> {
    let snapshot = status.snapshot();
    snapshot.validate()?;
    let body = serde_json::to_vec(&snapshot).map_err(|_| StatusError::Serialization)?;
    write_response(stream, 200, "application/json", &body).await
}

async fn write_metrics(
    stream: &mut TcpStream,
    status: &CoreStatusHandle,
) -> Result<(), StatusError> {
    let snapshot = status.snapshot();
    snapshot.validate()?;
    let mut body = format!(
        "# TYPE hl_core_ready gauge\nhl_core_ready {}\n# TYPE hl_core_live_qualified gauge\nhl_core_live_qualified 0\n# TYPE hl_core_stage_2_qualified gauge\nhl_core_stage_2_qualified 0\n",
        u8::from(snapshot.ready),
    );
    if let Some(watermark) = snapshot.last_applied_watermark {
        body.push_str("# TYPE hl_core_last_applied_watermark gauge\n");
        body.push_str(&format!("hl_core_last_applied_watermark {watermark}\n"));
    }
    write_response(
        stream,
        200,
        "text/plain; version=0.0.4; charset=utf-8",
        body.as_bytes(),
    )
    .await
}

fn find_header_end(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|window| window == b"\r\n\r\n")
}

async fn write_response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
) -> Result<(), StatusError> {
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        405 => "Method Not Allowed",
        431 => "Request Header Fields Too Large",
        503 => "Service Unavailable",
        _ => "Error",
    };
    let header = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream
        .write_all(header.as_bytes())
        .await
        .map_err(|_| StatusError::Io)?;
    stream.write_all(body).await.map_err(|_| StatusError::Io)?;
    stream.flush().await.map_err(|_| StatusError::Io)
}

fn validate_reason_code(value: &str) -> Result<(), StatusError> {
    if value.is_empty()
        || value.trim() != value
        || value.len() > 512
        || !value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_')
        })
    {
        Err(StatusError::InvalidField)
    } else {
        Ok(())
    }
}

#[derive(Debug, thiserror::Error, Clone, Copy, PartialEq, Eq)]
pub enum StatusError {
    #[error("hl-core status bind address is not loopback")]
    UnsafeBind,
    #[error("hl-core status listener bind failed")]
    Bind,
    #[error("hl-core status accept failed")]
    Accept,
    #[error("hl-core status connection I/O failed")]
    Io,
    #[error("hl-core status HTTP request is invalid")]
    InvalidRequest,
    #[error("hl-core status HTTP request exceeded its size limit")]
    RequestTooLarge,
    #[error("hl-core status serialization failed")]
    Serialization,
    #[error("hl-core status snapshot is invalid")]
    InvalidField,
}

impl StatusError {
    #[must_use]
    pub const fn reason_code(self) -> &'static str {
        match self {
            Self::UnsafeBind => "core_status.unsafe_bind",
            Self::Bind => "core_status.bind",
            Self::Accept => "core_status.accept",
            Self::Io => "core_status.io",
            Self::InvalidRequest => "core_status.invalid_request",
            Self::RequestTooLarge => "core_status.request_too_large",
            Self::Serialization => "core_status.serialization",
            Self::InvalidField => "core_status.invalid_field",
        }
    }
}
