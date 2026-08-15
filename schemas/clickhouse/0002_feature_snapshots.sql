-- Bitemporal feature snapshots. Historical research must pass as_of_known_at
-- explicitly; these objects never default to "latest knowledge".

CREATE TABLE IF NOT EXISTS feature_snapshots
(
    feature_set_version String,
    subject_type LowCardinality(String),
    subject_id String,
    effective_at Int64,
    known_at Int64,
    revision UInt32,
    input_watermark UInt64,
    data_health LowCardinality(String),
    provenance_hash FixedString(32)
)
ENGINE = MergeTree
ORDER BY (feature_set_version, subject_type, subject_id, effective_at, known_at, revision);

CREATE VIEW IF NOT EXISTS feature_snapshots_asof AS
SELECT *
FROM feature_snapshots
WHERE known_at <= {as_of_known_at:Int64};
