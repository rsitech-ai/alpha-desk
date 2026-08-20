CREATE TABLE IF NOT EXISTS signals
(
    signal_id String,
    signal_type LowCardinality(String),
    market_id String,
    lifecycle_state LowCardinality(String),
    evidence_bundle_hash FixedString(32),
    as_of_block UInt64,
    effective_at Int64,
    known_at Int64
)
ENGINE = MergeTree
ORDER BY (signal_id, known_at);

CREATE TABLE IF NOT EXISTS signal_lifecycle_events
(
    signal_id String,
    previous LowCardinality(Nullable(String)),
    next LowCardinality(String),
    reason_code String,
    evidence_bundle_hash FixedString(32),
    known_at Int64
)
ENGINE = MergeTree
ORDER BY (signal_id, known_at);
