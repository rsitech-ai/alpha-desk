CREATE TABLE IF NOT EXISTS relationship_features
(
    leader_account_id String,
    follower_account_id String,
    class LowCardinality(String),
    follower_probability_ppm UInt32,
    sample_size UInt32,
    median_lag_micros Int64,
    effective_at Int64,
    known_at Int64
)
ENGINE = MergeTree
ORDER BY (leader_account_id, follower_account_id, effective_at, known_at);
