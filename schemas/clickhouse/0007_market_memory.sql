CREATE TABLE IF NOT EXISTS market_memory_vectors
(
    manifest_hash FixedString(32),
    market_id String,
    episode_id String,
    effective_at Int64,
    known_at Int64,
    values_milli Array(Int64),
    outcome_bps Nullable(Int64)
)
ENGINE = MergeTree
ORDER BY (manifest_hash, market_id, episode_id, effective_at, known_at);
