use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum HealthState {
    Green,
    Amber,
    Red,
}

impl HealthState {
    #[must_use]
    pub const fn as_wire_name(self) -> &'static str {
        match self {
            Self::Green => "GREEN",
            Self::Amber => "AMBER",
            Self::Red => "RED",
        }
    }

    #[must_use]
    pub const fn suppresses_publication(self) -> bool {
        matches!(self, Self::Red)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeatureHealth {
    state: HealthState,
    reason_code: &'static str,
}

impl FeatureHealth {
    #[must_use]
    pub const fn green() -> Self {
        Self {
            state: HealthState::Green,
            reason_code: "core.health.green",
        }
    }

    #[must_use]
    pub const fn state(&self) -> HealthState {
        self.state
    }

    #[must_use]
    pub const fn reason_code(&self) -> &'static str {
        self.reason_code
    }

    pub fn observe_material_divergence(&mut self, unresolved: bool) {
        if unresolved {
            self.state = HealthState::Red;
            self.reason_code = "core.health.material_divergence";
        } else if self.reason_code == "core.health.material_divergence" {
            *self = Self::green();
        }
    }

    pub fn observe_disk_pressure(&mut self, under_reserve: bool) {
        if under_reserve {
            self.state = HealthState::Red;
            self.reason_code = "core.health.disk_pressure";
        } else if self.reason_code == "core.health.disk_pressure" {
            *self = Self::green();
        }
    }

    pub fn observe_backlog(&mut self, saturated: bool) {
        if saturated {
            self.state = HealthState::Red;
            self.reason_code = "core.health.backlog";
        } else if self.reason_code == "core.health.backlog" {
            *self = Self::green();
        }
    }
}

pub trait DiskSpaceProbe {
    fn available_bytes(&self) -> Result<u64, DiskPressureError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DiskPressureError {
    #[error("disk reserve configuration is invalid")]
    InvalidConfig,
    #[error("disk space probe failed")]
    Probe,
    #[error("disk reserve is exhausted: available {available} required {required}")]
    Exhausted { available: u64, required: u64 },
}

impl DiskPressureError {
    #[must_use]
    pub const fn reason_code(self) -> &'static str {
        match self {
            Self::InvalidConfig => "core.disk.invalid_config",
            Self::Probe => "core.disk.probe",
            Self::Exhausted { .. } => "core.disk.exhausted",
        }
    }
}

#[derive(Debug)]
pub struct DiskReserve<P> {
    probe: P,
    reserve_bytes: u64,
}

impl<P: DiskSpaceProbe> DiskReserve<P> {
    pub fn try_new(probe: P, reserve_bytes: u64) -> Result<Self, DiskPressureError> {
        if reserve_bytes == 0 {
            return Err(DiskPressureError::InvalidConfig);
        }
        Ok(Self {
            probe,
            reserve_bytes,
        })
    }

    pub fn ensure(&self) -> Result<u64, DiskPressureError> {
        let available = self.probe.available_bytes()?;
        if available < self.reserve_bytes {
            return Err(DiskPressureError::Exhausted {
                available,
                required: self.reserve_bytes,
            });
        }
        Ok(available)
    }
}

#[derive(Debug)]
pub struct ShutdownFlag {
    stop: AtomicBool,
}

impl ShutdownFlag {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            stop: AtomicBool::new(false),
        }
    }

    pub fn request_stop(&self) {
        self.stop.store(true, Ordering::Release);
    }

    #[must_use]
    pub fn is_stopped(&self) -> bool {
        self.stop.load(Ordering::Acquire)
    }
}

impl Default for ShutdownFlag {
    fn default() -> Self {
        Self::new()
    }
}
