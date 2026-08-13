use serde::Serializer;

/// Serialize any in-memory claim flag as `false`.
///
/// Reports keep these fields for schema stability, but JSON output cannot claim
/// alpha, significance, holdout pass, or stage pass even if a caller mutates the
/// struct.
pub fn serialize_unclaimed<S>(_: &bool, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_bool(false)
}
