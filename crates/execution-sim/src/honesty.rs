use serde::Serializer;
use serde::ser::Error;

use crate::fill::FillClass;

/// Serialize a claim flag as `false`, or fail closed if a caller set it `true`.
pub fn serialize_denied_true<S>(value: &bool, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    if *value {
        return Err(S::Error::custom(
            "execution-sim cannot claim invented fills or live execution",
        ));
    }
    serializer.serialize_bool(false)
}

pub fn serialize_synthetic_fill_class<S>(
    value: &FillClass,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match value {
        FillClass::Synthetic => serializer.serialize_str("synthetic"),
        FillClass::Venue => Err(S::Error::custom(
            "execution-sim fills are synthetic and are not venue fills",
        )),
    }
}
