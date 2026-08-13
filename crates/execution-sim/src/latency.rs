use serde::{Deserialize, Serialize};

use crate::error::SimError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LatencyModel {
    Fixed { delay_micros: u64 },
    Uniform { min_micros: u64, max_micros: u64 },
}

impl LatencyModel {
    pub fn sample(self, seed: u64) -> Result<u64, SimError> {
        match self {
            Self::Fixed { delay_micros } => Ok(delay_micros),
            Self::Uniform {
                min_micros,
                max_micros,
            } => {
                if min_micros > max_micros {
                    return Err(SimError::UnmodeledCost {
                        component: "latency_uniform_bounds",
                    });
                }
                let span = max_micros - min_micros;
                if span == 0 {
                    return Ok(min_micros);
                }
                Ok(min_micros + (seed % (span + 1)))
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LatencyAssumptions {
    detection: LatencyModel,
    network: LatencyModel,
    processing: LatencyModel,
    max_book_staleness_micros: u64,
}

impl LatencyAssumptions {
    pub fn new(
        detection: LatencyModel,
        network: LatencyModel,
        processing: LatencyModel,
        max_book_staleness_micros: u64,
    ) -> Self {
        Self {
            detection,
            network,
            processing,
            max_book_staleness_micros,
        }
    }

    #[must_use]
    pub const fn max_book_staleness_micros(self) -> u64 {
        self.max_book_staleness_micros
    }

    pub fn total_delay_micros(self, seed: u64) -> Result<u64, SimError> {
        let detection = self.detection.sample(seed)?;
        let network = self
            .network
            .sample(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15))?;
        let processing = self
            .processing
            .sample(seed.wrapping_mul(0xD1B5_4A32_D192_ED03))?;
        detection
            .checked_add(network)
            .and_then(|value| value.checked_add(processing))
            .ok_or(SimError::InvalidAmount)
    }
}
