CREATE TABLE IF NOT EXISTS market_feature_snapshots
(
    feature_set_version String,
    market_id String,
    horizon_micros UInt64,
    effective_at Int64,
    known_at Int64,
    input_watermark UInt64,
    directional_flow String,
    crowding String,
    data_health LowCardinality(String),
    provenance_hash FixedString(32)
)
ENGINE = MergeTree
ORDER BY (feature_set_version, market_id, horizon_micros, effective_at, known_at);

CREATE VIEW IF NOT EXISTS market_feature_snapshots_asof AS
SELECT *
FROM market_feature_snapshots
WHERE known_at <= {as_of_known_at:Int64};
