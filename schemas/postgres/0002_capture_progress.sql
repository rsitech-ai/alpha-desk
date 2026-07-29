BEGIN;

DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM capture_sequencer_cursors) THEN
        RAISE EXCEPTION
            '0002_capture_progress requires an empty legacy capture_sequencer_cursors table; reconcile and export legacy cursor evidence before migration'
            USING ERRCODE = '55000';
    END IF;
END;
$$;

CREATE TABLE capture_chain_progress (
    chain_id text PRIMARY KEY
        CHECK (
            chain_id <> ''
            AND chain_id = btrim(chain_id)
            AND octet_length(chain_id) <= 512
        ),
    first_block_height numeric(20, 0) NOT NULL
        CHECK (
            first_block_height >= 0
            AND first_block_height <= 18446744073709551615
        ),
    initialized_at_micros numeric(19, 0) NOT NULL
        CHECK (
            initialized_at_micros >= 0
            AND initialized_at_micros <= 9223372036854775807
        )
);

ALTER TABLE capture_sequencer_cursors
    ALTER COLUMN cursor_version TYPE numeric(20, 0)
        USING cursor_version::numeric(20, 0);

ALTER TABLE capture_sequencer_cursors
    DROP CONSTRAINT capture_sequencer_cursors_cursor_version_check;

ALTER TABLE capture_sequencer_cursors
    ADD CONSTRAINT capture_sequencer_cursors_cursor_version_check
    CHECK (
        cursor_version > 0
        AND cursor_version <= 18446744073709551615
    );

ALTER TABLE capture_sequencer_cursors
    ADD COLUMN updated_at_micros numeric(19, 0);

UPDATE capture_sequencer_cursors
SET updated_at_micros = round(extract(epoch FROM updated_at) * 1000000);

ALTER TABLE capture_sequencer_cursors
    ALTER COLUMN updated_at_micros SET NOT NULL;

ALTER TABLE capture_sequencer_cursors
    ADD CONSTRAINT capture_sequencer_cursors_updated_at_micros_check
    CHECK (
        updated_at_micros >= 0
        AND updated_at_micros <= 9223372036854775807
    );

CREATE TABLE capture_archived_blocks (
    chain_id text NOT NULL
        REFERENCES capture_chain_progress (chain_id)
        ON DELETE RESTRICT,
    block_height numeric(20, 0) NOT NULL
        CHECK (
            block_height >= 0
            AND block_height <= 18446744073709551615
        ),
    canonical_block_hash bytea NOT NULL
        CHECK (octet_length(canonical_block_hash) = 32),
    archive_receipt_id text NOT NULL
        CHECK (
            archive_receipt_id <> ''
            AND archive_receipt_id = btrim(archive_receipt_id)
            AND octet_length(archive_receipt_id) <= 512
        ),
    archive_manifest_id text NOT NULL
        CHECK (
            archive_manifest_id <> ''
            AND archive_manifest_id = btrim(archive_manifest_id)
            AND octet_length(archive_manifest_id) <= 512
        ),
    archive_object_hash bytea NOT NULL
        CHECK (octet_length(archive_object_hash) = 32),
    archive_manifest_hash bytea NOT NULL
        CHECK (octet_length(archive_manifest_hash) = 32),
    archive_schema_fingerprint bytea NOT NULL
        CHECK (octet_length(archive_schema_fingerprint) = 32),
    publication_count numeric(10, 0) NOT NULL
        CHECK (
            publication_count > 0
            AND publication_count <= 4294967295
        ),
    state text NOT NULL DEFAULT 'archived_pending'
        CHECK (
            state IN (
                'archived_pending',
                'publishing',
                'acknowledged',
                'quarantined'
            )
        ),
    archived_at_micros numeric(19, 0) NOT NULL
        CHECK (
            archived_at_micros >= 0
            AND archived_at_micros <= 9223372036854775807
        ),
    PRIMARY KEY (chain_id, block_height),
    UNIQUE (
        chain_id,
        block_height,
        canonical_block_hash,
        archive_receipt_id,
        archive_manifest_hash
    )
);

