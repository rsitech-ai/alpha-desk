use serde::{Deserialize, Serialize};

use crate::{EvidenceFamily, GraphError};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkPolicy {
    pub version: String,
    pub min_distinct_families: u32,
    pub posterior_threshold_ppm: u32,
    pub stability_duration_micros: u64,
}

impl LinkPolicy {
    pub fn from_toml(text: &str) -> Result<Self, GraphError> {
        let policy: Self = toml::from_str(text).map_err(|_| GraphError::Malformed {
            what: "link_policy",
            reason: "toml parse failed",
        })?;
        policy.validate()?;
        Ok(policy)
    }

    pub fn validate(&self) -> Result<(), GraphError> {
        if self.version.trim().is_empty() {
            return Err(GraphError::EmptyIdentifier {
                field: "link_policy.version",
            });
        }
        if self.min_distinct_families < 2 {
            return Err(GraphError::Malformed {
                what: "link_policy",
                reason: "min_distinct_families must be >= 2",
            });
        }
        if self.posterior_threshold_ppm == 0 || self.posterior_threshold_ppm > 1_000_000 {
            return Err(GraphError::Malformed {
                what: "link_policy",
                reason: "posterior threshold out of range",
            });
        }
        Ok(())
    }

    pub fn allows_soft_merge(&self, families: &[EvidenceFamily], posterior_ppm: u32) -> bool {
        let distinct = unique_family_count(families);
        distinct >= self.min_distinct_families && posterior_ppm >= self.posterior_threshold_ppm
    }
}

fn unique_family_count(families: &[EvidenceFamily]) -> u32 {
    let mut seen = Vec::new();
    for family in families {
        if matches!(family, EvidenceFamily::HardProtocol) {
            continue;
        }
        if !seen.contains(family) {
            seen.push(*family);
        }
    }
    u32::try_from(seen.len()).unwrap_or(u32::MAX)
}
