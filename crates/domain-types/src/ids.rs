use crate::ValueError;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

macro_rules! string_id {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, ValueError> {
                let value = value.into();
                if value.is_empty() || value.trim() != value {
                    return Err(ValueError::Invalid);
                }
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.serialize_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                Self::new(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
            }
        }
    };
}

string_id!(ChainId);
string_id!(TransactionId);
string_id!(EventId);
string_id!(AccountId);
string_id!(MasterAccountId);
string_id!(VaultId);
string_id!(EntityId);
string_id!(ClusterVersionId);
string_id!(DexId);
string_id!(MarketId);
string_id!(AssetId);
string_id!(OutcomeId);
string_id!(OrderId);
string_id!(ClientOrderId);
string_id!(TradeId);
string_id!(PositionEpisodeId);
string_id!(FeatureSetVersion);
string_id!(ModelVersion);
string_id!(SignalId);
string_id!(ExperimentId);
string_id!(RegimeId);
string_id!(CohortId);
string_id!(FeeScheduleId);
string_id!(SourceId);
string_id!(EvidenceId);
string_id!(ScenarioId);
string_id!(ManifestId);
string_id!(CheckpointId);
string_id!(LabelDefinitionId);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BlockHeight(u64);

impl BlockHeight {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Address([u8; 20]);

impl Address {
    pub const fn from_bytes(bytes: [u8; 20]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; 20] {
        &self.0
    }

    pub fn parse_api(input: &str) -> Result<Self, ValueError> {
        let hex_value = input.strip_prefix("0x").ok_or(ValueError::Invalid)?;
        if hex_value.len() != 40 || hex_value.bytes().any(|byte| byte.is_ascii_uppercase()) {
            return Err(ValueError::Invalid);
        }
        let mut bytes = [0_u8; 20];
        hex::decode_to_slice(hex_value, &mut bytes).map_err(|_| ValueError::Invalid)?;
        Ok(Self(bytes))
    }

    pub fn to_api_string(self) -> String {
        format!("0x{}", hex::encode(self.0))
    }
}

impl Serialize for Address {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_api_string())
    }
}

impl<'de> Deserialize<'de> for Address {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::parse_api(&String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}
