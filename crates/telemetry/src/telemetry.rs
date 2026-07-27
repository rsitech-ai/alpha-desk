use std::{
    fmt,
    sync::atomic::{AtomicU8, Ordering},
};

use opentelemetry::{
    KeyValue,
    trace::{TraceContextExt, TracerProvider as _},
};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{Resource, trace::SdkTracerProvider};
use prometheus::Registry;
use serde_json::{Map, Value, json};
use thiserror::Error;
use tracing::{
    Event, Subscriber,
    field::{Field, Visit},
};
use tracing_opentelemetry::OpenTelemetrySpanExt;
use tracing_subscriber::{
    fmt::{
        FmtContext,
        format::{FormatEvent, FormatFields, Writer},
    },
    layer::SubscriberExt,
    registry::LookupSpan,
    util::SubscriberInitExt,
};

use crate::{BuildProvenance, FoundationMetrics, encode_registry};

const UNINITIALIZED: u8 = 0;
const INITIALIZING: u8 = 1;
const INITIALIZED: u8 = 2;
static INITIALIZATION_STATE: AtomicU8 = AtomicU8::new(UNINITIALIZED);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelemetryConfig {
    service_name: String,
    service_version: String,
    otlp_endpoint: Option<String>,
}

impl TelemetryConfig {
    pub fn try_new(
        service_name: impl Into<String>,
        service_version: impl Into<String>,
        otlp_endpoint: Option<&str>,
    ) -> Result<Self, TelemetryError> {
        let service_name = service_name.into();
        let service_version = service_version.into();
        validate_config_text(&service_name, "service_name")?;
        validate_config_text(&service_version, "service_version")?;
        let otlp_endpoint = match otlp_endpoint {
            Some(endpoint) => {
                validate_otlp_endpoint(endpoint)?;
                Some(endpoint.to_owned())
            }
            None => None,
        };
        Ok(Self {
            service_name,
            service_version,
            otlp_endpoint,
        })
    }

    #[must_use]
    pub fn service_name(&self) -> &str {
        &self.service_name
    }

    #[must_use]
    pub fn service_version(&self) -> &str {
        &self.service_version
    }

