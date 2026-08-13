CREATE TABLE IF NOT EXISTS entity_link_evidence
(
    evidence_id String,
    left_kind LowCardinality(String),
    left_id String,
    right_kind LowCardinality(String),
    right_id String,
    link_kind LowCardinality(String),
    probability_ppm UInt32,
    effective_at Int64,
    known_at Int64,
    revision UInt32
)
ENGINE = MergeTree
ORDER BY (left_kind, left_id, right_kind, right_id, effective_at, known_at, revision);

CREATE TABLE IF NOT EXISTS entity_cluster_membership
(
    cluster_version_id String,
    entity_id String,
    member_account_id String,
    weight_ppm UInt32,
    effective_at Int64,
    known_at Int64,
    superseded_at Nullable(Int64),
    revision UInt32
)
ENGINE = MergeTree
ORDER BY (entity_id, member_account_id, effective_at, known_at, revision);

CREATE VIEW IF NOT EXISTS entity_cluster_membership_asof AS
SELECT *
FROM entity_cluster_membership
WHERE known_at <= {as_of_known_at:Int64}
  AND (superseded_at IS NULL OR superseded_at > {as_of_known_at:Int64});
