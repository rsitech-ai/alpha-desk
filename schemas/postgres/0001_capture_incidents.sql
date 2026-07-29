BEGIN;

CREATE TABLE capture_incidents (
    incident_id text PRIMARY KEY
        CHECK (incident_id ~ '^inc_[0-9a-f]{64}$'),
    chain_id text NOT NULL
        CHECK (chain_id <> '' AND chain_id = btrim(chain_id)),
    block_height numeric(20, 0) NOT NULL
        CHECK (
            block_height >= 0
            AND block_height <= 18446744073709551615
        ),
    reason_code text NOT NULL
        CHECK (reason_code <> '' AND reason_code = btrim(reason_code)),
    severity text NOT NULL DEFAULT 'critical'
        CHECK (severity IN ('critical')),
    status text NOT NULL DEFAULT 'open'
        CHECK (status IN ('open', 'resolved')),
    existing_source_count integer NOT NULL
        CHECK (existing_source_count > 0),
    conflicting_source_id text
        CHECK (
            conflicting_source_id IS NULL
            OR (
                conflicting_source_id <> ''
                AND conflicting_source_id = btrim(conflicting_source_id)
            )
        ),
    detected_at timestamptz NOT NULL,
    resolved_at timestamptz,
    resolution_reason text,
    resolution_approved_by text,
    CHECK (
        (
            reason_code = 'sequencer.committed_gap'
            AND conflicting_source_id IS NULL
        )
        OR (
            reason_code <> 'sequencer.committed_gap'
            AND conflicting_source_id IS NOT NULL
        )
    ),
    CHECK (
        (
            status = 'open'
            AND resolved_at IS NULL
            AND resolution_reason IS NULL
            AND resolution_approved_by IS NULL
        )
        OR (
            status = 'resolved'
            AND resolved_at IS NOT NULL
            AND resolution_reason <> ''
            AND resolution_reason = btrim(resolution_reason)
            AND resolution_approved_by <> ''
            AND resolution_approved_by = btrim(resolution_approved_by)
        )
    )
);

CREATE TABLE capture_incident_evidence (
    incident_id text NOT NULL
        REFERENCES capture_incidents (incident_id)
        ON DELETE RESTRICT,
    evidence_sequence integer NOT NULL
        CHECK (evidence_sequence >= 0),
    source_id text NOT NULL
        CHECK (source_id <> '' AND source_id = btrim(source_id)),
    source_cursor_epoch text NOT NULL
        CHECK (
            source_cursor_epoch <> ''
            AND source_cursor_epoch = btrim(source_cursor_epoch)
        ),
    source_cursor_offset numeric(20, 0) NOT NULL
        CHECK (
            source_cursor_offset >= 0
            AND source_cursor_offset <= 18446744073709551615
        ),
    source_content_hash bytea NOT NULL
        CHECK (octet_length(source_content_hash) = 32),
    canonical_block_hash bytea NOT NULL
        CHECK (octet_length(canonical_block_hash) = 32),
    spool_segment_manifest_hash bytea NOT NULL
        CHECK (octet_length(spool_segment_manifest_hash) = 32),
    recorded_at timestamptz NOT NULL,
    PRIMARY KEY (incident_id, evidence_sequence)
);

CREATE TABLE capture_sequencer_cursors (
    chain_id text PRIMARY KEY
        CHECK (chain_id <> '' AND chain_id = btrim(chain_id)),
    committed_block_height numeric(20, 0) NOT NULL
        CHECK (
            committed_block_height >= 0
            AND committed_block_height <= 18446744073709551615
        ),
    canonical_block_hash bytea NOT NULL
        CHECK (octet_length(canonical_block_hash) = 32),
    archive_manifest_hash bytea NOT NULL
        CHECK (octet_length(archive_manifest_hash) = 32),
    archive_receipt_id text NOT NULL
        CHECK (
            archive_receipt_id <> ''
            AND archive_receipt_id = btrim(archive_receipt_id)
        ),
    cursor_version bigint NOT NULL
        CHECK (cursor_version > 0),
    updated_at timestamptz NOT NULL
);

CREATE INDEX capture_incidents_open_height_idx
    ON capture_incidents (chain_id, block_height)
    WHERE status = 'open';

COMMIT;