    #[must_use]
    pub fn otlp_endpoint(&self) -> Option<&str> {
        self.otlp_endpoint.as_deref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TelemetryError {
    #[error("telemetry is already initialized or initializing")]
    AlreadyInitialized,
    #[error("invalid telemetry configuration field: {0}")]
    InvalidConfig(&'static str),
    #[error("OTLP endpoint must be an absolute HTTP(S) URL without credentials")]
    InvalidOtlpEndpoint,
    #[error("configured OTLP export requires an active Tokio runtime")]
    RuntimeUnavailable,
    #[error("configured OTLP export requires a multi-thread Tokio runtime")]
    UnsupportedRuntime,
    #[error("OTLP exporter configuration failed")]
    OtlpConfiguration,
    #[error("Prometheus metric registration failed")]
    MetricRegistration,
    #[error("Prometheus metric encoding failed")]
    MetricEncoding,
    #[error("global tracing subscriber installation failed")]
    GlobalInstall,
    #[error("OpenTelemetry provider shutdown failed")]
    ProviderShutdown,
}

pub struct TelemetryGuard {
    registry: Registry,
    metrics: FoundationMetrics,
    provider: Option<SdkTracerProvider>,
}

impl fmt::Debug for TelemetryGuard {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TelemetryGuard")
            .field("registry", &"private")
            .field("metrics", &self.metrics)
            .field("provider", &self.provider.as_ref().map(|_| "active"))
            .finish()
    }
}

impl TelemetryGuard {
    #[must_use]
    pub fn metrics(&self) -> &FoundationMetrics {
        &self.metrics
    }

    pub fn gather_prometheus(&self) -> Result<String, TelemetryError> {
        encode_registry(&self.registry)
    }

    pub fn shutdown(mut self) -> Result<(), TelemetryError> {
        self.shutdown_provider()
    }

    fn shutdown_provider(&mut self) -> Result<(), TelemetryError> {
        if let Some(provider) = self.provider.take() {
            provider
                .shutdown()
                .map_err(|_| TelemetryError::ProviderShutdown)?;
        }
        Ok(())
    }
}

impl Drop for TelemetryGuard {
    fn drop(&mut self) {
        let _shutdown_result = self.shutdown_provider();
    }
}

pub fn init_telemetry(
    config: &TelemetryConfig,
    build: &BuildProvenance,
) -> Result<TelemetryGuard, TelemetryError> {
    let reservation = InitializationReservation::acquire()?;

    if config.otlp_endpoint.is_some() {
        let handle = tokio::runtime::Handle::try_current()
            .map_err(|_| TelemetryError::RuntimeUnavailable)?;
        if handle.runtime_flavor() != tokio::runtime::RuntimeFlavor::MultiThread {
            return Err(TelemetryError::UnsupportedRuntime);
        }
    }

    let registry = Registry::new();
    let metrics = FoundationMetrics::register(&registry, build, config.otlp_endpoint.is_some())?;
    let provider = build_provider(config, build)?;
    let tracer = provider.tracer(config.service_name.clone());
    let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);
    let formatter = TelemetryJsonFormatter::new(config, build);
    let fmt_layer = tracing_subscriber::fmt::layer()
        .event_format(formatter)
        .with_writer(std::io::stderr);
    let subscriber = tracing_subscriber::registry()
        .with(otel_layer)
        .with(fmt_layer);

    if subscriber.try_init().is_err() {
        let _shutdown_result = provider.shutdown();
        return Err(TelemetryError::GlobalInstall);
    }

    reservation.commit();
    Ok(TelemetryGuard {
        registry,
        metrics,
        provider: Some(provider),
    })
}

fn build_provider(
    config: &TelemetryConfig,
    build: &BuildProvenance,
) -> Result<SdkTracerProvider, TelemetryError> {
    let resource = Resource::builder_empty()
        .with_service_name(config.service_name.clone())
        .with_attributes([
            KeyValue::new("service.version", config.service_version.clone()),
            KeyValue::new("build.git_sha", build.git_sha.clone()),
            KeyValue::new("build.dirty", build.dirty),
            KeyValue::new("build.schema_fingerprint", build.schema_fingerprint.clone()),
            KeyValue::new("build.cargo_lock_sha256", build.cargo_lock_sha256.clone()),
        ])
        .build();
    let builder = SdkTracerProvider::builder().with_resource(resource);
    match config.otlp_endpoint.as_deref() {
        Some(endpoint) => {
            let exporter = opentelemetry_otlp::SpanExporter::builder()
                .with_tonic()
                .with_endpoint(endpoint)
                .build()
                .map_err(|_| TelemetryError::OtlpConfiguration)?;
            Ok(builder.with_batch_exporter(exporter).build())
        }
        None => Ok(builder.build()),
    }
}

struct InitializationReservation {
    committed: bool,
}

impl InitializationReservation {
    fn acquire() -> Result<Self, TelemetryError> {
        INITIALIZATION_STATE
            .compare_exchange(
                UNINITIALIZED,
                INITIALIZING,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .map_err(|_| TelemetryError::AlreadyInitialized)?;
        Ok(Self { committed: false })
    }

    fn commit(mut self) {
        INITIALIZATION_STATE.store(INITIALIZED, Ordering::Release);
        self.committed = true;
    }
}

impl Drop for InitializationReservation {
    fn drop(&mut self) {
        if !self.committed {
            INITIALIZATION_STATE.store(UNINITIALIZED, Ordering::Release);
        }
    }
}

#[derive(Clone)]
struct TelemetryJsonFormatter {
    service_name: String,
    service_version: String,
    build: BuildProvenance,
}

impl TelemetryJsonFormatter {
    fn new(config: &TelemetryConfig, build: &BuildProvenance) -> Self {
        Self {
            service_name: config.service_name.clone(),
            service_version: config.service_version.clone(),
            build: build.clone(),
        }
    }
}

impl<S, N> FormatEvent<S, N> for TelemetryJsonFormatter
where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
    N: for<'writer> FormatFields<'writer> + 'static,
{
    fn format_event(
        &self,
        context: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        let mut visitor = JsonFieldVisitor::default();
        event.record(&mut visitor);
        let (trace_id, span_id) = event_trace_context(context);
        let payload = json!({
            "level": event.metadata().level().as_str(),
            "target": event.metadata().target(),
            "service": {
                "name": self.service_name,
                "version": self.service_version,
            },
            "build": {
                "git_sha": self.build.git_sha,
                "dirty": self.build.dirty,
                "schema_fingerprint": self.build.schema_fingerprint,
                "cargo_lock_sha256": self.build.cargo_lock_sha256,
            },
            "trace_id": trace_id,
            "span_id": span_id,
            "fields": visitor.fields,
        });
        let encoded = serde_json::to_string(&payload).map_err(|_| fmt::Error)?;
        writeln!(writer, "{encoded}")
    }
}

fn event_trace_context<S, N>(context: &FmtContext<'_, S, N>) -> (Option<String>, Option<String>)
where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
    N: for<'writer> FormatFields<'writer> + 'static,
{
    let event_span_id = context
        .event_scope()
        .and_then(|mut scope| scope.next().map(|span| span.id().clone()));
    let Some(event_span_id) = event_span_id else {
        return (None, None);
    };

    let otel_context = tracing::dispatcher::get_default(|dispatcher| {
        dispatcher.enter(&event_span_id);
        let otel_context = tracing::Span::current().context();
        dispatcher.exit(&event_span_id);
        otel_context
    });
    let span_context = otel_context.span().span_context().clone();
    if !span_context.is_valid() {
        return (None, None);
    }
    (
        Some(span_context.trace_id().to_string()),
        Some(span_context.span_id().to_string()),
    )
}

#[derive(Default)]
struct JsonFieldVisitor {
    fields: Map<String, Value>,
}

impl Visit for JsonFieldVisitor {
    fn record_bool(&mut self, field: &Field, value: bool) {
        self.fields
            .insert(field.name().to_owned(), Value::Bool(value));
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.fields
            .insert(field.name().to_owned(), Value::Number(value.into()));
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.fields
            .insert(field.name().to_owned(), Value::Number(value.into()));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.fields
            .insert(field.name().to_owned(), Value::String(value.to_owned()));
    }

    fn record_error(&mut self, field: &Field, value: &(dyn std::error::Error + 'static)) {
        self.fields
            .insert(field.name().to_owned(), Value::String(value.to_string()));
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.fields
            .insert(field.name().to_owned(), Value::String(format!("{value:?}")));
    }
}

fn validate_config_text(value: &str, field: &'static str) -> Result<(), TelemetryError> {
    if value.trim().is_empty() || value.trim() != value || value.chars().any(char::is_control) {
        return Err(TelemetryError::InvalidConfig(field));
    }
    Ok(())
}

fn validate_otlp_endpoint(endpoint: &str) -> Result<(), TelemetryError> {
    if endpoint.is_empty() || endpoint.chars().any(char::is_control) {
        return Err(TelemetryError::InvalidOtlpEndpoint);
    }
    let parsed = endpoint
        .parse::<http::Uri>()
        .map_err(|_| TelemetryError::InvalidOtlpEndpoint)?;
    let scheme = parsed
        .scheme_str()
        .ok_or(TelemetryError::InvalidOtlpEndpoint)?;
    let authority = parsed
        .authority()
        .ok_or(TelemetryError::InvalidOtlpEndpoint)?;
    let path_and_query = parsed
        .path_and_query()
        .ok_or(TelemetryError::InvalidOtlpEndpoint)?;
    let explicit_port_is_invalid = match authority.as_str().strip_prefix(authority.host()) {
        Some("") => false,
        Some(suffix) if suffix.starts_with(':') => {
            authority.port_u16().is_none_or(|port| port == 0)
        }
        _ => true,
    };
    if !matches!(scheme, "http" | "https")
        || authority.as_str().contains('@')
        || authority.host().is_empty()
        || explicit_port_is_invalid
        || !matches!(path_and_query.path(), "" | "/")
        || path_and_query.query().is_some()
        || endpoint.contains('#')
    {
        return Err(TelemetryError::InvalidOtlpEndpoint);
    }
    Ok(())
}