CREATE TABLE capture_block_publications (
    chain_id text NOT NULL,
    block_height numeric(20, 0) NOT NULL,
    publication_ordinal numeric(10, 0) NOT NULL
        CHECK (
            publication_ordinal >= 0
            AND publication_ordinal <= 4294967295
        ),
    message_id text NOT NULL
        CHECK (
            message_id <> ''
            AND message_id = btrim(message_id)
            AND octet_length(message_id) <= 512
        ),
    subject text NOT NULL
        CHECK (
            subject ~ '^[a-z0-9]+(\.[a-z0-9]+)*$'
            AND octet_length(subject) <= 512
        ),
    publication_hash bytea NOT NULL
        CHECK (octet_length(publication_hash) = 32),
    ack_stream text,
    ack_stream_sequence numeric(20, 0),
    ack_duplicate boolean,
    acknowledged_at_micros numeric(19, 0),
    PRIMARY KEY (chain_id, block_height, publication_ordinal),
    UNIQUE (chain_id, block_height, message_id),
    FOREIGN KEY (chain_id, block_height)
        REFERENCES capture_archived_blocks (chain_id, block_height)
        ON DELETE RESTRICT,
    CHECK (
        (
            ack_stream IS NULL
            AND ack_stream_sequence IS NULL
            AND ack_duplicate IS NULL
            AND acknowledged_at_micros IS NULL
        )
        OR (
            ack_stream ~ '^[A-Z0-9_]+$'
            AND octet_length(ack_stream) <= 512
            AND ack_stream_sequence > 0
            AND ack_stream_sequence <= 18446744073709551615
            AND ack_duplicate IS NOT NULL
            AND acknowledged_at_micros >= 0
            AND acknowledged_at_micros <= 9223372036854775807
        )
    )
);

ALTER TABLE capture_sequencer_cursors
    ADD CONSTRAINT capture_sequencer_cursors_chain_fk
    FOREIGN KEY (chain_id)
    REFERENCES capture_chain_progress (chain_id)
    ON DELETE RESTRICT;

ALTER TABLE capture_sequencer_cursors
    ADD CONSTRAINT capture_sequencer_cursors_archive_fk
    FOREIGN KEY (
        chain_id,
        committed_block_height,
        canonical_block_hash,
        archive_receipt_id,
        archive_manifest_hash
    )
    REFERENCES capture_archived_blocks (
        chain_id,
        block_height,
        canonical_block_hash,
        archive_receipt_id,
        archive_manifest_hash
    )
    ON DELETE RESTRICT;

CREATE INDEX capture_archived_blocks_pending_idx
    ON capture_archived_blocks (chain_id, block_height)
    WHERE state <> 'acknowledged';

CREATE INDEX capture_block_publications_unacknowledged_idx
    ON capture_block_publications (chain_id, block_height, publication_ordinal)
    WHERE ack_stream_sequence IS NULL;

CREATE FUNCTION validate_capture_block_publication_set()
RETURNS trigger
LANGUAGE plpgsql
AS $$
DECLARE
    checked_chain_id text;
    checked_block_height numeric(20, 0);
    expected_count numeric(10, 0);
    actual_count numeric(10, 0);
    acknowledged_count numeric(10, 0);
    block_state text;
BEGIN
    checked_chain_id := COALESCE(NEW.chain_id, OLD.chain_id);
    checked_block_height := COALESCE(NEW.block_height, OLD.block_height);

    SELECT publication_count, state
      INTO expected_count, block_state
      FROM capture_archived_blocks
     WHERE chain_id = checked_chain_id
       AND block_height = checked_block_height;

    IF NOT FOUND THEN
        RETURN NULL;
    END IF;

    SELECT count(*), count(*) FILTER (WHERE ack_stream_sequence IS NOT NULL)
      INTO actual_count, acknowledged_count
      FROM capture_block_publications
     WHERE chain_id = checked_chain_id
       AND block_height = checked_block_height;

    IF actual_count <> expected_count THEN
        RAISE EXCEPTION
            'capture publication count mismatch for chain % block %',
            checked_chain_id,
            checked_block_height
            USING ERRCODE = '23514';
    END IF;

    IF block_state = 'acknowledged' AND acknowledged_count <> expected_count THEN
        RAISE EXCEPTION
            'capture block marked acknowledged with incomplete publications'
            USING ERRCODE = '23514';
    END IF;

    IF block_state IN ('archived_pending', 'publishing')
       AND acknowledged_count = expected_count THEN
        RAISE EXCEPTION
            'capture block state does not reflect complete publications'
            USING ERRCODE = '23514';
    END IF;

    RETURN NULL;
END;
$$;

CREATE CONSTRAINT TRIGGER capture_archived_blocks_publication_set
AFTER INSERT OR UPDATE ON capture_archived_blocks
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW
EXECUTE FUNCTION validate_capture_block_publication_set();

CREATE CONSTRAINT TRIGGER capture_block_publications_publication_set
AFTER INSERT OR UPDATE OR DELETE ON capture_block_publications
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW
EXECUTE FUNCTION validate_capture_block_publication_set();

COMMIT;
