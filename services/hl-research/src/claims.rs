use serde::Serialize;
use serde::Serializer;
use serde::ser::Error;

use crate::error::ResearchError;

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

/// Serialize a corpus-honesty flag as `false`, or fail closed if a caller set it.
pub fn serialize_denied_true<S>(value: &bool, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    if *value {
        return Err(S::Error::custom(
            "hl-research cannot claim a live corpus or replica command usage",
        ));
    }
    serializer.serialize_bool(false)
}

pub fn refuse_corpus_claims(
    live_corpus: bool,
    replica_cmds_used: bool,
) -> Result<(), ResearchError> {
    if live_corpus {
        return Err(ResearchError::LiveCorpusForbidden);
    }
    if replica_cmds_used {
        return Err(ResearchError::ReplicaCmdsUsedForbidden);
    }
    Ok(())
}

pub fn encode_json<T: Serialize>(
    value: &T,
    live_corpus: bool,
    replica_cmds_used: bool,
) -> Result<Vec<u8>, ResearchError> {
    refuse_corpus_claims(live_corpus, replica_cmds_used)?;
    serde_json::to_vec(value).map_err(|_| ResearchError::InvalidFixture)
}
