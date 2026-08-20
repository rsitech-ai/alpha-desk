BEGIN;

CREATE TABLE cohort_definitions (
    cohort_id text PRIMARY KEY
        CHECK (cohort_id <> '' AND cohort_id = btrim(cohort_id)),
    version integer NOT NULL CHECK (version >= 1),
    predicate_hash bytea NOT NULL,
    exclusions text[] NOT NULL,
    created_at timestamptz NOT NULL
);

COMMIT;
