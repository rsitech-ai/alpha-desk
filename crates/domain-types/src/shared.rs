use crate::{BlockHeight, ValueError};
use serde::{Deserialize, Deserializer, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ClosedInterval<T> {
    pub lower: T,
    pub upper: T,
}

impl<T: PartialOrd> ClosedInterval<T> {
    pub fn new(lower: T, upper: T) -> Result<Self, ValueError> {
        if lower <= upper {
            Ok(Self { lower, upper })
        } else {
            Err(ValueError::OutOfRange)
        }
    }
}

impl<'de, T> Deserialize<'de> for ClosedInterval<T>
where
    T: Deserialize<'de> + PartialOrd,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct RawInterval<T> {
            lower: T,
            upper: T,
        }

        let raw = RawInterval::deserialize(deserializer)?;
        Self::new(raw.lower, raw.upper).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Horizon(u64);

impl Horizon {
    pub const MS_250: Self = Self(250_000);
    pub const SECOND_1: Self = Self(1_000_000);
    pub const SECONDS_5: Self = Self(5_000_000);
    pub const SECONDS_30: Self = Self(30_000_000);
    pub const MINUTES_2: Self = Self(120_000_000);
    pub const MINUTES_5: Self = Self(300_000_000);
    pub const MINUTES_30: Self = Self(1_800_000_000);
    pub const HOURS_4: Self = Self(14_400_000_000);
    pub const DAY_1: Self = Self(86_400_000_000);

    pub const fn from_micros(value: u64) -> Self {
        Self(value)
    }

    pub const fn as_micros(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction {
    Long,
    Short,
    Flat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OrderSide {
    Buy,
    Sell,
}

impl OrderSide {
    #[must_use]
    pub const fn as_wire_name(self) -> &'static str {
        match self {
            Self::Buy => "buy",
            Self::Sell => "sell",
        }
    }

    pub fn parse_wire(value: &str) -> Result<Self, ValueError> {
        match value {
            "buy" => Ok(Self::Buy),
            "sell" => Ok(Self::Sell),
            _ => Err(ValueError::Invalid),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CalibrationStatus {
    Calibrated,
    UnderReview,
    InsufficientEvidence,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct LatencyDistribution {
    pub p10_micros: u64,
    pub p50_micros: u64,
    pub p90_micros: u64,
    pub p99_micros: u64,
}

impl LatencyDistribution {
    pub fn new(
        p10_micros: u64,
        p50_micros: u64,
        p90_micros: u64,
        p99_micros: u64,
    ) -> Result<Self, ValueError> {
        if p10_micros <= p50_micros && p50_micros <= p90_micros && p90_micros <= p99_micros {
            Ok(Self {
                p10_micros,
                p50_micros,
                p90_micros,
                p99_micros,
            })
        } else {
            Err(ValueError::OutOfRange)
        }
    }
}

impl<'de> Deserialize<'de> for LatencyDistribution {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct RawLatencyDistribution {
            p10_micros: u64,
            p50_micros: u64,
            p90_micros: u64,
            p99_micros: u64,
        }

        let raw = RawLatencyDistribution::deserialize(deserializer)?;
        Self::new(
            raw.p10_micros,
            raw.p50_micros,
            raw.p90_micros,
            raw.p99_micros,
        )
        .map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct BlockRange {
    pub start_inclusive: BlockHeight,
    pub end_inclusive: BlockHeight,
}

impl BlockRange {
    pub fn new(
        start_inclusive: BlockHeight,
        end_inclusive: BlockHeight,
    ) -> Result<Self, ValueError> {
        if start_inclusive <= end_inclusive {
            Ok(Self {
                start_inclusive,
                end_inclusive,
            })
        } else {
            Err(ValueError::OutOfRange)
        }
    }
}

impl<'de> Deserialize<'de> for BlockRange {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct RawBlockRange {
            start_inclusive: BlockHeight,
            end_inclusive: BlockHeight,
        }

        let raw = RawBlockRange::deserialize(deserializer)?;
        Self::new(raw.start_inclusive, raw.end_inclusive).map_err(serde::de::Error::custom)
    }
}
