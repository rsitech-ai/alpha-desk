use std::collections::HashMap;

use prometheus::{Encoder, IntCounterVec, IntGauge, Opts, Registry, TextEncoder, core::Collector};

use crate::{BuildProvenance, HealthState, TelemetryError};

#[derive(Debug, Clone)]
pub struct FoundationMetrics {
    health_assessments: IntCounterVec,
}

impl FoundationMetrics {
    pub fn register(
        registry: &Registry,
        build: &BuildProvenance,
        otlp_enabled: bool,
    ) -> Result<Self, TelemetryError> {
        let initialized = IntGauge::new(
            "alpha_desk_telemetry_initialized",
            "Whether the foundation telemetry pipeline initialized successfully.",
        )
        .map_err(|_| TelemetryError::MetricRegistration)?;
        let otlp = IntGauge::new(
            "alpha_desk_otlp_export_enabled",
            "Whether OTLP trace export is explicitly configured.",
        )
        .map_err(|_| TelemetryError::MetricRegistration)?;
        otlp.set(i64::from(otlp_enabled));

        let health_assessments = IntCounterVec::new(
            Opts::new(
                "alpha_desk_health_assessments_total",
                "Health assessments observed by severity.",
            ),
            &["state"],
        )
        .map_err(|_| TelemetryError::MetricRegistration)?;
        let mut build_labels = HashMap::new();
        build_labels.insert("git_sha".to_owned(), build.git_sha.clone());
        build_labels.insert("rustc_version".to_owned(), build.rustc_version.clone());
        build_labels.insert("reproducible".to_owned(), build.reproducible.to_string());
        let build_info = IntGauge::with_opts(
            Opts::new(
                "alpha_desk_build_info",
                "Immutable build identity for this process.",
            )
            .const_labels(build_labels),
        )
        .map_err(|_| TelemetryError::MetricRegistration)?;
        build_info.set(1);

        let registrations: Vec<Box<dyn Collector>> = vec![
            Box::new(otlp.clone()),
            Box::new(health_assessments.clone()),
            Box::new(build_info.clone()),
            Box::new(initialized.clone()),
        ];
        let rollback_collectors: Vec<Box<dyn Collector>> = vec![
            Box::new(otlp),
            Box::new(health_assessments.clone()),
            Box::new(build_info),
            Box::new(initialized.clone()),
        ];
        for (registered_count, collector) in registrations.into_iter().enumerate() {
            if registry.register(collector).is_err() {
                for owned in rollback_collectors.into_iter().take(registered_count).rev() {
                    registry
                        .unregister(owned)
                        .map_err(|_| TelemetryError::MetricRegistration)?;
                }
                return Err(TelemetryError::MetricRegistration);
            }
        }
        initialized.set(1);

        Ok(Self { health_assessments })
    }

    pub fn observe_health(&self, state: HealthState) {
        let state = match state {
            HealthState::Green => "green",
            HealthState::Amber => "amber",
            HealthState::Red => "red",
        };
        self.health_assessments.with_label_values(&[state]).inc();
    }
}

pub fn encode_registry(registry: &Registry) -> Result<String, TelemetryError> {
    let families = registry.gather();
    let mut bytes = Vec::new();
    TextEncoder::new()
        .encode(&families, &mut bytes)
        .map_err(|_| TelemetryError::MetricEncoding)?;
    String::from_utf8(bytes).map_err(|_| TelemetryError::MetricEncoding)
}
