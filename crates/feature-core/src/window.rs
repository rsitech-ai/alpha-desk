use std::collections::{BTreeMap, BTreeSet, VecDeque};

use domain_types::{Decimal, EventId, ProtocolTime, RoundingMode, ValueError};
use serde::{Deserialize, Serialize};

use crate::FeatureError;

pub const WINDOW_PARAMETER_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WindowAlgorithm {
    EventCount,
    ProtocolTime,
    ExponentiallyWeighted,
    QuantileSketch,
    Covariance,
    RobustZScore,
}

impl WindowAlgorithm {
    #[must_use]
    pub const fn as_wire_name(self) -> &'static str {
        match self {
            Self::EventCount => "event_count",
            Self::ProtocolTime => "protocol_time",
            Self::ExponentiallyWeighted => "exponentially_weighted",
            Self::QuantileSketch => "quantile_sketch",
            Self::Covariance => "covariance",
            Self::RobustZScore => "robust_z_score",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowUpdate {
    pub event_id: EventId,
    pub event_time: ProtocolTime,
    pub sequence: u64,
    pub value: Decimal,
    pub paired_value: Option<Decimal>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowSnapshot {
    pub algorithm: WindowAlgorithm,
    pub parameter_version: u32,
    pub count: u64,
    pub value: Option<Decimal>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RollingWindow {
    algorithm: WindowAlgorithm,
    parameter_version: u32,
    event_capacity: usize,
    duration_micros: i64,
    decay_ppm: u32,
    quantile_ppm: u32,
    seen: BTreeSet<String>,
    samples: VecDeque<Sample>,
    ewma: Option<Decimal>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Sample {
    event_id: String,
    event_time: ProtocolTime,
    sequence: u64,
    value: Decimal,
    paired_value: Option<Decimal>,
}

impl RollingWindow {
    pub fn try_new(
        algorithm: WindowAlgorithm,
        event_capacity: usize,
        duration_micros: i64,
        decay_ppm: u32,
        quantile_ppm: u32,
    ) -> Result<Self, FeatureError> {
        if event_capacity == 0 {
            return Err(FeatureError::Malformed {
                what: "window",
                reason: "capacity must be >= 1",
            });
        }
        if duration_micros < 0 {
            return Err(FeatureError::Malformed {
                what: "window",
                reason: "duration must be non-negative",
            });
        }
        if decay_ppm > 1_000_000 || quantile_ppm > 1_000_000 {
            return Err(FeatureError::Malformed {
                what: "window",
                reason: "ppm parameter out of range",
            });
        }
        match algorithm {
            WindowAlgorithm::ExponentiallyWeighted if decay_ppm == 0 || decay_ppm == 1_000_000 => {
                return Err(FeatureError::Malformed {
                    what: "window",
                    reason: "ewma decay must be exclusive of 0 and 1",
                });
            }
            WindowAlgorithm::QuantileSketch if quantile_ppm > 1_000_000 => {
                return Err(FeatureError::Malformed {
                    what: "window",
                    reason: "quantile ppm out of range",
                });
            }
            WindowAlgorithm::EventCount
            | WindowAlgorithm::ProtocolTime
            | WindowAlgorithm::Covariance
            | WindowAlgorithm::RobustZScore
            | WindowAlgorithm::ExponentiallyWeighted
            | WindowAlgorithm::QuantileSketch => {}
        }
        Ok(Self {
            algorithm,
            parameter_version: WINDOW_PARAMETER_VERSION,
            event_capacity,
            duration_micros,
            decay_ppm,
            quantile_ppm,
            seen: BTreeSet::new(),
            samples: VecDeque::new(),
            ewma: None,
        })
    }

    pub fn update(&mut self, update: WindowUpdate) -> Result<bool, FeatureError> {
        if self.seen.contains(update.event_id.as_str()) {
            return Ok(false);
        }
        if self.samples.len() >= self.event_capacity
            && !matches!(
                self.algorithm,
                WindowAlgorithm::EventCount | WindowAlgorithm::ProtocolTime
            )
        {
            return Err(FeatureError::WindowCapacityExceeded);
        }
        match self.algorithm {
            WindowAlgorithm::Covariance => {
                if update.paired_value.is_none() {
                    return Err(FeatureError::Malformed {
                        what: "window_update",
                        reason: "covariance requires paired_value",
                    });
                }
            }
            WindowAlgorithm::EventCount
            | WindowAlgorithm::ProtocolTime
            | WindowAlgorithm::ExponentiallyWeighted
            | WindowAlgorithm::QuantileSketch
            | WindowAlgorithm::RobustZScore => {}
        }
        if self
            .samples
            .back()
            .is_some_and(|sample| update.sequence <= sample.sequence)
        {
            return Err(FeatureError::Malformed {
                what: "window_update",
                reason: "sequence must increase",
            });
        }
        self.seen.insert(update.event_id.to_string());
        self.samples.push_back(Sample {
            event_id: update.event_id.to_string(),
            event_time: update.event_time,
            sequence: update.sequence,
            value: update.value,
            paired_value: update.paired_value,
        });
        self.evict(update.event_time)?;
        if self.algorithm == WindowAlgorithm::ExponentiallyWeighted {
            self.ewma = Some(match self.ewma {
                None => update.value,
                Some(previous) => ewma_step(previous, update.value, self.decay_ppm)?,
            });
        }
        Ok(true)
    }

    pub fn snapshot(&self) -> Result<WindowSnapshot, FeatureError> {
        let value = match self.algorithm {
            WindowAlgorithm::EventCount | WindowAlgorithm::ProtocolTime => self.mean()?,
            WindowAlgorithm::ExponentiallyWeighted => self.ewma,
            WindowAlgorithm::QuantileSketch => self.quantile()?,
            WindowAlgorithm::Covariance => self.covariance()?,
            WindowAlgorithm::RobustZScore => self.robust_z()?,
        };
        Ok(WindowSnapshot {
            algorithm: self.algorithm,
            parameter_version: self.parameter_version,
            count: u64::try_from(self.samples.len()).unwrap_or(u64::MAX),
            value,
        })
    }

    fn evict(&mut self, now: ProtocolTime) -> Result<(), FeatureError> {
        match self.algorithm {
            WindowAlgorithm::EventCount => {
                while self.samples.len() > self.event_capacity {
                    if let Some(sample) = self.samples.pop_front() {
                        self.seen.remove(&sample.event_id);
                    }
                }
            }
            WindowAlgorithm::ProtocolTime => {
                let cutoff = now.unix_micros().checked_sub(self.duration_micros).ok_or(
                    FeatureError::Malformed {
                        what: "window",
                        reason: "protocol time underflow",
                    },
                )?;
                while self
                    .samples
                    .front()
                    .is_some_and(|sample| sample.event_time.unix_micros() < cutoff)
                {
                    if let Some(sample) = self.samples.pop_front() {
                        self.seen.remove(&sample.event_id);
                    }
                }
            }
            WindowAlgorithm::ExponentiallyWeighted
            | WindowAlgorithm::QuantileSketch
            | WindowAlgorithm::Covariance
            | WindowAlgorithm::RobustZScore => {}
        }
        Ok(())
    }

    fn mean(&self) -> Result<Option<Decimal>, FeatureError> {
        if self.samples.is_empty() {
            return Ok(None);
        }
        let scale = self.samples[0].value.scale();
        let mut sum = Decimal::from_raw(0, scale).map_err(value_error)?;
        for sample in &self.samples {
            sum = sum.checked_add(sample.value).map_err(value_error)?;
        }
        let count = Decimal::from_raw(
            i128::try_from(self.samples.len()).map_err(|_| FeatureError::Malformed {
                what: "window",
                reason: "count overflow",
            })?,
            0,
        )
        .map_err(value_error)?;
        Ok(Some(
            sum.checked_div(count, scale, RoundingMode::NearestTiesToEven)
                .map_err(value_error)?,
        ))
    }

    fn quantile(&self) -> Result<Option<Decimal>, FeatureError> {
        if self.samples.is_empty() {
            return Ok(None);
        }
        let mut values: Vec<Decimal> = self.samples.iter().map(|sample| sample.value).collect();
        values.sort();
        let numerator = (self.samples.len() - 1)
            .checked_mul(usize::try_from(self.quantile_ppm).map_err(|_| {
                FeatureError::Malformed {
                    what: "window",
                    reason: "quantile overflow",
                }
            })?)
            .ok_or(FeatureError::Malformed {
                what: "window",
                reason: "quantile overflow",
            })?;
        let index = numerator / 1_000_000;
        Ok(Some(values[index]))
    }

    fn covariance(&self) -> Result<Option<Decimal>, FeatureError> {
        if self.samples.len() < 2 {
            return Ok(None);
        }
        let scale = self.samples[0].value.scale();
        let mut sum_x = Decimal::from_raw(0, scale).map_err(value_error)?;
        let mut sum_y = Decimal::from_raw(0, scale).map_err(value_error)?;
        let mut sum_xy = Decimal::from_raw(0, scale.saturating_mul(2)).map_err(value_error)?;
        for sample in &self.samples {
            let y = sample.paired_value.ok_or(FeatureError::Malformed {
                what: "window",
                reason: "missing paired_value",
            })?;
            sum_x = sum_x.checked_add(sample.value).map_err(value_error)?;
            sum_y = sum_y.checked_add(y).map_err(value_error)?;
            let xy = sample
                .value
                .checked_mul(y, scale.saturating_mul(2), RoundingMode::NearestTiesToEven)
                .map_err(value_error)?;
            sum_xy = sum_xy.checked_add(xy).map_err(value_error)?;
        }
        let n = i128::try_from(self.samples.len()).map_err(|_| FeatureError::Malformed {
            what: "window",
            reason: "count overflow",
        })?;
        let n_dec = Decimal::from_raw(n, 0).map_err(value_error)?;
        let mean_x = sum_x
            .checked_div(n_dec, scale, RoundingMode::NearestTiesToEven)
            .map_err(value_error)?;
        let mean_y = sum_y
            .checked_div(n_dec, scale, RoundingMode::NearestTiesToEven)
            .map_err(value_error)?;
        let mean_xy = mean_x
            .checked_mul(
                mean_y,
                scale.saturating_mul(2),
                RoundingMode::NearestTiesToEven,
            )
            .map_err(value_error)?;
        let second = sum_xy
            .checked_div(
                n_dec,
                scale.saturating_mul(2),
                RoundingMode::NearestTiesToEven,
            )
            .map_err(value_error)?;
        Ok(Some(second.checked_sub(mean_xy).map_err(value_error)?))
    }

    fn robust_z(&self) -> Result<Option<Decimal>, FeatureError> {
        if self.samples.len() < 3 {
            return Err(FeatureError::InsufficientHistory);
        }
        let mut values: Vec<Decimal> = self.samples.iter().map(|sample| sample.value).collect();
        values.sort();
        let location = median(&values)?;
        let mut deviations = Vec::with_capacity(values.len());
        for value in &values {
            let delta = if *value >= location {
                value.checked_sub(location).map_err(value_error)?
            } else {
                location.checked_sub(*value).map_err(value_error)?
            };
            deviations.push(delta);
        }
        deviations.sort();
        let mad = median(&deviations)?;
        if mad.raw() == 0 {
            return Err(FeatureError::Malformed {
                what: "window",
                reason: "mad is zero",
            });
        }
        let last = *values.last().ok_or(FeatureError::InsufficientHistory)?;
        let last_dev = last.checked_sub(location).map_err(value_error)?;
        Ok(Some(
            last_dev
                .checked_div(mad, last.scale(), RoundingMode::NearestTiesToEven)
                .map_err(value_error)?,
        ))
    }
}

fn median(values: &[Decimal]) -> Result<Decimal, FeatureError> {
    if values.is_empty() {
        return Err(FeatureError::InsufficientHistory);
    }
    let mid = values.len() / 2;
    if values.len().is_multiple_of(2) {
        let two = Decimal::from_raw(2, 0).map_err(value_error)?;
        let sum = values[mid - 1]
            .checked_add(values[mid])
            .map_err(value_error)?;
        sum.checked_div(two, values[mid].scale(), RoundingMode::NearestTiesToEven)
            .map_err(value_error)
    } else {
        Ok(values[mid])
    }
}

fn ewma_step(previous: Decimal, next: Decimal, decay_ppm: u32) -> Result<Decimal, FeatureError> {
    let scale = previous.scale();
    let alpha = Decimal::from_raw(i128::from(decay_ppm), 6).map_err(value_error)?;
    let one = Decimal::from_raw(1_000_000, 6).map_err(value_error)?;
    let one_minus = one.checked_sub(alpha).map_err(value_error)?;
    let left = next
        .checked_mul(alpha, scale, RoundingMode::NearestTiesToEven)
        .map_err(value_error)?;
    let right = previous
        .checked_mul(one_minus, scale, RoundingMode::NearestTiesToEven)
        .map_err(value_error)?;
    left.checked_add(right).map_err(value_error)
}

fn value_error(error: ValueError) -> FeatureError {
    match error {
        ValueError::DivisionByZero => FeatureError::Malformed {
            what: "window",
            reason: "division by zero",
        },
        ValueError::Overflow => FeatureError::Malformed {
            what: "window",
            reason: "overflow",
        },
        ValueError::ScaleMismatch { .. } => FeatureError::Malformed {
            what: "window",
            reason: "scale mismatch",
        },
        ValueError::Empty
        | ValueError::Invalid
        | ValueError::ExcessPrecision { .. }
        | ValueError::ScaleOutOfRange { .. }
        | ValueError::DownwardExactRescale { .. }
        | ValueError::OutOfRange => FeatureError::Malformed {
            what: "window",
            reason: "invalid decimal",
        },
    }
}

#[must_use]
pub fn window_debug_state(window: &RollingWindow) -> BTreeMap<String, String> {
    let mut state = BTreeMap::new();
    state.insert(
        "algorithm".to_owned(),
        window.algorithm.as_wire_name().to_owned(),
    );
    state.insert(
        "parameter_version".to_owned(),
        window.parameter_version.to_string(),
    );
    state.insert("count".to_owned(), window.samples.len().to_string());
    state
}
