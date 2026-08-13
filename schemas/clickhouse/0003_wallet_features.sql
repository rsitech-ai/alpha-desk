CREATE TABLE IF NOT EXISTS wallet_feature_snapshots
(
    feature_set_version String,
    subject_type LowCardinality(String),
    subject_id String,
    effective_at Int64,
    known_at Int64,
    revision UInt32,
    trading_gain String,
    time_weighted_return String,
    data_health LowCardinality(String)
)
ENGINE = MergeTree
ORDER BY (feature_set_version, subject_type, subject_id, effective_at, known_at, revision);

CREATE VIEW IF NOT EXISTS wallet_feature_snapshots_asof AS
SELECT *
FROM wallet_feature_snapshots
WHERE known_at <= {as_of_known_at:Int64};
