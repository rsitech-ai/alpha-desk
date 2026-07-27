use crate::ValueError;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

macro_rules! protocol_time {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(i64);

        impl $name {
            pub fn from_unix_micros(value: i64) -> Result<Self, ValueError> {
                if value < 0 {
                    Err(ValueError::OutOfRange)
                } else {
                    Ok(Self(value))
                }
            }

            pub const fn unix_micros(self) -> i64 {
                self.0
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.serialize_i64(self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                Self::from_unix_micros(i64::deserialize(deserializer)?)
                    .map_err(serde::de::Error::custom)
            }
        }
    };
}

protocol_time!(ProtocolTime);
protocol_time!(KnownTime);
